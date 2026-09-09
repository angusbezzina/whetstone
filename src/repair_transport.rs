//! Provider-neutral transport for host-authenticated repair authority.
//!
//! A Whetstone process cannot mint repair authority from project data or CLI
//! arguments. A supporting host instead creates an owner-only Unix socket,
//! gives the child process a one-time secret through its inherited environment,
//! and authenticates each opaque authority locator over that socket. The wire
//! response is private to this module; only this adapter can construct a
//! [`VerifiedRepairAuthority`](crate::repair_host::VerifiedRepairAuthority).

#[cfg(unix)]
mod unix {
    use std::env;
    use std::fs;
    use std::io::{Read, Write};
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};

    use crate::domain::{ExternalRef, PrincipalRef};
    use crate::repair_host::{
        RepairAuthorityEvidence, RepairAuthorityTarget, RepairAuthorityVerifier,
        RepairCompletionEvidence, RepairCompletionTarget, RepairHostError, RepairTaskContext,
        VerifiedRepairAuthority, VerifiedRepairCompletion,
    };

    pub const AUTHORITY_SOCKET_ENV: &str = "WHETSTONE_REPAIR_AUTHORITY_SOCKET";
    pub const AUTHORITY_SECRET_ENV: &str = "WHETSTONE_REPAIR_AUTHORITY_SECRET";
    pub const REPAIR_LAUNCH_ENV: &str = "WHETSTONE_REPAIR_LAUNCH";
    const REQUEST_SCHEMA: &str = "whetstone.repair-authority-request.v1";
    const RESPONSE_SCHEMA: &str = "whetstone.repair-authority-response.v1";
    const COMPLETION_REQUEST_SCHEMA: &str = "whetstone.repair-completion-request.v1";
    const COMPLETION_RESPONSE_SCHEMA: &str = "whetstone.repair-completion-response.v1";
    const MAX_REQUEST_BYTES: usize = 256 * 1024;
    const MAX_RESPONSE_BYTES: u64 = 256 * 1024;
    const IO_TIMEOUT: Duration = Duration::from_secs(5);

    /// Host-owned adapter configuration. The secret is intentionally neither
    /// serializable nor printable and must never be written to project state.
    pub struct HostSocketAuthorityVerifier {
        socket_path: PathBuf,
        project_root: PathBuf,
        secret: String,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HostTransportUnavailable {
        MissingConfiguration,
        InvalidConfiguration,
    }

    /// Inert, bounded context injected by a supporting host when it asks the
    /// CLI to begin a session. The socket adapter must still authenticate this
    /// exact value; possessing or fabricating this JSON grants nothing.
    #[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    pub struct HostRepairLaunch {
        pub task: ExternalRef,
        pub authority_revision: u64,
        pub context: RepairTaskContext,
    }

    impl HostSocketAuthorityVerifier {
        /// Read a capability installed by the parent host. Project files and
        /// CLI values are deliberately not consulted.
        pub fn from_inherited_environment(
            project_root: &Path,
        ) -> Result<Self, HostTransportUnavailable> {
            let socket_path = env::var_os(AUTHORITY_SOCKET_ENV)
                .ok_or(HostTransportUnavailable::MissingConfiguration)
                .map(PathBuf::from)?;
            let secret = env::var(AUTHORITY_SECRET_ENV)
                .map_err(|_| HostTransportUnavailable::MissingConfiguration)?;
            Self::new(socket_path, project_root.to_path_buf(), secret)
        }

        pub fn launch_from_inherited_environment(
        ) -> Result<HostRepairLaunch, HostTransportUnavailable> {
            let encoded = env::var(REPAIR_LAUNCH_ENV)
                .map_err(|_| HostTransportUnavailable::MissingConfiguration)?;
            if encoded.len() >= MAX_REQUEST_BYTES {
                return Err(HostTransportUnavailable::InvalidConfiguration);
            }
            serde_json::from_str(&encoded)
                .map_err(|_| HostTransportUnavailable::InvalidConfiguration)
        }

        pub fn new(
            socket_path: PathBuf,
            project_root: PathBuf,
            secret: String,
        ) -> Result<Self, HostTransportUnavailable> {
            if !socket_path.is_absolute()
                || secret.len() < 32
                || secret.chars().any(char::is_whitespace)
            {
                return Err(HostTransportUnavailable::InvalidConfiguration);
            }
            let project_root = fs::canonicalize(project_root)
                .map_err(|_| HostTransportUnavailable::InvalidConfiguration)?;
            Ok(Self {
                socket_path,
                project_root,
                secret,
            })
        }

        fn exchange(
            &self,
            evidence: &RepairAuthorityEvidence,
            target: &RepairAuthorityTarget,
        ) -> Result<VerifiedRepairAuthority, RepairHostError> {
            validate_socket(&self.socket_path, &self.project_root)?;
            let request_id = request_id(&self.secret, evidence, target);
            let request = AuthorityRequest {
                schema: REQUEST_SCHEMA,
                request_id: request_id.clone(),
                one_time_secret: &self.secret,
                evidence_locator: &evidence.locator,
                target: AuthorityTarget {
                    session_id: &target.session_id,
                    task: &target.task,
                    project: &target.project,
                    authority_revision: target.authority_revision,
                    context: &target.context,
                    now_unix: target.now_unix,
                },
            };
            let response: AuthorityResponse =
                exchange_json(&self.socket_path, &request, RESPONSE_SCHEMA, &request_id)?;
            match response.state {
                AuthorityResponseState::Granted => {
                    let grant = response.grant.ok_or(RepairHostError::MissingAuthority)?;
                    Ok(VerifiedRepairAuthority {
                        evidence_id: grant.evidence_id,
                        principal: grant.principal,
                        task: grant.task,
                        project: grant.project,
                        authority_revision: grant.authority_revision,
                        expires_at: grant.expires_at,
                        expires_at_unix: grant.expires_at_unix,
                        context: grant.context,
                        may_edit_source: grant.may_edit_source,
                        may_edit_policy: grant.may_edit_policy,
                        may_edit_checks: grant.may_edit_checks,
                        may_reset_baselines: grant.may_reset_baselines,
                        may_publish: grant.may_publish,
                        may_merge: grant.may_merge,
                        may_release: grant.may_release,
                    })
                }
                AuthorityResponseState::Denied | AuthorityResponseState::Unavailable => {
                    Err(RepairHostError::MissingAuthority)
                }
            }
        }

        fn exchange_completion(
            &self,
            evidence: &RepairCompletionEvidence,
            target: &RepairCompletionTarget,
        ) -> Result<VerifiedRepairCompletion, RepairHostError> {
            validate_socket(&self.socket_path, &self.project_root)?;
            let request_id = completion_request_id(&self.secret, evidence, target);
            let request = CompletionRequest {
                schema: COMPLETION_REQUEST_SCHEMA,
                request_id: request_id.clone(),
                one_time_secret: &self.secret,
                evidence_locator: &evidence.locator,
                target: CompletionTarget {
                    session_id: &target.session_id,
                    task: &target.task,
                    project: &target.project,
                    authority_revision: target.authority_revision,
                    candidate_workspace: &target.candidate_workspace,
                    check_snapshot: &target.check_snapshot,
                },
            };
            let response: CompletionResponse = exchange_json(
                &self.socket_path,
                &request,
                COMPLETION_RESPONSE_SCHEMA,
                &request_id,
            )?;
            match response.state {
                AuthorityResponseState::Granted => {
                    let grant = response.grant.ok_or(RepairHostError::CompletionRejected)?;
                    Ok(VerifiedRepairCompletion {
                        evidence_id: grant.evidence_id,
                        session_id: grant.session_id,
                        task: grant.task,
                        project: grant.project,
                        authority_revision: grant.authority_revision,
                        candidate_workspace: grant.candidate_workspace,
                        check_snapshot: grant.check_snapshot,
                        task_acceptance_satisfied: grant.task_acceptance_satisfied,
                        required_reviews_satisfied: grant.required_reviews_satisfied,
                    })
                }
                AuthorityResponseState::Denied | AuthorityResponseState::Unavailable => {
                    Err(RepairHostError::CompletionRejected)
                }
            }
        }
    }

    impl RepairAuthorityVerifier for HostSocketAuthorityVerifier {
        fn verify(
            &self,
            evidence: &RepairAuthorityEvidence,
            target: &RepairAuthorityTarget,
        ) -> Result<VerifiedRepairAuthority, RepairHostError> {
            self.exchange(evidence, target)
        }

        fn verify_completion(
            &self,
            evidence: &RepairCompletionEvidence,
            target: &RepairCompletionTarget,
        ) -> Result<VerifiedRepairCompletion, RepairHostError> {
            self.exchange_completion(evidence, target)
        }
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct AuthorityRequest<'a> {
        schema: &'static str,
        request_id: String,
        one_time_secret: &'a str,
        evidence_locator: &'a str,
        target: AuthorityTarget<'a>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct AuthorityTarget<'a> {
        session_id: &'a str,
        task: &'a ExternalRef,
        project: &'a str,
        authority_revision: u64,
        context: &'a RepairTaskContext,
        now_unix: u64,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AuthorityResponse {
        #[serde(rename = "schema")]
        _schema: String,
        #[serde(rename = "request_id")]
        _request_id: String,
        state: AuthorityResponseState,
        #[serde(default)]
        grant: Option<AuthorityGrant>,
        #[serde(default)]
        _reason_code: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum AuthorityResponseState {
        Granted,
        Denied,
        Unavailable,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AuthorityGrant {
        evidence_id: String,
        principal: PrincipalRef,
        task: ExternalRef,
        project: String,
        authority_revision: u64,
        expires_at: String,
        expires_at_unix: u64,
        context: RepairTaskContext,
        may_edit_source: bool,
        may_edit_policy: bool,
        may_edit_checks: bool,
        may_reset_baselines: bool,
        may_publish: bool,
        may_merge: bool,
        may_release: bool,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct CompletionRequest<'a> {
        schema: &'static str,
        request_id: String,
        one_time_secret: &'a str,
        evidence_locator: &'a str,
        target: CompletionTarget<'a>,
    }

    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct CompletionTarget<'a> {
        session_id: &'a str,
        task: &'a ExternalRef,
        project: &'a str,
        authority_revision: u64,
        candidate_workspace: &'a crate::domain::ContentDigest,
        check_snapshot: &'a crate::domain::RepairSnapshotRecord,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CompletionResponse {
        #[serde(rename = "schema")]
        _schema: String,
        #[serde(rename = "request_id")]
        _request_id: String,
        state: AuthorityResponseState,
        #[serde(default)]
        grant: Option<CompletionGrant>,
        #[serde(default)]
        _reason_code: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CompletionGrant {
        evidence_id: String,
        session_id: String,
        task: ExternalRef,
        project: String,
        authority_revision: u64,
        candidate_workspace: crate::domain::ContentDigest,
        check_snapshot: crate::domain::RepairSnapshotRecord,
        task_acceptance_satisfied: bool,
        required_reviews_satisfied: bool,
    }

    fn validate_socket(socket_path: &Path, project_root: &Path) -> Result<(), RepairHostError> {
        let metadata =
            fs::symlink_metadata(socket_path).map_err(|_| RepairHostError::MissingAuthority)?;
        if !metadata.file_type().is_socket() || metadata.mode() & 0o077 != 0 {
            return Err(RepairHostError::MissingAuthority);
        }
        let parent = socket_path
            .parent()
            .ok_or(RepairHostError::MissingAuthority)?;
        let parent_metadata =
            fs::metadata(parent).map_err(|_| RepairHostError::MissingAuthority)?;
        if parent_metadata.mode() & 0o077 != 0 {
            return Err(RepairHostError::MissingAuthority);
        }
        let socket_parent =
            fs::canonicalize(parent).map_err(|_| RepairHostError::MissingAuthority)?;
        let project_root =
            fs::canonicalize(project_root).map_err(|_| RepairHostError::MissingAuthority)?;
        let project_metadata =
            fs::metadata(&project_root).map_err(|_| RepairHostError::MissingAuthority)?;
        if socket_parent.starts_with(&project_root)
            || metadata.uid() != parent_metadata.uid()
            || metadata.uid() != project_metadata.uid()
        {
            return Err(RepairHostError::MissingAuthority);
        }
        Ok(())
    }

    fn request_id(
        secret: &str,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"whetstone.repair-authority-request.v1\0");
        hasher.update(secret.as_bytes());
        hasher.update(b"\0");
        hasher.update(evidence.locator.as_bytes());
        hasher.update(b"\0");
        hasher.update(target.session_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(target.authority_revision.to_le_bytes());
        hasher.update(b"\0");
        hasher.update(target.now_unix.to_le_bytes());
        format!("authority-{:x}", hasher.finalize())
    }

    fn completion_request_id(
        secret: &str,
        evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"whetstone.repair-completion-request.v1\0");
        hasher.update(secret.as_bytes());
        hasher.update(b"\0");
        hasher.update(evidence.locator.as_bytes());
        hasher.update(b"\0");
        hasher.update(target.session_id.as_bytes());
        hasher.update(b"\0");
        hasher.update(target.candidate_workspace.as_str().as_bytes());
        format!("completion-{:x}", hasher.finalize())
    }

    fn exchange_json<Request: Serialize, Response: for<'de> Deserialize<'de>>(
        socket_path: &Path,
        request: &Request,
        expected_schema: &str,
        expected_request_id: &str,
    ) -> Result<Response, RepairHostError> {
        let mut encoded =
            serde_json::to_vec(request).map_err(|_| RepairHostError::MissingAuthority)?;
        if encoded.len() >= MAX_REQUEST_BYTES {
            return Err(RepairHostError::MissingAuthority);
        }
        encoded.push(b'\n');
        let deadline = Instant::now() + IO_TIMEOUT;
        let mut stream = connect_before(socket_path, deadline)?;
        write_before(&mut stream, &encoded, deadline)?;
        let mut response_bytes = Vec::new();
        read_frame_before(&mut stream, &mut response_bytes, deadline)?;
        if response_bytes.is_empty()
            || response_bytes.len() as u64 >= MAX_RESPONSE_BYTES
            || !response_bytes.ends_with(b"\n")
        {
            return Err(RepairHostError::MissingAuthority);
        }
        let value: serde_json::Value = serde_json::from_slice(&response_bytes)
            .map_err(|_| RepairHostError::MissingAuthority)?;
        if value.get("schema").and_then(serde_json::Value::as_str) != Some(expected_schema)
            || value.get("request_id").and_then(serde_json::Value::as_str)
                != Some(expected_request_id)
        {
            return Err(RepairHostError::MissingAuthority);
        }
        serde_json::from_value(value).map_err(|_| RepairHostError::MissingAuthority)
    }

    fn remaining(deadline: Instant) -> Result<Duration, RepairHostError> {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or(RepairHostError::MissingAuthority)
    }

    fn connect_before(path: &Path, deadline: Instant) -> Result<UnixStream, RepairHostError> {
        let (sender, receiver) = mpsc::sync_channel(1);
        let path = path.to_path_buf();
        thread::spawn(move || {
            let _ = sender.send(UnixStream::connect(path));
        });
        receiver
            .recv_timeout(remaining(deadline)?)
            .map_err(|_| RepairHostError::MissingAuthority)?
            .map_err(|_| RepairHostError::MissingAuthority)
    }

    fn write_before(
        stream: &mut UnixStream,
        mut bytes: &[u8],
        deadline: Instant,
    ) -> Result<(), RepairHostError> {
        while !bytes.is_empty() {
            stream
                .set_write_timeout(Some(remaining(deadline)?))
                .map_err(|_| RepairHostError::MissingAuthority)?;
            let written = stream
                .write(bytes)
                .map_err(|_| RepairHostError::MissingAuthority)?;
            if written == 0 {
                return Err(RepairHostError::MissingAuthority);
            }
            bytes = &bytes[written..];
        }
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|_| RepairHostError::MissingAuthority)?;
        stream
            .flush()
            .map_err(|_| RepairHostError::MissingAuthority)
    }

    fn read_frame_before(
        stream: &mut UnixStream,
        response: &mut Vec<u8>,
        deadline: Instant,
    ) -> Result<(), RepairHostError> {
        let mut buffer = [0_u8; 8 * 1024];
        while response.len() < MAX_RESPONSE_BYTES as usize {
            stream
                .set_read_timeout(Some(remaining(deadline)?))
                .map_err(|_| RepairHostError::MissingAuthority)?;
            let read = stream
                .read(&mut buffer)
                .map_err(|_| RepairHostError::MissingAuthority)?;
            if read == 0 {
                return Err(RepairHostError::MissingAuthority);
            }
            response.extend_from_slice(&buffer[..read]);
            if let Some(newline) = response.iter().position(|byte| *byte == b'\n') {
                if newline + 1 != response.len() {
                    return Err(RepairHostError::MissingAuthority);
                }
                return Ok(());
            }
        }
        Err(RepairHostError::MissingAuthority)
    }
}

#[cfg(unix)]
pub use unix::{
    HostRepairLaunch, HostSocketAuthorityVerifier, HostTransportUnavailable, AUTHORITY_SECRET_ENV,
    AUTHORITY_SOCKET_ENV, REPAIR_LAUNCH_ENV,
};

#[cfg(not(unix))]
mod unsupported {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum HostTransportUnavailable {
        MissingConfiguration,
        InvalidConfiguration,
        UnsupportedPlatform,
    }
}

#[cfg(not(unix))]
pub use unsupported::HostTransportUnavailable;
