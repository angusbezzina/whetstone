//! The single dispatch for every mechanical enforcer of an in-force rule
//! (`wh check`), plus the Git helpers checks share.
//!
//! Gate commands never run through a shell. `&&` chains are executed as a
//! sequence of literal argv vectors; pipes, redirection and substitution are
//! refused with an actionable message. The environment is an allowlist,
//! time and output are bounded, and every run writes evidence artifacts to
//! the private state directory, where they survive instance cleanup.
//!
//! Outcomes are honest: a timeout, truncation or a run without evidence is
//! unknown, never a pass.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{ContentDigest, Enforcer, EvidenceRef, RecordId, Rule, Strength};
use crate::layout::ProjectLayout;

pub mod ast;
pub mod files;
pub mod lint;

pub use files::{FileSet, Source};

/// Evidence system of artifacts written under the private evidence root.
pub const EVIDENCE_SYSTEM: &str = "whetstone_evidence";
pub const DEFAULT_GATE_TIMEOUT: Duration = Duration::from_secs(900);
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
/// The flags a blocking check lists for each rule; a sweep lists every one.
const MAX_LISTED: usize = 20;

/// Host variables a gate may see. Everything else is cleared.
pub const ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TERM",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "RUSTUP_TOOLCHAIN",
    "CARGO_TARGET_DIR",
    "NODE_PATH",
    "PYTHONPATH",
    "VIRTUAL_ENV",
    "JAVA_HOME",
    "GOPATH",
    "GOROOT",
    "SSL_CERT_FILE",
    "XDG_CACHE_HOME",
];

pub fn evidence_root(layout: &ProjectLayout) -> PathBuf {
    layout.state_root().join("evidence")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// Paths changed since `base`; with `worktree` false, only what the commits
/// changed (a push sends commits, not the working tree).
pub fn changed_paths_between(
    project_root: &Path,
    base: Option<&str>,
    worktree: bool,
) -> Result<Vec<String>, String> {
    let mut paths = if worktree {
        changed_paths(project_root)?
    } else {
        Vec::new()
    };
    let Some(base) = base else {
        return Ok(paths);
    };
    if base.is_empty()
        || base.len() > 200
        || base.starts_with('-')
        || !base
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/._-~^@{}".contains(character))
    {
        return Err(format!("{base} is not a revision name."));
    }
    let git = |args: &[&str]| -> Result<Vec<u8>, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(project_root)
            .args(args)
            .output()
            .map_err(|error| format!("git is unavailable: {error}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    };
    let revision = format!("{base}^{{commit}}");
    git(&[
        "rev-parse",
        "--verify",
        "--quiet",
        "--end-of-options",
        &revision,
    ])
    .map_err(|_| format!("{base} is not a commit in this repository."))?;
    let merge_base =
        String::from_utf8_lossy(&git(&["merge-base", "--end-of-options", base, "HEAD"])?)
            .trim()
            .to_string();
    let listed = git(&[
        "diff",
        "--name-only",
        "--no-renames",
        "-z",
        &merge_base,
        "HEAD",
        "--",
    ])?;
    paths.extend(
        listed
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf8_lossy(entry).to_string()),
    );
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Repository-relative paths changed against HEAD, including untracked files.
pub fn changed_paths(project_root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .output()
        .map_err(|error| format!("git is unavailable: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let mut paths = Vec::new();
    let mut entries = output.stdout.split(|byte| *byte == 0);
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let status = &entry[..2];
        paths.push(String::from_utf8_lossy(&entry[3..]).to_string());
        // Renames carry the original path as the next NUL-separated field.
        if status.contains(&b'R') || status.contains(&b'C') {
            if let Some(original) = entries.next() {
                paths.push(String::from_utf8_lossy(original).to_string());
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Split a gate command into literal argv sequences joined by `&&`.
pub fn parse_command(line: &str) -> Result<Vec<Vec<String>>, String> {
    let mut sequences = vec![Vec::new()];
    let mut current = String::new();
    let mut in_token = false;
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    let refuse = |what: &str| {
        Err(format!(
            "Gate commands run without a shell, so {what} is not supported. Chain steps with && or point the gate at a script in the repository."
        ))
    };
    while let Some(character) = chars.next() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                } else if character == '\\' && open == '"' {
                    if let Some(next) = chars.next() {
                        current.push(next);
                    }
                } else {
                    current.push(character);
                }
            }
            None => match character {
                '\'' | '"' => {
                    quote = Some(character);
                    in_token = true;
                }
                '\\' => {
                    if let Some(next) = chars.next() {
                        current.push(next);
                        in_token = true;
                    }
                }
                ' ' | '\t' | '\n' => {
                    if in_token {
                        sequences
                            .last_mut()
                            .expect("sequence")
                            .push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                '&' if chars.peek() == Some(&'&') => {
                    chars.next();
                    if in_token {
                        sequences
                            .last_mut()
                            .expect("sequence")
                            .push(std::mem::take(&mut current));
                        in_token = false;
                    }
                    if sequences.last().is_some_and(Vec::is_empty) {
                        return Err("A gate command has an empty step around &&.".into());
                    }
                    sequences.push(Vec::new());
                }
                '|' => return refuse("a pipe"),
                ';' => return refuse("a ; separator"),
                '>' | '<' => return refuse("redirection"),
                '`' => return refuse("command substitution"),
                '$' if chars.peek() == Some(&'(') => return refuse("command substitution"),
                '&' => return refuse("a background &"),
                _ => {
                    current.push(character);
                    in_token = true;
                }
            },
        }
    }
    if quote.is_some() {
        return Err("A gate command has an unterminated quote.".into());
    }
    if in_token {
        sequences.last_mut().expect("sequence").push(current);
    }
    if sequences.iter().any(Vec::is_empty) {
        return Err("A gate command is empty.".into());
    }
    Ok(sequences)
}

/// Resolve an executable without a shell: repository-relative when it names
/// a path, otherwise the first match on the allowlisted `PATH`.
pub fn resolve_program(project_root: &Path, program: &str) -> Result<PathBuf, String> {
    if program.contains('/') {
        let candidate = project_root.join(program);
        let resolved = candidate
            .canonicalize()
            .map_err(|_| format!("The gate program {program} does not exist in the repository."))?;
        let root = project_root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        if !resolved.starts_with(&root) {
            return Err(format!(
                "The gate program {program} escapes the repository."
            ));
        }
        return Ok(resolved);
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(program);
        if is_executable(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "The gate program {program} is not on PATH. Install it or name a repository script."
    ))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn program_digest(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(sha256_hex(&bytes))
}

pub fn gate_environment(extra: &[(&str, String)]) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    for key in ENV_ALLOWLIST {
        if let Ok(value) = std::env::var(key) {
            environment.insert((*key).to_string(), value);
        }
    }
    for (key, value) in extra {
        environment.insert((*key).to_string(), value.clone());
    }
    environment
}

/// What one gate run established; anything short of a pass with evidence is
/// fail or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateState {
    Pass,
    Fail,
    Unknown,
}

/// Where a gate failed and what it said.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ArtifactFailure {
    pub location: String,
    pub message: String,
}

/// Everything a gate run needs, prepared once per `wh check`.
pub struct GateContext<'a> {
    pub project_root: &'a Path,
    pub run_dir: &'a Path,
    pub run_id: &'a str,
    pub timeout: Duration,
    /// The files content checks read (the staged index at pre-commit).
    pub files: &'a FileSet,
    /// Paths the change touched (empty for a whole-repository check).
    pub changed: &'a [String],
    /// What kind of check runs: scripts read it as `WH_MODE`.
    pub mode: RunMode,
    /// The revision a base check compares with, as `WH_BASE`.
    pub base: Option<&'a str>,
}

/// What a check is looking at, as a script enforcer reads it in `WH_MODE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunMode {
    /// The whole repository (a plain `wh check`, or a rule probe).
    #[default]
    All,
    /// The staged index only (pre-commit).
    Staged,
    /// What changed in the working tree, with no base revision.
    Changed,
    /// What changed since `WH_BASE` (pre-push and CI).
    Base,
    /// A sweep of the whole repository for findings.
    Sweep,
}

impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Staged => "staged",
            Self::Changed => "changed",
            Self::Base => "base",
            Self::Sweep => "sweep",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GateOutcome {
    pub id: String,
    pub name: String,
    pub strength: Strength,
    pub mechanism: String,
    pub command: String,
    pub state: GateState,
    pub summary: String,
    pub failures: Vec<ArtifactFailure>,
    /// Flags found beyond the ones `failures` lists.
    #[serde(skip_serializing_if = "is_zero")]
    pub omitted: usize,
    pub evidence: Vec<EvidenceRef>,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub program: Option<String>,
    pub program_digest: Option<String>,
}

impl GateOutcome {
    pub fn new(id: &RecordId, rule: &Rule) -> Self {
        let (mechanism, command) = mechanism_label(rule);
        Self {
            id: id.as_str().into(),
            name: rule.statement.clone(),
            strength: rule.strength,
            mechanism,
            command,
            state: GateState::Unknown,
            summary: String::new(),
            failures: Vec::new(),
            omitted: 0,
            evidence: Vec::new(),
            exit_code: None,
            elapsed_ms: 0,
            program: None,
            program_digest: None,
        }
    }

    /// Keep the first flags of a blocking check; the summary keeps the
    /// true count and says how many were left out.
    fn shortened(mut self) -> Self {
        if self.failures.len() > MAX_LISTED {
            let total = self.failures.len();
            self.omitted = total - MAX_LISTED;
            self.failures.truncate(MAX_LISTED);
            self.summary.push_str(&format!(
                " The first {MAX_LISTED} of {total} flags are listed; {} are left out.",
                self.omitted
            ));
        }
        self
    }

    pub fn unknown(mut self, summary: impl Into<String>) -> Self {
        self.state = GateState::Unknown;
        self.summary = summary.into();
        self
    }
}

/// The human mechanism name and the exact command or target of an enforcer.
pub fn mechanism_label(rule: &Rule) -> (String, String) {
    match &rule.enforcer {
        Enforcer::Test { command } => ("Test".into(), command.clone()),
        Enforcer::Validator { command } => ("Validator".into(), command.clone()),
        Enforcer::Formatter { tool } => ("Formatter".into(), tool.clone()),
        Enforcer::Lint { tool, code } => ("Lint".into(), format!("{tool} ({code})")),
        Enforcer::Ast { query, .. } => ("AST query".into(), query.clone()),
    }
}

fn slug(id: &str) -> String {
    id.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn write_evidence(
    context: &GateContext<'_>,
    file: &str,
    bytes: &[u8],
) -> Result<EvidenceRef, String> {
    fs::create_dir_all(context.run_dir).map_err(|error| error.to_string())?;
    let path = context.run_dir.join(file);
    fs::write(&path, bytes).map_err(|error| error.to_string())?;
    Ok(EvidenceRef {
        system: EVIDENCE_SYSTEM.into(),
        locator: format!("{}/{file}", context.run_id),
        digest: ContentDigest::new(sha256_hex(bytes)).ok(),
    })
}

/// Extract the findings a command printed: a `path:line[:col]` token on a
/// line is a located finding, and a line that starts `path: message` is a
/// finding about the whole file, wherever the path is a file in the
/// repository. Anything else stays in the evidence log only.
pub fn extract_failures(project_root: &Path, output: &str) -> Vec<ArtifactFailure> {
    let mut failures = Vec::new();
    for line in output.lines() {
        let before = failures.len();
        for token in line.split_whitespace() {
            let token = token.trim_matches(|character: char| {
                matches!(character, '(' | ')' | '[' | ']' | ',' | '\'' | '"' | '`')
            });
            let mut parts = token.splitn(3, ':');
            let (Some(path), Some(line_number)) = (parts.next(), parts.next()) else {
                continue;
            };
            let path = path.trim_start_matches("./");
            if path.is_empty()
                || path.starts_with('/')
                || path.contains("..")
                || line_number.is_empty()
                || !line_number
                    .chars()
                    .all(|character| character.is_ascii_digit())
                || !project_root.join(path).is_file()
            {
                continue;
            }
            let location = format!("{path}:{line_number}");
            if failures
                .iter()
                .any(|failure: &ArtifactFailure| failure.location == location)
            {
                continue;
            }
            let message = line.trim();
            failures.push(ArtifactFailure {
                location,
                message: message.chars().take(240).collect(),
            });
            break;
        }
        if failures.len() == before {
            if let Some(failure) = file_level_failure(project_root, line) {
                if !failures
                    .iter()
                    .any(|known: &ArtifactFailure| known.location == failure.location)
                {
                    failures.push(failure);
                }
            }
        }
    }
    failures
}

/// A `path: message` line about a whole repository file.
fn file_level_failure(project_root: &Path, line: &str) -> Option<ArtifactFailure> {
    let (path, message) = line.trim().split_once(": ")?;
    let path = path.trim_start_matches("./");
    let message = message.trim();
    if path.is_empty()
        || message.is_empty()
        || path.contains(char::is_whitespace)
        || !crate::domain::safe_relative_path(path)
        || !project_root.join(path).is_file()
    {
        return None;
    }
    Some(ArtifactFailure {
        location: path.to_string(),
        message: format!("{path}: {message}").chars().take(240).collect(),
    })
}

fn last_meaningful_line(text: &str) -> String {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .chars()
        .take(240)
        .collect()
}

struct CommandRun {
    exit_code: Option<i32>,
    output: String,
    truncated: bool,
    timed_out: bool,
    elapsed_ms: u64,
    program: String,
    program_digest: Option<String>,
}

fn run_sequence(
    context: &GateContext<'_>,
    sequences: &[Vec<String>],
    extra_env: &[(&str, String)],
) -> Result<CommandRun, String> {
    let environment = gate_environment(extra_env);
    let mut output = String::new();
    let mut elapsed_ms = 0;
    let mut last = None;
    for argv in sequences {
        let program = resolve_program(context.project_root, &argv[0])?;
        let digest = program_digest(&program);
        output.push_str(&format!("$ {}\n", argv.join(" ")));
        let remaining = context
            .timeout
            .saturating_sub(Duration::from_millis(elapsed_ms));
        let result = crate::execution::run_bounded(
            &program,
            &argv[1..],
            context.project_root,
            &environment,
            remaining,
            OUTPUT_LIMIT,
            OUTPUT_LIMIT,
        )
        .map_err(|error| format!("{} could not start: {error}", argv[0]))?;
        elapsed_ms += u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX);
        output.push_str(&result.stdout.text);
        output.push_str(&result.stderr.text);
        let exit_code = result.status.code();
        let truncated = result.stdout.truncated || result.stderr.truncated;
        let run = CommandRun {
            exit_code,
            output: String::new(),
            truncated,
            timed_out: result.timed_out,
            elapsed_ms,
            program: program.display().to_string(),
            program_digest: digest,
        };
        let stop = run.timed_out || run.truncated || exit_code != Some(0);
        last = Some(run);
        if stop {
            break;
        }
    }
    let mut run = last.ok_or_else(|| "A gate command is empty.".to_string())?;
    run.output = output;
    Ok(run)
}

fn finish(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    log: Option<&str>,
    extra: Value,
) -> GateOutcome {
    let slug = slug(&outcome.id);
    if let Some(log) = log {
        match write_evidence(context, &format!("{slug}.log"), log.as_bytes()) {
            Ok(evidence) => outcome.evidence.push(evidence),
            Err(error) => {
                return outcome.unknown(format!("Evidence could not be written: {error}"))
            }
        }
    }
    let files = outcome
        .evidence
        .iter()
        .map(|evidence| evidence.locator.clone())
        .collect::<Vec<_>>();
    let document = json!({
        "schema": "whetstone.gate-run.v1",
        "gate": outcome.id,
        "name": outcome.name,
        "mechanism": outcome.mechanism,
        "command": outcome.command,
        "state": outcome.state,
        "summary": outcome.summary,
        "failures": outcome.failures,
        "files": files,
        "exit_code": outcome.exit_code,
        "elapsed_ms": outcome.elapsed_ms,
        "program": outcome.program,
        "program_digest": outcome.program_digest,
        "detail": extra,
    });
    match serde_json::to_vec_pretty(&document)
        .map_err(|error| error.to_string())
        .and_then(|bytes| write_evidence(context, &format!("{slug}.json"), &bytes))
    {
        Ok(evidence) => outcome.evidence.push(evidence),
        Err(error) => return outcome.unknown(format!("Evidence could not be written: {error}")),
    }
    // A pass requires evidence by construction; make it explicit anyway.
    if outcome.state == GateState::Pass && outcome.evidence.is_empty() {
        outcome.state = GateState::Unknown;
        outcome.summary = "The gate reported success without evidence.".into();
    }
    outcome
}

/// Run one rule's enforcer.
/// A sweep keeps every flag; any other check lists the first few, and the
/// evidence log keeps them all.
pub fn run_gate(context: &GateContext<'_>, id: &RecordId, rule: &Rule) -> GateOutcome {
    let outcome = GateOutcome::new(id, rule);
    let outcome = match &rule.enforcer {
        Enforcer::Test { command }
        | Enforcer::Validator { command }
        | Enforcer::Formatter { tool: command } => {
            run_command_gate(context, outcome, rule, command, None)
        }
        Enforcer::Lint { tool, code } => {
            run_command_gate(context, outcome, rule, tool, Some(code.as_str()))
        }
        Enforcer::Ast { query, language } => {
            run_ast_gate(context, outcome, rule, query, language.as_deref())
        }
    };
    if context.mode == RunMode::Sweep {
        outcome
    } else {
        outcome.shortened()
    }
}

fn is_zero(count: &usize) -> bool {
    *count == 0
}

/// The files a command enforcer is about, one repository path per line,
/// written for the script to read as `WH_PATHS`: the check's files the rule's
/// paths cover.
fn write_paths_file(context: &GateContext<'_>, rule: &Rule, slug: &str) -> Result<PathBuf, String> {
    let mut listed = String::new();
    for path in context.files.paths().filter(|path| rule.applies_to(path)) {
        listed.push_str(path);
        listed.push('\n');
    }
    fs::create_dir_all(context.run_dir).map_err(|error| error.to_string())?;
    let path = context.run_dir.join(format!("{slug}.paths"));
    fs::write(&path, listed).map_err(|error| error.to_string())?;
    Ok(path)
}

/// What a command enforcer is told, beside the allowlisted environment.
fn script_environment(
    context: &GateContext<'_>,
    rule_id: &str,
    paths_file: &Path,
) -> Vec<(&'static str, String)> {
    vec![
        ("WH_EVIDENCE_DIR", context.run_dir.display().to_string()),
        ("WH_RUN_ID", context.run_id.to_string()),
        ("WH_MODE", context.mode.as_str().to_string()),
        ("WH_RULE", rule_id.to_string()),
        ("WH_PATHS", paths_file.display().to_string()),
        ("WH_BASE", context.base.unwrap_or_default().to_string()),
    ]
}

fn run_command_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    rule: &Rule,
    command: &str,
    lint_code: Option<&str>,
) -> GateOutcome {
    let mut sequences = match parse_command(command) {
        Ok(sequences) => sequences,
        Err(error) => {
            let outcome = outcome.unknown(error);
            return finish(context, outcome, None, json!({"parse_error": true}));
        }
    };
    // A lint code is read from the linter's structured report on the last
    // step of the chain; earlier steps (for example a build) run as written.
    let reporter = lint_code.and_then(|_| sequences.last_mut()).map(|argv| {
        let (reporter, structured) = lint::structured(argv);
        *argv = structured;
        reporter
    });
    let paths_file = match write_paths_file(context, rule, &slug(&outcome.id)) {
        Ok(path) => path,
        Err(error) => {
            let outcome = outcome.unknown(format!(
                "The in-scope file list could not be written: {error}"
            ));
            return finish(context, outcome, None, json!({"started": false}));
        }
    };
    let environment = script_environment(context, &outcome.id, &paths_file);
    let run = match run_sequence(context, &sequences, &environment) {
        Ok(run) => run,
        Err(error) => {
            let outcome = outcome.unknown(error);
            return finish(context, outcome, None, json!({"started": false}));
        }
    };
    outcome.exit_code = run.exit_code;
    outcome.elapsed_ms = run.elapsed_ms;
    outcome.program = Some(run.program.clone());
    outcome.program_digest = run.program_digest.clone();
    if run.timed_out {
        outcome = outcome.unknown(format!(
            "The gate timed out after {}s; a timeout is not a result.",
            context.timeout.as_secs()
        ));
    } else if run.truncated {
        outcome = outcome.unknown("The gate output exceeded its bound and was stopped.");
    } else if let (Some(code), Some(reporter)) = (lint_code, reporter) {
        match lint::findings(reporter, &run.output, code)
            .map(|found| in_scope(context.project_root, rule, found))
        {
            Ok((found, dropped)) if found.is_empty() && dropped > 0 => {
                outcome.state = GateState::Pass;
                outcome.summary = format!(
                    "The linter reported {code} only on files outside the rule's scope ({dropped} time(s), {}).",
                    reporter.label()
                );
            }
            Ok((found, _)) if !found.is_empty() => {
                outcome.state = GateState::Fail;
                outcome.summary = format!(
                    "The linter reported {code} {} time(s) ({}).",
                    found.len(),
                    reporter.label()
                );
                outcome.failures = found;
            }
            Ok(_) if run.exit_code == Some(0) || reporter != lint::Reporter::Text => {
                outcome.state = GateState::Pass;
                outcome.summary = format!("The linter reported no {code} ({}).", reporter.label());
            }
            Ok(_) => {
                outcome = outcome.unknown(format!(
                    "The linter exited {:?} without reporting {code}; the result is unknown.",
                    run.exit_code
                ));
            }
            Err(error) => {
                outcome = outcome.unknown(format!(
                    "The linter's report could not be read ({error}); the result is unknown."
                ));
            }
        }
    } else {
        match run.exit_code {
            Some(0) => {
                outcome.state = GateState::Pass;
                outcome.summary = format!("Passed in {:.1}s.", run.elapsed_ms as f64 / 1000.0);
            }
            Some(code) => {
                outcome.state = GateState::Fail;
                outcome.failures = extract_failures(context.project_root, &run.output);
                if matches!(rule.enforcer, Enforcer::Formatter { .. }) {
                    let (kept, dropped) = in_scope(
                        context.project_root,
                        rule,
                        std::mem::take(&mut outcome.failures),
                    );
                    outcome.failures = kept;
                    if outcome.failures.is_empty() && dropped > 0 {
                        outcome.state = GateState::Pass;
                        outcome.summary = format!(
                            "The formatter flagged only files outside the rule's scope ({dropped})."
                        );
                        return finish(
                            context,
                            outcome,
                            Some(&run.output),
                            json!({"argv": sequences}),
                        );
                    }
                }
                if outcome.failures.is_empty() {
                    outcome.failures.push(ArtifactFailure {
                        location: format!("exit code {code}"),
                        message: last_meaningful_line(&run.output),
                    });
                }
                outcome.summary = format!("Failed with exit code {code}.");
            }
            None => {
                outcome = outcome.unknown("The gate was terminated by a signal.");
            }
        }
    }
    finish(
        context,
        outcome,
        Some(&run.output),
        json!({"argv": sequences}),
    )
}

/// A lint's or formatter's flags split by the rule's scope: the tool reads
/// the whole repository, so a flag on a file the rule does not apply to
/// (outside its paths, or excluded) is dropped and counted. A flag that
/// names no repository file stays.
fn in_scope(
    project_root: &Path,
    rule: &Rule,
    failures: Vec<ArtifactFailure>,
) -> (Vec<ArtifactFailure>, usize) {
    let total = failures.len();
    let kept = failures
        .into_iter()
        .filter(|failure| {
            let path = failure
                .location
                .split(':')
                .next()
                .unwrap_or_default()
                .trim_start_matches("./");
            !(crate::domain::safe_relative_path(path)
                && project_root.join(path).is_file()
                && !rule.applies_to(path))
        })
        .collect::<Vec<_>>();
    let dropped = total - kept.len();
    (kept, dropped)
}

fn run_ast_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    rule: &Rule,
    query: &str,
    language: Option<&str>,
) -> GateOutcome {
    match ast::run_query(context.files, query, language, |path| rule.applies_to(path)) {
        Err(error) => {
            outcome = outcome.unknown(format!("The AST query is not usable: {error}"));
        }
        // A staged or changed check reads only what the change touched: a
        // change with no file the query reads cannot have broken it. Over
        // the whole repository, nothing in scope means a misaimed rule.
        Ok(run)
            if run.files_checked == 0
                && (context.files.source == Source::Staged || !context.changed.is_empty()) =>
        {
            outcome.state = GateState::Pass;
            outcome.summary = format!(
                "The change touches no {} file in scope, so it cannot break this rule.",
                run.languages.join(" or ")
            );
        }
        Ok(run) if run.files_checked == 0 => {
            outcome = outcome.unknown(format!(
                "No {} file in scope, so the query checked nothing; that is not a pass.",
                run.languages.join(" or ")
            ));
        }
        Ok(run) if !run.failures.is_empty() => {
            outcome.state = GateState::Fail;
            outcome.summary = format!(
                "The AST query matched {} time(s) in {} file(s).",
                run.failures.len(),
                run.files_checked
            );
            outcome.failures = run.failures;
        }
        Ok(run) => {
            outcome.state = GateState::Pass;
            outcome.summary = format!(
                "The AST query found nothing in {} file(s).",
                run.files_checked
            );
        }
    }
    let log = format!("query: {query}\nfiles: {}\n", context.files.len());
    finish(
        context,
        outcome,
        Some(&log),
        json!({"query": query, "language": language}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pass, flag and unavailable for every enforcer kind `run_gate`
    /// dispatches.
    #[test]
    fn every_enforcer_kind_passes_flags_and_is_honest_when_unavailable() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@example.com"],
            vec!["config", "user.name", "T"],
        ] {
            assert!(std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .status()
                .expect("git")
                .success());
        }
        fs::write(root.join("lib.rs"), "pub fn run(a: u8) -> u8 { a }\n").expect("lib");
        for args in [vec!["add", "-A"], vec!["commit", "-qm", "base"]] {
            assert!(std::process::Command::new("git")
                .args(&args)
                .current_dir(root)
                .status()
                .expect("git")
                .success());
        }
        let run_dir = root.join(".git/run");
        fs::create_dir_all(&run_dir).expect("run dir");
        let id = RecordId::new("rule.case").expect("id");
        let rule = |enforcer: serde_json::Value| -> Rule {
            serde_json::from_value(json!({
                "schema": "whetstone.rule.v2",
                "statement": "A case.",
                "rationale": "Because.",
                "strength": "must",
                "enforcer": enforcer,
                "source": {"kind": "owner"},
            }))
            .expect("rule")
        };
        let files = |entries: &[(&str, &str)]| {
            FileSet::from_contents(
                Source::WorkingTree,
                entries
                    .iter()
                    .map(|(path, text)| (path.to_string(), text.as_bytes().to_vec()))
                    .collect(),
            )
        };
        let changed = vec!["lib.rs".to_string()];
        let run = |enforcer: serde_json::Value, set: &FileSet| {
            let context = GateContext {
                project_root: root,
                run_dir: &run_dir,
                run_id: "case",
                timeout: Duration::from_secs(20),
                files: set,
                changed: &changed,
                mode: RunMode::All,
                base: None,
            };
            run_gate(&context, &id, &rule(enforcer)).state
        };
        let missing = "whetstone-no-such-program-7f3a";
        let clean = files(&[("lib.rs", "pub fn run(a: u8) -> u8 { a }\n")]);
        let (pass, fail, unknown) = (GateState::Pass, GateState::Fail, GateState::Unknown);
        let cases: Vec<(&str, GateState, GateState)> = vec![
            (
                "ast",
                run(
                    json!({"kind": "ast", "query": "(unsafe_block) @match", "language": "rust"}),
                    &clean,
                ),
                run(
                    json!({"kind": "ast", "query": "(unsafe_block) @match", "language": "rust"}),
                    &files(&[("lib.rs", "fn f(p: *const u8) -> u8 { unsafe { *p } }\n")]),
                ),
            ),
            (
                "lint",
                run(
                    json!({"kind": "lint", "tool": "echo clean", "code": "F401"}),
                    &clean,
                ),
                run(
                    json!({"kind": "lint", "tool": "echo lib.rs:1:1: F401 unused", "code": "F401"}),
                    &clean,
                ),
            ),
            (
                "formatter",
                run(json!({"kind": "formatter", "tool": "true"}), &clean),
                run(json!({"kind": "formatter", "tool": "false"}), &clean),
            ),
            (
                "test",
                run(json!({"kind": "test", "command": "true"}), &clean),
                run(json!({"kind": "test", "command": "false"}), &clean),
            ),
            (
                "validator",
                run(json!({"kind": "validator", "command": "true"}), &clean),
                run(json!({"kind": "validator", "command": "false"}), &clean),
            ),
        ];
        for (kind, passed, flagged) in &cases {
            assert_eq!(*passed, pass, "{kind} passes a clean change");
            assert_ne!(*flagged, pass, "{kind} never passes a breaking change");
        }
        for (kind, _, flagged) in &cases {
            assert_eq!(*flagged, fail, "{kind} flags a breaking change");
        }
        // Unavailable: the enforcer cannot run, and says so as unknown.
        let unavailable = [
            (
                "ast",
                run(
                    json!({"kind": "ast", "query": "(not a query", "language": "rust"}),
                    &clean,
                ),
            ),
            (
                "lint",
                run(
                    json!({"kind": "lint", "tool": missing, "code": "F401"}),
                    &clean,
                ),
            ),
            (
                "formatter",
                run(json!({"kind": "formatter", "tool": missing}), &clean),
            ),
            (
                "test",
                run(json!({"kind": "test", "command": missing}), &clean),
            ),
            (
                "validator",
                run(json!({"kind": "validator", "command": missing}), &clean),
            ),
        ];
        for (kind, state) in unavailable {
            assert_eq!(state, unknown, "{kind} is unknown when it cannot run");
        }
    }

    #[test]
    fn commands_split_into_literal_argv_chains_and_refuse_shell_syntax() {
        assert_eq!(
            parse_command("cargo clippy -- -D warnings && cargo test").expect("parse"),
            vec![
                vec!["cargo", "clippy", "--", "-D", "warnings"],
                vec!["cargo", "test"]
            ]
        );
        assert_eq!(
            parse_command(r#"node "scripts/a b.mjs" --name 'x y'"#).expect("parse"),
            vec![vec!["node", "scripts/a b.mjs", "--name", "x y"]]
        );
        for refused in [
            "cargo test | tee out",
            "a; b",
            "a > b",
            "echo $(whoami)",
            "echo `id`",
            "sleep 1 &",
            "a && && b",
            "\"open",
            "",
        ] {
            assert!(parse_command(refused).is_err(), "{refused} accepted");
        }
    }

    #[test]
    fn failures_are_extracted_only_for_real_repository_paths() {
        let temp = tempfile::tempdir().expect("temp");
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::write(temp.path().join("src/lib.rs"), "fn main() {}").expect("file");
        let output = "error[E0308]: mismatched types\n --> src/lib.rs:12:5\nnote: /etc/passwd:1 ignored\nother.rs:3 missing";
        let failures = extract_failures(temp.path(), output);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].location, "src/lib.rs:12");
    }

    #[test]
    fn a_tool_s_flags_on_files_out_of_the_rule_s_scope_are_dropped() {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path();
        for path in ["src/lib.rs", "src/generated/api.rs", "docs/a.rs"] {
            fs::create_dir_all(root.join(path).parent().expect("parent")).expect("dir");
            fs::write(root.join(path), "fn main() {}\n").expect("file");
        }
        let rule: Rule = serde_json::from_value(json!({
            "schema": "whetstone.rule.v2",
            "statement": "No unused imports.",
            "rationale": "Because.",
            "strength": "must",
            "enforcer": {"kind": "lint", "tool": "ruff check", "code": "F401"},
            "source": {"kind": "owner"},
            "paths": ["src/**"],
        }))
        .expect("rule");
        let flag = |location: &str| ArtifactFailure {
            location: location.into(),
            message: "F401".into(),
        };
        let (kept, dropped) = in_scope(
            root,
            &rule,
            vec![
                flag("src/lib.rs:1"),
                flag("src/generated/api.rs:1"),
                flag("docs/a.rs"),
                flag("linter output"),
                flag("src/missing.rs:3"),
            ],
        );
        assert_eq!(dropped, 2, "generated code and a file outside the paths");
        assert_eq!(
            kept.iter()
                .map(|failure| failure.location.as_str())
                .collect::<Vec<_>>(),
            ["src/lib.rs:1", "linter output", "src/missing.rs:3"],
            "a flag on no repository file stays"
        );
    }

    #[test]
    fn programs_resolve_inside_the_repository_or_on_path_only() {
        let temp = tempfile::tempdir().expect("temp");
        assert!(resolve_program(temp.path(), "../escape.sh").is_err());
        assert!(resolve_program(temp.path(), "definitely-not-a-program-xyz").is_err());
        assert!(resolve_program(temp.path(), "sh").is_ok());
    }
}
