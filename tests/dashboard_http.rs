//! The dashboard's HTTP service: loopback bootstrap, host, origin, session
//! and CSRF gates, hosted read-only mode, the static shell's security
//! headers, and the JSON API over the same command service the CLI uses
//! (inspection, init agree, rule changes and reviews, checks, flag labels
//! and Beads task queueing). Asset markup is deliberately not asserted here.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use whetstone::beads::RecordStore;
use whetstone::dashboard::{BackendResponse, DashboardBackend, DashboardHandle, DashboardMode};
use whetstone::dashboard_service::CommandDashboardBackend;
use whetstone::domain::{RecordBody, RecordId};
use whetstone::service::{
    CommandService, DashRequest, InitAction, InitRequest, ServiceRequest, ServiceResponse,
    ServiceState,
};
use whetstone::storage::{ProjectLayout, StoreKind};

const MISSION: &str = "Make project intent inspectable.";
const CUSTOM_PRINCIPLE: &str = "Evidence before assertion.";

/// Jev is never reached from these tests; every question is unavailable.
fn jev_offline() {
    std::env::set_var("WHETSTONE_JEV_OFFLINE", "1");
}

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

fn inspect_init(root: &Path, request_id: &str) -> ServiceResponse {
    service(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some(request_id.into()),
        action: InitAction::Inspect,
        ..Default::default()
    }))
}

/// Mission, one pstack principle, one custom principle and the mechanical
/// public-surface starter, recorded through the service directly.
fn install_private_agreement(root: &Path) {
    let inspection = inspect_init(root, "dashboard-agreement");
    let accepted = service(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("dashboard-agreement".into()),
        action: InitAction::Agree,
        expected_revision: inspection.expected_revision,
        resume_token: inspection.resume_token,
        mission: Some(MISSION.into()),
        principles: vec!["prove-it-works".into()],
        custom_principles: vec![CUSTOM_PRINCIPLE.into()],
        starters: vec!["rule.ask-before-public-api".into()],
        ..Default::default()
    }));
    assert_eq!(
        accepted.state,
        ServiceState::NeedsInput,
        "{}",
        accepted.summary
    );
    assert_eq!(accepted.data["progress"]["agreement_complete"], true);
}

fn start(project: &Path, allow_mutations: bool) -> DashboardHandle {
    DashboardHandle::start(
        DashboardMode::Local { allow_mutations },
        Arc::new(CommandDashboardBackend::new(project.to_path_buf(), None)),
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

    /// Send a change twice: once to learn the base revision and token, then
    /// confirmed with them, exactly as the dashboard's editor does.
    fn confirmed_change(&self, body: Value) -> Value {
        let probe = self.command(&body);
        assert_eq!(probe["state"], "needs_input", "{probe}");
        let mut confirmed = body;
        confirmed["expected_revision"] = probe["expected_revision"].clone();
        confirmed["resume_token"] = probe["resume_token"].clone();
        self.command(&confirmed)
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
fn hosted_mode_refuses_unsafe_configuration_and_remains_read_only() {
    let backend = Arc::new(Backend::default());
    let invalid = DashboardHandle::start(
        DashboardMode::Hosted {
            bind: "127.0.0.1:0".parse().expect("address"),
            expected_host: "dash.example.test".into(),
            public_origin: "http://dash.example.test".into(),
            authenticated_tls_boundary: false,
            read_only: false,
        },
        backend.clone(),
    );
    assert!(invalid.is_err());

    let handle = DashboardHandle::start(
        DashboardMode::Hosted {
            bind: "127.0.0.1:0".parse().expect("address"),
            expected_host: "dash.example.test".into(),
            public_origin: "https://dash.example.test".into(),
            authenticated_tls_boundary: true,
            read_only: true,
        },
        backend.clone(),
    )
    .expect("hosted inspection server");
    let response = send(
        handle.address(),
        "GET / HTTP/1.1\r\nHost: dash.example.test\r\nX-Forwarded-Proto: https\r\nX-Whetstone-Authenticated: true\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let inspection = send(
        handle.address(),
        "GET /api/inspect HTTP/1.1\r\nHost: dash.example.test\r\nX-Forwarded-Proto: https\r\nX-Whetstone-Authenticated: true\r\n\r\n",
    );
    assert!(inspection.contains("shared-service"));
    let mutation = send(
        handle.address(),
        "POST /api/command HTTP/1.1\r\nHost: dash.example.test\r\nOrigin: https://dash.example.test\r\nSec-Fetch-Site: same-origin\r\nX-Forwarded-Proto: https\r\nX-Whetstone-Authenticated: true\r\nContent-Length: 2\r\n\r\n{}",
    );
    assert!(!mutation.starts_with("HTTP/1.1 200"), "{mutation}");
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 0);
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
    let view = &body["data"]["current"];
    assert_eq!(view["established"], false, "{view}");
    assert_eq!(view["mission"], Value::Null);
    assert_eq!(view["principles"], json!([]));
    assert_eq!(view["rules"], json!([]));
    let steps = view["onboarding"]["steps"]
        .as_array()
        .expect("onboarding steps")
        .iter()
        .map(|step| step["key"].as_str().expect("key"))
        .collect::<Vec<_>>();
    assert_eq!(
        steps,
        [
            "tools",
            "mission",
            "principles",
            "exemplars",
            "rules",
            "gates"
        ]
    );
    let starters = view["onboarding"]["starters"].as_array().expect("starters");
    let ids = starters
        .iter()
        .map(|starter| starter["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "rule.prove-it-works",
            "rule.ask-before-public-api",
            "rule.tests-only-when-asked"
        ]
    );
    let prove = &starters[0];
    assert_eq!(prove["family"], "mechanical", "Cargo.toml means cargo test");
    assert_eq!(prove["strength"], "must");
    assert_eq!(prove["accepted"], false);
    assert_eq!(starters[2]["family"], "question");
    assert!(
        view["onboarding"]["catalogue"]
            .as_array()
            .expect("catalogue")
            .iter()
            .any(|entry| entry["id"] == "prove-it-works"),
        "the pstack principle catalogue is offered"
    );
    // The removed direction has no place in the view.
    assert!(view.get("stages").is_none(), "stages were removed");
    assert!(
        view["onboarding"].get("decisions").is_none(),
        "the eight decisions were removed"
    );
    // Nothing was written by looking.
    assert!(!ProjectLayout::resolve(temp.path())
        .expect("layout")
        .private_store_exists());
}

#[test]
fn dashboard_init_agree_records_mission_principles_and_starters() {
    jev_offline();
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let session = Session::open(start(temp.path(), true));
    let inspected = session.command(&json!({
        "workflow": "init",
        "request_id": "dash-agree",
        "action": "inspect"
    }));
    assert_eq!(inspected["state"], "needs_input");
    assert_eq!(inspected["data"]["read_only"], true);
    let agree = json!({
        "workflow": "init",
        "request_id": "dash-agree",
        "action": "agree",
        "expected_revision": inspected["expected_revision"],
        "resume_token": inspected["resume_token"],
        "mission": MISSION,
        "principles": ["prove-it-works"],
        "custom_principles": [CUSTOM_PRINCIPLE],
        "starters": ["rule.ask-before-public-api", "rule.tests-only-when-asked"]
    });

    // A dry run writes nothing and lists the exact records.
    let mut dry = agree.clone();
    dry["dry_run"] = json!(true);
    let dry = session.command(&dry);
    assert_eq!(dry["state"], "needs_decision", "{dry}");
    assert_eq!(dry["data"]["dry_run"], true);
    let planned = dry["data"]["records"]
        .as_array()
        .expect("records")
        .iter()
        .map(|record| record["id"].as_str().expect("id").to_string())
        .collect::<Vec<_>>();
    assert!(
        planned.contains(&"mission.project".to_string()),
        "{planned:?}"
    );
    assert!(planned.contains(&"principle.prove-it-works".to_string()));
    assert!(planned.contains(&"rule.ask-before-public-api".to_string()));
    assert!(planned.contains(&"rule.tests-only-when-asked".to_string()));
    assert!(!ProjectLayout::resolve(temp.path())
        .expect("layout")
        .private_store_exists());

    let agreed = session.command(&agree);
    assert_eq!(agreed["state"], "needs_input", "{agreed}");
    assert_eq!(agreed["data"]["progress"]["agreement_complete"], true);
    assert_eq!(agreed["data"]["shared"], false);
    assert_eq!(agreed["data"]["records"].as_array().map(Vec::len), Some(5));
    // Replaying the exact answer is idempotent.
    let replay = session.command(&agree);
    assert_eq!(
        replay["data"]["records"], agreed["data"]["records"],
        "{replay}"
    );

    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(view["established"], true, "{view}");
    assert_eq!(view["mission"]["title"], MISSION);
    assert_eq!(view["mission"]["kind"], "mission");
    let principles = view["principles"].as_array().expect("principles");
    assert_eq!(principles.len(), 2, "{principles:?}");
    assert!(principles
        .iter()
        .any(|entry| entry["id"] == "principle.prove-it-works"));
    assert!(principles
        .iter()
        .any(|entry| entry["title"] == CUSTOM_PRINCIPLE));
    let api = rule_entry(&view, "rule.ask-before-public-api");
    assert_eq!(api["kind"], "rule");
    assert_eq!(api["strength"], "must");
    assert_eq!(api["family"], "mechanical");
    assert_eq!(api["shadow"], false);
    assert_eq!(api["hand_raise"], json!(["flag"]));
    let tests = rule_entry(&view, "rule.tests-only-when-asked");
    assert_eq!(tests["strength"], "should");
    assert_eq!(tests["family"], "question");
    assert_eq!(tests["shadow"], true, "a starter question starts in shadow");
    assert_eq!(tests["local_only"], false);
    assert!(tests["examples"]
        .as_array()
        .is_some_and(|examples| examples.len() >= 2));
    let starters = view["onboarding"]["starters"].as_array().expect("starters");
    let accepted = |id: &str| {
        starters
            .iter()
            .find(|starter| starter["id"] == id)
            .map(|starter| starter["accepted"].clone())
    };
    assert_eq!(accepted("rule.ask-before-public-api"), Some(json!(true)));
    assert_eq!(accepted("rule.prove-it-works"), Some(json!(false)));

    // A second mission through init is refused: it is a change now.
    let again = session.command(&json!({
        "workflow": "init",
        "request_id": "dash-agree-2",
        "action": "inspect"
    }));
    let second = session.command(&json!({
        "workflow": "init",
        "request_id": "dash-agree-2",
        "action": "agree",
        "expected_revision": again["expected_revision"],
        "resume_token": again["resume_token"],
        "mission": "Another mission."
    }));
    assert_eq!(second["state"], "needs_decision", "{second}");
    let view = session.inspect()["data"]["current"].clone();
    assert_eq!(view["mission"]["title"], MISSION);
}

#[test]
fn command_backend_preserves_service_stale_rejection_and_fixed_project_scope() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let inspection = inspect_init(temp.path(), "dashboard-stale");
    let current_revision = inspection.expected_revision.expect("revision");
    let body = json!({
        "workflow": "init",
        "request_id": "dashboard-stale",
        "action": "agree",
        "expected_revision": current_revision,
        "resume_token": "stale-token",
        "mission": MISSION,
        "principles": ["prove-it-works"],
        "starters": ["all"]
    });
    let direct = service(ServiceRequest::Init(InitRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-stale".into()),
        action: InitAction::Agree,
        expected_revision: Some(current_revision),
        resume_token: Some("stale-token".into()),
        mission: Some(MISSION.into()),
        principles: vec!["prove-it-works".into()],
        starters: vec!["all".into()],
        ..Default::default()
    }));
    let session = Session::open(start(temp.path(), true));
    let response = session.command(&body);
    assert_eq!(
        response,
        serde_json::to_value(direct).expect("service JSON")
    );
    assert_eq!(response["state"], "stale");
    assert!(
        !ProjectLayout::resolve(temp.path())
            .expect("layout")
            .private_store_exists(),
        "a stale answer writes nothing, not even an empty store"
    );

    // Request bodies cannot name another project, a removed workflow, a
    // removed init field or a removed change kind.
    for rejected in [
        r#"{"workflow":"check","project_dir":"/","paths":["."]}"#,
        r#"{"workflow":"pull"}"#,
        r#"{"workflow":"init","action":"agree","values":"Evidence."}"#,
        r#"{"workflow":"init","action":"agree","desired_outcome":"Less rework."}"#,
        r#"{"workflow":"init","action":"wire"}"#,
        r#"{"workflow":"change","kind":"value","record_id":"value.core"}"#,
        r#"{"workflow":"change","kind":"exception","record_id":"exception.one"}"#,
        r#"{"workflow":"change","flag":{"receipt":"x","verdict":"maybe"}}"#,
        r#"{"workflow":"check","mode":"repair"}"#,
        r#"{"workflow":"task","title":"No description"}"#,
        "not json",
    ] {
        let denied = session.raw_command(rejected);
        assert!(denied.starts_with("HTTP/1.1 400"), "{rejected}: {denied}");
        assert_eq!(response_body(&denied)["state"], "unknown", "{rejected}");
    }
}

#[test]
fn dashboard_init_inspection_is_identical_to_the_cli_service_path() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("manifest fixture");
    let direct = inspect_init(temp.path(), "dashboard-inspect");
    let session = Session::open(start(temp.path(), true));
    let response = session.command(&json!({
        "workflow": "init",
        "request_id": "dashboard-inspect",
        "action": "inspect"
    }));
    assert_eq!(
        response,
        serde_json::to_value(direct).expect("service JSON")
    );
    assert!(!temp.path().join(".git/whetstone").exists());
}

#[test]
fn private_only_dashboard_queries_are_typed_filtered_and_do_not_create_shareable_state() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let layout = ProjectLayout::resolve(temp.path()).expect("project layout");
    assert!(layout.shared_store().is_none());

    let direct = service(ServiceRequest::Dash(DashRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-filter".into()),
        search: Some(CUSTOM_PRINCIPLE.into()),
        as_of: Some("2099-01-01T00:00:00Z".into()),
        page_size: 7,
        expected_snapshot: None,
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
    let page = &body["data"]["history"]["decision_history"]["items"];
    assert_eq!(page.as_array().map(Vec::len), Some(1), "{page}");
    assert_eq!(page[0]["record"]["record_type"], "principle", "{page}");
    assert_eq!(
        page[0]["record"]["record"]["statement"], CUSTOM_PRINCIPLE,
        "{page}"
    );
    let current = &body["data"]["current"];
    assert_eq!(
        current["mission"]["title"], MISSION,
        "filtered history must never replace the independent current-state projection"
    );
    assert_eq!(current["principles"].as_array().map(Vec::len), Some(2));
    assert_eq!(current["rules"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        current["checks"]["last_complete"],
        Value::Null,
        "no gate has run, so no completed check may be claimed"
    );
    assert_eq!(
        current["header"]["visibility"], "private",
        "{}",
        current["header"]
    );
    assert!(layout.shared_store().is_none());
}

#[test]
fn dashboard_history_query_rejects_unknown_fields_and_marks_invalid_time_unknown() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let handle = start(temp.path(), false);
    let host = host(&handle);
    for (body, expected_status) in [
        (json!({"project_dir": "/"}), "HTTP/1.1 400"),
        (json!({"history_after": "cursor"}), "HTTP/1.1 400"),
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
            assert_eq!(
                response_body(&response)["data"]["history_state"],
                "unavailable"
            );
        } else {
            assert_eq!(response_body(&response)["state"], "unknown");
        }
    }
}

#[test]
fn a_rule_change_is_drafted_accepted_checked_flagged_and_dismissed() {
    jev_offline();
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
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

    // A rule needs its definition: one strength and one enforcer.
    let missing_definition = session.confirmed_change(json!({
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
                "enforcers": [{"kind": "review", "reviewer": {"by": "interrogate"}}]
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
    let probe = session.command(&change);
    assert_eq!(probe["state"], "needs_input");
    let mut preview = change.clone();
    preview["expected_revision"] = probe["expected_revision"].clone();
    preview["resume_token"] = probe["resume_token"].clone();
    preview["preview"] = json!(true);
    let preview = session.command(&preview);
    assert_eq!(preview["state"], "needs_decision", "{preview}");
    assert_eq!(preview["data"]["preview_only"], true);
    assert_eq!(preview["data"]["diff"]["before"], Value::Null);
    assert_eq!(preview["data"]["diff"]["after"]["record_type"], "rule");
    assert_eq!(
        preview["data"]["diff"]["after"]["record"]["schema"],
        "whetstone.rule.v2"
    );
    assert_eq!(
        preview["data"]["diff"]["after"]["record"]["enforcer"],
        json!({"kind": "test", "command": "false"})
    );
    let view = session.inspect()["data"]["current"].clone();
    assert!(
        !view["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .any(|rule| rule["id"] == "rule.always-flags"),
        "a preview writes nothing"
    );

    let drafted = session.confirmed_change(change.clone());
    assert_eq!(drafted["state"], "success", "{drafted}");
    assert_eq!(drafted["data"]["shared"], false);
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal id")
        .to_string();
    // A draft is not in force: it shows as pending.
    let view = session.inspect()["data"]["current"].clone();
    let pending = rule_entry(&view, "rule.always-flags");
    assert_eq!(pending["lifecycle"], "draft", "{pending}");
    assert_eq!(pending["stats"]["checks"], 0);
    // The same change replayed is the same draft.
    let replayed = session.confirmed_change(change);
    assert_eq!(replayed["data"]["idempotent_replay"], true, "{replayed}");

    let accepted = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-rule-accept",
        "review": {"proposal": proposal, "verdict": "accept"},
        "rationale": "Accepted from the dashboard."
    }));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let view = session.inspect()["data"]["current"].clone();
    let rule = rule_entry(&view, "rule.always-flags");
    assert_eq!(rule["strength"], "should");
    assert_eq!(rule["family"], "mechanical");
    assert_eq!(rule["lifecycle"], "accepted", "{rule}");
    assert_eq!(rule["examples"].as_array().map(Vec::len), Some(1));

    // A check through the dashboard runs every in-force rule; a should-rule
    // failure is a flag, not a failed check.
    let check = session.command(&json!({
        "workflow": "check",
        "request_id": "dash-check",
        "mode": "all"
    }));
    assert_ne!(check["state"], "violated", "{}", check["summary"]);
    assert_eq!(check["data"]["selection"]["mode"], "all", "{check}");
    let selected = check["data"]["selection"]["rules"]
        .as_array()
        .expect("rules");
    assert!(
        selected.contains(&json!("rule.always-flags")),
        "{selected:?}"
    );
    assert!(
        selected.contains(&json!("rule.ask-before-public-api")),
        "{selected:?}"
    );
    assert_eq!(
        check["data"]["flags"],
        json!(["rule.always-flags"]),
        "{}",
        check["data"]
    );
    let gate = check["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == "rule.always-flags")
        .expect("the should rule ran")
        .clone();
    assert_eq!(gate["state"], "fail");
    assert_eq!(gate["strength"], "should");
    assert_eq!(gate["family"], "mechanical");
    let receipt = gate_receipt_for(&project, &check, "rule.always-flags");
    assert!(receipt.starts_with("verification.gate_"), "{receipt}");

    // A label needs a reason, and only a flagging receipt can be labelled.
    let no_reason = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-flag-empty",
        "flag": {"receipt": receipt, "verdict": "dismiss"}
    }));
    assert_eq!(no_reason["state"], "needs_input", "{no_reason}");
    let unknown = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-flag-unknown",
        "flag": {"receipt": "verification.gate_missing", "verdict": "accept"},
        "rationale": "No such receipt."
    }));
    assert_eq!(unknown["state"], "unknown", "{unknown}");

    let dismissed = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-flag",
        "flag": {"receipt": receipt, "verdict": "dismiss"},
        "rationale": "The fixture fails on purpose."
    }));
    assert_eq!(dismissed["state"], "success", "{dismissed}");
    assert_eq!(dismissed["data"]["rule"], "rule.always-flags");
    assert_eq!(dismissed["data"]["verdict"], "dismiss");
    let view = session.inspect()["data"]["current"].clone();
    let stats = &rule_entry(&view, "rule.always-flags")["stats"];
    assert_eq!(stats["flags"], 1, "{stats}");
    assert_eq!(stats["dismissed"], 1, "{stats}");
    assert_eq!(stats["accepted"], 0, "{stats}");
    assert_eq!(stats["undecided"], 0, "{stats}");
    assert_eq!(stats["false_flag_rate"], "100.0%", "{stats}");

    // A second check flags again; accepting that flag is counted too.
    let again = session.command(&json!({
        "workflow": "check",
        "request_id": "dash-check-2"
    }));
    let second = gate_receipt_for(&project, &again, "rule.always-flags");
    assert_ne!(second, receipt);
    let accepted = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-flag-2",
        "flag": {"receipt": second, "verdict": "accept"},
        "rationale": "Right this time."
    }));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let view = session.inspect()["data"]["current"].clone();
    let stats = &rule_entry(&view, "rule.always-flags")["stats"];
    assert_eq!(
        (
            stats["flags"].clone(),
            stats["accepted"].clone(),
            stats["dismissed"].clone()
        ),
        (json!(2), json!(1), json!(1)),
        "{stats}"
    );
}

/// The gate receipt a check persisted for one rule, read from the private
/// store by its `gate:<rule>` subject.
fn gate_receipt_for(project: &Path, check: &Value, rule: &str) -> String {
    let layout = ProjectLayout::resolve(project).expect("layout");
    let store = RecordStore::open_existing(&layout.private_store(), StoreKind::Private)
        .expect("private store");
    let receipts = check["data"]["gate_receipts"]
        .as_array()
        .expect("gate receipts");
    let matching = receipts
        .iter()
        .filter_map(|reference| {
            let id = reference["id"].as_str()?;
            let record = store.latest(&RecordId::new(id).ok()?).ok().flatten()?;
            match record.body {
                RecordBody::VerificationReceipt(body)
                    if body.subject.stable_id == format!("gate:{rule}") =>
                {
                    Some(id.to_string())
                }
                _ => None,
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1, "one receipt for {rule} in {receipts:?}");
    matching[0].clone()
}

#[test]
fn a_check_of_only_should_rules_still_reports_its_flags() {
    // "State = must rules only; should-rule failures are listed in
    // data.flags and do not fail the check" holds even when the selection
    // has no must rule at all.
    jev_offline();
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let session = Session::open(start(temp.path(), true));
    let drafted = session.confirmed_change(json!({
        "workflow": "change",
        "request_id": "dash-should-only",
        "kind": "rule",
        "record_id": "rule.should-only",
        "content": "The fixture command must succeed.",
        "rationale": "A should rule whose check always fails.",
        "definition": {"type": "rule", "strength": "should", "enforcer": {"kind": "test", "command": "false"}}
    }));
    let proposal = drafted["data"]["proposal"]["id"].clone();
    let accepted = session.command(&json!({
        "workflow": "change",
        "request_id": "dash-should-only-accept",
        "review": {"proposal": proposal, "verdict": "accept"}
    }));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let check = session.command(&json!({
        "workflow": "check",
        "request_id": "dash-should-only-check",
        "rules": ["rule.should-only"]
    }));
    assert_ne!(check["state"], "violated", "{check}");
    assert_eq!(
        check["data"]["flags"],
        json!(["rule.should-only"]),
        "{check}"
    );
}

#[test]
fn check_modes_reach_the_service_as_changed_staged_and_sweep() {
    jev_offline();
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let session = Session::open(start(temp.path(), true));
    for mode in ["all", "changed", "staged", "sweep"] {
        let check = session.command(&json!({
            "workflow": "check",
            "request_id": format!("dash-mode-{mode}"),
            "mode": mode
        }));
        assert_eq!(check["workflow"], "check", "{mode}");
        assert_eq!(
            check["data"]["selection"]["mode"], mode,
            "{mode}: {}",
            check["data"]
        );
    }
    // Staged runs only content checks: the public-surface starter qualifies.
    let staged = session.command(&json!({
        "workflow": "check",
        "request_id": "dash-mode-staged-2",
        "mode": "staged"
    }));
    let rules = staged["data"]["selection"]["rules"]
        .as_array()
        .expect("rules");
    assert!(
        rules
            .iter()
            .all(|rule| rule == "rule.ask-before-public-api"),
        "{rules:?}"
    );
}

#[test]
fn mutations_are_observed_but_inspections_and_rejected_bodies_are_not() {
    jev_offline();
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
    session
        .command(&json!({"workflow": "init", "request_id": "observed-init", "action": "inspect"}));
    assert_eq!(
        seen.lock().expect("observer").as_slice(),
        ["init:observed-init"]
    );
}

fn bd_available() -> bool {
    Command::new("bd")
        .arg("version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn a_task_is_queued_in_the_repositorys_beads_tracker() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let session = Session::open(start(temp.path(), true));
    // Without a tracker the task is honestly unavailable.
    let unavailable = session.command(&json!({
        "workflow": "task",
        "title": "Tidy the parser",
        "description": "Queued from the dashboard."
    }));
    assert_eq!(unavailable["state"], "unavailable", "{unavailable}");
    let too_long = session.command(&json!({
        "workflow": "task",
        "title": "x".repeat(201),
        "description": ""
    }));
    assert_eq!(too_long["state"], "needs_input", "{too_long}");
    if !bd_available() {
        assert!(
            std::env::var_os("CI").is_none(),
            "a tool this test needs is missing in CI"
        );
        eprintln!("bd is not installed; skipping the queued-task half");
        return;
    }
    // A throwaway tracker in the temporary repository, never this one's.
    let output = Command::new("bd")
        .args([
            "init",
            "--non-interactive",
            "--skip-agents",
            "--skip-hooks",
            "-p",
            "tsk",
            "-q",
        ])
        .current_dir(temp.path())
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect("bd init");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let queued = session.command(&json!({
        "workflow": "task",
        "title": "Tidy the parser",
        "description": "Queued from the dashboard."
    }));
    assert_eq!(queued["state"], "success", "{queued}");
    let issue = queued["data"]["issue"]
        .as_str()
        .expect("issue id")
        .to_string();
    assert!(issue.starts_with("tsk-"), "{issue}");
    let shown = Command::new("bd")
        .args(["show", &issue, "--json"])
        .current_dir(temp.path())
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect("bd show");
    let text = String::from_utf8_lossy(&shown.stdout);
    let value: Value =
        serde_json::from_str(&text[text.find(['[', '{']).expect("json")..]).expect("bd JSON");
    let value = value
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(value);
    assert_eq!(value["title"], "Tidy the parser");
    assert_eq!(value["issue_type"], "task");
    assert!(
        value["labels"].to_string().contains("whetstone-task"),
        "{value}"
    );
}

#[test]
fn evidence_is_served_only_from_inside_the_evidence_root() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let layout = ProjectLayout::resolve(temp.path()).expect("layout");
    let root = whetstone::gates::evidence_root(&layout);
    std::fs::create_dir_all(root.join("run1")).expect("run dir");
    std::fs::write(root.join("run1/shot.png"), b"\x89PNG proof").expect("shot");
    std::fs::write(root.join("run1/gate.log"), b"passed").expect("log");
    std::fs::write(root.join("run1/tool.sh"), b"#!/bin/sh").expect("script");
    let outside = temp.path().join("secret.png");
    std::fs::write(&outside, b"private").expect("outside");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("run1/link.png")).expect("symlink");
    let backend = CommandDashboardBackend::new(temp.path().to_path_buf(), None);
    let shot = backend.evidence("run1/shot.png");
    assert_eq!((shot.status, shot.content_type), (200, "image/png"));
    assert_eq!(shot.body, b"\x89PNG proof");
    assert_eq!(backend.evidence("run1/gate.log").status, 200);
    for refused in [
        "run1/tool.sh",
        "../secret.png",
        "run1/../../secret.png",
        "/etc/passwd.txt",
        "run1/link.png",
        "run1/missing.png",
        "run1/%2e%2e/secret.png",
    ] {
        assert_eq!(
            backend.evidence(refused).status,
            404,
            "{refused} was served"
        );
    }
    // The same boundary holds over HTTP.
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(backend),
    )
    .expect("dashboard");
    let host = host(&handle);
    let served = send(
        handle.address(),
        &format!("GET /api/evidence/run1/gate.log HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    assert!(
        served.starts_with("HTTP/1.1 200") && served.ends_with("passed"),
        "{served}"
    );
    let refused = send(
        handle.address(),
        &format!("GET /api/evidence/run1/link.png HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    assert!(refused.starts_with("HTTP/1.1 404"), "{refused}");
}

#[test]
fn accepting_two_drafts_without_request_ids_accepts_both() {
    // The dashboard's review buttons send no request id; each review must
    // still decide its own draft.
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let session = Session::open(start(temp.path(), true));
    let mut proposals = Vec::new();
    for name in ["first", "second"] {
        let drafted = session.confirmed_change(json!({
            "workflow": "change",
            "request_id": format!("dash-two-{name}"),
            "kind": "principle",
            "record_id": format!("principle.custom-{name}"),
            "content": format!("The {name} principle."),
            "rationale": "Two drafts."
        }));
        assert_eq!(drafted["state"], "success", "{drafted}");
        proposals.push(drafted["data"]["proposal"]["id"].clone());
    }
    for proposal in proposals {
        let reviewed = session.command(&json!({
            "workflow": "change",
            "review": {"proposal": proposal, "verdict": "accept"}
        }));
        assert_eq!(reviewed["state"], "success", "{reviewed}");
        assert_eq!(reviewed["data"]["proposal"]["id"], proposal, "{reviewed}");
    }
    let view = session.inspect()["data"]["current"].clone();
    for id in ["principle.custom-first", "principle.custom-second"] {
        let entry = view["principles"]
            .as_array()
            .expect("principles")
            .iter()
            .find(|entry| entry["id"] == id)
            .unwrap_or_else(|| panic!("{id} missing"));
        assert_eq!(entry["lifecycle"], "accepted", "{entry}");
    }
}
