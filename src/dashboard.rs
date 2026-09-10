//! Minimal authenticated HTTP transport for the shared Whetstone services.
//!
//! The dashboard owns transport security, not project semantics. Callers pass a
//! backend that delegates to the same typed command service used by the CLI.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024;
const SESSION_COOKIE: &str = "whetstone_session";

const INDEX_HTML: &str = include_str!("../assets/dashboard/index.html");

const APP_CSS: &str = include_str!("../assets/dashboard/app.css");

const APP_JS: &str = include_str!("../assets/dashboard/app.js");

const EDIT_JS: &str = include_str!("../assets/dashboard/edit.js");
const VIEWS_JS: &str = include_str!("../assets/dashboard/views.js");
const VIEWS_CSS: &str = include_str!("../assets/dashboard/views.css");

/// Adapter implemented by a thin wrapper over `CommandService`.
///
/// Transport code never edits files, evaluates authority, or executes tools.
/// Mutations must preserve the service's expected-revision/resume-token checks.
pub trait DashboardBackend: Send + Sync + 'static {
    /// Read-only query body. An empty body requests the default project view.
    fn inspect(&self, request_body: &[u8]) -> BackendResponse;
    fn mutate(&self, request_body: &[u8]) -> BackendResponse;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

impl BackendResponse {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self {
            status,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(value).unwrap_or_else(|_| {
                br#"{"state":"unknown","summary":"response serialization failed"}"#.to_vec()
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardMode {
    Local {
        /// Inspection is always available; this only permits an authenticated
        /// session to explicitly enter edit mode.
        allow_mutations: bool,
    },
    Hosted {
        /// The adapter must be reachable only from this authenticated proxy.
        bind: SocketAddr,
        expected_host: String,
        public_origin: String,
        authenticated_tls_boundary: bool,
        read_only: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DashboardError {
    InvalidHostedConfiguration(String),
    EntropyUnavailable(String),
    BindFailed(String),
    StartupFailed(String),
}

impl fmt::Display for DashboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidHostedConfiguration(message)
            | Self::EntropyUnavailable(message)
            | Self::BindFailed(message)
            | Self::StartupFailed(message) => formatter.write_str(message),
        }
    }
}

pub struct DashboardHandle {
    address: SocketAddr,
    public_origin: String,
    bootstrap: Mutex<Option<String>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl fmt::Debug for DashboardHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DashboardHandle")
            .field("address", &self.address)
            .field("public_origin", &self.public_origin)
            .field("bootstrap", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl DashboardHandle {
    pub fn start(
        mode: DashboardMode,
        backend: Arc<dyn DashboardBackend>,
    ) -> Result<Self, DashboardError> {
        let (bind, host, origin, allow_mutations, hosted) = match mode {
            DashboardMode::Local { allow_mutations } => {
                let bind = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);
                (bind, String::new(), String::new(), allow_mutations, false)
            }
            DashboardMode::Hosted {
                bind,
                expected_host,
                public_origin,
                authenticated_tls_boundary,
                read_only,
            } => {
                if !bind.ip().is_loopback()
                    || !authenticated_tls_boundary
                    || !read_only
                    || !public_origin.starts_with("https://")
                    || expected_host.is_empty()
                    || public_origin.trim_end_matches('/').ends_with('/')
                {
                    return Err(DashboardError::InvalidHostedConfiguration(
                        "hosted mode requires a loopback proxy target, exact host, HTTPS origin, configured authentication/TLS, and read-only operation".into(),
                    ));
                }
                (bind, expected_host, public_origin, false, true)
            }
        };
        let listener = TcpListener::bind(bind).map_err(|error| {
            DashboardError::BindFailed(format!("dashboard bind failed: {error}"))
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| DashboardError::StartupFailed(error.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|error| DashboardError::StartupFailed(error.to_string()))?;
        let expected_host = if hosted {
            host
        } else {
            format!("127.0.0.1:{}", address.port())
        };
        let public_origin = if hosted {
            origin
        } else {
            format!("http://{expected_host}")
        };
        let bootstrap = random_secret().map_err(DashboardError::EntropyUnavailable)?;
        let state = Arc::new(Mutex::new(ServerState {
            bootstrap: Some(bootstrap.clone()),
            sessions: BTreeMap::new(),
        }));
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_origin = public_origin.clone();
        let worker = thread::Builder::new()
            .name("whetstone-dashboard".into())
            .spawn(move || {
                while !worker_shutdown.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            // Some platforms inherit the listener's nonblocking
                            // mode onto accepted sockets. A browser can connect
                            // before it has written the request, so make the
                            // bounded per-connection read explicitly blocking.
                            let _ = stream.set_nonblocking(false);
                            handle_connection(
                                stream,
                                &expected_host,
                                &worker_origin,
                                hosted,
                                allow_mutations,
                                &state,
                                backend.as_ref(),
                            )
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| DashboardError::StartupFailed(error.to_string()))?;
        Ok(Self {
            address,
            public_origin,
            bootstrap: Mutex::new(Some(bootstrap)),
            shutdown,
            worker: Some(worker),
        })
    }

    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Credential-free URL suitable for terminal output and logs.
    pub fn public_url(&self) -> &str {
        &self.public_origin
    }

    /// One-time launch handoff for a browser. Keep separate from printed URLs;
    /// the caller may append `#bootstrap=<value>` immediately before opening.
    pub fn take_bootstrap_fragment(&self) -> Option<String> {
        self.bootstrap.lock().ok()?.take()
    }

    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        // Wake the loop without relying on or terminating any unrelated server.
        let _ = TcpStream::connect(self.address);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for DashboardHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct ServerState {
    bootstrap: Option<String>,
    sessions: BTreeMap<String, Session>,
}

struct Session {
    csrf: String,
    can_mutate: bool,
}

fn handle_connection(
    mut stream: TcpStream,
    expected_host: &str,
    expected_origin: &str,
    hosted: bool,
    mutations_enabled: bool,
    state: &Mutex<ServerState>,
    backend: &dyn DashboardBackend,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let request = match read_request(&mut stream) {
        Ok(request) => request,
        Err((status, reason)) => {
            let _ = write_response(
                &mut stream,
                status,
                "text/plain; charset=utf-8",
                reason.as_bytes(),
                &[],
            );
            return;
        }
    };
    if request.header("host") != Some(expected_host) {
        let _ = deny(&mut stream, 403, "invalid_host");
        return;
    }
    if hosted
        && (request.header("x-forwarded-proto") != Some("https")
            || request.header("x-whetstone-authenticated") != Some("true"))
    {
        let _ = deny(&mut stream, 401, "host_authentication_required");
        return;
    }
    if request
        .header("sec-fetch-site")
        .is_some_and(|value| !matches!(value, "same-origin" | "none"))
    {
        let _ = deny(&mut stream, 403, "cross_site_request_denied");
        return;
    }
    let mut extra_headers = Vec::new();
    let response = match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/") => BackendResponse {
            status: 200,
            content_type: "text/html; charset=utf-8",
            body: INDEX_HTML.as_bytes().to_vec(),
        },
        ("GET", "/app.css") => BackendResponse {
            status: 200,
            content_type: "text/css; charset=utf-8",
            body: APP_CSS.as_bytes().to_vec(),
        },
        ("GET", "/app.js") => BackendResponse {
            status: 200,
            content_type: "text/javascript; charset=utf-8",
            body: APP_JS.as_bytes().to_vec(),
        },
        ("GET", "/edit.js") => BackendResponse {
            status: 200,
            content_type: "text/javascript; charset=utf-8",
            body: EDIT_JS.as_bytes().to_vec(),
        },
        ("GET", "/views.js") => BackendResponse {
            status: 200,
            content_type: "text/javascript; charset=utf-8",
            body: VIEWS_JS.as_bytes().to_vec(),
        },
        ("GET", "/views.css") => BackendResponse {
            status: 200,
            content_type: "text/css; charset=utf-8",
            body: VIEWS_CSS.as_bytes().to_vec(),
        },
        ("POST", "/session/bootstrap") => {
            if !same_origin(&request, expected_origin)
                || request.header("sec-fetch-site") != Some("same-origin")
            {
                denied("invalid_origin")
            } else {
                let presented = request.header("x-whetstone-bootstrap").unwrap_or_default();
                let mut locked = match state.lock() {
                    Ok(state) => state,
                    Err(_) => {
                        let _ = deny(&mut stream, 500, "session_state_unavailable");
                        return;
                    }
                };
                if !locked
                    .bootstrap
                    .as_deref()
                    .is_some_and(|value| constant_time_eq(value, presented))
                {
                    denied("bootstrap_denied")
                } else {
                    locked.bootstrap = None;
                    match (random_secret(), random_secret()) {
                        (Ok(session_id), Ok(csrf)) => {
                            locked.sessions.insert(
                                session_id.clone(),
                                Session {
                                    csrf: csrf.clone(),
                                    can_mutate: false,
                                },
                            );
                            extra_headers.push((
                                "Set-Cookie",
                                format!("{SESSION_COOKIE}={session_id}; HttpOnly; SameSite=Strict; Path=/"),
                            ));
                            BackendResponse::json(
                                200,
                                &serde_json::json!({"csrf_token": csrf, "mode": "inspect"}),
                            )
                        }
                        _ => denied("session_entropy_unavailable"),
                    }
                }
            }
        }
        // Local inspection is intentionally credential-free. It is
        // loopback-only, exact-host checked, and cannot mutate state. Hosted
        // inspection passed the authenticated proxy checks above.
        ("GET", "/api/inspect") => backend.inspect(&[]),
        ("POST", "/api/inspect") => {
            if !same_origin(&request, expected_origin)
                || request.header("sec-fetch-site") != Some("same-origin")
            {
                denied("invalid_origin")
            } else {
                backend.inspect(&request.body)
            }
        }
        ("POST", "/session/edit") => match authorized_mutation(&request, expected_origin, state) {
            Ok(session_id) if mutations_enabled && !hosted => {
                if let Ok(mut locked) = state.lock() {
                    if let Some(session) = locked.sessions.get_mut(&session_id) {
                        session.can_mutate = true;
                    }
                }
                BackendResponse::json(200, &serde_json::json!({"mode": "edit"}))
            }
            Ok(_) => denied("read_only"),
            Err(response) => response,
        },
        ("POST", "/api/command") => match authorized_mutation(&request, expected_origin, state) {
            Ok(session_id) => {
                let permitted = state
                    .lock()
                    .ok()
                    .and_then(|locked| {
                        locked
                            .sessions
                            .get(&session_id)
                            .map(|session| session.can_mutate)
                    })
                    .unwrap_or(false);
                if !mutations_enabled || hosted || !permitted {
                    denied("read_only")
                } else {
                    backend.mutate(&request.body)
                }
            }
            Err(response) => response,
        },
        _ => denied_status(404, "not_found"),
    };
    let borrowed = extra_headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect::<Vec<_>>();
    let _ = write_response(
        &mut stream,
        response.status,
        response.content_type,
        &response.body,
        &borrowed,
    );
}

fn authorized_mutation(
    request: &HttpRequest,
    expected_origin: &str,
    state: &Mutex<ServerState>,
) -> Result<String, BackendResponse> {
    if !same_origin(request, expected_origin)
        || request.header("sec-fetch-site") != Some("same-origin")
    {
        return Err(denied("invalid_origin"));
    }
    let session_id = authenticated_session(request, state)
        .ok_or_else(|| denied_status(401, "authentication_required"))?;
    let csrf = request.header("x-whetstone-csrf").unwrap_or_default();
    let matches = state
        .lock()
        .ok()
        .and_then(|locked| {
            locked
                .sessions
                .get(&session_id)
                .map(|session| constant_time_eq(&session.csrf, csrf))
        })
        .unwrap_or(false);
    if !matches {
        Err(denied("csrf_denied"))
    } else {
        Ok(session_id)
    }
}

fn authenticated_session(request: &HttpRequest, state: &Mutex<ServerState>) -> Option<String> {
    let cookie = request.header("cookie")?;
    let value = cookie.split(';').find_map(|part| {
        let (name, value) = part.trim().split_once('=')?;
        (name == SESSION_COOKIE).then(|| value.to_string())
    })?;
    state
        .lock()
        .ok()?
        .sessions
        .keys()
        .find(|known| constant_time_eq(known, &value))
        .cloned()
}

fn same_origin(request: &HttpRequest, expected: &str) -> bool {
    request.header("origin") == Some(expected)
}

fn denied(reason: &str) -> BackendResponse {
    denied_status(403, reason)
}

fn denied_status(status: u16, reason: &str) -> BackendResponse {
    BackendResponse::json(
        status,
        &serde_json::json!({"state": "denied", "reason_code": reason}),
    )
}

struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, (u16, String)> {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 2048];
    let header_end = loop {
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err((431, "headers_too_large".into()));
        }
        let read = stream
            .read(&mut chunk)
            .map_err(|_| (400, "invalid_request".into()))?;
        if read == 0 {
            return Err((400, "incomplete_request".into()));
        }
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head =
        std::str::from_utf8(&bytes[..header_end]).map_err(|_| (400, "invalid_headers".into()))?;
    let mut lines = head.split("\r\n");
    let request_line = lines.next().ok_or((400, "missing_request_line".into()))?;
    let parts = request_line.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err((400, "invalid_request_line".into()));
    }
    let method = parts[0].to_string();
    if !matches!(method.as_str(), "GET" | "POST") {
        return Err((405, "method_not_allowed".into()));
    }
    let path = parts[1].to_string();
    if !path.starts_with('/') || path.contains("..") || path.contains('%') || path.contains('?') {
        return Err((400, "unsafe_request_path".into()));
    }
    let mut headers = BTreeMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or((400, "invalid_header".into()))?;
        let key = name.trim().to_ascii_lowercase();
        if key.is_empty() || headers.insert(key, value.trim().to_string()).is_some() {
            return Err((400, "duplicate_or_invalid_header".into()));
        }
    }
    if headers.contains_key("transfer-encoding") {
        return Err((400, "transfer_encoding_unsupported".into()));
    }
    let content_length = headers
        .get("content-length")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| (400, "invalid_content_length".into()))
        })
        .transpose()?
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err((413, "payload_too_large".into()));
    }
    while bytes.len() - header_end < content_length {
        let read = stream
            .read(&mut chunk)
            .map_err(|_| (400, "invalid_body".into()))?;
        if read == 0 {
            return Err((400, "incomplete_body".into()));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    extra_headers: &[(&str, &str)],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nX-Frame-Options: DENY\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nPermissions-Policy: camera=(), microphone=(), geolocation=()\r\nCross-Origin-Opener-Policy: same-origin\r\nCross-Origin-Resource-Policy: same-origin\r\n",
        body.len()
    )?;
    for (name, value) in extra_headers {
        write!(stream, "{name}: {value}\r\n")?;
    }
    write!(stream, "Connection: close\r\n\r\n")?;
    stream.write_all(body)
}

fn deny(stream: &mut TcpStream, status: u16, reason: &str) -> io::Result<()> {
    let response = denied_status(status, reason);
    write_response(stream, status, response.content_type, &response.body, &[])
}

fn random_secret() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|error| format!("cryptographic entropy unavailable: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}
