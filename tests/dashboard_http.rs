//! The dashboard's HTTP service: loopback bootstrap, host, origin, session
//! and CSRF gates, the static shell's security headers, and the JSON API over
//! the same command service the CLI uses: the view of
//! `whetstone/rulebook.json`, and edits, accepts, withdrawals and retirements
//! through `wh change`. Every project is a throwaway Git repository and no
//! record store is opened. Asset markup is deliberately not asserted here.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use whetstone::dashboard::{BackendResponse, DashboardBackend, DashboardHandle, DashboardMode};
use whetstone::dashboard_service::CommandDashboardBackend;
use whetstone::domain::ActorKind;
use whetstone::service::{
    ChangeKind, ChangeRequest, CheckRequest, CommandService, DashRequest, GateMode, ServiceRequest,
    ServiceResponse, ServiceState,
};

const MISSION: &str = "Make project intent inspectable.";
const CUSTOM_PRINCIPLE: &str = "Evidence before assertion.";

#[derive(Default)]
struct Backend {
    inspections: AtomicUsize,
    mutations: AtomicUsize,
}

impl DashboardBackend for Backend {
    fn inspect(&self, _request_body: &[u8]) -> BackendResponse {
        self.inspections.fetch_add(1, Ordering::Relaxed);
        BackendResponse::json(200, &json!({"state":"success","source":"shared-service"}))
    }

    fn mutate(&self, body: &[u8]) -> BackendResponse {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        BackendResponse::json(
            200,
            &json!({"state":"success","request":String::from_utf8_lossy(body)}),
        )
    }
}

fn send(address: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("connect dashboard");
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .expect("read timeout");
    stream.write_all(request.as_bytes()).expect("write request");
    let mut response = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => response.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
            Err(error) => panic!("read response: {error}"),
        }
    }
    String::from_utf8(response).expect("UTF-8 response")
}

fn host(handle: &DashboardHandle) -> String {
    format!("127.0.0.1:{}", handle.address().port())
}

fn response_body(response: &str) -> Value {
    serde_json::from_str(
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("HTTP response body"),
    )
    .expect("JSON response")
}

fn init_git(root: &Path) {
    for args in [
        vec!["init", "--quiet"],
        vec!["config", "user.name", "Dashboard Owner"],
        vec!["config", "user.email", "owner@example.test"],
        vec!["config", "commit.gpgsign", "false"],
    ] {
        let output = Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .expect("initialize Git fixture");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn service(request: ServiceRequest) -> ServiceResponse {
    CommandService.execute(request)
}

/// A test command that passes, kept out of Git, so the prove-it-works
/// starter is one a check can hold.
fn with_test_command(root: &Path) {
    std::fs::write(root.join("package.json"), r#"{"scripts":{"test":"true"}}"#)
        .expect("write package.json");
    std::fs::write(root.join(".git/info/exclude"), "package.json\n").expect("exclude");
}

fn change(root: &Path, request: ChangeRequest) -> ServiceResponse {
    let response = service(ServiceRequest::Change(ChangeRequest {
        project_dir: root.to_path_buf(),
        ..request
    }));
    assert_eq!(
        response.state,
        ServiceState::Success,
        "{}",
        response.summary
    );
    response
}

/// Mission, one pstack principle, one custom principle and a passing must
/// rule drafted into the rulebook file through the service directly.
fn draft_rulebook(root: &Path) {
    with_test_command(root);
    change(
        root,
        ChangeRequest {
            kind: Some(ChangeKind::Mission),
            content: Some(MISSION.into()),
            ..Default::default()
        },
    );
    for (id, content, definition) in [
        (
            "principle.prove-it-works",
            "Prove it works.",
            Some(r#"{"type":"principle","pstack":"prove-it-works"}"#),
        ),
        ("principle.custom-evidence", CUSTOM_PRINCIPLE, None),
    ] {
        change(
            root,
            ChangeRequest {
                kind: Some(ChangeKind::Principle),
                record_id: Some(id.into()),
                content: Some(content.into()),
                definition: definition
                    .map(|text| Box::new(serde_json::from_str(text).expect("definition"))),
                ..Default::default()
            },
        );
    }
    change(
        root,
        ChangeRequest {
            kind: Some(ChangeKind::Rule),
            record_id: Some("rule.tests-pass".into()),
            content: Some("The tests pass.".into()),
            rationale: Some("Work is done when it runs.".into()),
            definition: Some(Box::new(
                serde_json::from_value(json!({
                    "type": "rule",
                    "strength": "must",
                    "enforcer": {"kind": "test", "command": "true"}
                }))
                .expect("definition"),
            )),
            ..Default::default()
        },
    );
}

fn start(project: &Path, allow_mutations: bool) -> DashboardHandle {
    DashboardHandle::start(
        DashboardMode::Local { allow_mutations },
        Arc::new(CommandDashboardBackend::new(project.to_path_buf(), None)),
    )
    .expect("dashboard")
}

/// A dashboard a person opened: only a person may widen a sweep class.
fn start_for_a_person(project: &Path) -> DashboardHandle {
    DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: true,
        },
        Arc::new(
            CommandDashboardBackend::new(project.to_path_buf(), None).acting_as(ActorKind::Person),
        ),
    )
    .expect("dashboard")
}

fn bootstrap(handle: &DashboardHandle) -> (String, String) {
    let host = host(handle);
    let token = handle.take_bootstrap_fragment().expect("bootstrap token");
    let response = send(
        handle.address(),
        &format!(
            "POST /session/bootstrap HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nX-Whetstone-Bootstrap: {token}\r\nContent-Length: 0\r\n\r\n"
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("HttpOnly; SameSite=Strict"));
    let cookie = response
        .lines()
        .find(|line| line.starts_with("Set-Cookie:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.strip_suffix(';'))
        .expect("session cookie")
        .to_string();
    let json = response_body(&response);
    let csrf = json["csrf_token"].as_str().expect("csrf").to_string();
    (cookie, csrf)
}

/// A bootstrapped browser session with editing switched on.
struct Session {
    handle: DashboardHandle,
    host: String,
    cookie: String,
    csrf: String,
}

impl Session {
    fn open(handle: DashboardHandle) -> Self {
        let host = host(&handle);
        let (cookie, csrf) = bootstrap(&handle);
        let edit = send(
            handle.address(),
            &format!("POST /session/edit HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 0\r\n\r\n"),
        );
        assert!(edit.starts_with("HTTP/1.1 200"), "{edit}");
        Self {
            handle,
            host,
            cookie,
            csrf,
        }
    }

    fn raw_command(&self, body: &str) -> String {
        let Self {
            host, cookie, csrf, ..
        } = self;
        send(
            self.handle.address(),
            &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()),
        )
    }

    fn command(&self, body: &Value) -> Value {
        let response = self.raw_command(&body.to_string());
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        response_body(&response)
    }

    fn inspect(&self) -> Value {
        let response = send(
            self.handle.address(),
            &format!("GET /api/inspect HTTP/1.1\r\nHost: {}\r\n\r\n", self.host),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        response_body(&response)
    }
}

fn rule_entry<'a>(view: &'a Value, id: &str) -> &'a Value {
    view["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["id"] == id)
        .unwrap_or_else(|| panic!("no rule {id} in {}", view["rules"]))
}

#[test]
fn loopback_bootstrap_is_one_time_and_secrets_are_not_in_printable_values() {
    let backend = Arc::new(Backend::default());
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        backend,
    )
    .expect("start dashboard");
    assert_eq!(handle.address().ip().to_string(), "127.0.0.1");
    assert_ne!(handle.address().port(), 0);
    assert!(!handle.public_url().contains('#'));
    assert!(format!("{handle:?}").contains("[REDACTED]"));
    let token = handle.take_bootstrap_fragment().expect("token");
    assert!(handle.take_bootstrap_fragment().is_none());
    let host = host(&handle);
    let first = send(
        handle.address(),
        &format!("POST /session/bootstrap HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nX-Whetstone-Bootstrap: {token}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(first.starts_with("HTTP/1.1 200"));
    let replay = send(
        handle.address(),
        &format!("POST /session/bootstrap HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nX-Whetstone-Bootstrap: {token}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(replay.starts_with("HTTP/1.1 403"));
}

#[test]
fn exact_host_origin_fetch_metadata_session_and_csrf_gate_mutations() {
    let backend = Arc::new(Backend::default());
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: true,
        },
        backend.clone(),
    )
    .expect("start dashboard");
    let host = host(&handle);
    let (cookie, csrf) = bootstrap(&handle);

    let bad_host = send(
        handle.address(),
        "GET /api/inspect HTTP/1.1\r\nHost: attacker.example\r\n\r\n",
    );
    assert!(bad_host.starts_with("HTTP/1.1 403"));
    let cross_site = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: https://attacker.example\r\nSec-Fetch-Site: cross-site\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(cross_site.starts_with("HTTP/1.1 403"));
    let no_csrf = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(no_csrf.starts_with("HTTP/1.1 403"));
    let wrong_csrf = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: not-the-token\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(wrong_csrf.starts_with("HTTP/1.1 403"), "{wrong_csrf}");
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 0);

    // A session starts read-only until editing is switched on.
    let read_only = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(read_only.contains("read_only"));
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 0);
    let edit = send(
        handle.address(),
        &format!("POST /session/edit HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(edit.starts_with("HTTP/1.1 200"));
    let mutation = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(mutation.starts_with("HTTP/1.1 200"), "{mutation}");
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 1);
    // Inspections never count as mutations.
    let inspection = send(
        handle.address(),
        &format!("GET /api/inspect HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    assert!(inspection.contains("shared-service"));
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 1);
    assert_eq!(backend.inspections.load(Ordering::Relaxed), 1);
}

#[test]
fn a_read_only_dashboard_never_enables_editing() {
    let backend = Arc::new(Backend::default());
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        backend.clone(),
    )
    .expect("start dashboard");
    let host = host(&handle);
    let (cookie, csrf) = bootstrap(&handle);
    let edit = send(
        handle.address(),
        &format!("POST /session/edit HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(edit.contains("read_only"), "{edit}");
    let mutation = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(mutation.contains("read_only"), "{mutation}");
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 0);
}

#[test]
fn traversal_oversize_and_security_header_fixtures_fail_closed() {
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(Backend::default()),
    )
    .expect("start dashboard");
    let host = host(&handle);
    let traversal = send(
        handle.address(),
        &format!("GET /../secret HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    assert!(traversal.starts_with("HTTP/1.1 400"));
    let oversized = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nContent-Length: 70000\r\n\r\n"),
    );
    assert!(oversized.starts_with("HTTP/1.1 413"));
    let missing = send(
        handle.address(),
        &format!("GET /not-an-asset.js HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    assert!(missing.starts_with("HTTP/1.1 404"), "{missing}");
    let page = send(
        handle.address(),
        &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    for expected in [
        "Content-Security-Policy: default-src 'none'",
        "X-Frame-Options: DENY",
        "Referrer-Policy: no-referrer",
        "Cache-Control: no-store",
        "Cross-Origin-Resource-Policy: same-origin",
    ] {
        assert!(page.contains(expected), "missing {expected}");
    }
    assert!(!page
        .to_ascii_lowercase()
        .contains("access-control-allow-origin"));
    assert!(!page.contains("https://"));

    // The shell and its startup assets stay within the startup budget and
    // make no third-party request; lazy modules are bounded on their own.
    let body = |response: &str| response.split_once("\r\n\r\n").expect("body").1.to_string();
    let mut startup_bytes = body(&page).len();
    for (path, budget, lazy) in [
        ("/app.css", 24 * 1024, false),
        ("/app.js", 24 * 1024, false),
        ("/edit.js", 16 * 1024, true),
        ("/views.js", 16 * 1024, true),
        ("/views.css", 12 * 1024, true),
    ] {
        let asset = send(
            handle.address(),
            &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        );
        assert!(asset.starts_with("HTTP/1.1 200"), "{path}: {asset}");
        assert!(asset.contains("Cache-Control: no-store"), "{path}");
        let content = body(&asset);
        assert!(content.len() < budget, "{path} exceeds {budget} bytes");
        if !lazy {
            startup_bytes += content.len();
        }
        let without_svg_namespace = content.replace("http://www.w3.org/2000/svg", "");
        assert!(
            !without_svg_namespace.contains("http://")
                && !without_svg_namespace.contains("https://"),
            "{path} has a third-party request"
        );
    }
    assert!(
        startup_bytes < 24 * 1024,
        "complete dashboard HTML, CSS, and JavaScript exceed 24 KiB"
    );
}

#[test]
fn the_shell_loads_under_its_csp_without_inline_code_or_markup_writes() {
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(Backend::default()),
    )
    .expect("dashboard");
    let host = host(&handle);
    let get = |path: &str| {
        let response = send(
            handle.address(),
            &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        );
        assert!(response.starts_with("HTTP/1.1 200"), "{path}: {response}");
        response
    };
    let page = get("/");
    assert!(
        page.to_ascii_lowercase()
            .contains("content-type: text/html"),
        "the shell is HTML"
    );
    let html = page.split_once("\r\n\r\n").expect("body").1;
    assert!(!html.contains("<script>"), "no inline script under the CSP");
    assert!(!html.contains("style="), "no inline style under the CSP");
    assert!(
        html.contains("/app.js") && html.contains("/app.css"),
        "the shell loads its startup assets"
    );
    for path in ["/app.js", "/views.js", "/edit.js"] {
        let script = get(path);
        assert!(
            script.to_ascii_lowercase().contains("javascript"),
            "{path} is served as JavaScript"
        );
        let script = script.split_once("\r\n\r\n").expect("body").1;
        assert!(
            !script.contains("innerHTML"),
            "{path} must build DOM without innerHTML"
        );
        assert!(
            !script.contains("outerHTML"),
            "{path} must not write markup"
        );
        assert!(
            !script.contains("eval("),
            "{path} must not evaluate strings"
        );
    }
}

#[test]
fn repeated_launch_and_owned_shutdown_do_not_touch_another_server() {
    let first = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(Backend::default()),
    )
    .expect("first");
    let second = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(Backend::default()),
    )
    .expect("second");
    assert_ne!(first.address(), second.address());
    let second_address = second.address();
    let second_host = host(&second);
    first.shutdown();
    let response = send(
        second_address,
        &format!("GET / HTTP/1.1\r\nHost: {second_host}\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"));
}

#[test]
fn command_backend_inspection_matches_the_cli_service_exactly() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let request_id = Some("dashboard-parity".to_string());
    let expected = service(ServiceRequest::Dash(DashRequest::basic(
        temp.path().to_path_buf(),
        request_id.clone(),
    )));
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(CommandDashboardBackend::new(
            temp.path().to_path_buf(),
            request_id,
        )),
    )
    .expect("dashboard");
    let response = send(
        handle.address(),
        &format!(
            "GET /api/inspect HTTP/1.1\r\nHost: {}\r\n\r\n",
            host(&handle)
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(
        response_body(&response),
        serde_json::to_value(expected).expect("service JSON")
    );
}

#[test]
fn an_unestablished_project_shows_onboarding_and_no_removed_sections() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("manifest");
    let session = Session::open(start(temp.path(), true));
    let body = session.inspect();
    assert_eq!(body["workflow"], "dash");
    assert_eq!(body["data"]["rulebook"]["state"], "missing");
    let view = &body["data"]["current"];
    assert_eq!(view["established"], false, "{view}");
    assert_eq!(view["header"]["agreement"], "not started");
    assert_eq!(view["mission"], Value::Null);
    assert_eq!(view["principles"], json!([]));
    assert_eq!(view["rules"], json!([]));
    assert_eq!(view["drafts"], json!([]));
    let steps = view["onboarding"]["steps"]
        .as_array()
        .expect("onboarding steps")
        .iter()
        .map(|step| step["key"].as_str().expect("key"))
        .collect::<Vec<_>>();
    assert_eq!(steps, ["mission", "principles", "rules", "gates"]);
    let starters = view["onboarding"]["starters"].as_array().expect("starters");
    let ids = starters
        .iter()
        .map(|starter| starter["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    assert_eq!(ids, ["rule.prove-it-works"]);
    let prove = &starters[0];
    assert_eq!(prove["enforcer"], "Test", "Cargo.toml means cargo test");
    assert_eq!(prove["strength"], "must");
    assert_eq!(prove["accepted"], false);
    assert_eq!(prove["edit"]["kind"], "rule");
    assert_eq!(
        prove["edit"]["definition"]["enforcer"],
        json!({"kind": "test", "command": "cargo test"})
    );
    assert!(
        view["onboarding"]["catalogue"]
            .as_array()
            .expect("catalogue")
            .iter()
            .any(|entry| entry["id"] == "prove-it-works"),
        "the pstack principle catalogue is offered"
    );
    // The removed direction and the record store have no place in the view.
    assert!(view.get("stages").is_none(), "stages were removed");
    assert!(
        view["onboarding"].get("decisions").is_none(),
        "the eight decisions were removed"
    );
    assert!(view["onboarding"].get("tools").is_none());
    for gone in ["store", "sync", "progress", "history"] {
        assert!(
            body["data"].get(gone).is_none(),
            "{gone} is not in the view"
        );
    }
    let workflows = body["data"]["workflows"]
        .as_array()
        .expect("workflows")
        .iter()
        .map(|workflow| workflow["name"].as_str().expect("name"))
        .collect::<Vec<_>>();
    assert_eq!(workflows, ["init", "dash", "change", "check", "explain"]);
    // Nothing was written by looking.
    assert!(!temp.path().join("whetstone").exists());
}

#[test]
fn onboarding_drafts_the_starter_through_change_and_accepting_commits_it() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    with_test_command(temp.path());
    let session = Session::open(start(temp.path(), true));
    let starter = session.inspect()["data"]["current"]["onboarding"]["starters"][0].clone();
    assert_eq!(starter["id"], "rule.prove-it-works");

    // What the onboarding form sends: the mission, a catalogue principle,
    // a principle of the owner's own and the starter, each a change.
    for body in [
        json!({"workflow": "change", "kind": "mission", "content": MISSION}),
        json!({
            "workflow": "change",
            "kind": "principle",
            "record_id": "principle.prove-it-works",
            "content": "Prove it works.",
            "definition": {"type": "principle", "pstack": "prove-it-works"}
        }),
        json!({
            "workflow": "change",
            "kind": "principle",
            "record_id": "principle.custom-evidence-before-assertion",
            "content": CUSTOM_PRINCIPLE
        }),
        json!({
            "workflow": "change",
            "kind": "rule",
            "record_id": starter["edit"]["record_id"],
            "content": starter["edit"]["content"],
            "rationale": starter["edit"]["rationale"],
            "definition": starter["edit"]["definition"]
        }),
    ] {
        let drafted = session.command(&body);
        assert_eq!(drafted["state"], "success", "{body}: {drafted}");
    }
    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(view["established"], true, "{view}");
    assert_eq!(view["header"]["agreement"], "drafted, not accepted");
    assert_eq!(view["header"]["revision"], "uncommitted");
    assert_eq!(
        view["drafts"],
        json!([
            "set the mission",
            "add principle.custom-evidence-before-assertion",
            "add principle.prove-it-works",
            format!(
                "add rule.prove-it-works ({})",
                rule_entry(&view, "rule.prove-it-works")["digest"]
                    .as_str()
                    .expect("digest")
            )
        ])
    );
    assert_eq!(view["header"]["drafts"], 4);
    assert_eq!(view["attention"][0]["focus"], "drafts");
    let prove = rule_entry(&view, "rule.prove-it-works");
    assert_eq!(prove["lifecycle"], "new");
    assert_eq!(prove["state"]["tone"], "draft");
    assert_eq!(view["onboarding"]["starters"][0]["accepted"], true);
    assert_eq!(
        rulebook(temp.path())["rules"][0]["id"],
        "rule.prove-it-works"
    );

    let accepted = session.command(&json!({
        "workflow": "change",
        "accept": true,
        "rationale": "The owner agreed the mission and the first rule."
    }));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let body = session.inspect();
    let view = &body["data"]["current"];
    assert_eq!(view["header"]["agreement"], "accepted");
    assert_eq!(view["header"]["revision"], accepted["data"]["commit"]);
    assert_eq!(view["drafts"], json!([]));
    assert_eq!(view["attention"], json!([]));
    assert_eq!(view["mission"]["title"], MISSION);
    assert_eq!(view["mission"]["lifecycle"], "accepted");
    assert_eq!(view["mission"]["edit"]["record_id"], Value::Null);
    assert_eq!(view["principles"].as_array().map(Vec::len), Some(2));
    let prove = rule_entry(view, "rule.prove-it-works");
    assert_eq!(prove["lifecycle"], "accepted");
    assert!(prove["examples"]
        .as_array()
        .is_some_and(|examples| examples.len() >= 2));
    let changelog = &body["data"]["changelog"];
    assert_eq!(changelog.as_array().map(Vec::len), Some(1));
    assert_eq!(changelog[0]["commit"], accepted["data"]["commit"]);
    assert_eq!(changelog[0]["owner"], "Dashboard Owner");
    assert_eq!(view["latest_change"]["title"], changelog[0]["title"]);
}

#[test]
fn request_bodies_name_no_other_project_and_no_workflow_but_change() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let session = Session::open(start(temp.path(), true));
    // Request bodies cannot name another project, another workflow (init
    // writes nothing from the dashboard, and there is no push or pull), a
    // retired field or a retired change kind.
    for rejected in [
        r#"{"workflow":"check","project_dir":"/","paths":["."]}"#,
        r#"{"workflow":"init","action":"inspect"}"#,
        r#"{"workflow":"init","action":"agree","mission":"Make intent inspectable."}"#,
        r#"{"workflow":"pull"}"#,
        r#"{"workflow":"push"}"#,
        r#"{"workflow":"change","project_dir":"/","kind":"mission","content":"x"}"#,
        r#"{"workflow":"change","kind":"value","record_id":"value.core"}"#,
        r#"{"workflow":"change","kind":"exception","record_id":"exception.one"}"#,
        r#"{"workflow":"change","flag":{"receipt":"x","verdict":"dismiss"}}"#,
        r#"{"workflow":"change","expected_revision":1,"resume_token":"x"}"#,
        r#"{"workflow":"change","review":{"proposal":"proposal.x","verdict":"accept"}}"#,
        r#"{"workflow":"check","mode":"repair"}"#,
        r#"{"workflow":"task","title":"No description"}"#,
        "not json",
    ] {
        let denied = session.raw_command(rejected);
        assert!(denied.starts_with("HTTP/1.1 400"), "{rejected}: {denied}");
        assert_eq!(response_body(&denied)["state"], "unknown", "{rejected}");
    }
    assert!(!temp.path().join("whetstone").exists());
}

#[test]
fn dashboard_change_preview_is_identical_to_the_cli_service_path() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let direct = service(ServiceRequest::Change(ChangeRequest {
        project_dir: temp.path().to_path_buf(),
        kind: Some(ChangeKind::Mission),
        content: Some(MISSION.into()),
        preview: true,
        actor: ActorKind::Agent,
        ..Default::default()
    }));
    let session = Session::open(start(temp.path(), true));
    let response = session.command(&json!({
        "workflow": "change",
        "kind": "mission",
        "content": MISSION,
        "preview": true
    }));
    assert_eq!(
        response,
        serde_json::to_value(direct).expect("service JSON")
    );
    assert_eq!(response["data"]["preview_only"], true);
    assert!(!temp.path().join("whetstone/rulebook.json").exists());
}

#[test]
fn dashboard_queries_are_typed_filtered_and_match_the_cli_service() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    draft_rulebook(temp.path());

    let direct = service(ServiceRequest::Dash(DashRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-filter".into()),
        search: Some(CUSTOM_PRINCIPLE.into()),
        as_of: Some("2099-01-01T00:00:00Z".into()),
        page_size: 7,
        trail: false,
    }));
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(CommandDashboardBackend::new(
            temp.path().to_path_buf(),
            Some("dashboard-filter".into()),
        )),
    )
    .expect("dashboard");
    let host = host(&handle);
    let serialized = json!({
        "search": CUSTOM_PRINCIPLE,
        "as_of": "2099-01-01T00:00:00Z",
        "page_size": 7
    })
    .to_string();
    let response = send(
        handle.address(),
        &format!(
            "POST /api/inspect HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}",
            serialized.len()
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let body = response_body(&response);
    assert_eq!(body, serde_json::to_value(direct).expect("service JSON"));
    assert_eq!(
        body["data"]["changelog"],
        json!([]),
        "the changelog is the rulebook's Git log, and nothing committed it"
    );
    let current = &body["data"]["current"];
    assert_eq!(
        current["mission"]["title"], MISSION,
        "a filtered changelog must never replace the independent current-state projection"
    );
    assert_eq!(current["principles"].as_array().map(Vec::len), Some(2));
    assert_eq!(current["rules"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        current["checks"]["last_complete"],
        Value::Null,
        "no gate has run, so no completed check may be claimed"
    );
    assert_eq!(current["header"]["revision"], "uncommitted");
    assert_eq!(body["data"]["rulebook"]["revision"], "uncommitted");
}

#[test]
fn dashboard_changelog_query_rejects_unknown_fields_and_marks_invalid_time_unknown() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    draft_rulebook(temp.path());
    let handle = start(temp.path(), false);
    let host = host(&handle);
    for (body, expected_status) in [
        (json!({"project_dir": "/"}), "HTTP/1.1 400"),
        (json!({"history_after": "cursor"}), "HTTP/1.1 400"),
        (json!({"expected_snapshot": "sha256:x"}), "HTTP/1.1 400"),
        (json!({"as_of": "not-a-time"}), "HTTP/1.1 200"),
    ] {
        let serialized = body.to_string();
        let response = send(
            handle.address(),
            &format!(
                "POST /api/inspect HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}",
                serialized.len()
            ),
        );
        assert!(response.starts_with(expected_status), "{body}: {response}");
        if body.get("as_of").is_some() {
            assert_eq!(response_body(&response)["state"], "unknown");
            assert!(response_body(&response)["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("not an RFC 3339 UTC instant")));
        } else {
            assert_eq!(response_body(&response)["state"], "unknown");
        }
    }
}

#[test]
fn a_malformed_rulebook_is_reported_and_never_shown_as_empty() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("whetstone")).expect("dir");
    std::fs::write(
        temp.path().join("whetstone/rulebook.json"),
        r#"{"schema":"whetstone.rulebook.v1","mission":{"statement":"x"},"principles":[],"rules":[],"extra":1}"#,
    )
    .expect("write");
    let session = Session::open(start(temp.path(), true));
    let body = session.inspect();
    assert_eq!(body["state"], "unknown", "{body}");
    assert_eq!(body["data"]["rulebook"]["state"], "invalid");
    assert!(body["data"]["rulebook"]["detail"]
        .as_str()
        .is_some_and(|detail| detail.contains("extra")));
    assert_eq!(body["data"]["current"], Value::Null);
}

/// The rulebook file as the dashboard's change commands left it.
fn rulebook(root: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(root.join("whetstone/rulebook.json")).expect("rulebook"),
    )
    .expect("rulebook JSON")
}

#[test]
fn a_rule_change_is_drafted_in_the_file_and_accepted_as_a_commit() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let project = temp.path().to_path_buf();
    let handle = start(&project, true);
    let host = host(&handle);

    // Without a session the change never reaches the service.
    let probe = json!({
        "workflow": "change",
        "request_id": "dash-rule",
        "kind": "rule",
        "record_id": "rule.always-flags"
    })
    .to_string();
    let denied = send(
        handle.address(),
        &format!(
            "POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Length: {}\r\n\r\n{probe}",
            probe.len()
        ),
    );
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
    let session = Session::open(handle);
    let mission = session.command(&json!({
        "workflow": "change",
        "kind": "mission",
        "content": MISSION
    }));
    assert_eq!(mission["state"], "success", "{mission}");

    // A rule needs its definition: one strength and one enforcer.
    let missing_definition = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-rule-bare",
        "kind": "rule",
        "record_id": "rule.bare",
        "content": "A rule with no enforcer.",
        "rationale": "Probe."
    }));
    assert_eq!(
        missing_definition["state"], "needs_input",
        "{missing_definition}"
    );
    let two_enforcers = session.raw_command(
        &json!({
            "workflow": "change",
            "request_id": "dash-rule-two",
            "kind": "rule",
            "record_id": "rule.two",
            "content": "Two enforcers.",
            "rationale": "Probe.",
            "definition": {
                "type": "rule",
                "strength": "must",
                "enforcer": {"kind": "test", "command": "cargo test"},
                "enforcers": [{"kind": "lint", "tool": "ruff", "code": "F401"}]
            }
        })
        .to_string(),
    );
    assert!(two_enforcers.starts_with("HTTP/1.1 400"), "{two_enforcers}");

    let change = json!({
        "workflow": "change",
        "request_id": "dash-rule",
        "kind": "rule",
        "record_id": "rule.always-flags",
        "content": "The fixture command must succeed.",
        "rationale": "A should rule whose check always fails, to exercise flags.",
        "definition": {
            "type": "rule",
            "strength": "should",
            "enforcer": {"kind": "test", "command": "false"},
            "examples": [{"input": "false", "expected": "flag", "reason": "exits non-zero"}]
        }
    });
    // Preview first: nothing is written.
    let mut preview = change.clone();
    preview["preview"] = json!(true);
    let preview = session.command(&preview);
    assert_eq!(preview["state"], "needs_decision", "{preview}");
    assert_eq!(preview["data"]["preview_only"], true);
    assert_eq!(preview["data"]["diff"]["before"], Value::Null);
    assert_eq!(
        preview["data"]["diff"]["after"]["enforcer"],
        json!({"kind": "test", "command": "false"})
    );
    assert_eq!(
        rulebook(&project)["rules"],
        json!([]),
        "a preview writes nothing"
    );

    let drafted = session.command(&change);
    assert_eq!(drafted["state"], "success", "{drafted}");
    assert_eq!(drafted["data"]["revision"], "uncommitted");
    let rule = &rulebook(&project)["rules"][0];
    assert_eq!(rule["id"], "rule.always-flags");
    assert_eq!(rule["strength"], "should");
    assert_eq!(rule["examples"].as_array().map(Vec::len), Some(1));
    // The same change again leaves the file as it is.
    let replayed = session.command(&change);
    assert_eq!(replayed["data"]["written"], false, "{replayed}");

    let accepted = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-rule-accept",
        "accept": true,
        "rationale": "Accepted from the dashboard."
    }));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let log = Command::new("git")
        .args(["log", "-1", "--format=%H%n%B"])
        .current_dir(&project)
        .output()
        .expect("git log");
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(
        log.starts_with(accepted["data"]["commit"].as_str().expect("commit")),
        "{log}"
    );
    assert!(log.contains("Accepted from the dashboard."), "{log}");
    let changelog = session.inspect()["data"]["changelog"].clone();
    assert_eq!(changelog[0]["commit"], accepted["data"]["commit"]);

    // Checks run from the CLI and the Git gates, never from the dashboard.
    let check = session.raw_command(
        &json!({"workflow": "check", "request_id": "dash-check", "mode": "all"}).to_string(),
    );
    assert!(check.starts_with("HTTP/1.1 400"), "{check}");
}

#[test]
fn mutations_are_observed_but_inspections_and_rejected_bodies_are_not() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let sink = Arc::clone(&seen);
    let backend = CommandDashboardBackend::new(temp.path().to_path_buf(), None).observed(Arc::new(
        move |response: &ServiceResponse| {
            sink.lock()
                .expect("observer")
                .push(format!("{}:{}", response.workflow, response.request_id));
        },
    ));
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: true,
        },
        Arc::new(backend),
    )
    .expect("dashboard");
    let session = Session::open(handle);
    session.inspect();
    let host = session.host.clone();
    let filtered = json!({"search": "x"}).to_string();
    send(
        session.handle.address(),
        &format!("POST /api/inspect HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Length: {}\r\n\r\n{filtered}", filtered.len()),
    );
    assert!(
        seen.lock().expect("observer").is_empty(),
        "inspections are not reported"
    );
    let rejected = session.raw_command(r#"{"workflow":"nope"}"#);
    assert!(rejected.starts_with("HTTP/1.1 400"));
    assert!(
        seen.lock().expect("observer").is_empty(),
        "rejected bodies never reach the service"
    );
    session.command(
        &json!({"workflow": "change", "request_id": "observed-change", "kind": "mission", "content": MISSION}),
    );
    assert_eq!(
        seen.lock().expect("observer").as_slice(),
        ["change:observed-change"]
    );
}

#[test]
fn one_accept_without_a_request_id_commits_every_drafted_edit() {
    // The dashboard's accept button sends no request id.
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let session = Session::open(start(temp.path(), true));
    session.command(&json!({"workflow": "change", "kind": "mission", "content": MISSION}));
    for name in ["first", "second"] {
        let drafted = session.command(&json!({
            "workflow": "change",
            "kind": "principle",
            "record_id": format!("principle.custom-{name}"),
            "content": format!("The {name} principle."),
            "rationale": "Two drafts."
        }));
        assert_eq!(drafted["state"], "success", "{drafted}");
    }
    let accepted = session.command(&json!({"workflow": "change", "accept": true}));
    assert_eq!(accepted["state"], "success", "{accepted}");
    assert_eq!(
        accepted["data"]["changes"],
        json!([
            "set the mission",
            "add principle.custom-first",
            "add principle.custom-second"
        ])
    );
    let ids = rulebook(temp.path())["principles"]
        .as_array()
        .expect("principles")
        .iter()
        .map(|entry| entry["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            json!("principle.custom-first"),
            json!("principle.custom-second")
        ]
    );
}

/// One `print(...)` call is a flag for the AST rule below.
const PRINT: &str = "((call function: (identifier) @f) @match (#eq? @f \"print\"))";

fn sweep_findings(root: &Path) -> Vec<Value> {
    let swept = service(ServiceRequest::Check(CheckRequest {
        project_dir: root.to_path_buf(),
        gate_mode: GateMode::Sweep,
        ..Default::default()
    }));
    let text = swept.data["findings_json"]
        .as_str()
        .unwrap_or_else(|| panic!("no findings: {}", swept.summary));
    serde_json::from_str(text).expect("findings.json")
}

#[test]
fn the_rule_form_sets_the_sweep_fields_and_the_sweep_reads_them_back() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("src")).expect("src");
    std::fs::write(temp.path().join("src/app.py"), "print('one')\n").expect("write");
    let session = Session::open(start_for_a_person(temp.path()));
    session.command(&json!({"workflow": "change", "kind": "mission", "content": MISSION}));
    // What the rule form sends: class, priority and one doubt.
    let mut form = json!({
        "workflow": "change",
        "kind": "rule",
        "record_id": "rule.no-print",
        "content": "Library code does not print.",
        "rationale": "Output goes through the logger.",
        "definition": {
            "type": "rule",
            "strength": "should",
            "enforcer": {"kind": "ast", "language": "python", "query": PRINT},
            "paths": ["src/**"],
            "sweep": "confirm_first",
            "sweep_priority": 3,
            "sweep_unsure": [{"paths": ["src/demo/**"], "reason": "demos print on purpose"}]
        }
    });
    let mut preview = form.clone();
    preview["preview"] = json!(true);
    let preview = session.command(&preview);
    assert_eq!(preview["state"], "needs_decision", "{preview}");
    assert_eq!(preview["data"]["diff"]["after"]["sweep"], "confirm_first");
    assert_eq!(preview["data"]["diff"]["after"]["sweep_priority"], 3);
    let drafted = session.command(&form);
    assert_eq!(drafted["state"], "success", "{drafted}");
    let accepted = session.command(&json!({"workflow": "change", "accept": true}));
    assert_eq!(accepted["state"], "success", "{accepted}");

    let view = session.inspect()["data"]["current"].clone();
    let rule = rule_entry(&view, "rule.no-print");
    assert_eq!(rule["sweep_class"], "confirm_first");
    assert_eq!(rule["sweep_priority"], 3);
    assert_eq!(rule["sweep_unsure"][0]["reason"], "demos print on purpose");
    assert_eq!(rule["edit"]["definition"]["sweep"], "confirm_first");
    let findings = sweep_findings(temp.path());
    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0]["class"], "confirm_first");
    assert_eq!(findings[0]["priority"], 3);
    assert_eq!(findings[0]["responsibility"], "rule.no-print");

    // Changing the fields from the form changes what the sweep reports.
    form["definition"]["sweep"] = json!("draft_pr");
    form["definition"]["sweep_priority"] = json!(1);
    let changed = session.command(&form);
    assert_eq!(changed["data"]["diff"]["before"]["sweep"], "confirm_first");
    assert_ne!(
        changed["data"]["digest"]["before"],
        changed["data"]["digest"]["after"]
    );
    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(rule_entry(&view, "rule.no-print")["lifecycle"], "changed");
    session.command(&json!({"workflow": "change", "accept": true}));
    let findings = sweep_findings(temp.path());
    assert_eq!(findings[0]["class"], "draft_pr");
    assert_eq!(findings[0]["priority"], 1);

    // A dashboard started for automation never widens a sweep class.
    let agent = Session::open(start(temp.path(), true));
    form["definition"]["sweep"] = json!("discuss");
    let narrowed = agent.command(&form);
    assert_eq!(narrowed["state"], "success", "{narrowed}");
    form["definition"]["sweep"] = json!("draft_pr");
    let widened = agent.command(&form);
    assert_eq!(widened["state"], "needs_input", "{widened}");
}

#[test]
fn retiring_from_the_dashboard_needs_a_reason_and_the_changelog_says_who_when_and_why() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    draft_rulebook(temp.path());
    let session = Session::open(start(temp.path(), true));
    session.command(&json!({"workflow": "change", "accept": true}));

    let bare = session.command(&json!({"workflow": "change", "retire": "rule.tests-pass"}));
    assert_eq!(bare["state"], "needs_input", "{bare}");
    let reason = "CI runs the tests on every push.";
    for target in ["rule.tests-pass", "principle.custom-evidence"] {
        let request = json!({"workflow": "change", "retire": target, "rationale": reason});
        let mut preview = request.clone();
        preview["preview"] = json!(true);
        let preview = session.command(&preview);
        assert_eq!(preview["state"], "needs_decision", "{preview}");
        assert_eq!(preview["data"]["diff"]["after"], Value::Null);
        let retired = session.command(&request);
        assert_eq!(retired["state"], "success", "{retired}");
    }
    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(
        view["drafts"],
        json!([
            format!("retire principle.custom-evidence: {reason}"),
            format!("retire rule.tests-pass: {reason}"),
        ])
    );
    assert!(view["rules"].as_array().is_some_and(Vec::is_empty));
    assert_eq!(view["principles"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        rulebook(temp.path())["retired"],
        json!([
            {"id": "principle.custom-evidence", "reason": reason},
            {"id": "rule.tests-pass", "reason": reason}
        ])
    );

    let accepted = session.command(&json!({"workflow": "change", "accept": true}));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let body = session.inspect();
    let view = &body["data"]["current"];
    assert_eq!(view["retired"].as_array().map(Vec::len), Some(2));
    assert_eq!(view["retired"][1]["reason"], reason);
    let latest = &body["data"]["changelog"][0];
    assert_eq!(latest["commit"], accepted["data"]["commit"]);
    assert_eq!(latest["owner"], "Dashboard Owner");
    assert!(latest["recorded_at"]
        .as_str()
        .is_some_and(|at| at.ends_with('Z')));
    assert!(
        latest["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains(&format!("Retire rule.tests-pass: {reason}"))),
        "{latest}"
    );
    // A retired entry can be retired once.
    let again = session.command(
        &json!({"workflow": "change", "retire": "rule.tests-pass", "rationale": "Twice."}),
    );
    assert_eq!(again["state"], "needs_input", "{again}");
}

#[test]
fn withdrawing_restores_the_rulebook_as_last_committed() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    draft_rulebook(temp.path());
    let session = Session::open(start(temp.path(), true));
    session.command(&json!({"workflow": "change", "accept": true}));
    let committed = std::fs::read(temp.path().join("whetstone/rulebook.json")).expect("rulebook");

    let edited = session.command(&json!({
        "workflow": "change",
        "kind": "mission",
        "content": "A different mission."
    }));
    assert_eq!(edited["state"], "success", "{edited}");
    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(view["header"]["agreement"], "accepted, with drafts");
    assert_eq!(view["mission"]["lifecycle"], "changed");
    assert_eq!(view["drafts"], json!(["change the mission"]));

    let preview =
        session.command(&json!({"workflow": "change", "withdraw": true, "preview": true}));
    assert_eq!(preview["data"]["discards"], json!(["change the mission"]));
    let withdrawn = session.command(&json!({"workflow": "change", "withdraw": true}));
    assert_eq!(withdrawn["state"], "success", "{withdrawn}");
    assert_eq!(
        std::fs::read(temp.path().join("whetstone/rulebook.json")).expect("rulebook"),
        committed
    );
    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(view["drafts"], json!([]));
    assert_eq!(view["mission"]["title"], MISSION);
    let nothing = session.command(&json!({"workflow": "change", "withdraw": true}));
    assert_eq!(nothing["state"], "needs_input", "{nothing}");
}
