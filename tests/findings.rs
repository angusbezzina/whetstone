//! Sweep findings: `wh check --sweep` turns every flag of a rule below must
//! into one finding a night sweep's controller works. The sweep reads the
//! rules from `whetstone/rulebook.json` and runs without `bd`; the sweep
//! fields a person or an agent sets through `wh change` are covered against
//! the record store. Every repository is a throwaway; nothing touches this
//! repository's stores.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use whetstone::domain::ActorKind;
use whetstone::service::{ChangeKind, ChangeRequest, CommandService, ServiceRequest, ServiceState};

#[path = "support/rulebook.rs"]
mod rulebook;
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

/// The binary with only the system tools on `PATH`: no `bd` to start.
fn run_without_bd(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(args)
        .current_dir(root)
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .output()
        .expect("whetstone")
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no JSON envelope ({error}): {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
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

/// One finding as findings.json carries it, byte for byte.
fn finding_line(
    rule: &str,
    subject: &str,
    flagged: &str,
    class: &str,
    priority: u8,
    title: &str,
    unsure: Option<&str>,
) -> (String, String) {
    let fingerprint = sha256(format!("{rule}\0github.com/example/app\0{subject}").as_bytes());
    let unsure = unsure.map_or_else(String::new, |reason| format!(",\"unsure\":\"{reason}\""));
    let line = format!(
        "{{\"fingerprint\":\"{fingerprint}\",\"subjectDigest\":\"{}\",\"class\":\"{class}\",\"priority\":{priority},\"title\":\"{title}\",\"responsibility\":\"{rule}\"{unsure}}}",
        sha256(flagged.as_bytes())
    );
    (fingerprint, line)
}

/// findings.json for these findings: a JSON array, one finding per line,
/// sorted by fingerprint.
fn findings_json(mut findings: Vec<(String, String)>) -> String {
    findings.sort();
    let lines = findings
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

#[test]
fn a_sweep_reads_the_rulebook_file_and_its_findings_are_byte_stable_and_below_must() {
    let temp = repository();
    let root = temp.path();
    git(
        root,
        &["remote", "add", "origin", "git@github.com:example/app.git"],
    );
    std::fs::write(root.join("src/app.py"), "print('one')\nvalue = 1\n").expect("write");
    let mut should = print_rule("should");
    should["sweep"] = json!("draft_pr");
    should["sweep_priority"] = json!(0);
    rulebook::add_rule(
        root,
        "rule.print-should",
        "Prefer logging to print.",
        &should,
    );
    let mut doubtful = print_rule("should");
    doubtful["sweep_unsure"] = json!([{"matching": "print\\('one'\\)", "reason": "a one-off print in a demo is borderline"}]);
    rulebook::add_rule(root, "rule.print-doubtful", "Print is a smell.", &doubtful);
    rulebook::add_rule(
        root,
        "rule.print-must",
        "No print at all.",
        &print_rule("must"),
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "rules"]);

    // Findings for the rules below must only (a sweep prints them by
    // default), byte-identical across runs and to the findings contract:
    // fingerprint sha256(rule \0 origin \0 subject), the flagged line's
    // digest, class, priority, title, responsibility and unsure.
    let first = run_without_bd(root, &["check", "--sweep"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = run_without_bd(root, &["check", "--sweep", "--format", "findings"]);
    assert_eq!(
        first.stdout, second.stdout,
        "the same state gives the same findings"
    );
    let should_one = finding_line(
        "rule.print-should",
        "src/app.py",
        "print('one')",
        "draft_pr",
        0,
        "Prefer logging to print. (src/app.py)",
        None,
    );
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        findings_json(vec![
            should_one.clone(),
            finding_line(
                "rule.print-doubtful",
                "src/app.py",
                "print('one')",
                "discuss",
                2,
                "Print is a smell. (src/app.py)",
                Some("a one-off print in a demo is borderline"),
            ),
        ]),
        "a must rule never emits"
    );
    // One JSON document (Messo's `messo-ctl observe` parses the whole
    // findings.json).
    serde_json::from_slice::<Vec<Value>>(&first.stdout).expect("findings are one JSON array");

    // The fingerprint holds while the flagged content changes; the digest
    // moves, and the doubt covers only the flags its pattern matches. A
    // repeat in the same subject is numbered in source order.
    std::fs::write(
        root.join("src/app.py"),
        "print('two')\nvalue = 1\nprint('two')\n",
    )
    .expect("write");
    let changed = run_without_bd(root, &["check", "--sweep"]);
    let should_two = finding_line(
        "rule.print-should",
        "src/app.py",
        "print('two')",
        "draft_pr",
        0,
        "Prefer logging to print. (src/app.py)",
        None,
    );
    assert_eq!(should_two.0, should_one.0);
    assert_ne!(should_two.1, should_one.1);
    let doubtful_two = |subject: &str| {
        finding_line(
            "rule.print-doubtful",
            subject,
            "print('two')",
            "discuss",
            2,
            "Print is a smell. (src/app.py)",
            None,
        )
    };
    assert_eq!(
        String::from_utf8_lossy(&changed.stdout),
        findings_json(vec![
            should_two,
            finding_line(
                "rule.print-should",
                "src/app.py#2",
                "print('two')",
                "draft_pr",
                0,
                "Prefer logging to print. (src/app.py)",
                None,
            ),
            doubtful_two("src/app.py"),
            doubtful_two("src/app.py#2"),
        ])
    );

    // A should flag is never required; the must rule's is. Each result
    // names its rule by id, digest and the rulebook's revision.
    let checked = envelope(&run_without_bd(root, &["check", "--json"]));
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("git");
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    assert_eq!(
        checked["data"]["rulebook"],
        json!({"path": "whetstone/rulebook.json", "revision": head})
    );
    let result = |id: &str| {
        checked["data"]["report"]["results"]
            .as_array()
            .and_then(|results| {
                results
                    .iter()
                    .find(|result| result["requirement_id"] == format!("gate:{id}"))
            })
            .cloned()
            .unwrap_or_else(|| panic!("{id} checked: {}", checked["data"]["report"]))
    };
    assert_eq!(result("rule.print-should")["required"], json!(false));
    assert_eq!(result("rule.print-must")["required"], json!(true));
    let governing = &result("rule.print-must")["governing_rule"];
    assert_eq!(governing["id"], "rule.print-must");
    assert_eq!(governing["revision"], json!(head));
    assert!(
        governing["digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:")),
        "{governing}"
    );

    // `--format json` keeps the envelope, with the same findings in it.
    let enveloped = envelope(&run_without_bd(
        root,
        &["check", "--sweep", "--format", "json"],
    ));
    assert_eq!(enveloped["schema"], "whetstone.command-response.v1");
    assert_eq!(
        enveloped["data"]["findings_json"]
            .as_str()
            .map(str::as_bytes),
        Some(changed.stdout.as_slice()),
        "{enveloped}"
    );
}

#[test]
fn a_check_without_a_rulebook_or_with_a_malformed_one_never_passes() {
    let temp = repository();
    let root = temp.path();
    for args in [
        &["check", "--json"][..],
        &["check", "--ci", "--json"],
        &["check", "--staged", "--json"],
        &["check", "--sweep", "--format", "json"],
    ] {
        let response = envelope(&run_without_bd(root, args));
        assert_eq!(response["state"], "unknown", "{args:?}: {response}");
        assert!(
            response["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("whetstone/rulebook.json does not exist")),
            "{response}"
        );
        assert!(
            response["permitted_actions"][0]
                .as_str()
                .is_some_and(|action| action.contains("wh change")
                    && action.contains("wh init --action export")),
            "{response}"
        );
    }
    let swept = run_without_bd(root, &["check", "--sweep"]);
    assert_ne!(swept.status.code(), Some(0));
    assert!(swept.stdout.is_empty(), "no findings.json without rules");
    assert!(String::from_utf8_lossy(&swept.stderr).contains("does not exist"));

    // A local check reads the file as it is; CI only a committed one.
    rulebook::add_rule(
        root,
        "rule.print-must",
        "No print at all.",
        &print_rule("must"),
    );
    let local = envelope(&run_without_bd(root, &["check", "--json"]));
    assert_eq!(local["state"], "success", "{local}");
    let ci = envelope(&run_without_bd(root, &["check", "--ci", "--json"]));
    assert_eq!(ci["state"], "unknown", "{ci}");
    assert!(
        ci["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("uncommitted changes")),
        "{ci}"
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "rules"]);
    let ci = envelope(&run_without_bd(root, &["check", "--ci", "--json"]));
    assert_eq!(ci["state"], "success", "{ci}");
    let unknown = envelope(&run_without_bd(
        root,
        &["check", "--json", "--rule", "rule.print-elsewhere"],
    ));
    assert_eq!(unknown["state"], "unknown", "{unknown}");
    assert!(unknown["summary"]
        .as_str()
        .is_some_and(|summary| summary.contains("rule.print-elsewhere")));

    // A malformed rulebook names the field.
    for (name, extra, field) in [
        ("priority", json!({"sweep_priority": 5}), "rules[1]"),
        ("unknown field", json!({"colour": "red"}), "colour"),
    ] {
        let mut bad = print_rule("should");
        for (key, value) in extra.as_object().expect("object") {
            bad[key] = value.clone();
        }
        git(root, &["checkout", "-q", "--", "whetstone/rulebook.json"]);
        rulebook::add_rule(root, "rule.print-bad", "Bad.", &bad);
        let response = envelope(&run_without_bd(root, &["check", "--json"]));
        assert_eq!(response["state"], "unknown", "{name}: {response}");
        assert!(
            response["summary"].as_str().is_some_and(|summary| summary
                .contains("whetstone/rulebook.json is not a rulebook")
                && summary.contains(field)),
            "{name}: {response}"
        );
    }
}

#[test]
fn a_sweep_covers_a_large_repository_with_in_repository_symlinks() {
    let temp = repository();
    let root = temp.path();
    let bulk = root.join("bulk");
    std::fs::create_dir(&bulk).expect("bulk");
    for index in 0..4_200 {
        std::fs::write(bulk.join(format!("{index}.txt")), "x\n").expect("write");
    }
    std::fs::write(root.join("src/app.py"), "print('one')\n").expect("write");
    std::os::unix::fs::symlink("src", root.join("alias")).expect("directory link");
    std::os::unix::fs::symlink("README.md", root.join("NOTES.md")).expect("file link");
    rulebook::add_rule(
        root,
        "rule.print-should",
        "Prefer logging to print.",
        &print_rule("should"),
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "bulk"]);

    let swept = run_without_bd(root, &["check", "--sweep"]);
    assert_eq!(
        swept.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&swept.stdout),
        String::from_utf8_lossy(&swept.stderr)
    );
    let findings: Vec<Value> = serde_json::from_slice(&swept.stdout).expect("findings");
    assert!(
        findings
            .iter()
            .any(|finding| finding["responsibility"] == "rule.print-should"),
        "{findings:?}"
    );
}
