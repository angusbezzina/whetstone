//! The single dispatch for every mechanical enforcer of an in-force rule
//! (`wh check`), plus the Git helpers checks share.
//!
//! Gate commands never run through a shell. `&&` chains are executed as a
//! sequence of literal argv vectors; pipes, redirection and substitution are
//! refused with an actionable message. The environment is an allowlist,
//! time and output are bounded, and every run writes evidence artifacts to
//! the private state directory, where they survive instance cleanup.
//!
//! Outcomes are honest: a timeout, truncation, missing driver, failed doctor
//! or a drive without evidence is unknown, never a pass.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    ContentDigest, Enforcer, EvidenceRef, Feature, RecordId, Rule, Strength, VerificationAxis,
};
use crate::proof::{ArtifactFailure, GateArtifact, EVIDENCE_SYSTEM};
use crate::storage::ProjectLayout;

pub mod ast;
mod drive;
pub mod files;
pub mod lint;
pub mod surface;
pub mod tokens;

pub use drive::*;
pub use files::{FileSet, Source};

pub const DRIVER_RELATIVE: &str = "whetstone/verify/drive.mjs";
pub const DEFAULT_GATE_TIMEOUT: Duration = Duration::from_secs(900);
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;
const MAX_FAILURES: usize = 20;

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
    "CHROME_BIN",
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

pub fn driver_path(project_root: &Path) -> Option<String> {
    project_root
        .join(DRIVER_RELATIVE)
        .is_file()
        .then(|| DRIVER_RELATIVE.to_string())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// A cheap, exact fingerprint of the working tree: the HEAD commit plus the
/// content of every modified or untracked (non-ignored) file.
pub fn workspace_fingerprint(project_root: &Path) -> Result<String, String> {
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
    let head = git(&["rev-parse", "--verify", "-q", "HEAD"]).unwrap_or_default();
    let status = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all"])?;
    let mut hasher = Sha256::new();
    hasher.update(b"whetstone.workspace.v1\0");
    hasher.update(&head);
    hasher.update(&status);
    for entry in status.split(|byte| *byte == 0) {
        if entry.len() < 4 {
            continue;
        }
        let path = String::from_utf8_lossy(&entry[3..]).to_string();
        let full = project_root.join(&path);
        if let Ok(metadata) = fs::symlink_metadata(&full) {
            if metadata.is_file() && metadata.len() <= 16 * 1024 * 1024 {
                if let Ok(bytes) = fs::read(&full) {
                    hasher.update(path.as_bytes());
                    hasher.update(Sha256::digest(&bytes));
                }
            }
        }
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// The current HEAD commit, if the repository has one.
pub fn head_commit(project_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["rev-parse", "--verify", "-q", "HEAD"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| !head.is_empty())
}

/// Paths changed in commits after `since` (exclusive) up to HEAD.
pub fn changed_since(project_root: &Path, since: &str) -> Result<Vec<String>, String> {
    if since.is_empty() || !since.chars().all(|character| character.is_ascii_hexdigit()) {
        return Err("invalid commit id".into());
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["diff", "--name-only", "-z", since, "HEAD"])
        .output()
        .map_err(|error| format!("git is unavailable: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8_lossy(entry).to_string())
        .collect())
}

/// Paths changed in the working tree plus, with `base`, every path that
/// differs between the merge base of `base` and HEAD (a committed change).
pub fn changed_paths_since(project_root: &Path, base: Option<&str>) -> Result<Vec<String>, String> {
    changed_paths_between(project_root, base, true)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DoctorResult {
    pub ok: bool,
    pub detail: String,
}

/// A brief an agent recorded, as the brief enforcer sees it.
#[derive(Debug, Clone)]
pub struct BriefView {
    pub area: String,
    pub recorded_at: String,
    pub commit: Option<String>,
    pub skills: Vec<String>,
}

/// Everything a gate run needs, prepared once per `wh check`.
pub struct GateContext<'a> {
    pub project_root: &'a Path,
    pub run_dir: &'a Path,
    pub run_id: &'a str,
    pub features: &'a BTreeMap<RecordId, Feature>,
    pub doctor: Option<&'a DoctorResult>,
    pub timeout: Duration,
    /// Change-specific steps for one feature, appended to its accepted steps.
    pub change_steps: Option<(&'a RecordId, &'a [String])>,
    /// The files content checks read (the staged index at pre-commit).
    pub files: &'a FileSet,
    /// Paths the change touched (empty for a whole-repository check).
    pub changed: &'a [String],
    /// Where public surfaces are compared from: the last pushed revision.
    pub surface_base: Option<&'a str>,
    /// Briefs recorded for this change.
    pub briefs: &'a [BriefView],
}

#[derive(Debug, Clone, Serialize)]
pub struct GateOutcome {
    pub id: String,
    pub name: String,
    pub strength: Strength,
    /// The enforcement ladder rung: mechanical, question or review.
    pub family: &'static str,
    pub mechanism: String,
    pub command: String,
    pub state: VerificationAxis,
    pub summary: String,
    pub failures: Vec<ArtifactFailure>,
    pub evidence: Vec<EvidenceRef>,
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub program: Option<String>,
    pub program_digest: Option<String>,
    /// The result is the owner's call: raise a hand instead of repairing.
    pub raise_hand: bool,
    /// Recorded and shown, never enforced (a question rule in shadow).
    pub shadow: bool,
}

impl GateOutcome {
    pub fn new(id: &RecordId, rule: &Rule) -> Self {
        let (mechanism, command) = mechanism_label(rule);
        Self {
            id: id.as_str().into(),
            name: rule.statement.clone(),
            strength: rule.strength,
            family: rule.enforcer.family().label(),
            mechanism,
            command,
            state: VerificationAxis::Unknown,
            summary: String::new(),
            failures: Vec::new(),
            evidence: Vec::new(),
            exit_code: None,
            elapsed_ms: 0,
            program: None,
            program_digest: None,
            raise_hand: false,
            shadow: rule.in_shadow(),
        }
    }

    pub fn unknown(mut self, summary: impl Into<String>) -> Self {
        self.state = VerificationAxis::Unknown;
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
        Enforcer::Drive { feature } => ("Drive".into(), format!("prove {}", feature.as_str())),
        Enforcer::DesignTokens { tokens, .. } => ("Design tokens".into(), tokens.clone()),
        Enforcer::PublicSurface { surfaces } => (
            "Public surface".into(),
            if surfaces.is_empty() {
                "exports, cli, json".into()
            } else {
                surfaces.join(", ")
            },
        ),
        Enforcer::Brief { skills } => (
            "Brief".into(),
            if skills.is_empty() {
                "/how, /why or /blast-radius".into()
            } else {
                skills.join(", ")
            },
        ),
        Enforcer::Question {
            question, model, ..
        } => ("Jev question".into(), format!("{question} ({model})")),
        Enforcer::Review { reviewer } => ("Review".into(), reviewer.label()),
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

/// Extract `path:line[:col]` locations that exist in the repository.
pub fn extract_failures(project_root: &Path, output: &str) -> Vec<ArtifactFailure> {
    let mut failures = Vec::new();
    for line in output.lines() {
        if failures.len() >= MAX_FAILURES {
            break;
        }
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
    }
    failures
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
    let artifact = GateArtifact {
        summary: outcome.summary.clone(),
        failures: outcome.failures.clone(),
        files,
    };
    let document = json!({
        "schema": "whetstone.gate-run.v1",
        "gate": outcome.id,
        "name": outcome.name,
        "mechanism": outcome.mechanism,
        "command": outcome.command,
        "state": outcome.state,
        "summary": artifact.summary,
        "failures": artifact.failures,
        "files": artifact.files,
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
    if outcome.state == VerificationAxis::Pass && outcome.evidence.is_empty() {
        outcome.state = VerificationAxis::Unknown;
        outcome.summary = "The gate reported success without evidence.".into();
    }
    outcome
}

/// Summary prefix of a drive the driver could not reach, with the unmet
/// prerequisite it named.
pub const UNREACHABLE_PREFIX: &str = "Unreachable: ";

/// A gate that was deliberately not run; unknown, never a pass.
pub fn skipped_outcome(id: &RecordId, rule: &Rule, reason: &str) -> GateOutcome {
    GateOutcome::new(id, rule).unknown(reason)
}

/// Run one mechanical enforcer. Question and review enforcers are resolved by
/// the judgment and attestation paths, never here.
pub fn run_gate(context: &GateContext<'_>, id: &RecordId, rule: &Rule) -> GateOutcome {
    let outcome = GateOutcome::new(id, rule);
    match &rule.enforcer {
        Enforcer::Test { command }
        | Enforcer::Validator { command }
        | Enforcer::Formatter { tool: command } => {
            run_command_gate(context, outcome, command, None)
        }
        Enforcer::Lint { tool, code } => {
            run_command_gate(context, outcome, tool, Some(code.as_str()))
        }
        Enforcer::Ast { query, language } => {
            run_ast_gate(context, outcome, rule, query, language.as_deref())
        }
        Enforcer::Drive { feature } => run_drive_gate(context, outcome, feature),
        Enforcer::DesignTokens {
            tokens,
            stylesheets,
            sizes,
        } => run_tokens_gate(context, outcome, rule, tokens, stylesheets, *sizes),
        Enforcer::PublicSurface { surfaces } => run_surface_gate(context, outcome, rule, surfaces),
        Enforcer::Brief { skills } => run_brief_gate(context, outcome, rule, skills),
        Enforcer::Question { .. } | Enforcer::Review { .. } => {
            let outcome = outcome
                .unknown("This rule is held by a question or a review, not a mechanical check.");
            finish(context, outcome, None, json!({}))
        }
    }
}

fn run_command_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
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
    let run = match run_sequence(
        context,
        &sequences,
        &[
            ("WH_EVIDENCE_DIR", context.run_dir.display().to_string()),
            ("WH_RUN_ID", context.run_id.to_string()),
        ],
    ) {
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
        match lint::findings(reporter, &run.output, code) {
            Ok(found) if !found.is_empty() => {
                outcome.state = VerificationAxis::Fail;
                outcome.summary = format!(
                    "The linter reported {code} {} time(s) ({}).",
                    found.len(),
                    reporter.label()
                );
                outcome.failures = found;
            }
            Ok(_) if run.exit_code == Some(0) || reporter != lint::Reporter::Text => {
                outcome.state = VerificationAxis::Pass;
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
                outcome.state = VerificationAxis::Pass;
                outcome.summary = format!("Passed in {:.1}s.", run.elapsed_ms as f64 / 1000.0);
            }
            Some(code) => {
                outcome.state = VerificationAxis::Fail;
                outcome.failures = extract_failures(context.project_root, &run.output);
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

fn run_ast_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    rule: &Rule,
    query: &str,
    language: Option<&str>,
) -> GateOutcome {
    // An earlier standard named a rule in `whetstone/rules` instead of a
    // query; that keeps working through the compiled-in scanner.
    if !query.trim_start().starts_with('(') && !query.trim_start().starts_with('[') {
        return run_scanner_rule_gate(context, outcome, query);
    }
    match ast::run_query(
        context.files,
        query,
        language,
        |path| rule.applies_to(path),
        MAX_FAILURES,
    ) {
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
            outcome.state = VerificationAxis::Pass;
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
            outcome.state = VerificationAxis::Fail;
            outcome.summary = format!(
                "The AST query matched {} time(s) in {} file(s).",
                run.failures.len(),
                run.files_checked
            );
            outcome.failures = run.failures;
        }
        Ok(run) => {
            outcome.state = VerificationAxis::Pass;
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

fn run_scanner_rule_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    scanner_rule: &str,
) -> GateOutcome {
    let scan_paths = vec![context.project_root.to_path_buf()];
    let filter = vec![scanner_rule.to_string()];
    let result = crate::check::run(crate::check::CheckOptions {
        project_dir: context.project_root,
        rules_dir: None,
        scan_paths: &scan_paths,
        lang_filter: None,
        rule_filter: Some(&filter),
        execute_command_validators: false,
    });
    let count = |field: &str| result.get(field).and_then(Value::as_u64).unwrap_or(0);
    let rules = count("rules_applied");
    let violations = count("violations_count");
    if rules == 0 {
        outcome = outcome.unknown(format!(
            "No scanner rule named {scanner_rule} applied; bind the rule to an AST query or an existing rule in whetstone/rules."
        ));
    } else if violations > 0 {
        outcome.state = VerificationAxis::Fail;
        outcome.summary = format!("The scanner rule reported {violations} violation(s).");
        outcome.failures = result
            .get("violations")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .take(MAX_FAILURES)
                    .map(|item| ArtifactFailure {
                        location: format!(
                            "{}:{}",
                            item.get("file")
                                .and_then(Value::as_str)
                                .unwrap_or("unknown"),
                            item.get("line").and_then(Value::as_u64).unwrap_or(0)
                        ),
                        message: item
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("violation")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
    } else {
        outcome.state = VerificationAxis::Pass;
        outcome.summary = format!("The scanner applied {rules} rule(s) without violations.");
    }
    let log = serde_json::to_string_pretty(&result).unwrap_or_default();
    finish(
        context,
        outcome,
        Some(&log),
        json!({"scanner_rule": scanner_rule}),
    )
}

fn run_tokens_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    rule: &Rule,
    token_file: &str,
    stylesheets: &[String],
    sizes: bool,
) -> GateOutcome {
    // The vocabulary comes from what is being committed when the token file
    // itself is staged, otherwise from the working tree.
    let token_text = context
        .files
        .text(token_file)
        .map(str::to_owned)
        .or_else(|| fs::read_to_string(context.project_root.join(token_file)).ok());
    let Some(token_text) = token_text else {
        let outcome = outcome.unknown(format!(
            "The token file {token_file} does not exist, so there is no vocabulary to check against."
        ));
        return finish(context, outcome, None, json!({"tokens": token_file}));
    };
    let vocabulary = match tokens::Tokens::parse(token_file, &token_text) {
        Ok(vocabulary) => vocabulary,
        Err(error) => {
            let outcome = outcome.unknown(format!("{token_file}: {error}."));
            return finish(context, outcome, None, json!({"tokens": token_file}));
        }
    };
    let in_scope = |path: &str| {
        tokens::is_stylesheet(path)
            && rule.applies_to(path)
            && (stylesheets.is_empty()
                || stylesheets
                    .iter()
                    .any(|pattern| crate::domain::entry_point_matches(pattern, path)))
    };
    let mut checked = 0;
    for (path, _) in context.files.iter() {
        if !in_scope(path) {
            continue;
        }
        let Some(text) = context.files.text(path) else {
            continue;
        };
        checked += 1;
        outcome.failures.extend(tokens::check_stylesheet(
            path,
            text,
            &vocabulary,
            path == token_file,
            sizes,
        ));
    }
    outcome.failures.truncate(MAX_FAILURES * 5);
    if checked == 0 && context.changed.is_empty() && context.files.source == Source::WorkingTree {
        outcome = outcome
            .unknown("No stylesheet is in scope, so nothing was checked; that is not a pass.");
    } else if outcome.failures.is_empty() {
        outcome.state = VerificationAxis::Pass;
        outcome.summary = format!(
            "{checked} stylesheet(s) use only the {} token(s) in {token_file}.",
            vocabulary.len()
        );
    } else {
        outcome.state = VerificationAxis::Fail;
        outcome.summary = format!(
            "{} literal colour or size value(s) outside {token_file}.",
            outcome.failures.len()
        );
    }
    finish(
        context,
        outcome,
        None,
        json!({"tokens": token_file, "stylesheets_checked": checked}),
    )
}

fn run_surface_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    rule: &Rule,
    surfaces: &[String],
) -> GateOutcome {
    outcome.raise_hand = true;
    let Some(base) = context.surface_base else {
        let outcome = outcome.unknown(
            "No pushed revision to compare with (no upstream or origin/HEAD); the public surface is unknown.",
        );
        return finish(context, outcome, None, json!({}));
    };
    let mut compared = 0;
    let mut paths = context
        .files
        .paths()
        .filter(|path| surface::is_surface_file(path) && rule.applies_to(path))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    paths.extend(
        context
            .files
            .deleted
            .iter()
            .filter(|path| surface::is_surface_file(path) && rule.applies_to(path))
            .cloned(),
    );
    for path in paths {
        // A file the base did not have is new surface, not a change to a
        // promise anyone relies on yet: it is compared from the next change.
        let Some(before) = files::at_revision(context.project_root, base, &path)
            .map(|text| surface::surface_of(&path, &text, surfaces))
        else {
            continue;
        };
        let after = context
            .files
            .text(&path)
            .map(|text| surface::surface_of(&path, text, surfaces))
            .unwrap_or_default();
        compared += 1;
        outcome
            .failures
            .extend(surface::diff(&path, &before, &after));
    }
    outcome.failures.truncate(MAX_FAILURES * 5);
    if outcome.failures.is_empty() {
        outcome.state = VerificationAxis::Pass;
        outcome.summary = format!(
            "No public surface changed in {compared} file(s) since {}.",
            &base[..base.len().min(12)]
        );
    } else {
        outcome.state = VerificationAxis::Fail;
        outcome.summary = format!(
            "The public surface changed ({} item(s)); this is the owner's call: raise a hand before continuing.",
            outcome.failures.len()
        );
    }
    finish(
        context,
        outcome,
        None,
        json!({"base": base, "files_compared": compared}),
    )
}

fn run_brief_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    rule: &Rule,
    skills: &[String],
) -> GateOutcome {
    let touched = context
        .changed
        .iter()
        .filter(|path| rule.applies_to(path))
        .cloned()
        .collect::<Vec<_>>();
    if touched.is_empty() {
        outcome.state = VerificationAxis::Pass;
        outcome.summary = "The change touches no area this rule briefs.".into();
        outcome.evidence.push(EvidenceRef {
            system: "whetstone_brief".into(),
            locator: "no matching change".into(),
            digest: None,
        });
        return finish(context, outcome, None, json!({"touched": []}));
    }
    let head = head_commit(context.project_root);
    let covering = context.briefs.iter().find(|brief| {
        let area_covers =
            touched.iter().any(|path| {
                crate::domain::entry_point_matches(brief.area.trim_start_matches("./"), path)
                    || rule.paths.iter().any(|pattern| pattern == &brief.area)
                    || context
                        .features
                        .get(&RecordId::new(brief.area.as_str()).unwrap_or_else(|_| {
                            RecordId::new("unknown.feature").expect("constant id")
                        }))
                        .is_some_and(|feature| touched.iter().any(|path| feature.covers_path(path)))
            });
        // Every skill the rule names must have run, not just one of them.
        let skills_ok = skills.is_empty()
            || skills.iter().all(|skill| {
                brief
                    .skills
                    .iter()
                    .any(|ran| ran.trim_start_matches('/') == skill.trim_start_matches('/'))
            });
        let current = brief.commit.is_none() || brief.commit == head;
        area_covers && skills_ok && current
    });
    match covering {
        Some(brief) => {
            outcome.state = VerificationAxis::Pass;
            outcome.summary = format!(
                "A brief for {} ({}) was recorded at {}.",
                brief.area,
                brief.skills.join(", "),
                brief.recorded_at
            );
            outcome.evidence.push(EvidenceRef {
                system: "whetstone_brief".into(),
                locator: format!("{}@{}", brief.area, brief.recorded_at),
                digest: None,
            });
        }
        None => {
            outcome = outcome.unknown(format!(
                "No brief covers {}: run pstack {} and record it with wh check --brief --area <feature or path>. Missing evidence is not a pass.",
                touched.iter().take(3).cloned().collect::<Vec<_>>().join(", "),
                if skills.is_empty() { "/how, /why and /blast-radius".to_string() } else { skills.join(", ") }
            ));
            outcome.failures = touched
                .iter()
                .take(MAX_FAILURES)
                .map(|path| ArtifactFailure {
                    location: path.clone(),
                    message: "changed without a recorded brief".into(),
                })
                .collect();
        }
    }
    finish(context, outcome, None, json!({"touched": touched}))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pass, flag and unavailable for every enforcer kind `run_gate`
    /// dispatches (question, review and drive have their own suites).
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
        fs::write(
            root.join("tokens.css"),
            ":root { --accent: #ff8800; --space: 4px; }\n",
        )
        .expect("tokens");
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
        let features = BTreeMap::new();
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
        let briefs = vec![BriefView {
            area: "lib.rs".into(),
            recorded_at: "2026-09-28T00:00:00Z".into(),
            commit: None,
            skills: vec!["how".into(), "why".into()],
        }];
        let changed = vec!["lib.rs".to_string()];
        let run = |enforcer: serde_json::Value,
                   set: &FileSet,
                   base: Option<&str>,
                   briefs: &[BriefView]| {
            let context = GateContext {
                project_root: root,
                run_dir: &run_dir,
                run_id: "case",
                features: &features,
                doctor: None,
                timeout: Duration::from_secs(20),
                change_steps: None,
                files: set,
                changed: &changed,
                surface_base: base,
                briefs,
            };
            run_gate(&context, &id, &rule(enforcer)).state
        };
        let missing = "whetstone-no-such-program-7f3a";
        let clean = files(&[("lib.rs", "pub fn run(a: u8) -> u8 { a }\n")]);
        let (pass, fail, unknown) = (
            VerificationAxis::Pass,
            VerificationAxis::Fail,
            VerificationAxis::Unknown,
        );
        let cases: Vec<(&str, VerificationAxis, VerificationAxis)> = vec![
            (
                "ast",
                run(
                    json!({"kind": "ast", "query": "(unsafe_block) @match", "language": "rust"}),
                    &clean,
                    None,
                    &[],
                ),
                run(
                    json!({"kind": "ast", "query": "(unsafe_block) @match", "language": "rust"}),
                    &files(&[("lib.rs", "fn f(p: *const u8) -> u8 { unsafe { *p } }\n")]),
                    None,
                    &[],
                ),
            ),
            (
                "lint",
                run(
                    json!({"kind": "lint", "tool": "echo clean", "code": "F401"}),
                    &clean,
                    None,
                    &[],
                ),
                run(
                    json!({"kind": "lint", "tool": "echo lib.rs:1:1: F401 unused", "code": "F401"}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "formatter",
                run(
                    json!({"kind": "formatter", "tool": "true"}),
                    &clean,
                    None,
                    &[],
                ),
                run(
                    json!({"kind": "formatter", "tool": "false"}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "test",
                run(
                    json!({"kind": "test", "command": "true"}),
                    &clean,
                    None,
                    &[],
                ),
                run(
                    json!({"kind": "test", "command": "false"}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "validator",
                run(
                    json!({"kind": "validator", "command": "true"}),
                    &clean,
                    None,
                    &[],
                ),
                run(
                    json!({"kind": "validator", "command": "false"}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "design_tokens",
                run(
                    json!({"kind": "design_tokens", "tokens": "tokens.css"}),
                    &files(&[("a.css", ".a { color: var(--accent); }\n")]),
                    None,
                    &[],
                ),
                run(
                    json!({"kind": "design_tokens", "tokens": "tokens.css"}),
                    &files(&[("a.css", ".a { color: #123456; }\n")]),
                    None,
                    &[],
                ),
            ),
            (
                "public_surface",
                run(json!({"kind": "public_surface"}), &clean, Some("HEAD"), &[]),
                run(
                    json!({"kind": "public_surface"}),
                    &files(&[("lib.rs", "pub fn run(b: u16) -> u8 { 0 }\n")]),
                    Some("HEAD"),
                    &[],
                ),
            ),
            (
                "brief",
                run(
                    json!({"kind": "brief", "skills": ["how", "why"]}),
                    &clean,
                    None,
                    &briefs,
                ),
                run(
                    json!({"kind": "brief", "skills": ["how", "why", "blast-radius"]}),
                    &clean,
                    None,
                    &briefs,
                ),
            ),
        ];
        for (kind, passed, flagged) in &cases {
            assert_eq!(*passed, pass, "{kind} passes a clean change");
            assert_ne!(*flagged, pass, "{kind} never passes a breaking change");
        }
        // Flagged means fail, except a brief that is missing a skill, which is
        // missing evidence (unknown), never a pass.
        for (kind, _, flagged) in &cases {
            if *kind != "brief" {
                assert_eq!(*flagged, fail, "{kind} flags a breaking change");
            }
        }
        // Unavailable: the enforcer cannot run, and says so as unknown.
        let unavailable = [
            (
                "ast",
                run(
                    json!({"kind": "ast", "query": "(not a query", "language": "rust"}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "lint",
                run(
                    json!({"kind": "lint", "tool": missing, "code": "F401"}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "formatter",
                run(
                    json!({"kind": "formatter", "tool": missing}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "test",
                run(
                    json!({"kind": "test", "command": missing}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "validator",
                run(
                    json!({"kind": "validator", "command": missing}),
                    &clean,
                    None,
                    &[],
                ),
            ),
            (
                "design_tokens",
                run(
                    json!({"kind": "design_tokens", "tokens": "missing-tokens.css"}),
                    &files(&[("a.css", ".a { color: red; }\n")]),
                    None,
                    &[],
                ),
            ),
            (
                "public_surface",
                run(json!({"kind": "public_surface"}), &clean, None, &[]),
            ),
            (
                "brief",
                run(
                    json!({"kind": "brief", "skills": ["how"]}),
                    &clean,
                    None,
                    &[],
                ),
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
    fn programs_resolve_inside_the_repository_or_on_path_only() {
        let temp = tempfile::tempdir().expect("temp");
        assert!(resolve_program(temp.path(), "../escape.sh").is_err());
        assert!(resolve_program(temp.path(), "definitely-not-a-program-xyz").is_err());
        assert!(resolve_program(temp.path(), "sh").is_ok());
    }
}
