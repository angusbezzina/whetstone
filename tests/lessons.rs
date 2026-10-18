//! Memory as the whetstone, standalone (whetstone-jpd): lessons are proposed
//! by anyone and admitted only by a person, anchored to the files they name
//! so a hash comparison says when they may be stale, and drafted into rules
//! that link back to them.
//!
//! The agent is the real binary (no terminal, so never a person); the
//! person is the library's service called with a person as the actor, which
//! is what a terminal or the dashboard a person opened passes. Every
//! repository is a throwaway; nothing touches this repository's stores.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::{ActorKind, Trust};
use whetstone::service::{ChangeRequest, CommandService, LessonOp, ServiceRequest, ServiceState};

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

/// An agent remembers a lesson through the binary.
fn remember(root: &Path, request: &str, text: &str, definition: Value) -> Value {
    wh(
        root,
        &[
            "change",
            "--json",
            "--request-id",
            request,
            "--kind",
            "lesson",
            "--content",
            text,
            "--definition",
            &definition.to_string(),
        ],
    )
}

/// A person's lesson decision, as a terminal or the dashboard passes it.
fn person(
    root: &Path,
    request: &str,
    op: LessonOp,
    content: Option<&str>,
    rationale: Option<&str>,
) -> Value {
    let response = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some(request.into()),
        actor: ActorKind::Person,
        lesson: Some(op),
        content: content.map(str::to_owned),
        rationale: rationale.map(str::to_owned),
        ..ChangeRequest::default()
    }));
    serde_json::to_value(response).expect("response")
}

fn lessons(root: &Path) -> Vec<Value> {
    let dash = wh(root, &["dash", "--json"]);
    dash["data"]["memory"]["lessons"]
        .as_array()
        .cloned()
        .unwrap_or_else(|| panic!("dash lists lessons: {}", dash["data"]["memory"]))
}

fn lesson(root: &Path, key: &str) -> Value {
    lessons(root)
        .into_iter()
        .find(|lesson| lesson["key"] == key)
        .unwrap_or_else(|| panic!("no lesson {key}"))
}

#[test]
fn a_lesson_is_proposed_by_anyone_admitted_by_a_person_and_goes_stale_with_its_anchor() {
    if !have_bd() {
        return;
    }
    let temp = repository();
    let root = temp.path();

    // An agent remembers: proposed, inferred, stamped by the store.
    let remembered = remember(
        root,
        "l1",
        "Load config through the schema, never with json.load directly.",
        json!({
            "type": "lesson",
            "key": "config-through-schema",
            "about": [{"kind": "path", "path": "src/config.py"}, {"kind": "repo", "repo": "notes"}],
            "concepts": ["config"],
        }),
    );
    assert_eq!(remembered["state"], "success", "{remembered}");
    let recorded = &remembered["data"]["lesson"];
    assert_eq!(recorded["status"], "proposed");
    assert_eq!(recorded["trust"], "inferred");
    assert_eq!(recorded["stamp"]["actor"], "agent");
    assert_eq!(recorded["stamp"]["branch"], "main");
    assert!(recorded["stamp"]["commit"]
        .as_str()
        .is_some_and(|commit| commit.len() == 40));
    // Anchors are the paths the lesson names, not every file around.
    let anchors = recorded["anchors"].as_array().expect("anchors");
    assert_eq!(anchors.len(), 1, "{anchors:?}");
    assert_eq!(anchors[0]["path"], "src/config.py");
    assert_eq!(recorded["may_be_stale"], false);

    // The same request again records nothing new.
    let replay = remember(
        root,
        "l1",
        "Load config through the schema, never with json.load directly.",
        json!({
            "type": "lesson",
            "key": "config-through-schema",
            "about": [{"kind": "path", "path": "src/config.py"}, {"kind": "repo", "repo": "notes"}],
            "concepts": ["config"],
        }),
    );
    assert_eq!(replay["data"]["lesson"]["revision"], 1, "{replay}");

    // An agent can never admit, by the change family or the dashboard route.
    let refused = run(
        root,
        &["change", "--json", "--admit", "config-through-schema"],
    );
    assert_eq!(refused.status.code(), Some(8));
    let refused: Value = serde_json::from_slice(&refused.stdout).expect("json");
    assert_eq!(
        refused["data"]["refusal"]["code"], "agent_cannot_admit",
        "{refused}"
    );
    let agent_admit = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("sneaky".into()),
        lesson: Some(LessonOp::Admit {
            key: "config-through-schema".into(),
            trust: Some(Trust::Authoritative),
        }),
        ..ChangeRequest::default()
    }));
    assert_eq!(
        agent_admit.state,
        ServiceState::Conflict,
        "an omitted actor is an agent"
    );
    assert_eq!(lesson(root, "config-through-schema")["status"], "proposed");

    // A request that supplies provenance, trust or a writer is refused.
    for (field, value) in [
        ("provenance", json!({"who": "Owner"})),
        ("trust", json!("validated")),
        ("status", json!("admitted")),
    ] {
        let mut definition = json!({"type": "lesson", "key": format!("supplied-{field}")});
        definition[field] = value;
        let response = remember(
            root,
            &format!("supplied-{field}"),
            "Supplied fields are refused.",
            definition,
        );
        assert_eq!(
            response["data"]["refusal"]["code"], "provenance_is_stamped",
            "{field}: {response}"
        );
    }
    let sourced = wh(
        root,
        &[
            "change",
            "--json",
            "--kind",
            "lesson",
            "--content",
            "A lesson with a writer.",
            "--source",
            "Owner",
        ],
    );
    assert_eq!(
        sourced["data"]["refusal"]["code"], "provenance_is_stamped",
        "{sourced}"
    );

    // A subject narrower than the audience is refused at write.
    std::fs::create_dir_all(root.join("secrets")).expect("dir");
    std::fs::write(root.join("secrets/token.md"), "local only\n").expect("write");
    let narrower = remember(
        root,
        "narrow",
        "The token file is rotated weekly.",
        json!({"type": "lesson", "key": "token-rotation", "about": [{"kind": "path", "path": "secrets/token.md"}]}),
    );
    assert_eq!(
        narrower["data"]["refusal"]["code"], "scope_exceeds_authority",
        "{narrower}"
    );
    let personal = remember(
        root,
        "narrow-personal",
        "The token file is rotated weekly.",
        json!({
            "type": "lesson",
            "key": "token-rotation",
            "about": [{"kind": "path", "path": "secrets/token.md"}],
            "audience": {"kind": "person", "name": "Owner"},
        }),
    );
    assert_eq!(
        personal["state"], "success",
        "a personal lesson may name it: {personal}"
    );
    let draft_subject = remember(
        root,
        "personal-record",
        "My scratch notes record.",
        json!({"type": "lesson", "key": "scratch-notes", "about": [{"kind": "record", "locator": "personal:scratch"}]}),
    );
    assert_eq!(
        draft_subject["data"]["refusal"]["code"],
        "scope_exceeds_authority"
    );

    // A person admits it.
    let admitted = person(
        root,
        "admit-1",
        LessonOp::Admit {
            key: "config-through-schema".into(),
            trust: None,
        },
        None,
        None,
    );
    assert_eq!(admitted["state"], "success", "{admitted}");
    let listed = lesson(root, "config-through-schema");
    assert_eq!(listed["status"], "admitted");
    assert_eq!(listed["trust"], "validated");
    assert_eq!(listed["decision"]["verb"], "admit");
    assert_eq!(listed["decision"]["by"], "Owner");
    assert_eq!(listed["may_be_stale"], false);
    let dash = wh(root, &["dash", "--json"]);
    assert_eq!(dash["data"]["memory"]["backend"], "beads");
    assert!(dash["data"]["memory"]["proposals"]
        .as_array()
        .is_some_and(|proposals| proposals.contains(&json!("token-rotation"))));

    // A file the lesson does not name changing leaves it fresh; its anchor
    // changing flags it, without any model call.
    std::fs::write(root.join("README.md"), "notes, edited\n").expect("write");
    assert_eq!(lesson(root, "config-through-schema")["may_be_stale"], false);
    std::fs::write(
        root.join("src/config.py"),
        "import json\n\ndef load(path):\n    return json.loads(open(path).read())\n",
    )
    .expect("write");
    let stale = lesson(root, "config-through-schema");
    assert_eq!(stale["may_be_stale"], true, "{stale}");
    assert_eq!(stale["anchors"][0]["changed"], true);
    let dash = wh(root, &["dash", "--json"]);
    assert!(dash["data"]["memory"]["may_be_stale"]
        .as_array()
        .is_some_and(|keys| keys.contains(&json!("config-through-schema"))));

    // wh check records the comparison as a receipt, once per result.
    let checked = wh(root, &["check", "--json"]);
    let recorded = checked["data"]["lesson_checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        recorded.iter().any(
            |check| check["lesson"] == "config-through-schema" && check["may_be_stale"] == true
        ),
        "{}",
        checked["data"]
    );
    let again = wh(root, &["check", "--json"]);
    let again_recorded = again["data"]["lesson_checks"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let first = recorded
        .iter()
        .find(|check| check["lesson"] == "config-through-schema")
        .expect("first");
    let second = again_recorded
        .iter()
        .find(|check| check["lesson"] == "config-through-schema")
        .expect("second");
    assert_eq!(
        first["receipt"], second["receipt"],
        "the same comparison is one receipt"
    );

    // The dashboard carries the admitted lesson and its flag.
    let dash = wh(root, &["dash", "--json"]);
    let lesson = dash["data"]["memory"]["lessons"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|lesson| {
            lesson["text"]
                .as_str()
                .is_some_and(|text| text.contains("Load config through the schema"))
        })
        .unwrap_or_else(|| panic!("{}", dash["data"]["memory"]));
    assert_eq!(lesson["status"], "admitted", "{lesson}");
    assert_eq!(lesson["may_be_stale"], true, "{lesson}");
}

#[test]
fn corrections_and_forgets_wait_for_a_person_and_two_corrections_send_a_lesson_back_to_review() {
    if !have_bd() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    remember(
        root,
        "first",
        "Config files are JSON.",
        json!({"type": "lesson", "key": "config-format", "about": [{"kind": "path", "path": "src/config.py"}]}),
    );
    person(
        root,
        "admit-first",
        LessonOp::Admit {
            key: "config-format".into(),
            trust: None,
        },
        None,
        None,
    );

    // An agent's correction is a proposal; the old lesson stays delivered.
    let proposed = wh(
        root,
        &[
            "change",
            "--json",
            "--request-id",
            "agent-fix",
            "--supersede",
            "config-format",
            "--content",
            "Config files are TOML since the move.",
        ],
    );
    assert_eq!(proposed["data"]["applied"], false, "{proposed}");
    let correction = proposed["data"]["lesson"]["key"]
        .as_str()
        .expect("key")
        .to_string();
    assert_eq!(correction, "config-format-2");
    assert_eq!(proposed["data"]["lesson"]["status"], "proposed");
    assert_eq!(proposed["data"]["lesson"]["supersedes"], "config-format");
    assert_eq!(lesson(root, "config-format")["status"], "admitted");

    // A person admitting the correction supersedes the old lesson.
    person(
        root,
        "admit-fix",
        LessonOp::Admit {
            key: correction.clone(),
            trust: None,
        },
        None,
        None,
    );
    let old = lesson(root, "config-format");
    assert_eq!(old["status"], "superseded");
    assert_eq!(old["superseded_by"], "config-format-2");

    // A person's own correction is applied at once.
    let applied = person(
        root,
        "person-fix",
        LessonOp::Supersede {
            key: correction.clone(),
        },
        Some("Config files are TOML, loaded with tomllib."),
        None,
    );
    assert_eq!(applied["data"]["applied"], true, "{applied}");
    let third = lesson(root, "config-format-3");
    assert_eq!(third["status"], "admitted");
    assert_eq!(third["corrections"], 2);
    assert_eq!(
        third["needs_review"], true,
        "corrected twice: back to review"
    );
    assert_eq!(lesson(root, &correction)["status"], "superseded");

    // An agent proposes forgetting; only a person forgets.
    let proposal = wh(
        root,
        &[
            "change",
            "--json",
            "--request-id",
            "agent-forget",
            "--forget",
            "config-format-3",
            "--rationale",
            "The config module was deleted.",
        ],
    );
    assert_eq!(proposal["data"]["applied"], false, "{proposal}");
    let still = lesson(root, "config-format-3");
    assert_eq!(still["status"], "admitted");
    assert_eq!(
        still["forget_proposed"]["reason"],
        "The config module was deleted."
    );
    let forgotten = person(
        root,
        "person-forget",
        LessonOp::Forget {
            key: "config-format-3".into(),
        },
        None,
        Some("The config module was deleted."),
    );
    assert_eq!(forgotten["data"]["applied"], true, "{forgotten}");
    let gone = lesson(root, "config-format-3");
    assert_eq!(gone["status"], "forgotten");
    assert_eq!(gone["decision"]["verb"], "forget");
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
fn a_rule_drafted_from_an_admitted_lesson_walks_back_to_its_lesson() {
    if !have_bd() {
        return;
    }
    let temp = repository();
    let root = temp.path();
    remember(
        root,
        "origin",
        "Never call print in src; use logging.",
        json!({"type": "lesson", "key": "no-print-in-src", "about": [{"kind": "path", "path": "src/config.py"}]}),
    );
    let rule = json!({
        "type": "rule",
        "strength": "should",
        "enforcer": {
            "kind": "ast",
            "language": "python",
            "query": "((call function: (identifier) @f) @match (#eq? @f \"print\"))"
        }
    });

    // Only an admitted lesson can be drafted into a rule.
    let early = draft_rule(
        root,
        "early",
        "rule.no-print",
        "No print calls in src.",
        rule.clone(),
        &["--from-lesson", "no-print-in-src"],
    );
    assert_ne!(early["state"], "success", "{early}");
    assert!(
        early["blocking_questions"]
            .to_string()
            .contains("only an admitted"),
        "{early}"
    );

    person(
        root,
        "admit-origin",
        LessonOp::Admit {
            key: "no-print-in-src".into(),
            trust: Some(Trust::Authoritative),
        },
        None,
        None,
    );

    // An external drafter (Messo's night sweep, here the binary) proposes it.
    let drafted = draft_rule(
        root,
        "from-lesson",
        "rule.no-print",
        "No print calls in src.",
        rule,
        &["--from-lesson", "no-print-in-src"],
    );
    assert_eq!(drafted["state"], "success", "{drafted}");
    let after = &drafted["data"]["diff"]["after"]["record"];
    assert_eq!(after["source"]["kind"], "lesson");
    assert_eq!(after["source"]["reference"], "no-print-in-src");
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();

    // It is a rule proposal in wh dash; nothing accepted it.
    let dash = wh(root, &["dash", "--json"]);
    let listed = dash["data"]["current"]["rules"]
        .as_array()
        .and_then(|rules| {
            rules
                .iter()
                .find(|rule| rule["id"] == "rule.no-print")
                .cloned()
        })
        .expect("rule listed");
    assert_eq!(listed["lifecycle"], "draft");
    assert_eq!(listed["origin_lesson"], "no-print-in-src");
    assert_eq!(
        lesson(root, "no-print-in-src")["derived_rules"],
        json!(["rule.no-print"])
    );

    // Accepting it records the origin lesson it rested on.
    let accepted = accept(root, &proposal);
    assert_eq!(accepted["state"], "success", "{accepted}");
    let cites = accepted["data"]["cites"].as_array().expect("cites");
    assert_eq!(cites.len(), 1, "{accepted}");
    assert_eq!(cites[0]["system"], "beads_lesson");
    assert!(cites[0]["locator"]
        .as_str()
        .is_some_and(|locator| locator.starts_with("whetstone:lesson.no-print-in-src@")));

    // From the rule, walk to the accepting decision, the lesson and its anchors.
    let dash = wh(root, &["dash", "--json"]);
    let listed = dash["data"]["current"]["rules"]
        .as_array()
        .and_then(|rules| {
            rules
                .iter()
                .find(|rule| rule["id"] == "rule.no-print")
                .cloned()
        })
        .expect("rule listed");
    let lineage = &listed["lineage"];
    assert_eq!(lineage["accepted_by"]["verdict"], "accept", "{lineage}");
    assert_eq!(lineage["accepted_by"]["cites"], json!(cites));
    assert_eq!(lineage["origin"]["key"], "no-print-in-src");
    assert_eq!(lineage["origin"]["trust"], "authoritative");
    assert_eq!(lineage["origin"]["decision"]["verb"], "admit");
    assert_eq!(lineage["origin"]["anchors"][0]["path"], "src/config.py");

    // A should rule's flag blocks nobody.
    std::fs::write(root.join("src/config.py"), "print('loading')\n").expect("write");
    let checked = run(root, &["check", "--json", "--rule", "rule.no-print"]);
    let checked_value: Value = serde_json::from_slice(&checked.stdout).expect("json");
    assert_eq!(checked.status.code(), Some(0), "{checked_value}");
    let gate = checked_value["data"]["gates"]
        .as_array()
        .and_then(|gates| {
            gates
                .iter()
                .find(|gate| gate["id"] == "rule.no-print")
                .cloned()
        })
        .expect("gate");
    assert_eq!(gate["state"], "fail");
    assert!(
        checked_value["data"]["gate_receipts"]
            .as_array()
            .is_some_and(|receipts| !receipts.is_empty()),
        "the flag is recorded: {}",
        checked_value["data"]
    );

    // Once the origin lesson is corrected, the rule drawn from it goes back
    // to a person; nothing changes force.
    person(
        root,
        "correct-origin",
        LessonOp::Supersede {
            key: "no-print-in-src".into(),
        },
        Some("Never call print anywhere; use logging."),
        None,
    );
    let dash = wh(root, &["dash", "--json"]);
    let review = &dash["data"]["memory"]["rules_to_review"][0];
    assert_eq!(review["rule"], "rule.no-print", "{dash}");
    assert!(review["reason"]
        .as_str()
        .is_some_and(|reason| reason.contains("superseded by no-print-in-src-2")));
    let listed = dash["data"]["current"]["rules"]
        .as_array()
        .and_then(|rules| {
            rules
                .iter()
                .find(|rule| rule["id"] == "rule.no-print")
                .cloned()
        })
        .expect("rule listed");
    assert_eq!(listed["lifecycle"], "accepted", "the rule stays as it is");
}

fn plain(root: &Path, program: &str, args: &[&str], beads: bool) -> String {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(root)
        .env("BD_NON_INTERACTIVE", "1");
    if beads {
        command.env("BEADS_DIR", root.join(".beads"));
    }
    let output = command.output().expect(program);
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn admitted_team_lessons_travel_with_wh_push_and_personal_or_unadmitted_ones_never_do() {
    if !have_bd() {
        return;
    }
    let temp = TempDir::new().expect("temp");
    let origin = temp.path().join("origin.git");
    plain(
        temp.path(),
        "git",
        &[
            "init",
            "-q",
            "--bare",
            "--initial-branch=main",
            origin.to_str().expect("path"),
        ],
        false,
    );
    let url = format!("git+file://{}", origin.display());
    let a = temp.path().join("a");
    plain(
        temp.path(),
        "git",
        &[
            "clone",
            "-q",
            origin.to_str().expect("path"),
            a.to_str().expect("path"),
        ],
        false,
    );
    plain(
        &a,
        "git",
        &["config", "user.email", "ada@example.com"],
        false,
    );
    plain(&a, "git", &["config", "user.name", "Ada"], false);
    plain(&a, "git", &["config", "commit.gpgsign", "false"], false);
    std::fs::write(a.join("app.py"), "value = 1\n").expect("write");
    plain(&a, "git", &["add", "."], false);
    plain(&a, "git", &["commit", "-qm", "init"], false);
    plain(&a, "git", &["push", "-q", "origin", "HEAD:main"], false);
    plain(
        &a,
        "bd",
        &[
            "init",
            "--non-interactive",
            "--skip-agents",
            "--skip-hooks",
            "-p",
            "team",
            "-q",
        ],
        true,
    );
    plain(&a, "bd", &["dolt", "remote", "add", "origin", &url], true);

    remember(
        &a,
        "team",
        "Values are integers.",
        json!({"type": "lesson", "key": "values-are-integers", "about": [{"kind": "path", "path": "app.py"}]}),
    );
    remember(
        &a,
        "waiting",
        "Nobody admitted this yet.",
        json!({"type": "lesson", "key": "not-yet-admitted"}),
    );
    remember(
        &a,
        "mine",
        "I prefer short functions.",
        json!({"type": "lesson", "key": "my-own-taste", "audience": {"kind": "person", "name": "Ada"}}),
    );
    person(
        &a,
        "admit-team",
        LessonOp::Admit {
            key: "values-are-integers".into(),
            trust: None,
        },
        None,
        None,
    );
    person(
        &a,
        "admit-mine",
        LessonOp::Admit {
            key: "my-own-taste".into(),
            trust: None,
        },
        None,
        None,
    );

    let reviewed = wh(&a, &["push", "--json"]);
    assert_eq!(reviewed["state"], "needs_decision", "{reviewed}");
    let package = reviewed["data"]["package"].to_string();
    assert!(package.contains("lesson.values-are-integers"), "{package}");
    assert!(
        !package.contains("not-yet-admitted"),
        "an unadmitted lesson stays private"
    );
    assert!(
        !package.contains("my-own-taste"),
        "a personal lesson never travels"
    );
    let token = reviewed["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let pushed = wh(&a, &["push", "--json", "--confirm", &token]);
    assert_eq!(pushed["state"], "success", "{pushed}");
    assert_eq!(pushed["data"]["transmitted"], true);

    // A teammate's fresh clone sees the team lesson, admitted, and nothing else.
    let b = temp.path().join("b");
    plain(
        temp.path(),
        "git",
        &[
            "clone",
            "-q",
            origin.to_str().expect("path"),
            b.to_str().expect("path"),
        ],
        false,
    );
    plain(&b, "bd", &["bootstrap", "--yes"], true);
    let lessons = lessons(&b);
    let keys = lessons
        .iter()
        .map(|lesson| lesson["key"].clone())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec![json!("values-are-integers")], "{lessons:?}");
    assert_eq!(lessons[0]["status"], "admitted");
    assert_eq!(lessons[0]["decision"]["by"], "Ada");
    assert_eq!(lessons[0]["may_be_stale"], false);
}
