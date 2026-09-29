//! Three starter rules that work on day one, chosen by what the repository
//! looks like. Each comes with labelled examples and the cheapest enforcer
//! that can hold it; the owner accepts or skips each one.

use std::path::Path;

use serde::Serialize;

use crate::domain::{
    ContextUnit, Enforcer, ExampleVerdict, HandRaiseTrigger, Privacy, Reviewer, Rule, RuleExample,
    RuleSource, RuleSourceKind, Strength, DEFAULT_JEV_MODEL, RULE_SCHEMA_V2,
};

pub const PROVE_IT_WORKS: &str = "rule.prove-it-works";
pub const ASK_BEFORE_PUBLIC_API: &str = "rule.ask-before-public-api";
pub const TESTS_ONLY_WHEN_ASKED: &str = "rule.tests-only-when-asked";

#[derive(Debug, Clone, Serialize)]
pub struct Starter {
    pub id: &'static str,
    pub title: &'static str,
    /// Why this enforcer was chosen for this repository.
    pub detected: String,
    pub rule: Rule,
}

fn example(input: &str, expected: ExampleVerdict, reason: &str, path: Option<&str>) -> RuleExample {
    RuleExample {
        input: input.into(),
        expected,
        reason: reason.into(),
        path: path.map(str::to_owned),
    }
}

fn source(principle: &str) -> RuleSource {
    RuleSource {
        kind: RuleSourceKind::Starter,
        reference: Some(format!("pstack:{principle}")),
        provenance: Vec::new(),
    }
}

/// The test command a repository already uses, if one is evident.
pub fn detected_test_command(root: &Path) -> Option<(String, &'static str)> {
    if root.join("Cargo.toml").is_file() {
        return Some(("cargo test".into(), "Cargo.toml"));
    }
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        if serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|value| value.pointer("/scripts/test").cloned())
            .is_some()
        {
            return Some(("npm test".into(), "package.json scripts.test"));
        }
    }
    if root.join("pytest.ini").is_file()
        || root.join("pyproject.toml").is_file() && root.join("tests").is_dir()
    {
        return Some(("python3 -m pytest -q".into(), "pytest configuration"));
    }
    None
}

/// Source globs where a public surface lives, by detected language.
fn surface_paths(root: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    if root.join("Cargo.toml").is_file() {
        paths.push("src/**".into());
    }
    if root.join("package.json").is_file() {
        for directory in ["src", "lib"] {
            if root.join(directory).is_dir() {
                paths.push(format!("{directory}/**"));
            }
        }
    }
    for schema in ["references", "schemas"] {
        if root.join(schema).is_dir() {
            paths.push(format!("{schema}/**"));
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

pub fn starters(root: &Path) -> Vec<Starter> {
    let (prove_enforcer, prove_detected) = match detected_test_command(root) {
        Some((command, from)) => (
            Enforcer::Test { command },
            format!("Proves with the test command from {from} at pre-push; map a feature later to prove it by driving the app."),
        ),
        None => (
            Enforcer::Review {
                reviewer: Reviewer::Interrogate,
            },
            "No test command was detected, so /interrogate reviews that the change was proven; replace it with a test or drive proof when one exists.".into(),
        ),
    };
    let prove = Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: "Prove it works before calling it done.".into(),
        rationale: "Work is finished when the real artifact has been run and checked, not when it compiles or an agent says so.".into(),
        strength: Strength::Must,
        enforcer: prove_enforcer,
        examples: vec![
            example(
                "Changed the parser and ran the test suite; it passed.",
                ExampleVerdict::Pass,
                "The change was run against the real test suite.",
                None,
            ),
            example(
                "Changed the parser and reported done without running anything.",
                ExampleVerdict::Flag,
                "Nothing proved the change works.",
                None,
            ),
        ],
        source: source("prove-it-works"),
        paths: Vec::new(),
        hand_raise: vec![HandRaiseTrigger::Unavailable],
        privacy: Privacy::default(),
    };
    let surface_paths = surface_paths(root);
    let api = Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: "Ask before changing a public API.".into(),
        rationale: "Exported items, command-line flags and JSON fields are promises to other people; changing one is the owner's call.".into(),
        strength: Strength::Must,
        enforcer: Enforcer::PublicSurface {
            surfaces: Vec::new(),
        },
        examples: vec![
            example(
                "pub fn run(input: &str) -> u8 { 1 }\n=== after ===\npub fn run(input: &str) -> u8 { 2 }",
                ExampleVerdict::Pass,
                "Only the body changed; callers see the same signature.",
                Some("src/lib.rs"),
            ),
            example(
                "pub fn run(input: &str) -> u8 { 1 }\n=== after ===\npub fn run(input: &str, strict: bool) -> u8 { 1 }",
                ExampleVerdict::Flag,
                "The exported signature changed.",
                Some("src/lib.rs"),
            ),
        ],
        source: source("boundary-discipline"),
        paths: surface_paths.clone(),
        hand_raise: vec![HandRaiseTrigger::Flag],
        privacy: Privacy::default(),
    };
    let tests = Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: "Only add tests when the task asks for them.".into(),
        rationale: "Unrequested tests are code someone must maintain; add them when the task calls for them or a bug needs a regression test.".into(),
        strength: Strength::Should,
        enforcer: Enforcer::Question {
            question: "Does this change add a test that its commit message does not ask for or explain?".into(),
            yes_means: Some("A new test appears and the commit message is about something else".into()),
            no_means: Some("The commit asks for the test, or it is a regression test for the fix it describes".into()),
            unit: ContextUnit::AddedFile,
            model: DEFAULT_JEV_MODEL.into(),
            shadow: true,
            threshold_bp: 5_000,
            confidence_bar_bp: 4_000,
            promote_after: 50,
            precision_bar_bp: 9_000,
        },
        examples: vec![
            example(
                "commit: Fix typo in the README\nadded file tests/readme_links_test.rs\n#[test] fn links_resolve() {}",
                ExampleVerdict::Flag,
                "A typo fix does not ask for a new test.",
                Some("tests/readme_links_test.rs"),
            ),
            example(
                "commit: Add tests for parser edge cases\nadded file tests/parser_edges.rs\n#[test] fn empty_input() {}",
                ExampleVerdict::Pass,
                "The task is the tests.",
                Some("tests/parser_edges.rs"),
            ),
        ],
        source: source("laziness-protocol"),
        paths: vec![
            "tests/**".into(),
            "**/*_test.*".into(),
            "**/*.test.*".into(),
            "**/test_*.py".into(),
        ],
        hand_raise: vec![HandRaiseTrigger::LowConfidence],
        privacy: Privacy::default(),
    };
    vec![
        Starter {
            id: PROVE_IT_WORKS,
            title: "Prove it works before done",
            detected: prove_detected,
            rule: prove,
        },
        Starter {
            id: ASK_BEFORE_PUBLIC_API,
            title: "Ask before changing a public API",
            detected: if surface_paths.is_empty() {
                "Compares every source file's exports, CLI flags and JSON schemas with the last pushed revision at pre-commit, and raises a hand on any change.".into()
            } else {
                format!(
                    "Compares exports, CLI flags and JSON schemas under {} with the last pushed revision at pre-commit, and raises a hand on any change.",
                    surface_paths.join(", ")
                )
            },
            rule: api,
        },
        Starter {
            id: TESTS_ONLY_WHEN_ASKED,
            title: "Only add tests when the task asks",
            detected: "Asks Jev about each new test file at pre-push, in shadow: answers are recorded, not enforced, until you promote it.".into(),
            rule: tests,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starters_pick_the_cheapest_enforcer_the_repository_supports() {
        let temp = tempfile::tempdir().expect("temp");
        let bare = starters(temp.path());
        assert_eq!(bare.len(), 3);
        assert!(matches!(bare[0].rule.enforcer, Enforcer::Review { .. }));
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("write");
        let rust = starters(temp.path());
        assert!(
            matches!(&rust[0].rule.enforcer, Enforcer::Test { command } if command == "cargo test")
        );
        assert_eq!(rust[1].rule.paths, vec!["src/**".to_string()]);
        for starter in rust {
            starter.rule.validate().expect("valid starter rule");
            assert!(starter.rule.examples.len() >= 2);
        }
    }
}
