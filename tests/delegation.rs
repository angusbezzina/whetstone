//! The delegation loop end to end through the real binary: Jev questions
//! against a stub TypeSafe endpoint (what leaves the machine, and what each
//! answer is allowed to mean), raising a hand into Beads, and learning from
//! labelled flags. Every repository is a throwaway; nothing touches this
//! repository's `.beads` or private store.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;
use tempfile::TempDir;

const KEY: &str = "test-key-7f3a9c2e1b";

fn have(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .output()
        .is_ok_and(|output| output.status.success())
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

fn wh(root: &Path, args: &[&str], env: &[(&str, String)]) -> Value {
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
    let output = command.output().expect("whetstone");
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&text).unwrap_or_else(|error| {
        panic!(
            "wh {args:?} did not print JSON ({error}): {text}\n{}",
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
    std::fs::write(root.join("README.md"), "notes\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);
    temp
}

/// Record the agreement with a mission and one starter, so rules can be added.
fn agree(root: &Path) {
    let inspected = wh(root, &["init", "--json", "--no-open"], &[]);
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
            "--mission",
            "Keep notes that stay true",
            "--starter",
            "rule.tests-only-when-asked",
            "--expected-revision",
            &revision,
            "--resume",
            &token,
            "--json",
        ],
        &[],
    );
    assert_eq!(
        agreed["data"]["progress"]["agreement_complete"], true,
        "{agreed}"
    );
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
    let inspected = wh(root, &base, &[]);
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
    let drafted = wh(root, &confirm, &[]);
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .unwrap_or_else(|| panic!("a proposal: {drafted}"))
        .to_string();
    let accepted = wh(
        root,
        &[
            "change",
            "--accept",
            &proposal,
            "--rationale",
            "agreed",
            "--json",
        ],
        &[],
    );
    assert_eq!(accepted["state"], "success", "{accepted}");
    let wired = wh(
        root,
        &["init", "--action", "wire", "--host", "claude", "--json"],
        &[],
    );
    assert_eq!(wired["state"], "success", "{wired}");
    // The generated skill is committed on its own, so the change under test
    // is the only thing Jev is asked about.
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
            rule.insert(key.clone(), value.clone());
        }
    }
    rule
}

fn commit(root: &Path, file: &str, text: &str) {
    std::fs::write(root.join(file), text).expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", &format!("change {file}")]);
}

#[derive(Default)]
struct Seen {
    bodies: Vec<String>,
    authorizations: Vec<String>,
}

/// A stub of TypeSafe's System One endpoint that answers every question with
/// one probability (or one HTTP status) and records what it received.
struct Stub {
    url: String,
    seen: Arc<Mutex<Seen>>,
}

impl Stub {
    fn start(status: u16, probability: f64) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let seen = Arc::new(Mutex::new(Seen::default()));
        let record = Arc::clone(&seen);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut length = 0usize;
                let mut authorization = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                    if lower.starts_with("authorization:") {
                        authorization = line["authorization:".len()..].trim().to_string();
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                if let Ok(mut seen) = record.lock() {
                    seen.bodies.push(String::from_utf8_lossy(&body).to_string());
                    seen.authorizations.push(authorization);
                }
                let payload = if status == 200 {
                    serde_json::json!({
                        "model": "jev-1.13.0",
                        "answers": {"rule": {"noul": probability}},
                        "usage": {"input_tokens": 42}
                    })
                    .to_string()
                } else {
                    serde_json::json!({"detail": "stub failure"}).to_string()
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} STUB\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        Self { url, seen }
    }

    fn env(&self) -> Vec<(&'static str, String)> {
        vec![
            ("TYPESAFE_BASE_URL", self.url.clone()),
            ("TYPESAFE_API_KEY", KEY.to_string()),
        ]
    }

    fn bodies(&self) -> Vec<String> {
        self.seen.lock().expect("seen").bodies.clone()
    }
}

fn gate<'a>(check: &'a Value, rule: &str) -> &'a Value {
    check["data"]["gates"]
        .as_array()
        .and_then(|gates| gates.iter().find(|gate| gate["id"] == rule))
        .unwrap_or_else(|| panic!("no gate for {rule}: {check}"))
}

fn ready() -> bool {
    if !have("node", "--version") {
        assert!(
            std::env::var_os("CI").is_none(),
            "a tool this test needs is missing in CI"
        );
        eprintln!("skipping: node is not installed");
        return false;
    }
    true
}

#[test]
fn jev_sees_only_redacted_text_and_its_flag_breaks_a_must_rule() {
    if !ready() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    agree(root);
    add_rule(
        root,
        "rule.no-todo",
        &question_rule(serde_json::json!({
            "privacy": {"redact_patterns": ["CODENAME-[0-9]+"], "redact_globs": ["secrets/**"]}
        })),
    );
    std::fs::create_dir_all(root.join("secrets")).expect("dir");
    std::fs::write(root.join("secrets/plan.txt"), "launch on the ninth\n").expect("write");
    commit(
        root,
        "notes.py",
        &format!(
            "x = 1  # TODO CODENAME-77 remove\napi_key = \"abcdefgh12345678\"\nhint = \"{KEY}\"\n"
        ),
    );
    let stub = Stub::start(200, 0.93);
    let check = wh(root, &["check", "--base", "HEAD~1", "--json"], &stub.env());

    let bodies = stub.bodies();
    assert!(!bodies.is_empty(), "Jev was asked: {check}");
    for body in &bodies {
        for secret in [
            "CODENAME-77",
            "abcdefgh12345678",
            KEY,
            "launch on the ninth",
        ] {
            assert!(!body.contains(secret), "{secret} left the machine: {body}");
        }
        let request: Value = serde_json::from_str(body).expect("json body");
        assert_eq!(request["questions"]["rule"]["type"], "noul", "{request}");
        assert_eq!(
            request["questions"]["rule"]["instructions"],
            "Does this change add a TODO comment?"
        );
    }
    assert!(
        bodies.iter().any(|body| body.contains("TODO [REDACTED:")),
        "the unit is sent with placeholders: {bodies:?}"
    );
    let authorizations = stub.seen.lock().expect("seen").authorizations.clone();
    assert!(
        authorizations
            .iter()
            .all(|value| value == &format!("Bearer {KEY}")),
        "{authorizations:?}"
    );

    assert_eq!(check["state"], "violated", "{check}");
    let outcome = gate(&check, "rule.no-todo");
    assert_eq!(outcome["state"], "fail", "{outcome}");
    assert!(
        outcome["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("redaction"),
        "the receipt says redaction happened: {outcome}"
    );
    let receipts = check["data"]["judgment_receipts"]
        .as_array()
        .expect("receipts");
    assert!(!receipts.is_empty(), "{check}");

    // The key is never written into the private store or its receipts.
    let trail = wh(root, &["dash", "--trail", "--json"], &[]);
    assert!(
        !trail.to_string().contains(KEY),
        "the key reached the trail"
    );
    let dash = wh(root, &["dash", "--json"], &[]);
    assert!(
        !dash.to_string().contains(KEY),
        "the key reached the dashboard"
    );
}

#[test]
fn a_clear_answer_never_passes_a_must_rule_and_doubt_raises_a_hand() {
    if !ready() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    agree(root);
    add_rule(root, "rule.no-todo", &question_rule(serde_json::json!({})));
    commit(root, "notes.py", "x = 1\n");

    let clear = Stub::start(200, 0.02);
    let check = wh(root, &["check", "--base", "HEAD~1", "--json"], &clear.env());
    assert!(!clear.bodies().is_empty(), "Jev was asked: {check}");
    let outcome = gate(&check, "rule.no-todo");
    assert_ne!(
        outcome["state"], "pass",
        "Jev cannot pass a must rule: {outcome}"
    );
    assert_ne!(check["state"], "satisfied", "{check}");

    commit(root, "more.py", "y = 2\n");
    let unsure = Stub::start(200, 0.55);
    let check = wh(
        root,
        &["check", "--base", "HEAD~1", "--json"],
        &unsure.env(),
    );
    let outcome = gate(&check, "rule.no-todo");
    assert_eq!(outcome["state"], "unknown", "{outcome}");
    assert_eq!(
        outcome["raise_hand"], true,
        "low confidence raises a hand: {outcome}"
    );
}

#[test]
fn an_unreachable_jev_is_unknown_and_a_local_only_rule_never_calls_it() {
    if !ready() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    agree(root);
    add_rule(root, "rule.no-todo", &question_rule(serde_json::json!({})));
    add_rule(
        root,
        "rule.private-todo",
        &question_rule(serde_json::json!({"privacy": {"local_only": true}})),
    );
    commit(root, "notes.py", "x = 1  # TODO\n");

    let broken = Stub::start(503, 0.0);
    let check = wh(
        root,
        &["check", "--base", "HEAD~1", "--json"],
        &broken.env(),
    );
    let outcome = gate(&check, "rule.no-todo");
    assert_eq!(
        outcome["state"], "unknown",
        "unavailable is never a pass: {outcome}"
    );
    assert_ne!(check["state"], "satisfied", "{check}");

    let local = gate(&check, "rule.private-todo");
    assert_ne!(local["state"], "pass", "{local}");
    for body in broken.bodies() {
        assert!(
            !body.contains("private-todo"),
            "a local-only rule reached the network: {body}"
        );
    }
    // Only the shared rule's question reached the endpoint (retries included).
    let asked = broken.bodies().len();
    let offline = wh(
        root,
        &[
            "check",
            "--base",
            "HEAD~1",
            "--rule",
            "rule.private-todo",
            "--json",
        ],
        &broken.env(),
    );
    assert_eq!(
        broken.bodies().len(),
        asked,
        "local-only asked Jev: {offline}"
    );
}

#[test]
fn a_raised_hand_is_a_beads_issue_and_the_answer_is_recorded() {
    if !have("bd", "--version") {
        assert!(
            std::env::var_os("CI").is_none(),
            "a tool this test needs is missing in CI"
        );
        eprintln!("skipping: bd is not installed");
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

    let raised = wh(
        root,
        &[
            "check",
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
        &[],
    );
    let issue = raised["data"]["issue"]
        .as_str()
        .or_else(|| raised["data"]["hand"]["issue"].as_str())
        .unwrap_or_else(|| panic!("an issue id: {raised}"))
        .to_string();
    let listed = Command::new("bd")
        .args(["list", "--label", "human", "--json"])
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect("bd list");
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains(&issue),
        "the issue is labelled human: {}",
        String::from_utf8_lossy(&listed.stdout)
    );

    let dash = wh(root, &["dash", "--json"], &[]);
    let requests = dash["data"]["current"]["requests"]
        .as_array()
        .expect("requests");
    assert!(
        requests
            .iter()
            .any(|request| request["issue"] == issue.as_str()),
        "the dashboard lists the request: {requests:?}"
    );

    let answered = wh(
        root,
        &[
            "change",
            "--answer",
            &issue,
            "--content",
            "Local only for now.",
            "--json",
        ],
        &[],
    );
    assert_eq!(answered["state"], "success", "{answered}");
    let dash = wh(root, &["dash", "--json"], &[]);
    let request = dash["data"]["current"]["requests"]
        .as_array()
        .and_then(|requests| {
            requests
                .iter()
                .find(|request| request["issue"] == issue.as_str())
        })
        .cloned()
        .expect("the request");
    assert_eq!(request["answer"], "Local only for now.", "{request}");
}

/// Commit a flagged change `count` times and label each flag.
fn flag_and_label(root: &Path, rule: &str, count: usize, verdict: &str) {
    for index in 0..count {
        commit(
            root,
            &format!("todo{index}.py"),
            &format!("x = {index}  # TODO\n"),
        );
        let stub = Stub::start(200, 0.97);
        let check = wh(
            root,
            &["check", "--base", "HEAD~1", "--rule", rule, "--json"],
            &stub.env(),
        );
        let receipt = check["data"]["judgment_receipts"]
            .as_array()
            .and_then(|receipts| receipts.first())
            .and_then(|receipt| receipt["id"].as_str())
            .unwrap_or_else(|| panic!("a judgment receipt: {check}"))
            .to_string();
        let flag = format!("--{verdict}-flag");
        let labelled = wh(
            root,
            &[
                "change",
                &flag,
                &receipt,
                "--rationale",
                "labelled in the test",
                "--json",
            ],
            &[],
        );
        assert_eq!(labelled["state"], "success", "{labelled}");
    }
}

#[test]
fn dismissed_flags_propose_a_demotion_and_accepted_flags_propose_hardening() {
    if !ready() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    agree(root);
    add_rule(root, "rule.no-todo", &question_rule(serde_json::json!({})));

    flag_and_label(root, "rule.no-todo", 2, "dismiss");
    let dash = wh(root, &["dash", "--json"], &[]);
    let rule = dash["data"]["current"]["rules"]
        .as_array()
        .and_then(|rules| rules.iter().find(|rule| rule["id"] == "rule.no-todo"))
        .cloned()
        .expect("the rule");
    assert_eq!(rule["stats"]["dismissed"], 2, "{rule}");
    let tuned = wh(root, &["change", "--tune", "--json"], &[]);
    assert_eq!(tuned["state"], "success", "{tuned}");
    let text = tuned.to_string();
    assert!(
        text.contains("demotion") && text.contains("rule.no-todo"),
        "{tuned}"
    );

    // A second rule whose flags keep being right is a hardening candidate.
    let temp = repository();
    let root = temp.path();
    agree(root);
    add_rule(root, "rule.no-todo", &question_rule(serde_json::json!({})));
    flag_and_label(root, "rule.no-todo", 3, "accept");
    let dash = wh(root, &["dash", "--json"], &[]);
    let suggestions = dash["data"]["current"]["suggestions"]
        .as_array()
        .expect("suggestions");
    assert!(
        suggestions
            .iter()
            .any(|suggestion| suggestion["kind"] == "hardening"
                && suggestion["rule"] == "rule.no-todo"),
        "{suggestions:?}"
    );
}

fn head(root: &Path, revision: &str) -> String {
    let output = Command::new("git")
        .args(["rev-parse", revision])
        .current_dir(root)
        .output()
        .expect("git");
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

#[test]
fn a_push_is_checked_only_when_the_working_tree_is_the_pushed_commit() {
    let temp = repository();
    let root = temp.path();
    agree(root);
    commit(root, "notes.py", "x = 1\n");
    let offline = [("WHETSTONE_JEV_OFFLINE", "1".to_string())];

    // Pushing a commit that is not checked out is refused, not vouched for.
    let older = head(root, "HEAD~1");
    let refused = wh(
        root,
        &["check", "--base", "HEAD~1", "--pushed", &older, "--json"],
        &offline,
    );
    assert_eq!(refused["state"], "unknown", "{refused}");
    assert!(
        refused["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("checked out"),
        "{refused}"
    );

    // Uncommitted tracked edits are not what is pushed.
    let tip = head(root, "HEAD");
    std::fs::write(root.join("notes.py"), "x = 2\n").expect("write");
    let dirty = wh(
        root,
        &["check", "--base", "HEAD~1", "--pushed", &tip, "--json"],
        &offline,
    );
    assert_eq!(dirty["state"], "unknown", "{dirty}");
    assert!(
        dirty["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("uncommitted"),
        "{dirty}"
    );

    // With the tree clean, an untracked file is not part of the push.
    git(root, &["checkout", "--", "notes.py"]);
    std::fs::write(root.join("scratch.py"), "print('local only')\n").expect("write");
    let clean = wh(
        root,
        &["check", "--base", "HEAD~1", "--pushed", &tip, "--json"],
        &offline,
    );
    let changed = clean["data"]["selection"]["changed_paths"].to_string();
    assert!(changed.contains("notes.py"), "{clean}");
    assert!(
        !changed.contains("scratch.py"),
        "untracked files are not pushed: {clean}"
    );

    // Only a shadow rule applies: nothing is enforced, so the push is not
    // blocked on it.
    assert_ne!(clean["state"], "unknown", "{clean}");
}
