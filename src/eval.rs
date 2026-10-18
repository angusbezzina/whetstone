//! `wh eval` for rules: every rule scored against its labelled examples, per
//! rule and per enforcer, plus the pstack round-trip contract.
//!
//! AST enforcers read content, so they run on each example directly. Command-based enforcers (tests,
//! linters, drives) are not scored by examples, and the report says so
//! rather than counting them as passes.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{Enforcer, ExampleVerdict, Rule};
use crate::gates::FileSet;

/// Precision and recall over one rule's (or one enforcer's) examples.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Score {
    pub true_flags: u32,
    pub false_flags: u32,
    pub true_passes: u32,
    pub missed_flags: u32,
    /// Examples that could not be scored (an enforcer that examples cannot
    /// exercise).
    pub unscored: u32,
}

impl Score {
    pub fn checked(&self) -> u32 {
        self.true_flags + self.false_flags + self.true_passes + self.missed_flags
    }

    fn ratio(numerator: u32, denominator: u32) -> Option<String> {
        (denominator > 0).then(|| format!("{:.3}", f64::from(numerator) / f64::from(denominator)))
    }

    pub fn precision(&self) -> Option<String> {
        Self::ratio(self.true_flags, self.true_flags + self.false_flags)
    }

    pub fn recall(&self) -> Option<String> {
        Self::ratio(self.true_flags, self.true_flags + self.missed_flags)
    }

    fn add(&mut self, other: &Score) {
        self.true_flags += other.true_flags;
        self.false_flags += other.false_flags;
        self.true_passes += other.true_passes;
        self.missed_flags += other.missed_flags;
        self.unscored += other.unscored;
    }

    fn record(&mut self, expected: ExampleVerdict, flagged: Option<bool>) {
        match (expected, flagged) {
            (_, None) => self.unscored += 1,
            (ExampleVerdict::Flag, Some(true)) => self.true_flags += 1,
            (ExampleVerdict::Flag, Some(false)) => self.missed_flags += 1,
            (ExampleVerdict::Pass, Some(true)) => self.false_flags += 1,
            (ExampleVerdict::Pass, Some(false)) => self.true_passes += 1,
        }
    }
}

/// Whether the enforcer flags one example, or `None` when examples cannot
/// exercise it.
fn flags_example(
    rule: &Rule,
    example: &crate::domain::RuleExample,
) -> Result<Option<bool>, String> {
    let path = example.path.clone();
    match &rule.enforcer {
        Enforcer::Ast { query, language } => {
            if !query.trim_start().starts_with('(') && !query.trim_start().starts_with('[') {
                return Ok(None);
            }
            let path = path.unwrap_or_else(|| match language.as_deref() {
                Some("python") => "example.py".into(),
                Some("typescript") | Some("javascript") => "example.ts".into(),
                _ => "example.rs".into(),
            });
            let files = FileSet::from_contents(
                crate::gates::Source::WorkingTree,
                BTreeMap::from([(path, example.input.as_bytes().to_vec())]),
            );
            let run =
                crate::gates::ast::run_query(&files, query, language.as_deref(), |_| true, 50)?;
            Ok(Some(!run.failures.is_empty()))
        }
        _ => Ok(None),
    }
}

/// One rule's scorecard.
#[derive(Debug, Clone, Serialize)]
pub struct RuleScore {
    pub rule: String,
    pub source: &'static str,
    pub enforcer: &'static str,
    pub strength: &'static str,
    pub examples: usize,
    pub score: Score,
    pub precision: Option<String>,
    pub recall: Option<String>,
    pub mismatches: Vec<Value>,
    pub errors: Vec<String>,
}

/// Score a set of rules: `(id, where it came from, rule)`.
pub fn score_rules(rules: &[(String, &'static str, Rule)]) -> Vec<RuleScore> {
    let mut result = Vec::new();
    for (id, source, rule) in rules {
        let mut score = Score::default();
        let mut mismatches = Vec::new();
        let mut errors = Vec::new();
        for example in &rule.examples {
            match flags_example(rule, example) {
                Ok(flagged) => {
                    score.record(example.expected, flagged);
                    if let Some(flagged) = flagged {
                        if flagged != (example.expected == ExampleVerdict::Flag) {
                            mismatches.push(json!({
                                "expected": example.expected,
                                "flagged": flagged,
                                "reason": example.reason,
                                "input": example.input.chars().take(300).collect::<String>(),
                            }));
                        }
                    }
                }
                Err(error) => {
                    score.unscored += 1;
                    errors.push(error);
                }
            }
        }
        result.push(RuleScore {
            rule: id.clone(),
            source,
            enforcer: rule.enforcer.kind(),
            strength: rule.strength.label(),
            examples: rule.examples.len(),
            precision: score.precision(),
            recall: score.recall(),
            score,
            mismatches,
            errors,
        });
    }
    result
}

/// The pstack round-trip contract on the pinned example: every feature file
/// and the README re-render byte-identically after import.
pub fn pstack_roundtrip() -> Value {
    let results = crate::feature_map::pinned_roundtrip();
    let ok = results.iter().all(|(_, result)| result.is_ok());
    json!({
        "ok": ok,
        "pstack_version": crate::setup::PSTACK_VERSION,
        "files": results
            .iter()
            .map(|(file, result)| json!({"file": file, "ok": result.is_ok(), "error": result.as_ref().err()}))
            .collect::<Vec<_>>(),
    })
}

/// The whole rule evaluation for a project: its rules (in force and drafts),
/// the starter rules, per-enforcer totals and the round-trip contract.
pub fn evaluate(project: &Path) -> Value {
    let state = crate::storage::ProjectLayout::resolve(project)
        .ok()
        .and_then(|layout| crate::service::load_records(&layout).ok())
        .filter(crate::service::LoadedRecords::exists)
        .map(|loaded| AgreementState::from_records(loaded.union().0));
    let mut rules = Vec::<(String, &'static str, Rule)>::new();
    if let Some(state) = &state {
        for id in crate::proof::rule_ids(state) {
            let record = state.pending(&id).or_else(|| state.in_force(&id));
            if let Some(rule) = record.and_then(|record| record.body.rule_view()) {
                rules.push((
                    id.as_str().to_string(),
                    if state.pending(&id).is_some() {
                        "draft"
                    } else {
                        "agreement"
                    },
                    rule.into_owned(),
                ));
            }
        }
    }
    for starter in crate::starter::starters(project) {
        rules.push((starter.id.to_string(), "starter", starter.rule));
    }
    let scores = score_rules(&rules);
    let mut by_enforcer = BTreeMap::<&'static str, Score>::new();
    for score in &scores {
        by_enforcer
            .entry(score.enforcer)
            .or_default()
            .add(&score.score);
    }
    let checked = scores
        .iter()
        .map(|score| score.score.checked())
        .sum::<u32>();
    let mismatches = scores
        .iter()
        .map(|score| score.mismatches.len())
        .sum::<usize>();
    let errors = scores.iter().map(|score| score.errors.len()).sum::<usize>();
    let roundtrip = pstack_roundtrip();
    // Only AST rules can be scored by their examples; a project whose rules
    // are all command checks has nothing here to score, which is not a
    // failure (the scanner half of `wh eval` still has to exercise work).
    let ok = mismatches == 0 && errors == 0 && roundtrip["ok"] == true;
    json!({
        "ok": ok,
        "rules_scored": scores.len(),
        "examples_checked": checked,
        "mismatch_count": mismatches,
        "error_count": errors,
        "rules": scores,
        "by_enforcer": by_enforcer
            .iter()
            .map(|(enforcer, score)| json!({
                "enforcer": enforcer,
                "score": score,
                "precision": score.precision(),
                "recall": score.recall(),
            }))
            .collect::<Vec<_>>(),
        "pstack_roundtrip": roundtrip,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ast_examples_score_and_command_starters_are_unscored() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("cargo");
        let mut rules = crate::starter::starters(temp.path())
            .into_iter()
            .map(|starter| (starter.id.to_string(), "starter", starter.rule))
            .collect::<Vec<_>>();
        let ast: Rule = serde_json::from_value(json!({
            "schema": crate::domain::RULE_SCHEMA_V2,
            "statement": "No unsafe blocks.",
            "rationale": "Memory safety.",
            "strength": "must",
            "enforcer": {"kind": "ast", "query": "(unsafe_block) @match", "language": "rust"},
            "examples": [
                {"input": "fn f(p: *const u8) -> u8 { unsafe { *p } }", "expected": "flag", "reason": "An unsafe block."},
                {"input": "fn f(a: u8) -> u8 { a }", "expected": "pass", "reason": "Safe."}
            ],
            "source": {"kind": "owner"}
        }))
        .expect("rule");
        rules.push(("rule.no-unsafe".into(), "agreement", ast));
        let scores = score_rules(&rules);
        let ast = scores
            .iter()
            .find(|score| score.enforcer == "ast")
            .expect("ast");
        assert_eq!(ast.score.checked(), 2);
        assert_eq!(ast.precision.as_deref(), Some("1.000"));
        assert!(ast.mismatches.is_empty());
        let prove = scores
            .iter()
            .find(|score| score.rule == crate::starter::PROVE_IT_WORKS)
            .expect("prove-it-works");
        assert_eq!(
            prove.score.unscored, 2,
            "a test command is not scored by examples"
        );
    }

    #[test]
    fn the_pinned_pstack_example_round_trips() {
        assert_eq!(pstack_roundtrip()["ok"], true);
    }
}
