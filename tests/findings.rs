//! Sweep findings: `wh check --sweep --format findings` turns every flag of
//! a rule below must into one finding a night sweep's controller works. The
//! fixture holds only rules; the agent is the real binary and the person is
//! the library's service called with a person as the actor. Every repository
//! is a throwaway; nothing touches this repository's stores.

use std::collections::BTreeSet;
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

#[test]
fn a_rules_sweep_class_round_trips_and_sweep_findings_are_stable_and_below_must() {
    if !have_bd() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    std::fs::write(root.join("src/app.py"), "print('one')\nvalue = 1\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "app"]);
    let query = "((call function: (identifier) @f) @match (#eq? @f \"print\"))";
    let prioritised = |mut definition: Value| {
        definition["sweep_priority"] = json!(0);
        definition
    };
    let rule = |strength: &str, sweep: Option<&str>| {
        let mut definition = json!({
            "type": "rule",
            "strength": strength,
            "enforcer": {"kind": "ast", "language": "python", "query": query},
        });
        if let Some(sweep) = sweep {
            definition["sweep"] = json!(sweep);
        }
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
    let mut doubtful = rule("should", None);
    doubtful["sweep_unsure"] = json!([{"matching": "print\\('one'\\)", "reason": "a one-off print in a demo is borderline"}]);
    let doubtful = draft_rule(
        root,
        "doubtful",
        "rule.print-doubtful",
        "Print is a smell.",
        doubtful,
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
    let class_of = |id: &str| {
        dash["data"]["current"]["rules"]
            .as_array()
            .and_then(|rules| rules.iter().find(|rule| rule["id"] == id).cloned())
            .map(|rule| rule["sweep_class"].clone())
    };
    assert_eq!(class_of("rule.print-should"), Some(json!("draft_pr")));
    assert_eq!(class_of("rule.print-doubtful"), Some(json!("discuss")));
    assert_eq!(class_of("rule.print-must"), Some(json!("discuss")));
    let priority_of = |id: &str| {
        dash["data"]["current"]["rules"]
            .as_array()
            .and_then(|rules| rules.iter().find(|rule| rule["id"] == id).cloned())
            .map(|rule| rule["sweep_priority"].clone())
    };
    assert_eq!(priority_of("rule.print-should"), Some(json!(0)));
    assert_eq!(
        priority_of("rule.print-doubtful"),
        Some(json!(2)),
        "no priority reads as 2"
    );

    // Findings for the rules below must only, byte-identical across runs.
    let first = run(root, &["check", "--sweep", "--format", "findings"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = run(root, &["check", "--sweep", "--format", "findings"]);
    assert_eq!(
        first.stdout, second.stdout,
        "the same state gives the same findings"
    );
    // One JSON document (Messo's `messo-ctl observe` parses the whole
    // findings.json), one finding per line.
    let findings: Vec<Value> =
        serde_json::from_slice(&first.stdout).expect("findings are one JSON array");
    assert_eq!(
        String::from_utf8_lossy(&first.stdout).lines().count(),
        findings.len() + 2,
        "one finding per line"
    );
    assert_eq!(findings.len(), 2, "{findings:?}");
    for finding in &findings {
        let keys = finding
            .as_object()
            .expect("object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut expected = [
            "class",
            "fingerprint",
            "priority",
            "responsibility",
            "subjectDigest",
            "title",
        ]
        .map(String::from)
        .into_iter()
        .collect::<BTreeSet<_>>();
        if finding["responsibility"] == "rule.print-doubtful" {
            expected.insert("unsure".into());
        }
        assert_eq!(keys, expected, "{finding}");
        assert!(
            ["draft_pr", "confirm_first", "discuss"]
                .contains(&finding["class"].as_str().expect("class")),
            "{finding}"
        );
        assert!(
            finding["priority"]
                .as_u64()
                .is_some_and(|priority| priority <= 4),
            "{finding}"
        );
        for key in ["fingerprint", "subjectDigest"] {
            assert!(
                finding[key]
                    .as_str()
                    .is_some_and(|value| value.starts_with("sha256:")),
                "{key}: {finding}"
            );
        }
        assert!(
            finding["title"]
                .as_str()
                .is_some_and(|title| !title.is_empty()),
            "{finding}"
        );
        assert_ne!(
            finding["responsibility"], "rule.print-must",
            "a must rule never emits"
        );
    }
    let by_rule = |id: &str| {
        findings
            .iter()
            .find(|finding| finding["responsibility"] == id)
            .cloned()
            .expect(id)
    };
    assert_eq!(by_rule("rule.print-should")["class"], "draft_pr");
    assert_eq!(by_rule("rule.print-doubtful")["class"], "discuss");
    assert_eq!(by_rule("rule.print-should")["priority"], 0);
    assert_eq!(by_rule("rule.print-doubtful")["priority"], 2);
    assert_eq!(
        by_rule("rule.print-doubtful")["unsure"],
        "a one-off print in a demo is borderline"
    );

    // The fingerprint holds while the flagged content changes; the digest moves.
    let before = by_rule("rule.print-should");
    std::fs::write(root.join("src/app.py"), "print('two')\nvalue = 1\n").expect("write");
    let changed = run(root, &["check", "--sweep", "--format", "findings"]);
    let after = serde_json::from_slice::<Vec<Value>>(&changed.stdout)
        .expect("findings")
        .into_iter()
        .find(|finding| finding["responsibility"] == "rule.print-should")
        .expect("still flagged");
    assert_eq!(after["fingerprint"], before["fingerprint"]);
    assert_ne!(after["subjectDigest"], before["subjectDigest"]);
    let doubtful_after = serde_json::from_slice::<Vec<Value>>(&changed.stdout)
        .expect("findings")
        .into_iter()
        .find(|finding| finding["responsibility"] == "rule.print-doubtful")
        .expect("still flagged");
    assert!(
        doubtful_after.get("unsure").is_none(),
        "the doubt covers only the flags its pattern matches: {doubtful_after}"
    );

    // A should flag is never required; the must rule's is.
    let checked = wh(root, &["check", "--json"]);
    let required = |id: &str| {
        checked["data"]["report"]["results"]
            .as_array()
            .and_then(|results| {
                results
                    .iter()
                    .find(|result| result["requirement_id"] == format!("gate:{id}"))
            })
            .map(|result| result["required"].clone())
            .unwrap_or_else(|| panic!("{id} checked: {}", checked["data"]["report"]))
    };
    assert_eq!(required("rule.print-should"), json!(false));
    assert_eq!(required("rule.print-must"), json!(true));

    // The ordinary feature sweep is unchanged: it drives features only.
    let sweep = wh(root, &["check", "--sweep", "--json"]);
    assert!(sweep["data"]["findings"].is_null(), "{}", sweep["data"]);
}
