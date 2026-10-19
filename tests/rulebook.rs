//! The rulebook file: Whetstone's own `whetstone/rulebook.json` loads, the
//! published schema matches the reader, and the revision follows Git.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use whetstone::domain::{Enforcer, Strength};
use whetstone::rulebook::{self, Revision, RulebookError, RULEBOOK_PATH, RULEBOOK_SCHEMA};

fn schema() -> Value {
    serde_json::from_str(include_str!("../references/rulebook.schema.json"))
        .expect("valid rulebook schema")
}

fn keys(value: &Value) -> BTreeSet<String> {
    value.as_object().expect("object").keys().cloned().collect()
}

#[test]
fn whetstones_own_rulebook_holds_its_three_shared_must_rules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let text = std::fs::read_to_string(root.join(RULEBOOK_PATH)).expect("rulebook");
    let book = rulebook::parse(&text, Revision::Uncommitted).expect("Whetstone's rulebook loads");
    assert_eq!(book.mission.statement, "Demonstrate Whetstone annotations.");
    assert!(book.principles.is_empty());
    let ids = book
        .rules
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "rule.rust-formatted",
            "rule.rust-no-dead-code",
            "rule.rust-no-debug-macros"
        ]
    );
    for entry in &book.rules {
        assert_eq!(entry.rule.strength, Strength::Must, "{}", entry.id.as_str());
        assert_eq!(entry.rule.paths, ["src/**"], "{}", entry.id.as_str());
    }
    assert!(matches!(
        &book.rules[0].rule.enforcer,
        Enforcer::Formatter { tool } if tool == "cargo fmt --all --check"
    ));
    assert!(matches!(
        &book.rules[1].rule.enforcer,
        Enforcer::Lint { code, .. } if code == "dead_code"
    ));
    assert!(matches!(
        &book.rules[2].rule.enforcer,
        Enforcer::Ast { language: Some(language), .. } if language == "rust"
    ));
    let digests = book
        .rules
        .iter()
        .map(|entry| entry.digest.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(digests.len(), 3, "each rule has its own digest");
}

#[test]
fn the_published_schema_names_exactly_the_fields_the_reader_accepts() {
    let schema = schema();
    assert_eq!(schema["properties"]["schema"]["const"], RULEBOOK_SCHEMA);
    assert_eq!(schema["additionalProperties"], false);
    let full_rule = json!({
        "id": "rule.full",
        "statement": "Every field is set.",
        "rationale": "The schema and the reader agree.",
        "strength": "should",
        "enforcer": {"kind": "test", "command": "cargo test"},
        "examples": [{"input": "x", "expected": "flag", "reason": "an example", "path": "src/x.rs"}],
        "source": {"kind": "principle", "reference": "principle.prove-it-works", "provenance": [
            {"system": "git", "locator": "README.md"}
        ]},
        "paths": ["src/**"],
        "sweep": "draft_pr",
        "sweep_priority": 0,
        "sweep_unsure": [{"paths": ["tests/**"], "matching": "fixture", "reason": "fixtures may flag"}]
    });
    let full_principle = json!({
        "id": "principle.prove-it-works",
        "statement": "Prove it works.",
        "source": {"kind": "pstack", "id": "prove-it-works", "version": "0.15.5"},
        "rationale": "Evidence over claims."
    });
    let book = json!({
        "schema": RULEBOOK_SCHEMA,
        "mission": {"statement": "Ship code people can trust."},
        "principles": [full_principle],
        "rules": [full_rule]
    });
    rulebook::parse(&book.to_string(), Revision::Uncommitted).expect("every field parses");
    assert_eq!(keys(&schema["properties"]), keys(&book));
    assert_eq!(
        keys(&schema["properties"]["mission"]["properties"]),
        keys(&book["mission"])
    );
    let defs = &schema["$defs"];
    assert_eq!(keys(&defs["rule"]["properties"]), keys(&book["rules"][0]));
    assert_eq!(
        keys(&defs["principle"]["properties"]),
        keys(&book["principles"][0])
    );
    assert_eq!(
        keys(&defs["sweep_unsure"]["properties"]),
        keys(&book["rules"][0]["sweep_unsure"][0])
    );
    for name in [
        "rule",
        "principle",
        "sweep_unsure",
        "rule_example",
        "rule_source",
    ] {
        assert_eq!(defs[name]["additionalProperties"], false, "{name}");
    }
    let required = |value: &Value| {
        value["required"]
            .as_array()
            .expect("required")
            .iter()
            .map(|field| field.as_str().expect("field").to_string())
            .collect::<Vec<_>>()
    };
    for field in required(&defs["rule"]) {
        let mut missing = book.clone();
        missing["rules"][0]
            .as_object_mut()
            .expect("rule")
            .remove(&field);
        let error = rulebook::parse(&missing.to_string(), Revision::Uncommitted)
            .expect_err("a required field is missing");
        assert!(
            error
                .to_string()
                .contains(&format!("missing field `{field}`")),
            "{error}"
        );
    }
    let kinds = defs["enforcer"]["oneOf"]
        .as_array()
        .expect("enforcers")
        .iter()
        .map(|option| {
            option["properties"]["kind"]["const"]
                .as_str()
                .expect("kind")
        })
        .collect::<Vec<_>>();
    assert_eq!(kinds, ["ast", "lint", "formatter", "test", "validator"]);
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn the_revision_is_the_last_commit_to_touch_the_file_or_uncommitted() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    assert_eq!(rulebook::load(root), Err(RulebookError::Missing));
    git(root, &["init", "-q"]);
    std::fs::create_dir(root.join("whetstone")).expect("dir");
    let text = include_str!("../whetstone/rulebook.json");
    std::fs::write(root.join(RULEBOOK_PATH), text).expect("write");
    assert_eq!(
        rulebook::load(root).expect("untracked").revision,
        Revision::Uncommitted
    );
    git(root, &["add", RULEBOOK_PATH]);
    assert_eq!(
        rulebook::load(root).expect("staged").revision,
        Revision::Uncommitted
    );
    git(root, &["commit", "-q", "-m", "Add the rulebook"]);
    let touched = git(root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("README.md"), "elsewhere\n").expect("write");
    git(root, &["add", "README.md"]);
    git(root, &["commit", "-q", "-m", "Touch something else"]);
    let book = rulebook::load(root).expect("committed");
    assert_eq!(book.revision, Revision::Commit(touched.clone()));
    assert_eq!(
        serde_json::to_value(&book.revision).expect("encode"),
        json!(touched)
    );
    std::fs::write(root.join(RULEBOOK_PATH), text.replace("\n", "\n\n")).expect("edit");
    let edited = rulebook::load(root).expect("edited");
    assert_eq!(edited.revision, Revision::Uncommitted);
    assert_eq!(
        edited
            .rules
            .iter()
            .map(|entry| &entry.digest)
            .collect::<Vec<_>>(),
        book.rules
            .iter()
            .map(|entry| &entry.digest)
            .collect::<Vec<_>>(),
        "whitespace leaves every digest alone"
    );
}

#[test]
fn outside_git_the_revision_is_uncommitted() {
    let temp = tempfile::tempdir().expect("temp");
    std::fs::create_dir(temp.path().join("whetstone")).expect("dir");
    std::fs::write(
        temp.path().join(RULEBOOK_PATH),
        include_str!("../whetstone/rulebook.json"),
    )
    .expect("write");
    assert_eq!(
        rulebook::load(temp.path()).expect("loads").revision,
        Revision::Uncommitted
    );
}
