//! The Virgil backend (whetstone-jpd.8, .10, .11) against a fake `virgil`
//! (tests/fake_virgil.py) that speaks the shipped `virgil lessons`,
//! `virgil tasks` and `virgil repos` flags, JSON lines and exit codes, and
//! logs every call with the `VIRGIL_VIA` it saw.
//!
//! One test in its own binary, because it puts the fake first on this
//! process's PATH for the person's in-process calls.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::ActorKind;
use whetstone::memory::virgil::{idempotency_ulid, Remember, VirgilCli};
use whetstone::service::{
    ChangeDefinition, ChangeKind, ChangeRequest, CommandService, InitAction, InitRequest, LessonOp,
    ServiceRequest, ServiceResponse, ServiceState,
};

const GROUP: &str = "01J8Z0000000000000000000AD";
const REPOSITORY: &str = "01J8Z0000000000000000000RP";

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
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_virgil.py"),
            &program,
        )
        .expect("copy fake");
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

    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("call"))
            .collect()
    }

    fn clear(&self) {
        let _ = std::fs::remove_file(&self.log);
    }

    /// The calls of one `virgil lessons <op>` (or tasks).
    fn ops(&self, family: &str, op: &str) -> Vec<Value> {
        self.calls()
            .into_iter()
            .filter(|call| call["argv"][0] == family && call["argv"][1] == op)
            .collect()
    }

    fn state(&self) -> Value {
        std::fs::read_to_string(&self.state)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_else(|| json!({}))
    }

    fn knob(&self, name: &str, value: Value) {
        let mut state = self.state();
        if state.get("knobs").is_none() {
            state["knobs"] = json!({"attached": null, "fail": {}, "stale_get": false, "page": 100});
        }
        state["knobs"][name] = value;
        std::fs::write(&self.state, state.to_string()).expect("state");
    }

    fn fail_next(&self, op: &str, exit: i32, landed: bool) {
        let mut state = self.state();
        state["knobs"]["fail"][op] = json!([exit, landed]);
        std::fs::write(&self.state, state.to_string()).expect("state");
    }

    fn lesson(&self, key: &str) -> Value {
        self.state()["lessons"][key].clone()
    }
}

fn argv(call: &Value) -> Vec<String> {
    call["argv"]
        .as_array()
        .expect("argv")
        .iter()
        .map(|arg| arg.as_str().expect("arg").to_string())
        .collect()
}

fn has(call: &Value, arg: &str) -> bool {
    argv(call).iter().any(|given| given == arg)
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

fn dash_memory(root: &Path, fake: &Fake) -> Value {
    wh(root, fake, &["dash", "--json"])["data"]["memory"].clone()
}

fn listed(root: &Path, fake: &Fake, key: &str) -> Value {
    let memory = dash_memory(root, fake);
    memory["lessons"]
        .as_array()
        .and_then(|lessons| lessons.iter().find(|lesson| lesson["key"] == key).cloned())
        .unwrap_or_else(|| panic!("no lesson {key}: {memory}"))
}

/// A person's `wh change` in this process (an agent cannot pass as one).
fn person(root: &Path, request_id: &str, change: ChangeRequest) -> ServiceResponse {
    CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some(request_id.into()),
        actor: ActorKind::Person,
        ..change
    }))
}

fn lesson_definition(value: Value) -> Option<Box<ChangeDefinition>> {
    Some(Box::new(serde_json::from_value(value).expect("definition")))
}

fn remember(
    root: &Path,
    request_id: &str,
    actor: ActorKind,
    text: &str,
    definition: Value,
) -> ServiceResponse {
    CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some(request_id.into()),
        actor,
        kind: Some(ChangeKind::Lesson),
        content: Some(text.into()),
        definition: lesson_definition(definition),
        ..ChangeRequest::default()
    }))
}

fn admit(root: &Path, request_id: &str, key: &str) -> ServiceResponse {
    person(
        root,
        request_id,
        ChangeRequest {
            lesson: Some(LessonOp::Admit {
                key: key.into(),
                trust: None,
            }),
            ..ChangeRequest::default()
        },
    )
}

fn memory_init(root: &Path, request_id: &str, actor: ActorKind, yes: bool) -> ServiceResponse {
    CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some(request_id.into()),
        action: InitAction::Store,
        store: Some("virgil".into()),
        yes,
        actor,
        ..InitRequest::default()
    }))
}

fn ok(response: &ServiceResponse) {
    assert_eq!(
        response.state,
        ServiceState::Success,
        "{}: {}",
        response.summary,
        response.data
    );
}

/// The importer's header grammar (Virgil lessons_import.rs, parse_header).
fn parses_as_beads_header(line: &str) -> bool {
    let parts = line.split(' ').collect::<Vec<_>>();
    if !matches!(parts.len(), 4 | 5) {
        return false;
    }
    let status = parts[0];
    let field = |index: usize, name: &str| parts[index].strip_prefix(&format!("{name}=")[..]);
    let scope_ok = matches!(field(1, "scope"), Some("repo" | "team" | "personal"));
    let evidence_ok = matches!(
        field(2, "evidence"),
        Some("feedback" | "inference" | "code" | "test")
    );
    let verified_ok = field(3, "verified").is_some_and(|date| {
        date.len() == 10
            && date.chars().enumerate().all(|(index, character)| {
                if index == 4 || index == 7 {
                    character == '-'
                } else {
                    character.is_ascii_digit()
                }
            })
    });
    let successor = parts.get(4).map(|part| part.strip_prefix("successor="));
    let successor_ok = match (status, successor) {
        ("CURRENT" | "OBSOLETE", None) => true,
        ("SUPERSEDED" | "OBSOLETE", Some(Some(key))) => {
            !key.is_empty()
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        }
        _ => false,
    };
    scope_ok && evidence_ok && verified_ok && successor_ok
}

#[test]
fn lessons_live_in_virgil_through_the_shipped_cli() {
    if !ready() {
        return;
    }
    let fake = Fake::new();
    // The person's in-process calls find the same fake (the only test here).
    std::env::set_var("PATH", fake.path());
    std::env::set_var("FAKE_VIRGIL_LOG", &fake.log);
    std::env::set_var("FAKE_VIRGIL_STATE", &fake.state);
    let home = TempDir::new().expect("home");
    std::env::set_var("HOME", home.path());
    let temp = TempDir::new().expect("temp");
    let root = temp.path().canonicalize().expect("root");
    let root = root.as_path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("app.py"), "value = 1\n").expect("write");
    std::fs::write(root.join("units.py"), "unit = 'm'\n").expect("write");
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

    // ── Standalone: lessons in Whetstone's store, never a virgil call. ──
    let path_about = json!({"type": "lesson", "key": "values-are-integers", "about": [{"kind": "path", "path": "app.py"}, {"kind": "repo", "repo": "fixture"}]});
    ok(&remember(
        root,
        "s1",
        ActorKind::Agent,
        "Values are integers.",
        path_about,
    ));
    ok(&admit(root, "s1-admit", "values-are-integers"));
    ok(&remember(
        root,
        "s2",
        ActorKind::Agent,
        "Units are metres.",
        json!({"type": "lesson", "key": "units-are-metres"}),
    ));
    ok(&admit(root, "s2-admit", "units-are-metres"));
    // A person's correction: units-are-metres is superseded by its successor.
    let corrected = person(
        root,
        "s3",
        ChangeRequest {
            lesson: Some(LessonOp::Supersede {
                key: "units-are-metres".into(),
            }),
            content: Some("Units are metres, stored as floats.".into()),
            ..ChangeRequest::default()
        },
    );
    ok(&corrected);
    let successor = corrected.data["lesson"]["key"]
        .as_str()
        .expect("successor")
        .to_string();
    ok(&remember(
        root,
        "s4",
        ActorKind::Agent,
        "Old transport retries.",
        json!({"type": "lesson", "key": "old-transport-retries"}),
    ));
    ok(&admit(root, "s4-admit", "old-transport-retries"));
    ok(&person(
        root,
        "s4-forget",
        ChangeRequest {
            lesson: Some(LessonOp::Forget {
                key: "old-transport-retries".into(),
            }),
            rationale: Some("The transport was removed.".into()),
            ..ChangeRequest::default()
        },
    ));
    ok(&remember(
        root,
        "s5",
        ActorKind::Agent,
        "I prefer short answers.",
        json!({"type": "lesson", "key": "prefer-short-answers", "audience": {"kind": "person", "name": "Owner"}}),
    ));
    ok(&admit(root, "s5-admit", "prefer-short-answers"));
    ok(&remember(
        root,
        "s6",
        ActorKind::Agent,
        "Maybe cache the parser.",
        json!({"type": "lesson", "key": "maybe-cache-the-parser"}),
    ));
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
    assert!(
        fake.calls().is_empty(),
        "standalone never calls virgil: {:?}",
        fake.calls()
    );

    // ── Flipping without an attachment says what is missing; no flip. ──
    fake.knob("attached", Value::Null);
    let unattached = memory_init(root, "flip-1", ActorKind::Person, false);
    assert_eq!(
        unattached.state,
        ServiceState::NeedsInput,
        "{}",
        unattached.summary
    );
    assert!(
        unattached.summary.contains("not attached"),
        "{}",
        unattached.summary
    );
    assert!(unattached
        .permitted_actions
        .iter()
        .any(|action| action.starts_with("virgil repos attach")));
    assert!(
        !root.join("whetstone/store.json").exists(),
        "the switch did not flip"
    );

    // ── Attached: the export is the importer's file, outside the repo. ──
    fake.knob("attached", json!([GROUP, REPOSITORY]));
    std::fs::create_dir_all(root.join(".virgil")).expect("attachment");
    std::fs::write(
        root.join(".virgil/repo.toml"),
        format!("group = \"{GROUP}\"\nrepository = \"{REPOSITORY}\"\n"),
    )
    .expect("attachment");
    let exported = memory_init(root, "flip-2", ActorKind::Person, false);
    assert_eq!(
        exported.state,
        ServiceState::NeedsDecision,
        "{}",
        exported.summary
    );
    assert!(
        !root.join("whetstone/store.json").exists(),
        "no flip before the import"
    );
    let file = PathBuf::from(
        exported.data["migration"]["lessons"]["file"]
            .as_str()
            .expect("file"),
    );
    assert!(
        !file.starts_with(root),
        "the importer refuses exports inside the repository"
    );
    let text = std::fs::read_to_string(&file).expect("export");
    let export: serde_json::Map<String, Value> =
        serde_json::from_str(&text).expect("one JSON object");
    let mut keys = export.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut expected = vec![
        "old-transport-retries".to_string(),
        "prefer-short-answers".into(),
        "units-are-metres".into(),
        "values-are-integers".into(),
        successor.clone(),
    ];
    expected.sort();
    assert_eq!(keys, expected, "every key once, no proposals");
    for (key, body) in &export {
        let (header, lesson_text) = body
            .as_str()
            .expect("body")
            .split_once('\n')
            .expect("header line");
        assert!(parses_as_beads_header(header), "{key}: {header}");
        assert!(!lesson_text.is_empty());
    }
    let header = |key: &str| {
        export[key]
            .as_str()
            .expect("body")
            .lines()
            .next()
            .expect("line")
            .to_string()
    };
    assert!(
        header("values-are-integers").starts_with("CURRENT scope=repo evidence=feedback verified=")
    );
    assert!(header("prefer-short-answers").starts_with("CURRENT scope=personal "));
    assert!(header("units-are-metres").starts_with("SUPERSEDED scope=team "));
    assert!(header("units-are-metres").ends_with(&format!(" successor={successor}")));
    assert!(header("old-transport-retries").starts_with("OBSOLETE "));
    let proposals: Value = serde_json::from_str(
        &std::fs::read_to_string(
            exported.data["migration"]["lessons"]["proposals_file"]
                .as_str()
                .expect("proposals"),
        )
        .expect("read"),
    )
    .expect("json");
    assert!(
        proposals.get("maybe-cache-the-parser").is_some(),
        "{proposals}"
    );
    assert_eq!(
        exported.data["migration"]["lessons"]["personal"],
        json!(["prefer-short-answers"])
    );
    assert!(exported.permitted_actions.iter().any(|action| action
        .contains(&format!("import-beads --repo {REPOSITORY} --file"))
        && action.ends_with("--dry-run --json")));
    let again = memory_init(root, "flip-3", ActorKind::Person, false);
    assert_eq!(again.state, ServiceState::NeedsDecision);
    assert_eq!(
        std::fs::read_to_string(&file).expect("again"),
        text,
        "the flip rewrites an identical file"
    );

    // Until the import is applied, a switch flipped by hand reports what was
    // left behind. (The retired lessons-only memory.json still reads: lessons
    // in Virgil, records still in Beads.)
    std::fs::create_dir_all(root.join("whetstone")).expect("dir");
    std::fs::write(
        root.join("whetstone/memory.json"),
        json!({"store": "virgil", "group": GROUP}).to_string(),
    )
    .expect("switch");
    let memory = dash_memory(root, &fake);
    assert_eq!(memory["backend"], "virgil");
    assert_eq!(memory["not_migrated"]["lessons"], 4, "{memory}");
    assert!(
        memory["proposals"]
            .as_array()
            .expect("proposals")
            .contains(&json!("maybe-cache-the-parser")),
        "proposals stay in Whetstone's store: {memory}"
    );
    std::fs::remove_file(root.join("whetstone/memory.json")).expect("unflip");

    // The person reviews and applies the import (Virgil's command, faked).
    let file_arg = format!("--file={}", file.display());
    for mode in ["--dry-run", "--apply"] {
        let output = Command::new(fake.bin.join("virgil"))
            .args([
                "lessons",
                "import-beads",
                &format!("--repo={REPOSITORY}"),
                &file_arg,
                mode,
                "--json",
            ])
            .current_dir(root)
            .output()
            .expect("import");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if mode == "--dry-run" {
            let plan: Value = serde_json::from_slice(&output.stdout).expect("plan");
            let personal = plan["result"]["entries"]
                .as_array()
                .expect("entries")
                .iter()
                .find(|entry| entry["key"] == "prefer-short-answers")
                .cloned()
                .expect("personal entry");
            assert_eq!(personal["disposition"], "personal_needs_owner");
        }
    }
    // An agent cannot confirm it; a person's confirmation reads back,
    // re-anchors and flips.
    fake.clear();
    let agent_confirm = wh(
        root,
        &fake,
        &[
            "init", "--json", "--action", "memory", "--store", "virgil", "--group", GROUP, "--yes",
        ],
    );
    assert_eq!(agent_confirm["state"], "conflict", "{agent_confirm}");
    assert!(!root.join("whetstone/store.json").exists());
    let confirmed = memory_init(root, "flip-4", ActorKind::Person, true);
    ok(&confirmed);
    assert_eq!(confirmed.data["migration"]["lessons"]["imported"], 4);
    assert_eq!(
        confirmed.data["migration"]["lessons"]["re_anchored"],
        json!(["values-are-integers"])
    );
    let reanchor = fake.ops("lessons", "remember");
    assert_eq!(reanchor.len(), 1, "{reanchor:?}");
    assert!(has(&reanchor[0], "--key=values-are-integers"));
    assert!(has(&reanchor[0], "--if-revision=1"));
    assert!(has(&reanchor[0], "--reason=re-anchored after import"));
    assert!(has(&reanchor[0], "--anchor=app.py"));
    assert!(
        reanchor[0]["via"].is_null(),
        "the person's write carries no marker"
    );
    assert_eq!(
        serde_json::from_str::<Value>(
            &std::fs::read_to_string(root.join("whetstone/store.json")).expect("switch")
        )
        .expect("json"),
        json!({"store": "virgil"})
    );
    assert!(!root.join("whetstone/memory.json").exists());
    let memory = dash_memory(root, &fake);
    assert!(memory["not_migrated"].is_null(), "{memory}");
    assert_eq!(
        listed(root, &fake, "values-are-integers")["status"],
        "admitted"
    );
    assert!(listed(root, &fake, "values-are-integers")["locator"]
        .as_str()
        .expect("locator")
        .starts_with("virgil:"));

    // ── Virgil mode: an agent's lesson stays a proposal in Whetstone. ──
    fake.clear();
    let proposed = wh(
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
            r#"{"type":"lesson","key":"values-are-floats","about":[{"kind":"path","path":"app.py"},{"kind":"repo","repo":"fixture"}]}"#,
        ],
    );
    assert_eq!(proposed["state"], "success", "{proposed}");
    assert!(proposed["data"]["lesson"]["locator"]
        .as_str()
        .expect("locator")
        .starts_with("whetstone:"));
    assert!(
        fake.ops("lessons", "remember").is_empty(),
        "an agent's proposal never reaches Virgil"
    );
    assert!(fake.state()["lessons"].get("values-are-floats").is_none());
    let refused = wh(
        root,
        &fake,
        &["change", "--json", "--admit", "values-are-floats"],
    );
    assert_eq!(refused["data"]["refusal"]["code"], "agent_cannot_admit");
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
    let chosen = person(
        root,
        "v-trust",
        ChangeRequest {
            lesson: Some(LessonOp::Admit {
                key: "values-are-floats".into(),
                trust: Some(whetstone::domain::Trust::Authoritative),
            }),
            ..ChangeRequest::default()
        },
    );
    assert_eq!(
        chosen.data["refusal"]["code"], "trust_not_chosen",
        "{}",
        chosen.summary
    );
    assert!(fake.ops("lessons", "remember").is_empty());

    // A person's admit is their `virgil lessons remember`: validated.
    let admitted = admit(root, "v-admit", "values-are-floats");
    ok(&admitted);
    let writes = fake.ops("lessons", "remember");
    assert_eq!(writes.len(), 1);
    let write = &writes[0];
    for flag in [
        "--key=values-are-floats",
        "--text=Values are floats since the units change.",
        "--audience=channel",
        "--about=repo",
        "--anchor=app.py",
    ] {
        assert!(has(write, flag), "{flag}: {write}");
    }
    assert!(argv(write)
        .iter()
        .any(|arg| arg.starts_with("--idempotency-key=")
            && arg.len() == "--idempotency-key=".len() + 26));
    assert!(
        !argv(write).iter().any(|arg| arg.starts_with("--group")),
        "no --group: the attachment scopes it"
    );
    assert!(write["via"].is_null());
    assert_eq!(fake.lesson("values-are-floats")["trust"], "validated");
    let shown = listed(root, &fake, "values-are-floats");
    assert_eq!(shown["status"], "admitted");
    assert_eq!(shown["trust"], "validated");
    assert!(shown["decision"]["receipt"]
        .as_str()
        .expect("receipt")
        .starts_with("virgil:"));
    assert!(!dash_memory(root, &fake)["proposals"]
        .as_array()
        .expect("proposals")
        .contains(&json!("values-are-floats")));
    // Staleness is Virgil's: the anchor changed.
    assert_eq!(shown["may_be_stale"], false);
    std::fs::write(root.join("app.py"), "value = 1.0\n").expect("write");
    assert_eq!(
        listed(root, &fake, "values-are-floats")["may_be_stale"],
        true
    );
    assert!(!fake
        .ops("lessons", "list")
        .iter()
        .filter(|call| has(call, "--changed"))
        .collect::<Vec<_>>()
        .is_empty());
    std::fs::write(root.join("app.py"), "value = 1\n").expect("write");

    // Text that starts with `-` is data, never a flag.
    ok(&remember(
        root,
        "v-dash",
        ActorKind::Person,
        "--never-a-flag: pass values as --flag=value.",
        json!({"type": "lesson", "key": "values-are-not-flags"}),
    ));
    ok(&admit(root, "v-dash-admit", "values-are-not-flags"));
    assert_eq!(
        fake.lesson("values-are-not-flags")["text"],
        "--never-a-flag: pass values as --flag=value."
    );

    // An agent's correction waits; a person's rewrites in place, keeping
    // the subjects and anchors, revision-checked.
    fake.clear();
    let agent_fix = wh(
        root,
        &fake,
        &[
            "change",
            "--json",
            "--supersede",
            "values-are-floats",
            "--content",
            "Values are decimals.",
        ],
    );
    assert_eq!(agent_fix["state"], "success", "{agent_fix}");
    assert_eq!(agent_fix["data"]["applied"], false);
    assert!(fake.ops("lessons", "remember").is_empty());
    let revision = fake.lesson("values-are-floats")["revision"]
        .as_u64()
        .expect("revision");
    let fixed = person(
        root,
        "v-fix",
        ChangeRequest {
            lesson: Some(LessonOp::Supersede {
                key: "values-are-floats".into(),
            }),
            content: Some("Values are floats in SI units.".into()),
            rationale: Some("The units settled.".into()),
            ..ChangeRequest::default()
        },
    );
    ok(&fixed);
    assert!(
        fixed.summary.contains("rewritten in place"),
        "{}",
        fixed.summary
    );
    let rewrite = fake.ops("lessons", "remember");
    assert_eq!(rewrite.len(), 1, "{rewrite:?}");
    for flag in [
        "--key=values-are-floats".to_string(),
        format!("--if-revision={revision}"),
        "--reason=The units settled.".into(),
        "--about=repo".into(),
        "--anchor=app.py".into(),
    ] {
        assert!(has(&rewrite[0], &flag), "{flag}: {}", rewrite[0]);
    }
    assert_eq!(
        fake.lesson("values-are-floats")["text"],
        "Values are floats in SI units."
    );
    assert_eq!(fake.lesson("values-are-floats")["revision"], revision + 1);

    // A moved revision is read-again, never retried blind.
    fake.clear();
    fake.knob("stale_get", json!(true));
    let moved = person(
        root,
        "v-moved",
        ChangeRequest {
            lesson: Some(LessonOp::Supersede {
                key: "values-are-floats".into(),
            }),
            content: Some("Values are floats, always.".into()),
            rationale: Some("Again.".into()),
            ..ChangeRequest::default()
        },
    );
    assert_eq!(
        moved.data["refusal"]["code"], "read_again",
        "{}",
        moved.summary
    );
    assert_eq!(fake.ops("lessons", "remember").len(), 1, "one attempt only");
    fake.knob("stale_get", json!(false));

    // Supersede onto a successor that already exists: Virgil's own verb.
    fake.clear();
    let retired = person(
        root,
        "v-retire",
        ChangeRequest {
            lesson: Some(LessonOp::Supersede {
                key: "values-are-not-flags".into(),
            }),
            record_id: Some("lesson.values-are-floats".into()),
            content: Some("Folded into values-are-floats.".into()),
            rationale: Some("One lesson is enough.".into()),
            ..ChangeRequest::default()
        },
    );
    ok(&retired);
    let supersede = fake.ops("lessons", "supersede");
    assert_eq!(supersede.len(), 1, "{:?}", fake.calls());
    assert!(has(&supersede[0], "values-are-not-flags"));
    assert!(has(&supersede[0], "--with=values-are-floats"));
    assert!(argv(&supersede[0])
        .iter()
        .any(|arg| arg.starts_with("--if-revision=")));
    assert!(argv(&supersede[0])
        .iter()
        .any(|arg| arg.starts_with("--successor-revision=")));
    assert_eq!(fake.lesson("values-are-not-flags")["status"], "superseded");

    // Forget: an agent proposes (nothing in Virgil); a person's is
    // revision-checked.
    fake.clear();
    let agent_forget = wh(
        root,
        &fake,
        &[
            "change",
            "--json",
            "--forget",
            "prefer-short-answers",
            "--rationale",
            "Stale preference.",
        ],
    );
    assert_eq!(agent_forget["state"], "success", "{agent_forget}");
    assert!(fake.ops("lessons", "forget").is_empty());
    assert!(!listed(root, &fake, "prefer-short-answers")["forget_proposed"].is_null());
    let forgotten = person(
        root,
        "v-forget",
        ChangeRequest {
            lesson: Some(LessonOp::Forget {
                key: "prefer-short-answers".into(),
            }),
            rationale: Some("No longer true.".into()),
            ..ChangeRequest::default()
        },
    );
    ok(&forgotten);
    let forget = fake.ops("lessons", "forget");
    assert_eq!(forget.len(), 1);
    assert!(has(&forget[0], "--if-revision=1"));
    assert!(has(&forget[0], "--reason=No longer true."));
    assert_eq!(fake.lesson("prefer-short-answers")["status"], "forgotten");

    // Exit codes: an id means it landed, even at 7 or 5; without one it is
    // refused with Whetstone's code; 6 unattached says attach.
    ok(&remember(
        root,
        "e1",
        ActorKind::Person,
        "Pending lessons still land.",
        json!({"type": "lesson", "key": "pending-lessons-land"}),
    ));
    fake.fail_next("remember", 7, true);
    ok(&admit(root, "e1-admit", "pending-lessons-land"));
    ok(&remember(
        root,
        "e2",
        ActorKind::Person,
        "Follow-ups may be refused.",
        json!({"type": "lesson", "key": "follow-ups-refused"}),
    ));
    fake.fail_next("remember", 5, true);
    ok(&admit(root, "e2-admit", "follow-ups-refused"));
    ok(&remember(
        root,
        "e3",
        ActorKind::Person,
        "Some writes are not permitted.",
        json!({"type": "lesson", "key": "writes-not-permitted"}),
    ));
    fake.fail_next("remember", 5, false);
    assert_eq!(
        admit(root, "e3-admit", "writes-not-permitted").data["refusal"]["code"],
        "not_permitted"
    );
    fake.fail_next("remember", 7, false);
    let unavailable = admit(root, "e3-again", "writes-not-permitted");
    assert_eq!(
        unavailable.state,
        ServiceState::Unavailable,
        "{}",
        unavailable.summary
    );
    fake.knob("attached", Value::Null);
    let detached = admit(root, "e4", "writes-not-permitted");
    assert_eq!(
        detached.data["refusal"]["code"], "unattached",
        "{}",
        detached.summary
    );
    assert!(
        detached.summary.contains("Attach the repository"),
        "{}",
        detached.summary
    );
    fake.knob("attached", json!([GROUP, REPOSITORY]));

    // ── whetstone-jpd.11: an agent's writes are marked; a person's not. ──
    fake.clear();
    let host = whetstone::memory::agent_host();
    let agent = VirgilCli::new(root, ActorKind::Agent);
    let written = agent
        .remember(&Remember {
            key: "agents-write-inferred",
            text: "An agent's direct write is inferred.",
            audience: "channel",
            about: vec!["repo".into()],
            anchors: vec!["units.py".into()],
            if_revision: None,
            reason: None,
            idempotency: idempotency_ulid("agent-write", 0),
        })
        .expect("agent remember");
    assert_eq!(written.trust, Some(whetstone::domain::Trust::Inferred));
    assert_eq!(written.via, format!("agent:{host}"));
    let as_proposal = listed(root, &fake, "agents-write-inferred");
    assert_eq!(
        as_proposal["status"], "proposed",
        "a current but inferred lesson waits on a person"
    );
    agent
        .supersede(
            "agents-write-inferred",
            "values-are-floats",
            99,
            99,
            "test",
            &idempotency_ulid("agent-supersede", 0),
        )
        .expect_err("a moved revision is refused; only the marker matters here");
    agent
        .forget(
            "agents-write-inferred",
            99,
            "test",
            &idempotency_ulid("agent-forget", 0),
        )
        .expect_err("a moved revision is refused");
    for call in fake.calls().iter().filter(|call| {
        matches!(
            call["argv"][1].as_str(),
            Some("remember" | "supersede" | "forget")
        )
    }) {
        assert_eq!(call["via"], format!("agent:{host}"), "{call}");
    }
    // The person's admit rewrites it as theirs: validated, no marker.
    fake.clear();
    ok(&admit(root, "j11-admit", "agents-write-inferred"));
    let rewrite = fake.ops("lessons", "remember");
    assert!(rewrite[0]["via"].is_null());
    assert!(has(&rewrite[0], "--anchor=units.py") && has(&rewrite[0], "--if-revision=1"));
    assert_eq!(fake.lesson("agents-write-inferred")["trust"], "validated");
    // A host that breaks Virgil's client rule never reaches it unnormalised.
    assert_eq!(
        whetstone::memory::virgil_client("Cursor Agent!"),
        "cursor-agent-"
    );
    assert_eq!(whetstone::memory::virgil_client("--"), "whetstone");
    assert_eq!(
        whetstone::memory::virgil_client(&"x".repeat(65)),
        "whetstone"
    );

    // ── Raised hands: a task in the channel naming the agent, no marker. ──
    fake.clear();
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
    let description = fake.state()["tasks"][&task]["description"]
        .as_str()
        .expect("description")
        .to_string();
    assert!(
        description.starts_with(&format!("Raised by {host} (an agent) through Whetstone.")),
        "{description}"
    );
    for call in fake
        .calls()
        .iter()
        .filter(|call| call["argv"][0] == "tasks")
    {
        assert!(
            call["via"].is_null(),
            "virgil tasks never gets VIRGIL_VIA: {call}"
        );
        assert!(has(call, "--group") && has(call, GROUP));
    }
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
    assert_eq!(fake.state()["tasks"][&task]["state"], "done");
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

    // Backend b never calls anything but `virgil lessons`, `tasks`, `repos`
    // and `rulebook` (every other record), and setup's `--version` probe.
    for call in fake.calls() {
        assert!(
            matches!(
                call["argv"][0].as_str(),
                Some("lessons" | "tasks" | "repos" | "rulebook" | "--version")
            ),
            "{call}"
        );
    }
}
