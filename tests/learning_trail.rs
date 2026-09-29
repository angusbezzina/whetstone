//! Learning and the decision trail through the real binary: a shadow Jev
//! rule is promoted only by an accepted draft (E4), and every kind of owner
//! and agent decision leaves a row in the show-me-your-work trail and the
//! dashboard journal (S7). Jev answers come from a stub TypeSafe endpoint.
//! Every repository is a throwaway; nothing touches this repository's
//! `.beads` or private store.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::thread;

use serde_json::Value;
use tempfile::TempDir;

const KEY: &str = "test-key-5d1e8b0c4a";

fn have(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Node runs the driver that asks Jev and bd holds every record. Without
/// them the test skips locally and fails in CI.
fn ready() -> bool {
    let missing = [("node", "--version"), ("bd", "--version")]
        .into_iter()
        .filter(|(program, arg)| !have(program, arg))
        .map(|(program, _)| program)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return true;
    }
    assert!(
        std::env::var_os("CI").is_none(),
        "CI must have {missing:?} installed"
    );
    eprintln!("skipping: {missing:?} not installed");
    false
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

fn command(root: &Path, args: &[&str], env: &[(&str, String)]) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_whetstone"));
    command
        .args(args)
        .current_dir(root)
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .env("BD_NON_INTERACTIVE", "1")
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .env_remove("TYPESAFE_API_KEY")
        .env_remove("TYPESAFE_BASE_URL")
        .env_remove("WHETSTONE_JEV_OFFLINE");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("whetstone")
}

fn wh(root: &Path, args: &[&str], env: &[(&str, String)]) -> Value {
    let output = command(root, args, env);
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "wh {args:?} did not print JSON ({error}): {text}\n{}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn offline() -> Vec<(&'static str, String)> {
    vec![("WHETSTONE_JEV_OFFLINE", "1".to_string())]
}

fn repository() -> TempDir {
    let temp = TempDir::new().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("README.md"), "notes\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);
    temp
}

fn agree(root: &Path) {
    let inspected = wh(
        root,
        &["init", "--json", "--no-open", "--request-id", "agree"],
        &offline(),
    );
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = wh(
        root,
        &[
            "init",
            "--action",
            "agree",
            "--request-id",
            "agree",
            "--mission",
            "Keep notes that stay true",
            "--starter",
            "rule.ask-before-public-api",
            "--expected-revision",
            &revision,
            "--resume",
            &token,
            "--json",
        ],
        &offline(),
    );
    assert_eq!(
        agreed["data"]["progress"]["agreement_complete"], true,
        "{agreed}"
    );
}

fn accept(root: &Path, proposal: &str) {
    let accepted = wh(
        root,
        &[
            "change",
            "--request-id",
            &format!("accept-{proposal}"),
            "--accept",
            proposal,
            "--rationale",
            "agreed",
            "--json",
        ],
        &offline(),
    );
    assert_eq!(accepted["state"], "success", "{accepted}");
}

/// Draft a rule, accept it, and wire the driver that asks Jev.
fn add_rule(root: &Path, id: &str, definition: &Value) {
    let definition = definition.to_string();
    let request = format!("add-{id}");
    let base = [
        "change",
        "--json",
        "--request-id",
        request.as_str(),
        "--kind",
        "rule",
        "--record-id",
        id,
        "--content",
        "No TODO lands in shipped code.",
        "--rationale",
        "TODOs are promises nobody keeps.",
        "--definition",
        definition.as_str(),
    ];
    let inspected = wh(root, &base, &offline());
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let mut confirm = base.to_vec();
    confirm.extend(["--expected-revision", &revision, "--resume", &token]);
    let drafted = wh(root, &confirm, &offline());
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a proposal: {drafted}"))
        .to_string();
    accept(root, &proposal);
    let wired = wh(
        root,
        &["init", "--action", "wire", "--host", "claude", "--json"],
        &offline(),
    );
    assert_eq!(wired["state"], "success", "{wired}");
    git(root, &["add", "."]);
    git(
        root,
        &["commit", "-qm", &format!("wire {id}"), "--allow-empty"],
    );
}

fn question_rule(extra: Value) -> Value {
    let mut rule = serde_json::json!({
        "type": "rule",
        "strength": "must",
        "enforcer": {
            "kind": "question",
            "question": "Does this change add a TODO comment?",
            "shadow": false
        }
    });
    if let (Some(rule), Some(extra)) = (rule.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            match (rule.get_mut(key), value) {
                (Some(Value::Object(existing)), Value::Object(more)) => {
                    for (inner, value) in more {
                        existing.insert(inner.clone(), value.clone());
                    }
                }
                _ => {
                    rule.insert(key.clone(), value.clone());
                }
            }
        }
    }
    rule
}

fn shadow_rule() -> Value {
    question_rule(serde_json::json!({
        "enforcer": {"shadow": true, "promote_after": 2, "precision_bar_bp": 5000}
    }))
}

fn commit(root: &Path, file: &str, text: &str) {
    std::fs::write(root.join(file), text).expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", &format!("change {file}")]);
}

/// A stub of TypeSafe's System One endpoint that answers every question with
/// one probability.
struct Stub {
    url: String,
}

impl Stub {
    fn start(probability: f64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let payload = serde_json::json!({
                    "model": "jev-1.13.0",
                    "answers": {"rule": {"noul": probability}},
                    "usage": {"input_tokens": 42}
                })
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 STUB\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        Self { url }
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("TYPESAFE_BASE_URL", self.url.clone()),
            ("TYPESAFE_API_KEY", KEY.to_string()),
        ]
    }
}

fn gate<'a>(check: &'a Value, rule: &str) -> &'a Value {
    check["data"]["gates"]
        .as_array()
        .and_then(|gates| gates.iter().find(|gate| gate["id"] == rule))
        .unwrap_or_else(|| panic!("no gate for {rule}: {check}"))
}

fn rule_view(root: &Path, id: &str) -> Value {
    let dash = wh(root, &["dash", "--json"], &offline());
    dash["data"]["current"]["rules"]
        .as_array()
        .and_then(|rules| rules.iter().find(|rule| rule["id"] == id))
        .cloned()
        .unwrap_or_else(|| panic!("no rule {id}: {dash}"))
}

/// Ask Jev (stubbed at `probability`) about the latest commit for `rule`.
fn ask(root: &Path, rule: &str, probability: f64) -> Value {
    let stub = Stub::start(probability);
    wh(
        root,
        &["check", "--base", "HEAD~1", "--rule", rule, "--json"],
        &stub.env(),
    )
}

fn first_receipt(check: &Value) -> String {
    check["data"]["judgment_receipts"]
        .as_array()
        .and_then(|receipts| receipts.first())
        .and_then(|receipt| receipt["id"].as_str())
        .unwrap_or_else(|| panic!("a judgment receipt: {check}"))
        .to_string()
}

fn label(root: &Path, receipt: &str, verdict: &str) {
    let flag = format!("--{verdict}-flag");
    let labelled = wh(
        root,
        &[
            "change",
            "--request-id",
            &format!("label-{receipt}"),
            &flag,
            receipt,
            "--rationale",
            &format!("labelled {verdict} in the test"),
            "--json",
        ],
        &offline(),
    );
    assert_eq!(labelled["state"], "success", "{labelled}");
}

/// Run `wh change --tune` and return the proposal of the draft of `kind` for `rule`.
fn tuned_draft(tuned: &Value, kind: &str, rule: &str) -> String {
    tuned["data"]["drafts"]
        .as_array()
        .and_then(|drafts| {
            drafts
                .iter()
                .find(|draft| draft["kind"] == kind && draft["record"]["id"] == rule)
        })
        .and_then(|draft| draft["proposal"]["id"].as_str())
        .unwrap_or_else(|| panic!("a {kind} draft for {rule}: {tuned}"))
        .to_string()
}

#[test]
fn a_shadow_rule_is_enforced_only_after_its_promotion_draft_is_accepted() {
    if !ready() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    agree(root);
    add_rule(root, "rule.no-todo", &shadow_rule());
    assert_eq!(rule_view(root, "rule.no-todo")["shadow"], true);

    // Flag it across two commits; in shadow, a flag never fails the check.
    for index in 0..2 {
        commit(
            root,
            &format!("todo{index}.py"),
            &format!("x = {index}  # TODO\n"),
        );
        let check = ask(root, "rule.no-todo", 0.95);
        assert_ne!(check["state"], "violated", "{check}");
        let outcome = gate(&check, "rule.no-todo");
        assert_eq!(outcome["shadow"], true, "{outcome}");
        assert_ne!(outcome["state"], "pass", "{outcome}");
        label(root, &first_receipt(&check), "accept");
    }

    let tuned = wh(
        root,
        &["change", "--request-id", "tune-1", "--tune", "--json"],
        &offline(),
    );
    assert_eq!(tuned["state"], "success", "{tuned}");
    let promotion = tuned_draft(&tuned, "promotion", "rule.no-todo");

    // A draft is not a decision: the rule stays in shadow.
    assert_eq!(rule_view(root, "rule.no-todo")["shadow"], true);
    commit(root, "todo-pending.py", "y = 1  # TODO\n");
    let pending = ask(root, "rule.no-todo", 0.95);
    assert_ne!(pending["state"], "violated", "{pending}");
    assert_eq!(gate(&pending, "rule.no-todo")["shadow"], true);

    accept(root, &promotion);
    assert_eq!(rule_view(root, "rule.no-todo")["shadow"], false);
    commit(root, "todo-after.py", "z = 1  # TODO\n");
    let enforced = ask(root, "rule.no-todo", 0.95);
    assert_eq!(enforced["state"], "violated", "{enforced}");
    let outcome = gate(&enforced, "rule.no-todo");
    assert_eq!(outcome["state"], "fail", "{outcome}");
    assert_eq!(outcome["shadow"], false, "{outcome}");
}

/// The trail as TSV rows of (phase, decision, why, evidence, result).
fn trail_rows(root: &Path) -> Vec<Vec<String>> {
    let output = command(root, &["dash", "--trail"], &offline());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).expect("utf-8 trail");
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("ts\tphase\tdecision\twhy\tevidence\tresult")
    );
    lines
        .map(|line| {
            let cells = line.split('\t').map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(cells.len(), 6, "a trail row has six cells: {line}");
            cells[1..].to_vec()
        })
        .collect()
}

fn has_row(rows: &[Vec<String>], phase: &str, decision: &str, rest: &str) -> bool {
    rows.iter().any(|row| {
        row[0] == phase && row[1].contains(decision) && row[1..].join("\t").contains(rest)
    })
}

#[test]
fn every_kind_of_decision_leaves_a_trail_row_and_a_journal_entry() {
    if !ready() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    let output = Command::new("bd")
        .args(["init", "--prefix", "t", "--quiet"])
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .env("BD_NON_INTERACTIVE", "1")
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("bd");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    agree(root);
    // A must rule whose flags turn out noisy (demoted), with redaction.
    add_rule(
        root,
        "rule.no-todo",
        &question_rule(serde_json::json!({
            "privacy": {"redact_patterns": ["CODENAME-[0-9]+"]}
        })),
    );
    // A shadow rule whose flags turn out right (promoted).
    add_rule(root, "rule.shadow-todo", &shadow_rule());

    for index in 0..2 {
        commit(
            root,
            &format!("todo{index}.py"),
            &format!("x = {index}  # TODO CODENAME-{index}7\n"),
        );
        let noisy = ask(root, "rule.no-todo", 0.95);
        assert_eq!(gate(&noisy, "rule.no-todo")["state"], "fail", "{noisy}");
        assert!(
            gate(&noisy, "rule.no-todo")["summary"]
                .as_str()
                .unwrap_or_default()
                .contains("redaction"),
            "{noisy}"
        );
        label(root, &first_receipt(&noisy), "dismiss");
        let right = ask(root, "rule.shadow-todo", 0.95);
        label(root, &first_receipt(&right), "accept");
    }
    let tuned = wh(
        root,
        &["change", "--request-id", "tune-1", "--tune", "--json"],
        &offline(),
    );
    assert_eq!(tuned["state"], "success", "{tuned}");
    let demotion = tuned_draft(&tuned, "demotion", "rule.no-todo");
    let promotion = tuned_draft(&tuned, "promotion", "rule.shadow-todo");
    accept(root, &demotion);
    accept(root, &promotion);
    assert_eq!(rule_view(root, "rule.no-todo")["strength"], "should");
    assert_eq!(rule_view(root, "rule.shadow-todo")["shadow"], false);

    let brief = wh(
        root,
        &[
            "check",
            "--request-id",
            "brief-1",
            "--brief",
            "--area",
            "notes",
            "--skill",
            "how",
            "--reuse",
            "the notes module",
            "--risk",
            "note parsing",
            "--notes",
            "How notes are stored.",
            "--json",
        ],
        &offline(),
    );
    assert_eq!(brief["state"], "success", "{brief}");
    let attested = wh(
        root,
        &[
            "check",
            "--request-id",
            "attest-1",
            "--attest",
            "rule.no-todo",
            "--verdict",
            "pass",
            "--reviewer",
            "Ada",
            "--notes",
            "Read the change; no TODO.",
            "--json",
        ],
        &offline(),
    );
    assert_eq!(attested["state"], "success", "{attested}");
    let raised = wh(
        root,
        &[
            "check",
            "--request-id",
            "hand-1",
            "--raise-hand",
            "--question",
            "Should notes sync to the cloud?",
            "--tried",
            "Read the mission; it says nothing about sync.",
            "--recommend",
            "Keep notes local until the owner decides.",
            "--trigger",
            "vague-spec",
            "--json",
        ],
        &offline(),
    );
    let issue = raised["data"]["issue"]
        .as_str()
        .or_else(|| raised["data"]["hand"]["issue"].as_str())
        .unwrap_or_else(|| panic!("an issue id: {raised}"))
        .to_string();
    let answered = wh(
        root,
        &[
            "change",
            "--request-id",
            "answer-1",
            "--answer",
            &issue,
            "--content",
            "Local only for now.",
            "--json",
        ],
        &offline(),
    );
    assert_eq!(answered["state"], "success", "{answered}");

    let rows = trail_rows(root);
    let dump = rows
        .iter()
        .map(|row| row.join(" | "))
        .collect::<Vec<_>>()
        .join("\n");
    for (what, phase, decision, rest) in [
        (
            "hand raise",
            "hands",
            "Raised a hand (",
            "Should notes sync",
        ),
        (
            "hand answer",
            "hands",
            &*format!("Answered {issue}"),
            "Local only",
        ),
        (
            "flag accept",
            "rules",
            "Flag accepted on rule.shadow-todo",
            "true flag",
        ),
        (
            "flag dismiss",
            "rules",
            "Flag dismissed on rule.no-todo",
            "false flag",
        ),
        (
            "strength change",
            "rules",
            "strength must → should",
            "accepted",
        ),
        (
            "shadow promotion",
            "rules",
            "promoted from shadow",
            "accepted",
        ),
        (
            "Jev redaction",
            "checks",
            "Asked Jev about",
            "redacted sha256:",
        ),
        ("brief", "brief", "Brief for notes", "ran how"),
        (
            "attestation",
            "review",
            "Review of rule.no-todo by Ada",
            "pass",
        ),
    ] {
        assert!(
            has_row(&rows, phase, decision, rest),
            "no trail row for the {what} ({phase}: {decision} … {rest}):\n{dump}"
        );
    }

    let dash = wh(root, &["dash", "--json"], &offline());
    let journal = dash["data"]["changelog"]
        .as_array()
        .unwrap_or_else(|| panic!("a journal: {dash}"));
    let entries = journal
        .iter()
        .map(|entry| {
            format!(
                "{} | {} | {}",
                entry["title"].as_str().unwrap_or_default(),
                entry["note"].as_str().unwrap_or_default(),
                entry["summary"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    let listing = entries.join("\n");
    for (what, needle) in [
        ("hand raise", "Raised a hand: Should notes sync"),
        ("hand answer", &*format!("Answered {issue}")),
        ("flag accept", "Flag accepted on rule.shadow-todo"),
        ("flag dismiss", "Flag dismissed on rule.no-todo"),
        ("strength change", "strength must → should"),
        ("shadow promotion", "promoted from shadow"),
        ("Jev receipt", "rule.no-todo (Jev)"),
        ("brief", "Brief recorded: notes"),
        ("attestation", "Review attested: rule.no-todo"),
    ] {
        assert!(
            entries.iter().any(|entry| entry.contains(needle)),
            "no journal entry for the {what} ({needle}):\n{listing}"
        );
    }
    // The check run that asked about the redacted rule says so in its title.
    assert!(
        journal.iter().any(|entry| {
            entry["title"].as_str().is_some_and(|title| {
                title.starts_with("Checks ran:") && title.contains("redacted before")
            }) && entry["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("rule.no-todo (Jev)"))
        }),
        "no journal check run says it redacted before asking Jev:\n{listing}"
    );
    // The key never reaches the trail or the journal.
    assert!(!dump.contains(KEY) && !listing.contains(KEY));
}
