//! Where the rulebook is thin or noisy.
//!
//! Coverage: which parts of the repository (by top-level directory and
//! language) no rule in force reads, and which rules read no file at all.
//! A rule reads a file when its paths cover it and its enforcer reads that
//! kind of file: an AST query its language's files, a command its tool's
//! language when the rule names no paths, anything else every file it covers.
//!
//! Outcomes: every `wh check --sweep --format findings` run is remembered
//! (when, and which rule raised which fingerprint) in the private state, and
//! a runner can report back what happened to each finding with
//! `wh pull --outcomes <file>` ({fingerprint, outcome: fixed | dismissed |
//! stalled}). Per rule that gives flags per run and a dismissal rate, so
//! priorities are tuned from evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{Enforcer, Rule};
use crate::storage::ProjectLayout;

/// Sweep runs remembered; older runs fall off.
pub const MAX_SWEEP_RUNS: usize = 100;
const SWEEPS_FILE: &str = "sweeps.jsonl";
const OUTCOMES_FILE: &str = "outcomes.json";
pub const OUTCOMES: [&str; 3] = ["fixed", "dismissed", "stalled"];

fn language_of(path: &str) -> &'static str {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(crate::types::source_language_for_extension)
        .unwrap_or("other")
}

fn same_language(file: &str, rule: &str) -> bool {
    file == rule
        || (rule == "typescript" && file == "javascript")
        || (rule == "javascript" && file == "typescript")
}

/// The languages a command's tool reads, when it is one Whetstone knows.
fn tool_languages(command: &str) -> Option<&'static [&'static str]> {
    let program = command
        .split_whitespace()
        .map(|token| token.rsplit('/').next().unwrap_or(token))
        .find(|token| !token.contains('='))?;
    Some(match program {
        "cargo" | "rustfmt" | "clippy-driver" => &["rust"],
        "ruff" | "black" | "pytest" | "python" | "python3" | "mypy" | "flake8" | "tox" | "uv" => {
            &["python"]
        }
        "npm" | "npx" | "pnpm" | "yarn" | "bun" | "node" | "deno" | "eslint" | "prettier"
        | "biome" | "tsc" | "vitest" | "jest" => &["typescript", "javascript"],
        _ => return None,
    })
}

/// Whether `rule` reads the repository file at `path`.
pub fn reads(rule: &Rule, path: &str) -> bool {
    if !rule.applies_to(path) {
        return false;
    }
    let language = language_of(path);
    match &rule.enforcer {
        Enforcer::Ast {
            language: Some(wanted),
            ..
        } => same_language(language, wanted),
        Enforcer::Ast { language: None, .. } => {
            matches!(language, "rust" | "typescript" | "javascript" | "python")
        }
        Enforcer::Test { command }
        | Enforcer::Validator { command }
        | Enforcer::Formatter { tool: command }
        | Enforcer::Lint { tool: command, .. }
            if rule.paths.is_empty() && !rule.enforcer.is_script() =>
        {
            tool_languages(command).map_or(true, |languages| {
                languages
                    .iter()
                    .any(|wanted| same_language(language, wanted))
            })
        }
        Enforcer::Test { .. }
        | Enforcer::Validator { .. }
        | Enforcer::Formatter { .. }
        | Enforcer::Lint { .. } => true,
        Enforcer::DesignTokens { .. } => language == "css",
        Enforcer::PublicSurface { .. } => language != "other",
        // Held by a person, a question or a proof, not by reading files.
        Enforcer::Question { .. }
        | Enforcer::Review { .. }
        | Enforcer::Brief { .. }
        | Enforcer::Drive { .. } => false,
    }
}

/// Whether a rule is held by reading the repository's files at all.
fn mechanical(rule: &Rule) -> bool {
    !matches!(
        rule.enforcer,
        Enforcer::Question { .. }
            | Enforcer::Review { .. }
            | Enforcer::Brief { .. }
            | Enforcer::Drive { .. }
    )
}

/// Uncovered parts of the repository and rules that read nothing.
pub fn coverage(project_root: &Path, state: &AgreementState) -> Value {
    let files = crate::gates::FileSet::repository(project_root)
        .map(|files| {
            files
                .paths()
                .filter(|path| !crate::skill::is_generated(path))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let rules = crate::proof::rule_ids(state)
        .into_iter()
        .filter_map(|id| {
            let record = state.in_force(&id)?;
            let rule = record.body.rule_view()?.into_owned();
            Some((id.as_str().to_string(), rule))
        })
        .collect::<Vec<_>>();
    let mut groups = BTreeMap::<(String, &'static str), Vec<String>>::new();
    let mut covered = 0usize;
    let mut reads_any = BTreeSet::new();
    for path in &files {
        let mut read = false;
        for (id, rule) in &rules {
            if reads(rule, path) {
                read = true;
                reads_any.insert(id.as_str());
            }
        }
        if read {
            covered += 1;
            continue;
        }
        let directory = match path.split_once('/') {
            Some((top, _)) => format!("{top}/"),
            None => "(root)".to_string(),
        };
        groups
            .entry((directory, language_of(path)))
            .or_default()
            .push(path.clone());
    }
    let uncovered = groups
        .into_iter()
        .map(|((directory, language), paths)| {
            json!({
                "directory": directory,
                "language": language,
                "files": paths.len(),
                "sample": paths.iter().take(3).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let dead = rules
        .iter()
        .filter(|(id, rule)| mechanical(rule) && !reads_any.contains(id.as_str()))
        .map(|(id, rule)| {
            json!({
                "rule": id,
                "statement": rule.statement,
                "paths": rule.paths,
                "enforcer": rule.enforcer.kind(),
                "why": if rule.paths.is_empty() {
                    "its enforcer reads no file of a kind this repository has"
                } else {
                    "its paths match no file in the repository"
                },
            })
        })
        .collect::<Vec<_>>();
    json!({
        "files": files.len(),
        "covered": covered,
        "uncovered": uncovered,
        "dead_rules": dead,
    })
}

/// One remembered sweep run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SweepRun {
    pub at: String,
    pub findings: Vec<SweepEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SweepEntry {
    pub fingerprint: String,
    pub rule: String,
}

fn sweeps_path(layout: &ProjectLayout) -> PathBuf {
    layout.state_root().join(SWEEPS_FILE)
}

fn outcomes_path(layout: &ProjectLayout) -> PathBuf {
    layout.state_root().join(OUTCOMES_FILE)
}

/// Remember one sweep run, keeping the last [`MAX_SWEEP_RUNS`].
pub fn record_sweep(layout: &ProjectLayout, findings: Vec<SweepEntry>) -> std::io::Result<()> {
    let mut runs = sweep_runs(layout);
    runs.push(SweepRun {
        at: crate::service::utc_timestamp(),
        findings,
    });
    let keep = runs.len().saturating_sub(MAX_SWEEP_RUNS);
    std::fs::create_dir_all(layout.state_root())?;
    let mut text = String::new();
    for run in &runs[keep..] {
        text.push_str(&serde_json::to_string(run).unwrap_or_default());
        text.push('\n');
    }
    let path = sweeps_path(layout);
    let partial = path.with_extension("jsonl.partial");
    std::fs::File::create(&partial)?.write_all(text.as_bytes())?;
    std::fs::rename(partial, path)
}

pub fn sweep_runs(layout: &ProjectLayout) -> Vec<SweepRun> {
    std::fs::read_to_string(sweeps_path(layout))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// What a runner reported for each fingerprint, latest first wins.
pub fn outcomes(layout: &ProjectLayout) -> BTreeMap<String, Value> {
    std::fs::read_to_string(outcomes_path(layout))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Read a runner's outcomes file and remember each fingerprint's outcome.
pub fn import_outcomes(layout: &ProjectLayout, file: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("{} could not be read: {error}", file.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not JSON: {error}", file.display()))?;
    let items = match &value {
        Value::Array(items) => items.clone(),
        Value::Object(object) => object
            .get("outcomes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut known = outcomes(layout);
    let mut recorded = 0;
    for item in items {
        let fingerprint = item
            .get("fingerprint")
            .and_then(Value::as_str)
            .ok_or("every outcome names its fingerprint")?;
        let outcome = item
            .get("outcome")
            .and_then(Value::as_str)
            .filter(|outcome| OUTCOMES.contains(outcome))
            .ok_or_else(|| {
                format!(
                    "the outcome of {fingerprint} must be one of {}",
                    OUTCOMES.join(", ")
                )
            })?;
        known.insert(
            fingerprint.to_string(),
            json!({"outcome": outcome, "at": crate::service::utc_timestamp()}),
        );
        recorded += 1;
    }
    std::fs::create_dir_all(layout.state_root()).map_err(|error| error.to_string())?;
    std::fs::write(
        outcomes_path(layout),
        serde_json::to_vec_pretty(&known).unwrap_or_default(),
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({"recorded": recorded, "known": known.len()}))
}

/// Per rule: sweep runs, flags in the last run and over all runs, and what
/// runners reported (with the dismissal rate).
pub fn rule_stats(layout: &ProjectLayout) -> (usize, BTreeMap<String, Value>) {
    let runs = sweep_runs(layout);
    let outcomes = outcomes(layout);
    let mut rule_of = BTreeMap::<&str, &str>::new();
    let mut flagged_runs = BTreeMap::<&str, usize>::new();
    let mut total = BTreeMap::<&str, usize>::new();
    for run in &runs {
        let mut seen = BTreeSet::new();
        for entry in &run.findings {
            rule_of.insert(entry.fingerprint.as_str(), entry.rule.as_str());
            *total.entry(entry.rule.as_str()).or_default() += 1;
            if seen.insert(entry.rule.as_str()) {
                *flagged_runs.entry(entry.rule.as_str()).or_default() += 1;
            }
        }
    }
    let mut last = BTreeMap::<&str, usize>::new();
    if let Some(run) = runs.last() {
        for entry in &run.findings {
            *last.entry(entry.rule.as_str()).or_default() += 1;
        }
    }
    let mut reported = BTreeMap::<&str, BTreeMap<&str, usize>>::new();
    for (fingerprint, outcome) in &outcomes {
        let Some(rule) = rule_of.get(fingerprint.as_str()) else {
            continue;
        };
        let Some(outcome) = outcome.get("outcome").and_then(Value::as_str) else {
            continue;
        };
        let outcome = OUTCOMES
            .iter()
            .find(|known| **known == outcome)
            .copied()
            .unwrap_or("stalled");
        *reported
            .entry(rule)
            .or_default()
            .entry(outcome)
            .or_default() += 1;
    }
    let mut stats = BTreeMap::new();
    for rule in total
        .keys()
        .chain(reported.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let counts = reported.get(rule).cloned().unwrap_or_default();
        let fixed = counts.get("fixed").copied().unwrap_or(0);
        let dismissed = counts.get("dismissed").copied().unwrap_or(0);
        let stalled = counts.get("stalled").copied().unwrap_or(0);
        let decided = fixed + dismissed + stalled;
        stats.insert(
            rule.to_string(),
            json!({
                "runs_flagged": flagged_runs.get(rule).copied().unwrap_or(0),
                "flags_last_run": last.get(rule).copied().unwrap_or(0),
                "flags_total": total.get(rule).copied().unwrap_or(0),
                "fixed": fixed,
                "dismissed": dismissed,
                "stalled": stalled,
                "dismissal_rate": (decided > 0).then(|| (dismissed as f64 / decided as f64 * 100.0).round() / 100.0),
            }),
        );
    }
    (runs.len(), stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(enforcer: serde_json::Value, paths: &[&str]) -> Rule {
        serde_json::from_value(json!({
            "schema": "whetstone.rule.v2",
            "statement": "s",
            "rationale": "r",
            "strength": "must",
            "enforcer": enforcer,
            "source": {"kind": "owner"},
            "paths": paths,
        }))
        .expect("rule")
    }

    #[test]
    fn a_rule_reads_what_its_paths_and_enforcer_cover() {
        let rust_ast = rule(
            json!({"kind": "ast", "query": "(x) @match", "language": "rust"}),
            &[],
        );
        assert!(reads(&rust_ast, "src/main.rs"));
        assert!(!reads(&rust_ast, "web/app.ts"));
        let cargo = rule(json!({"kind": "test", "command": "cargo test"}), &[]);
        assert!(reads(&cargo, "src/main.rs"));
        assert!(!reads(&cargo, "docs/guide.md"));
        let scoped = rule(
            json!({"kind": "test", "command": "cargo test"}),
            &["docs/**"],
        );
        assert!(reads(&scoped, "docs/guide.md"));
        let script = rule(
            json!({"kind": "validator", "command": "sh whetstone/checks/x.sh"}),
            &[],
        );
        assert!(reads(&script, "docs/guide.md"));
        let unknown = rule(json!({"kind": "validator", "command": "make lint"}), &[]);
        assert!(reads(&unknown, "docs/guide.md"));
        let review = rule(
            json!({"kind": "review", "reviewer": {"by": "interrogate"}}),
            &[],
        );
        assert!(!reads(&review, "src/main.rs"));
    }
}
