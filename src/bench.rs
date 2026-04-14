//! Rule-quality benchmark harness.
//!
//! The benchmark corpus lives at `benchmarks/<language>/<scenario>/`. Each
//! scenario directory contains:
//! - `meta.yaml`  — scenario name, language, rule ids under test
//! - `expected.json` — the set of violations the runner must report
//! - source files (under `src/` by convention)
//!
//! `wh bench run` executes `wh check` on each scenario and compares the
//! actual violations against `expected.json`. A scenario is considered
//! **passing** when the set of expected `(rule_id, relative_path, line)`
//! triples equals the set of actual triples — no false positives, no
//! missed detections. The aggregate report lists per-scenario precision,
//! recall, and F1 plus a rolled-up summary.
//!
//! `wh bench run --check` exits non-zero when any scenario regresses below
//! its threshold, enabling CI gating.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::check::{self, CheckOptions};

const DEFAULT_CORPUS: &str = "benchmarks";

pub struct BenchOptions<'a> {
    pub project_dir: &'a Path,
    pub corpus_dir: Option<&'a Path>,
    pub scenario_filter: Option<&'a str>,
    pub min_f1: f64,
}

pub fn run(opts: BenchOptions<'_>) -> Result<Value> {
    let corpus_root = opts
        .corpus_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| opts.project_dir.join(DEFAULT_CORPUS));

    if !corpus_root.exists() {
        return Ok(json!({
            "status": "no_corpus",
            "corpus_dir": corpus_root.display().to_string(),
            "scenarios": [],
            "summary": {"total": 0, "passing": 0, "failing": 0, "min_f1": opts.min_f1},
            "warnings": ["No benchmark corpus found. Create benchmarks/<language>/<scenario>/."],
            "next_command": "Create benchmarks/<language>/<scenario>/expected.json and source files.",
        }));
    }

    let scenarios = discover_scenarios(&corpus_root)?;
    if scenarios.is_empty() {
        return Ok(json!({
            "status": "empty_corpus",
            "corpus_dir": corpus_root.display().to_string(),
            "scenarios": [],
            "summary": {"total": 0, "passing": 0, "failing": 0, "min_f1": opts.min_f1},
            "warnings": ["Corpus directory exists but has no scenarios."],
        }));
    }

    let mut reports: Vec<Value> = Vec::new();
    let mut passing = 0usize;
    let mut failing = 0usize;
    for scenario in &scenarios {
        if let Some(filter) = opts.scenario_filter {
            if !scenario.name.contains(filter) {
                continue;
            }
        }
        let report = score_scenario(opts.project_dir, scenario)?;
        if report.f1 >= opts.min_f1 {
            passing += 1;
        } else {
            failing += 1;
        }
        reports.push(report.to_json());
    }

    let regressions: Vec<&Value> = reports
        .iter()
        .filter(|r| {
            r.get("f1")
                .and_then(|v| v.as_f64())
                .map(|f| f < opts.min_f1)
                .unwrap_or(false)
        })
        .collect();

    Ok(json!({
        "status": if failing == 0 { "ok" } else { "regressed" },
        "corpus_dir": corpus_root.display().to_string(),
        "summary": {
            "total": reports.len(),
            "passing": passing,
            "failing": failing,
            "min_f1": opts.min_f1,
            "regressions": regressions.iter().filter_map(|r| r.get("scenario")).cloned().collect::<Vec<_>>(),
        },
        "scenarios": reports,
        "next_command": if failing > 0 {
            "Investigate failing scenarios and update rule signals or expected.json."
        } else {
            "Benchmark is green. Persist baseline if needed."
        },
    }))
}

/// Persist the latest successful run as a baseline for future regression
/// checks. Writes `whetstone/.state/bench-snapshot.json`.
pub fn snapshot(project_dir: &Path, result: &Value) -> Result<PathBuf> {
    let dir = project_dir.join("whetstone").join(".state");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("bench-snapshot.json");
    crate::state::atomic_write(&path, result);
    Ok(path)
}

// ── Scenarios ──

struct Scenario {
    name: String,
    language: String,
    dir: PathBuf,
    rules: Vec<String>,
    expected: ExpectedSet,
}

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    language: String,
    #[serde(default)]
    rules: Vec<String>,
}

#[derive(Deserialize, Serialize)]
struct ExpectedViolation {
    rule_id: String,
    file: String,
    line: i64,
}

#[derive(Deserialize)]
struct ExpectedWrapper {
    violations: Vec<ExpectedViolation>,
}

#[derive(Default)]
struct ExpectedSet {
    items: BTreeSet<(String, String, i64)>,
}

fn discover_scenarios(root: &Path) -> Result<Vec<Scenario>> {
    let mut out = Vec::new();
    for lang_entry in std::fs::read_dir(root)? {
        let lang_entry = lang_entry?;
        if !lang_entry.file_type()?.is_dir() {
            continue;
        }
        let language = lang_entry.file_name().to_string_lossy().to_string();
        for scenario_entry in std::fs::read_dir(lang_entry.path())? {
            let scenario_entry = scenario_entry?;
            if !scenario_entry.file_type()?.is_dir() {
                continue;
            }
            let dir = scenario_entry.path();
            let name = format!(
                "{}/{}",
                language,
                scenario_entry.file_name().to_string_lossy()
            );

            let meta_path = dir.join("meta.yaml");
            let meta: Meta = if meta_path.exists() {
                let text = std::fs::read_to_string(&meta_path)?;
                serde_yaml::from_str(&text)
                    .map_err(|e| anyhow!("invalid meta.yaml at {}: {e}", meta_path.display()))?
            } else {
                Meta {
                    language: language.clone(),
                    rules: Vec::new(),
                }
            };

            let expected_path = dir.join("expected.json");
            let expected = if expected_path.exists() {
                let text = std::fs::read_to_string(&expected_path)?;
                let wrapper: ExpectedWrapper = serde_json::from_str(&text).map_err(|e| {
                    anyhow!("invalid expected.json at {}: {e}", expected_path.display())
                })?;
                let items: BTreeSet<_> = wrapper
                    .violations
                    .into_iter()
                    .map(|v| (v.rule_id, v.file, v.line))
                    .collect();
                ExpectedSet { items }
            } else {
                ExpectedSet::default()
            };

            let scenario_language = if meta.language.is_empty() {
                language.clone()
            } else {
                meta.language.clone()
            };
            out.push(Scenario {
                name,
                language: scenario_language,
                dir,
                rules: meta.rules,
                expected,
            });
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

// ── Scoring ──

struct ScenarioReport {
    scenario: String,
    language: String,
    expected_count: usize,
    actual_count: usize,
    true_positives: usize,
    false_positives: Vec<(String, String, i64)>,
    false_negatives: Vec<(String, String, i64)>,
    precision: f64,
    recall: f64,
    f1: f64,
}

impl ScenarioReport {
    fn to_json(&self) -> Value {
        json!({
            "scenario": self.scenario,
            "language": self.language,
            "expected_count": self.expected_count,
            "actual_count": self.actual_count,
            "true_positives": self.true_positives,
            "false_positives": self.false_positives.iter().map(|(r, f, l)| json!({
                "rule_id": r,
                "file": f,
                "line": l,
            })).collect::<Vec<_>>(),
            "false_negatives": self.false_negatives.iter().map(|(r, f, l)| json!({
                "rule_id": r,
                "file": f,
                "line": l,
            })).collect::<Vec<_>>(),
            "precision": self.precision,
            "recall": self.recall,
            "f1": self.f1,
            "passed": self.f1 >= 1.0 || (self.expected_count == 0 && self.actual_count == 0),
        })
    }
}

fn score_scenario(project_dir: &Path, scenario: &Scenario) -> Result<ScenarioReport> {
    let rule_refs: Vec<String> = scenario.rules.clone();
    let rule_slice: Option<&[String]> = if rule_refs.is_empty() {
        None
    } else {
        Some(rule_refs.as_slice())
    };
    let lang_filter = if scenario.language.is_empty() {
        None
    } else {
        Some(scenario.language.as_str())
    };
    let scan_paths = vec![scenario.dir.clone()];

    let result = check::run(CheckOptions {
        project_dir,
        scan_paths: &scan_paths,
        lang_filter,
        rule_filter: rule_slice,
    })?;

    let mut actual: BTreeSet<(String, String, i64)> = BTreeSet::new();
    if let Some(arr) = result.get("violations").and_then(|v| v.as_array()) {
        for v in arr {
            let rule_id = v
                .get("rule_id")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string();
            let file_abs = v
                .get("file")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();
            let line = v.get("line").and_then(|l| l.as_i64()).unwrap_or(0);
            let rel = relative_path(&scenario.dir, Path::new(&file_abs));
            actual.insert((rule_id, rel, line));
        }
    }

    let expected = &scenario.expected.items;
    let tp: BTreeSet<_> = expected.intersection(&actual).cloned().collect();
    let fp: Vec<_> = actual.difference(expected).cloned().collect();
    let fn_: Vec<_> = expected.difference(&actual).cloned().collect();

    let precision = ratio(tp.len(), actual.len());
    let recall = ratio(tp.len(), expected.len());
    let f1 = if precision + recall == 0.0 {
        if expected.is_empty() && actual.is_empty() {
            1.0
        } else {
            0.0
        }
    } else {
        2.0 * precision * recall / (precision + recall)
    };

    Ok(ScenarioReport {
        scenario: scenario.name.clone(),
        language: scenario.language.clone(),
        expected_count: expected.len(),
        actual_count: actual.len(),
        true_positives: tp.len(),
        false_positives: fp,
        false_negatives: fn_,
        precision,
        recall,
        f1,
    })
}

fn ratio(num: usize, denom: usize) -> f64 {
    if denom == 0 {
        1.0
    } else {
        num as f64 / denom as f64
    }
}

fn relative_path(base: &Path, p: &Path) -> String {
    p.strip_prefix(base)
        .map(|r| r.display().to_string())
        .unwrap_or_else(|_| p.display().to_string())
}

// ── Human formatter ──

pub fn format_human_output(result: &Value) -> String {
    let mut out = String::new();
    let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("?");
    out.push_str(&format!("wh bench: {status}\n"));
    let summary = result
        .get("summary")
        .cloned()
        .unwrap_or(Value::Object(serde_json::Map::new()));
    out.push_str(&format!(
        "  total: {} | passing: {} | failing: {} | min_f1: {}\n",
        summary.get("total").and_then(|v| v.as_i64()).unwrap_or(0),
        summary.get("passing").and_then(|v| v.as_i64()).unwrap_or(0),
        summary.get("failing").and_then(|v| v.as_i64()).unwrap_or(0),
        summary
            .get("min_f1")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    ));
    if let Some(scenarios) = result.get("scenarios").and_then(|v| v.as_array()) {
        for s in scenarios {
            let name = s
                .get("scenario")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let f1 = s.get("f1").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let passed = s.get("passed").and_then(|v| v.as_bool()).unwrap_or(false);
            let sigil = if passed { "✓" } else { "✗" };
            out.push_str(&format!("  {sigil} {name} (f1={f1:.2})\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_handles_zero_denominator() {
        assert_eq!(ratio(0, 0), 1.0);
        assert_eq!(ratio(1, 2), 0.5);
    }

    #[test]
    fn relative_path_strips_prefix_when_possible() {
        let base = Path::new("/tmp/a");
        assert_eq!(
            relative_path(base, Path::new("/tmp/a/src/x.py")),
            "src/x.py"
        );
        assert_eq!(relative_path(base, Path::new("/other/y.py")), "/other/y.py");
    }
}
