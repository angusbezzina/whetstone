//! Execution of in-force gates (Standard records) for `wh check`.
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
    ContentDigest, Enforcement, EvidenceRef, Feature, RecordId, Standard, StandardStrength,
    VerificationAxis,
};
use crate::projection::{ArtifactFailure, GateArtifact, EVIDENCE_SYSTEM};
use crate::storage::ProjectLayout;

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

/// Everything a gate run needs, prepared once per `wh check`.
pub struct GateContext<'a> {
    pub project_root: &'a Path,
    pub run_dir: &'a Path,
    pub run_id: &'a str,
    pub features: &'a BTreeMap<RecordId, Feature>,
    pub doctor: Option<&'a DoctorResult>,
    pub timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateOutcome {
    pub id: String,
    pub name: String,
    pub strength: StandardStrength,
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
}

impl GateOutcome {
    fn new(id: &RecordId, standard: &Standard) -> Self {
        let (mechanism, command) = crate::projection::mechanism_label(standard);
        Self {
            id: id.as_str().into(),
            name: standard.statement.clone(),
            strength: standard.strength,
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
        }
    }

    fn unknown(mut self, summary: impl Into<String>) -> Self {
        self.state = VerificationAxis::Unknown;
        self.summary = summary.into();
        self
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

pub fn run_gate(context: &GateContext<'_>, id: &RecordId, standard: &Standard) -> GateOutcome {
    let outcome = GateOutcome::new(id, standard);
    match &standard.enforcement {
        Enforcement::Test { command_ref }
        | Enforcement::Validator { command_ref }
        | Enforcement::Formatter { tool: command_ref } => {
            run_command_gate(context, outcome, command_ref, None)
        }
        Enforcement::LintProxy { tool, code } => {
            run_command_gate(context, outcome, tool, Some(code.as_str()))
        }
        Enforcement::Ast { query } => run_ast_gate(context, outcome, query),
        Enforcement::Drive { feature } => run_drive_gate(context, outcome, feature),
    }
}

fn run_command_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    command: &str,
    lint_code: Option<&str>,
) -> GateOutcome {
    let sequences = match parse_command(command) {
        Ok(sequences) => sequences,
        Err(error) => {
            let outcome = outcome.unknown(error);
            return finish(context, outcome, None, json!({"parse_error": true}));
        }
    };
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
    } else if let Some(code) = lint_code {
        if run.output.contains(code) {
            outcome.state = VerificationAxis::Fail;
            outcome.failures = extract_failures(context.project_root, &run.output)
                .into_iter()
                .filter(|failure| failure.message.contains(code))
                .collect();
            outcome.summary = format!("The linter reported {code}.");
        } else if run.exit_code == Some(0) {
            outcome.state = VerificationAxis::Pass;
            outcome.summary = format!("The linter ran cleanly without {code}.");
        } else {
            outcome = outcome.unknown(format!(
                "The linter exited {:?} without reporting {code}; the result is unknown.",
                run.exit_code
            ));
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

fn run_ast_gate(context: &GateContext<'_>, mut outcome: GateOutcome, query: &str) -> GateOutcome {
    let scan_paths = vec![context.project_root.to_path_buf()];
    let filter = vec![query.to_string()];
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
            "No scanner rule named {query} applied; bind the gate to an existing rule in whetstone/rules."
        ));
    } else if violations > 0 {
        outcome.state = VerificationAxis::Fail;
        outcome.summary = format!("The AST query reported {violations} violation(s).");
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
        outcome.summary = format!("The AST query applied {rules} rule(s) without violations.");
    }
    let log = serde_json::to_string_pretty(&result).unwrap_or_default();
    finish(context, outcome, Some(&log), json!({"query": query}))
}

/// Run the project driver's read-only health check.
pub fn run_doctor(project_root: &Path, run_dir: &Path) -> DoctorResult {
    let Some(driver) = driver_path(project_root) else {
        return DoctorResult {
            ok: false,
            detail: format!("No verification driver exists at {DRIVER_RELATIVE}."),
        };
    };
    let node = match resolve_program(project_root, "node") {
        Ok(node) => node,
        Err(error) => {
            return DoctorResult {
                ok: false,
                detail: error,
            }
        }
    };
    let environment = gate_environment(&[("WH_EVIDENCE_DIR", run_dir.display().to_string())]);
    match crate::execution::run_bounded(
        &node,
        &[driver, "doctor".into(), "--json".into()],
        project_root,
        &environment,
        Duration::from_secs(60),
        OUTPUT_LIMIT,
        OUTPUT_LIMIT,
    ) {
        Ok(result) => {
            let parsed = result
                .stdout
                .text
                .lines()
                .rev()
                .find_map(|line| serde_json::from_str::<Value>(line).ok());
            let ok = result.status.success()
                && parsed
                    .as_ref()
                    .and_then(|value| value.get("ok"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            DoctorResult {
                ok,
                detail: parsed
                    .as_ref()
                    .and_then(|value| value.get("detail"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| last_meaningful_line(&result.stderr.text)),
            }
        }
        Err(error) => DoctorResult {
            ok: false,
            detail: format!("The driver could not start: {error}"),
        },
    }
}

fn run_drive_gate(
    context: &GateContext<'_>,
    mut outcome: GateOutcome,
    feature_id: &RecordId,
) -> GateOutcome {
    let Some(feature) = context.features.get(feature_id) else {
        let outcome = outcome.unknown(format!(
            "Feature {} is not in force; accept it before it can be proven.",
            feature_id.as_str()
        ));
        return finish(context, outcome, None, json!({}));
    };
    if feature.drive_steps.is_empty() {
        let outcome = outcome.unknown(format!(
            "Feature {} has no drive steps to prove it.",
            feature.name
        ));
        return finish(context, outcome, None, json!({}));
    }
    let Some(driver) = driver_path(context.project_root) else {
        let outcome = outcome.unknown(format!(
            "No verification driver exists at {DRIVER_RELATIVE}; run wh init --action wire."
        ));
        return finish(context, outcome, None, json!({}));
    };
    match context.doctor {
        Some(doctor) if doctor.ok => {}
        Some(doctor) => {
            let outcome = outcome.unknown(format!(
                "Doctor failed before driving: {}. A drive against an unhealthy instance is not evidence.",
                doctor.detail
            ));
            return finish(context, outcome, None, json!({"doctor": doctor}));
        }
        None => {
            let outcome = outcome.unknown("Doctor did not run before driving.");
            return finish(context, outcome, None, json!({}));
        }
    }
    let slug = slug(&outcome.id);
    let steps_file = format!("{slug}.steps.json");
    let steps = json!({"feature": feature_id.as_str(), "name": feature.name, "steps": feature.drive_steps, "proof": feature.proof});
    if let Err(error) = serde_json::to_vec_pretty(&steps)
        .map_err(|error| error.to_string())
        .and_then(|bytes| write_evidence(context, &steps_file, &bytes))
    {
        return outcome.unknown(format!("Evidence could not be written: {error}"));
    }
    let shots = context.run_dir.join(format!("{slug}.drive"));
    if let Err(error) = fs::create_dir_all(&shots) {
        return outcome.unknown(format!("Evidence could not be written: {error}"));
    }
    let node = match resolve_program(context.project_root, "node") {
        Ok(node) => node,
        Err(error) => return finish(context, outcome.unknown(error), None, json!({})),
    };
    let environment = gate_environment(&[
        ("WH_EVIDENCE_DIR", shots.display().to_string()),
        ("WH_RUN_ID", context.run_id.to_string()),
    ]);
    let result = crate::execution::run_bounded(
        &node,
        &[
            driver,
            "prove".into(),
            "--steps-file".into(),
            context.run_dir.join(&steps_file).display().to_string(),
            "--json".into(),
        ],
        context.project_root,
        &environment,
        context.timeout,
        OUTPUT_LIMIT,
        OUTPUT_LIMIT,
    );
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let outcome = outcome.unknown(format!("The driver could not start: {error}"));
            return finish(context, outcome, None, json!({}));
        }
    };
    outcome.exit_code = result.status.code();
    outcome.elapsed_ms = u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX);
    outcome.program = Some(node.display().to_string());
    outcome.program_digest = program_digest(&context.project_root.join(DRIVER_RELATIVE));
    let log = format!("{}{}", result.stdout.text, result.stderr.text);
    let report = result
        .stdout
        .text
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<Value>(line).ok());
    // Evidence the driver actually produced, bound by digest.
    let mut produced = Vec::new();
    if let Ok(entries) = fs::read_dir(&shots) {
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            if let Ok(bytes) = fs::read(&path) {
                if bytes.is_empty() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                produced.push(EvidenceRef {
                    system: EVIDENCE_SYSTEM.into(),
                    locator: format!("{}/{slug}.drive/{name}", context.run_id),
                    digest: ContentDigest::new(sha256_hex(&bytes)).ok(),
                });
            }
        }
    }
    outcome.evidence.extend(produced.clone());
    let reported = report
        .as_ref()
        .and_then(|report| report.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    outcome.failures = report
        .as_ref()
        .and_then(|report| report.get("failures"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .take(MAX_FAILURES)
                .map(|item| ArtifactFailure {
                    location: item
                        .get("location")
                        .and_then(Value::as_str)
                        .unwrap_or("drive step")
                        .to_string(),
                    message: item
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("step failed")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    if result.timed_out {
        outcome = outcome.unknown("The drive timed out; a timeout is not a result.");
    } else if reported == "pass" && result.status.success() {
        if produced.is_empty() {
            outcome = outcome.unknown(
                "The driver reported success but produced no evidence; that is not a pass.",
            );
        } else {
            outcome.state = VerificationAxis::Pass;
            outcome.summary = format!(
                "Drove {} step(s); proof: {}",
                feature.drive_steps.len(),
                feature.proof
            );
        }
    } else if reported == "fail" {
        outcome.state = VerificationAxis::Fail;
        if outcome.failures.is_empty() {
            outcome.failures.push(ArtifactFailure {
                location: "drive".into(),
                message: last_meaningful_line(&log),
            });
        }
        outcome.summary = format!("Driving {} did not reach the proof.", feature.name);
    } else {
        outcome = outcome.unknown(format!(
            "The driver returned no verdict: {}",
            last_meaningful_line(&log)
        ));
    }
    finish(
        context,
        outcome,
        Some(&log),
        json!({"feature": feature_id.as_str(), "report": report}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
