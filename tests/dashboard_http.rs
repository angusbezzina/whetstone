use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use whetstone::dashboard::{BackendResponse, DashboardBackend, DashboardHandle, DashboardMode};
use whetstone::dashboard_service::CommandDashboardBackend;
use whetstone::service::{BasicRequest, CommandService, InitAction, InitRequest, ServiceRequest};

#[derive(Default)]
struct Backend {
    inspections: AtomicUsize,
    mutations: AtomicUsize,
}

impl DashboardBackend for Backend {
    fn inspect(&self) -> BackendResponse {
        self.inspections.fetch_add(1, Ordering::Relaxed);
        BackendResponse::json(
            200,
            &serde_json::json!({"state":"success","source":"shared-service"}),
        )
    }

    fn mutate(&self, body: &[u8]) -> BackendResponse {
        self.mutations.fetch_add(1, Ordering::Relaxed);
        BackendResponse::json(
            200,
            &serde_json::json!({"state":"success","request":String::from_utf8_lossy(body)}),
        )
    }
}

fn send(address: SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address).expect("connect dashboard");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
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

fn response_body(response: &str) -> serde_json::Value {
    serde_json::from_str(
        response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("HTTP response body"),
    )
    .expect("JSON response")
}

fn init_git(root: &Path) {
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .expect("initialize Git fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
    let body = response.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("bootstrap JSON");
    let csrf = json["csrf_token"].as_str().expect("csrf").to_string();
    (cookie, csrf)
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
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 0);

    let read_only = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(read_only.contains("read_only"));
    let edit = send(
        handle.address(),
        &format!("POST /session/edit HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(edit.starts_with("HTTP/1.1 200"));
    let mutation = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 2\r\n\r\n{{}}"),
    );
    assert!(mutation.contains("shared-service") || mutation.contains("success"));
    assert_eq!(backend.mutations.load(Ordering::Relaxed), 1);
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
    let page = send(
        handle.address(),
        &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
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

    for path in ["/app.css", "/app.js"] {
        let asset = send(
            handle.address(),
            &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        );
        assert!(asset.starts_with("HTTP/1.1 200"), "{asset}");
        let body = asset.split_once("\r\n\r\n").expect("asset body").1;
        assert!(body.len() < 16 * 1024, "{path} exceeds the startup budget");
        assert!(
            !body.contains("http://"),
            "{path} has a third-party request"
        );
        assert!(
            !body.contains("https://"),
            "{path} has a third-party request"
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
        backend,
    )
    .expect("hosted inspection server");
    let response = send(
        handle.address(),
        "GET / HTTP/1.1\r\nHost: dash.example.test\r\nX-Forwarded-Proto: https\r\nX-Whetstone-Authenticated: true\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 200"));
    let inspection = send(
        handle.address(),
        "GET /api/inspect HTTP/1.1\r\nHost: dash.example.test\r\nX-Forwarded-Proto: https\r\nX-Whetstone-Authenticated: true\r\n\r\n",
    );
    assert!(inspection.contains("shared-service"));
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
    let expected = CommandService.execute(ServiceRequest::Dash(BasicRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: request_id.clone(),
    }));
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
fn command_backend_preserves_service_stale_rejection_and_fixed_project_scope() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let inspection = CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-stale".into()),
        action: InitAction::Inspect,
        expected_revision: None,
        resume_token: None,
        mission: None,
        values: None,
        philosophy: None,
    }));
    let current_revision = inspection.expected_revision.expect("revision");
    let body = serde_json::json!({
        "workflow": "init",
        "request_id": "dashboard-stale",
        "action": "agree",
        "expected_revision": current_revision,
        "resume_token": "stale-token",
        "mission": "Make project intent inspectable.",
        "values": "Trust evidence over assertion.",
        "philosophy": "Keep deterministic behavior in typed services."
    });
    let direct = CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-stale".into()),
        action: InitAction::Agree,
        expected_revision: Some(current_revision),
        resume_token: Some("stale-token".into()),
        mission: Some("Make project intent inspectable.".into()),
        values: Some("Trust evidence over assertion.".into()),
        philosophy: Some("Keep deterministic behavior in typed services.".into()),
    }));

    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: true,
        },
        Arc::new(CommandDashboardBackend::new(
            temp.path().to_path_buf(),
            None,
        )),
    )
    .expect("dashboard");
    let host = host(&handle);
    let (cookie, csrf) = bootstrap(&handle);
    let edit = send(
        handle.address(),
        &format!("POST /session/edit HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(edit.starts_with("HTTP/1.1 200"), "{edit}");
    let serialized = serde_json::to_string(&body).expect("command JSON");
    let response = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: {}\r\n\r\n{serialized}", serialized.len()),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(
        response_body(&response),
        serde_json::to_value(direct).expect("service JSON")
    );
    assert_eq!(response_body(&response)["state"], "stale");

    let escaped = r#"{"workflow":"check","project_dir":"/","paths":["."]}"#;
    let denied = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: {}\r\n\r\n{escaped}", escaped.len()),
    );
    assert!(denied.starts_with("HTTP/1.1 400"), "{denied}");
    assert_eq!(response_body(&denied)["state"], "unknown");
}
