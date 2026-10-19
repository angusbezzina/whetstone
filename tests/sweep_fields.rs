//! The sweep fields of a rule set through `wh change`: an agent cannot widen
//! a rule's sweep class and a person can, out-of-range priorities and doubts
//! that cover every flag are refused, and what was set lands in
//! `whetstone/rulebook.json`. Every repository is a throwaway; no `bd` runs.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::ActorKind;
use whetstone::service::{ChangeKind, ChangeRequest, CommandService, ServiceRequest, ServiceState};

mod support;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(args)
        .current_dir(root)
        .env("PATH", support::path_with_bd_template())
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .env("BD_NON_INTERACTIVE", "1")
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .output()
        .expect("whetstone")
}

fn wh(root: &Path, args: &[&str]) -> Value {
    let output = run(root, args);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "wh {args:?} did not print JSON ({error}): {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn repository() -> TempDir {
    let temp = TempDir::new().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(
        root.join("src/config.py"),
        "import json\n\ndef load(path):\n    return json.load(open(path))\n",
    )
    .expect("write");
    std::fs::write(root.join("README.md"), "notes\n").expect("write");
    std::fs::write(root.join(".gitignore"), "secrets/\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);
    temp
}

/// Record a rule draft through the binary (inspect, then confirm).
/// Draft a rule into the rulebook file through the binary.
fn draft_rule(root: &Path, id: &str, statement: &str, definition: Value) -> Value {
    wh(
        root,
        &[
            "change",
            "--json",
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            statement,
            "--definition",
            &definition.to_string(),
            "--rationale",
            "Drafted for the test.",
        ],
    )
}

const PRINT: &str = "((call function: (identifier) @f) @match (#eq? @f \"print\"))";

fn print_rule(strength: &str) -> Value {
    json!({
        "type": "rule",
        "strength": strength,
        "enforcer": {"kind": "ast", "language": "python", "query": PRINT},
    })
}

#[test]
fn a_rules_sweep_fields_are_set_through_change_into_the_rulebook() {
    let temp = repository();
    let root = temp.path();
    let mission = wh(
        root,
        &[
            "change",
            "--json",
            "--kind",
            "mission",
            "--content",
            "Ship.",
        ],
    );
    assert_eq!(mission["state"], "success", "{mission}");
    let rule = |strength: &str, sweep: Option<&str>| {
        let mut definition = print_rule(strength);
        if let Some(sweep) = sweep {
            definition["sweep"] = json!(sweep);
        }
        definition
    };

    // An agent cannot widen a sweep class; a person can.
    let widened = draft_rule(
        root,
        "rule.print-should",
        "Prefer logging to print.",
        rule("should", Some("draft_pr")),
    );
    assert!(
        widened["blocking_questions"]
            .to_string()
            .contains("cannot widen"),
        "{widened}"
    );
    let mut definition = rule("should", Some("draft_pr"));
    definition["sweep_priority"] = json!(0);
    let by_person = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        actor: ActorKind::Person,
        kind: Some(ChangeKind::Rule),
        record_id: Some("rule.print-should".into()),
        content: Some("Prefer logging to print.".into()),
        rationale: Some("Mechanical, safe to open as a draft PR.".into()),
        definition: Some(Box::new(
            serde_json::from_value(definition).expect("definition"),
        )),
        ..ChangeRequest::default()
    }));
    assert_eq!(
        by_person.state,
        ServiceState::Success,
        "{}",
        by_person.summary
    );
    // A priority outside 0..4 and a doubt that covers every flag are refused.
    for (name, bad) in [
        ("priority-5", json!({"sweep_priority": 5})),
        (
            "unsure-all",
            json!({"sweep_unsure": [{"paths": ["**"], "reason": "everything"}]}),
        ),
        (
            "unsure-any",
            json!({"sweep_unsure": [{"matching": ".*", "reason": "everything"}]}),
        ),
        (
            "unsure-blank",
            json!({"sweep_unsure": [{"paths": ["src/**"], "reason": " "}]}),
        ),
    ] {
        let mut definition = rule("should", None);
        for (key, value) in bad.as_object().expect("object") {
            definition[key] = value.clone();
        }
        let refused = draft_rule(root, "rule.print-bad", "Bad.", definition);
        assert!(
            refused["state"] != "success" && refused["data"]["written"].is_null(),
            "{name}: {refused}"
        );
    }
    for (id, strength) in [
        ("rule.print-doubtful", "should"),
        ("rule.print-must", "must"),
    ] {
        let drafted = draft_rule(root, id, "Print is a smell.", rule(strength, None));
        assert_eq!(drafted["state"], "success", "{drafted}");
    }

    // The file holds what was set; a class or priority left out is absent.
    let book: Value = serde_json::from_slice(
        &std::fs::read(root.join("whetstone/rulebook.json")).expect("rulebook"),
    )
    .expect("rulebook JSON");
    let field_of = |id: &str, field: &str| {
        book["rules"]
            .as_array()
            .and_then(|rules| rules.iter().find(|rule| rule["id"] == id).cloned())
            .map(|rule| rule[field].clone())
    };
    assert_eq!(
        field_of("rule.print-should", "sweep"),
        Some(json!("draft_pr"))
    );
    assert_eq!(
        field_of("rule.print-should", "sweep_priority"),
        Some(json!(0))
    );
    assert_eq!(field_of("rule.print-doubtful", "sweep"), Some(Value::Null));
    assert_eq!(
        field_of("rule.print-must", "sweep_priority"),
        Some(Value::Null)
    );
    assert_eq!(
        field_of("rule.print-bad", "id"),
        None,
        "a refused rule is never written"
    );
}
