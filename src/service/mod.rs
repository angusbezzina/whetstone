//! Shared command services used by the CLI and the dashboard: one typed
//! request and one versioned response envelope per workflow.
//!
//! Each workflow lives in its own module: `init`, `dash`, `change` and
//! `check`; `record` builds the records and receipts they write. `wh push`
//! and `wh pull` are in `crate::sync`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    ActorKind, AgreementRecord, ContentDigest, Enforcer, RuleExample, RuleSource, Strength,
    SweepClass,
};
use crate::storage::{ProjectLayout, StorageError};

mod change;
mod check;
mod dash;
mod export;
mod init;
pub(crate) mod record;

pub use record::{agreement_record, digest_bytes, utc_timestamp};

/// Environment variables agent hosts set in the shells they run. Any of them
/// makes the caller an agent, whatever else is true.
pub const AGENT_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CODEX_SANDBOX",
    "CODEX_THREAD_ID",
    "CURSOR_AGENT",
    "GEMINI_CLI",
];

/// Who is calling: a person at a terminal, or an agent. Anything that is not
/// clearly a person (no terminal on stdin, an agent host's marker in the
/// environment, or a launcher's `VIRGIL_VIA=agent:<client>`) is an agent, so
/// an agent can never pass as a person by leaving something out.
pub fn detect_actor() -> ActorKind {
    use std::io::IsTerminal;
    let marked = AGENT_MARKERS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        || std::env::var("VIRGIL_VIA").is_ok_and(|via| via.starts_with("agent:"));
    if !marked && std::io::stdin().is_terminal() {
        ActorKind::Person
    } else {
        ActorKind::Agent
    }
}

pub const RESPONSE_SCHEMA: &str = "whetstone.command-response.v1";

#[derive(Debug, Clone)]
pub enum ServiceRequest {
    Orientation,
    Init(InitRequest),
    Dash(DashRequest),
    Change(ChangeRequest),
    Check(CheckRequest),
    Pull(crate::sync::PullRequest),
    Push(crate::sync::PushRequest),
}

#[derive(Debug, Clone)]
pub struct DashRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub search: Option<String>,
    pub as_of: Option<String>,
    pub page_size: usize,
    pub expected_snapshot: Option<ContentDigest>,
    /// Include the show-me-your-work TSV decision trail as `data.trail`.
    pub trail: bool,
}

impl DashRequest {
    pub fn basic(project_dir: PathBuf, request_id: Option<String>) -> Self {
        Self {
            project_dir,
            request_id,
            search: None,
            as_of: None,
            page_size: 100,
            expected_snapshot: None,
            trail: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitAction {
    #[default]
    Inspect,
    /// Record the owner's mission, principles and accepted starter rules.
    Agree,
    Cancel,
    /// Install the Git hooks and scaffold CI.
    Wire,
    /// Detect and install the record store's tool.
    Setup,
    /// Choose the record store for every record kind: Beads or Virgil.
    Store,
    /// Write the Beads stores' accepted records as the rulebook file.
    Export,
}

#[derive(Debug, Clone, Default)]
pub struct InitRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub action: InitAction,
    pub expected_revision: Option<u64>,
    pub resume_token: Option<String>,
    pub mission: Option<String>,
    /// Catalogue principle ids the owner picked (agree).
    pub principles: Vec<String>,
    /// The owner's own principles, in their words (agree).
    pub custom_principles: Vec<String>,
    /// Starter rule ids the owner accepts, or `all` (agree).
    pub starters: Vec<String>,
    /// Report exact writes without performing them.
    pub dry_run: bool,
    /// Install the Git pre-commit and pre-push hooks (wire).
    pub hooks: bool,
    /// Scaffold the workflow that runs the required whetstone/policy check (wire).
    pub ci: bool,
    /// Run the install commands setup found (setup); replace a rulebook
    /// file that differs (export).
    pub yes: bool,
    /// The record store (store): `beads` or `virgil`.
    pub store: Option<String>,
    /// Who asked: confirming an import into Virgil is a person's (store).
    pub actor: crate::domain::ActorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Mission,
    Principle,
    Rule,
}

/// Explicit solo review of a pending local draft.
#[derive(Debug, Clone)]
pub struct ReviewRequest {
    pub proposal: String,
    pub verdict: crate::domain::LocalReviewVerdict,
}

/// Always stored boxed (`Option<Box<ChangeDefinition>>`), so the rule
/// variant's size never inflates the requests that carry it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeDefinition {
    Rule {
        strength: Strength,
        enforcer: Enforcer,
        #[serde(default)]
        examples: Vec<RuleExample>,
        #[serde(default)]
        source: Option<RuleSource>,
        #[serde(default)]
        paths: Vec<String>,
        /// How a night sweep works the rule's flags (discuss when absent).
        #[serde(default)]
        sweep: Option<SweepClass>,
        /// How urgent its sweep findings are, 0 (most) to 4 (2 when absent).
        #[serde(default)]
        sweep_priority: Option<u8>,
        /// Where its own flags are doubtful: findings there say `unsure`.
        #[serde(default)]
        sweep_unsure: Vec<crate::domain::SweepUnsure>,
    },
    Principle {
        /// A pstack principle id from the catalogue; absent means custom.
        #[serde(default)]
        pstack: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ChangeRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub kind: Option<ChangeKind>,
    pub record_id: Option<String>,
    pub content: Option<String>,
    pub definition: Option<Box<ChangeDefinition>>,
    /// Change the accountable owner on the proposed record revision.
    pub new_owner: Option<String>,
    pub rationale: Option<String>,
    pub source: Option<String>,
    pub expected_effect: Option<String>,
    pub impact: Option<String>,
    pub examples: Vec<String>,
    pub conflicts: Vec<String>,
    pub expected_revision: Option<u64>,
    pub resume_token: Option<String>,
    pub preview: bool,
    /// Accept or withdraw an existing draft instead of proposing content.
    pub review: Option<ReviewRequest>,
    /// Propose retiring an accepted record (reviewed archival; history stays).
    pub retire: Option<String>,
    /// Who is asking: a person, or an agent (the default, so nothing passes
    /// as a person by omission).
    pub actor: ActorKind,
}

/// Which in-force rules a check runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateMode {
    /// Every in-force rule (or those named by `rules`).
    #[default]
    All,
    /// Rules that apply to what changed.
    Changed,
    /// Pre-commit: mechanical content checks on the staged index only.
    Staged,
    /// Every rule below must, each flag reported as a finding.
    Sweep,
}

#[derive(Debug, Clone, Default)]
pub struct CheckRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub rules: Vec<String>,
    pub gate_mode: GateMode,
    pub timeout_seconds: Option<u64>,
    /// Report the selection and exact commands without executing anything.
    pub dry_run: bool,
    /// The required CI check: enforce the committed rulebook only, refusing
    /// one with uncommitted changes.
    pub ci: bool,
    /// Include paths changed since this revision (a committed change), not
    /// only the working tree.
    pub base: Option<String>,
    /// The commit a push sends (pre-push). The check reads the working tree,
    /// so it refuses unless that commit is checked out with no uncommitted
    /// tracked changes; changed paths then come from commits only.
    pub pushed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Success,
    Violated,
    Unknown,
    Unavailable,
    NeedsDecision,
    NeedsInput,
    Stale,
    Conflict,
}

impl ServiceState {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::Violated => 1,
            Self::Unknown => 3,
            Self::Unavailable => 4,
            Self::NeedsDecision => 5,
            Self::NeedsInput => 6,
            Self::Stale => 7,
            Self::Conflict => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceEvidence {
    pub kind: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checker_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceResponse {
    pub schema: String,
    pub schema_version: u16,
    pub request_id: String,
    pub workflow: String,
    pub state: ServiceState,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_snapshot: Option<RequiredSnapshot>,
    #[serde(default)]
    pub evidence: Vec<ServiceEvidence>,
    #[serde(default)]
    pub blocking_questions: Vec<String>,
    #[serde(default)]
    pub permitted_actions: Vec<String>,
    pub data: Value,
}

impl ServiceResponse {
    /// Construct an envelope for workflows implemented outside this module.
    pub fn new_public(
        request_id: String,
        workflow: &str,
        state: ServiceState,
        summary: impl Into<String>,
    ) -> Self {
        Self::new(request_id, workflow, state, summary)
    }

    pub(crate) fn new(
        request_id: String,
        workflow: &str,
        state: ServiceState,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            schema: RESPONSE_SCHEMA.into(),
            schema_version: 1,
            request_id,
            workflow: workflow.into(),
            state,
            summary: summary.into(),
            expected_revision: None,
            resume_token: None,
            required_snapshot: None,
            evidence: Vec::new(),
            blocking_questions: Vec::new(),
            permitted_actions: Vec::new(),
            data: json!({}),
        }
    }
}

/// Records from the private store and the repository's shared store.
#[derive(Debug, Clone, Default)]
pub struct LoadedRecords {
    pub private: Option<Vec<AgreementRecord>>,
    pub shared: Option<Vec<AgreementRecord>>,
}

/// A private revision that collides with a different shared revision.
#[derive(Debug, Clone, Serialize)]
pub struct SyncConflict {
    pub record: String,
    pub revision: u64,
    pub private_digest: String,
    pub shared_digest: String,
}

impl LoadedRecords {
    pub fn exists(&self) -> bool {
        self.private.is_some()
            || self
                .shared
                .as_ref()
                .is_some_and(|shared| !shared.is_empty())
    }

    /// Shared and private records as one agreement. The same revision in
    /// both stores is one record; a private revision that collides with a
    /// different shared revision is left out and reported, never merged.
    pub fn union(&self) -> (Vec<AgreementRecord>, Vec<SyncConflict>) {
        let shared = self.shared.clone().unwrap_or_default();
        let mut by_slot = std::collections::BTreeMap::new();
        for record in &shared {
            if let Ok(reference) = record.reference() {
                by_slot.insert((record.id.clone(), record.revision), reference.digest);
            }
        }
        let mut records = shared;
        let mut conflicts = Vec::new();
        for record in self.private.clone().unwrap_or_default() {
            let Ok(reference) = record.reference() else {
                continue;
            };
            match by_slot.get(&(record.id.clone(), record.revision)) {
                Some(digest) if *digest == reference.digest => {}
                Some(digest) => conflicts.push(SyncConflict {
                    record: record.id.as_str().into(),
                    revision: record.revision,
                    private_digest: reference.digest.as_str().into(),
                    shared_digest: digest.as_str().into(),
                }),
                None => records.push(record),
            }
        }
        (records, conflicts)
    }
}

/// Latest shared revision of every record in the team's store, as
/// `id@revision#digest` (what `wh push` shared and CI enforces).
pub(crate) fn shared_refs(
    loaded: Option<&LoadedRecords>,
) -> std::collections::BTreeMap<String, String> {
    let mut refs = std::collections::BTreeMap::new();
    for record in loaded
        .and_then(|loaded| loaded.shared.as_ref())
        .into_iter()
        .flatten()
    {
        if let Ok(reference) = record.reference() {
            let slot = refs
                .entry(record.id.as_str().to_string())
                .or_insert_with(|| (0, String::new()));
            if reference.revision >= slot.0 {
                *slot = (
                    reference.revision,
                    crate::projection::reference_text(&reference),
                );
            }
        }
    }
    refs.into_iter().map(|(id, (_, text))| (id, text)).collect()
}

/// Read both stores without creating either.
pub fn load_records(layout: &ProjectLayout) -> Result<LoadedRecords, StorageError> {
    let private = if layout.private_store_exists() {
        Some(crate::storage::open_private(layout)?.all_records()?)
    } else {
        None
    };
    let shared = match crate::storage::open_shared(layout)? {
        Some(store) => Some(store.all_records()?),
        None => None,
    };
    Ok(LoadedRecords { private, shared })
}

#[derive(Debug, Default)]
pub struct CommandService;

impl CommandService {
    pub fn execute(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Orientation => orientation(),
            ServiceRequest::Init(request) => {
                let project_dir = request.project_dir.clone();
                let mut response = init::init(request);
                crate::sync::attach_publication(&mut response, &project_dir);
                response
            }
            ServiceRequest::Dash(request) => dash::dash(request),
            ServiceRequest::Change(request) => {
                let project_dir = request.project_dir.clone();
                let mut response = change::change(request);
                crate::sync::attach_publication(&mut response, &project_dir);
                response
            }
            ServiceRequest::Check(request) => check::check(request),
            ServiceRequest::Pull(request) => crate::sync::pull(request),
            ServiceRequest::Push(request) => crate::sync::push(request),
        }
    }
}

/// Bare `wh`: honest, read-only orientation, including any tool that is
/// missing or out of date and the command that fixes it.
fn orientation() -> ServiceResponse {
    let mut response = ServiceResponse::new(
        "orientation".into(),
        "orientation",
        ServiceState::Success,
        "Whetstone turns your mission and principles into rules every agent is briefed on, checked against and told when to ask.",
    );
    response.permitted_actions = vec![
        "wh init".into(),
        "wh dash".into(),
        "wh change".into(),
        "wh check".into(),
        "wh pull".into(),
        "wh push".into(),
    ];
    let root = std::env::current_dir()
        .ok()
        .and_then(|dir| ProjectLayout::resolve(&dir).ok())
        .map(|layout| layout.project_root().to_path_buf());
    let setup = root.as_deref().map(crate::setup::detect);
    if let Some(setup) = &setup {
        if !setup.ready {
            response.summary.push_str(" Setup needs attention: ");
            response.summary.push_str(
                &setup
                    .tools
                    .iter()
                    .filter(|tool| tool.state != "ok")
                    .map(|tool| format!("{} is {}", tool.tool, tool.state))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            response.summary.push('.');
            response.permitted_actions.extend(setup.fixes.clone());
        }
    }
    response.data = json!({
        "workflows": ["init", "dash", "change", "check", "pull", "push"],
        "available_now": ["init", "dash", "change", "check", "pull", "push"],
        "requires": {"pull": "a shared Beads database with a remote", "push": "a shared Beads database (bd init) and, to transmit, a remote"},
        "tools": setup.as_ref().map(|setup| &setup.tools),
        "fixes": setup.as_ref().map(|setup| &setup.fixes),
        "read_only": true,
    });
    response
}

/// Current UTC time in RFC 3339 form, second precision.
pub(crate) fn utc_now() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| value.ends_with('Z'))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn bounded_input(
    value: Option<String>,
    name: &str,
    maximum: usize,
) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("Provide {name}."))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("Provide non-empty {name}."))
    } else if trimmed.len() > maximum {
        Err(format!("{name} exceeds the {maximum}-byte limit."))
    } else {
        Ok(trimmed.to_string())
    }
}

pub(crate) fn optional_input(
    value: Option<String>,
    name: &str,
    maximum: usize,
    fallback: &str,
) -> Result<String, String> {
    match value.as_deref().map(str::trim) {
        None | Some("") => Ok(fallback.to_string()),
        Some(_) => bounded_input(value, name, maximum),
    }
}

pub(crate) fn bounded_list(
    values: &[String],
    name: &str,
    maximum_items: usize,
    maximum_bytes: usize,
) -> Result<Vec<String>, String> {
    if values.len() > maximum_items {
        return Err(format!("{name} exceeds the {maximum_items}-item limit."));
    }
    values
        .iter()
        .map(|value| bounded_input(Some(value.clone()), name, maximum_bytes))
        .collect()
}

pub(crate) fn bounded_request_id(
    value: Option<String>,
    fallback: String,
) -> Result<String, String> {
    let value = value.unwrap_or(fallback);
    if value.is_empty()
        || value.len() > 128
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
    {
        Err("Request ID must be 1-128 ASCII letters, digits, dots, colons, underscores, or hyphens."
            .into())
    } else {
        Ok(value)
    }
}

pub(crate) fn resume_token(
    project_id: &str,
    workflow: &str,
    request_id: &str,
    revision: u64,
) -> String {
    format!(
        "resume-v1:{:x}",
        Sha256::digest(format!("{project_id}\0{workflow}\0{request_id}\0{revision}").as_bytes())
    )
}

pub(crate) fn needs_input(
    workflow: &str,
    request_id: String,
    revision: u64,
    resume: String,
    question: String,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        workflow,
        ServiceState::NeedsInput,
        "More owner input is required; no agreement change was recorded.",
    );
    response.expected_revision = Some(revision);
    response.resume_token = Some(resume);
    response.blocking_questions = vec![question];
    response.permitted_actions = vec![format!("resume wh {workflow} with the returned token")];
    response
}

pub(crate) fn stale_response(
    workflow: &str,
    request_id: String,
    revision: u64,
    resume: String,
    summary: &str,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(request_id, workflow, ServiceState::Stale, summary);
    response.expected_revision = Some(revision);
    response.resume_token = Some(resume);
    response.permitted_actions = vec![format!(
        "inspect, then resume wh {workflow} at revision {revision}"
    )];
    response
}

pub(crate) fn unknown_response(
    workflow: &str,
    request_id: String,
    summary: String,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(request_id, workflow, ServiceState::Unknown, summary);
    response.permitted_actions = vec![format!(
        "repair the input or environment, then wh {workflow}"
    )];
    response
}

pub(crate) fn project_error(
    workflow: &str,
    request_id: Option<String>,
    error: StorageError,
) -> ServiceResponse {
    unknown_response(
        workflow,
        request_id.unwrap_or_else(|| workflow.into()),
        format!("Project identity could not be resolved: {error}"),
    )
}

pub(crate) fn storage_error(
    workflow: &str,
    request_id: String,
    error: StorageError,
) -> ServiceResponse {
    let state = match error {
        StorageError::StaleRevision { .. } => ServiceState::Stale,
        StorageError::IdempotencyConflict(_) => ServiceState::Conflict,
        StorageError::BeadsMissing | StorageError::UnsupportedBeadsVersion { .. } => {
            ServiceState::Unavailable
        }
        StorageError::ConflictingRevision { .. } => ServiceState::Conflict,
        StorageError::Virgil { ref code, .. } => match code.as_str() {
            "store_unavailable" | "unattached" => ServiceState::Unavailable,
            "not_permitted" | "refused_content" | "read_again" => ServiceState::Conflict,
            _ => ServiceState::Unknown,
        },
        _ => ServiceState::Unknown,
    };
    let mut response = ServiceResponse::new(
        request_id,
        workflow,
        state,
        format!("The operation stopped without partial success: {error}"),
    );
    response.permitted_actions = vec!["inspect storage health and retry the same request".into()];
    response
}

pub(crate) fn domain_response(
    workflow: &str,
    request_id: String,
    error: crate::domain::DomainError,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        workflow,
        ServiceState::Conflict,
        format!("The agreement input violates the domain contract: {error}"),
    );
    response.permitted_actions = vec!["correct the bounded input and submit a new request".into()];
    response
}

pub(crate) fn idempotency_conflict(
    workflow: &str,
    request_id: String,
    key: &str,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        workflow,
        ServiceState::Conflict,
        format!("Request key {key} was already used with different input."),
    );
    response.permitted_actions =
        vec!["inspect the accepted result or submit a new request ID".into()];
    response
}

pub(crate) fn project_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".into())
}

pub(crate) fn latest_record<'a>(
    records: &'a [AgreementRecord],
    id: &str,
) -> Option<&'a AgreementRecord> {
    records
        .iter()
        .filter(|record| record.id.as_str() == id)
        .max_by_key(|record| record.revision)
}

/// The owner's display name for records: Git's user.name, if set.
pub(crate) fn owner_name(project_root: &Path) -> Option<String> {
    Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_states_have_stable_distinct_exit_codes() {
        let codes = [
            ServiceState::Success,
            ServiceState::Violated,
            ServiceState::Unknown,
            ServiceState::Unavailable,
            ServiceState::NeedsDecision,
            ServiceState::NeedsInput,
            ServiceState::Stale,
            ServiceState::Conflict,
        ]
        .map(ServiceState::exit_code);
        let unique = codes.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), codes.len());
    }

    #[test]
    fn push_without_an_agreement_never_claims_an_effect() {
        let temp = tempfile::tempdir().expect("temp");
        assert!(Command::new("git")
            .args(["init", "-q"])
            .current_dir(temp.path())
            .status()
            .expect("git")
            .success());
        let response = CommandService.execute(ServiceRequest::Push(crate::sync::PushRequest {
            project_dir: temp.path().to_path_buf(),
            request_id: Some("push-1".into()),
            ..Default::default()
        }));
        assert_eq!(response.state, ServiceState::NeedsInput);
        assert!(response.summary.contains("nothing was published"));
        assert_eq!(response.data, json!({}));
    }

    #[test]
    fn resume_tokens_bind_workflow_project_and_revision() {
        assert_eq!(
            resume_token("a", "init", "request", 1),
            resume_token("a", "init", "request", 1)
        );
        assert_ne!(
            resume_token("a", "init", "request", 1),
            resume_token("a", "init", "request", 2)
        );
        assert_ne!(
            resume_token("a", "init", "request", 1),
            resume_token("a", "change", "request", 1)
        );
        assert_ne!(
            resume_token("a", "init", "request", 1),
            resume_token("a", "init", "other", 1)
        );
    }
}
