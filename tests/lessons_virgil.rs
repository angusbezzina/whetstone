//! The Virgil backend (whetstone-jpd.2, .5): with the memory switch naming a
//! Virgil group, lessons live in the group's lesson collection and raised
//! hands are tasks in its channel, reached only through `virgil lessons` and
//! `virgil tasks`. A fake `virgil` on PATH stands in until Off Beads step 2a
//! ships the real `virgil lessons` commands; it logs every call.
//!
//! One test in its own binary, because it puts the fake first on this
//! process's PATH for the person's in-process calls.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::ActorKind;
use whetstone::service::{ChangeRequest, CommandService, LessonOp, ServiceRequest};

const GROUP: &str = "01J8Z0000000000000000000AD";

/// A small stand-in for the daemon's CLI: one JSON request on stdin, one
/// JSON line out, state in a file, every argv logged.
const FAKE_VIRGIL: &str = r#"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
with open(os.environ["FAKE_VIRGIL_LOG"], "a") as log:
    log.write(json.dumps(args) + "\n")
family, op = args[0], args[1]
request = json.loads(sys.stdin.read() or "{}")
path = os.environ["FAKE_VIRGIL_STATE"]
state = json.load(open(path)) if os.path.exists(path) else {"lessons": {}, "tasks": {}, "n": 0}
api = "virgil." + family + "/v1"
def done(result):
    json.dump(state, open(path, "w"))
    print(json.dumps({"api_version": api, "result": result}))
    sys.exit(0)
def fail(code, message, exit_code):
    print(json.dumps({"api_version": api, "error": {"code": code, "message": message, "hint": ""}}))
    sys.exit(exit_code)
def stamp():
    state["n"] += 1
    return {"who": "principal:01J8Z0000000000000000000PR", "actor": "agent", "at": "2026-10-13T09:00:%02dZ" % (state["n"] % 60)}
lessons = state["lessons"]
if family == "lessons":
    if op in ("find", "proposals"):
        found = list(lessons.values())
        if op == "proposals":
            found = [l for l in found if l["status"] == "proposed" or l.get("forget_proposed")]
        done({"lessons": found})
    if op in ("remember", "supersede"):
        body = request["lesson"]
        for field in ("provenance", "stamp", "trust", "status", "decision"):
            if field in body:
                fail("provenance_supplied", field + " is stamped by the daemon", 2)
        if body["key"] in lessons:
            fail("exists", "lesson exists", 2)
        lesson = dict(body)
        lesson.update({"id": "lesson:%03d" % (len(lessons) + 1), "revision": 1, "provenance": stamp(), "trust": "inferred", "status": "proposed"})
        if op == "supersede":
            lesson["supersedes"] = request["key"]
        lesson = {k: v for k, v in lesson.items() if v is not None}
        lessons[body["key"]] = lesson
        done({"lesson": lesson})
    lesson = lessons.get(request.get("key"))
    if lesson is None:
        fail("not_found", "no such lesson", 2)
    decision = {"by": "Owner", "at": "2026-10-13T10:00:00Z", "rationale": request.get("rationale") or request.get("reason") or "decided"}
    if op == "admit":
        receipt = "receipt:admit-%s" % lesson["key"]
        lesson.update({"status": "admitted", "trust": request.get("trust", "validated"), "decision": dict(decision, verb="admit", receipt=receipt), "revision": lesson["revision"] + 1, "provenance": stamp()})
        old = lessons.get(lesson.get("supersedes") or "")
        if old:
            old.update({"status": "superseded", "superseded_by": lesson["key"], "decision": dict(decision, verb="supersede"), "revision": old["revision"] + 1})
        done({"lesson": lesson, "receipt": receipt})
    if op == "forget":
        if request.get("propose"):
            lesson["forget_proposed"] = {"reason": request["reason"], "by": "an agent", "at": "2026-10-13T10:00:00Z"}
        else:
            lesson.update({"status": "forgotten", "decision": dict(decision, verb="forget")})
            lesson.pop("forget_proposed", None)
        lesson["revision"] += 1
        done({"lesson": lesson})
    fail("unknown_op", op, 2)
if family == "tasks":
    tasks = state["tasks"]
    if op == "create":
        ref = "task:01J8Z0000000000000000000T%d" % (len(tasks) + 1)
        tasks[ref] = {"task": ref, "title": request["title"], "state": "todo", "comments": []}
        done({"scope": request.get("scope"), "task": ref, "origin_source": "", "queued": False})
    ref = args[2]
    task = tasks[ref]
    if op == "show":
        done(task)
    if op == "comment":
        task["comments"].append({"text": request["text"]})
        done({"task": ref, "queued": False})
    if op == "move":
        task["state"] = request["state"]
        done({"task": ref, "state": task["state"], "changed": True})
fail("unknown", family + " " + op, 2)
"#;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}");
}

fn ready() -> bool {
    let have = |program: &str| {
        Command::new(program)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    };
    let missing = ["bd", "python3"]
        .into_iter()
        .filter(|program| !have(program))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return true;
    }
    assert!(std::env::var_os("CI").is_none(), "CI must have {missing:?}");
    eprintln!("skipping: {missing:?} not installed");
    false
}

struct Fake {
    _dir: TempDir,
    bin: PathBuf,
    log: PathBuf,
    state: PathBuf,
}

impl Fake {
    fn new() -> Self {
        let dir = TempDir::new().expect("temp");
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).expect("bin");
        let program = bin.join("virgil");
        std::fs::write(&program, FAKE_VIRGIL).expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        Self {
            log: dir.path().join("calls.jsonl"),
            state: dir.path().join("state.json"),
            bin,
            _dir: dir,
        }
    }

    fn path(&self) -> String {
        format!(
            "{}:{}",
            self.bin.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }

    fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("call"))
            .collect()
    }
}

fn run(root: &Path, fake: &Fake, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(args)
        .current_dir(root)
        .env("PATH", fake.path())
        .env("FAKE_VIRGIL_LOG", &fake.log)
        .env("FAKE_VIRGIL_STATE", &fake.state)
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .env("BD_NON_INTERACTIVE", "1")
        .env("WHETSTONE_JEV_OFFLINE", "1")
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .output()
        .expect("whetstone")
}

fn wh(root: &Path, fake: &Fake, args: &[&str]) -> Value {
    let output = run(root, fake, args);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "wh {args:?} did not print JSON ({error}): {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn lesson(root: &Path, fake: &Fake, key: &str) -> Value {
    let dash = wh(root, fake, &["dash", "--json"]);
    dash["data"]["memory"]["lessons"]
        .as_array()
        .and_then(|lessons| lessons.iter().find(|lesson| lesson["key"] == key).cloned())
        .unwrap_or_else(|| panic!("no lesson {key}: {}", dash["data"]["memory"]))
}

#[test]
fn with_the_switch_on_lessons_and_hands_live_in_virgil_and_nothing_migrates() {
    if !ready() {
        return;
    }
    let fake = Fake::new();
    // The person's in-process calls find the same fake (the only test here).
    std::env::set_var("PATH", fake.path());
    std::env::set_var("FAKE_VIRGIL_LOG", &fake.log);
    std::env::set_var("FAKE_VIRGIL_STATE", &fake.state);
    let temp = TempDir::new().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("app.py"), "value = 1\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);
    let bd = Command::new("bd")
        .args(["init", "--prefix", "t", "--quiet"])
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .env("BD_NON_INTERACTIVE", "1")
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("bd");
    assert!(
        bd.status.success(),
        "{}",
        String::from_utf8_lossy(&bd.stderr)
    );

    // Standalone first: a lesson in Whetstone's Beads store, a hand in bd human.
    let standalone = wh(
        root,
        &fake,
        &[
            "change",
            "--json",
            "--kind",
            "lesson",
            "--content",
            "Values are integers.",
            "--definition",
            r#"{"type":"lesson","key":"values-are-integers"}"#,
        ],
    );
    assert_eq!(standalone["state"], "success", "{standalone}");
    assert!(standalone["data"]["lesson"]["locator"]
        .as_str()
        .is_some_and(|locator| locator.starts_with("whetstone:")));
    let hand = wh(
        root,
        &fake,
        &[
            "check",
            "--json",
            "--raise-hand",
            "--question",
            "Integers or floats?",
            "--tried",
            "Read app.py",
            "--recommend",
            "Integers",
        ],
    );
    assert_eq!(hand["state"], "needs_decision", "{hand}");
    assert!(hand["data"]["route"].is_null());
    assert!(!hand["data"]["issue"]
        .as_str()
        .expect("issue")
        .starts_with("task:"));
    assert!(
        fake.calls().is_empty(),
        "standalone never calls virgil: {:?}",
        fake.calls()
    );

    // Flip the switch: nothing migrates.
    let switched = wh(
        root,
        &fake,
        &[
            "init", "--json", "--action", "memory", "--store", "virgil", "--group", GROUP,
        ],
    );
    assert_eq!(switched["state"], "success", "{switched}");
    assert_eq!(switched["data"]["migrated"], 0);
    let written: Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("whetstone/memory.json")).expect("switch"),
    )
    .expect("json");
    assert_eq!(written, json!({"store": "virgil", "group": GROUP}));
    let dash = wh(root, &fake, &["dash", "--json"]);
    let memory = &dash["data"]["memory"];
    assert_eq!(memory["backend"], "virgil");
    assert_eq!(memory["group"], GROUP);
    assert_eq!(memory["not_migrated"]["lessons"], 1, "{memory}");
    assert_eq!(
        memory["lessons"],
        json!([]),
        "the Beads lesson was not copied"
    );

    // The same verbs, now in Virgil: an agent proposes and cannot admit.
    let remembered = wh(
        root,
        &fake,
        &[
            "change",
            "--json",
            "--request-id",
            "v1",
            "--kind",
            "lesson",
            "--content",
            "Values are floats since the units change.",
            "--definition",
            r#"{"type":"lesson","key":"values-are-floats","about":[{"kind":"path","path":"app.py"}]}"#,
        ],
    );
    assert_eq!(remembered["state"], "success", "{remembered}");
    assert!(remembered["data"]["lesson"]["locator"]
        .as_str()
        .is_some_and(|locator| locator.starts_with("virgil:lesson:")));
    let sent = fake
        .calls()
        .into_iter()
        .find(|call| call[0] == "lessons" && call[1] == "remember")
        .expect("remember went to virgil");
    assert!(sent.contains(&GROUP.to_string()));
    let refused = wh(
        root,
        &fake,
        &["change", "--json", "--admit", "values-are-floats"],
    );
    assert_eq!(refused["data"]["refusal"]["code"], "agent_cannot_admit");
    assert!(
        !fake.calls().iter().any(|call| call[1] == "admit"),
        "a refused admit never reaches the store"
    );
    let provenance = wh(
        root,
        &fake,
        &[
            "change",
            "--json",
            "--kind",
            "lesson",
            "--content",
            "Stamped by me.",
            "--definition",
            r#"{"type":"lesson","provenance":{"who":"me"}}"#,
        ],
    );
    assert_eq!(
        provenance["data"]["refusal"]["code"],
        "provenance_is_stamped"
    );

    // A person admits; the daemon's receipt is the decision's record.
    let admitted = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("v-admit".into()),
        actor: ActorKind::Person,
        lesson: Some(LessonOp::Admit {
            key: "values-are-floats".into(),
            trust: None,
        }),
        ..ChangeRequest::default()
    }));
    assert_eq!(
        admitted.state,
        whetstone::service::ServiceState::Success,
        "{}",
        admitted.summary
    );
    let listed = lesson(root, &fake, "values-are-floats");
    assert_eq!(listed["status"], "admitted");
    assert_eq!(listed["trust"], "validated");
    assert_eq!(
        listed["decision"]["receipt"],
        "receipt:admit-values-are-floats"
    );
    assert_eq!(listed["may_be_stale"], false);
    std::fs::write(root.join("app.py"), "value = 1.0\n").expect("write");
    assert_eq!(
        lesson(root, &fake, "values-are-floats")["may_be_stale"],
        true,
        "staleness is Whetstone's, whatever the store"
    );

    // A raised hand is a task in the group's channel, and the trail has it.
    let hand = wh(
        root,
        &fake,
        &[
            "check",
            "--json",
            "--raise-hand",
            "--question",
            "Floats everywhere?",
            "--tried",
            "Read app.py",
            "--recommend",
            "Yes",
        ],
    );
    assert_eq!(hand["data"]["route"], format!("virgil:{GROUP}"), "{hand}");
    let task = hand["data"]["issue"].as_str().expect("task").to_string();
    assert!(task.starts_with("task:"));
    let dash = wh(root, &fake, &["dash", "--json"]);
    let request = dash["data"]["current"]["requests"]
        .as_array()
        .and_then(|requests| {
            requests
                .iter()
                .find(|request| request["issue"] == task.as_str())
                .cloned()
        })
        .expect("the hand is listed");
    assert_eq!(request["question"], "Floats everywhere?");
    let answered = wh(
        root,
        &fake,
        &[
            "change",
            "--json",
            "--answer",
            &task,
            "--content",
            "Yes, floats.",
        ],
    );
    assert_eq!(answered["state"], "success", "{answered}");
    let calls = fake.calls();
    assert!(calls
        .iter()
        .any(|call| call[0] == "tasks" && call[1] == "comment" && call[2] == task));
    assert!(calls
        .iter()
        .any(|call| call[0] == "tasks" && call[1] == "move" && call[2] == task));

    // Backend b never calls anything but `virgil lessons` and `virgil tasks`.
    for call in fake.calls() {
        assert!(matches!(call[0].as_str(), "lessons" | "tasks"), "{call:?}");
        assert!(
            call.windows(2)
                .any(|pair| pair[0] == "--group" && pair[1] == GROUP),
            "{call:?}"
        );
    }
}
