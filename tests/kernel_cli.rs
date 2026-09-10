use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use whetstone::domain::{AuthorizationAxis, RecordBody, RecordId, VerificationAxis};
use whetstone::storage::{DoltRepository, ProjectLayout, StoreKind};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
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

fn write_rule_project(root: &Path, source: &str) {
    std::fs::create_dir_all(root.join("whetstone/rules/python")).expect("create rule fixture");
    std::fs::create_dir_all(root.join("src")).expect("create source fixture");
    std::fs::write(
        root.join("whetstone/rules/python/names.yaml"),
        r#"source:
  name: team
rules:
  - id: team.lowercase-functions
    severity: must
    confidence: high
    category: convention
    description: Function names must begin with a lowercase character.
    source_url: https://example.com/team/functions
    approved: true
    status: approved
    signals:
      - id: uppercase-function
        strategy: ast
        description: Finds uppercase function names.
        weight: required
        ast_query: '((function_definition name: (identifier) @match) (#match? @match "^[A-Z]"))'
    golden_examples:
      - code: "def read_config():\n    pass\n"
        verdict: pass
        reason: Lowercase names comply with the rule.
      - code: "def ReadConfig():\n    pass\n"
        verdict: fail
        reason: Uppercase names violate the rule.
"#,
    )
    .expect("write rule fixture");
    std::fs::write(root.join("src/app.py"), source).expect("write source fixture");
}

fn init_git(root: &Path) {
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .expect("initialize isolated git fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_status(root: &Path) -> String {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(root)
        .output()
        .expect("inspect fixture status");
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("utf-8 git status")
}

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
    assert_eq!(
        value["data"]["lean_baseline_revision"],
        "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48"
    );
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
    for command in ["publish", "status", "debt", "report", "mcp", "actions"] {
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for workflow in ["pull", "push"] {
        let output = run(&[workflow, "--json", "--request-id", "probe-1"], root);
        assert_eq!(output.status.code(), Some(4));
        let value = json(&output);
        assert_eq!(value["schema"], "whetstone.command-response.v1");
        assert_eq!(value["state"], "unavailable");
        assert_eq!(value["request_id"], "probe-1");
    }
    let output = run(&["dash", "--json", "--request-id", "probe-1"], root);
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output);
    assert_eq!(value["state"], "success");
    assert_eq!(value["data"]["read_only"], true);
}

#[test]
fn init_is_resumable_and_duplicate_requests_are_idempotent() {
    let temp = tempfile::tempdir().expect("create init fixture");
    init_git(temp.path());
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
    assert_eq!(handoff["data"]["progress"]["private_store"], "proposed");
    assert!(!temp.path().join(".git/whetstone").exists());
    assert!(git_status(temp.path()).is_empty());
    let token = handoff["resume_token"].as_str().expect("resume token");
    let revision = handoff["expected_revision"].as_u64().expect("revision");

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
    let agreement_args = [
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
        "--mission",
        "Keep project work aligned.",
        "--desired-outcome",
        "Reduce avoidable rework.",
        "--values",
        "Safety before speed.",
        "--philosophy",
        "Prefer small deterministic boundaries.",
        "--owner",
        "Platform lead",
        "--initial-safeguard",
        "Never weaken a failing check.",
        "--safeguard-scope",
        "All repository changes",
        "--revision-triggers",
        "Mission, architecture, or repeated-friction changes",
    ];
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
    assert_eq!(
        accepted_json["data"]["records"].as_array().map(Vec::len),
        Some(4)
    );
    assert_eq!(accepted_json["data"]["progress"]["agreement"], "approved");
    assert_eq!(
        accepted_json["data"]["progress"]["private_store"],
        "installed"
    );
    assert_eq!(
        accepted_json["data"]["progress"]["feedback_loop"],
        "proposed"
    );
    let layout = ProjectLayout::resolve(temp.path(), None).expect("project layout");
    assert!(!layout.store_path(StoreKind::Shareable).exists());
    assert!(git_status(temp.path()).is_empty());

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
        established_json["data"]["progress"]["agreement"],
        "approved"
    );
    assert_eq!(
        established_json["data"]["progress"]["private_store"],
        "installed"
    );
    assert_eq!(
        established_json["data"]["progress"]["feedback_loop"],
        "proposed"
    );
    assert_eq!(
        established_json["data"]["inspection_writes"],
        serde_json::json!([])
    );

    let replay = run(&agreement_args, temp.path());
    assert_eq!(replay.status.code(), Some(6));
    assert_eq!(
        json(&replay)["data"]["records"],
        accepted_json["data"]["records"]
    );

    let conflict = run(
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
            &revision_text,
            "--resume",
            token,
            "--mission",
            "Different mission.",
            "--desired-outcome",
            "Reduce avoidable rework.",
            "--values",
            "Safety before speed.",
            "--philosophy",
            "Prefer small deterministic boundaries.",
            "--owner",
            "Platform lead",
            "--initial-safeguard",
            "Never weaken a failing check.",
            "--safeguard-scope",
            "All repository changes",
            "--revision-triggers",
            "Mission, architecture, or repeated-friction changes",
        ],
        temp.path(),
    );
    assert_eq!(conflict.status.code(), Some(8));
    assert_eq!(json(&conflict)["state"], "conflict");
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
        let response = json(&output);
        assert_eq!(
            response["data"]["setup"]["project_root"],
            temp.path()
                .canonicalize()
                .expect("root")
                .to_string_lossy()
                .as_ref()
        );
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
fn init_requires_every_owner_decision_and_never_accepts_proposed_defaults() {
    let temp = tempfile::tempdir().expect("create required-input fixture");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("whetstone/packs/team"))
        .expect("create inert import fixture");
    std::fs::write(
        temp.path().join("whetstone/packs/team/install.sh"),
        "exit 99\n",
    )
    .expect("write inert imported content");
    let project = temp.path().to_string_lossy();
    let inspect = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "explicit-input",
        ],
        temp.path(),
    );
    let handoff = json(&inspect);
    assert_eq!(handoff["state"], "needs_input");
    assert_eq!(
        handoff["data"]["setup"]["proposed_defaults"]
            .as_array()
            .map(Vec::len),
        Some(5)
    );
    assert!(handoff["data"]["setup"]["imported_material_trust"]
        .as_str()
        .expect("trust statement")
        .contains("remains inert"));
    assert_eq!(
        handoff["data"]["setup"]["executable_access"][0],
        "none until a checker manifest is explicitly trusted"
    );
    assert!(!temp.path().join(".git/whetstone").exists());

    let revision = handoff["expected_revision"].as_u64().expect("revision");
    let revision_text = revision.to_string();
    let token = handoff["resume_token"].as_str().expect("resume token");
    let incomplete = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--action",
            "agree",
            "--request-id",
            "explicit-input",
            "--expected-revision",
            &revision_text,
            "--resume",
            token,
            "--mission",
            "Keep project work aligned.",
            "--values",
            "Safety before speed.",
            "--philosophy",
            "Prefer small deterministic boundaries.",
        ],
        temp.path(),
    );
    assert_eq!(incomplete.status.code(), Some(6));
    assert_eq!(json(&incomplete)["state"], "needs_input");
    let layout = ProjectLayout::resolve(temp.path(), None).expect("project layout");
    let repository =
        DoltRepository::open_existing(&layout.store_path(StoreKind::Private), StoreKind::Private)
            .expect("private store exists only after explicit agreement attempt");
    assert!(repository.all_records().expect("records").is_empty());
    assert!(!layout.store_path(StoreKind::Shareable).exists());
}

#[test]
fn init_repairs_only_missing_established_agreement_fields_and_replays_exactly() {
    let temp = tempfile::tempdir().expect("create established agreement fixture");
    init_git(temp.path());
    let project = temp.path().to_string_lossy();
    let inspect = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "initial-agreement",
        ],
        temp.path(),
    );
    let handoff = json(&inspect);
    let revision = handoff["expected_revision"].as_u64().expect("revision");
    let revision_text = revision.to_string();
    let token = handoff["resume_token"].as_str().expect("token");
    let accepted = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--action",
            "agree",
            "--request-id",
            "initial-agreement",
            "--expected-revision",
            &revision_text,
            "--resume",
            token,
            "--mission",
            "Keep project work aligned.",
            "--desired-outcome",
            "Reduce avoidable rework.",
            "--values",
            "Safety before speed.",
            "--philosophy",
            "Prefer small deterministic boundaries.",
            "--owner",
            "Platform lead",
            "--initial-safeguard",
            "Never weaken a failing check.",
            "--safeguard-scope",
            "All repository changes",
            "--revision-triggers",
            "Mission or architecture changes",
        ],
        temp.path(),
    );
    assert_eq!(accepted.status.code(), Some(6));

    let layout = ProjectLayout::resolve(temp.path(), None).expect("layout");
    let private =
        DoltRepository::open_existing(&layout.store_path(StoreKind::Private), StoreKind::Private)
            .expect("private store");
    let mission_id = RecordId::new("mission.project").expect("mission ID");
    let mut incomplete = private
        .latest(&mission_id)
        .expect("mission query")
        .expect("mission");
    incomplete.supersedes = Some(incomplete.reference().expect("mission ref"));
    incomplete.revision += 1;
    incomplete.idempotency_key = "legacy-incomplete-mission".into();
    incomplete.owner.display_name = None;
    let RecordBody::Mission(mission) = &mut incomplete.body else {
        panic!("mission body")
    };
    mission.desired_outcomes.clear();
    private
        .append(&incomplete, Some(1))
        .expect("model an established incomplete agreement");

    let resume = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "complete-established",
        ],
        temp.path(),
    );
    assert_eq!(resume.status.code(), Some(6));
    let resume_json = json(&resume);
    assert_eq!(
        resume_json["data"]["progress"]["missing_decisions"],
        serde_json::json!(["accountable owner", "desired outcome"])
    );
    let resume_revision = resume_json["expected_revision"]
        .as_u64()
        .expect("resume revision")
        .to_string();
    let resume_token = resume_json["resume_token"].as_str().expect("resume token");
    let completion_args = [
        "init",
        "--json",
        "--project-dir",
        &project,
        "--action",
        "agree",
        "--request-id",
        "complete-established",
        "--expected-revision",
        &resume_revision,
        "--resume",
        resume_token,
        "--desired-outcome",
        "Reduce avoidable rework.",
        "--owner",
        "Platform lead",
    ];
    let completed = run(&completion_args, temp.path());
    assert_eq!(completed.status.code(), Some(6));
    let completed_json = json(&completed);
    assert_eq!(
        completed_json["data"]["progress"]["missing_decisions"],
        serde_json::json!([])
    );
    assert_eq!(
        completed_json["data"]["records"].as_array().map(Vec::len),
        Some(1)
    );
    let replay = run(&completion_args, temp.path());
    assert_eq!(replay.status.code(), Some(6));
    assert_eq!(
        json(&replay)["data"]["records"],
        completed_json["data"]["records"]
    );
    assert_eq!(
        private
            .all_records()
            .expect("records")
            .into_iter()
            .filter(|record| record.id == mission_id)
            .count(),
        3
    );
}

#[test]
fn init_agree_resumes_after_interruption_between_dolt_bootstrap_and_migration() {
    let temp = tempfile::tempdir().expect("create interrupted bootstrap fixture");
    init_git(temp.path());
    let project = temp.path().to_string_lossy();
    let inspect = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "resume-bootstrap",
        ],
        temp.path(),
    );
    let handoff = json(&inspect);
    let revision = handoff["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = handoff["resume_token"].as_str().expect("token");
    let layout = ProjectLayout::resolve(temp.path(), None).expect("layout");
    let private_root = layout.store_path(StoreKind::Private);
    std::fs::create_dir_all(&private_root).expect("create private root");
    let interrupted = Command::new("dolt")
        .current_dir(&private_root)
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .args([
            "init",
            "--name",
            "Whetstone",
            "--email",
            "local@whetstone.invalid",
            "--initial-branch",
            "main",
        ])
        .output()
        .expect("model process stop after dolt init");
    assert!(
        interrupted.status.success(),
        "{}",
        String::from_utf8_lossy(&interrupted.stderr)
    );
    let cancelled = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--action",
            "cancel",
            "--request-id",
            "cancel-interrupted-bootstrap",
        ],
        temp.path(),
    );
    assert!(cancelled.status.success());
    assert_eq!(json(&cancelled)["data"]["progress_state"], "unavailable");
    let tables = Command::new("dolt")
        .current_dir(&private_root)
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .args(["sql", "-r", "json", "-q", "SHOW TABLES"])
        .output()
        .expect("inspect interrupted schema after cancel");
    assert!(tables.status.success());
    let table_json =
        serde_json::from_slice::<serde_json::Value>(&tables.stdout).expect("table JSON");
    assert!(
        table_json
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
            || table_json
                .get("rows")
                .and_then(serde_json::Value::as_array)
                .is_some_and(Vec::is_empty)
    );
    let resumed = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--action",
            "agree",
            "--request-id",
            "resume-bootstrap",
            "--expected-revision",
            &revision,
            "--resume",
            token,
            "--mission",
            "Keep project work aligned.",
            "--desired-outcome",
            "Reduce avoidable rework.",
            "--values",
            "Safety before speed.",
            "--philosophy",
            "Prefer small deterministic boundaries.",
            "--owner",
            "Platform lead",
            "--initial-safeguard",
            "Never weaken a failing check.",
            "--safeguard-scope",
            "All repository changes",
            "--revision-triggers",
            "Mission or architecture changes",
        ],
        temp.path(),
    );
    assert_eq!(resumed.status.code(), Some(6));
    assert_eq!(json(&resumed)["data"]["progress"]["agreement"], "approved");
    assert_eq!(
        DoltRepository::open_existing(&private_root, StoreKind::Private)
            .expect("recovered store")
            .all_records()
            .expect("records")
            .len(),
        4
    );
}

#[test]
fn init_replay_keeps_the_base_revision_when_new_records_have_no_supersedes() {
    let temp = tempfile::tempdir().expect("create mixed established fixture");
    init_git(temp.path());
    let project = temp.path().to_string_lossy();
    let probe = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "existing-value",
            "--record-id",
            "value.core",
        ],
        temp.path(),
    );
    let proposal = json(&probe);
    let change_token = proposal["resume_token"].as_str().expect("change token");
    let changed = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "existing-value",
            "--kind",
            "value",
            "--record-id",
            "value.core",
            "--content",
            "Safety before speed.",
            "--rationale",
            "Existing accepted value.",
            "--source",
            "owner:existing-value",
            "--expected-effect",
            "Reduce avoidable rework.",
            "--impact",
            "Project-wide engineering work.",
            "--expected-revision",
            "0",
            "--resume",
            change_token,
        ],
        temp.path(),
    );
    assert!(changed.status.success());

    let inspect = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "fill-around-value",
        ],
        temp.path(),
    );
    let handoff = json(&inspect);
    assert_eq!(handoff["expected_revision"], 1);
    let token = handoff["resume_token"].as_str().expect("init token");
    let agreement_args = [
        "init",
        "--json",
        "--project-dir",
        &project,
        "--action",
        "agree",
        "--request-id",
        "fill-around-value",
        "--expected-revision",
        "1",
        "--resume",
        token,
        "--mission",
        "Keep project work aligned.",
        "--desired-outcome",
        "Reduce avoidable rework.",
        "--philosophy",
        "Prefer small deterministic boundaries.",
        "--owner",
        "Platform lead",
        "--initial-safeguard",
        "Never weaken a failing check.",
        "--safeguard-scope",
        "All repository changes",
        "--revision-triggers",
        "Mission or architecture changes",
    ];
    let accepted = run(&agreement_args, temp.path());
    assert_eq!(accepted.status.code(), Some(6));
    assert_eq!(
        json(&accepted)["data"]["records"].as_array().map(Vec::len),
        Some(3)
    );
    let replay = run(&agreement_args, temp.path());
    assert_eq!(replay.status.code(), Some(6));
    assert_eq!(
        json(&replay)["data"]["records"],
        json(&accepted)["data"]["records"]
    );
}

#[test]
fn stale_change_is_rejected_and_standard_needs_a_decision() {
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
            "guidance.local",
        ],
        temp.path(),
    );
    assert_eq!(probe.status.code(), Some(6));
    let handoff = json(&probe);
    let token = handoff["resume_token"].as_str().expect("resume token");

    let stale = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--kind",
            "guidance",
            "--record-id",
            "guidance.local",
            "--content",
            "Use explicit boundaries.",
            "--rationale",
            "Keep integration boundaries explicit.",
            "--source",
            "owner:change-stale",
            "--expected-effect",
            "Fewer accidental cross-boundary dependencies.",
            "--impact",
            "Local engineering guidance.",
            "--example",
            "Use a typed adapter.",
            "--expected-revision",
            "1",
            "--resume",
            token,
        ],
        temp.path(),
    );
    assert_eq!(stale.status.code(), Some(7));
    assert_eq!(json(&stale)["state"], "stale");

    let accepted = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--kind",
            "guidance",
            "--record-id",
            "guidance.local",
            "--content",
            "Use explicit boundaries.",
            "--rationale",
            "Keep integration boundaries explicit.",
            "--source",
            "owner:change-stale",
            "--expected-effect",
            "Fewer accidental cross-boundary dependencies.",
            "--impact",
            "Local engineering guidance.",
            "--example",
            "Use a typed adapter.",
            "--expected-revision",
            "0",
            "--resume",
            token,
        ],
        temp.path(),
    );
    assert!(accepted.status.success());
    let accepted_json = json(&accepted);
    assert_eq!(accepted_json["data"]["base_revision"], 0);
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
    let replay = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--kind",
            "guidance",
            "--record-id",
            "guidance.local",
            "--content",
            "Use explicit boundaries.",
            "--rationale",
            "Keep integration boundaries explicit.",
            "--source",
            "owner:change-stale",
            "--expected-effect",
            "Fewer accidental cross-boundary dependencies.",
            "--impact",
            "Local engineering guidance.",
            "--example",
            "Use a typed adapter.",
            "--expected-revision",
            "0",
            "--resume",
            token,
        ],
        temp.path(),
    );
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

    let decision_probe = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "standard-1",
            "--record-id",
            "standard.boundary",
        ],
        temp.path(),
    );
    let decision_handoff = json(&decision_probe);
    let decision_token = decision_handoff["resume_token"]
        .as_str()
        .expect("decision token");
    let decision = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "standard-1",
            "--kind",
            "standard",
            "--record-id",
            "standard.boundary",
            "--content",
            "Do not import private modules.",
            "--definition",
            r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"test","command_ref":"cargo test"}}"#,
            "--rationale",
            "Protect module boundaries.",
            "--source",
            "architecture:module-map",
            "--expected-effect",
            "Invalid imports are rejected.",
            "--impact",
            "Requires a trusted deterministic checker.",
            "--example",
            "Public modules may not import internal modules.",
            "--expected-revision",
            "0",
            "--resume",
            decision_token,
        ],
        temp.path(),
    );
    assert_eq!(decision.status.code(), Some(0));
    let decision_json = json(&decision);
    assert_eq!(decision_json["state"], "success");
    assert_eq!(decision_json["data"]["recorded"], true);
    assert_eq!(decision_json["data"]["base_revision"], 0);
    assert_eq!(
        decision_json["data"]["diff"]["after"]["record_type"],
        "standard"
    );
    // A recorded standard is a private draft: it is not in force, so a check
    // never runs it until the owner explicitly accepts it.
    let proposal = decision_json["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal id")
        .to_string();
    let draft_check = run(
        &[
            "check",
            "--json",
            "--project-dir",
            &project,
            "--rule",
            "standard.boundary",
        ],
        temp.path(),
    );
    assert_eq!(
        json(&draft_check)["data"]["selection"]["gates"],
        serde_json::json!([])
    );
    assert_eq!(
        json(&draft_check)["data"]["selection"]["skipped_drafts"],
        serde_json::json!(["standard.boundary"])
    );
    let accepted = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "accept-1",
            "--accept",
            &proposal,
            "--rationale",
            "Owner accepts the boundary gate.",
        ],
        temp.path(),
    );
    assert_eq!(
        json(&accepted)["state"],
        "success",
        "{}",
        String::from_utf8_lossy(&accepted.stdout)
    );
    assert_eq!(json(&accepted)["data"]["team_activation"], false);
    let replayed = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "accept-2",
            "--accept",
            &proposal,
        ],
        temp.path(),
    );
    assert_eq!(
        json(&replayed)["state"],
        "needs_input",
        "a reviewed draft is no longer pending"
    );
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
    let output = run(
        &[
            "check",
            "--json",
            "--project-dir",
            &project,
            "--path",
            "src",
        ],
        temp.path(),
    );
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
fn validate_eval_and_self_scan_are_non_vacuous() {
    let temp = tempfile::tempdir().expect("create gate fixture");
    write_rule_project(temp.path(), "def ReadConfig():\n    pass\n");
    let root = temp.path();
    let root_arg = root.to_string_lossy();

    let validate = run(&["validate", "--project-dir", &root_arg, "--json"], root);
    assert!(validate.status.success());
    let validate_json = json(&validate);
    assert_eq!(validate_json["ok"], true);
    let report = validate_json["report"]
        .as_str()
        .expect("validation report should be text");
    assert!(report.contains("Checking "), "{validate_json}");
    assert!(!report.contains("Checking 0 rule files"), "{validate_json}");

    let eval = run(&["eval", "--project-dir", &root_arg, "--json"], root);
    assert!(eval.status.success());
    let eval_json = json(&eval);
    assert_eq!(eval_json["ok"], true);
    assert!(
        eval_json["rules_evaluated"]
            .as_u64()
            .expect("rules_evaluated should be numeric")
            > 0
    );
    let checked: u64 = eval_json["scorecards"]
        .as_array()
        .expect("scorecards should be an array")
        .iter()
        .filter_map(|card| card["golden_checked"].as_u64())
        .sum();
    assert!(
        checked > 0,
        "eval performed no scanner-backed golden checks"
    );

    let scan = run(
        &[
            "scan",
            "src",
            "--project-dir",
            &root_arg,
            "--lang",
            "python",
            "--json",
            "--no-fail",
        ],
        root,
    );
    assert!(scan.status.success());
    let scan_json = json(&scan);
    assert_eq!(scan_json["violations_count"], 1);
    assert!(
        scan_json["files_scanned"]
            .as_u64()
            .expect("files_scanned should be numeric")
            > 0
    );
    assert!(
        scan_json["rules_applied"]
            .as_u64()
            .expect("rules_applied should be numeric")
            > 0
    );
}

#[test]
fn scanner_finds_known_bad_and_accepts_known_good() {
    let temp = tempfile::tempdir().expect("create scanner fixture");
    write_rule_project(temp.path(), "def ReadConfig():\n    pass\n");
    let project = temp.path().to_string_lossy();
    let bad = run(
        &[
            "scan",
            "src",
            "--project-dir",
            &project,
            "--lang",
            "python",
            "--json",
            "--no-fail",
        ],
        temp.path(),
    );
    assert!(bad.status.success());
    let bad_json = json(&bad);
    assert_eq!(bad_json["violations_count"], 1);
    assert_eq!(bad_json["files_scanned"], 1);
    assert_eq!(bad_json["rules_applied"], 1);

    let public_bad = run(
        &[
            "check",
            "--project-dir",
            &project,
            "--path",
            "src",
            "--lang",
            "python",
            "--json",
        ],
        temp.path(),
    );
    assert_eq!(public_bad.status.code(), Some(1));
    let public_bad_json = json(&public_bad);
    assert_eq!(public_bad_json["state"], "violated");
    assert_eq!(public_bad_json["data"]["report"]["state"], "violated");
    assert_eq!(
        public_bad_json["data"]["report"]["results"][0]["findings"][0]["file"],
        "src/app.py"
    );
    for field in [
        "rationale",
        "observed",
        "expected",
        "repair_direction",
        "permitted_next_action",
        "verification_command",
    ] {
        assert!(
            public_bad_json["data"]["report"]["results"][0]["findings"][0][field]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }
    assert_eq!(public_bad_json["data"]["receipt_persisted"], false);
    let bad_code_digest = public_bad_json["required_snapshot"]["code_digest"]
        .as_str()
        .expect("bad code digest")
        .to_string();

    std::fs::write(
        temp.path().join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("update source fixture");
    let good = run(
        &[
            "scan",
            "src",
            "--project-dir",
            &project,
            "--lang",
            "python",
            "--json",
            "--no-fail",
        ],
        temp.path(),
    );
    assert!(good.status.success());
    assert_eq!(json(&good)["violations_count"], 0);

    let public_good = run(
        &[
            "check",
            "--project-dir",
            &project,
            "--path",
            "src",
            "--lang",
            "python",
            "--json",
        ],
        temp.path(),
    );
    assert!(public_good.status.success());
    let public_good_json = json(&public_good);
    assert_eq!(public_good_json["state"], "success");
    assert_eq!(public_good_json["data"]["report"]["state"], "success");
    assert!(public_good_json["summary"].as_str().is_some_and(|summary| {
        summary.contains("whetstone.native-scan") && summary.contains("PASS")
    }));
    for field in [
        "code_digest",
        "policy_digest",
        "checker_digest",
        "scope_digest",
        "environment_digest",
        "trust_digest",
    ] {
        assert!(public_good_json["required_snapshot"][field]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:")));
    }
    assert_ne!(
        public_good_json["required_snapshot"]["code_digest"],
        bad_code_digest
    );
}

#[test]
fn check_persists_one_idempotent_private_receipt_when_storage_exists() {
    let temp = tempfile::tempdir().expect("create receipt fixture");
    init_git(temp.path());
    write_rule_project(temp.path(), "def read_config():\n    pass\n");
    let project = temp.path().to_string_lossy();
    let layout = ProjectLayout::resolve(temp.path(), None).expect("resolve receipt fixture");
    DoltRepository::initialize(&layout.store_path(StoreKind::Private), StoreKind::Private)
        .expect("initialize receipt store explicitly");

    let args = [
        "check",
        "--json",
        "--project-dir",
        &project,
        "--request-id",
        "check-idempotent",
        "--path",
        "src",
        "--lang",
        "python",
    ];
    let first = json(&run(&args, temp.path()));
    let second = json(&run(&args, temp.path()));
    assert_eq!(first["state"], "success");
    assert_eq!(first["data"]["receipt_persisted"], true);
    assert_eq!(
        first["data"]["receipt_record"],
        second["data"]["receipt_record"]
    );

    let private =
        DoltRepository::initialize(&layout.store_path(StoreKind::Private), StoreKind::Private)
            .expect("open private store");
    let receipts = private
        .all_records()
        .expect("read private records")
        .into_iter()
        .filter_map(|record| match record.body {
            RecordBody::VerificationReceipt(receipt) => Some(receipt),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].verification, VerificationAxis::Pass);
    assert_eq!(receipts[0].authorization, AuthorizationAxis::Unknown);
    assert_eq!(receipts[0].evidence.len(), 1);
}

#[test]
fn public_check_does_not_execute_untrusted_command_validators() {
    let temp = tempfile::tempdir().expect("create validator fixture");
    init_git(temp.path());
    let rules = temp.path().join("whetstone/rules/python");
    std::fs::create_dir_all(&rules).expect("create rules");
    std::fs::create_dir_all(temp.path().join("src")).expect("create source");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source");
    std::fs::write(
        rules.join("command.yaml"),
        r#"source:
  name: team
rules:
  - id: team.untrusted-command
    severity: must
    confidence: high
    category: convention
    description: An untrusted command must never run at the public boundary.
    source_url: https://example.com/command
    approved: true
    status: approved
    validators:
      - adapter: command
        rule: command-check
        config:
          command: "touch should-not-exist; printf '{\"violations\":[]}'"
          allow_shell: true
    golden_examples: []
"#,
    )
    .expect("write validator rule");
    let project = temp.path().to_string_lossy();
    let output = run(
        &[
            "check",
            "--json",
            "--project-dir",
            &project,
            "--path",
            "src",
            "--lang",
            "python",
        ],
        temp.path(),
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["state"], "unknown");
    assert!(!temp.path().join("should-not-exist").exists());
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

fn scan_json(root: &Path, no_fail: bool) -> Output {
    let project = root.to_string_lossy();
    let mut args = vec![
        "scan",
        "src",
        "--project-dir",
        &project,
        "--lang",
        "python",
        "--json",
    ];
    if no_fail {
        args.push("--no-fail");
    }
    run(&args, root)
}

fn complete_rule(id: &str, status: &str, approved: bool, query: Option<&str>) -> String {
    let query = query
        .map(|query| format!("        ast_query: '{query}'\n"))
        .unwrap_or_default();
    format!(
        "source:\n  name: team\nrules:\n  - id: {id}\n    severity: must\n    confidence: high\n    category: convention\n    description: A deterministic test rule.\n    source_url: https://example.com/rule\n    approved: {approved}\n    status: {status}\n    signals:\n      - id: signal\n        strategy: ast\n        weight: required\n{query}    golden_examples:\n      - code: 'def good(): pass'\n        verdict: pass\n        reason: Expected pass.\n      - code: 'def Bad(): pass'\n        verdict: fail\n        reason: Expected failure.\n"
    )
}

#[test]
fn malformed_rule_inputs_are_configuration_failures() {
    let temp = tempfile::tempdir().expect("create malformed-rule fixture");
    let rules = temp.path().join("whetstone/rules/python");
    std::fs::create_dir_all(&rules).expect("create rule directory");
    std::fs::create_dir_all(temp.path().join("src")).expect("create source directory");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source fixture");
    std::fs::write(
        rules.join("valid.yaml"),
        complete_rule(
            "team.valid",
            "approved",
            true,
            Some("((function_definition name: (identifier) @match) (#match? @match \"^[A-Z]\"))"),
        ),
    )
    .expect("write valid rule");

    std::fs::write(rules.join("broken.yaml"), "source: [\nrules: nope")
        .expect("write malformed YAML");
    let malformed_yaml = scan_json(temp.path(), false);
    assert_eq!(malformed_yaml.status.code(), Some(1));
    let malformed_yaml_json = json(&malformed_yaml);
    assert_eq!(malformed_yaml_json["status"], "config_issues_found");
    assert!(
        malformed_yaml_json["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );

    std::fs::write(
        rules.join("broken.yaml"),
        complete_rule(
            "team.bad-query",
            "approved",
            true,
            Some("(function_definition"),
        ),
    )
    .expect("write malformed query");
    let malformed_query = scan_json(temp.path(), false);
    assert_eq!(malformed_query.status.code(), Some(1));
    assert!(
        json(&malformed_query)["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );
    let project = temp.path().to_string_lossy();
    let malformed_validate = run(
        &["validate", "--project-dir", &project, "--json"],
        temp.path(),
    );
    assert_eq!(malformed_validate.status.code(), Some(1));
    assert_eq!(json(&malformed_validate)["ok"], false);

    std::fs::write(
        rules.join("broken.yaml"),
        complete_rule("team.missing-query", "approved", true, None),
    )
    .expect("write missing query");
    let missing_query = scan_json(temp.path(), false);
    assert_eq!(missing_query.status.code(), Some(1));
    assert!(
        json(&missing_query)["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );
}

#[test]
fn only_consistent_approved_rules_are_loaded() {
    let temp = tempfile::tempdir().expect("create lifecycle fixture");
    let rules = temp.path().join("whetstone/rules/python");
    std::fs::create_dir_all(&rules).expect("create rule directory");
    std::fs::create_dir_all(temp.path().join("src")).expect("create source directory");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source fixture");

    std::fs::write(
        rules.join("candidate.yaml"),
        complete_rule(
            "team.candidate",
            "candidate",
            false,
            Some("(function_definition) @match"),
        ),
    )
    .expect("write valid candidate");
    let candidate = scan_json(temp.path(), true);
    let candidate_json = json(&candidate);
    assert_eq!(candidate_json["rules_applied"], 0);
    assert_eq!(candidate_json["config_issues_count"], 0);

    std::fs::write(
        rules.join("candidate.yaml"),
        complete_rule(
            "team.inconsistent",
            "candidate",
            true,
            Some("(function_definition) @match"),
        ),
    )
    .expect("write inconsistent candidate");
    let inconsistent = scan_json(temp.path(), false);
    assert_eq!(inconsistent.status.code(), Some(1));
    assert!(
        json(&inconsistent)["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );
}

#[test]
fn retained_gate_commands_do_not_mutate_user_data() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let protected: Vec<PathBuf> = [
        root.join("whetstone/.metrics.jsonl"),
        root.join("whetstone/.personal/config.yaml"),
        root.join("whetstone/.personal/rules/python/snake.yaml"),
        root.join("whetstone/.state/extraction-handoff.json"),
        root.join("whetstone/.state/inventory.json"),
        root.join("whetstone/.state/manifests.json"),
        root.join("whetstone/.state/refresh-log.json"),
        root.join("whetstone/.state/source-cache.json"),
        root.join("whetstone/context/AGENTS.md"),
        root.join("whetstone/evals/rust/test_anyhow.rs"),
        root.join("whetstone/evals/rust/test_clap.rs"),
        root.join("whetstone/evals/rust/test_reqwest.rs"),
        root.join("whetstone/evals/rust/test_serde_yaml.rs"),
        root.join("whetstone/evals/rust/test_whetstone:recommended/rust.rs"),
        root.join("whetstone/rules/rust/anyhow.yaml"),
        root.join("whetstone/whetstone.yaml"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect();
    // Published crates deliberately exclude project/user state. Repository
    // checkouts exercise every protected record that is present.
    if protected.is_empty() {
        return;
    }
    let before: Vec<Vec<u8>> = protected
        .iter()
        .map(|path| std::fs::read(path).expect("read protected fixture"))
        .collect();
    let root_arg = root.to_string_lossy();

    assert!(run(&["validate", "--project-dir", &root_arg], root)
        .status
        .success());
    assert!(run(&["eval", "--project-dir", &root_arg], root)
        .status
        .success());
    assert!(run(
        &[
            "scan",
            "src",
            "--project-dir",
            &root_arg,
            "--lang",
            "rust",
            "--no-fail",
        ],
        root,
    )
    .status
    .success());

    for (path, expected) in protected.iter().zip(before) {
        assert_eq!(
            std::fs::read(path).expect("reread protected fixture"),
            expected,
            "mutated {}",
            path.display()
        );
    }
}
