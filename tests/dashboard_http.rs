use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use whetstone::dashboard::{BackendResponse, DashboardBackend, DashboardHandle, DashboardMode};
use whetstone::dashboard_service::CommandDashboardBackend;
use whetstone::service::{
    ChangeKind, ChangeRequest, CheckRequest, CommandService, DashRequest, InitAction, InitRequest,
    ServiceRequest,
};
use whetstone::storage::{ProjectLayout, StoreKind};

#[derive(Default)]
struct Backend {
    inspections: AtomicUsize,
    mutations: AtomicUsize,
}

impl DashboardBackend for Backend {
    fn inspect(&self, _request_body: &[u8]) -> BackendResponse {
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

fn install_private_agreement(root: &Path) {
    let inspection = CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("dashboard-agreement".into()),
        action: InitAction::Inspect,
        expected_revision: None,
        resume_token: None,
        mission: None,
        desired_outcome: None,
        values: None,
        philosophy: None,
        owner: None,
        initial_safeguard: None,
        safeguard_scope: None,
        revision_triggers: None,
    }));
    let accepted = CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: root.to_path_buf(),
        request_id: Some("dashboard-agreement".into()),
        action: InitAction::Agree,
        expected_revision: inspection.expected_revision,
        resume_token: inspection.resume_token,
        mission: Some("Make project intent inspectable.".into()),
        desired_outcome: Some("Reduce avoidable rework.".into()),
        values: Some("Evidence before assertion.".into()),
        philosophy: Some("Use narrow deterministic boundaries.".into()),
        owner: Some("Platform lead".into()),
        initial_safeguard: Some("Never weaken a failing check to get green.".into()),
        safeguard_scope: Some("All repository changes".into()),
        revision_triggers: Some("Mission, architecture, or repeated friction".into()),
    }));
    assert_eq!(
        accepted.state,
        whetstone::service::ServiceState::NeedsDecision
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

    let mut asset_bytes = page
        .split_once("\r\n\r\n")
        .expect("dashboard HTML body")
        .1
        .len();
    for path in ["/app.css", "/app.js"] {
        let asset = send(
            handle.address(),
            &format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n"),
        );
        assert!(asset.starts_with("HTTP/1.1 200"), "{asset}");
        let body = asset.split_once("\r\n\r\n").expect("asset body").1;
        assert!(body.len() < 24 * 1024, "{path} exceeds the startup budget");
        asset_bytes += body.len();
        assert!(
            !body.contains("http://"),
            "{path} has a third-party request"
        );
        assert!(
            !body.contains("https://"),
            "{path} has a third-party request"
        );
    }
    assert!(
        asset_bytes < 24 * 1024,
        "complete dashboard HTML, CSS, and JavaScript exceed 24 KiB"
    );
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
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
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
    let expected = CommandService.execute(ServiceRequest::Dash(DashRequest::basic(
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
        desired_outcome: None,
        values: None,
        philosophy: None,
        owner: None,
        initial_safeguard: None,
        safeguard_scope: None,
        revision_triggers: None,
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
        desired_outcome: Some("Reduce avoidable rework.".into()),
        values: Some("Trust evidence over assertion.".into()),
        philosophy: Some("Keep deterministic behavior in typed services.".into()),
        owner: Some("Platform lead".into()),
        initial_safeguard: Some("Never weaken a failing gate to get green.".into()),
        safeguard_scope: Some("All repository changes".into()),
        revision_triggers: Some("Mission or architecture changes".into()),
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

#[test]
fn dashboard_init_inspection_is_identical_to_the_cli_service_path() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    std::fs::write(
        temp.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("manifest fixture");
    let direct = CommandService.execute(ServiceRequest::Init(InitRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-inspect".into()),
        action: InitAction::Inspect,
        expected_revision: None,
        resume_token: None,
        mission: None,
        desired_outcome: None,
        values: None,
        philosophy: None,
        owner: None,
        initial_safeguard: None,
        safeguard_scope: None,
        revision_triggers: None,
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
    let body = serde_json::json!({
        "workflow": "init",
        "request_id": "dashboard-inspect",
        "action": "inspect"
    });
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
    assert!(!temp.path().join(".git/whetstone").exists());
}

#[test]
fn private_only_dashboard_queries_are_typed_filtered_and_do_not_create_shareable_state() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let layout = ProjectLayout::resolve(temp.path(), None).expect("project layout");
    assert!(!layout.store_path(StoreKind::Shareable).exists());

    let direct = CommandService.execute(ServiceRequest::Dash(DashRequest {
        project_dir: temp.path().to_path_buf(),
        request_id: Some("dashboard-filter".into()),
        search: Some("Evidence before assertion".into()),
        as_of: Some("2099-01-01T00:00:00Z".into()),
        history_after: None,
        page_size: 7,
        expected_snapshot: None,
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
    let body = serde_json::json!({
        "search": "Evidence before assertion",
        "as_of": "2099-01-01T00:00:00Z",
        "page_size": 7
    });
    let serialized = serde_json::to_string(&body).expect("query JSON");
    let response = send(
        handle.address(),
        &format!(
            "POST /api/inspect HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}",
            serialized.len()
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert_eq!(
        response_body(&response),
        serde_json::to_value(direct).expect("service JSON")
    );
    let body = response_body(&response);
    let page = &body["data"]["history"]["decision_history"]["items"];
    assert_eq!(page.as_array().map(Vec::len), Some(1));
    assert_eq!(page[0]["record"]["record_type"], "core_value", "{page}");
    assert_eq!(
        body["data"]["current"]["local_agreement"]["mission"]["record"]["statement"],
        "Make project intent inspectable.",
        "filtered history must never replace the independent current-state projection"
    );
    assert_eq!(
        body["data"]["current"]["workspace"]["verification_currentness"],
        "unknown_without_exact_recheck"
    );
    assert!(!layout.store_path(StoreKind::Shareable).exists());
}

#[test]
fn dashboard_history_query_rejects_unknown_fields_and_marks_invalid_time_unknown() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(CommandDashboardBackend::new(
            temp.path().to_path_buf(),
            None,
        )),
    )
    .expect("dashboard");
    let host = host(&handle);
    for (body, expected_status) in [
        (serde_json::json!({"project_dir": "/"}), "HTTP/1.1 400"),
        (serde_json::json!({"as_of": "not-a-time"}), "HTTP/1.1 200"),
    ] {
        let serialized = serde_json::to_string(&body).expect("query JSON");
        let response = send(
            handle.address(),
            &format!(
                "POST /api/inspect HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}",
                serialized.len()
            ),
        );
        assert!(response.starts_with(expected_status), "{response}");
        if body.get("as_of").is_some() {
            assert_eq!(response_body(&response)["state"], "unknown");
            assert_eq!(
                response_body(&response)["data"]["history_state"],
                "unavailable"
            );
        }
    }
}

#[test]
fn dashboard_assets_expose_five_accessible_views_exact_review_and_safe_rendering() {
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: false,
        },
        Arc::new(Backend::default()),
    )
    .expect("dashboard");
    let host = host(&handle);
    let html = send(
        handle.address(),
        &format!("GET / HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    let html = html.split_once("\r\n\r\n").expect("HTML body").1;
    assert!(html.contains("role=tabpanel"));
    for view in ["onepager", "workspace", "setup", "workflows", "decisions"] {
        assert!(html.contains(&format!("id={view}")), "missing {view}");
    }
    assert_eq!(html.matches("role=tab ").count(), 5);
    assert!(html.contains("aria-live=polite"));
    assert!(html.contains("review-dialog"));
    assert!(html.contains("Print one-pager"));

    let script = send(
        handle.address(),
        &format!("GET /app.js HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    let script = script.split_once("\r\n\r\n").expect("script body").1;
    assert!(script.contains("Object.freeze"));
    assert!(script.contains("ArrowLeft"));
    assert!(script.contains("returnFocus"));
    assert!(script.contains("textContent"));
    assert!(!script.contains("innerHTML"));
    assert!(script.contains("sessionStorage.getItem(\"whetstone_csrf\")"));

    let css = send(
        handle.address(),
        &format!("GET /app.css HTTP/1.1\r\nHost: {host}\r\n\r\n"),
    );
    let css = css.split_once("\r\n\r\n").expect("CSS body").1;
    for width in ["1024px", "768px", "390px", "320px"] {
        assert!(css.contains(width), "missing {width} breakpoint");
    }
    assert!(css.contains("@media print"));
}

#[test]
fn dashboard_change_check_conflict_and_permission_paths_preserve_service_semantics() {
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    install_private_agreement(temp.path());
    let project = temp.path().to_path_buf();
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: true,
        },
        Arc::new(CommandDashboardBackend::new(project.clone(), None)),
    )
    .expect("dashboard");
    let host = host(&handle);

    let unauthenticated = serde_json::json!({
        "workflow": "change",
        "request_id": "dashboard-change-probe",
        "record_id": "guidance.local"
    });
    let serialized = serde_json::to_string(&unauthenticated).expect("command JSON");
    let denied = send(
        handle.address(),
        &format!(
            "POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nContent-Length: {}\r\n\r\n{serialized}",
            serialized.len()
        ),
    );
    assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");

    let (cookie, csrf) = bootstrap(&handle);
    let edit = send(
        handle.address(),
        &format!("POST /session/edit HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Length: 0\r\n\r\n"),
    );
    assert!(edit.starts_with("HTTP/1.1 200"), "{edit}");

    let direct_probe = CommandService.execute(ServiceRequest::Change(ChangeRequest {
        project_dir: project.clone(),
        request_id: Some("dashboard-change-probe".into()),
        kind: None,
        record_id: Some("guidance.local".into()),
        content: None,
        rationale: None,
        source: None,
        expected_effect: None,
        impact: None,
        examples: vec![],
        conflicts: vec![],
        expected_revision: None,
        resume_token: None,
        preview: false,
    }));
    let probe = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}", serialized.len()),
    );
    assert_eq!(
        response_body(&probe),
        serde_json::to_value(&direct_probe).expect("service JSON")
    );

    let accepted_request = ChangeRequest {
        project_dir: project.clone(),
        request_id: Some("dashboard-change-exact".into()),
        kind: Some(ChangeKind::Guidance),
        record_id: Some("guidance.local".into()),
        content: Some("Use typed boundaries.".into()),
        rationale: Some("Keep responsibilities explicit.".into()),
        source: Some("owner:dashboard".into()),
        expected_effect: Some("Fewer accidental dependencies.".into()),
        impact: Some("Local engineering guidance.".into()),
        examples: vec!["Use the shared service.".into()],
        conflicts: vec![],
        expected_revision: Some(0),
        resume_token: Some(
            CommandService
                .execute(ServiceRequest::Change(ChangeRequest {
                    project_dir: project.clone(),
                    request_id: Some("dashboard-change-exact".into()),
                    kind: None,
                    record_id: Some("guidance.local".into()),
                    content: None,
                    rationale: None,
                    source: None,
                    expected_effect: None,
                    impact: None,
                    examples: vec![],
                    conflicts: vec![],
                    expected_revision: None,
                    resume_token: None,
                    preview: false,
                }))
                .resume_token
                .expect("change token"),
        ),
        preview: false,
    };
    let preview_request = ChangeRequest {
        preview: true,
        ..accepted_request.clone()
    };
    let preview = CommandService.execute(ServiceRequest::Change(preview_request));
    assert_eq!(
        preview.state,
        whetstone::service::ServiceState::NeedsDecision
    );
    assert_eq!(preview.data["preview_only"], true);
    assert_eq!(preview.data["base_revision"], 0);
    assert_eq!(preview.data["diff"]["before"], serde_json::Value::Null);
    assert_eq!(preview.data["diff"]["after"]["record_type"], "guidance");
    let layout = ProjectLayout::resolve(&project, None).expect("layout");
    let private = whetstone::storage::DoltRepository::open_existing(
        &layout.store_path(StoreKind::Private),
        StoreKind::Private,
    )
    .expect("private store");
    assert!(!private
        .all_records()
        .expect("records after preview")
        .iter()
        .any(|record| {
            matches!(
                record.id.as_str(),
                "guidance.local" | "proposal.change.dashboard-change-exact"
            )
        }));
    let accepted = CommandService.execute(ServiceRequest::Change(accepted_request.clone()));
    assert_eq!(accepted.state, whetstone::service::ServiceState::Success);
    let conflicting = ChangeRequest {
        content: Some("Use implicit global boundaries.".into()),
        ..accepted_request
    };
    let direct_conflict = CommandService.execute(ServiceRequest::Change(conflicting.clone()));
    assert_eq!(
        direct_conflict.state,
        whetstone::service::ServiceState::Conflict
    );
    let body = serde_json::json!({
        "workflow": "change",
        "request_id": conflicting.request_id,
        "kind": "guidance",
        "record_id": conflicting.record_id,
        "content": conflicting.content,
        "rationale": conflicting.rationale,
        "source": conflicting.source,
        "expected_effect": conflicting.expected_effect,
        "impact": conflicting.impact,
        "examples": conflicting.examples,
        "conflicts": conflicting.conflicts,
        "expected_revision": conflicting.expected_revision,
        "resume_token": conflicting.resume_token,
    });
    let serialized = serde_json::to_string(&body).expect("conflict JSON");
    let conflict = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}", serialized.len()),
    );
    assert_eq!(
        response_body(&conflict),
        serde_json::to_value(direct_conflict).expect("service conflict JSON")
    );

    let direct_check = CommandService.execute(ServiceRequest::Check(CheckRequest {
        project_dir: project,
        request_id: Some("dashboard-check-parity".into()),
        paths: vec![Path::new(".").to_path_buf()],
        language: None,
        rules: vec![],
    }));
    let body = serde_json::json!({
        "workflow": "check",
        "request_id": "dashboard-check-parity",
        "paths": ["."],
        "language": null,
        "rules": []
    });
    let serialized = serde_json::to_string(&body).expect("check JSON");
    let check = send(
        handle.address(),
        &format!("POST /api/command HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nSec-Fetch-Site: same-origin\r\nCookie: {cookie}\r\nX-Whetstone-CSRF: {csrf}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{serialized}", serialized.len()),
    );
    let check = response_body(&check);
    assert_eq!(check["state"], serde_json::json!(direct_check.state));
    assert_eq!(
        check["required_snapshot"],
        serde_json::to_value(direct_check.required_snapshot).expect("snapshot JSON")
    );
    assert!(check["data"]["report"].is_object());
    assert!(check["data"]["raw_scan"].is_object());
    assert_eq!(check["data"]["raw_scan"], direct_check.data["raw_scan"]);
    assert_eq!(
        check["data"]["report"]["state"],
        direct_check.data["report"]["state"]
    );
    assert_eq!(check["data"]["receipt_persisted"], true);
    assert!(
        check["data"]["receipt_record"]["id"]
            .as_str()
            .is_some_and(|id| id.starts_with("verification.check_") && id.len() == 51),
        "the persisted receipt must use the content-derived verification identity"
    );
}
