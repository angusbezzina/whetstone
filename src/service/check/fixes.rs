//! Findings carry the tool's own fix: for a formatter or a lint rule whose
//! tool can rewrite files, the tool runs in its fix mode on a throwaway copy
//! of the repository, and the unified diff it produced is attached to the
//! rule's findings as `fix`. The working tree is never written; the diff
//! applies with `git apply` at the repository root.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::domain::Enforcer;
use crate::gates::{self, lint, FileSet};

use super::findings::SweepFinding;
use super::SelectedRule;

/// The largest diff a finding carries; a larger one is left out, since a
/// truncated diff would not apply.
pub(crate) const MAX_FIX_BYTES: usize = 64 * 1024;
const FIX_TIMEOUT: Duration = Duration::from_secs(300);
const OUTPUT_LIMIT: usize = 2 * 1024 * 1024;

/// The command that rewrites what an enforcer only checks, if its tool has one.
pub(crate) fn fix_command(enforcer: &Enforcer) -> Option<Vec<Vec<String>>> {
    match enforcer {
        Enforcer::Formatter { tool } => {
            let mut sequences = gates::parse_command(tool).ok()?;
            let argv = sequences.last_mut()?;
            let program = program_name(argv);
            if program == "prettier" || argv.iter().any(|arg| arg == "prettier") {
                replace_flag(argv, "--check", "--write")?;
            } else if program == "gofmt" {
                replace_flag(argv, "-l", "-w")?;
            } else if argv.iter().any(|arg| arg == "biome") || program == "biome" {
                if !argv.iter().any(|arg| arg == "--write") {
                    argv.push("--write".into());
                }
            } else {
                let before = argv.len();
                argv.retain(|arg| arg != "--check");
                if argv.len() == before {
                    return None;
                }
                // `cargo fmt -- --check` leaves a dangling `--`.
                if argv.last().is_some_and(|arg| arg == "--") {
                    argv.pop();
                }
            }
            Some(sequences)
        }
        Enforcer::Lint { tool, code } => {
            let mut sequences = gates::parse_command(tool).ok()?;
            let argv = sequences.last_mut()?;
            match lint::structured(argv).0 {
                lint::Reporter::Ruff => {
                    argv.extend(["--fix".into(), "--select".into(), code.clone()]);
                }
                lint::Reporter::Cargo if argv.iter().any(|arg| arg == "clippy") => {
                    let at = argv
                        .iter()
                        .position(|arg| arg == "--")
                        .unwrap_or(argv.len());
                    argv.truncate(at);
                    argv.extend(
                        [
                            "--fix",
                            "--allow-dirty",
                            "--allow-staged",
                            "--allow-no-vcs",
                            "--",
                            "-A",
                            "warnings",
                            "-W",
                        ]
                        .map(String::from),
                    );
                    argv.push(code.clone());
                }
                lint::Reporter::Eslint => argv.push("--fix".into()),
                lint::Reporter::Biome => argv.push("--write".into()),
                _ => return None,
            }
            Some(sequences)
        }
        _ => None,
    }
}

fn program_name(argv: &[String]) -> String {
    argv.first()
        .map(|program| program.rsplit('/').next().unwrap_or(program).to_string())
        .unwrap_or_default()
}

fn replace_flag(argv: &mut [String], from: &str, to: &str) -> Option<()> {
    let at = argv.iter().position(|arg| arg == from)?;
    argv[at] = to.into();
    Some(())
}

/// Attach each fixable rule's diff to its findings: the part for the
/// finding's file, or the whole diff for a repository-wide finding.
pub(crate) fn attach(project: &Path, selected: &[SelectedRule], found: &mut [SweepFinding]) {
    let rules = selected
        .iter()
        .map(|selected| (selected.id.as_str().to_string(), &selected.rule))
        .collect::<BTreeMap<_, _>>();
    let mut diffs = BTreeMap::<String, Option<BTreeMap<String, String>>>::new();
    for finding in found.iter_mut() {
        let Some(rule) = rules.get(&finding.responsibility) else {
            continue;
        };
        let per_file = diffs
            .entry(finding.responsibility.clone())
            .or_insert_with(|| {
                fix_command(&rule.enforcer).and_then(|command| fix_diff(project, &command))
            });
        let Some(per_file) = per_file else {
            continue;
        };
        let diff = match per_file.get(&finding.subject) {
            Some(diff) => diff.clone(),
            None if finding.subject == "repository" => per_file.values().cloned().collect(),
            None => continue,
        };
        if !diff.is_empty() && diff.len() <= MAX_FIX_BYTES {
            finding.fix = Some(diff);
        }
    }
}

/// A private scratch directory, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Option<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path =
            std::env::temp_dir().join(format!("whetstone-fix-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path).ok()?;
        Some(Self(path))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Run the fixer on a copy of the repository and return its diff per file.
pub(crate) fn fix_diff(
    project: &Path,
    command: &[Vec<String>],
) -> Option<BTreeMap<String, String>> {
    let files = FileSet::repository(project).ok()?;
    let scratch = Scratch::new()?;
    let copy = scratch.0.as_path();
    for (path, bytes) in files.iter() {
        let target = copy.join(path);
        std::fs::create_dir_all(target.parent()?).ok()?;
        std::fs::write(&target, bytes).ok()?;
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(project.join(path)) {
            let _ = std::fs::set_permissions(&target, metadata.permissions());
        }
    }
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(copy)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .env("GIT_AUTHOR_NAME", "whetstone")
            .env("GIT_AUTHOR_EMAIL", "whetstone@localhost")
            .env("GIT_COMMITTER_NAME", "whetstone")
            .env("GIT_COMMITTER_EMAIL", "whetstone@localhost")
            .output()
            .ok()
            .filter(|output| output.status.success())
    };
    git(&["init", "--quiet"])?;
    git(&["add", "-A"])?;
    git(&["commit", "--quiet", "--no-verify", "-m", "before"])?;
    let environment = gates::gate_environment(&[]);
    for argv in command {
        let program = gates::resolve_program(copy, argv.first()?).ok()?;
        let run = crate::execution::run_bounded(
            &program,
            &argv[1..],
            copy,
            &environment,
            FIX_TIMEOUT,
            OUTPUT_LIMIT,
            OUTPUT_LIMIT,
        )
        .ok()?;
        if run.timed_out {
            return None;
        }
    }
    // Only files the repository already tracks: a fixer's caches and build
    // output are not part of the fix.
    let output = git(&["diff", "--no-color", "--no-ext-diff"])?;
    let text = String::from_utf8(output.stdout).ok()?;
    let mut per_file = BTreeMap::new();
    let mut current: Option<(String, String)> = None;
    for line in text.split_inclusive('\n') {
        if let Some(header) = line.strip_prefix("diff --git a/") {
            if let Some((path, diff)) = current.take() {
                per_file.insert(path, diff);
            }
            let path = header
                .split(" b/")
                .next()
                .unwrap_or_default()
                .trim()
                .to_string();
            current = Some((path, String::new()));
        }
        if let Some((_, diff)) = current.as_mut() {
            diff.push_str(line);
        }
    }
    if let Some((path, diff)) = current {
        per_file.insert(path, diff);
    }
    Some(per_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(enforcer: serde_json::Value) -> Option<Vec<Vec<String>>> {
        fix_command(&serde_json::from_value(enforcer).expect("enforcer"))
    }

    #[test]
    fn check_commands_become_their_tools_fix_commands() {
        let line = |sequences: Option<Vec<Vec<String>>>| {
            sequences.map(|sequences| {
                sequences
                    .iter()
                    .map(|argv| argv.join(" "))
                    .collect::<Vec<_>>()
                    .join(" && ")
            })
        };
        assert_eq!(
            line(command(
                serde_json::json!({"kind": "formatter", "tool": "cargo fmt --all -- --check"})
            )),
            Some("cargo fmt --all".into())
        );
        assert_eq!(
            line(command(
                serde_json::json!({"kind": "formatter", "tool": "ruff format --check ."})
            )),
            Some("ruff format .".into())
        );
        assert_eq!(
            line(command(
                serde_json::json!({"kind": "formatter", "tool": "npx prettier --check ."})
            )),
            Some("npx prettier --write .".into())
        );
        assert_eq!(
            line(command(
                serde_json::json!({"kind": "lint", "tool": "ruff check .", "code": "F401"})
            )),
            Some("ruff check . --fix --select F401".into())
        );
        assert_eq!(
            line(command(serde_json::json!({"kind": "lint", "tool": "cargo clippy --all-targets", "code": "clippy::needless_return"}))),
            Some("cargo clippy --all-targets --fix --allow-dirty --allow-staged --allow-no-vcs -- -A warnings -W clippy::needless_return".into())
        );
        assert_eq!(
            command(serde_json::json!({"kind": "lint", "tool": "mylint .", "code": "W1"})),
            None
        );
        assert_eq!(
            command(serde_json::json!({"kind": "formatter", "tool": "sh format-check.sh"})),
            None
        );
    }
}
