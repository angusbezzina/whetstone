use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use whetstone::beads::BeadsStore;
use whetstone::domain::{AgreementRecord, RecordBody};
use whetstone::storage::{ProjectLayout, StoreKind};

#[path = "support/rulebook.rs"]
mod rulebook;
mod support;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("PATH", support::path_with_bd_template())
        .env_remove("BEADS_DIR")
        .output()
        .expect("run lean Whetstone binary")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git in isolated fixture");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 git output")
}

fn init_git(root: &Path) {
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Test Owner"]);
    git(root, &["config", "user.email", "owner@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn git_status(root: &Path) -> String {
    git(root, &["status", "--porcelain=v1"])
}

/// A throwaway Git repository: never this repository's `.beads` or store.
struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create isolated repository");
        let root = temp.path().canonicalize().expect("canonical fixture root");
        init_git(&root);
        // A test command that passes, so the prove-it-works starter is one a
        // check can hold; kept out of Git so the tree stays clean.
        std::fs::write(root.join("package.json"), r#"{"scripts":{"test":"true"}}"#)
            .expect("write package.json");
        std::fs::write(root.join(".git/info/exclude"), "package.json\n")
            .expect("exclude package.json");
        Self { _temp: temp, root }
    }

    fn wh(&self, args: &[&str]) -> Output {
        let mut all = args.to_vec();
        all.push("--json");
        run(&all, &self.root)
    }

    fn wh_json(&self, args: &[&str]) -> serde_json::Value {
        json(&self.wh(args))
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(full, text).expect("write fixture file");
    }

    fn commit(&self, message: &str) {
        git(&self.root, &["add", "-A"]);
        git(
            &self.root,
            &["commit", "--quiet", "--no-verify", "-m", message],
        );
    }

    /// Inspect, then agree with the returned revision and token.
    fn agree(&self, request_id: &str, answers: &[&str]) -> serde_json::Value {
        let inspect = self.wh_json(&["init", "--request-id", request_id]);
        let revision = inspect["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let token = inspect["resume_token"]
            .as_str()
            .expect("resume token")
            .to_string();
        let mut args = vec![
            "init",
            "--action",
            "agree",
            "--request-id",
            request_id,
            "--expected-revision",
            &revision,
            "--resume",
            &token,
        ];
        args.extend_from_slice(answers);
        self.wh_json(&args)
    }

    /// Probe for the base revision and token, then record the draft.
    fn propose(
        &self,
        request_id: &str,
        kind: &str,
        record_id: &str,
        content: &str,
        definition: Option<&str>,
    ) -> serde_json::Value {
        let mut base = vec![
            "change",
            "--request-id",
            request_id,
            "--kind",
            kind,
            "--record-id",
            record_id,
            "--content",
            content,
            "--rationale",
            "The owner wants this.",
        ];
        if let Some(definition) = definition {
            base.extend(["--definition", definition]);
        }
        let probe = self.wh_json(&base);
        assert_eq!(probe["state"], "needs_input", "{probe}");
        let revision = probe["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let token = probe["resume_token"].as_str().expect("token").to_string();
        let mut args = base.clone();
        args.extend(["--expected-revision", &revision, "--resume", &token]);
        self.wh_json(&args)
    }

    fn accept(&self, request_id: &str, proposal: &str) -> serde_json::Value {
        self.wh_json(&["change", "--request-id", request_id, "--accept", proposal])
    }

    /// Write a rule into the rulebook, where a check reads it.
    fn rule(&self, record_id: &str, definition: &str) {
        rulebook::add_rule(
            &self.root,
            record_id,
            &format!("Statement of {record_id}."),
            &serde_json::from_str(definition).expect("definition JSON"),
        );
    }

    fn check(&self, args: &[&str]) -> (Option<i32>, serde_json::Value) {
        let mut all = vec!["check"];
        all.extend_from_slice(args);
        let output = self.wh(&all);
        (output.status.code(), json(&output))
    }

    /// The rules `wh dash` shows as in force or pending.
    fn rule_entries(&self) -> Vec<serde_json::Value> {
        self.wh_json(&["dash"])["data"]["current"]["rules"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn rule_entry(&self, id: &str) -> serde_json::Value {
        self.rule_entries()
            .into_iter()
            .find(|rule| rule["id"] == id)
            .unwrap_or_else(|| panic!("no rule {id} in wh dash"))
    }

    fn records(&self) -> Vec<AgreementRecord> {
        let layout = ProjectLayout::resolve(&self.root).expect("layout");
        BeadsStore::open_existing(&layout.private_store(), StoreKind::Private)
            .expect("private store")
            .all_records()
            .expect("records")
    }
}

fn gate<'a>(response: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    response["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == id)
        .unwrap_or_else(|| panic!("no gate {id} in {response}"))
}

fn record_ids(response: &serde_json::Value) -> Vec<String> {
    response["data"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["id"].as_str().expect("record id").to_string())
        .collect()
}

const AGREEMENT: [&str; 6] = [
    "--mission",
    "Keep project work aligned.",
    "--principle",
    "prove-it-works",
    "--starter",
    "prove-it-works",
];

#[test]
fn bare_json_is_truthful_read_only_orientation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run(&["--json"], root);
    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["schema"], "whetstone.command-response.v1");
    assert_eq!(value["state"], "success");
    assert_eq!(value["workflow"], "orientation");
    assert_eq!(
        value["data"]["workflows"],
        serde_json::json!(["init", "dash", "change", "check", "pull", "push"])
    );
    assert_eq!(value["data"]["read_only"], true);
    let actions = value["permitted_actions"]
        .as_array()
        .expect("permitted actions")
        .iter()
        .take(6)
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        [
            "wh init",
            "wh dash",
            "wh change",
            "wh check",
            "wh pull",
            "wh push"
        ]
    );
    // Orientation reports tools honestly: every listed tool has a state.
    for tool in value["data"]["tools"].as_array().expect("tools") {
        assert!(tool["state"]
            .as_str()
            .is_some_and(|state| !state.is_empty()));
    }
}

#[test]
fn public_help_exposes_exactly_six_workflow_families() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run(&["--help"], root);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    let commands = help
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        ["init", "dash", "change", "check", "pull", "push"]
    );
}

#[test]
fn retired_commands_cannot_execute() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for command in [
        "publish", "status", "debt", "report", "mcp", "actions", "validate", "eval", "scan",
    ] {
        let output = run(&[command], root);
        assert_eq!(output.status.code(), Some(2), "{command} unexpectedly ran");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"),
            "unexpected error for {command}"
        );
    }

    let old_check_alias = run(&["check", "src"], root);
    assert_eq!(old_check_alias.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&old_check_alias.stderr).contains("unexpected argument"));
}

#[test]
fn sync_is_honestly_unavailable_and_dashboard_inspects() {
    // A throwaway repository: never this repository's real .beads.
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    assert!(std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .expect("git")
        .success());
    let pull = run(&["pull", "--json", "--request-id", "probe-1"], root);
    assert_eq!(pull.status.code(), Some(4));
    let value = json(&pull);
    assert_eq!(value["schema"], "whetstone.command-response.v1");
    assert_eq!(value["state"], "unavailable");
    assert_eq!(value["request_id"], "probe-1");
    assert!(value["summary"]
        .as_str()
        .expect("summary")
        .contains("no shared Beads database"));
    let push = run(&["push", "--json", "--request-id", "probe-1"], root);
    assert_eq!(push.status.code(), Some(6));
    let value = json(&push);
    assert_eq!(value["state"], "needs_input");
    assert!(value["summary"]
        .as_str()
        .expect("summary")
        .contains("nothing was published"));
    let output = run(&["dash", "--json", "--request-id", "probe-1"], root);
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output);
    assert_eq!(value["state"], "success");
    assert_eq!(value["data"]["read_only"], true);
}

#[test]
fn removed_flags_and_change_kinds_are_rejected_before_anything_runs() {
    let repo = Repo::new();
    for args in [
        vec!["init", "--action", "agree", "--desired-outcome", "x"],
        vec!["init", "--action", "agree", "--values", "x"],
        vec!["init", "--action", "agree", "--philosophy", "x"],
        vec!["init", "--action", "agree", "--owner", "x"],
        vec!["init", "--action", "agree", "--initial-safeguard", "x"],
        vec!["init", "--action", "agree", "--gate-command", "true"],
        vec!["change", "--kind", "value"],
        vec!["change", "--kind", "philosophy"],
        vec!["change", "--kind", "metric"],
        vec!["change", "--kind", "standard"],
        vec!["change", "--kind", "guidance"],
        vec!["change", "--kind", "exception"],
        vec!["change", "--activate", "proposal.x"],
        vec!["change", "--review-triggers", "x"],
        vec!["check", "--required"],
        vec!["check", "--repair-session", "x"],
        vec!["check", "--path", "src"],
        vec!["check", "--lang", "rust"],
        vec!["change", "--migrate"],
        vec!["push", "--propose"],
        vec!["dash", "--history-after", "x"],
    ] {
        let output = run(&args, &repo.root);
        assert_eq!(output.status.code(), Some(2), "{args:?} unexpectedly ran");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("unexpected argument") || stderr.contains("invalid value"),
            "{args:?}: {stderr}"
        );
    }
    assert!(!repo.root.join(".git/whetstone").exists());
    assert!(git_status(&repo.root).is_empty());
}

#[test]
fn init_is_resumable_and_duplicate_requests_never_duplicate_records() {
    let temp = tempfile::tempdir().expect("create init fixture");
    init_git(temp.path());
    // A test command makes the prove-it-works starter one a check can hold.
    std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n")
        .expect("write Cargo.toml");
    git(temp.path(), &["add", "-A"]);
    git(
        temp.path(),
        &["commit", "--quiet", "--no-verify", "-m", "base"],
    );
    let project = temp.path().to_string_lossy();
    let inspect = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "onboard-1",
        ],
        temp.path(),
    );
    assert_eq!(inspect.status.code(), Some(6));
    let handoff = json(&inspect);
    assert_eq!(handoff["state"], "needs_input");
    assert_eq!(handoff["data"]["inspection_writes"], serde_json::json!([]));
    assert_eq!(handoff["data"]["read_only"], true);
    assert_eq!(
        handoff["data"]["progress"]["missing_decisions"],
        serde_json::json!(["mission", "rules"])
    );
    assert_eq!(handoff["data"]["progress"]["agreement_complete"], false);
    let steps = handoff["data"]["onboarding"]["steps"]
        .as_array()
        .expect("onboarding steps")
        .iter()
        .map(|step| step["key"].as_str().expect("step key"))
        .collect::<Vec<_>>();
    assert_eq!(steps, ["tools", "mission", "principles", "rules", "gates"]);
    assert!(handoff["data"]["onboarding"]["starters"]
        .as_array()
        .expect("starters")
        .iter()
        .all(|starter| starter["accepted"] == false));
    assert!(!temp.path().join(".git/whetstone").exists());
    assert!(git_status(temp.path()).is_empty());
    let token = handoff["resume_token"].as_str().expect("resume token");
    let revision = handoff["expected_revision"].as_u64().expect("revision");
    assert_eq!(revision, 0);

    let restarted = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "onboard-1",
        ],
        temp.path(),
    );
    assert_eq!(json(&restarted)["resume_token"], token);

    let revision_text = revision.to_string();
    let stale = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--action",
            "agree",
            "--request-id",
            "onboard-1",
            "--expected-revision",
            "7",
            "--resume",
            token,
            "--mission",
            "Keep project work aligned.",
            "--starter",
            "all",
        ],
        temp.path(),
    );
    assert_eq!(stale.status.code(), Some(7));
    assert_eq!(json(&stale)["state"], "stale");

    let mut agreement_args = vec![
        "init",
        "--json",
        "--project-dir",
        &project,
        "--action",
        "agree",
        "--request-id",
        "onboard-1",
        "--expected-revision",
        &revision_text,
        "--resume",
        token,
    ];
    agreement_args.extend([
        "--mission",
        "Keep project work aligned.",
        "--principle",
        "prove-it-works",
        "--custom-principle",
        "Leave the campsite cleaner.",
        "--starter",
        "all",
    ]);
    let accepted = run(&agreement_args, temp.path());
    assert_eq!(
        accepted.status.code(),
        Some(6),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&accepted.stdout),
        String::from_utf8_lossy(&accepted.stderr)
    );
    let accepted_json = json(&accepted);
    assert_eq!(accepted_json["state"], "needs_input");
    let mut ids = record_ids(&accepted_json);
    ids.sort();
    assert_eq!(ids.len(), 4, "{accepted_json}");
    assert_eq!(ids[0], "mission.project");
    assert!(ids.contains(&"principle.prove-it-works".to_string()));
    assert!(ids.iter().any(|id| id.starts_with("principle.custom-")));
    assert!(ids.contains(&"rule.prove-it-works".to_string()));
    assert_eq!(
        accepted_json["data"]["progress"]["agreement_complete"],
        true
    );
    assert_eq!(
        accepted_json["data"]["progress"]["missing_decisions"],
        serde_json::json!([])
    );
    assert_eq!(accepted_json["data"]["shared"], false);
    let layout = ProjectLayout::resolve(temp.path()).expect("project layout");
    assert!(layout.shared_store().is_none());
    assert!(git_status(temp.path()).is_empty());
    let private = BeadsStore::open_existing(&layout.private_store(), StoreKind::Private)
        .expect("private store");
    let count = private.all_records().expect("records").len();
    // Onboarding records are the owner's own words: in force, no drafts.
    assert!(!private
        .all_records()
        .expect("records")
        .iter()
        .any(|record| matches!(record.body, RecordBody::Proposal(_))));

    let established = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "inspect-established",
        ],
        temp.path(),
    );
    assert_eq!(established.status.code(), Some(6));
    let established_json = json(&established);
    assert_eq!(established_json["state"], "needs_input");
    assert_eq!(
        established_json["data"]["progress"]["missing_decisions"],
        serde_json::json!([])
    );
    assert_eq!(
        established_json["data"]["progress"]["agreement_complete"],
        true
    );
    assert_eq!(
        established_json["data"]["inspection_writes"],
        serde_json::json!([])
    );
    assert!(established_json["data"]["onboarding"]["starters"]
        .as_array()
        .expect("starters")
        .iter()
        .all(|starter| starter["accepted"] == true));

    // Repeating the request never writes a second copy of anything.
    let replay = run(&agreement_args, temp.path());
    assert_ne!(json(&replay)["state"], "success");
    assert_eq!(private.all_records().expect("records").len(), count);
}

#[test]
fn init_agree_exact_replay_returns_the_same_records_and_reuse_conflicts() {
    let repo = Repo::new();
    let inspect = repo.wh_json(&["init", "--request-id", "replay-1"]);
    let token = inspect["resume_token"].as_str().expect("token").to_string();
    let mut args = vec![
        "init",
        "--action",
        "agree",
        "--request-id",
        "replay-1",
        "--expected-revision",
        "0",
        "--resume",
        &token,
    ];
    args.extend_from_slice(&AGREEMENT);
    let first = repo.wh(&args);
    assert_eq!(first.status.code(), Some(6));
    let first = json(&first);
    let replay = repo.wh(&args);
    assert_eq!(replay.status.code(), Some(6), "{}", json(&replay));
    assert_eq!(json(&replay)["data"]["records"], first["data"]["records"]);

    let different = args
        .iter()
        .map(|arg| {
            if *arg == "Keep project work aligned." {
                "A different mission."
            } else {
                arg
            }
        })
        .collect::<Vec<_>>();
    let conflict = repo.wh(&different);
    assert_eq!(conflict.status.code(), Some(8), "{}", json(&conflict));
    assert_eq!(json(&conflict)["state"], "conflict");
}

#[test]
fn init_needs_a_mission_and_names_only_catalogue_principles_and_starters() {
    let repo = Repo::new();
    std::fs::create_dir_all(repo.root.join("whetstone/packs/team"))
        .expect("create inert import fixture");
    std::fs::write(
        repo.root.join("whetstone/packs/team/install.sh"),
        "touch executed\nexit 99\n",
    )
    .expect("write inert imported content");
    let handoff = repo.wh_json(&["init", "--request-id", "explicit-input"]);
    assert_eq!(handoff["state"], "needs_input");
    assert!(!repo.root.join(".git/whetstone").exists());
    let token = handoff["resume_token"].as_str().expect("token").to_string();
    let agree = |extra: &[&str]| {
        let mut args = vec![
            "init",
            "--action",
            "agree",
            "--request-id",
            "explicit-input",
            "--expected-revision",
            "0",
            "--resume",
            &token,
        ];
        args.extend_from_slice(extra);
        repo.wh(&args)
    };

    // No mission: nothing is recorded, and the question is asked.
    let no_mission = agree(&["--starter", "all"]);
    assert_eq!(no_mission.status.code(), Some(6));
    let no_mission = json(&no_mission);
    assert_eq!(no_mission["state"], "needs_input");
    assert!(no_mission["blocking_questions"][0]
        .as_str()
        .expect("question")
        .contains("mission"));

    for (extra, needle) in [
        (
            [
                "--mission",
                "Keep work aligned.",
                "--principle",
                "not-a-principle",
            ],
            "not-a-principle is not a principle in the catalogue",
        ),
        (
            [
                "--mission",
                "Keep work aligned.",
                "--starter",
                "rule.made-up",
            ],
            "rule.made-up is not a starter rule",
        ),
    ] {
        let output = agree(&extra);
        assert_eq!(output.status.code(), Some(6));
        let value = json(&output);
        assert!(
            value["blocking_questions"][0]
                .as_str()
                .expect("question")
                .contains(needle),
            "{value}"
        );
    }
    let blank = agree(&["--mission", "   ", "--starter", "all"]);
    assert_eq!(blank.status.code(), Some(6));

    let layout = ProjectLayout::resolve(&repo.root).expect("project layout");
    // A refused agreement writes nothing, not even an empty store.
    assert!(
        !layout.private_store_exists(),
        "a refused agreement created the private store"
    );
    assert!(layout.shared_store().is_none());
    assert!(!repo.root.join("executed").exists());
}

#[test]
fn init_completes_an_established_agreement_that_has_no_rule_yet() {
    let repo = Repo::new();
    repo.write("Cargo.toml", "[package]\nname = \"x\"\n");
    repo.commit("base");
    let mission_only = repo.agree("mission-only", &["--mission", "Keep work aligned."]);
    assert_eq!(mission_only["state"], "needs_input");
    assert_eq!(record_ids(&mission_only), ["mission.project"]);
    assert_eq!(
        mission_only["data"]["progress"]["agreement_complete"],
        false
    );
    assert_eq!(
        mission_only["data"]["progress"]["missing_decisions"],
        serde_json::json!(["rules"])
    );

    let inspect = repo.wh_json(&["init", "--request-id", "add-rules"]);
    assert_eq!(inspect["expected_revision"], 1);
    assert_eq!(
        inspect["data"]["progress"]["missing_decisions"],
        serde_json::json!(["rules"])
    );
    // A second mission through init is refused: missions change through wh change.
    let token = inspect["resume_token"].as_str().expect("token").to_string();
    let again = repo.wh(&[
        "init",
        "--action",
        "agree",
        "--request-id",
        "add-rules",
        "--expected-revision",
        "1",
        "--resume",
        &token,
        "--mission",
        "Another mission.",
    ]);
    assert_eq!(again.status.code(), Some(5));
    assert_eq!(json(&again)["state"], "needs_decision");

    let completed = repo.wh_json(&[
        "init",
        "--action",
        "agree",
        "--request-id",
        "add-rules",
        "--expected-revision",
        "1",
        "--resume",
        &token,
        "--starter",
        "prove-it-works",
    ]);
    assert_eq!(record_ids(&completed), ["rule.prove-it-works"]);
    assert_eq!(completed["data"]["progress"]["agreement_complete"], true);
    let missions = repo
        .records()
        .into_iter()
        .filter(|record| record.id.as_str() == "mission.project")
        .count();
    assert_eq!(missions, 1);
}

#[test]
fn init_dry_run_shows_the_exact_records_and_writes_nothing() {
    let repo = Repo::new();
    let inspect = repo.wh_json(&["init", "--request-id", "dry-1"]);
    let token = inspect["resume_token"].as_str().expect("token").to_string();
    let mut args = vec![
        "init",
        "--action",
        "agree",
        "--request-id",
        "dry-1",
        "--expected-revision",
        "0",
        "--resume",
        &token,
        "--dry-run",
    ];
    args.extend_from_slice(&AGREEMENT);
    let output = repo.wh(&args);
    assert_eq!(output.status.code(), Some(5));
    let value = json(&output);
    assert_eq!(value["data"]["dry_run"], true);
    assert_eq!(value["data"]["records"].as_array().map(Vec::len), Some(3));
    assert_eq!(value["data"]["effects"]["team_share"], false);
    assert!(!ProjectLayout::resolve(&repo.root)
        .expect("layout")
        .private_store_exists());
}

#[test]
fn init_agree_resumes_after_interruption_between_store_bootstrap_and_records() {
    let repo = Repo::new();
    let handoff = repo.wh_json(&["init", "--request-id", "resume-bootstrap"]);
    let token = handoff["resume_token"].as_str().expect("token").to_string();
    let layout = ProjectLayout::resolve(&repo.root).expect("layout");
    // Model a process stop right after the private Beads store was created.
    BeadsStore::initialize(&layout.private_store(), StoreKind::Private)
        .expect("model the bootstrapped, empty store");
    assert!(repo.records().is_empty());

    let cancelled = repo.wh(&[
        "init",
        "--action",
        "cancel",
        "--request-id",
        "cancel-interrupted-bootstrap",
    ]);
    assert!(cancelled.status.success());
    let cancelled = json(&cancelled);
    assert_eq!(cancelled["data"]["cancelled"], true);
    assert_eq!(cancelled["data"]["writes"], serde_json::json!([]));
    assert!(repo.records().is_empty(), "cancel wrote a record");

    let again = repo.wh_json(&["init", "--request-id", "resume-bootstrap"]);
    assert_eq!(again["resume_token"], token.as_str());
    let mut args = vec![
        "init",
        "--action",
        "agree",
        "--request-id",
        "resume-bootstrap",
        "--expected-revision",
        "0",
        "--resume",
        &token,
    ];
    args.extend_from_slice(&AGREEMENT);
    let resumed = repo.wh(&args);
    assert_eq!(resumed.status.code(), Some(6));
    let resumed = json(&resumed);
    assert_eq!(resumed["data"]["progress"]["agreement_complete"], true);
    let agreement = repo
        .records()
        .into_iter()
        .filter(|record| {
            matches!(
                record.body,
                RecordBody::Mission(_) | RecordBody::Principle(_) | RecordBody::Rule(_)
            )
        })
        .count();
    assert_eq!(agreement, 3);
}

#[test]
fn init_agree_completes_a_partly_written_batch() {
    let repo = Repo::new();
    let handoff = repo.wh_json(&["init", "--request-id", "partial"]);
    let token = handoff["resume_token"].as_str().expect("token").to_string();
    let layout = ProjectLayout::resolve(&repo.root).expect("layout");
    let private =
        BeadsStore::initialize(&layout.private_store(), StoreKind::Private).expect("private store");
    // The first record of the batch landed before the process stopped.
    let mission = whetstone::service::agreement_record(
        &layout,
        "mission.project",
        "partial:base-0:mission".into(),
        RecordBody::Mission(whetstone::domain::Mission {
            statement: "Keep project work aligned.".into(),
            desired_outcomes: Vec::new(),
        }),
        Some("Test Owner"),
    )
    .expect("mission record");
    let written = private.append(&mission, None).expect("partial write");
    let mut args = vec![
        "init",
        "--action",
        "agree",
        "--request-id",
        "partial",
        "--expected-revision",
        "0",
        "--resume",
        &token,
    ];
    args.extend_from_slice(&AGREEMENT);
    let resumed = repo.wh(&args);
    assert_eq!(resumed.status.code(), Some(6), "{}", json(&resumed));
    let resumed = json(&resumed);
    assert_eq!(resumed["data"]["progress"]["agreement_complete"], true);
    assert!(resumed["data"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .any(|record| record["digest"] == written.digest.as_str()));
    assert_eq!(
        repo.records()
            .iter()
            .filter(|record| record.id.as_str() == "mission.project")
            .count(),
        1
    );
}

#[test]
fn init_keeps_the_base_revision_when_new_records_have_no_supersedes() {
    let repo = Repo::new();
    repo.write("Cargo.toml", "[package]\nname = \"x\"\n");
    repo.commit("base");
    // An accepted principle exists before onboarding.
    let drafted = repo.propose(
        "existing-principle",
        "principle",
        "principle.small-steps",
        "Take small, reversible steps.",
        None,
    );
    assert_eq!(drafted["state"], "success", "{drafted}");
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    assert_eq!(
        repo.accept("accept-principle", &proposal)["state"],
        "success"
    );

    let inspect = repo.wh_json(&["init", "--request-id", "fill-around-principle"]);
    assert_eq!(inspect["expected_revision"], 1);
    assert_eq!(
        inspect["data"]["progress"]["missing_decisions"],
        serde_json::json!(["mission", "rules"])
    );
    let token = inspect["resume_token"].as_str().expect("token").to_string();
    let agreed = repo.wh(&[
        "init",
        "--action",
        "agree",
        "--request-id",
        "fill-around-principle",
        "--expected-revision",
        "1",
        "--resume",
        &token,
        "--mission",
        "Keep project work aligned.",
        "--starter",
        "prove-it-works",
    ]);
    assert_eq!(agreed.status.code(), Some(6));
    let agreed = json(&agreed);
    let mut ids = record_ids(&agreed);
    ids.sort();
    assert_eq!(ids, ["mission.project", "rule.prove-it-works"]);
    for record in repo.records() {
        if matches!(record.body, RecordBody::Mission(_) | RecordBody::Rule(_)) {
            assert_eq!(record.revision, 1);
            assert!(record.supersedes.is_none());
            assert!(record
                .idempotency_key
                .starts_with("fill-around-principle:base-1:"));
        }
    }
    assert_eq!(agreed["data"]["progress"]["agreement_complete"], true);
}

#[test]
fn init_inspect_and_cancel_from_nested_directory_are_read_only() {
    let temp = tempfile::tempdir().expect("create nested onboarding fixture");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("apps/web/src/components"))
        .expect("create nested project");
    std::fs::create_dir_all(temp.path().join(".github/workflows")).expect("create CI fixture");
    std::fs::write(temp.path().join("ruff.toml"), "line-length = 100\n")
        .expect("write native config");
    std::fs::write(temp.path().join(".github/workflows/ci.yml"), "name: ci\n")
        .expect("write CI config");
    std::fs::write(temp.path().join("AGENTS.md"), "# Existing instructions\n")
        .expect("write agent context");
    let nested = temp.path().join("apps/web");
    let before = git_status(temp.path());
    let native_before = [
        std::fs::read(temp.path().join("ruff.toml")).expect("read ruff config"),
        std::fs::read(temp.path().join(".github/workflows/ci.yml")).expect("read CI config"),
        std::fs::read(temp.path().join("AGENTS.md")).expect("read agent context"),
    ];

    for (request_id, action) in [("nested-inspect", None), ("nested-cancel", Some("cancel"))] {
        let mut args = vec![
            "init",
            "--json",
            "--project-dir",
            ".",
            "--request-id",
            request_id,
        ];
        if let Some(action) = action {
            args.extend(["--action", action]);
        }
        let output = run(&args, &nested);
        if action.is_some() {
            assert!(output.status.success());
            assert_eq!(json(&output)["data"]["cancelled"], true);
        } else {
            assert_eq!(output.status.code(), Some(6));
        }
        assert_eq!(git_status(temp.path()), before);
        assert!(!temp.path().join(".git/whetstone").exists());
        assert!(!nested.join(".git").exists());
    }

    let native_after = [
        std::fs::read(temp.path().join("ruff.toml")).expect("read ruff config"),
        std::fs::read(temp.path().join(".github/workflows/ci.yml")).expect("read CI config"),
        std::fs::read(temp.path().join("AGENTS.md")).expect("read agent context"),
    ];
    assert_eq!(native_after, native_before);
}

#[test]
fn stale_change_is_rejected_and_a_replay_returns_the_same_draft() {
    let temp = tempfile::tempdir().expect("create change fixture");
    init_git(temp.path());
    let project = temp.path().to_string_lossy();
    let probe = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--record-id",
            "principle.boundaries",
        ],
        temp.path(),
    );
    assert_eq!(probe.status.code(), Some(6));
    let handoff = json(&probe);
    assert_eq!(handoff["expected_revision"], 0);
    let token = handoff["resume_token"].as_str().expect("resume token");
    let change_args = |revision: &'static str| {
        vec![
            "change",
            "--json",
            "--project-dir",
            project.as_ref(),
            "--request-id",
            "change-stale",
            "--kind",
            "principle",
            "--record-id",
            "principle.boundaries",
            "--content",
            "Use explicit boundaries.",
            "--rationale",
            "Keep integration boundaries explicit.",
            "--source",
            "owner:change-stale",
            "--expected-effect",
            "Fewer accidental cross-boundary dependencies.",
            "--impact",
            "Local engineering principle.",
            "--example",
            "Use a typed adapter.",
            "--expected-revision",
            revision,
            "--resume",
            token,
        ]
    };

    let stale = run(&change_args("1"), temp.path());
    assert_eq!(stale.status.code(), Some(7));
    assert_eq!(json(&stale)["state"], "stale");

    let accepted = run(&change_args("0"), temp.path());
    assert!(accepted.status.success());
    let accepted_json = json(&accepted);
    assert_eq!(accepted_json["data"]["base_revision"], 0);
    assert_eq!(accepted_json["data"]["recorded"], true);
    assert_eq!(accepted_json["data"]["shared"], false);
    assert!(accepted_json["data"]["proposal"].is_object());
    assert_eq!(
        accepted_json["data"]["explanation"]["expected_effect"],
        "Fewer accidental cross-boundary dependencies."
    );
    assert_eq!(
        accepted_json["data"]["explanation"]["examples"],
        serde_json::json!(["Use a typed adapter."])
    );
    assert_eq!(
        accepted_json["data"]["diff"]["before"],
        serde_json::Value::Null
    );
    assert_eq!(
        accepted_json["data"]["diff"]["after"]["record_type"],
        "principle"
    );
    let replay = run(&change_args("0"), temp.path());
    assert!(replay.status.success());
    let replay_json = json(&replay);
    assert_eq!(replay_json["data"]["idempotent_replay"], true);
    assert_eq!(
        replay_json["data"]["record"],
        accepted_json["data"]["record"]
    );
    assert_eq!(
        replay_json["data"]["proposal"],
        accepted_json["data"]["proposal"]
    );

    // The same request id with different content is a conflict, not a new draft.
    let mut different = change_args("0");
    different[11] = "Use implicit boundaries.";
    let conflict = run(&different, temp.path());
    assert_eq!(conflict.status.code(), Some(8));
    assert_eq!(json(&conflict)["state"], "conflict");
}

#[test]
fn change_preview_shows_the_exact_draft_and_records_nothing() {
    let repo = Repo::new();
    let definition =
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"}}"#;
    let base = [
        "change",
        "--request-id",
        "preview-1",
        "--kind",
        "rule",
        "--record-id",
        "rule.previewed",
        "--content",
        "Run the tests.",
        "--rationale",
        "Prove it works.",
        "--definition",
        definition,
    ];
    let probe = repo.wh_json(&base);
    let token = probe["resume_token"].as_str().expect("token").to_string();
    let mut args = base.to_vec();
    args.extend(["--expected-revision", "0", "--resume", &token, "--preview"]);
    let preview = repo.wh(&args);
    assert_eq!(preview.status.code(), Some(5));
    let preview = json(&preview);
    assert_eq!(preview["state"], "needs_decision");
    assert_eq!(preview["data"]["preview_only"], true);
    assert_eq!(preview["data"]["diff"]["before"], serde_json::Value::Null);
    assert_eq!(
        preview["data"]["diff"]["after"]["record"]["enforcer"],
        serde_json::json!({"kind": "test", "command": "true"})
    );
    assert_eq!(preview["data"]["effects"]["team_share"], false);
    assert!(repo.records().is_empty(), "a preview recorded something");
}

#[test]
fn a_rule_draft_is_only_in_force_once_accepted_and_withdraw_and_retire_keep_history() {
    let repo = Repo::new();
    let definition =
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"}}"#;
    let drafted = repo.propose(
        "rule-1",
        "rule",
        "rule.boundary",
        "Do not import private modules.",
        Some(definition),
    );
    assert_eq!(drafted["state"], "success", "{drafted}");
    assert_eq!(drafted["data"]["recorded"], true);
    assert_eq!(drafted["data"]["diff"]["after"]["record_type"], "rule");
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal id")
        .to_string();

    // A draft is not in force until accepted.
    assert_eq!(repo.rule_entry("rule.boundary")["lifecycle"], "draft");

    let accepted = repo.accept("accept-1", &proposal);
    assert_eq!(accepted["state"], "success", "{accepted}");
    assert_eq!(accepted["data"]["verdict"], "accept");
    assert_eq!(accepted["data"]["shared"], false);
    let replayed = repo.accept("accept-1", &proposal);
    assert_eq!(replayed["data"]["idempotent_replay"], true);
    let again = repo.wh(&["change", "--request-id", "accept-2", "--accept", &proposal]);
    assert_eq!(again.status.code(), Some(6));
    assert_eq!(
        json(&again)["state"],
        "needs_input",
        "a reviewed draft is no longer pending"
    );
    assert_eq!(repo.rule_entry("rule.boundary")["lifecycle"], "accepted");

    // A withdrawn revision leaves the accepted one in force.
    let revised = repo.propose(
        "rule-2",
        "rule",
        "rule.boundary",
        "Do not import private modules, ever.",
        Some(r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"false"}}"#),
    );
    assert_eq!(revised["data"]["base_revision"], 1);
    assert!(revised["data"]["diff"]["before"].is_object());
    let revised_proposal = revised["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    let withdrawn = repo.wh_json(&[
        "change",
        "--request-id",
        "withdraw-1",
        "--withdraw",
        &revised_proposal,
    ]);
    assert_eq!(withdrawn["state"], "success");
    assert_eq!(withdrawn["data"]["verdict"], "withdraw");
    let still = repo.rule_entry("rule.boundary");
    assert_eq!(
        still["command"], "true",
        "the withdrawn revision took force: {still}"
    );

    // Retiring needs a reason, is a draft, and takes force only once accepted.
    let no_reason = repo.wh(&[
        "change",
        "--request-id",
        "retire-0",
        "--retire",
        "rule.boundary",
    ]);
    assert_eq!(no_reason.status.code(), Some(6));
    let retire = repo.wh_json(&[
        "change",
        "--request-id",
        "retire-1",
        "--retire",
        "rule.boundary",
        "--rationale",
        "The module boundary is gone.",
    ]);
    assert_eq!(retire["state"], "success", "{retire}");
    let retirement = retire["data"]["proposal"]["id"]
        .as_str()
        .expect("retirement proposal")
        .to_string();
    assert_eq!(repo.rule_entry("rule.boundary")["lifecycle"], "accepted");
    assert_eq!(
        repo.accept("accept-retire", &retirement)["state"],
        "success"
    );
    assert!(
        repo.rule_entries()
            .iter()
            .all(|rule| rule["id"] != "rule.boundary"),
        "a retired rule is no longer in force"
    );
    let history = repo
        .records()
        .into_iter()
        .filter(|record| record.id.as_str() == "rule.boundary")
        .count();
    assert_eq!(history, 2, "retiring deleted history");
    let nothing = repo.wh(&[
        "change",
        "--request-id",
        "retire-2",
        "--retire",
        "rule.boundary",
        "--rationale",
        "Again.",
    ]);
    assert_eq!(nothing.status.code(), Some(6));
}

#[test]
fn rule_definitions_hold_one_strength_and_one_enforcer() {
    let repo = Repo::new();
    let preview = |request_id: &str, definition: &str| {
        let base = [
            "change",
            "--request-id",
            request_id,
            "--kind",
            "rule",
            "--record-id",
            "rule.candidate",
            "--content",
            "A candidate rule.",
            "--rationale",
            "Checking the definition.",
            "--definition",
            definition,
        ];
        let probe = repo.wh(&base);
        let probe_json = json(&probe);
        let token = probe_json["resume_token"]
            .as_str()
            .expect("token")
            .to_string();
        let mut args = base.to_vec();
        args.extend(["--expected-revision", "0", "--resume", &token, "--preview"]);
        repo.wh(&args)
    };
    let enforcers = [
        r#"{"kind":"ast","query":"(identifier) @match","language":"python"}"#,
        r#"{"kind":"lint","tool":"ruff check .","code":"F401"}"#,
        r#"{"kind":"formatter","tool":"ruff format --check ."}"#,
        r#"{"kind":"test","command":"cargo test"}"#,
        r#"{"kind":"validator","command":"python3 scripts/check.py"}"#,
    ];
    for (index, strength) in ["must", "should"].into_iter().enumerate() {
        for (offset, enforcer) in enforcers.iter().enumerate() {
            let definition =
                format!(r#"{{"type":"rule","strength":"{strength}","enforcer":{enforcer}}}"#);
            let output = preview(&format!("family-{index}-{offset}"), &definition);
            assert_eq!(
                output.status.code(),
                Some(5),
                "{strength} {enforcer}: {}",
                json(&output)
            );
            let value = json(&output);
            let rule = &value["data"]["diff"]["after"]["record"];
            assert_eq!(rule["strength"], strength);
            let expected: serde_json::Value = serde_json::from_str(enforcer).expect("enforcer");
            assert_eq!(rule["enforcer"]["kind"], expected["kind"]);
        }
    }

    // Two enforcers, an unknown kind or strength, or stray fields never parse.
    for definition in [
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"},"enforcers":[{"kind":"lint","tool":"ruff","code":"F401"}]}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"},"enforcer":{"kind":"lint","tool":"ruff","code":"F401"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"review","reviewer":{"by":"interrogate"}}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"brief","skills":["how"]}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"question","question":"Is this a test?"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"public_surface"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"drive","feature":"feature.search"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true","query":"(identifier)"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"vibes"}}"#,
        r#"{"type":"rule","strength":"may","enforcer":{"kind":"test","command":"true"}}"#,
        r#"{"type":"rule","strength":"advisory","enforcer":{"kind":"test","command":"true"}}"#,
        r#"{"type":"rule","strength":"must"}"#,
        r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"test","command_ref":"cargo test"}}"#,
    ] {
        let output = run(
            &[
                "change",
                "--json",
                "--kind",
                "rule",
                "--record-id",
                "rule.bad",
                "--content",
                "Bad.",
                "--rationale",
                "Bad.",
                "--definition",
                definition,
            ],
            &repo.root,
        );
        assert_eq!(output.status.code(), Some(2), "{definition} parsed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid change definition"),
            "{definition}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Well-formed but invalid enforcers are asked about, never recorded.
    for (index, enforcer) in [
        r#"{"kind":"test","command":"cargo test | tee out"}"#,
        r#"{"kind":"lint","tool":"ruff","code":""}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let definition = format!(r#"{{"type":"rule","strength":"must","enforcer":{enforcer}}}"#);
        let output = preview(&format!("invalid-{index}"), &definition);
        assert_eq!(output.status.code(), Some(6), "{enforcer} was accepted");
        assert_eq!(json(&output)["state"], "needs_input");
    }
    assert!(
        repo.records().is_empty(),
        "a rejected definition was recorded"
    );

    // A rule without a definition is asked for its strength and enforcer.
    let missing = repo.propose("no-definition", "rule", "rule.vague", "Be good.", None);
    assert_eq!(missing["state"], "needs_input");
    assert!(missing["blocking_questions"][0]
        .as_str()
        .expect("question")
        .contains("strength"));
}

#[test]
fn check_is_observational_and_fails_closed_without_rules() {
    let temp = tempfile::tempdir().expect("create observational fixture");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("src")).expect("create source");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source");
    let before = tree_digest(temp.path());
    let project = temp.path().to_string_lossy();
    let output = run(&["check", "--json", "--project-dir", &project], temp.path());
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["state"], "unknown");
    assert_eq!(
        tree_digest(temp.path()),
        before,
        "check changed project or Git state"
    );
}

fn tree_digest(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let path = entry
                .path()
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            let bytes = std::fs::read(entry.path()).expect("read tree fixture");
            (path, bytes)
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[test]
fn a_check_writes_no_record() {
    let repo = Repo::new();
    repo.write("src/app.py", "def read_config():\n    pass\n");
    repo.commit("base");
    repo.rule("rule.lowercase-functions", UPPERCASE_FUNCTIONS);

    let args = ["--request-id", "check-idempotent"];
    let (_, first) = repo.check(&args);
    let (_, second) = repo.check(&args);
    assert_eq!(first["state"], "success", "{first}");
    assert_eq!(second["state"], "success", "{second}");
    assert_eq!(
        first["data"]["report"]["state"],
        second["data"]["report"]["state"]
    );
    let layout = ProjectLayout::resolve(&repo.root).expect("layout");
    assert!(
        !layout.private_store().exists(),
        "a check is its report, never a record"
    );
}

#[test]
fn command_response_schema_and_clients_cover_all_states() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let schema: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("references/command-response-v1.schema.json"))
            .expect("read response schema"),
    )
    .expect("parse response schema");
    assert_eq!(
        schema["properties"]["schema"]["const"],
        "whetstone.command-response.v1"
    );
    assert_eq!(schema["additionalProperties"], false);
    let states = schema["properties"]["state"]["enum"]
        .as_array()
        .expect("state enum")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "success",
        "violated",
        "unknown",
        "unavailable",
        "needs_decision",
        "needs_input",
        "stale",
        "conflict",
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(states, expected);
    for client in [
        "examples/whetstone-client.sh",
        "examples/whetstone_client.py",
    ] {
        let text = std::fs::read_to_string(root.join(client)).expect("read example client");
        for state in &expected {
            assert!(text.contains(state), "{client} does not handle {state}");
        }
    }
    assert!(Command::new("sh")
        .args(["-n", "examples/whetstone-client.sh"])
        .current_dir(root)
        .status()
        .expect("syntax-check shell client")
        .success());
}

const UPPERCASE_FUNCTIONS: &str = r#"{"type":"rule","strength":"must","enforcer":{"kind":"ast","language":"python","query":"((function_definition name: (identifier) @match) (#match? @match \"^[A-Z]\"))"}}"#;

#[test]
fn staged_check_reads_the_index_and_ignores_unstaged_edits() {
    let repo = Repo::new();
    repo.write("src/app.py", "def read_config():\n    pass\n");
    repo.commit("base");
    repo.rule("rule.lowercase-functions", UPPERCASE_FUNCTIONS);

    // Stage a good edit, then leave a bad edit unstaged on top of it.
    repo.write("src/app.py", "def read_config():\n    return 1\n");
    git(&repo.root, &["add", "src/app.py"]);
    repo.write("src/app.py", "def ReadConfig():\n    return 1\n");

    let (code, staged) = repo.check(&["--staged"]);
    assert_eq!(code, Some(0), "{staged}");
    assert_eq!(staged["state"], "success");
    assert_eq!(staged["data"]["selection"]["mode"], "staged");
    assert_eq!(
        staged["data"]["selection"]["changed_paths"],
        serde_json::json!(["src/app.py"])
    );
    assert_eq!(gate(&staged, "rule.lowercase-functions")["state"], "pass");

    // The working tree has the violation, and a full check says so.
    let (code, working) = repo.check(&["--rule", "rule.lowercase-functions"]);
    assert_eq!(code, Some(1), "{working}");
    assert_eq!(working["state"], "violated");
    let failing = gate(&working, "rule.lowercase-functions");
    assert_eq!(failing["state"], "fail");
    assert!(failing["failures"][0]["location"]
        .as_str()
        .expect("location")
        .starts_with("src/app.py"));

    // Once staged, pre-commit sees it too.
    git(&repo.root, &["add", "src/app.py"]);
    let (code, staged_bad) = repo.check(&["--staged"]);
    assert_eq!(code, Some(1), "{staged_bad}");
    assert_eq!(
        gate(&staged_bad, "rule.lowercase-functions")["state"],
        "fail"
    );
}

#[test]
fn a_should_rule_failure_is_a_flag_and_does_not_fail_the_check() {
    let repo = Repo::new();
    repo.write("README.md", "fixture\n");
    repo.commit("base");
    repo.rule(
        "rule.must-pass",
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"}}"#,
    );
    repo.rule(
        "rule.should-fail",
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"test","command":"false"}}"#,
    );
    let (code, checked) = repo.check(&[]);
    assert_eq!(code, Some(0), "{checked}");
    assert_eq!(checked["state"], "success");
    assert_eq!(
        checked["data"]["flags"],
        serde_json::json!(["rule.should-fail"])
    );
    assert_eq!(gate(&checked, "rule.must-pass")["state"], "pass");
    let should = gate(&checked, "rule.should-fail");
    assert_eq!(should["state"], "fail");
    assert_eq!(should["strength"], "should");
    assert!(checked["summary"]
        .as_str()
        .expect("summary")
        .contains("rule.should-fail"));

    // The same must rule failing does fail the check.
    repo.rule(
        "rule.must-fail",
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"false"}}"#,
    );
    let (code, failed) = repo.check(&[]);
    assert_eq!(code, Some(1), "{failed}");
    assert_eq!(failed["state"], "violated");
    assert_eq!(
        failed["data"]["flags"],
        serde_json::json!(["rule.should-fail"])
    );
}

#[test]
fn a_check_of_only_should_rules_lists_the_flag() {
    let repo = Repo::new();
    repo.rule(
        "rule.should-fail",
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"test","command":"false"}}"#,
    );
    let (code, checked) = repo.check(&["--rule", "rule.should-fail"]);
    assert_eq!(code, Some(0), "{checked}");
    assert_eq!(
        checked["data"]["flags"],
        serde_json::json!(["rule.should-fail"])
    );
}
