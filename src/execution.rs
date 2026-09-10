//! Content-addressed, bounded execution of explicitly trusted native checkers.
//!
//! This runner is deliberately narrow. It does not resolve commands through
//! `PATH`, invoke a shell, inherit the host environment, or execute an imported
//! manifest before a separate approval is supplied. Repository confinement and
//! time/output limits are enforced here; this is not an OS security boundary.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::PrincipalRef;

pub const MANIFEST_SCHEMA: &str = "whetstone.checker-manifest.v1";
pub const RECEIPT_SCHEMA: &str = "whetstone.execution-receipt.v1";
const MAX_TIMEOUT_MS: u64 = 5_000;
const MAX_OUTPUT_BYTES: usize = 1_048_576;
const MAX_ENV_VALUE_BYTES: usize = 16_384;
const MAX_CONFIGURATION_FILES: usize = 4_096;
const MAX_CONFIGURATION_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckerManifest {
    pub schema: String,
    pub checker_id: String,
    pub checker_version: String,
    pub source: ManifestSource,
    /// Repository-relative executable path. `PATH` lookup is never performed.
    pub executable: String,
    pub executable_digest: String,
    #[serde(default)]
    pub args: Vec<Argument>,
    pub working_directory: String,
    #[serde(default)]
    pub configurations: Vec<ConfigurationBinding>,
    #[serde(default)]
    pub accepted_input_scopes: Vec<String>,
    #[serde(default)]
    pub environment_allowlist: BTreeSet<String>,
    pub timeout_ms: u64,
    pub stdout_limit_bytes: usize,
    pub stderr_limit_bytes: usize,
    #[serde(default = "default_violation_codes")]
    pub violation_exit_codes: BTreeSet<i32>,
    #[serde(default)]
    pub citations: Vec<String>,
}

fn default_violation_codes() -> BTreeSet<i32> {
    BTreeSet::from([1])
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManifestSource {
    Local { authored_by: String },
    Imported { source_digest: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Argument {
    Literal { value: String },
    Input { index: usize },
    Configuration { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigurationBinding {
    pub id: String,
    pub path: String,
    pub digest: String,
}

/// Opaque locator interpreted only by the configured authority adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionApprovalEvidence {
    pub locator: String,
}

/// Exact executable binding for which an execution grant is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionGrantTarget {
    pub manifest_digest: String,
    pub checker_id: String,
    pub checker_version: String,
    pub authority_revision: u64,
}

/// Authenticated grant returned by a trusted authority adapter.
///
/// This type is deliberately not deserializable. A caller may supply only
/// opaque evidence; it cannot make JSON fields such as `approved_by` authoritative.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedExecutionGrant {
    pub grant_reference: String,
    pub manifest_digest: String,
    pub principal: PrincipalRef,
    pub authority_revision: u64,
    pub authority_checked_at: String,
    pub expires_at: String,
}

pub trait ExecutionAuthorityVerifier {
    fn verify_execution_grant(
        &self,
        evidence: &ExecutionApprovalEvidence,
        target: &ExecutionGrantTarget,
    ) -> Result<VerifiedExecutionGrant, ExecutionAuthorizationError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionAuthorizationError {
    Unavailable,
    Rejected,
}

pub struct TrustedExecutionService<V> {
    verifier: V,
}

impl<V: ExecutionAuthorityVerifier> TrustedExecutionService<V> {
    pub fn new(verifier: V) -> Self {
        Self { verifier }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn execute(
        &self,
        project_root: &Path,
        manifest: &CheckerManifest,
        evidence: Option<&ExecutionApprovalEvidence>,
        principal: &PrincipalRef,
        authority_revision: u64,
        now: &str,
        request: &ExecutionRequest,
    ) -> ExecutionReceipt {
        let started = Instant::now();
        let digest = match manifest_digest(manifest) {
            Ok(value) => value,
            Err(error) => {
                return receipt(
                    manifest,
                    "sha256:unavailable".into(),
                    ExecutionState::Unknown,
                    "manifest_serialization_failed",
                    error,
                    false,
                    started,
                )
            }
        };
        let Some(evidence) = evidence else {
            return receipt(
                manifest,
                digest,
                ExecutionState::NeedsExecutionApproval,
                "manifest_not_approved",
                "The exact checker manifest has no verified execution grant; nothing executed."
                    .into(),
                false,
                started,
            );
        };
        let target = ExecutionGrantTarget {
            manifest_digest: digest.clone(),
            checker_id: manifest.checker_id.clone(),
            checker_version: manifest.checker_version.clone(),
            authority_revision,
        };
        let grant = match self.verifier.verify_execution_grant(evidence, &target) {
            Ok(grant) => grant,
            Err(ExecutionAuthorizationError::Unavailable) => {
                return receipt(
                    manifest,
                    digest,
                    ExecutionState::Unavailable,
                    "execution_authority_unavailable",
                    "Execution authority could not be checked; nothing executed.".into(),
                    false,
                    started,
                )
            }
            Err(ExecutionAuthorizationError::Rejected) => {
                return receipt(
                    manifest,
                    digest,
                    ExecutionState::NeedsExecutionApproval,
                    "execution_grant_rejected",
                    "The authority adapter rejected the execution grant; nothing executed.".into(),
                    false,
                    started,
                )
            }
        };
        if grant.manifest_digest != target.manifest_digest
            || grant.principal != *principal
            || grant.authority_revision != target.authority_revision
            || now > grant.expires_at.as_str()
        {
            return receipt(
                manifest,
                digest,
                ExecutionState::NeedsExecutionApproval,
                "execution_grant_mismatch",
                "The verified grant does not bind the current manifest, principal, authority, and time; nothing executed.".into(),
                false,
                started,
            );
        }
        let mut output = execute_verified_checker(project_root, manifest, &grant, request);
        output.execution_grant_reference = Some(grant.grant_reference);
        output.authority_revision = Some(grant.authority_revision);
        output.authority_checked_at = Some(grant.authority_checked_at);
        output
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionRequest {
    pub inputs: Vec<String>,
    /// Only keys named by `environment_allowlist` are accepted. Values are
    /// supplied at execution time, never persisted in the manifest or receipt.
    pub environment: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Success,
    Violated,
    Unknown,
    Unavailable,
    NeedsExecutionApproval,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionReceipt {
    pub schema: String,
    pub checker_id: String,
    pub checker_version: String,
    pub manifest_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_grant_reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_checked_at: Option<String>,
    pub state: ExecutionState,
    pub reason_code: String,
    pub summary: String,
    pub process_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_digest: Option<String>,
    #[serde(default)]
    pub consumed_configurations: Vec<ConsumedConfiguration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
    pub limits: LimitEvidence,
    pub process_group_cleanup: String,
    pub isolation_caveat: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsumedConfiguration {
    pub id: String,
    pub path: String,
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct CapturedOutput {
    pub text: String,
    pub bytes_captured: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitEvidence {
    pub timeout_ms: u64,
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub timeout_mechanism: String,
    pub output_mechanism: String,
    pub filesystem_network_memory_process_limits: String,
}

pub fn manifest_digest(manifest: &CheckerManifest) -> Result<String, String> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| format!("could not serialize checker manifest: {error}"))?;
    Ok(sha256_bytes(&bytes))
}

pub fn file_digest(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        hash.update(&chunk[..read]);
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

/// Hashes one configuration file or a directory tree without following
/// symlinks. Relative names and contents are included in stable sort order.
pub fn configuration_digest(path: &Path) -> io::Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "configuration symlinks are not accepted",
        ));
    }
    if metadata.is_file() {
        return file_digest(path);
    }
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration must be a file or directory",
        ));
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(path).follow_links(false) {
        let entry = entry.map_err(io::Error::other)?;
        let entry_metadata = std::fs::symlink_metadata(entry.path())?;
        if entry_metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "configuration trees may not contain symlinks",
            ));
        }
        if entry_metadata.is_file() {
            files.push((entry.path().to_path_buf(), entry_metadata.len()));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.len() > MAX_CONFIGURATION_FILES
        || files.iter().map(|(_, size)| *size).sum::<u64>() > MAX_CONFIGURATION_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "configuration tree exceeds its file or byte bound",
        ));
    }
    let mut hash = Sha256::new();
    for (file, _) in files {
        let relative = file
            .strip_prefix(path)
            .map_err(io::Error::other)?
            .to_string_lossy();
        hash.update((relative.len() as u64).to_be_bytes());
        hash.update(relative.as_bytes());
        let mut reader = File::open(&file)?;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            hash.update(&chunk[..read]);
        }
    }
    Ok(format!("sha256:{:x}", hash.finalize()))
}

fn execute_verified_checker(
    project_root: &Path,
    manifest: &CheckerManifest,
    grant: &VerifiedExecutionGrant,
    request: &ExecutionRequest,
) -> ExecutionReceipt {
    let started = Instant::now();
    let digest = match manifest_digest(manifest) {
        Ok(value) => value,
        Err(error) => {
            return receipt(
                manifest,
                "sha256:unavailable".into(),
                ExecutionState::Unknown,
                "manifest_serialization_failed",
                error,
                false,
                started,
            )
        }
    };

    if grant.manifest_digest != digest {
        return receipt(
            manifest,
            digest,
            ExecutionState::NeedsExecutionApproval,
            "manifest_changed",
            "The checker manifest differs from the approved digest; nothing executed.".into(),
            false,
            started,
        );
    }

    if let Err(error) = validate_manifest(manifest) {
        return receipt(
            manifest,
            digest,
            ExecutionState::Unknown,
            "invalid_manifest",
            error,
            false,
            started,
        );
    }
    let root = match project_root.canonicalize() {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unavailable,
                "project_root_unavailable",
                "The project root is not a directory.".into(),
                false,
                started,
            )
        }
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unavailable,
                "project_root_unavailable",
                format!("The project root is unavailable: {error}"),
                false,
                started,
            )
        }
    };

    let executable = match canonical_repo_path(&root, &manifest.executable, true) {
        Ok(path) => path,
        Err(PathFailure::Missing(error)) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unavailable,
                "executable_unavailable",
                error,
                false,
                started,
            )
        }
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "unsafe_executable_path",
                error.to_string(),
                false,
                started,
            )
        }
    };
    if is_shell(&executable) {
        return receipt(
            manifest,
            digest,
            ExecutionState::Unknown,
            "shell_execution_denied",
            "Shell executables are not supported by the argv-only adapter.".into(),
            false,
            started,
        );
    }
    let observed_executable_digest = match file_digest(&executable) {
        Ok(value) => value,
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unavailable,
                "executable_unavailable",
                format!("Could not read the checker executable: {error}"),
                false,
                started,
            )
        }
    };
    if observed_executable_digest != manifest.executable_digest {
        return receipt(
            manifest,
            digest,
            ExecutionState::NeedsExecutionApproval,
            "executable_changed",
            "The executable digest changed after approval; nothing executed.".into(),
            false,
            started,
        );
    }

    let cwd = match canonical_repo_path(&root, &manifest.working_directory, true) {
        Ok(path) if path.is_dir() => path,
        Ok(_) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "invalid_working_directory",
                "The working directory is not a directory.".into(),
                false,
                started,
            )
        }
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "unsafe_working_directory",
                error.to_string(),
                false,
                started,
            )
        }
    };

    let mut config_paths = BTreeMap::new();
    let mut consumed = Vec::new();
    for config in &manifest.configurations {
        let path = match canonical_repo_path(&root, &config.path, true) {
            Ok(path) if path.is_file() || path.is_dir() => path,
            Ok(_) => {
                return receipt(
                    manifest,
                    digest,
                    ExecutionState::Unknown,
                    "invalid_configuration",
                    format!("Configuration {} is not a file or directory.", config.id),
                    false,
                    started,
                )
            }
            Err(error) => {
                return receipt(
                    manifest,
                    digest,
                    ExecutionState::Unknown,
                    "unsafe_configuration_path",
                    error.to_string(),
                    false,
                    started,
                )
            }
        };
        let observed = match configuration_digest(&path) {
            Ok(value) => value,
            Err(error) => {
                return receipt(
                    manifest,
                    digest,
                    ExecutionState::Unknown,
                    "configuration_unreadable",
                    error.to_string(),
                    false,
                    started,
                )
            }
        };
        if observed != config.digest {
            return receipt(
                manifest,
                digest,
                ExecutionState::NeedsExecutionApproval,
                "configuration_changed",
                format!("Configuration {} changed after approval.", config.id),
                false,
                started,
            );
        }
        config_paths.insert(config.id.clone(), path);
        consumed.push(ConsumedConfiguration {
            id: config.id.clone(),
            path: config.path.clone(),
            digest: observed,
        });
    }

    let scopes = match manifest
        .accepted_input_scopes
        .iter()
        .map(|path| canonical_repo_path(&root, path, true))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(scopes) => scopes,
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "unsafe_input_scope",
                error.to_string(),
                false,
                started,
            )
        }
    };
    let mut input_paths = Vec::new();
    for input in &request.inputs {
        let path = match canonical_repo_path(&root, input, true) {
            Ok(path) => path,
            Err(error) => {
                return receipt(
                    manifest,
                    digest,
                    ExecutionState::Unknown,
                    "unsafe_input_path",
                    error.to_string(),
                    false,
                    started,
                )
            }
        };
        if scopes.is_empty() || !scopes.iter().any(|scope| path.starts_with(scope)) {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "input_out_of_scope",
                format!("Input {input} is outside every accepted input scope."),
                false,
                started,
            );
        }
        input_paths.push(path);
    }

    if request.environment.iter().any(|(key, value)| {
        !manifest.environment_allowlist.contains(key)
            || !valid_env_name(key)
            || value.len() > MAX_ENV_VALUE_BYTES
            || value.contains('\0')
    }) {
        return receipt(
            manifest,
            digest,
            ExecutionState::Unknown,
            "environment_denied",
            "An environment key/value was not explicitly allowed or exceeded its bound.".into(),
            false,
            started,
        );
    }

    let argv = match render_args(&manifest.args, &input_paths, &config_paths) {
        Ok(args) => args,
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "invalid_arguments",
                error,
                false,
                started,
            )
        }
    };
    let run = run_bounded(
        &executable,
        &argv,
        &cwd,
        &request.environment,
        Duration::from_millis(manifest.timeout_ms),
        manifest.stdout_limit_bytes,
        manifest.stderr_limit_bytes,
    );
    let mut result = match run {
        Ok(result) => result,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unavailable,
                "executable_unavailable",
                error.to_string(),
                false,
                started,
            )
        }
        Err(error) => {
            return receipt(
                manifest,
                digest,
                ExecutionState::Unknown,
                "execution_failed",
                error.to_string(),
                false,
                started,
            )
        }
    };
    redact(&mut result.stdout.text, request.environment.values());
    redact(&mut result.stderr.text, request.environment.values());

    // Prefer the concrete bounded-output cause when both guards fire. A
    // process may continue emitting until process-group cleanup observes the
    // timeout, but the first actionable failure is still the exhausted output
    // budget.
    let (state, reason, summary) = if result.stdout.truncated || result.stderr.truncated {
        (
            ExecutionState::Unknown,
            "output_limit_exceeded",
            "The checker exceeded a hard captured-output limit.",
        )
    } else if result.timed_out {
        (
            ExecutionState::Unknown,
            "execution_timed_out",
            "The checker exceeded its hard wall-clock limit.",
        )
    } else if result.status.success() {
        (
            ExecutionState::Success,
            "check_passed",
            "The checker passed.",
        )
    } else if result
        .status
        .code()
        .is_some_and(|code| manifest.violation_exit_codes.contains(&code))
    {
        (
            ExecutionState::Violated,
            "check_violated",
            "The checker reported a policy violation.",
        )
    } else {
        (
            ExecutionState::Unknown,
            "unexpected_exit",
            "The checker exited without a recognized pass or violation result.",
        )
    };
    let mut output = receipt(
        manifest,
        digest,
        state,
        reason,
        summary.into(),
        true,
        started,
    );
    output.executable_digest = Some(observed_executable_digest);
    output.consumed_configurations = consumed;
    output.exit_code = result.status.code();
    output.elapsed_ms = result.elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
    output.stdout = result.stdout;
    output.stderr = result.stderr;
    output.process_group_cleanup = result.cleanup;
    output
}

fn validate_manifest(manifest: &CheckerManifest) -> Result<(), String> {
    if manifest.schema != MANIFEST_SCHEMA {
        return Err(format!("unsupported manifest schema {}", manifest.schema));
    }
    if manifest.checker_id.trim().is_empty() || manifest.checker_version.trim().is_empty() {
        return Err("checker_id and checker_version are required".into());
    }
    validate_digest(&manifest.executable_digest)?;
    if manifest.timeout_ms == 0 || manifest.timeout_ms > MAX_TIMEOUT_MS {
        return Err(format!("timeout_ms must be in 1..={MAX_TIMEOUT_MS}"));
    }
    if manifest.stdout_limit_bytes == 0
        || manifest.stdout_limit_bytes > MAX_OUTPUT_BYTES
        || manifest.stderr_limit_bytes == 0
        || manifest.stderr_limit_bytes > MAX_OUTPUT_BYTES
    {
        return Err(format!(
            "each output limit must be in 1..={MAX_OUTPUT_BYTES}"
        ));
    }
    let mut ids = BTreeSet::new();
    for config in &manifest.configurations {
        if config.id.trim().is_empty() || !ids.insert(config.id.clone()) {
            return Err("configuration IDs must be non-empty and unique".into());
        }
        validate_digest(&config.digest)?;
    }
    let referenced_configurations = manifest
        .args
        .iter()
        .filter_map(|argument| match argument {
            Argument::Configuration { id } => Some(id.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    if referenced_configurations.len() != ids.len()
        || referenced_configurations
            .iter()
            .any(|id| !ids.contains(*id))
    {
        return Err(
            "every declared configuration must be passed to the checker exactly by ID".into(),
        );
    }
    if manifest
        .environment_allowlist
        .iter()
        .any(|key| !valid_env_name(key))
    {
        return Err("environment allowlist contains an invalid key".into());
    }
    if manifest.args.iter().any(|arg| match arg {
        Argument::Literal { value } => value.contains('\0'),
        _ => false,
    }) {
        return Err("argv literals may not contain NUL".into());
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<(), String> {
    let valid = value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.chars().all(|character| character.is_ascii_hexdigit())
    });
    if valid {
        Ok(())
    } else {
        Err(format!("invalid SHA-256 digest: {value}"))
    }
}

fn render_args(
    specs: &[Argument],
    inputs: &[PathBuf],
    configs: &BTreeMap<String, PathBuf>,
) -> Result<Vec<String>, String> {
    specs
        .iter()
        .map(|spec| match spec {
            Argument::Literal { value } => Ok(value.clone()),
            Argument::Input { index } => inputs
                .get(*index)
                .map(|path| path.to_string_lossy().into_owned())
                .ok_or_else(|| format!("input argument index {index} is unavailable")),
            Argument::Configuration { id } => configs
                .get(id)
                .map(|path| path.to_string_lossy().into_owned())
                .ok_or_else(|| format!("configuration argument {id} is unavailable")),
        })
        .collect()
}

#[derive(Debug)]
enum PathFailure {
    Unsafe(String),
    Missing(String),
}

impl std::fmt::Display for PathFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsafe(message) | Self::Missing(message) => formatter.write_str(message),
        }
    }
}

fn canonical_repo_path(
    root: &Path,
    relative: &str,
    must_exist: bool,
) -> Result<PathBuf, PathFailure> {
    let path = Path::new(relative);
    if relative.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(PathFailure::Unsafe(format!(
            "path must be a normalized repository-relative path: {relative}"
        )));
    }
    let joined = root.join(path);
    if !must_exist {
        return Ok(joined);
    }
    let canonical = joined.canonicalize().map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            PathFailure::Missing(format!("path is unavailable: {relative}"))
        } else {
            PathFailure::Unsafe(format!("could not canonicalize {relative}: {error}"))
        }
    })?;
    if !canonical.starts_with(root) {
        return Err(PathFailure::Unsafe(format!(
            "path escapes the repository through a symlink: {relative}"
        )));
    }
    Ok(canonical)
}

fn is_shell(executable: &Path) -> bool {
    executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "sh" | "bash"
                    | "dash"
                    | "zsh"
                    | "fish"
                    | "csh"
                    | "tcsh"
                    | "cmd"
                    | "cmd.exe"
                    | "powershell"
                    | "powershell.exe"
                    | "pwsh"
                    | "env"
            )
        })
}

fn valid_env_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphanumeric() && (index > 0 || !byte.is_ascii_digit())
        })
}

pub(crate) struct RunResult {
    pub(crate) status: ExitStatus,
    pub(crate) elapsed: Duration,
    pub(crate) stdout: CapturedOutput,
    pub(crate) stderr: CapturedOutput,
    pub(crate) timed_out: bool,
    pub(crate) cleanup: String,
}

/// Run a literal argv (no shell) with a cleared environment, a process
/// group, a wall-clock bound and bounded output capture.
pub(crate) fn run_bounded(
    executable: &Path,
    argv: &[String],
    cwd: &Path,
    environment: &BTreeMap<String, String>,
    timeout: Duration,
    stdout_limit: usize,
    stderr_limit: usize,
) -> io::Result<RunResult> {
    let mut command = Command::new(executable);
    command
        .args(argv)
        .current_dir(cwd)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn()?;
    let pid = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("stdout pipe unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("stderr pipe unavailable"))?;
    let exceeded = Arc::new(AtomicBool::new(false));
    let stdout_reader = spawn_reader(stdout, stdout_limit, Arc::clone(&exceeded));
    let stderr_reader = spawn_reader(stderr, stderr_limit, Arc::clone(&exceeded));
    let began = Instant::now();
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if began.elapsed() >= timeout || exceeded.load(Ordering::Relaxed) {
            timed_out = began.elapsed() >= timeout;
            terminate_process_tree(&mut child, pid);
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(5));
    };
    // A checker may leave descendants holding inherited pipes. Kill the fresh
    // process group after its leader exits so reader joins remain bounded.
    let cleanup = cleanup_process_group(pid);
    let stdout = stdout_reader
        .join()
        .map_err(|_| io::Error::other("stdout reader panicked"))??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| io::Error::other("stderr reader panicked"))??;
    Ok(RunResult {
        status,
        elapsed: began.elapsed(),
        stdout,
        stderr,
        timed_out,
        cleanup,
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    exceeded: Arc<AtomicBool>,
) -> thread::JoinHandle<io::Result<CapturedOutput>> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(limit.min(16 * 1024));
        let mut chunk = [0_u8; 8 * 1024];
        let mut truncated = false;
        loop {
            let read = reader.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            let remaining = limit.saturating_sub(bytes.len());
            bytes.extend_from_slice(&chunk[..read.min(remaining)]);
            if read > remaining {
                truncated = true;
                exceeded.store(true, Ordering::Relaxed);
                break;
            }
        }
        Ok(CapturedOutput {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            bytes_captured: bytes.len(),
            truncated,
        })
    })
}

fn terminate_process_tree(child: &mut Child, pid: u32) {
    #[cfg(unix)]
    unsafe {
        kill(-(pid as i32), 9);
    }
    let _ = child.kill();
}

fn cleanup_process_group(pid: u32) -> String {
    #[cfg(unix)]
    {
        // SAFETY: a negative, freshly spawned process-group ID targets only the
        // checker group established by `CommandExt::process_group(0)`.
        unsafe {
            kill(-(pid as i32), 9);
        }
        "hard:unix-process-group-sigkill".into()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        "advisory:direct-child-only-on-this-platform".into()
    }
}

#[cfg(unix)]
unsafe extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
}

fn redact<'a>(text: &mut String, values: impl Iterator<Item = &'a String>) {
    for value in values.filter(|value| value.len() >= 3) {
        if text.contains(value) {
            *text = text.replace(value, "[REDACTED]");
        }
    }
}

fn receipt(
    manifest: &CheckerManifest,
    manifest_digest: String,
    state: ExecutionState,
    reason_code: &str,
    summary: String,
    process_started: bool,
    started: Instant,
) -> ExecutionReceipt {
    ExecutionReceipt {
        schema: RECEIPT_SCHEMA.into(),
        checker_id: manifest.checker_id.clone(),
        checker_version: manifest.checker_version.clone(),
        manifest_digest,
        execution_grant_reference: None,
        authority_revision: None,
        authority_checked_at: None,
        state,
        reason_code: reason_code.into(),
        summary,
        process_started,
        executable_digest: None,
        consumed_configurations: Vec::new(),
        exit_code: None,
        elapsed_ms: started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        stdout: CapturedOutput::default(),
        stderr: CapturedOutput::default(),
        limits: LimitEvidence {
            timeout_ms: manifest.timeout_ms,
            stdout_bytes: manifest.stdout_limit_bytes,
            stderr_bytes: manifest.stderr_limit_bytes,
            timeout_mechanism: "hard:parent-wall-clock-and-process-termination".into(),
            output_mechanism: "hard:bounded-pipe-readers".into(),
            filesystem_network_memory_process_limits:
                "advisory:not-an-os-sandbox; use the existing isolated runner".into(),
        },
        process_group_cleanup: "not_started".into(),
        isolation_caveat:
            "The cooperative runner is not a security boundary against the operating-system owner."
                .into(),
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(all(test, unix))]
mod tests {
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};

    use tempfile::TempDir;

    use super::*;
    use crate::domain::{PrincipalKind, PrincipalRef};

    #[derive(Default)]
    struct FixtureVerifier(BTreeMap<String, VerifiedExecutionGrant>);

    impl ExecutionAuthorityVerifier for FixtureVerifier {
        fn verify_execution_grant(
            &self,
            evidence: &ExecutionApprovalEvidence,
            _target: &ExecutionGrantTarget,
        ) -> Result<VerifiedExecutionGrant, ExecutionAuthorizationError> {
            self.0
                .get(&evidence.locator)
                .cloned()
                .ok_or(ExecutionAuthorizationError::Rejected)
        }
    }

    fn principal() -> PrincipalRef {
        PrincipalRef {
            kind: PrincipalKind::LocalUser,
            stable_id: "owner-1".into(),
            display_name: None,
        }
    }

    fn fixture(script: &str) -> (TempDir, CheckerManifest, ExecutionRequest) {
        let temp = TempDir::new().expect("temp repo");
        fs::create_dir_all(temp.path().join("tools")).expect("tools");
        fs::create_dir_all(temp.path().join("src")).expect("src");
        fs::write(temp.path().join("src/input.txt"), "good\n").expect("input");
        fs::write(temp.path().join("checker.conf"), "strict=true\n").expect("config");
        let executable = temp.path().join("tools/checker");
        fs::write(&executable, script).expect("script");
        let mut permissions = fs::metadata(&executable).expect("metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).expect("permissions");
        let manifest = CheckerManifest {
            schema: MANIFEST_SCHEMA.into(),
            checker_id: "fixture.native".into(),
            checker_version: "1.0.0".into(),
            source: ManifestSource::Imported {
                source_digest: sha256_bytes(b"fixture import"),
            },
            executable: "tools/checker".into(),
            executable_digest: file_digest(&executable).expect("executable digest"),
            args: vec![
                Argument::Literal {
                    value: "$(touch should-not-run)".into(),
                },
                Argument::Configuration {
                    id: "strict".into(),
                },
                Argument::Input { index: 0 },
            ],
            working_directory: ".".into(),
            configurations: vec![ConfigurationBinding {
                id: "strict".into(),
                path: "checker.conf".into(),
                digest: file_digest(&temp.path().join("checker.conf")).expect("config digest"),
            }],
            accepted_input_scopes: vec!["src".into()],
            environment_allowlist: BTreeSet::from(["CHECK_TOKEN".into()]),
            timeout_ms: 1_000,
            stdout_limit_bytes: 4_096,
            stderr_limit_bytes: 4_096,
            violation_exit_codes: BTreeSet::from([1]),
            citations: vec!["https://example.invalid/checker".into()],
        };
        let request = ExecutionRequest {
            inputs: vec!["src/input.txt".into()],
            environment: BTreeMap::from([("CHECK_TOKEN".into(), "super-secret".into())]),
        };
        (temp, manifest, request)
    }

    fn grant(manifest: &CheckerManifest) -> VerifiedExecutionGrant {
        VerifiedExecutionGrant {
            grant_reference: "decision:1".into(),
            manifest_digest: manifest_digest(manifest).expect("manifest digest"),
            principal: principal(),
            authority_revision: 7,
            authority_checked_at: "2026-09-09T10:00:00Z".into(),
            expires_at: "2026-09-10T10:00:00Z".into(),
        }
    }

    fn service(grant: VerifiedExecutionGrant) -> TrustedExecutionService<FixtureVerifier> {
        TrustedExecutionService::new(FixtureVerifier(BTreeMap::from([("grant-1".into(), grant)])))
    }

    fn execute_authorized(
        project_root: &Path,
        manifest: &CheckerManifest,
        request: &ExecutionRequest,
    ) -> ExecutionReceipt {
        service(grant(manifest)).execute(
            project_root,
            manifest,
            Some(&ExecutionApprovalEvidence {
                locator: "grant-1".into(),
            }),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            request,
        )
    }

    #[test]
    fn imported_manifest_is_inert_until_exact_digest_is_approved() {
        let (temp, mut manifest, request) = fixture("#!/bin/sh\ntouch executed\nexit 0\n");
        let empty = TrustedExecutionService::new(FixtureVerifier::default());
        let receipt = empty.execute(
            temp.path(),
            &manifest,
            None,
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &request,
        );
        assert_eq!(receipt.state, ExecutionState::NeedsExecutionApproval);
        assert!(!receipt.process_started);
        assert!(!temp.path().join("executed").exists());

        let accepted = grant(&manifest);
        manifest.args.push(Argument::Literal {
            value: "changed".into(),
        });
        let receipt = service(accepted).execute(
            temp.path(),
            &manifest,
            Some(&ExecutionApprovalEvidence {
                locator: "grant-1".into(),
            }),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &request,
        );
        assert_eq!(receipt.reason_code, "execution_grant_mismatch");
        assert!(!temp.path().join("executed").exists());
    }

    #[test]
    fn forged_stale_or_wrong_principal_grants_never_execute() {
        let (temp, manifest, request) = fixture("#!/bin/sh\ntouch executed\n");
        let evidence = ExecutionApprovalEvidence {
            locator: "self-reported".into(),
        };
        let empty = TrustedExecutionService::new(FixtureVerifier::default());
        let rejected = empty.execute(
            temp.path(),
            &manifest,
            Some(&evidence),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &request,
        );
        assert_eq!(rejected.reason_code, "execution_grant_rejected");

        let mut wrong_principal = grant(&manifest);
        wrong_principal.principal.stable_id = "someone-else".into();
        let denied = service(wrong_principal).execute(
            temp.path(),
            &manifest,
            Some(&ExecutionApprovalEvidence {
                locator: "grant-1".into(),
            }),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &request,
        );
        assert_eq!(denied.reason_code, "execution_grant_mismatch");

        let expired = service(grant(&manifest)).execute(
            temp.path(),
            &manifest,
            Some(&ExecutionApprovalEvidence {
                locator: "grant-1".into(),
            }),
            &principal(),
            7,
            "2026-09-11T12:00:00Z",
            &request,
        );
        assert_eq!(expired.reason_code, "execution_grant_mismatch");
        assert!(!temp.path().join("executed").exists());
    }

    #[test]
    fn argv_is_literal_environment_is_redacted_and_config_is_evidenced() {
        let (temp, manifest, request) = fixture(
            "#!/bin/sh\nprintf '%s\\n' \"$1\"\nprintf '%s\\n' \"$CHECK_TOKEN\"\ntest -f \"$2\" && test -f \"$3\"\n",
        );
        let receipt = execute_authorized(temp.path(), &manifest, &request);
        assert_eq!(receipt.state, ExecutionState::Success, "{receipt:#?}");
        assert!(receipt.stdout.text.contains("$(touch should-not-run)"));
        assert!(receipt.stdout.text.contains("[REDACTED]"));
        assert!(!receipt.stdout.text.contains("super-secret"));
        assert!(!temp.path().join("should-not-run").exists());
        assert_eq!(receipt.consumed_configurations.len(), 1);
    }

    #[test]
    fn symlink_escape_and_unallowlisted_environment_fail_without_execution() {
        let (temp, mut manifest, mut request) = fixture("#!/bin/sh\ntouch executed\n");
        let outside = TempDir::new().expect("outside");
        fs::write(outside.path().join("checker"), "#!/bin/sh\nexit 0\n").expect("outside file");
        fs::remove_file(temp.path().join("tools/checker")).expect("remove checker");
        symlink(
            outside.path().join("checker"),
            temp.path().join("tools/checker"),
        )
        .expect("symlink");
        manifest.executable_digest = file_digest(&outside.path().join("checker")).expect("digest");
        let receipt = execute_authorized(temp.path(), &manifest, &request);
        assert_eq!(receipt.reason_code, "unsafe_executable_path");
        assert!(!receipt.process_started);

        let (temp, manifest, _) = fixture("#!/bin/sh\ntouch executed\n");
        request
            .environment
            .insert("UNAPPROVED".into(), "value".into());
        let receipt = execute_authorized(temp.path(), &manifest, &request);
        assert_eq!(receipt.reason_code, "environment_denied");
        assert!(!temp.path().join("executed").exists());

        let (temp, mut manifest, request) = fixture("#!/bin/sh\ntouch executed\n");
        let outside = TempDir::new().expect("outside config");
        fs::write(outside.path().join("checker.conf"), "strict=true\n").expect("config");
        fs::remove_file(temp.path().join("checker.conf")).expect("remove config");
        symlink(
            outside.path().join("checker.conf"),
            temp.path().join("checker.conf"),
        )
        .expect("config symlink");
        manifest.configurations[0].digest =
            file_digest(&outside.path().join("checker.conf")).expect("digest");
        let receipt = execute_authorized(temp.path(), &manifest, &request);
        assert_eq!(receipt.reason_code, "unsafe_configuration_path");
        assert!(!temp.path().join("executed").exists());
    }

    #[test]
    fn timeout_and_output_overflow_are_unknown_and_bounded() {
        let (temp, mut manifest, request) = fixture("#!/bin/sh\nwhile :; do :; done\n");
        manifest.timeout_ms = 50;
        let receipt = execute_authorized(temp.path(), &manifest, &request);
        assert_eq!(receipt.reason_code, "execution_timed_out");
        assert!(receipt.elapsed_ms < 2_000);
        assert!(receipt.process_group_cleanup.contains("process-group"));

        let (temp, mut manifest, request) =
            fixture("#!/bin/sh\nwhile :; do printf 'xxxxxxxxxxxxxxxx'; done\n");
        manifest.stdout_limit_bytes = 128;
        let receipt = execute_authorized(temp.path(), &manifest, &request);
        assert_eq!(receipt.reason_code, "output_limit_exceeded");
        assert_eq!(receipt.stdout.bytes_captured, 128);
        assert!(receipt.stdout.truncated);
    }

    #[test]
    fn missing_or_changed_executable_never_becomes_success() {
        let (temp, manifest, request) = fixture("#!/bin/sh\nexit 0\n");
        let accepted = grant(&manifest);
        fs::remove_file(temp.path().join("tools/checker")).expect("remove");
        let receipt = service(accepted).execute(
            temp.path(),
            &manifest,
            Some(&ExecutionApprovalEvidence {
                locator: "grant-1".into(),
            }),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &request,
        );
        assert_eq!(receipt.state, ExecutionState::Unavailable);

        let (temp, mut changed, request) = fixture("#!/bin/sh\nexit 0\n");
        let accepted = grant(&changed);
        fs::write(temp.path().join("tools/checker"), "#!/bin/sh\nexit 1\n").expect("change");
        let receipt = service(accepted).execute(
            temp.path(),
            &changed,
            Some(&ExecutionApprovalEvidence {
                locator: "grant-1".into(),
            }),
            &principal(),
            7,
            "2026-09-09T12:00:00Z",
            &request,
        );
        assert_eq!(receipt.reason_code, "executable_changed");
        assert_eq!(receipt.state, ExecutionState::NeedsExecutionApproval);
        changed.executable = "/bin/sh".into();
        let receipt = execute_authorized(temp.path(), &changed, &request);
        assert_eq!(receipt.reason_code, "unsafe_executable_path");
    }
}
