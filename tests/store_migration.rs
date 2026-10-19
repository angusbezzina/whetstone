//! Moving a repository's records from Beads into Virgil, against the fake
//! `virgil` (tests/fake_virgil.py): `wh init --action setup --store virgil`
//! exports every accepted record kind and the decisions that accepted it,
//! refuses to confirm
//! without an export, refuses to flip while any revision is missing or
//! differs (naming it), flips once everything reads back, and `wh dash`
//! reports records left in the store the switch no longer names, on either
//! side of a flip. Bare `wh init` proposes Virgil first on an attached
//! checkout, and nothing under Virgil offers to install `bd`.
//!
//! One test in its own binary: a person's confirmation runs in this process,
//! so the fake goes first on this process's PATH.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::beads::BeadsStore;
use whetstone::domain::ActorKind;
use whetstone::service::{
    CommandService, InitAction, InitRequest, ServiceRequest, ServiceResponse, ServiceState,
};
use whetstone::storage::{RecordStore, StoreKind};
use whetstone::store::virgil::VirgilStore;

mod support;

mod conformance;

const GROUP: &str = "01J8Z0000000000000000000AD";
const REPOSITORY: &str = "01J8Z0000000000000000000RP";

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

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("git");
    assert!(output.status.success(), "git {args:?}");
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

/// `wh init --action store --store <store>` as a person, in this process.
fn store_init(root: &Path, request_id: &str, store: &str, yes: bool) -> ServiceResponse {
    store_init_as(root, request_id, store, yes, ActorKind::Person)
}

fn store_init_as(
    root: &Path,
    request_id: &str,
    store: &str,
    yes: bool,
    actor: ActorKind,
) -> ServiceResponse {
    CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some(request_id.into()),
        action: InitAction::Store,
        store: Some(store.into()),
        yes,
        actor,
        ..InitRequest::default()
    }))
}

fn fake_state(root: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(root.join(".fake-virgil/state.json")).expect("state"),
    )
    .expect("json")
}

fn write_fake_state(root: &Path, state: &Value) {
    std::fs::write(root.join(".fake-virgil/state.json"), state.to_string()).expect("state");
}

/// Run the fake `virgil` with one JSON request on stdin.
fn virgil(root: &Path, args: &[&str], request: Option<&Value>) -> Value {
    let mut child = Command::new("virgil")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("virgil");
    if let Some(request) = request {
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(request.to_string().as_bytes())
            .expect("write");
    }
    let output = child.wait_with_output().expect("virgil");
    assert!(
        output.status.success(),
        "virgil {args:?}: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).expect("json")
}

fn import_rulebook(root: &Path, file: &Path) {
    let request = json!({"scope": "repository", "file": file.display().to_string(), "apply": true});
    virgil(
        root,
        &["rulebook", "import-beads", "--input", "-"],
        Some(&request),
    );
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

fn propose_and_accept(root: &Path, request_id: &str, id: &str, content: &str) -> Value {
    let definition =
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"git --version"}}"#;
    let base = [
        "change",
        "--json",
        "--request-id",
        request_id,
        "--kind",
        "rule",
        "--record-id",
        id,
        "--content",
        content,
        "--rationale",
        "Git answers.",
        "--definition",
        definition,
    ];
    let probe = wh(root, &base);
    let revision = probe["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let resume = probe["resume_token"].as_str().expect("token").to_string();
    let mut args = base.to_vec();
    args.extend(["--expected-revision", &revision, "--resume", &resume]);
    wh(root, &args)
}

#[test]
fn records_move_from_beads_into_virgil_and_flip_only_once_they_read_back() {
    if !ready() {
        return;
    }
    let bin = TempDir::new().expect("bin");
    let program = bin.path().join("virgil");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_virgil.py"),
        &program,
    )
    .expect("copy fake");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    std::env::set_var(
        "PATH",
        format!(
            "{}:{}",
            bin.path().display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    std::env::set_var("PATH", support::path_with_bd_template());
    std::env::remove_var("FAKE_VIRGIL_STATE");
    std::env::remove_var("FAKE_VIRGIL_LOG");
    let home = TempDir::new().expect("home");
    std::env::set_var("HOME", home.path());
    let temp = TempDir::new().expect("temp");
    let root = temp.path().canonicalize().expect("root").join("repo");
    std::fs::create_dir_all(&root).expect("repo");
    let root = root.as_path();
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
    std::fs::create_dir_all(root.join(".fake-virgil")).expect("fake");
    write_fake_state(
        root,
        &json!({"knobs": {"attached": [GROUP, REPOSITORY], "fail": {}, "page": 100}}),
    );

    // ── Beads holds every kind: agreement, proposal, review, retirement,
    // plus a draft that never travels. ──
    // A test command, so the prove-it-works starter is offered.
    std::fs::write(root.join("package.json"), r#"{"scripts":{"test":"true"}}"#)
        .expect("package.json");
    let inspected = wh(root, &["init", "--json", "--request-id", "i1"]);
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = wh(
        root,
        &[
            "init",
            "--json",
            "--action",
            "agree",
            "--request-id",
            "i1",
            "--expected-revision",
            "0",
            "--resume",
            &token,
            "--mission",
            "Move without loss.",
            "--principle",
            "prove-it-works",
            "--starter",
            "prove-it-works",
        ],
    );
    assert_eq!(
        agreed["data"]["progress"]["agreement_complete"], true,
        "{agreed}"
    );
    // Share the agreement through Beads first, so the export has both sources.
    let review = wh(root, &["push", "--json"]);
    let confirm = review["resume_token"]
        .as_str()
        .expect("push token")
        .to_string();
    let pushed = wh(root, &["push", "--json", "--confirm", &confirm]);
    assert!(
        pushed["data"]["copied"]
            .as_array()
            .is_some_and(|copied| copied.len() == 3),
        "{pushed}"
    );
    let changed = propose_and_accept(root, "c1", "rule.git-answers", "Git answers.");
    let proposal = changed["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    let accepted = wh(
        root,
        &[
            "change",
            "--json",
            "--request-id",
            "a1",
            "--accept",
            &proposal,
        ],
    );
    assert_eq!(accepted["state"], "success", "{accepted}");
    let retire = wh(
        root,
        &[
            "change",
            "--json",
            "--request-id",
            "r1",
            "--retire",
            "rule.prove-it-works",
            "--rationale",
            "The tests are checked elsewhere.",
        ],
    );
    let retirement = retire["data"]["proposal"]["id"]
        .as_str()
        .expect("retirement")
        .to_string();
    assert_eq!(
        wh(
            root,
            &[
                "change",
                "--json",
                "--request-id",
                "a2",
                "--accept",
                &retirement
            ]
        )["state"],
        "success"
    );
    let draft = propose_and_accept(root, "c2", "rule.git-still-answers", "Git still answers.");
    assert_eq!(draft["state"], "success", "{draft}");
    assert!(
        fake_state(root)["rulebook"]["repository"]
            .as_array()
            .map_or(true, Vec::is_empty),
        "nothing reaches Virgil before the flip"
    );

    // ── Bare wh init on the attached checkout proposes Virgil first. ──
    std::fs::create_dir_all(root.join(".virgil")).expect("dir");
    std::fs::write(root.join(".virgil/repo.toml"), "repository = \"fixture\"\n").expect("toml");
    let bare = wh(root, &["init", "--json"]);
    assert_eq!(
        bare["data"]["store"]["proposed"],
        json!(["virgil", "beads"]),
        "{bare}"
    );
    assert_eq!(
        bare["permitted_actions"][0], "wh init --action setup --store virgil",
        "{bare}"
    );
    assert_eq!(
        bare["permitted_actions"][1],
        "wh init --action setup --store beads"
    );

    // ── Each refusal names what is missing; nothing flips. ──
    let mut state = fake_state(root);
    state["knobs"]["attached"] = Value::Null;
    write_fake_state(root, &state);
    let unattached = store_init(root, "s0", "virgil", false);
    assert_eq!(
        unattached.state,
        ServiceState::NeedsInput,
        "{}",
        unattached.summary
    );
    assert_eq!(
        unattached.data["migration"]["missing"],
        ".virgil/repo.toml (virgil repos attach)"
    );
    state["knobs"]["attached"] = json!([GROUP, REPOSITORY]);
    state["knobs"]["no_rulebook"] = json!(true);
    write_fake_state(root, &state);
    let old_virgil = store_init(root, "s0b", "virgil", false);
    assert_eq!(
        old_virgil.state,
        ServiceState::Conflict,
        "{}",
        old_virgil.summary
    );
    assert!(
        old_virgil.summary.contains("virgil rulebook"),
        "{}",
        old_virgil.summary
    );
    state["knobs"]["no_rulebook"] = json!(false);
    write_fake_state(root, &state);
    let without_export = store_init(root, "s1", "virgil", true);
    assert_eq!(
        without_export.state,
        ServiceState::NeedsInput,
        "{}",
        without_export.summary
    );
    assert!(
        without_export.summary.contains("nothing to confirm"),
        "{}",
        without_export.summary
    );
    assert!(!root.join("whetstone/store.json").exists());

    // ── The review writes the export, outside the repository. ──
    let reviewed = store_init(root, "s2", "virgil", false);
    assert_eq!(
        reviewed.state,
        ServiceState::NeedsDecision,
        "{}",
        reviewed.summary
    );
    assert!(
        !root.join("whetstone/store.json").exists(),
        "no flip before the import"
    );
    let rulebook_file = PathBuf::from(
        reviewed.data["migration"]["rulebook"]["file"]
            .as_str()
            .expect("rulebook file"),
    );
    assert!(!rulebook_file.starts_with(root));
    let export: Value =
        serde_json::from_str(&std::fs::read_to_string(&rulebook_file).expect("export"))
            .expect("json");
    assert_eq!(export["schema"], "whetstone.rulebook-export/v1");
    assert_eq!(export["scope"]["repository"], REPOSITORY);
    let exported = export["records"].as_array().expect("records").clone();
    let types = exported
        .iter()
        .map(|agreement| agreement["type"].as_str().expect("type").to_string())
        .collect::<std::collections::BTreeSet<_>>();
    for kind in [
        "mission",
        "principle",
        "rule",
        "proposal",
        "local_review",
        "retirement",
    ] {
        assert!(types.contains(kind), "{kind} is exported: {types:?}");
    }
    // Each revision carries its resolved lifecycle view, not "accepted".
    let lifecycle_of = |id: &str| {
        exported
            .iter()
            .rfind(|agreement| agreement["record_id"] == id)
            .map(|agreement| agreement["lifecycle"].clone())
    };
    assert_eq!(lifecycle_of("rule.prove-it-works"), Some(json!("retired")));
    assert_eq!(lifecycle_of("rule.git-answers"), Some(json!("accepted")));
    assert_eq!(lifecycle_of(&proposal), Some(json!("operational")));
    let ids = exported
        .iter()
        .map(|agreement| agreement["record_id"].as_str().expect("id").to_string())
        .collect::<Vec<_>>();
    assert!(
        !ids.contains(&"rule.git-still-answers".to_string()),
        "a draft stays"
    );
    let sources = exported
        .iter()
        .map(|agreement| agreement["evidence"]["source"].as_str().expect("source"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(sources, ["private", "shared"].into_iter().collect());
    assert!(reviewed
        .permitted_actions
        .iter()
        .any(|action| action.contains("virgil rulebook import-beads --input -")));
    assert_eq!(
        reviewed.permitted_actions.last().map(String::as_str),
        Some("wh init --action setup --store virgil --yes")
    );

    // The person applies the import (Virgil's command, faked).
    import_rulebook(root, &rulebook_file);

    // ── One revision missing: refused, named. ──
    let mut state = fake_state(root);
    let removed = state["rulebook"]["repository"]
        .as_array_mut()
        .expect("repository scope")
        .remove(0);
    write_fake_state(root, &state);
    let missing = store_init(root, "s3", "virgil", true);
    assert_eq!(
        missing.state,
        ServiceState::NeedsInput,
        "{}",
        missing.summary
    );
    let named = format!(
        "{}@{} (missing)",
        removed["record_id"].as_str().expect("id"),
        removed["revision"]
    );
    assert!(missing.summary.contains(&named), "{}", missing.summary);
    assert!(!root.join("whetstone/store.json").exists());
    import_rulebook(root, &rulebook_file);

    // ── One revision with another body: refused, named. ──
    let mut state = fake_state(root);
    let original = state["rulebook"]["repository"][0]["wh_record"].clone();
    let tampered_id = format!(
        "{}@{}",
        state["rulebook"]["repository"][0]["record_id"]
            .as_str()
            .expect("id"),
        state["rulebook"]["repository"][0]["revision"]
    );
    state["rulebook"]["repository"][0]["wh_record"] = json!(original
        .as_str()
        .expect("record")
        .replacen("\"statement\":\"", "\"statement\":\"Edited ", 1));
    write_fake_state(root, &state);
    let mismatched = store_init(root, "s4", "virgil", true);
    assert_eq!(
        mismatched.state,
        ServiceState::Conflict,
        "{}",
        mismatched.summary
    );
    assert!(
        mismatched
            .summary
            .contains(&format!("{tampered_id} (different digest)")),
        "{}",
        mismatched.summary
    );
    assert!(!root.join("whetstone/store.json").exists());
    let mut state = fake_state(root);
    state["rulebook"]["repository"][0]["wh_record"] = original;
    write_fake_state(root, &state);

    // ── Everything reads back, but an agent cannot confirm the move. ──
    let agent = store_init_as(root, "s5a", "virgil", true, ActorKind::Agent);
    assert_eq!(agent.state, ServiceState::Conflict, "{}", agent.summary);
    assert!(agent.summary.contains("a person's"), "{}", agent.summary);
    assert!(!root.join("whetstone/store.json").exists());

    // ── Everything reads back: the switch flips. ──
    // The confirmation is the spelling Whetstone prints: setup with --store
    // and --yes. Its --yes confirms the move and nothing else: the tool
    // installs setup detects are listed, never run.
    let flipped = CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("s5".into()),
        action: InitAction::Setup,
        store: Some("virgil".into()),
        yes: true,
        actor: ActorKind::Person,
        ..InitRequest::default()
    }));
    // An install that ran lands in `ran`, one that was tried and failed in
    // `failed`; the move's --yes may do neither.
    assert_eq!(
        (&flipped.data["ran"], &flipped.data["failed"]),
        (&json!([]), &json!([])),
        "the move's --yes tried no install: {}",
        flipped.data
    );
    // Tool setup may still need a person (the virgil CLI may not answer), so
    // the move is read from the switch's own answer inside setup's.
    assert!(
        matches!(
            flipped.state,
            ServiceState::Success | ServiceState::NeedsDecision
        ),
        "{}: {}",
        flipped.summary,
        flipped.data
    );
    let flipped = flipped.data["store"].clone();
    assert_eq!(flipped["written"], true, "{flipped}");
    assert_eq!(
        serde_json::from_str::<Value>(
            &std::fs::read_to_string(root.join("whetstone/store.json")).expect("switch")
        )
        .expect("json"),
        json!({"store": "virgil"})
    );
    assert_eq!(
        flipped["migration"]["rulebook"]["confirmed"],
        exported.len(),
        "{}",
        flipped
    );
    assert!(
        flipped["migration"]["carried_private"]
            .as_u64()
            .expect("carried")
            > 0
    );
    let virgil_store = VirgilStore::shared(root);
    for agreement in &exported {
        let id = whetstone::domain::RecordId::new(agreement["record_id"].as_str().expect("id"))
            .expect("id");
        let revision = agreement["revision"].as_u64().expect("revision");
        let found = virgil_store
            .fetch(&id, revision)
            .expect("fetch")
            .unwrap_or_else(|| panic!("{} read back", id.as_str()));
        assert_eq!(
            found.reference.digest.as_str(),
            agreement["digest"],
            "{}",
            id.as_str()
        );
    }

    // After the flip: the draft moved to the private scope and nothing is
    // left behind.
    assert!(
        fake_state(root)["rulebook"]["private"]
            .as_array()
            .is_some_and(|records| !records.is_empty()),
        "the private Beads store was carried into the private scope"
    );
    let dash = wh(root, &["dash", "--json"]);
    assert_eq!(dash["data"]["store"]["kind"], "virgil");
    assert_eq!(
        dash["data"]["store"]["left_behind"],
        json!([]),
        "{}",
        dash["data"]["store"]
    );
    assert_eq!(
        dash["data"]["current"]["header"]["drafts"], 1,
        "the draft survived"
    );
    let inspected = wh(root, &["init", "--json"]).to_string();
    assert!(
        !inspected.contains("brew install beads") && !inspected.contains("releases/download"),
        "never offers to install bd under Virgil: {inspected}"
    );
    let pushed = wh(root, &["push", "--json"]);
    assert_eq!(pushed["data"]["refusal"], "records_sync_through_the_daemon");

    // ── A record that only reached Beads after the flip is reported. ──
    let shared = BeadsStore::open_existing(root, StoreKind::Shareable).expect("shared beads");
    let stray = conformance::record(
        "rule.stray-in-beads",
        1,
        "stray:beads",
        conformance::rule("Stray."),
    );
    shared.append(&stray, None).expect("stray");
    let left = wh(root, &["dash", "--json"])["data"]["store"]["left_behind"].clone();
    assert!(
        left.as_array()
            .expect("left")
            .iter()
            .any(|item| item["record"] == "rule.stray-in-beads"
                && item["store"] == "beads"
                && item["kind"] == "rule"),
        "{left}"
    );

    // ── Flipping back: records only in Virgil are reported. ──
    let back = store_init(root, "s6", "beads", false);
    ok(&back);
    assert_eq!(
        serde_json::from_str::<Value>(
            &std::fs::read_to_string(root.join("whetstone/store.json")).expect("switch")
        )
        .expect("json"),
        json!({"store": "beads"})
    );
    let only_virgil = conformance::record(
        "rule.only-in-virgil",
        1,
        "stray:virgil",
        conformance::rule("Only in Virgil."),
    );
    virgil_store
        .append(&only_virgil, None)
        .expect("virgil only");
    let only_private = conformance::record(
        "rule.only-in-private-scope",
        1,
        "stray:private",
        conformance::rule("Only in my private scope."),
    );
    VirgilStore::private(root)
        .append(&only_private, None)
        .expect("private only");
    let left = wh(root, &["dash", "--json"])["data"]["store"]["left_behind"].clone();
    assert!(
        left.as_array()
            .expect("left")
            .iter()
            .any(|item| item["record"] == "rule.only-in-virgil" && item["store"] == "virgil"),
        "{left}"
    );
    assert!(
        left.as_array()
            .expect("left")
            .iter()
            .any(|item| item["record"] == "rule.only-in-private-scope" && item["store"] == "virgil"),
        "{left}"
    );
    assert!(
        !left
            .as_array()
            .expect("left")
            .iter()
            .any(|item| item["record"] == "rule.stray-in-beads"),
        "{left}"
    );

    // ── An accepted revision that collides with the shared database is
    // named and the move refused, never dropped. ──
    let collide = propose_and_accept(root, "c3", "rule.collide", "Ours.");
    let collide_proposal = collide["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    assert_eq!(
        wh(
            root,
            &[
                "change",
                "--json",
                "--request-id",
                "a3",
                "--accept",
                &collide_proposal
            ]
        )["state"],
        "success"
    );
    let theirs = conformance::record(
        "rule.collide",
        1,
        "theirs:collide",
        conformance::rule("Theirs."),
    );
    shared.append(&theirs, None).expect("theirs");
    let collided = store_init(root, "s7", "virgil", false);
    assert_eq!(
        collided.state,
        ServiceState::Conflict,
        "{}",
        collided.summary
    );
    assert!(
        collided.summary.contains("rule.collide@1"),
        "{}",
        collided.summary
    );
    assert_eq!(
        serde_json::from_str::<Value>(
            &std::fs::read_to_string(root.join("whetstone/store.json")).expect("switch")
        )
        .expect("json"),
        json!({"store": "beads"}),
        "the switch did not flip"
    );
}
