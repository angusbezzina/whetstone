//! The rule probe: before a rule is written into the rulebook, prove its
//! enforcer runs and matches on this repository. Nothing in the working tree
//! changes; the probe's own evidence goes under the private state root.
//!
//! A probe answers four questions: did the enforcer run (and if not, the
//! tool's error), how many files are in scope, how many places it matched,
//! and where (up to ten samples). A rule whose paths cover no file, whose
//! query does not parse, whose lint code the tool does not know or whose
//! command cannot start is "not validatable here".

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::domain::{Enforcer, RecordId, Rule, SCRIPTS_DIR};
use crate::gates::{self, lint, ArtifactFailure, FileSet, GateContext, GateState, RunMode};
use crate::layout::ProjectLayout;

/// The most samples a probe shows.
pub const MAX_SAMPLES: usize = 10;
/// A probe runs the enforcer once; a command is bounded tighter than a gate.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(300);

/// What one probe found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProbeReport {
    /// The enforcer kind probed.
    pub enforcer: &'static str,
    /// The exact query, tool or command.
    pub target: String,
    /// Whether the enforcer ran to a result.
    pub ran: bool,
    /// Whether the rule can be checked on this repository.
    pub validatable: bool,
    /// Why not, when it cannot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Repository files the rule's paths cover.
    pub files_in_scope: usize,
    /// Places the enforcer flagged (a command that failed with no location
    /// counts once).
    pub matched: usize,
    /// Up to ten of them, with file and line where the enforcer gave one.
    pub samples: Vec<ArtifactFailure>,
    /// How the lint code was confirmed, for a lint rule.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_check: Option<String>,
    /// The command's exit code, for a command enforcer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// One sentence for a person.
    pub summary: String,
}

impl ProbeReport {
    fn new(rule: &Rule, files_in_scope: usize) -> Self {
        let (_, target) = gates::mechanism_label(rule);
        Self {
            enforcer: rule.enforcer.kind(),
            target,
            ran: false,
            validatable: false,
            reason: None,
            files_in_scope,
            matched: 0,
            samples: Vec::new(),
            code_check: None,
            exit_code: None,
            summary: String::new(),
        }
    }

    fn refuse(mut self, reason: impl Into<String>) -> Self {
        let reason = reason.into();
        self.validatable = false;
        self.summary = format!("Not validatable here: {reason}");
        self.reason = Some(reason);
        self
    }

    fn finish(mut self, failures: Vec<ArtifactFailure>, total: usize) -> Self {
        self.ran = true;
        self.validatable = true;
        self.matched = total;
        self.samples = failures.into_iter().take(MAX_SAMPLES).collect();
        self.summary = if self.matched == 0 {
            format!(
                "The {} enforcer ran over {} file(s) in scope and matched nothing: the rule holds today.",
                self.enforcer, self.files_in_scope
            )
        } else {
            format!(
                "The {} enforcer ran over {} file(s) in scope and matched {} place(s); the samples show where.",
                self.enforcer, self.files_in_scope, self.matched
            )
        };
        self
    }
}

/// The repository's files a check reads: tracked and untracked, without
/// Whetstone's own wiring.
fn repository_files(root: &Path) -> Result<FileSet, String> {
    FileSet::repository(root).map(|files| files.retain(|path| !crate::wire::is_generated(path)))
}

/// Probe `rule` (written as `id`) on the repository at `layout`.
pub fn probe(layout: &ProjectLayout, id: &RecordId, rule: &Rule) -> ProbeReport {
    let root = layout.project_root();
    let files = match repository_files(root) {
        Ok(files) => files,
        Err(error) => {
            return ProbeReport::new(rule, 0).refuse(format!(
                "the repository's files could not be listed: {error}"
            ))
        }
    };
    let in_scope = files.paths().filter(|path| rule.applies_to(path)).count();
    let report = ProbeReport::new(rule, in_scope);
    // Paths that cover nothing aim the rule at nothing. A command with no
    // paths is about the whole repository, files or not.
    if in_scope == 0 && !rule.paths.is_empty() {
        return report.refuse(format!(
            "no file in the repository matches the rule's paths ({})",
            rule.paths.join(", ")
        ));
    }
    match &rule.enforcer {
        Enforcer::Ast { query, language } => match gates::ast::run_query(
            &files,
            query,
            language.as_deref(),
            |path| rule.applies_to(path),
            usize::MAX,
        ) {
            Err(error) => report.refuse(format!("the AST query does not parse: {error}")),
            Ok(run) if run.files_checked == 0 => report.refuse(format!(
                "no {} file is in scope, so the query would check nothing",
                run.languages.join(" or ")
            )),
            Ok(run) => {
                let total = run.failures.len();
                let mut report = report;
                report.files_in_scope = run.files_checked;
                report.finish(run.failures, total)
            }
        },
        Enforcer::Lint { tool, code } => {
            let argv = match gates::parse_command(tool) {
                Ok(sequences) => sequences.last().cloned().unwrap_or_default(),
                Err(error) => {
                    return report.refuse(format!("the lint command is not usable: {error}"))
                }
            };
            match known_code(root, &argv, code) {
                Ok(how) => {
                    let mut report = run_command(layout, &files, id, rule, report);
                    report.code_check = Some(how);
                    report
                }
                Err(reason) => report.refuse(reason),
            }
        }
        Enforcer::Formatter { tool: command }
        | Enforcer::Test { command }
        | Enforcer::Validator { command } => match gates::parse_command(command) {
            Ok(sequences) => match missing_script(root, &sequences) {
                Some(script) => report.refuse(format!(
                    "the script {script} does not exist; write it first (see {})",
                    crate::scripts::README
                )),
                None => run_command(layout, &files, id, rule, report),
            },
            Err(error) => report.refuse(format!("the command is not usable: {error}")),
        },
    }
}

/// A script under `whetstone/checks/` that a command names but that is not
/// there: running it would report the shell's error as a finding.
fn missing_script(root: &Path, sequences: &[Vec<String>]) -> Option<String> {
    sequences
        .iter()
        .flatten()
        .map(|token| token.trim_start_matches("./"))
        .find(|token| token.starts_with(SCRIPTS_DIR) && !root.join(token).is_file())
        .map(str::to_owned)
}

/// Run a command enforcer once over the whole repository, as the gate does.
fn run_command(
    layout: &ProjectLayout,
    files: &FileSet,
    id: &RecordId,
    rule: &Rule,
    report: ProbeReport,
) -> ProbeReport {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let run_id = format!(
        "probe-{}",
        &format!(
            "{:x}",
            Sha256::digest(
                format!(
                    "{}\0{}\0{}\0{nanos}",
                    id.as_str(),
                    report.target,
                    std::process::id()
                )
                .as_bytes()
            )
        )[..12]
    );
    let run_dir = gates::evidence_root(layout).join(&run_id);
    let context = GateContext {
        project_root: layout.project_root(),
        run_dir: &run_dir,
        run_id: &run_id,
        timeout: PROBE_TIMEOUT,
        files,
        changed: &[],
        mode: RunMode::All,
        base: None,
    };
    let outcome = gates::run_gate(&context, id, rule);
    let mut report = report;
    report.exit_code = outcome.exit_code;
    match outcome.state {
        GateState::Pass => report.finish(Vec::new(), 0),
        GateState::Fail => {
            let total = outcome.failures.len();
            report.finish(outcome.failures, total)
        }
        GateState::Unknown => report.refuse(format!(
            "the enforcer did not run to a result: {}",
            outcome.summary
        )),
    }
}

/// Confirm a lint code with the tool where it can say, and by its shape
/// where it cannot; an unknown code is refused by name.
pub fn known_code(project_root: &Path, argv: &[String], code: &str) -> Result<String, String> {
    let code = code.trim();
    if code.is_empty() || code.contains(char::is_whitespace) {
        return Err("a lint code is one word, for example E501 or clippy::unwrap_used".into());
    }
    let (reporter, _) = lint::structured(argv);
    let shaped = |valid: bool, tool: &str, example: &str| {
        if valid {
            Ok(())
        } else {
            Err(format!(
                "{code} is not a {tool} code (they look like {example})"
            ))
        }
    };
    let ask = |program: &str, args: &[&str]| -> Option<bool> {
        let program = gates::resolve_program(project_root, program).ok()?;
        let output = std::process::Command::new(program)
            .args(args)
            .current_dir(project_root)
            .output()
            .ok()?;
        Some(output.status.success())
    };
    match reporter {
        lint::Reporter::Ruff => {
            let letters = code.chars().take_while(char::is_ascii_uppercase).count();
            shaped(
                (1..=4).contains(&letters)
                    && code.len() > letters
                    && code[letters..]
                        .chars()
                        .all(|character| character.is_ascii_digit()),
                "ruff",
                "E501 or PLR0913",
            )?;
            match ask("ruff", &["rule", code]) {
                Some(true) => Ok(format!("ruff rule {code} knows it")),
                Some(false) => Err(format!("ruff does not know the code {code}")),
                None => Ok("ruff is not installed here; the code's shape was checked".into()),
            }
        }
        lint::Reporter::Cargo => {
            let name = code.trim_start_matches("clippy::");
            shaped(
                !name.is_empty()
                    && name.chars().all(|character| {
                        character.is_ascii_lowercase()
                            || character.is_ascii_digit()
                            || character == '_'
                    }),
                "clippy or rustc lint",
                "clippy::unwrap_used or unused_imports",
            )?;
            if code.starts_with("clippy::") {
                match ask("cargo", &["clippy", "--explain", name]) {
                    Some(true) => Ok(format!("cargo clippy --explain {name} knows it")),
                    Some(false) => Err(format!("clippy does not know the lint {name}")),
                    None => Ok("cargo is not installed here; the code's shape was checked".into()),
                }
            } else {
                Ok("a rustc lint name; its shape was checked".into())
            }
        }
        lint::Reporter::Biome => {
            shaped(
                code.chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '/'),
                "biome",
                "lint/suspicious/noDebugger or noDebugger",
            )?;
            Ok("biome codes are checked by shape".into())
        }
        lint::Reporter::Eslint => {
            shaped(
                code.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '-' | '/' | '@' | '_')
                }),
                "eslint",
                "no-console or @typescript-eslint/no-explicit-any",
            )?;
            Ok("eslint rule ids are checked by shape".into())
        }
        lint::Reporter::Text => Ok(
            "the linter has no structured report; the code is searched for in its output".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn lint_codes_are_refused_by_shape_before_anything_runs() {
        let root = std::env::temp_dir();
        assert!(known_code(&root, &argv("ruff check ."), "not a code").is_err());
        assert!(known_code(&root, &argv("ruff check ."), "unwrap_used").is_err());
        assert!(known_code(&root, &argv("cargo clippy"), "E501").is_err());
        assert!(known_code(&root, &argv("eslint ."), "no console").is_err());
        assert!(known_code(&root, &argv("biome lint ."), "noDebugger").is_ok());
        assert!(known_code(&root, &argv("mylint ."), "W100").is_ok());
    }
}
