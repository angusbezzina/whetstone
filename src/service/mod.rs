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
    AgreementRecord, ContentDigest, Enforcer, HandRaiseTrigger, HandTrigger, Privacy, RecordId,
    Reviewer, RuleExample, RuleSource, Strength,
};
use crate::storage::{ProjectLayout, RecordStore, StorageError, StoreKind};

mod change;
mod check;
mod dash;
mod init;
pub(crate) mod record;

pub use record::{agreement_record, digest_bytes, utc_timestamp};

/// Storage and domain failures as envelopes, for workflows outside this module.
pub(crate) fn record_storage_error(
    workflow: &str,
    request_id: String,
    error: StorageError,
) -> ServiceResponse {
    storage_error(workflow, request_id, error)
}

pub(crate) fn record_domain_response(
    workflow: &str,
    request_id: String,
    error: crate::domain::DomainError,
) -> ServiceResponse {
    domain_response(workflow, request_id, error)
}

pub const RESPONSE_SCHEMA: &str = "whetstone.command-response.v1";
/// Receipt subject of a reported maintain pass.
pub const MAINTAIN_SUBJECT: &str = "maintain:verification-skill";
pub const DRIVER_EVIDENCE: &str = "whetstone_driver";
pub const DRIVER_CONFIG_EVIDENCE: &str = "whetstone_driver_config";
pub const MAP_EVIDENCE: &str = "whetstone_map";
pub const DOCTOR_EVIDENCE: &str = "whetstone_doctor";

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
    /// Generate the skill, scaffold the driver, install hooks and CI.
    Wire,
    /// Read pstack-edited skill or rule files into private drafts.
    Import,
    /// Detect, install and pin Beads and pstack.
    Setup,
    /// Read an exemplar codebase and draft rules from it, with provenance.
    Exemplar,
}

#[derive(Debug, Clone, Default)]
pub struct InitRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub action: InitAction,
    pub expected_revision: Option<u64>,
    pub resume_token: Option<String>,
    pub mission: Option<String>,
    /// pstack principle ids the owner picked (agree).
    pub principles: Vec<String>,
    /// The owner's own principles, in their words (agree).
    pub custom_principles: Vec<String>,
    /// Starter rule ids the owner accepts, or `all` (agree).
    pub starters: Vec<String>,
    /// Report exact writes without performing them.
    pub dry_run: bool,
    /// Agent hosts to project the verification skill into (wire, setup).
    pub hosts: Vec<String>,
    /// Replace the team-owned driver with a fresh scaffold (wire).
    pub regenerate_driver: bool,
    /// A directory to read: a pstack skill (import) or an exemplar (exemplar).
    pub import_from: Option<PathBuf>,
    /// A Git URL of an exemplar codebase (exemplar).
    pub exemplar_url: Option<String>,
    /// Install agent-host hooks and the Git pre-commit and pre-push hooks (wire).
    pub hooks: bool,
    /// Scaffold the workflow that runs the required whetstone/policy check (wire).
    pub ci: bool,
    /// Run the install commands setup found (setup).
    pub yes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Mission,
    Principle,
    Rule,
    Feature,
    Map,
}

/// Explicit solo review of a pending local draft.
#[derive(Debug, Clone)]
pub struct ReviewRequest {
    pub proposal: String,
    pub verdict: crate::domain::LocalReviewVerdict,
}

/// The owner's label on one flag.
#[derive(Debug, Clone)]
pub struct FlagRequest {
    /// The judgment or gate receipt record id.
    pub receipt: String,
    pub verdict: crate::domain::FlagVerdict,
}

/// Always stored boxed (`Option<Box<ChangeDefinition>>`), so the feature
/// variant's size never inflates the requests that carry it.
#[allow(clippy::large_enum_variant)]
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
        #[serde(default)]
        hand_raise: Vec<HandRaiseTrigger>,
        #[serde(default)]
        privacy: Privacy,
    },
    Principle {
        /// A pstack principle id from the catalogue; absent means custom.
        #[serde(default)]
        pstack: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
    },
    Feature {
        summary: String,
        area: String,
        #[serde(default)]
        sweep_order: u32,
        #[serde(default)]
        sub_features: Vec<String>,
        user_path: String,
        #[serde(default)]
        drive_steps: Vec<String>,
        proof: String,
        #[serde(default)]
        gotchas: Vec<String>,
        #[serde(default)]
        entry_points: Vec<String>,
        #[serde(default)]
        serves: Vec<RecordId>,
        #[serde(default)]
        constrained_by: Vec<RecordId>,
        #[serde(default)]
        proven_by: Vec<RecordId>,
        #[serde(default)]
        index_summary: Option<String>,
        #[serde(default)]
        harness: Option<String>,
        #[serde(default)]
        preconditions: Vec<String>,
        #[serde(default)]
        drive_recipe: Vec<String>,
        #[serde(default)]
        mutations: Vec<crate::domain::Mutation>,
    },
    /// The feature map's conventions (`features/README.md` before the list).
    Map {
        intro: String,
        #[serde(default)]
        baseline_preconditions: Vec<String>,
        #[serde(default)]
        driving_conventions: Vec<String>,
        #[serde(default)]
        proof_reporting: Vec<String>,
        #[serde(default)]
        entry_contract: Option<String>,
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
    /// Accept or dismiss one flag.
    pub flag: Option<FlagRequest>,
    /// Answer a raised hand (a Beads issue id); the answer is the content.
    pub answer: Option<String>,
    /// Draft principles from earlier values and philosophy records.
    pub migrate: bool,
    /// Record the tuning drafts (demotion, promotion) the rules' records suggest.
    pub tune: bool,
    /// The rule a new mechanical rule replaces a recurring Jev flag of.
    pub hardens: Option<String>,
}

/// Which in-force rules a check runs besides the compiled-in scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateMode {
    /// Every in-force rule (or those named by `rules`).
    #[default]
    All,
    /// Rules that apply to what changed, and the features it touched.
    Changed,
    /// Pre-commit: mechanical content checks on the staged index only.
    Staged,
    /// Every drive rule, in feature-map sweep order.
    Sweep,
}

/// A review verdict submitted through `wh check`.
#[derive(Debug, Clone)]
pub struct AttestRequest {
    pub rule: String,
    pub verdict: crate::domain::AttestationVerdict,
    pub reviewer: Reviewer,
    pub notes: String,
    pub evidence: Option<String>,
}

/// A brief an agent recorded before building.
#[derive(Debug, Clone, Default)]
pub struct BriefRequest {
    pub area: String,
    pub skills: Vec<String>,
    pub reuse: Vec<String>,
    pub risks: Vec<String>,
    pub notes: String,
}

/// A question for the owner, filed as a `human` Beads issue.
#[derive(Debug, Clone)]
pub struct HandRequest {
    pub question: String,
    pub tried: String,
    pub recommendation: String,
    pub trigger: HandTrigger,
    pub rule: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CheckRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub paths: Vec<PathBuf>,
    pub language: Option<String>,
    pub rules: Vec<String>,
    /// Feature ids whose proving rules should run.
    pub features: Vec<String>,
    pub gate_mode: GateMode,
    pub timeout_seconds: Option<u64>,
    /// Report the selection and exact commands without executing anything.
    pub dry_run: bool,
    /// Record the outcome of pstack's maintain pass (clean, changed, blocked)
    /// as a receipt instead of running rules.
    pub maintain_outcome: Option<MaintainOutcome>,
    /// The maintain run's notes or PR: a repository path or a URL.
    pub maintain_evidence: Option<String>,
    /// The required CI check: enforce the team's shared, accepted records
    /// only, ignoring any private store and unshared drafts.
    pub ci: bool,
    /// Record an explicit host checkpoint: which skill revision this agent
    /// host has on disk right now.
    pub host: Option<String>,
    /// Include paths changed since this revision (a committed change), not
    /// only the working tree.
    pub base: Option<String>,
    /// With exactly one `--feature`: change-specific drive steps run after
    /// the accepted ones in the same session and recorded in the receipt.
    pub steps: Vec<String>,
    /// Record a review attestation instead of running rules.
    pub attest: Option<AttestRequest>,
    /// Record a brief instead of running rules.
    pub brief: Option<BriefRequest>,
    /// Raise a hand instead of running rules.
    pub raise_hand: Option<HandRequest>,
    /// Also prove each selected feature with its recorded mutations applied,
    /// in an isolated worktree; a proof that still passes is hollow.
    pub mutate: bool,
    /// The commit a push sends (pre-push). The check reads the working tree,
    /// so it refuses unless that commit is checked out with no uncommitted
    /// tracked changes; changed paths then come from commits only.
    pub pushed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintainOutcome {
    Clean,
    Changed,
    Blocked,
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
        Some(
            RecordStore::open_existing(&layout.private_store(), StoreKind::Private)?
                .all_records()?,
        )
    } else {
        None
    };
    let shared = match layout.shared_store() {
        Some(dir) => Some(RecordStore::open_existing(&dir, StoreKind::Shareable)?.all_records()?),
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
            ServiceRequest::Init(request) => init::init(request),
            ServiceRequest::Dash(request) => dash::dash(request),
            ServiceRequest::Change(request) => change::change(request),
            ServiceRequest::Check(request) => {
                let host = request.host.clone();
                let project_dir = request.project_dir.clone();
                let mut response = check::check(request);
                if let Some(host) = host {
                    record::attach_host_checkpoint(&mut response, &project_dir, &host);
                }
                response
            }
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
        "Whetstone turns your mission, principles and exemplars into rules every agent is briefed on, checked against and told when to ask.",
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
    let setup = root.as_deref().map(|root| crate::setup::detect(root, &[]));
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
