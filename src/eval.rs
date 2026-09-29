//! `wh eval` for rules: every rule scored against its labelled examples, per
//! rule and per enforcer, plus the pstack round-trip contract.
//!
//! Mechanical enforcers that read content (AST, design tokens, public
//! surface) run on each example directly. Jev questions are scored only from
//! recorded answers (judgment receipts, or a cassette in
//! `WHETSTONE_JEV_CASSETTE`), so CI never needs the network. Command-based
//! enforcers (tests, linters, drives, briefs, reviews) are not scored by
//! examples, and the report says so rather than counting them as passes.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{Enforcer, ExampleVerdict, JudgmentOutcome, RecordBody, Rule};
use crate::gates::FileSet;

/// Precision and recall over one rule's (or one enforcer's) examples.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Score {
    pub true_flags: u32,
    pub false_flags: u32,
    pub true_passes: u32,
    pub missed_flags: u32,
    /// Examples that could not be scored (no recorded answer, or an
    /// enforcer that examples cannot exercise).
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

/// A Jev answer recorded for one exact question and input.
#[derive(Debug, Clone, Default)]
pub struct Recordings {
    /// (question digest, input digest) -> flagged.
    answers: BTreeMap<(String, String), bool>,
}

impl Recordings {
    /// Answers from judgment receipts and, when set, a cassette file of
    /// `[{"question_digest", "input_digest", "probability"}]` entries.
    pub fn load(state: Option<&AgreementState>) -> Self {
        let mut answers = BTreeMap::new();
        for record in state.map(AgreementState::records).unwrap_or_default() {
            if let RecordBody::Judgment(body) = &record.body {
                let flagged = match body.outcome {
                    JudgmentOutcome::Flag => Some(true),
                    JudgmentOutcome::Clear => Some(false),
                    _ => None,
                };
                if let Some(flagged) = flagged {
                    answers.insert(
                        (
                            body.question_digest.as_str().to_string(),
                            body.input_digest.as_str().to_string(),
                        ),
                        flagged,
                    );
                }
            }
        }
        if let Some(path) = std::env::var_os("WHETSTONE_JEV_CASSETTE") {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(&text) {
                    for entry in entries {
                        let (Some(question), Some(input), Some(probability)) = (
                            entry["question_digest"].as_str(),
                            entry["input_digest"].as_str(),
                            entry["probability"].as_f64(),
                        ) else {
                            continue;
                        };
                        answers.insert((question.into(), input.into()), probability >= 0.5);
                    }
                }
            }
        }
        Self { answers }
    }
}

/// Whether the enforcer flags one example, or `None` when examples cannot
/// exercise it.
fn flags_example(
    project: &Path,
    rule: &Rule,
    example: &crate::domain::RuleExample,
    recordings: &Recordings,
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
        Enforcer::DesignTokens { tokens, sizes, .. } => {
            let token_text = std::fs::read_to_string(project.join(tokens)).ok();
            let vocabulary = match token_text {
                Some(text) => crate::gates::tokens::Tokens::parse(tokens, &text)?,
                // Examples can still be scored against a minimal vocabulary
                // when the token file is not in this repository.
                None => crate::gates::tokens::Tokens::parse(
                    "fallback.css",
                    ":root { --accent: #ff7a00; --ink: #0a0a0a; --space: 8px; }",
                )?,
            };
            let path = path.unwrap_or_else(|| "example.css".into());
            Ok(Some(
                !crate::gates::tokens::check_stylesheet(
                    &path,
                    &example.input,
                    &vocabulary,
                    false,
                    *sizes,
                )
                .is_empty(),
            ))
        }
        Enforcer::PublicSurface { surfaces } => {
            let Some((before, after)) = example.input.split_once("=== after ===") else {
                return Err(
                    "a public-surface example is `<before>\n=== after ===\n<after>`".into(),
                );
            };
            let path = path.unwrap_or_else(|| "src/lib.rs".into());
            let before = crate::gates::surface::surface_of(&path, before, surfaces);
            let after = crate::gates::surface::surface_of(&path, after, surfaces);
            Ok(Some(
                !crate::gates::surface::diff(&path, &before, &after).is_empty(),
            ))
        }
        Enforcer::Question { .. } => {
            if rule.privacy.local_only {
                return Ok(None);
            }
            let redacted =
                crate::judgment::redact(&[&rule.privacy], path.as_deref(), &example.input);
            let Some(request) = crate::judgment::question_request(rule, &redacted.text) else {
                return Ok(None);
            };
            let key = (
                crate::judgment::question_digest(&request)
                    .as_str()
                    .to_string(),
                crate::judgment::input_digest(&redacted.text)
                    .as_str()
                    .to_string(),
            );
            Ok(recordings.answers.get(&key).copied())
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
    pub shadow: bool,
    pub examples: usize,
    pub score: Score,
    pub precision: Option<String>,
    pub recall: Option<String>,
    pub mismatches: Vec<Value>,
    pub errors: Vec<String>,
}

/// Score a set of rules: `(id, where it came from, rule)`.
pub fn score_rules(
    project: &Path,
    rules: &[(String, &'static str, Rule)],
    recordings: &Recordings,
) -> Vec<RuleScore> {
    let mut result = Vec::new();
    for (id, source, rule) in rules {
        let mut score = Score::default();
        let mut mismatches = Vec::new();
        let mut errors = Vec::new();
        for example in &rule.examples {
            match flags_example(project, rule, example, recordings) {
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
            shadow: rule.in_shadow(),
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
    let recordings = Recordings::load(state.as_ref());
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
    let scores = score_rules(project, &rules, &recordings);
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
    let ok = checked > 0 && mismatches == 0 && errors == 0 && roundtrip["ok"] == true;
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
    fn starter_and_mechanical_examples_score_cleanly() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("cargo");
        let rules = crate::starter::starters(temp.path())
            .into_iter()
            .map(|starter| (starter.id.to_string(), "starter", starter.rule))
            .collect::<Vec<_>>();
        let scores = score_rules(temp.path(), &rules, &Recordings::default());
        let surface = scores
            .iter()
            .find(|score| score.enforcer == "public_surface")
            .expect("surface");
        assert_eq!(surface.score.checked(), 2);
        assert_eq!(surface.precision.as_deref(), Some("1.000"));
        assert!(surface.mismatches.is_empty());
        let question = scores
            .iter()
            .find(|score| score.enforcer == "question")
            .expect("question");
        assert_eq!(
            question.score.unscored, 2,
            "no recorded answers, no network"
        );
    }

    #[test]
    fn recorded_answers_score_a_question_rule() {
        let rule = crate::starter::starters(Path::new("/nonexistent"))
            .into_iter()
            .find(|starter| starter.id == crate::starter::TESTS_ONLY_WHEN_ASKED)
            .expect("starter")
            .rule;
        let mut recordings = Recordings::default();
        for example in &rule.examples {
            let redacted =
                crate::judgment::redact(&[&rule.privacy], example.path.as_deref(), &example.input);
            let request =
                crate::judgment::question_request(&rule, &redacted.text).expect("request");
            recordings.answers.insert(
                (
                    crate::judgment::question_digest(&request)
                        .as_str()
                        .to_string(),
                    crate::judgment::input_digest(&redacted.text)
                        .as_str()
                        .to_string(),
                ),
                // Jev flags the first example (a true flag) and also the
                // second (a false flag).
                true,
            );
        }
        let scores = score_rules(
            Path::new("."),
            &[("rule.tests".into(), "starter", rule)],
            &recordings,
        );
        assert_eq!(scores[0].score.true_flags, 1);
        assert_eq!(scores[0].score.false_flags, 1);
        assert_eq!(scores[0].precision.as_deref(), Some("0.500"));
        assert_eq!(scores[0].recall.as_deref(), Some("1.000"));
    }

    #[test]
    fn the_pinned_pstack_example_round_trips() {
        assert_eq!(pstack_roundtrip()["ok"], true);
    }
}
