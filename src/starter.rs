//! Starter rules that work on day one, chosen by what the repository looks
//! like. Each comes with labelled examples and the mechanical enforcer that
//! holds it; the owner accepts or skips each one.

use std::path::Path;

use serde::Serialize;

use crate::domain::{
    Enforcer, ExampleVerdict, Rule, RuleExample, RuleSource, RuleSourceKind, Strength,
    RULE_SCHEMA_V2,
};

pub const PROVE_IT_WORKS: &str = "rule.prove-it-works";
pub const NO_CONFLICT_MARKERS: &str = "rule.no-conflict-markers";

#[derive(Debug, Clone, Serialize)]
pub struct Starter {
    pub id: &'static str,
    pub title: &'static str,
    /// Why this enforcer was chosen for this repository.
    pub detected: String,
    pub rule: Rule,
}

fn example(input: &str, expected: ExampleVerdict, reason: &str) -> RuleExample {
    RuleExample {
        input: input.into(),
        expected,
        reason: reason.into(),
        path: None,
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

/// The starter rules this repository can hold mechanically: prove-it-works,
/// held by the repository's own test command (without one nothing mechanical
/// holds it, so it is not offered), and the script scaffold's example once
/// wiring has written it.
pub fn starters(root: &Path) -> Vec<Starter> {
    let mut offered = Vec::new();
    if let Some(starter) = prove_it_works(root) {
        offered.push(starter);
    }
    // Once wiring has written the script scaffold, its example is a rule too.
    if root.join(crate::scripts::EXAMPLE).is_file() {
        offered.push(Starter {
            id: NO_CONFLICT_MARKERS,
            title: "No leftover merge conflict markers",
            detected: format!(
                "Runs the script {} (a script enforcer; contract in {}) on the staged files at pre-commit and on every change at pre-push.",
                crate::scripts::EXAMPLE,
                crate::scripts::README
            ),
            rule: conflict_markers(),
        });
    }
    offered
}

fn prove_it_works(root: &Path) -> Option<Starter> {
    let (command, from) = detected_test_command(root)?;
    let rule = Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: "Prove it works before calling it done.".into(),
        rationale: "Work is finished when the real artifact has been run and checked, not when it compiles or an agent says so.".into(),
        strength: Strength::Must,
        enforcer: Enforcer::Test { command },
        examples: vec![
            example(
                "Changed the parser and ran the test suite; it passed.",
                ExampleVerdict::Pass,
                "The change was run against the real test suite.",
            ),
            example(
                "Changed the parser and reported done without running anything.",
                ExampleVerdict::Flag,
                "Nothing proved the change works.",
            ),
        ],
        source: source("prove-it-works"),
        paths: Vec::new(),
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
    };
    Some(Starter {
        id: PROVE_IT_WORKS,
        title: "Prove it works before done",
        detected: format!(
            "Proves with the test command from {from} at pre-push; map a feature later to prove it by driving the app."
        ),
        rule,
    })
}

/// The script starter: the scaffold's example as a must rule.
fn conflict_markers() -> Rule {
    let example = |input: &str, expected, reason: &str| RuleExample {
        path: Some("src/main.rs".into()),
        ..example(input, expected, reason)
    };
    Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: "Never commit a leftover merge conflict marker.".into(),
        rationale: "A marker left from a merge breaks the file for everyone, and no linter for every language catches it.".into(),
        strength: Strength::Must,
        enforcer: Enforcer::Validator {
            command: crate::scripts::EXAMPLE_COMMAND.into(),
        },
        examples: vec![
            example(
                "fn main() {}\n",
                ExampleVerdict::Pass,
                "No line starts a conflict marker.",
            ),
            example(
                "<<<<<<< HEAD\nfn main() {}\n=======\nfn main() { run() }\n>>>>>>> topic\n",
                ExampleVerdict::Flag,
                "The merge was left unresolved.",
            ),
        ],
        source: RuleSource {
            kind: RuleSourceKind::Starter,
            reference: Some(crate::scripts::EXAMPLE.into()),
            provenance: Vec::new(),
        },
        paths: Vec::new(),
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prove_it_works_is_offered_only_with_a_test_command() {
        let temp = tempfile::tempdir().expect("temp");
        assert!(
            starters(temp.path()).is_empty(),
            "without a test command nothing mechanical proves the work"
        );
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("write");
        let rust = starters(temp.path());
        assert_eq!(rust.len(), 1);
        assert_eq!(rust[0].id, PROVE_IT_WORKS);
        assert!(
            matches!(&rust[0].rule.enforcer, Enforcer::Test { command } if command == "cargo test")
        );
        rust[0].rule.validate().expect("valid starter rule");
        assert!(rust[0].rule.examples.len() >= 2);
    }
}
