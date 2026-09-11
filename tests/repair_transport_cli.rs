#![cfg(unix)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::{ExternalRef, ExternalSystem};
use whetstone::repair_host::{RepairBudget, RepairTaskContext};
use whetstone::repair_transport::{
    HostRepairLaunch, AUTHORITY_SECRET_ENV, AUTHORITY_SOCKET_ENV, REPAIR_LAUNCH_ENV,
};
use whetstone::storage::{ProjectLayout, RecordStore, StoreKind};

const SECRET: &str = "host-generated-secret-000000000000000000000001";
const AUTHORITY_LOCATOR: &str = "host-task:transport-42";
const COMPLETION_LOCATOR: &str = "host-acceptance:transport-42";
const EXPIRES_AT: &str = "2099-01-01T00:00:00Z";
const EXPIRES_AT_UNIX: u64 = 4_070_908_800;

#[test]
fn host_socket_returns_post_edit_feedback_and_explicit_final_checkpoint() {
    let fixture = project_fixture();
    let launch = serde_json::to_string(&HostRepairLaunch {
        task: ExternalRef {
            system: ExternalSystem::Beads,
            stable_id: "whetstone-k5r.11".into(),
            revision: Some("1".into()),
        },
        authority_revision: 1,
        context: repair_context(),
    })
    .expect("host launch context");
    let begin_socket = fixture.socket_dir.path().join("begin.sock");
    let begin_host = serve_host(begin_socket.clone(), 2);
    let begin = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .current_dir(&fixture.project)
        .env(AUTHORITY_SOCKET_ENV, &begin_socket)
        .env(AUTHORITY_SECRET_ENV, SECRET)
        .env(REPAIR_LAUNCH_ENV, launch)
        .args([
            "--json",
            "check",
            "--request-id",
            "transport-flow-begin",
            "--repair-session",
            "transport-flow",
            "--authority-evidence",
            AUTHORITY_LOCATOR,
            "--begin-repair",
        ])
        .output()
        .expect("run process-level repair begin");
    if begin_host.join().is_err() {
        panic!(
            "begin host failed; status={:?}; stdout={}; stderr={}",
            begin.status.code(),
            String::from_utf8_lossy(&begin.stdout),
            String::from_utf8_lossy(&begin.stderr)
        );
    }
    assert_eq!(begin.status.code(), Some(1));
    let begin_json: Value = serde_json::from_slice(&begin.stdout).expect("begin JSON");
    assert_eq!(begin_json["data"]["repair"]["state"], "ready");
    assert_eq!(begin_json["data"]["repair"]["edit_authorized"], true);
    assert_eq!(
        begin_json["data"]["host_callback"]["command_family"],
        "check"
    );
    assert_eq!(
        begin_json["data"]["host_callback"]["requires_inherited_authority"],
        true
    );
    let before = latest_revision(&fixture, "transport-flow");
    let unavailable = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .current_dir(&fixture.project)
        .env_remove(AUTHORITY_SOCKET_ENV)
        .env_remove(AUTHORITY_SECRET_ENV)
        .args([
            "--json",
            "check",
            "--repair-session",
            "transport-flow",
            "--repair-revision",
            "1",
            "--authority-evidence",
            AUTHORITY_LOCATOR,
        ])
        .output()
        .expect("run unavailable explicit checkpoint");
    assert_eq!(unavailable.status.code(), Some(4));
    let unavailable_json: Value =
        serde_json::from_slice(&unavailable.stdout).expect("unavailable JSON");
    assert_eq!(unavailable_json["state"], "unavailable");
    assert_eq!(
        unavailable_json["data"]["reason_code"],
        "repair_host_adapter_unavailable"
    );
    assert_eq!(latest_revision(&fixture, "transport-flow"), before);

    fs::write(
        fixture.project.join("src/app.py"),
        "def read_config():\n    return 1\n",
    )
    .expect("repair source");

    let checkpoint_socket = fixture.socket_dir.path().join("checkpoint.sock");
    let checkpoint_host = serve_host(checkpoint_socket.clone(), 2);
    let callback = &begin_json["data"]["host_callback"];
    let callback_session = callback["repair_session"]
        .as_str()
        .expect("callback session");
    let callback_revision = callback["repair_revision"]
        .as_u64()
        .expect("callback revision")
        .to_string();
    let callback_authority = callback["authority_evidence"]
        .as_str()
        .expect("callback authority locator");
    let project_arg = fixture.project.to_string_lossy().to_string();
    let checkpoint = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        // Exercise the explicit project boundary: the socket is under this
        // caller directory, but outside the actual --project-dir.
        .current_dir(fixture.socket_dir.path())
        .env(AUTHORITY_SOCKET_ENV, &checkpoint_socket)
        .env(AUTHORITY_SECRET_ENV, SECRET)
        .args([
            "--json",
            "check",
            "--project-dir",
            &project_arg,
            "--repair-session",
            callback_session,
            "--repair-revision",
            &callback_revision,
            "--authority-evidence",
            callback_authority,
            "--post-edit",
        ])
        .output()
        .expect("run process-level post-edit checkpoint");
    if checkpoint_host.join().is_err() {
        panic!(
            "checkpoint host failed; status={:?}; stdout={}; stderr={}",
            checkpoint.status.code(),
            String::from_utf8_lossy(&checkpoint.stdout),
            String::from_utf8_lossy(&checkpoint.stderr)
        );
    }
    assert_eq!(checkpoint.status.code(), Some(6));
    let checkpoint_json: Value =
        serde_json::from_slice(&checkpoint.stdout).expect("checkpoint JSON");
    assert_eq!(checkpoint_json["state"], "needs_input");
    assert_eq!(
        checkpoint_json["data"]["repair"]["checkpoint"],
        "post_edit_hook"
    );
    assert_eq!(
        checkpoint_json["data"]["repair"]["state"],
        "ready_for_final_verification"
    );
    assert_eq!(checkpoint_json["data"]["repair"]["edit_authorized"], false);

    let final_socket = fixture.socket_dir.path().join("final.sock");
    // Finalization authenticates before the check, attests the exact
    // candidate, then re-authenticates after attestation to catch revocation
    // or expiry during that external operation.
    let final_host = serve_host(final_socket.clone(), 3);
    let final_check = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .current_dir(&fixture.project)
        .env(AUTHORITY_SOCKET_ENV, &final_socket)
        .env(AUTHORITY_SECRET_ENV, SECRET)
        .args([
            "--json",
            "check",
            "--repair-session",
            "transport-flow",
            "--repair-revision",
            "2",
            "--authority-evidence",
            AUTHORITY_LOCATOR,
            "--finalize-with",
            COMPLETION_LOCATOR,
        ])
        .output()
        .expect("run process-level final checkpoint");
    final_host.join().expect("final host");
    assert!(
        final_check.status.success(),
        "final checkpoint failed: {}",
        String::from_utf8_lossy(&final_check.stderr)
    );
    let final_json: Value = serde_json::from_slice(&final_check.stdout).expect("final JSON");
    assert_eq!(final_json["state"], "success");
    assert_eq!(final_json["data"]["repair"]["state"], "verified");
    assert_eq!(
        final_json["data"]["repair"]["policy_change_authorized"],
        false
    );
    assert_eq!(final_json["data"]["repair"]["merge_authorized"], false);
    assert_eq!(final_json["data"]["repair"]["release_authorized"], false);
    assert_eq!(final_json["data"]["repair"]["outcome"], "unknown");

    let persisted = RecordStore::open_existing(&fixture.layout.private_store(), StoreKind::Private)
        .expect("reopen private store")
        .all_records()
        .expect("read private history");
    let persisted_json = serde_json::to_string(&persisted).expect("serialize private history");
    assert!(!persisted_json.contains(SECRET));
    assert!(!persisted_json.contains(checkpoint_socket.to_string_lossy().as_ref()));
    assert!(!persisted_json.contains(final_socket.to_string_lossy().as_ref()));
}

#[test]
fn project_local_socket_is_rejected_when_project_dir_is_a_subdirectory() {
    let project = tempfile::Builder::new()
        .prefix("whetstone-repair-host-")
        .tempdir_in("/tmp")
        .expect("repo-local socket project");
    fs::set_permissions(project.path(), fs::Permissions::from_mode(0o700))
        .expect("protect repo-local socket parent");
    // Bind before initializing Git: the execution sandbox intentionally
    // rejects creating sockets inside repositories, while production and CI
    // still exercise the exact resulting filesystem boundary below.
    let socket = project.path().join("a.sock");
    let listener = UnixListener::bind(&socket).expect("bind repo-local socket");
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))
        .expect("protect repo-local socket");
    command(project.path(), "git", &["init", "--quiet"]);
    let project_subdir = project.path().join("src");
    fs::create_dir(&project_subdir).expect("project subdirectory");
    let launch = serde_json::to_string(&HostRepairLaunch {
        task: ExternalRef {
            system: ExternalSystem::Beads,
            stable_id: "whetstone-k5r.11".into(),
            revision: Some("1".into()),
        },
        authority_revision: 1,
        context: repair_context(),
    })
    .expect("host launch context");
    listener
        .set_nonblocking(true)
        .expect("nonblocking repo-local listener");
    let project_arg = project_subdir.to_string_lossy().to_string();

    let output = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .current_dir(&project_subdir)
        .env(AUTHORITY_SOCKET_ENV, &socket)
        .env(AUTHORITY_SECRET_ENV, SECRET)
        .env(REPAIR_LAUNCH_ENV, launch)
        .args([
            "--json",
            "check",
            "--project-dir",
            &project_arg,
            "--repair-session",
            "repo-local-socket",
            "--authority-evidence",
            AUTHORITY_LOCATOR,
            "--begin-repair",
        ])
        .output()
        .expect("run with repo-local socket");

    assert_eq!(output.status.code(), Some(4));
    let response: Value = serde_json::from_slice(&output.stdout).expect("unavailable JSON");
    assert_eq!(
        response["data"]["reason_code"],
        "repair_authority_unavailable"
    );
    assert!(
        listener.accept().is_err(),
        "validation must reject a socket anywhere under the actual Git root before connecting"
    );
}

fn serve_host(socket_path: PathBuf, requests: usize) -> thread::JoinHandle<()> {
    let listener = UnixListener::bind(&socket_path).expect("bind host socket");
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
        .expect("protect host socket");
    listener
        .set_nonblocking(true)
        .expect("nonblocking host socket");
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut handled = 0;
        while handled < requests && Instant::now() < deadline {
            let mut stream = match listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(error) => panic!("host accept failed: {error}"),
            };
            let mut line = String::new();
            BufReader::new(stream.try_clone().expect("clone host stream"))
                .read_line(&mut line)
                .expect("read host request");
            let request: Value = serde_json::from_str(&line).expect("host request JSON");
            assert_eq!(request["one_time_secret"], SECRET);
            let request_id = request["request_id"].as_str().expect("request id");
            let response = match request["schema"].as_str().expect("request schema") {
                "whetstone.repair-authority-request.v1" => json!({
                    "schema": "whetstone.repair-authority-response.v1",
                    "request_id": request_id,
                    "state": "granted",
                    "grant": {
                        "evidence_id": "verified-socket-host:transport-42:r1",
                        "principal": {
                            "kind": "local_user",
                            "stable_id": "transport-owner-42"
                        },
                        "task": request["target"]["task"].clone(),
                        "project": request["target"]["project"].clone(),
                        "authority_revision": request["target"]["authority_revision"].clone(),
                        "expires_at": EXPIRES_AT,
                        "expires_at_unix": EXPIRES_AT_UNIX,
                        "context": request["target"]["context"].clone(),
                        "may_edit_source": true,
                        "may_edit_policy": false,
                        "may_edit_checks": false,
                        "may_reset_baselines": false,
                        "may_publish": false,
                        "may_merge": false,
                        "may_release": false
                    }
                }),
                "whetstone.repair-completion-request.v1" => {
                    assert_eq!(request["evidence_locator"], COMPLETION_LOCATOR);
                    json!({
                        "schema": "whetstone.repair-completion-response.v1",
                        "request_id": request_id,
                        "state": "granted",
                        "grant": {
                            "evidence_id": "verified-socket-acceptance:transport-42:r2",
                            "session_id": request["target"]["session_id"].clone(),
                            "task": request["target"]["task"].clone(),
                            "project": request["target"]["project"].clone(),
                            "authority_revision": request["target"]["authority_revision"].clone(),
                            "candidate_workspace": request["target"]["candidate_workspace"].clone(),
                            "check_snapshot": request["target"]["check_snapshot"].clone(),
                            "task_acceptance_satisfied": true,
                            "required_reviews_satisfied": true
                        }
                    })
                }
                other => panic!("unexpected host schema: {other}"),
            };
            serde_json::to_writer(&mut stream, &response).expect("write host response");
            stream.write_all(b"\n").expect("terminate host response");
            handled += 1;
        }
        assert_eq!(
            handled, requests,
            "host did not receive every expected request"
        );
    })
}

struct Fixture {
    _project_dir: TempDir,
    socket_dir: TempDir,
    project: PathBuf,
    layout: ProjectLayout,
}

fn project_fixture() -> Fixture {
    let project_dir = tempfile::Builder::new()
        .prefix("whetstone-repair-project-")
        .tempdir_in("/tmp")
        .expect("project tempdir");
    let socket_dir = tempfile::Builder::new()
        .prefix("whetstone-repair-host-")
        // Keep the Unix-socket path short on macOS while remaining valid on
        // Linux CI; `/private/tmp` is a macOS-specific alias.
        .tempdir_in("/tmp")
        .expect("socket tempdir");
    fs::set_permissions(socket_dir.path(), fs::Permissions::from_mode(0o700))
        .expect("protect socket directory");
    let project = project_dir.path().join("project");
    fs::create_dir(&project).expect("project root");
    command(&project, "git", &["init", "--quiet"]);
    fs::create_dir(project.join("src")).expect("source directory");
    fs::write(
        project.join("src/app.py"),
        "def ReadConfig():\n    return 1\n",
    )
    .expect("violating source");
    fs::create_dir_all(project.join("whetstone/rules/python")).expect("rules directory");
    fs::write(
        project.join("whetstone/rules/python/names.yaml"),
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
    .expect("rule fixture");
    command(&project, "git", &["add", "."]);
    command(
        &project,
        "git",
        &[
            "-c",
            "user.name=Whetstone Test",
            "-c",
            "user.email=test@whetstone.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    );
    let layout = ProjectLayout::resolve(&project, None).expect("layout");
    RecordStore::initialize(&layout.private_store(), StoreKind::Private)
        .expect("private repair store");
    Fixture {
        _project_dir: project_dir,
        socket_dir,
        project,
        layout,
    }
}

fn repair_context() -> RepairTaskContext {
    RepairTaskContext {
        objective: "Rename the function to satisfy the accepted rule.".into(),
        non_goals: vec!["Do not change policy, tests, baselines, or publication state.".into()],
        applicable_guidance: Vec::new(),
        allowed_paths: vec!["src".into()],
        excluded_paths: Vec::new(),
        check_paths: vec!["src".into()],
        check_language: Some("python".into()),
        required_rules: vec!["team.lowercase-functions".into()],
        final_check_paths: vec!["src".into()],
        final_check_language: Some("python".into()),
        final_required_rules: vec!["team.lowercase-functions".into()],
        budget: RepairBudget {
            max_attempts: 3,
            max_repeated_finding: 2,
            max_elapsed_seconds: 600,
            max_resource_units: 3,
        },
    }
}

fn latest_revision(fixture: &Fixture, session_id: &str) -> u64 {
    use whetstone::domain::RecordId;
    RecordStore::open_existing(&fixture.layout.private_store(), StoreKind::Private)
        .expect("open private store")
        .latest(&RecordId::new(format!("repair.session.{session_id}")).expect("session id"))
        .expect("read session")
        .expect("session record")
        .revision
}

fn command(cwd: &Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run fixture command");
    assert!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
