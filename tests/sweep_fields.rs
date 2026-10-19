//! The sweep fields of a rule set through `wh change`: an agent cannot widen
//! a rule's sweep class and a person can, out-of-range priorities and doubts
//! that cover every flag are refused, and `wh dash` shows class and
//! priority. Every repository is a throwaway with its own `bd init` store.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::ActorKind;
use whetstone::service::{ChangeKind, ChangeRequest, CommandService, ServiceRequest, ServiceState};

mod support;

fn have_bd() -> bool {
    let ok = Command::new("bd")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !ok {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI must have bd installed"
        );
        eprintln!("skipping: bd is not installed");
    }
    ok
}

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
fn draft_rule(
    root: &Path,
    request: &str,
    id: &str,
    statement: &str,
    definition: Value,
    extra: &[&str],
) -> Value {
    let mut base = vec![
        "change".to_string(),
        "--json".into(),
        "--request-id".into(),
        request.into(),
        "--kind".into(),
        "rule".into(),
        "--record-id".into(),
        id.into(),
        "--content".into(),
        statement.into(),
        "--definition".into(),
        definition.to_string(),
        "--rationale".into(),
        "Drafted for the test.".into(),
    ];
    base.extend(extra.iter().map(|value| value.to_string()));
    let args = base.iter().map(String::as_str).collect::<Vec<_>>();
    let inspected = wh(root, &args);
    let Some(revision) = inspected["expected_revision"].as_u64() else {
        return inspected;
    };
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let revision = revision.to_string();
    let mut confirm = args.clone();
    confirm.extend(["--expected-revision", &revision, "--resume", &token]);
    wh(root, &confirm)
}

fn accept(root: &Path, proposal: &str) -> Value {
    wh(root, &["change", "--json", "--accept", proposal])
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
fn a_rules_sweep_fields_are_set_through_change_and_shown_in_dash() {
    if !have_bd() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    let rule = |strength: &str, sweep: Option<&str>| {
        let mut definition = print_rule(strength);
        if let Some(sweep) = sweep {
            definition["sweep"] = json!(sweep);
        }
        definition
    };
    let prioritised = |mut definition: Value| {
        definition["sweep_priority"] = json!(0);
        definition
    };

    // An agent cannot widen a sweep class; a person can.
    let widened = draft_rule(
        root,
        "agent-wide",
        "rule.print-should",
        "Prefer logging to print.",
        rule("should", Some("draft_pr")),
        &[],
    );
    assert!(
        widened["blocking_questions"]
            .to_string()
            .contains("cannot widen"),
        "{widened}"
    );
    let by_person = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("person-wide".into()),
        actor: ActorKind::Person,
        kind: Some(ChangeKind::Rule),
        record_id: Some("rule.print-should".into()),
        content: Some("Prefer logging to print.".into()),
        rationale: Some("Mechanical, safe to open as a draft PR.".into()),
        definition: Some(Box::new(
            serde_json::from_value(prioritised(rule("should", Some("draft_pr"))))
                .expect("definition"),
        )),
        ..ChangeRequest::default()
    }));
    let token = by_person.resume_token.clone().expect("token");
    let confirmed = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("person-wide".into()),
        actor: ActorKind::Person,
        kind: Some(ChangeKind::Rule),
        record_id: Some("rule.print-should".into()),
        content: Some("Prefer logging to print.".into()),
        rationale: Some("Mechanical, safe to open as a draft PR.".into()),
        definition: Some(Box::new(
            serde_json::from_value(prioritised(rule("should", Some("draft_pr"))))
                .expect("definition"),
        )),
        expected_revision: by_person.expected_revision,
        resume_token: Some(token),
        ..ChangeRequest::default()
    }));
    assert_eq!(
        confirmed.state,
        ServiceState::Success,
        "{}",
        confirmed.summary
    );
    accept(
        root,
        confirmed.data["proposal"]["id"].as_str().expect("proposal"),
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
        let refused = draft_rule(root, name, "rule.print-bad", "Bad.", definition, &[]);
        assert!(
            refused["state"] != "success" && refused["data"]["proposal"].is_null(),
            "{name}: {refused}"
        );
    }
    let doubtful = draft_rule(
        root,
        "doubtful",
        "rule.print-doubtful",
        "Print is a smell.",
        rule("should", None),
        &[],
    );
    accept(
        root,
        doubtful["data"]["proposal"]["id"]
            .as_str()
            .expect("proposal"),
    );
    let must = draft_rule(
        root,
        "must",
        "rule.print-must",
        "No print at all.",
        rule("must", None),
        &[],
    );
    accept(
        root,
        must["data"]["proposal"]["id"].as_str().expect("proposal"),
    );

    // The class shows in wh dash; no class reads as discuss.
    let dash = wh(root, &["dash", "--json"]);
    let field_of = |id: &str, field: &str| {
        dash["data"]["current"]["rules"]
            .as_array()
            .and_then(|rules| rules.iter().find(|rule| rule["id"] == id).cloned())
            .map(|rule| rule[field].clone())
    };
    assert_eq!(
        field_of("rule.print-should", "sweep_class"),
        Some(json!("draft_pr"))
    );
    assert_eq!(
        field_of("rule.print-doubtful", "sweep_class"),
        Some(json!("discuss"))
    );
    assert_eq!(
        field_of("rule.print-must", "sweep_class"),
        Some(json!("discuss"))
    );
    assert_eq!(
        field_of("rule.print-should", "sweep_priority"),
        Some(json!(0))
    );
    assert_eq!(
        field_of("rule.print-doubtful", "sweep_priority"),
        Some(json!(2)),
        "no priority reads as 2"
    );
}
