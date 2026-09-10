//! Shared command services used by CLI and future HTTP adapters.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::agreement::AgreementState;
use crate::check;
use crate::domain::{
    AgreementRecord, AuthorizationAxis, ContentDigest, CoreValue, Enforcement, EvidenceRef,
    ExternalRef, ExternalSystem, Freshness, Guidance, ImplementationPhilosophy, MetricDefinition,
    MetricDirection, Mission, PolicyStateSnapshot, PrincipalKind, PrincipalRef, Proposal,
    ProposalState, Provenance, ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, Scope,
    Standard, StandardStrength, VerificationAxis, VerificationReceipt, SCHEMA_VERSION_V1,
};
use crate::history::{
    AccessBoundary, HistoryCursor, HistoryError, HistoryInspectionRequest, HistoryInspectionService,
};
use crate::onboarding;
use crate::projection;
use crate::storage::{AppendRequest, DoltRepository, ProjectLayout, StorageError, StoreKind};
use crate::verification::{
    self, AttestationState, EvidencePointer, Finding, RequirementKind, SnapshotBinding,
    TrustedEvidenceSet, VerificationEvidence, VerificationPlan, VerificationReport,
    VerificationRequirement, VerificationState, LEAN_BASELINE_REVISION,
};

pub const RESPONSE_SCHEMA: &str = "whetstone.command-response.v1";

#[derive(Debug, Clone)]
pub enum ServiceRequest {
    Orientation,
    Init(InitRequest),
    Dash(DashRequest),
    Change(ChangeRequest),
    Check(CheckRequest),
    Pull(BasicRequest),
    Push(BasicRequest),
}

#[derive(Debug, Clone)]
pub struct BasicRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DashRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub search: Option<String>,
    pub as_of: Option<String>,
    pub history_after: Option<HistoryCursor>,
    pub page_size: usize,
    pub expected_snapshot: Option<ContentDigest>,
}

impl DashRequest {
    pub fn basic(project_dir: PathBuf, request_id: Option<String>) -> Self {
        Self {
            project_dir,
            request_id,
            search: None,
            as_of: None,
            history_after: None,
            page_size: 100,
            expected_snapshot: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitAction {
    #[default]
    Inspect,
    Agree,
    Cancel,
    /// Generate the verification skill and scaffold the driver.
    Wire,
}

#[derive(Debug, Clone, Default)]
pub struct InitRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub action: InitAction,
    pub expected_revision: Option<u64>,
    pub resume_token: Option<String>,
    pub mission: Option<String>,
    pub desired_outcome: Option<String>,
    pub values: Option<String>,
    pub philosophy: Option<String>,
    pub owner: Option<String>,
    pub initial_safeguard: Option<String>,
    pub safeguard_scope: Option<String>,
    pub revision_triggers: Option<String>,
    /// Command that enforces the first gate; recorded as a Standard.
    pub gate_command: Option<String>,
    /// Report exact writes without performing them (agree and wire).
    pub dry_run: bool,
    /// Agent hosts to project the verification skill into (wire).
    pub hosts: Vec<String>,
    /// Replace the team-owned driver with a fresh scaffold (wire).
    pub regenerate_driver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Mission,
    Value,
    Philosophy,
    Metric,
    Guidance,
    Standard,
    Feature,
}

/// Explicit solo review of a pending local draft.
#[derive(Debug, Clone)]
pub struct ReviewRequest {
    pub proposal: String,
    pub verdict: crate::domain::LocalReviewVerdict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeDefinition {
    Metric {
        source_system: String,
        source_locator: String,
        cohort: String,
        window: String,
        direction: MetricDirection,
        threshold: String,
        freshness_seconds: u64,
    },
    Standard {
        strength: StandardStrength,
        enforcement: Enforcement,
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
    pub desired_outcome: Option<String>,
    pub review_triggers: Option<String>,
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
}

#[derive(Debug, Clone)]
struct ChangeNarrative {
    rationale: String,
    source: String,
    expected_effect: String,
    impact: String,
    examples: Vec<String>,
    conflicts: Vec<String>,
}

impl ChangeNarrative {
    fn render(&self, base_revision: u64) -> String {
        format!(
            "Base revision: {base_revision}\nRationale: {}\nSource: {}\nExpected effect: {}\nImpact: {}\nExamples: {}\nConflicts: {}",
            self.rationale,
            self.source,
            self.expected_effect,
            self.impact,
            self.examples.join("; "),
            self.conflicts.join("; ")
        )
    }
}

/// Which in-force gates a check executes besides the compiled-in scanner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateMode {
    /// Every in-force gate (or those named by `rules`).
    #[default]
    All,
    /// Gates proving features whose entry points cover changed paths.
    Changed,
    /// Every drive gate, in feature-map sweep order.
    Sweep,
    /// Scanner only; used inside repair sessions bound to the scanner snapshot.
    None,
}

#[derive(Debug, Clone, Default)]
pub struct CheckRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub paths: Vec<PathBuf>,
    pub language: Option<String>,
    pub rules: Vec<String>,
    /// Feature ids whose proving gates should run.
    pub features: Vec<String>,
    pub gate_mode: GateMode,
    pub timeout_seconds: Option<u64>,
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

    fn new(
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

#[derive(Debug, Default)]
pub struct CommandService;

impl CommandService {
    pub fn execute(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Orientation => self.orientation(),
            ServiceRequest::Init(request) => self.init(request),
            ServiceRequest::Dash(request) => self.dash(request),
            ServiceRequest::Change(request) => self.change(request),
            ServiceRequest::Check(request) => self.check(request),
            ServiceRequest::Pull(request) => unavailable(
                request,
                "pull",
                "Team synchronization is not implemented yet; no remote data was read.",
                "Continue locally or complete M2.1 with an authenticated team remote.",
            ),
            ServiceRequest::Push(request) => unavailable(
                request,
                "push",
                "Team synchronization is not implemented yet; nothing was published.",
                "Keep the draft local or complete M2.1 and independent approval wiring.",
            ),
        }
    }

    fn orientation(&self) -> ServiceResponse {
        let mut response = ServiceResponse::new(
            "orientation".into(),
            "orientation",
            ServiceState::Success,
            "Whetstone keeps project agreements, checks, and decisions inspectable.",
        );
        response.permitted_actions = vec![
            "wh init".into(),
            "wh dash".into(),
            "wh change".into(),
            "wh check".into(),
            "wh pull".into(),
            "wh push".into(),
        ];
        response.data = json!({
            "workflows": ["init", "dash", "change", "check", "pull", "push"],
            "available_now": ["init", "dash", "change", "check"],
            "unavailable_until_milestone": {"pull": "M2.1", "push": "M2.1"},
            "read_only": true,
            "lean_baseline_revision": "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48",
        });
        response
    }

    fn dash(&self, request: DashRequest) -> ServiceResponse {
        let layout = match ProjectLayout::resolve(&request.project_dir, None) {
            Ok(layout) => layout,
            Err(error) => return project_error("dash", request.request_id, error),
        };
        let request_id = match bounded_request_id(
            request.request_id,
            format!("dash-{}", &layout.project_id()[..16]),
        ) {
            Ok(value) => value,
            Err(summary) => return unknown_response("dash", "invalid-request-id".into(), summary),
        };
        let setup = match onboarding::inspect(layout.project_root()) {
            Ok(value) => value,
            Err(error) => {
                return unknown_response(
                    "dash",
                    request_id,
                    format!("Project inspection failed without changing state: {error:?}"),
                )
            }
        };
        let as_of = request.as_of.unwrap_or_else(utc_now);
        let page_size = request.page_size.clamp(1, 200);
        let private_store_exists = layout.store_path(StoreKind::Private).exists();
        let history = HistoryInspectionService::open(&layout).and_then(|service| {
            service.inspect(&HistoryInspectionRequest {
                project: format!("project-{}", &layout.project_id()[..12]),
                as_of,
                access: AccessBoundary::PrivateStore,
                search: request.search,
                history_after: request.history_after,
                page_size,
                expected_snapshot: request.expected_snapshot,
                redact_private_before: None,
            })
        });
        let progress = onboarding_progress(&layout);
        let mut response = ServiceResponse::new(
            request_id,
            "dash",
            ServiceState::Success,
            "Read-only project shape and decision history are ready.",
        );
        response.permitted_actions = vec!["wh change".into(), "wh check".into()];
        let (history, history_state, history_detail) = match history {
            Ok(history) => (Some(history), "available", None),
            Err(error) if !private_store_exists => {
                (None, "not_initialized", Some(error.to_string()))
            }
            Err(error @ HistoryError::StaleSnapshot { .. }) => {
                response.state = ServiceState::Stale;
                response.summary =
                    "Decision history changed; refresh before continuing this exact view.".into();
                (None, "stale", Some(error.to_string()))
            }
            Err(error) => {
                response.state = ServiceState::Unknown;
                response.summary =
                    "Decision history is unavailable; no current-state claim was inferred.".into();
                (None, "unavailable", Some(error.to_string()))
            }
        };
        let (progress, progress_state, progress_detail) = match progress {
            Ok(progress) => (Some(progress), "available", None),
            Err(error) => (None, "unavailable", Some(error.to_string())),
        };
        let agreement_state = if private_store_exists {
            DoltRepository::open_existing(
                &layout.store_path(StoreKind::Private),
                StoreKind::Private,
            )
            .and_then(|repository| repository.all_records())
            .map(|records| Some(AgreementState::from_records(records)))
        } else {
            Ok(Some(AgreementState::default()))
        };
        let current_projection = match (progress.as_ref(), agreement_state) {
            (Some(progress), Ok(Some(state))) => {
                let fingerprint = crate::gates::workspace_fingerprint(layout.project_root()).ok();
                let evidence_root = crate::gates::evidence_root(&layout);
                let view = projection::dashboard_view(
                    &state,
                    &projection::ProjectionInput {
                        project_label: project_label(layout.project_root()),
                        project_root: layout.project_root(),
                        evidence_root: &evidence_root,
                        agreement_complete: progress.agreement_complete,
                        missing_decisions: &progress.missing_decisions,
                        fingerprint: fingerprint.as_deref(),
                        driver_path: crate::gates::driver_path(layout.project_root()),
                        skill: crate::skill::read_manifest(&layout),
                    },
                );
                let journal = projection::journal(&state, history.as_ref());
                Ok(Some((view, journal)))
            }
            (None, _) => Ok(None),
            (_, Ok(None)) => Ok(None),
            (_, Err(error)) => Err(error),
        };
        let (current_projection, journal) = match current_projection {
            Ok(Some((mut view, journal))) => {
                view.latest_change =
                    journal
                        .iter()
                        .find(|entry| entry.kind == "decision")
                        .map(|entry| projection::LatestChange {
                            title: entry.title.clone(),
                            at: entry.recorded_at.clone(),
                        });
                (Some(view), journal)
            }
            Ok(None) => (None, Vec::new()),
            Err(_error) => {
                response.state = ServiceState::Unknown;
                response.summary =
                    "The current local agreement projection is unavailable; no state was inferred."
                        .into();
                response.evidence.push(ServiceEvidence {
                    kind: "current_projection_unavailable".into(),
                    locator: layout.store_path(StoreKind::Private).display().to_string(),
                    digest: None,
                });
                (None, Vec::new())
            }
        };
        if let Some(progress) = &progress {
            response.blocking_questions = progress
                .missing_decisions
                .iter()
                .map(|decision| format!("What should the project's {decision} be?"))
                .collect();
        }
        let changelog = journal;
        response.data = json!({
            "setup": setup,
            "progress": progress,
            "progress_state": progress_state,
            "progress_detail": progress_detail,
            "history": history,
            "history_state": history_state,
            "history_detail": history_detail,
            "changelog": changelog,
            "current": current_projection,
            "workflows": [
                {"name": "init", "state": "available", "effect": "inspect or establish private agreement"},
                {"name": "dash", "state": "available", "effect": "inspect local system and history"},
                {"name": "change", "state": "available", "effect": "propose a bounded local change"},
                {"name": "check", "state": "available", "effect": "verify and return repair feedback"},
                {"name": "pull", "state": "unavailable", "effect": "no remote changes are received"},
                {"name": "push", "state": "unavailable", "effect": "nothing is published"}
            ],
            "lean_baseline_revision": LEAN_BASELINE_REVISION,
            "read_only": true
        });
        response
    }

    fn init(&self, request: InitRequest) -> ServiceResponse {
        let layout = match ProjectLayout::resolve(&request.project_dir, None) {
            Ok(layout) => layout,
            Err(error) => return project_error("init", request.request_id, error),
        };
        let request_id = match bounded_request_id(
            request.request_id.clone(),
            format!("init-{}", &layout.project_id()[..16]),
        ) {
            Ok(request_id) => request_id,
            Err(summary) => return unknown_response("init", "invalid-request-id".into(), summary),
        };
        let setup = match onboarding::inspect(layout.project_root()) {
            Ok(value) => value,
            Err(error) => {
                return unknown_response(
                    "init",
                    request_id,
                    format!("Project inspection failed without changing state: {error:?}"),
                )
            }
        };
        // Agreement is the only onboarding action authorized to create or
        // resume private storage. Doing this before the read-side progress
        // query makes an exact retry recover a process stop between `dolt
        // init` and schema migration; inspect and cancel remain write-free.
        if request.action == InitAction::Wire {
            return crate::skill::wire(&layout, request_id, &request);
        }
        let private = if request.action == InitAction::Agree && !request.dry_run {
            match DoltRepository::initialize(
                &layout.store_path(StoreKind::Private),
                StoreKind::Private,
            ) {
                Ok(repository) => Some(repository),
                Err(error) => return storage_error("init", request_id, error),
            }
        } else {
            None
        };
        let progress = match onboarding_progress(&layout) {
            Ok(value) => value,
            Err(error) if request.action == InitAction::Cancel => {
                let mut response = ServiceResponse::new(
                    request_id,
                    "init",
                    ServiceState::Success,
                    "Onboarding was cancelled without repairing or changing incomplete private state, project files, integrations, or remotes.",
                );
                response.permitted_actions = vec!["wh init".into()];
                response.data = json!({
                    "setup": setup,
                    "progress": null,
                    "progress_state": "unavailable",
                    "progress_detail": error.to_string(),
                    "cancelled": true,
                    "writes": [],
                    "shared": false,
                });
                return response;
            }
            Err(error) => return storage_error("init", request_id, error),
        };
        let current_revision = progress.agreement_revision;
        let resume = resume_token(layout.project_id(), "init", &request_id, current_revision);
        if request.action == InitAction::Cancel {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::Success,
                "Onboarding was cancelled without changing project files, agreement state, integrations, or remotes.",
            );
            response.expected_revision = Some(current_revision);
            response.permitted_actions = vec!["wh init".into()];
            response.data = json!({
                "setup": setup,
                "progress": progress,
                "cancelled": true,
                "writes": [],
                "shared": false,
            });
            return response;
        }
        if request.action == InitAction::Inspect {
            return onboarding_inspection_response(request_id, resume, setup, progress);
        }
        let private = private.or_else(|| {
            DoltRepository::open_existing(
                &layout.store_path(StoreKind::Private),
                StoreKind::Private,
            )
            .ok()
        });
        let existing_records = match private
            .as_ref()
            .map(|private| init_idempotent_records(private, &request_id))
            .transpose()
        {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => return storage_error("init", request_id, error),
        };
        let retrying_same_request = !existing_records.is_empty();
        if progress.agreement_complete && !retrying_same_request {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::NeedsDecision,
                "Initialization cannot replace an existing mission; use the change workflow.",
            );
            response.expected_revision = Some(current_revision);
            response.permitted_actions = vec!["wh change --kind mission".into()];
            return response;
        }
        let target_revision = if retrying_same_request {
            match init_request_base_revision(&existing_records, &request_id) {
                Ok(Some(revision)) => revision,
                Ok(None) => existing_records
                    .iter()
                    .filter_map(|record| record.supersedes.as_ref().map(|prior| prior.revision))
                    .max()
                    .unwrap_or(0),
                Err(summary) => return unknown_response("init", request_id, summary),
            }
        } else {
            current_revision
        };
        let expected_resume =
            resume_token(layout.project_id(), "init", &request_id, target_revision);
        if request.expected_revision != Some(target_revision)
            || request.resume_token.as_deref() != Some(expected_resume.as_str())
        {
            return stale_response(
                "init",
                request_id,
                current_revision,
                resume,
                "The onboarding answer does not target the current project revision.",
            );
        }
        let stored_records = match private
            .as_ref()
            .map(DoltRepository::all_records)
            .transpose()
        {
            Ok(records) => records.unwrap_or_default(),
            Err(error) => return storage_error("init", request_id, error),
        };
        let current_mission = latest_record(&stored_records, "mission.project");
        let current_values = latest_record(&stored_records, "value.core");
        let current_philosophy = latest_record(&stored_records, "philosophy.implementation");
        let current_safeguard = latest_record(&stored_records, "guidance.initial-safeguard");
        let mission_input = request.mission.or_else(|| match &current_mission?.body {
            RecordBody::Mission(body) => Some(body.statement.clone()),
            _ => None,
        });
        let desired_outcome_input =
            request
                .desired_outcome
                .or_else(|| match &current_mission?.body {
                    RecordBody::Mission(body) => body.desired_outcomes.first().cloned(),
                    _ => None,
                });
        let values_input = request.values.or_else(|| match &current_values?.body {
            RecordBody::CoreValue(body) => Some(body.description.clone()),
            _ => None,
        });
        let philosophy_input = request
            .philosophy
            .or_else(|| match &current_philosophy?.body {
                RecordBody::ImplementationPhilosophy(body) => Some(body.statement.clone()),
                _ => None,
            });
        let owner_input = request
            .owner
            .or_else(|| current_mission?.owner.display_name.clone());
        let safeguard_input =
            request
                .initial_safeguard
                .or_else(|| match &current_safeguard?.body {
                    RecordBody::Guidance(body) => Some(body.statement.clone()),
                    _ => None,
                });
        let safeguard_scope_input = request.safeguard_scope.or_else(|| {
            let RecordBody::Guidance(body) = &current_safeguard?.body else {
                return None;
            };
            body.rationale
                .strip_prefix("Owner-confirmed initial safeguard scope: ")
                .map(str::to_owned)
        });
        let revision_triggers_input =
            request
                .revision_triggers
                .or_else(|| match &current_philosophy?.body {
                    RecordBody::ImplementationPhilosophy(body) => {
                        body.review_triggers.first().cloned()
                    }
                    _ => None,
                });
        let (
            mission,
            desired_outcome,
            values,
            philosophy,
            owner,
            initial_safeguard,
            safeguard_scope,
            revision_triggers,
        ) = match (
            bounded_input(mission_input, "mission", 250),
            bounded_input(desired_outcome_input, "desired outcome", 500),
            bounded_input(values_input, "values", 500),
            bounded_input(philosophy_input, "philosophy", 1000),
            bounded_input(owner_input, "accountable owner", 200),
            bounded_input(safeguard_input, "initial safeguard", 500),
            bounded_input(safeguard_scope_input, "initial safeguard scope", 500),
            bounded_input(revision_triggers_input, "revision triggers", 1000),
        ) {
            (
                Ok(mission),
                Ok(desired_outcome),
                Ok(values),
                Ok(philosophy),
                Ok(owner),
                Ok(initial_safeguard),
                Ok(safeguard_scope),
                Ok(revision_triggers),
            ) => (
                mission,
                desired_outcome,
                values,
                philosophy,
                owner,
                initial_safeguard,
                safeguard_scope,
                revision_triggers,
            ),
            (
                mission,
                desired_outcome,
                values,
                philosophy,
                owner,
                initial_safeguard,
                safeguard_scope,
                revision_triggers,
            ) => {
                let question = mission
                    .err()
                    .or_else(|| desired_outcome.err())
                    .or_else(|| values.err())
                    .or_else(|| philosophy.err())
                    .or_else(|| owner.err())
                    .or_else(|| initial_safeguard.err())
                    .or_else(|| safeguard_scope.err())
                    .or_else(|| revision_triggers.err())
                    .unwrap_or_else(|| "Required agreement input is missing.".into());
                return needs_input("init", request_id, current_revision, resume, question);
            }
        };
        let gate_command = match request
            .gate_command
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(command) => match crate::gates::parse_command(command) {
                Ok(_) if command.len() <= 1_000 => Some(command.to_string()),
                Ok(_) => {
                    return needs_input(
                        "init",
                        request_id,
                        current_revision,
                        resume,
                        "The gate command exceeds 1000 bytes.".into(),
                    )
                }
                Err(error) => {
                    return needs_input("init", request_id, current_revision, resume, error)
                }
            },
            None => None,
        };
        let initial_gate = gate_command.as_ref().map(|command| {
            agreement_record(
                &layout,
                projection::INITIAL_GATE_ID,
                format!("{request_id}:base-{target_revision}:gate"),
                RecordBody::Standard(Standard {
                    statement: initial_safeguard.clone(),
                    rationale: format!("Owner-confirmed first gate for {safeguard_scope}."),
                    strength: StandardStrength::Must,
                    enforcement: Enforcement::Test {
                        command_ref: command.clone(),
                    },
                    examples: Vec::new(),
                }),
                Some(&owner),
            )
        });
        let records = [
            agreement_record(
                &layout,
                "mission.project",
                format!("{request_id}:base-{target_revision}:mission"),
                RecordBody::Mission(Mission {
                    statement: mission,
                    desired_outcomes: vec![desired_outcome],
                }),
                Some(&owner),
            ),
            agreement_record(
                &layout,
                "value.core",
                format!("{request_id}:base-{target_revision}:values"),
                RecordBody::CoreValue(CoreValue {
                    name: "Core values".into(),
                    description: values,
                }),
                Some(&owner),
            ),
            agreement_record(
                &layout,
                "philosophy.implementation",
                format!("{request_id}:base-{target_revision}:philosophy"),
                RecordBody::ImplementationPhilosophy(ImplementationPhilosophy {
                    statement: philosophy,
                    rationale: "Confirmed by the project owner during onboarding.".into(),
                    review_triggers: vec![revision_triggers],
                }),
                Some(&owner),
            ),
            agreement_record(
                &layout,
                "guidance.initial-safeguard",
                format!("{request_id}:base-{target_revision}:safeguard"),
                RecordBody::Guidance(Guidance {
                    statement: initial_safeguard,
                    rationale: format!(
                        "Owner-confirmed initial safeguard scope: {safeguard_scope}"
                    ),
                    examples: Vec::new(),
                }),
                Some(&owner),
            ),
        ];
        let mut records = match records
            .into_iter()
            .chain(initial_gate)
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(records) => records,
            Err(error) => return domain_response("init", request_id, error),
        };
        if retrying_same_request {
            records.retain(|record| {
                existing_records
                    .iter()
                    .any(|existing| existing.idempotency_key == record.idempotency_key)
            });
        } else {
            records.retain(|record| match record.id.as_str() {
                "mission.project" => progress.missing_decisions.iter().any(|decision| {
                    matches!(
                        decision.as_str(),
                        "mission" | "desired outcome" | "accountable owner"
                    )
                }),
                "value.core" => progress
                    .missing_decisions
                    .iter()
                    .any(|decision| decision == "core values"),
                "philosophy.implementation" => progress.missing_decisions.iter().any(|decision| {
                    matches!(
                        decision.as_str(),
                        "implementation philosophy" | "revision triggers"
                    )
                }),
                "guidance.initial-safeguard" => progress.missing_decisions.iter().any(|decision| {
                    matches!(
                        decision.as_str(),
                        "initial safeguard" | "initial safeguard scope"
                    )
                }),
                projection::INITIAL_GATE_ID => {
                    latest_record(&stored_records, projection::INITIAL_GATE_ID).is_none()
                }
                _ => false,
            });
            for record in &mut records {
                let current = latest_record(&stored_records, record.id.as_str());
                record.revision = current.map_or(1, |current| current.revision + 1);
                record.supersedes = match current.map(AgreementRecord::reference) {
                    Some(Ok(reference)) => Some(reference),
                    Some(Err(error)) => return domain_response("init", request_id, error),
                    None => None,
                };
            }
        }
        let references = if retrying_same_request {
            if existing_records.len() != records.len() {
                return unknown_response(
                    "init",
                    request_id,
                    "A pre-atomic onboarding write is incomplete; inspect it and recover explicitly before continuing.".into(),
                );
            }
            let mut references = Vec::new();
            for record in &records {
                let Some(existing) = existing_records
                    .iter()
                    .find(|existing| existing.id == record.id)
                else {
                    return idempotency_conflict("init", request_id, &record.idempotency_key);
                };
                if existing.body != record.body || existing.owner != record.owner {
                    return idempotency_conflict("init", request_id, &record.idempotency_key);
                }
                match existing.reference() {
                    Ok(reference) => references.push(reference),
                    Err(error) => return domain_response("init", request_id, error),
                }
            }
            references
        } else if request.dry_run {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::NeedsDecision,
                "Dry run: these exact private records would be written on agreement; nothing was written.",
            );
            response.expected_revision = Some(current_revision);
            response.resume_token = Some(resume);
            response.permitted_actions =
                vec!["repeat the same request without --dry-run to record the agreement".into()];
            response.data = json!({
                "dry_run": true,
                "records": records,
                "effects": {
                    "private_store_would_be_created": !layout.store_path(StoreKind::Private).exists(),
                    "private_record_write_on_confirm": true,
                    "team_share": false,
                    "platform_configuration_writes": [],
                },
            });
            return response;
        } else {
            let Some(private) = private.as_ref() else {
                return unknown_response(
                    "init",
                    request_id,
                    "The private store is unavailable after initialization.".into(),
                );
            };
            let requests = records
                .iter()
                .map(|record| AppendRequest {
                    record,
                    expected_revision: record.supersedes.as_ref().map(|prior| prior.revision),
                })
                .collect::<Vec<_>>();
            match private.append_batch(&requests) {
                Ok(references) => references,
                Err(error) => return storage_error("init", request_id, error),
            }
        };
        let progress = match onboarding_progress(&layout) {
            Ok(value) => value,
            Err(error) => return storage_error("init", request_id, error),
        };
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            if progress.setup_complete {
                ServiceState::Success
            } else {
                ServiceState::NeedsInput
            },
            if progress.setup_complete {
                "The private project agreement is installed and the current repair loop is verified."
            } else {
                "The private project agreement is approved and installed; setup remains incomplete until a current repair proof is verified."
            },
        );
        response.expected_revision = Some(progress.agreement_revision);
        response.blocking_questions = Vec::new();
        response.permitted_actions = vec!["wh check".into(), "wh init".into(), "wh change".into()];
        response.data = json!({
            "records": references,
            "setup": setup,
            "progress": progress,
            "shared": false,
            "platform_configuration_writes": [],
        });
        response
    }

    fn change(&self, request: ChangeRequest) -> ServiceResponse {
        let layout = match ProjectLayout::resolve(&request.project_dir, None) {
            Ok(layout) => layout,
            Err(error) => return project_error("change", request.request_id, error),
        };
        let request_id = match bounded_request_id(
            request.request_id.clone(),
            format!("change-{}", &layout.project_id()[..16]),
        ) {
            Ok(request_id) => request_id,
            Err(summary) => {
                return unknown_response("change", "invalid-request-id".into(), summary)
            }
        };
        let private = match DoltRepository::initialize(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        ) {
            Ok(repository) => repository,
            Err(error) => return storage_error("change", request_id, error),
        };
        if let Some(review) = request.review.clone() {
            return review_draft(&layout, &private, request_id, &review, &request);
        }
        let id_text = request.record_id.as_deref().unwrap_or("change.pending");
        let record_id = match RecordId::new(id_text) {
            Ok(id) => id,
            Err(error) => return domain_response("change", request_id, error),
        };
        let current = match private.latest(&record_id) {
            Ok(current) => current,
            Err(error) => return storage_error("change", request_id, error),
        };
        let current_revision = current.as_ref().map_or(0, |record| record.revision);
        let resume = resume_token(
            layout.project_id(),
            "change",
            &format!("{request_id}:{id_text}"),
            current_revision,
        );
        let missing = [
            ("--kind", request.kind.is_none()),
            ("--record-id", request.record_id.is_none()),
            ("--content", request.content.is_none()),
            ("--rationale", request.rationale.is_none()),
        ]
        .into_iter()
        .filter_map(|(flag, absent)| absent.then_some(flag))
        .collect::<Vec<_>>();
        if !missing.is_empty() || request.expected_revision.is_none() {
            let question = if missing.is_empty() {
                format!(
                    "Base revision {current_revision} is ready. Repeat the same request with --expected-revision {current_revision} --resume {resume} to record it (add --dry-run to preview)."
                )
            } else {
                format!(
                    "Missing {}. Then repeat with --expected-revision {current_revision} --resume {resume}.",
                    missing.join(", ")
                )
            };
            let mut response = needs_input(
                "change",
                request_id,
                current_revision,
                resume.clone(),
                question,
            );
            response.permitted_actions = vec![format!(
                "wh change --request-id {} --kind <kind> --record-id {} --content <text> --rationale <why> --expected-revision {current_revision} --resume {resume}",
                response.request_id, id_text
            )];
            response.data = json!({
                "current_record": current,
                "base_revision": current_revision,
                "effects": {
                    "private_record_write": true,
                    "private_proposal_write": true,
                    "team_share": false,
                    "team_activation": false,
                    "platform_configuration_write": false,
                    "requires_later_review_to_share": true
                }
            });
            return response;
        }
        let narrative = match change_narrative(&request) {
            Ok(narrative) => narrative,
            Err(question) => {
                return needs_input("change", request_id, current_revision, resume, question)
            }
        };
        let mut body = match change_body(&request, id_text) {
            Ok(body) => body,
            Err(question) => {
                return needs_input("change", request_id, current_revision, resume, question)
            }
        };
        preserve_agreement_companions(
            &mut body,
            current.as_ref(),
            request.desired_outcome.is_none(),
            request.review_triggers.is_none(),
        );
        let existing = match private.by_idempotency_key(&request_id) {
            Ok(existing) => existing,
            Err(error) => return storage_error("change", request_id, error),
        };
        if let Some(existing) = existing.as_ref() {
            if existing.id != record_id || existing.body != body {
                return idempotency_conflict("change", request_id.clone(), &request_id);
            }
        }
        if existing.is_none()
            && (request.expected_revision != Some(current_revision)
                || request.resume_token.as_deref() != Some(resume.as_str()))
        {
            return stale_response(
                "change",
                request_id,
                current_revision,
                resume,
                "The change targets a stale agreement revision.",
            );
        }
        if request.preview {
            if existing.is_some() {
                return idempotency_conflict("change", request_id.clone(), &request_id);
            }
            let mut response = ServiceResponse::new(
                request_id,
                "change",
                ServiceState::NeedsDecision,
                "Review the exact local proposal and its bound base revision before recording it.",
            );
            response.expected_revision = Some(current_revision);
            response.resume_token = Some(resume);
            response.blocking_questions = vec![
                "Does this exact before-and-after change express the intended decision and consequences?"
                    .into(),
            ];
            response.permitted_actions = vec![
                "repeat this exact request with preview=false to record it".into(),
                "cancel without changing project state".into(),
            ];
            response.data = json!({
                "preview_only": true,
                "record_id": record_id,
                "base_revision": current_revision,
                "diff": {
                    "before": current.as_ref().map(|record| &record.body),
                    "after": &body,
                },
                "explanation": {
                    "rationale": narrative.rationale,
                    "source": narrative.source,
                    "expected_effect": narrative.expected_effect,
                    "impact": narrative.impact,
                    "examples": narrative.examples,
                    "conflicts": narrative.conflicts,
                    "owner": current.as_ref().map(|record| &record.owner),
                },
                "effects": {
                    "private_record_write_on_confirm": true,
                    "private_proposal_write_on_confirm": true,
                    "team_share": false,
                    "team_activation": false,
                    "platform_configuration_write": false,
                    "requires_later_review_to_share": true
                }
            });
            return response;
        }
        let idempotent_replay = existing.is_some();
        let base_revision = existing
            .as_ref()
            .map_or(current_revision, |record| record.revision.saturating_sub(1));
        let (record, reference) = if let Some(existing) = existing {
            let reference = match existing.reference() {
                Ok(reference) => reference,
                Err(error) => return domain_response("change", request_id, error),
            };
            (existing, reference)
        } else {
            let new_owner = request
                .new_owner
                .as_deref()
                .map(|owner| bounded_input(Some(owner.to_string()), "accountable owner", 200))
                .transpose();
            let new_owner = match new_owner {
                Ok(owner) => owner,
                Err(question) => {
                    return needs_input("change", request_id, current_revision, resume, question)
                }
            };
            let owner_display = new_owner.as_deref().or_else(|| {
                current
                    .as_ref()
                    .and_then(|record| record.owner.display_name.as_deref())
            });
            let mut record =
                match agreement_record(&layout, id_text, request_id.clone(), body, owner_display) {
                    Ok(record) => record,
                    Err(error) => return domain_response("change", request_id, error),
                };
            record.revision = current_revision + 1;
            record.supersedes = match current.as_ref().map(AgreementRecord::reference) {
                Some(Ok(reference)) => Some(reference),
                Some(Err(error)) => return domain_response("change", request_id, error),
                None => None,
            };
            record.provenance.sources = vec![EvidenceRef {
                system: "owner_selected_source".into(),
                locator: narrative.source.clone(),
                digest: None,
            }];
            let reference =
                match private.append(&record, (current_revision > 0).then_some(current_revision)) {
                    Ok(reference) => reference,
                    Err(error) => return storage_error("change", request_id, error),
                };
            (record, reference)
        };
        let proposal = match ensure_local_change_proposal(
            &private,
            &record,
            &reference,
            &narrative,
            base_revision,
            &request_id,
        ) {
            Ok(reference) => reference,
            Err(error) => return storage_error("change", request_id, error),
        };
        let before = match record.supersedes.as_ref() {
            Some(previous) => match private.get(previous) {
                Ok(previous) => previous.map(|record| record.body),
                Err(error) => return storage_error("change", request_id, error),
            },
            None => None,
        };
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::Success,
            if idempotent_replay {
                "The existing local draft was returned; no duplicate was created."
            } else {
                "The private draft was recorded. It is not in force until you accept it; nothing was shared."
            },
        );
        response.expected_revision = Some(record.revision);
        response.permitted_actions = vec![
            format!("wh change --accept {}", proposal.id.as_str()),
            format!("wh change --withdraw {}", proposal.id.as_str()),
        ];
        response.data = json!({
            "record": reference,
            "proposal": proposal,
            "shared": false,
            "recorded": true,
            "idempotent_replay": idempotent_replay,
            "base_revision": base_revision,
            "diff": {"before": before, "after": record.body},
            "explanation": {
                "rationale": narrative.rationale,
                "source": narrative.source,
                "expected_effect": narrative.expected_effect,
                "impact": narrative.impact,
                "examples": narrative.examples,
                "conflicts": narrative.conflicts,
                "owner": record.owner,
            }
        });
        response
    }

    fn check(&self, request: CheckRequest) -> ServiceResponse {
        if request.paths.len() > 64 || request.rules.len() > 64 || request.features.len() > 64 {
            return unknown_response(
                "check",
                "invalid-request".into(),
                "A check accepts at most 64 paths, 64 rule filters and 64 features.".into(),
            );
        }
        if request
            .paths
            .iter()
            .any(|path| path.as_os_str().to_string_lossy().len() > 4096)
            || request
                .rules
                .iter()
                .chain(request.features.iter())
                .any(|rule| rule.is_empty() || rule.len() > 256)
            || request.language.as_ref().is_some_and(|language| {
                language.is_empty()
                    || language.len() > 32
                    || !language
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
            })
            || request
                .timeout_seconds
                .is_some_and(|seconds| seconds == 0 || seconds > 3_600)
        {
            return unknown_response(
                "check",
                "invalid-request".into(),
                "A check input exceeds its bound or contains an invalid language, rule, feature or timeout.".into(),
            );
        }
        let project = match request.project_dir.canonicalize() {
            Ok(project) => project,
            Err(error) => {
                return unknown_response(
                    "check",
                    request.request_id.unwrap_or_else(|| "check".into()),
                    format!("Project path is unavailable: {error}"),
                )
            }
        };
        let request_id = match bounded_request_id(
            request.request_id.clone(),
            format!(
                "check-{:x}",
                Sha256::digest(project.to_string_lossy().as_bytes())
            )[..22]
                .to_string(),
        ) {
            Ok(request_id) => request_id,
            Err(summary) => return unknown_response("check", "invalid-request-id".into(), summary),
        };
        let layout = ProjectLayout::resolve(&project, None).ok();
        let agreement = match layout.as_ref() {
            Some(layout) if layout.store_path(StoreKind::Private).join(".dolt").is_dir() => {
                match DoltRepository::open_existing(
                    &layout.store_path(StoreKind::Private),
                    StoreKind::Private,
                )
                .and_then(|repository| repository.all_records())
                {
                    Ok(records) => Some(AgreementState::from_records(records)),
                    Err(error) => return storage_error("check", request_id, error),
                }
            }
            _ => None,
        };
        let selection = match select_gates(agreement.as_ref(), &project, &request) {
            Ok(selection) => selection,
            Err(summary) => return unknown_response("check", request_id, summary),
        };
        let scanner_rules = request
            .rules
            .iter()
            .filter(|rule| !selection.gate_ids.contains(rule.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        let rules_only_name_gates = !request.rules.is_empty() && scanner_rules.is_empty();
        let requested_paths = if request.paths.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            request.paths.clone()
        };
        let (scan_paths, _relative_paths, mut snapshot) = match compute_check_snapshot(
            &project,
            &requested_paths,
            request.language.as_deref(),
            &request.rules,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => return unknown_response("check", request_id, error),
        };
        if !selection.gates.is_empty() {
            snapshot.environment = digest_json(&json!({
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "command_validators": false,
                "owner_accepted_gates": true,
            }));
            snapshot.trust = digest_bytes(
                format!(
                    "{LEAN_BASELINE_REVISION}\0compiled-in-deterministic-scanner\0owner-accepted-gates-bounded-execution"
                )
                .as_bytes(),
            );
        }
        let filter = (!scanner_rules.is_empty()).then_some(scanner_rules.as_slice());
        let result = check::run(check::CheckOptions {
            project_dir: &project,
            rules_dir: None,
            scan_paths: &scan_paths,
            lang_filter: request.language.as_deref(),
            rule_filter: filter,
            execute_command_validators: false,
        });
        let violations = count(&result, "violations_count");
        let configuration_issues = count(&result, "config_issues_count");
        let rules_applied = count(&result, "rules_applied");
        let files_scanned = count(&result, "files_scanned");
        let skipped = array_len(&result, "skipped");
        let warnings = array_len(&result, "warnings");
        let native_state = if violations > 0 {
            AttestationState::Violated
        } else if configuration_issues > 0
            || rules_applied == 0
            || files_scanned == 0
            || skipped > 0
            || warnings > 0
        {
            AttestationState::Unknown
        } else {
            AttestationState::Success
        };
        // With owner-accepted gates, an empty scanner is simply not part of
        // this check rather than an unknown requirement.
        let include_native = !rules_only_name_gates
            && (selection.gates.is_empty() || rules_applied > 0 || violations > 0);
        let native_summary = match native_state {
            AttestationState::Success => format!(
                "The compiled-in scanner applied {rules_applied} rules to {files_scanned} files without violations."
            ),
            AttestationState::Violated => format!(
                "The compiled-in scanner found {violations} violations; incomplete evidence remains visible in the findings."
            ),
            AttestationState::Unknown => format!(
                "The compiled-in scanner could not establish complete evidence ({configuration_issues} configuration issues, {skipped} skipped checks, {warnings} warnings)."
            ),
            AttestationState::Unavailable => {
                "The compiled-in scanner was unavailable.".into()
            }
        };
        let findings = scan_findings(&project, &result);
        let scan_digest = digest_json(&result);
        let mut requirements = Vec::new();
        let mut evidence_items = Vec::new();
        if include_native {
            requirements.push(VerificationRequirement {
                id: "whetstone.native-scan".into(),
                kind: RequirementKind::NativeCheck,
                required: true,
                checker_manifest: Some(snapshot.checker_bundle.clone()),
                governing_records: Vec::new(),
                rationale: "Applicable accepted rules require deterministic local verification."
                    .into(),
                repair_direction: "Repair reported source or checker configuration without changing policy, tests, or baselines to hide the result.".into(),
                permitted_next_action: "repair within the current task scope, then wh check"
                    .into(),
                verification_command: "wh check --json".into(),
                freshness_seconds: 300,
            });
            evidence_items.push(VerificationEvidence::NativeCheck {
                requirement_id: "whetstone.native-scan".into(),
                snapshot: snapshot.clone(),
                state: native_state,
                evidence: EvidencePointer {
                    source: "whetstone-compiled-in-scanner".into(),
                    locator: "local-project".into(),
                    digest: scan_digest.clone(),
                },
                observed_at_unix: unix_now(),
                summary: native_summary,
                findings,
            });
        }
        // Execute selected in-force gates.
        let fingerprint = match crate::gates::workspace_fingerprint(&project) {
            Ok(value) => value,
            Err(error) if !selection.gates.is_empty() => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The working tree could not be fingerprinted, so gate results could not be bound: {error}"),
                )
            }
            Err(_) => String::new(),
        };
        let run_id = {
            let digest = digest_bytes(
                format!(
                    "{request_id}\0{fingerprint}\0{}",
                    selection
                        .gates
                        .iter()
                        .map(|gate| gate.reference.digest.as_str())
                        .collect::<Vec<_>>()
                        .join(",")
                )
                .as_bytes(),
            );
            digest.as_str()["sha256:".len().."sha256:".len() + 16].to_string()
        };
        let mut outcomes = Vec::new();
        let mut doctor = None;
        if let (Some(layout), false) = (layout.as_ref(), selection.gates.is_empty()) {
            let run_dir = crate::gates::evidence_root(layout).join(&run_id);
            if let Err(error) = fs::create_dir_all(&run_dir) {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The evidence directory could not be created: {error}"),
                );
            }
            if selection
                .gates
                .iter()
                .any(|gate| matches!(gate.standard.enforcement, Enforcement::Drive { .. }))
            {
                doctor = Some(crate::gates::run_doctor(&project, &run_dir));
            }
            let context = crate::gates::GateContext {
                project_root: &project,
                run_dir: &run_dir,
                run_id: &run_id,
                features: &selection.features,
                doctor: doctor.as_ref(),
                timeout: request.timeout_seconds.map_or(
                    crate::gates::DEFAULT_GATE_TIMEOUT,
                    std::time::Duration::from_secs,
                ),
            };
            for gate in &selection.gates {
                let outcome = crate::gates::run_gate(&context, &gate.id, &gate.standard);
                let state = match outcome.state {
                    VerificationAxis::Pass => AttestationState::Success,
                    VerificationAxis::Fail => AttestationState::Violated,
                    VerificationAxis::Unknown => AttestationState::Unknown,
                };
                let requirement_id = format!("gate:{}", gate.id.as_str());
                let manifest = gate.reference.digest.clone();
                requirements.push(VerificationRequirement {
                    id: requirement_id.clone(),
                    kind: RequirementKind::NativeCheck,
                    required: gate.standard.strength == StandardStrength::Must,
                    checker_manifest: Some(manifest.clone()),
                    governing_records: vec![gate.reference.clone()],
                    rationale: gate.standard.rationale.clone(),
                    repair_direction: "Repair within the current task scope; never change or weaken the gate to pass it.".into(),
                    permitted_next_action: format!("repair, then wh check --rule {}", gate.id.as_str()),
                    verification_command: format!("wh check --rule {}", gate.id.as_str()),
                    freshness_seconds: 3_600,
                });
                let evidence_digest = outcome
                    .evidence
                    .last()
                    .and_then(|evidence| evidence.digest.clone())
                    .unwrap_or_else(|| digest_bytes(outcome.summary.as_bytes()));
                evidence_items.push(VerificationEvidence::Execution {
                    requirement_id: requirement_id.clone(),
                    snapshot: snapshot.clone(),
                    receipt: Box::new(gate_execution_receipt(gate, &outcome, &manifest)),
                    evidence: EvidencePointer {
                        source: crate::projection::EVIDENCE_SYSTEM.into(),
                        locator: outcome
                            .evidence
                            .last()
                            .map_or_else(|| run_id.clone(), |evidence| evidence.locator.clone()),
                        digest: evidence_digest,
                    },
                    observed_at_unix: unix_now(),
                    findings: outcome
                        .failures
                        .iter()
                        .map(|failure| gate_finding(gate, failure))
                        .collect(),
                });
                let _ = state;
                outcomes.push(outcome);
            }
        }
        if requirements.is_empty() {
            let summary = if selection.skipped_drafts.is_empty() {
                "Nothing ran: no scanner rule or in-force gate applies, which is unknown, not a pass.".to_string()
            } else {
                format!(
                    "Nothing ran: {} is a draft and cannot run until accepted; that is unknown, not a pass.",
                    selection.skipped_drafts.join(", ")
                )
            };
            let mut response =
                ServiceResponse::new(request_id, "check", ServiceState::Unknown, summary);
            response.permitted_actions = if selection.skipped_drafts.is_empty() {
                vec!["record a gate with wh change --kind standard, then accept it".into()]
            } else {
                vec!["accept the draft with wh change --accept <proposal>, then wh check".into()]
            };
            response.data = json!({
                "gates": [],
                "selection": {
                    "mode": format!("{:?}", request.gate_mode).to_ascii_lowercase(),
                    "gates": [],
                    "skipped_drafts": selection.skipped_drafts,
                    "changed_paths": selection.changed_paths,
                    "features_affected": selection.features_affected,
                },
            });
            return response;
        }
        let plan = VerificationPlan {
            subject: format!(
                "project:{:x}:check",
                Sha256::digest(project.to_string_lossy().as_bytes())
            ),
            snapshot: snapshot.clone(),
            requirements,
        };
        let evidence = TrustedEvidenceSet {
            items: evidence_items,
        };
        let report = match verification::aggregate(&plan, &evidence, unix_now(), &[]) {
            Ok(report) => report,
            Err(error) => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("Verification evidence could not be aggregated: {error}"),
                )
            }
        };
        let checked_at = utc_now();
        let (receipt_record, gate_receipts) = match layout.as_ref() {
            Some(layout) => {
                let native = if include_native {
                    match persist_verification_receipt(layout, &request_id, &report, &checked_at) {
                        Ok(reference) => reference,
                        Err(error) => return storage_error("check", request_id, error),
                    }
                } else {
                    None
                };
                let mut gate_receipts = Vec::new();
                for (gate, outcome) in selection.gates.iter().zip(&outcomes) {
                    match persist_gate_receipt(
                        layout,
                        &run_id,
                        gate,
                        outcome,
                        &fingerprint,
                        &checked_at,
                    ) {
                        Ok(Some(reference)) => gate_receipts.push(reference),
                        Ok(None) => {}
                        Err(error) => return storage_error("check", request_id, error),
                    }
                }
                (native, gate_receipts)
            }
            None => (None, Vec::new()),
        };
        let state = service_state(report.state);
        let mut response = ServiceResponse::new(request_id, "check", state, report.human_summary());
        response.required_snapshot = Some(RequiredSnapshot {
            code_digest: Some(report.snapshot.code_tree.as_str().into()),
            policy_digest: Some(report.snapshot.policy.as_str().into()),
            checker_digest: Some(report.snapshot.checker_bundle.as_str().into()),
            scope_digest: Some(report.snapshot.scope.as_str().into()),
            environment_digest: Some(report.snapshot.environment.as_str().into()),
            trust_digest: Some(report.snapshot.trust.as_str().into()),
        });
        response.evidence = vec![ServiceEvidence {
            kind: "deterministic_scan".into(),
            locator: "local-project".into(),
            digest: Some(report.receipt_id.as_str().into()),
        }];
        for outcome in &outcomes {
            for evidence in &outcome.evidence {
                response.evidence.push(ServiceEvidence {
                    kind: format!("gate:{}", outcome.id),
                    locator: evidence.locator.clone(),
                    digest: evidence
                        .digest
                        .as_ref()
                        .map(|digest| digest.as_str().to_string()),
                });
            }
        }
        response.permitted_actions = match state {
            ServiceState::Success => vec!["handoff the verified result".into()],
            ServiceState::Violated => {
                let failing = outcomes
                    .iter()
                    .filter(|outcome| outcome.state == VerificationAxis::Fail)
                    .map(|outcome| format!("repair, then wh check --rule {}", outcome.id))
                    .collect::<Vec<_>>();
                if failing.is_empty() {
                    vec!["repair within the current task scope, then wh check".into()]
                } else {
                    failing
                }
            }
            _ => vec!["restore the required policy, checker or driver, then wh check".into()],
        };
        response.data = json!({
            "report": report,
            "raw_scan": result,
            "scanner_included": include_native,
            "receipt_persisted": receipt_record.is_some() || !gate_receipts.is_empty(),
            "receipt_record": receipt_record,
            "gate_receipts": gate_receipts,
            "gates": outcomes,
            "run_id": if selection.gates.is_empty() { None } else { Some(run_id) },
            "evidence_root": layout.as_ref().map(|layout| crate::gates::evidence_root(layout).display().to_string()),
            "doctor": doctor,
            "selection": {
                "mode": format!("{:?}", request.gate_mode).to_ascii_lowercase(),
                "gates": selection.gates.iter().map(|gate| gate.id.as_str()).collect::<Vec<_>>(),
                "skipped_drafts": selection.skipped_drafts,
                "changed_paths": selection.changed_paths,
                "features_affected": selection.features_affected,
            },
        });
        response
    }
}

fn review_draft(
    layout: &ProjectLayout,
    private: &DoltRepository,
    request_id: String,
    review: &ReviewRequest,
    request: &ChangeRequest,
) -> ServiceResponse {
    use crate::domain::{LocalReview, LocalReviewVerdict, LOCAL_REVIEW_ASSURANCE};
    let key = format!("review:{request_id}");
    match private.by_idempotency_key(&key) {
        Ok(Some(existing)) => {
            let mut response = ServiceResponse::new(
                request_id,
                "change",
                ServiceState::Success,
                "The existing review was returned; no duplicate was created.",
            );
            response.data = json!({"review": existing.reference().ok(), "idempotent_replay": true});
            return response;
        }
        Ok(None) => {}
        Err(error) => return storage_error("change", request_id, error),
    }
    let records = match private.all_records() {
        Ok(records) => records,
        Err(error) => return storage_error("change", request_id, error),
    };
    let state = AgreementState::from_records(records);
    let pending = state.pending_proposals();
    let Some(target) = pending
        .iter()
        .find(|pending| pending.proposal.id.as_str() == review.proposal)
    else {
        let mut response = needs_input(
            "change",
            request_id,
            0,
            String::new(),
            if pending.is_empty() {
                format!(
                    "{} is not a pending draft; nothing awaits review.",
                    review.proposal
                )
            } else {
                format!(
                    "{} is not a pending draft. Pending: {}.",
                    review.proposal,
                    pending
                        .iter()
                        .map(|pending| pending.proposal.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        );
        response.expected_revision = None;
        response.resume_token = None;
        response.data = json!({
            "pending": pending.iter().map(|pending| json!({
                "proposal": pending.proposal.id.as_str(),
                "title": projection::record_title(pending.proposal),
            })).collect::<Vec<_>>(),
        });
        return response;
    };
    let proposal_ref = match target.proposal.reference() {
        Ok(reference) => reference,
        Err(error) => return domain_response("change", request_id, error),
    };
    let rationale = request
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || match review.verdict {
                LocalReviewVerdict::Accept => "Explicit solo acceptance by the owner.".to_string(),
                LocalReviewVerdict::Withdraw => "Withdrawn by the owner.".to_string(),
            },
            |value| value.chars().take(4_000).collect(),
        );
    let reviewer = target.proposal.owner.clone();
    let suffix = &format!("{:x}", Sha256::digest(key.as_bytes()))[..24];
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: match RecordId::new(format!("review.{suffix}")) {
            Ok(id) => id,
            Err(error) => return domain_response("change", request_id, error),
        },
        revision: 1,
        scope: target.proposal.scope.clone(),
        owner: reviewer.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: reviewer.clone(),
            recorded_at: utc_now(),
            sources: vec![EvidenceRef {
                system: "whetstone_cli".into(),
                locator: "explicit_owner_review".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: key,
        body: RecordBody::LocalReview(LocalReview {
            proposal: proposal_ref.clone(),
            verdict: review.verdict,
            reviewer,
            assurance: LOCAL_REVIEW_ASSURANCE.into(),
            rationale: rationale.clone(),
            reviewed_at: utc_now(),
            team_activation_permitted: false,
        }),
    };
    let candidates = target
        .candidates
        .iter()
        .map(|candidate| {
            json!({
                "record": candidate.id.as_str(),
                "version": candidate.revision,
                "title": projection::record_title(candidate),
                "replaces": state.in_force(&candidate.id).map(|current| current.revision),
            })
        })
        .collect::<Vec<_>>();
    let verb = match review.verdict {
        LocalReviewVerdict::Accept => "accept",
        LocalReviewVerdict::Withdraw => "withdraw",
    };
    if request.preview {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            format!("Review the exact effect before you {verb} this draft."),
        );
        response.permitted_actions = vec!["repeat without --dry-run to record the review".into()];
        response.data = json!({
            "preview_only": true,
            "proposal": proposal_ref,
            "verdict": verb,
            "candidates": candidates,
            "effects": {
                "private_record_write_on_confirm": true,
                "becomes_in_force": review.verdict == LocalReviewVerdict::Accept,
                "team_share": false,
                "team_activation": false,
            },
        });
        return response;
    }
    let reference = match private.append(&record, None) {
        Ok(reference) => reference,
        Err(error) => return storage_error("change", request_id, error),
    };
    let _ = layout;
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Success,
        match review.verdict {
            LocalReviewVerdict::Accept => {
                "The draft was accepted locally and is now in force for this private agreement; nothing was shared or activated for a team."
            }
            LocalReviewVerdict::Withdraw => {
                "The draft was withdrawn; the previously accepted record stays in force."
            }
        },
    );
    response.permitted_actions = vec!["wh check".into(), "wh init --action wire".into()];
    response.data = json!({
        "review": reference,
        "proposal": proposal_ref,
        "verdict": verb,
        "candidates": candidates,
        "rationale": rationale,
        "shared": false,
        "team_activation": false,
    });
    response
}

fn init_idempotent_records(
    repository: &DoltRepository,
    request_id: &str,
) -> Result<Vec<AgreementRecord>, StorageError> {
    let prefix = format!("{request_id}:base-");
    let legacy_keys = ["mission", "values", "philosophy", "safeguard"]
        .map(|suffix| format!("{request_id}:{suffix}"));
    let records = repository
        .all_records()?
        .into_iter()
        .filter(|record| {
            record.idempotency_key.starts_with(&prefix)
                || legacy_keys.contains(&record.idempotency_key)
        })
        .collect();
    Ok(records)
}

fn init_request_base_revision(
    records: &[AgreementRecord],
    request_id: &str,
) -> Result<Option<u64>, String> {
    let prefix = format!("{request_id}:base-");
    let mut revisions = records
        .iter()
        .filter_map(|record| record.idempotency_key.strip_prefix(&prefix))
        .map(|tail| {
            tail.split_once(':')
                .and_then(|(revision, _)| revision.parse::<u64>().ok())
                .ok_or_else(|| {
                    "Stored onboarding request has an invalid base revision.".to_string()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    revisions.sort_unstable();
    revisions.dedup();
    if revisions.len() > 1 {
        return Err("Stored onboarding request spans conflicting base revisions.".into());
    }
    Ok(revisions.into_iter().next())
}

#[derive(Debug, Clone, Serialize)]
struct OnboardingProgress {
    discovery: onboarding::SetupState,
    agreement: onboarding::SetupState,
    private_store: onboarding::SetupState,
    feedback_loop: onboarding::SetupState,
    agreement_revision: u64,
    missing_decisions: Vec<String>,
    agreement_records: Vec<crate::domain::RecordRef>,
    repair_proof: Option<crate::domain::RecordRef>,
    proof_status: String,
    agreement_complete: bool,
    setup_complete: bool,
    shared: bool,
    platform_configuration_writes: Vec<String>,
    lean_baseline_revision: String,
}

fn onboarding_progress(layout: &ProjectLayout) -> Result<OnboardingProgress, StorageError> {
    let store_path = layout.store_path(StoreKind::Private);
    if !store_path.is_dir() {
        return Ok(OnboardingProgress {
            discovery: onboarding::SetupState::Detected,
            agreement: onboarding::SetupState::Proposed,
            private_store: onboarding::SetupState::Proposed,
            feedback_loop: onboarding::SetupState::Unavailable,
            agreement_revision: 0,
            missing_decisions: onboarding::inspect(layout.project_root())
                .map_err(|error| StorageError::UnexpectedData(format!("{error:?}")))?
                .owner_decisions,
            agreement_records: Vec::new(),
            repair_proof: None,
            proof_status: "no private agreement exists, so repair proof is not yet applicable"
                .into(),
            agreement_complete: false,
            setup_complete: false,
            shared: false,
            platform_configuration_writes: Vec::new(),
            lean_baseline_revision: LEAN_BASELINE_REVISION.into(),
        });
    }
    let repository = DoltRepository::open_existing(&store_path, StoreKind::Private)?;
    let records = repository.all_records()?;
    let mission = latest_record(&records, "mission.project");
    let values = latest_record(&records, "value.core");
    let philosophy = latest_record(&records, "philosophy.implementation");
    let safeguard = latest_record(&records, "guidance.initial-safeguard");
    let mut missing = Vec::new();
    match mission {
        Some(AgreementRecord {
            owner,
            body: RecordBody::Mission(body),
            ..
        }) => {
            if body.desired_outcomes.is_empty() {
                missing.push("desired outcome".into());
            }
            if owner.display_name.as_deref().map_or(true, str::is_empty) {
                missing.push("accountable owner".into());
            }
        }
        _ => {
            missing.push("mission".into());
            missing.push("desired outcome".into());
            missing.push("accountable owner".into());
        }
    }
    if !matches!(
        values.map(|record| &record.body),
        Some(RecordBody::CoreValue(_))
    ) {
        missing.push("core values".into());
    }
    match philosophy.map(|record| &record.body) {
        Some(RecordBody::ImplementationPhilosophy(body)) if !body.review_triggers.is_empty() => {}
        Some(RecordBody::ImplementationPhilosophy(_)) => missing.push("revision triggers".into()),
        _ => {
            missing.push("implementation philosophy".into());
            missing.push("revision triggers".into());
        }
    }
    match safeguard.map(|record| &record.body) {
        Some(RecordBody::Guidance(body)) => {
            if body
                .rationale
                .strip_prefix("Owner-confirmed initial safeguard scope: ")
                .map_or(true, |scope| scope.trim().is_empty())
            {
                missing.push("initial safeguard scope".into());
            }
        }
        _ => {
            missing.push("initial safeguard".into());
            missing.push("initial safeguard scope".into());
        }
    }
    missing.sort();
    missing.dedup();
    let current_safeguard = safeguard
        .map(AgreementRecord::reference)
        .transpose()
        .map_err(StorageError::Domain)?;
    let agreement_records = [mission, values, philosophy, safeguard]
        .into_iter()
        .flatten()
        .map(AgreementRecord::reference)
        .collect::<Result<Vec<_>, _>>()
        .map_err(StorageError::Domain)?;
    let agreement_revision = agreement_records
        .iter()
        .map(|reference| reference.revision)
        .max()
        .unwrap_or(0);
    let mut repair_proof = None;
    let mut stale_proof_seen = false;
    let mut incomplete_proof_seen = false;
    for record in records.iter().rev() {
        let RecordBody::RepairSession(session) = &record.body else {
            continue;
        };
        if session.state != crate::domain::RepairSessionStateRecord::Verified
            || session.lean_baseline_revision != LEAN_BASELINE_REVISION
        {
            continue;
        }
        if session.attempts.is_empty()
            || !current_safeguard.as_ref().is_some_and(|safeguard| {
                session
                    .applicable_guidance
                    .iter()
                    .any(|reference| reference == safeguard)
            })
        {
            incomplete_proof_seen = true;
            continue;
        }
        if !crate::repair_host::repair_workspace_is_current(layout.project_root(), session)
            .unwrap_or(false)
        {
            stale_proof_seen = true;
            continue;
        }
        let paths = session
            .final_check_paths
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        match compute_check_snapshot(
            layout.project_root(),
            &paths,
            session.final_check_language.as_deref(),
            &session.final_check_rules,
        ) {
            Ok((_, _, snapshot))
                if snapshot_matches_repair(&snapshot, &session.reviewed_snapshot) =>
            {
                repair_proof = Some(record.reference().map_err(StorageError::Domain)?);
                break;
            }
            _ => stale_proof_seen = true,
        }
    }
    let agreement_complete = missing.is_empty();
    let setup_complete = agreement_complete && repair_proof.is_some();
    Ok(OnboardingProgress {
        discovery: onboarding::SetupState::Detected,
        agreement: if agreement_complete {
            onboarding::SetupState::Approved
        } else {
            onboarding::SetupState::Proposed
        },
        private_store: onboarding::SetupState::Installed,
        feedback_loop: if repair_proof.is_some() {
            onboarding::SetupState::Verified
        } else if agreement_complete {
            onboarding::SetupState::Proposed
        } else {
            onboarding::SetupState::Unavailable
        },
        agreement_revision,
        missing_decisions: missing,
        agreement_records,
        repair_proof,
        proof_status: if setup_complete {
            "current exact repair proof verified".into()
        } else if stale_proof_seen {
            "a prior repair proof exists but its code, policy, checker, scope, environment, or trust snapshot is stale".into()
        } else if incomplete_proof_seen {
            "a verified check exists but it does not prove a known-bad repair under the current initial safeguard".into()
        } else {
            "a current known-bad to authorized-repair to known-good proof is still required".into()
        },
        agreement_complete,
        setup_complete,
        shared: false,
        platform_configuration_writes: Vec::new(),
        lean_baseline_revision: LEAN_BASELINE_REVISION.into(),
    })
}

fn snapshot_matches_repair(
    current: &SnapshotBinding,
    reviewed: &crate::domain::RepairSnapshotRecord,
) -> bool {
    current.code_tree == reviewed.code_tree
        && current.policy == reviewed.policy
        && current.checker_bundle == reviewed.checker_bundle
        && current.scope == reviewed.scope
        && current.environment == reviewed.environment
        && current.trust == reviewed.trust
}

fn project_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".into())
}

fn latest_record<'a>(records: &'a [AgreementRecord], id: &str) -> Option<&'a AgreementRecord> {
    records
        .iter()
        .filter(|record| record.id.as_str() == id)
        .max_by_key(|record| record.revision)
}

fn onboarding_inspection_response(
    request_id: String,
    resume: String,
    setup: onboarding::SetupPlan,
    progress: OnboardingProgress,
) -> ServiceResponse {
    let (state, summary) = if progress.setup_complete {
        (
            ServiceState::Success,
            "The private agreement, local installation, and current repair proof are verified.",
        )
    } else if progress.missing_decisions.is_empty() {
        (
            ServiceState::NeedsInput,
            "The agreement is installed locally. An agent must complete one current repair proof before setup is complete.",
        )
    } else {
        (
            ServiceState::NeedsInput,
            "Inspection is complete and read-only; only missing owner decisions are requested.",
        )
    };
    let mut response = ServiceResponse::new(request_id, "init", state, summary);
    response.expected_revision = Some(progress.agreement_revision);
    if state != ServiceState::Success {
        response.resume_token = Some(resume);
    }
    response.evidence = setup
        .facts
        .iter()
        .map(|fact| ServiceEvidence {
            kind: format!("detected_{:?}", fact.kind).to_ascii_lowercase(),
            locator: fact.path.clone(),
            digest: Some(fact.digest.as_str().into()),
        })
        .collect();
    response.blocking_questions = if progress.missing_decisions.is_empty() {
        Vec::new()
    } else {
        progress
            .missing_decisions
            .iter()
            .map(|decision| format!("Provide {decision}."))
            .collect()
    };
    response.permitted_actions = if progress.missing_decisions.is_empty() {
        vec!["wh check".into(), "wh init".into()]
    } else {
        vec!["wh init --action agree with the returned revision, token, and explicit owner decisions".into(), "wh init --action cancel".into()]
    };
    response.data = json!({
        "setup": setup,
        "progress": progress,
        "inspection_writes": [],
        "read_only": true,
    });
    response
}

fn optional_input(
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

fn change_narrative(request: &ChangeRequest) -> Result<ChangeNarrative, String> {
    Ok(ChangeNarrative {
        rationale: bounded_input(request.rationale.clone(), "rationale", 4_000)?,
        source: optional_input(request.source.clone(), "source", 2_048, "owner")?,
        expected_effect: optional_input(
            request.expected_effect.clone(),
            "expected effect",
            4_000,
            "not stated",
        )?,
        impact: optional_input(request.impact.clone(), "impact", 4_000, "not stated")?,
        examples: bounded_list(&request.examples, "examples", 16, 2_000)?,
        conflicts: bounded_list(&request.conflicts, "conflicts", 16, 2_000)?,
    })
}

fn bounded_list(
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

fn ensure_local_change_proposal(
    repository: &DoltRepository,
    candidate: &AgreementRecord,
    candidate_reference: &crate::domain::RecordRef,
    narrative: &ChangeNarrative,
    base_revision: u64,
    request_id: &str,
) -> Result<crate::domain::RecordRef, StorageError> {
    let proposal_key = format!("{request_id}:proposal");
    let proposal_id = RecordId::new(format!(
        "proposal.change_{}",
        &format!("{:x}", Sha256::digest(request_id.as_bytes()))[..24]
    ))
    .map_err(StorageError::Domain)?;
    let proposal = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: proposal_id,
        revision: 1,
        scope: candidate.scope.clone(),
        owner: candidate.owner.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: candidate.owner.clone(),
            recorded_at: utc_now(),
            sources: vec![EvidenceRef {
                system: "owner_selected_source".into(),
                locator: narrative.source.clone(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: proposal_key.clone(),
        body: RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: format!(
                "{} {}: {}",
                projection::kind_label(match &candidate.body {
                    RecordBody::Mission(_) => "mission",
                    RecordBody::CoreValue(_) => "value",
                    RecordBody::ImplementationPhilosophy(_) => "philosophy",
                    RecordBody::Guidance(_) => "guidance",
                    RecordBody::MetricDefinition(_) => "metric",
                    RecordBody::Standard(_) => "standard",
                    RecordBody::Feature(_) => "feature",
                    _ => "record",
                }),
                if candidate.revision <= 1 {
                    "proposed"
                } else {
                    "revised"
                },
                projection::record_title(candidate)
                    .chars()
                    .take(120)
                    .collect::<String>()
            ),
            rationale: narrative.render(base_revision),
            proposed_records: vec![candidate_reference.clone()],
            binding: None,
        }),
    };
    if let Some(existing) = repository.by_idempotency_key(&proposal_key)? {
        if existing.id != proposal.id || existing.body != proposal.body {
            return Err(StorageError::IdempotencyConflict(proposal_key));
        }
        return existing.reference().map_err(StorageError::Domain);
    }
    repository.append(&proposal, None)
}

fn change_body(request: &ChangeRequest, id_text: &str) -> Result<RecordBody, String> {
    let kind = request.kind.ok_or_else(|| "Provide kind.".to_string())?;
    let content = bounded_input(request.content.clone(), "content", 16_000)?;
    let rationale = request
        .rationale
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Owner-authored local change.")
        .trim()
        .to_string();
    Ok(match kind {
        ChangeKind::Mission => RecordBody::Mission(Mission {
            statement: content,
            desired_outcomes: request
                .desired_outcome
                .as_deref()
                .map(|value| bounded_input(Some(value.to_string()), "desired outcome", 500))
                .transpose()?
                .into_iter()
                .collect(),
        }),
        ChangeKind::Value => RecordBody::CoreValue(CoreValue {
            name: id_text.into(),
            description: content,
        }),
        ChangeKind::Philosophy => RecordBody::ImplementationPhilosophy(ImplementationPhilosophy {
            statement: content,
            rationale,
            review_triggers: request
                .review_triggers
                .as_deref()
                .map(|value| bounded_input(Some(value.to_string()), "review triggers", 1_000))
                .transpose()?
                .into_iter()
                .collect(),
        }),
        ChangeKind::Metric => {
            let Some(ChangeDefinition::Metric {
                source_system,
                source_locator,
                cohort,
                window,
                direction,
                threshold,
                freshness_seconds,
            }) = request.definition.as_deref()
            else {
                return Err("Provide the metric source, cohort, window, direction, threshold, and freshness.".into());
            };
            RecordBody::MetricDefinition(MetricDefinition {
                name: content,
                rationale,
                source: EvidenceRef {
                    system: bounded_input(
                        Some(source_system.clone()),
                        "metric source system",
                        200,
                    )?,
                    locator: bounded_input(
                        Some(source_locator.clone()),
                        "metric source locator",
                        2_000,
                    )?,
                    digest: None,
                },
                cohort: bounded_input(Some(cohort.clone()), "metric cohort", 500)?,
                window: bounded_input(Some(window.clone()), "metric window", 500)?,
                direction: *direction,
                threshold: bounded_input(Some(threshold.clone()), "metric threshold", 500)?,
                freshness_seconds: *freshness_seconds,
                expected_release: None,
            })
        }
        ChangeKind::Guidance => RecordBody::Guidance(Guidance {
            statement: content,
            rationale,
            examples: request.examples.clone(),
        }),
        ChangeKind::Standard => {
            let Some(ChangeDefinition::Standard {
                strength,
                enforcement,
            }) = request.definition.as_deref()
            else {
                return Err(
                    "Provide the standard strength and deterministic enforcement mechanism.".into(),
                );
            };
            if let Enforcement::Test { command_ref } | Enforcement::Validator { command_ref } =
                enforcement
            {
                crate::gates::parse_command(command_ref)?;
            }
            RecordBody::Standard(Standard {
                statement: content,
                rationale,
                strength: *strength,
                enforcement: enforcement.clone(),
                examples: request.examples.clone(),
            })
        }
        ChangeKind::Feature => {
            let Some(ChangeDefinition::Feature {
                summary,
                area,
                sweep_order,
                sub_features,
                user_path,
                drive_steps,
                proof,
                gotchas,
                entry_points,
                serves,
                constrained_by,
                proven_by,
            }) = request.definition.as_deref()
            else {
                return Err("Provide the feature summary, area, user path, drive steps, proof and entry points as a feature definition.".into());
            };
            let feature = crate::domain::Feature {
                name: content,
                summary: bounded_input(Some(summary.clone()), "feature summary", 500)?,
                area: bounded_input(Some(area.clone()), "feature area", 120)?,
                sweep_order: *sweep_order,
                sub_features: bounded_list(sub_features, "sub-features", 64, 500)?,
                user_path: bounded_input(Some(user_path.clone()), "user path", 2_000)?,
                drive_steps: bounded_list(drive_steps, "drive steps", 64, 1_000)?,
                proof: bounded_input(Some(proof.clone()), "proof", 1_000)?,
                gotchas: bounded_list(gotchas, "gotchas", 64, 1_000)?,
                entry_points: bounded_list(entry_points, "entry points", 64, 500)?,
                serves: serves.clone(),
                constrained_by: constrained_by.clone(),
                proven_by: proven_by.clone(),
            };
            RecordBody::Feature(feature)
        }
    })
}

fn preserve_agreement_companions(
    body: &mut RecordBody,
    current: Option<&AgreementRecord>,
    preserve_desired_outcomes: bool,
    preserve_review_triggers: bool,
) {
    let Some(current) = current else {
        return;
    };
    match (body, &current.body) {
        (RecordBody::Mission(next), RecordBody::Mission(previous)) if preserve_desired_outcomes => {
            next.desired_outcomes.clone_from(&previous.desired_outcomes);
        }
        (
            RecordBody::ImplementationPhilosophy(next),
            RecordBody::ImplementationPhilosophy(previous),
        ) if preserve_review_triggers => {
            next.review_triggers.clone_from(&previous.review_triggers);
        }
        _ => {}
    }
}

fn unavailable(
    request: BasicRequest,
    workflow: &str,
    summary: &str,
    next: &str,
) -> ServiceResponse {
    let request_id = match bounded_request_id(
        request.request_id,
        format!(
            "{workflow}-{:x}",
            Sha256::digest(request.project_dir.to_string_lossy().as_bytes())
        )[..22]
            .to_string(),
    ) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response(workflow, "invalid-request-id".into(), summary),
    };
    let mut response =
        ServiceResponse::new(request_id, workflow, ServiceState::Unavailable, summary);
    response.blocking_questions = vec![next.into()];
    response.permitted_actions = vec!["inspect local state".into()];
    response
}

fn agreement_record(
    layout: &ProjectLayout,
    id: &str,
    idempotency_key: String,
    body: RecordBody,
    owner_display: Option<&str>,
) -> Result<AgreementRecord, crate::domain::DomainError> {
    let stable_id = format!(
        "local:{}",
        &format!("{:x}", Sha256::digest(layout.project_id().as_bytes()))[..20]
    );
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id,
        display_name: owner_display.map(str::to_owned),
    };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id)?,
        revision: 1,
        scope: Scope {
            organization: None,
            project: format!("project-{}", &layout.project_id()[..12]),
            component: None,
            environment: None,
        },
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: principal,
            recorded_at: utc_now(),
            sources: vec![EvidenceRef {
                system: "whetstone_cli".into(),
                locator: "owner_input".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key,
        body,
    };
    record.validate()?;
    Ok(record)
}

/// Current UTC time in RFC 3339 form, second precision.
pub fn utc_timestamp() -> String {
    utc_now()
}

fn utc_now() -> String {
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

fn bounded_input(value: Option<String>, name: &str, maximum: usize) -> Result<String, String> {
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

fn bounded_request_id(value: Option<String>, fallback: String) -> Result<String, String> {
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

fn resume_token(project_id: &str, workflow: &str, request_id: &str, revision: u64) -> String {
    format!(
        "resume-v1:{:x}",
        Sha256::digest(format!("{project_id}\0{workflow}\0{request_id}\0{revision}").as_bytes())
    )
}

fn needs_input(
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

fn stale_response(
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

fn unknown_response(workflow: &str, request_id: String, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new(request_id, workflow, ServiceState::Unknown, summary);
    response.permitted_actions = vec![format!(
        "repair the input or environment, then wh {workflow}"
    )];
    response
}

fn project_error(
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

fn storage_error(workflow: &str, request_id: String, error: StorageError) -> ServiceResponse {
    let state = match error {
        StorageError::StaleRevision { .. } => ServiceState::Stale,
        StorageError::IdempotencyConflict(_) => ServiceState::Conflict,
        StorageError::DoltMissing | StorageError::UnsupportedDoltVersion { .. } => {
            ServiceState::Unavailable
        }
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

fn domain_response(
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

fn idempotency_conflict(workflow: &str, request_id: String, key: &str) -> ServiceResponse {
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

fn compute_check_snapshot(
    project: &Path,
    requested_paths: &[PathBuf],
    language: Option<&str>,
    rules: &[String],
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, SnapshotBinding), String> {
    let mut scan_paths = Vec::new();
    for path in requested_paths {
        let joined = if path.is_absolute() {
            path.clone()
        } else {
            project.join(path)
        };
        let canonical = joined
            .canonicalize()
            .map_err(|error| format!("A requested check path is unavailable: {error}"))?;
        if !canonical.starts_with(project) {
            return Err("A requested path escapes the project root.".into());
        }
        scan_paths.push(canonical);
    }
    let relative_paths = scan_paths
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(project).unwrap_or(path);
            if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative.to_path_buf()
            }
        })
        .collect::<Vec<_>>();
    let code_tree = verification::code_tree_digest(project, &relative_paths)
        .map_err(|error| format!("The code snapshot could not be bound safely: {error}"))?;
    let policy = digest_rule_inputs(project)
        .map_err(|error| error.to_string())
        .and_then(|digest| ContentDigest::new(digest).map_err(|error| error.to_string()))
        .map_err(|error| format!("The policy snapshot could not be bound safely: {error}"))?;
    let checker_bundle = checker_bundle_digest()
        .map_err(|error| format!("The compiled checker could not be content-bound: {error}"))?;
    let scope = digest_json(&json!({
        "project": project.to_string_lossy(),
        "paths": relative_paths,
        "language": language,
        "rules": rules,
    }));
    let environment = digest_json(&json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "command_validators": false,
    }));
    let trust = digest_bytes(
        format!(
            "{LEAN_BASELINE_REVISION}\0compiled-in-deterministic-scanner\0no-external-execution-receipt"
        )
        .as_bytes(),
    );
    Ok((
        scan_paths,
        relative_paths,
        SnapshotBinding {
            code_tree,
            policy,
            checker_bundle,
            scope,
            environment,
            trust,
        },
    ))
}

fn count(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn array_len(value: &Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(Value::as_array)
        .map_or(0, |items| items.len() as u64)
}

fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    match ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))) {
        Ok(digest) => digest,
        Err(_) => unreachable!("SHA-256 formatting always satisfies the digest contract"),
    }
}

fn digest_json(value: &Value) -> ContentDigest {
    digest_bytes(&serde_json::to_vec(value).unwrap_or_default())
}

fn checker_bundle_digest() -> Result<ContentDigest, String> {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "whetstone:{}:scanner-v1\0",
        env!("CARGO_PKG_VERSION")
    ));
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let bytes = fs::read(executable).map_err(|error| error.to_string())?;
    hasher.update(bytes);
    match ContentDigest::new(format!("sha256:{:x}", hasher.finalize())) {
        Ok(digest) => Ok(digest),
        Err(_) => unreachable!("SHA-256 formatting always satisfies the digest contract"),
    }
}

fn scan_findings(project: &Path, result: &Value) -> Vec<Finding> {
    let mut findings = Vec::new();
    if let Some(violations) = result.get("violations").and_then(Value::as_array) {
        for violation in violations {
            let rule_id = value_text(violation, "rule_id", "whetstone.unknown-rule");
            let description = value_text(
                violation,
                "description",
                "the accepted deterministic rule must be satisfied",
            );
            let source = value_text(violation, "source_url", "accepted project policy");
            let observed = value_text(violation, "match", "the rule matched this location");
            findings.push(Finding {
                file: finding_path(project, violation.get("file")),
                line: violation
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| u32::try_from(line).ok()),
                column: violation
                    .get("column")
                    .and_then(Value::as_u64)
                    .and_then(|column| u32::try_from(column).ok()),
                rule_id,
                observed,
                expected: description.clone(),
                rationale: format!("{description} Governing source: {source}."),
                repair_direction: "Change the reported source within the current task scope so the accepted rule no longer matches.".into(),
                permitted_next_action: "repair the reported source, then rerun the verification command".into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    if let Some(issues) = result.get("config_issues").and_then(Value::as_array) {
        for issue in issues {
            findings.push(Finding {
                file: finding_path(project, issue.get("path")),
                line: None,
                column: None,
                rule_id: value_text(issue, "rule_id", "whetstone.checker-configuration"),
                observed: value_text(issue, "issue", "checker configuration is incomplete"),
                expected: "Required rule and checker configuration must be available and valid."
                    .into(),
                rationale: "Unknown or unavailable required evidence cannot be treated as success."
                    .into(),
                repair_direction: value_text(
                    issue,
                    "fix",
                    "restore the required checker configuration without weakening the policy",
                ),
                permitted_next_action:
                    "repair checker configuration, then rerun the verification command".into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    if let Some(skipped) = result.get("skipped").and_then(Value::as_array) {
        for item in skipped {
            findings.push(Finding {
                file: None,
                line: None,
                column: None,
                rule_id: value_text(item, "rule_id", "whetstone.skipped-check"),
                observed: value_text(item, "reason", "a required check was skipped"),
                expected: "Every applicable required check must produce current evidence.".into(),
                rationale: "Skipped required evidence cannot be treated as success.".into(),
                repair_direction: "Restore a supported deterministic binding for the rule.".into(),
                permitted_next_action:
                    "restore the checker binding, then rerun the verification command".into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    if let Some(warnings) = result.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            findings.push(Finding {
                file: None,
                line: None,
                column: None,
                rule_id: "whetstone.scanner-warning".into(),
                observed: warning.into(),
                expected: "The scanner must inspect the complete requested scope without warnings."
                    .into(),
                rationale: "Partial scanner evidence cannot establish completion.".into(),
                repair_direction: "Resolve the warning without narrowing the requested scope."
                    .into(),
                permitted_next_action: "resolve the warning, then rerun the verification command"
                    .into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    findings
}

fn value_text(value: &Value, field: &str, fallback: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn finding_path(project: &Path, value: Option<&Value>) -> Option<String> {
    let text = value.and_then(Value::as_str)?;
    let path = Path::new(text);
    let relative = if path.is_absolute() {
        path.strip_prefix(project).ok()?
    } else {
        path
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        None
    } else {
        Some(relative.to_string_lossy().into_owned())
    }
}

fn service_state(state: VerificationState) -> ServiceState {
    match state {
        VerificationState::Success => ServiceState::Success,
        VerificationState::Violated => ServiceState::Violated,
        VerificationState::Unavailable => ServiceState::Unavailable,
        VerificationState::Unknown | VerificationState::NeedsExecutionApproval => {
            ServiceState::Unknown
        }
    }
}

fn persist_verification_receipt(
    layout: &ProjectLayout,
    request_id: &str,
    report: &VerificationReport,
    checked_at: &str,
) -> Result<Option<crate::domain::RecordRef>, StorageError> {
    let store_path = layout.store_path(StoreKind::Private);
    if !store_path.join(".dolt").is_dir() {
        return Ok(None);
    }
    let repository = DoltRepository::initialize(&store_path, StoreKind::Private)?;
    let identity = digest_bytes(
        format!(
            "{request_id}\0{}\0{}\0{}\0{}\0{}\0{}",
            report.snapshot.code_tree.as_str(),
            report.snapshot.policy.as_str(),
            report.snapshot.checker_bundle.as_str(),
            report.snapshot.scope.as_str(),
            report.snapshot.environment.as_str(),
            report.snapshot.trust.as_str(),
        )
        .as_bytes(),
    );
    let suffix = &identity.as_str()["sha256:".len().."sha256:".len() + 32];
    let idempotency_key = format!("check:{suffix}");
    if let Some(existing) = repository.by_idempotency_key(&idempotency_key)? {
        return existing.reference().map(Some).map_err(StorageError::Domain);
    }
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "whetstone:compiled-in-checker".into(),
        display_name: None,
    };
    let verification = match report.state {
        VerificationState::Success => VerificationAxis::Pass,
        VerificationState::Violated => VerificationAxis::Fail,
        VerificationState::Unknown
        | VerificationState::Unavailable
        | VerificationState::NeedsExecutionApproval => VerificationAxis::Unknown,
    };
    let freshness =
        if report.results.iter().any(|result| {
            result.required && result.freshness == verification::EvidenceFreshness::Stale
        }) {
            Freshness::Stale
        } else if report.results.iter().any(|result| {
            result.required && result.freshness == verification::EvidenceFreshness::Missing
        }) {
            Freshness::Missing
        } else {
            Freshness::Fresh
        };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(format!("verification.check_{suffix}")).map_err(StorageError::Domain)?,
        revision: 1,
        scope: Scope {
            organization: None,
            project: format!("project-{}", &layout.project_id()[..12]),
            component: None,
            environment: None,
        },
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::DeterministicCheck,
            recorded_by: principal,
            recorded_at: checked_at.into(),
            sources: vec![EvidenceRef {
                system: "whetstone_verification_report".into(),
                locator: report.receipt_id.as_str().into(),
                digest: Some(report.receipt_id.clone()),
            }],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes: None,
        idempotency_key,
        body: RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: format!("project:{}:check", layout.project_id()),
                revision: Some(report.receipt_id.as_str().into()),
            },
            code_digest: report.snapshot.code_tree.clone(),
            policy_state: PolicyStateSnapshot {
                accepted: None,
                required: None,
                installed: None,
                experimental: None,
            },
            verification,
            authorization: AuthorizationAxis::Unknown,
            freshness,
            checked_at: checked_at.into(),
            related_records: report
                .results
                .iter()
                .flat_map(|result| result.governing_records.clone())
                .collect(),
            evidence: vec![EvidenceRef {
                system: "whetstone_verification_report".into(),
                locator: report.receipt_id.as_str().into(),
                digest: Some(report.receipt_id.clone()),
            }],
        }),
    };
    repository.append(&record, None).map(Some)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// An in-force gate chosen for this check.
struct SelectedGate {
    id: RecordId,
    reference: crate::domain::RecordRef,
    standard: Standard,
}

#[derive(Default)]
struct GateSelection {
    gates: Vec<SelectedGate>,
    gate_ids: std::collections::BTreeSet<String>,
    features: std::collections::BTreeMap<RecordId, crate::domain::Feature>,
    skipped_drafts: Vec<String>,
    changed_paths: Vec<String>,
    features_affected: Vec<Value>,
}

fn select_gates(
    agreement: Option<&AgreementState>,
    project: &Path,
    request: &CheckRequest,
) -> Result<GateSelection, String> {
    let mut selection = GateSelection::default();
    let Some(state) = agreement else {
        if !request.features.is_empty() {
            return Err("No private agreement exists, so no feature can be proven yet.".into());
        }
        return Ok(selection);
    };
    for id in state.agreement_ids(|body| matches!(body, RecordBody::Feature(_))) {
        if let Some(record) = state.in_force(&id) {
            if let RecordBody::Feature(feature) = &record.body {
                selection.features.insert(id, feature.clone());
            }
        }
    }
    let standard_ids = state.agreement_ids(|body| matches!(body, RecordBody::Standard(_)));
    for id in &standard_ids {
        selection.gate_ids.insert(id.as_str().to_string());
    }
    if request.gate_mode == GateMode::None {
        return Ok(selection);
    }
    let named = request
        .rules
        .iter()
        .filter(|rule| selection.gate_ids.contains(rule.as_str()))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut wanted: Option<std::collections::BTreeSet<String>> =
        (!named.is_empty()).then_some(named);
    let mut feature_filter = request.features.clone();
    if request.gate_mode == GateMode::Changed {
        let changed = crate::gates::changed_paths(project)?;
        for (id, feature) in &selection.features {
            let touched = changed
                .iter()
                .filter(|path| feature.covers_path(path))
                .cloned()
                .collect::<Vec<_>>();
            if !touched.is_empty() {
                feature_filter.push(id.as_str().to_string());
                selection.features_affected.push(json!({
                    "feature": id.as_str(),
                    "name": feature.name,
                    "paths": touched,
                    "map_review": "Behaviour in these paths changed; confirm the feature entry still describes it, or revise it with wh change --kind feature.",
                }));
            }
        }
        selection.changed_paths = changed;
        if feature_filter.is_empty() {
            return Ok(selection);
        }
    }
    if !feature_filter.is_empty() {
        let mut ids = wanted.take().unwrap_or_default();
        for feature_id in &feature_filter {
            let id = RecordId::new(feature_id.as_str())
                .map_err(|_| format!("{feature_id} is not a valid feature id."))?;
            let Some(feature) = selection.features.get(&id) else {
                return Err(format!(
                    "Feature {feature_id} is not in force; accept its draft before proving it."
                ));
            };
            ids.extend(
                feature
                    .proven_by
                    .iter()
                    .map(|gate| gate.as_str().to_string()),
            );
            // Drive gates that name this feature prove it even when unlisted.
            for gate_id in &standard_ids {
                if let Some(RecordBody::Standard(standard)) =
                    state.in_force(gate_id).map(|record| &record.body)
                {
                    if matches!(&standard.enforcement, Enforcement::Drive { feature } if feature == &id)
                    {
                        ids.insert(gate_id.as_str().to_string());
                    }
                }
            }
        }
        wanted = Some(ids);
    }
    for id in &standard_ids {
        if wanted
            .as_ref()
            .is_some_and(|wanted| !wanted.contains(id.as_str()))
        {
            continue;
        }
        let Some(record) = state.in_force(id) else {
            selection.skipped_drafts.push(id.as_str().to_string());
            continue;
        };
        let RecordBody::Standard(standard) = &record.body else {
            continue;
        };
        if request.gate_mode == GateMode::Sweep
            && !matches!(standard.enforcement, Enforcement::Drive { .. })
        {
            continue;
        }
        let reference = record.reference().map_err(|error| error.to_string())?;
        selection.gates.push(SelectedGate {
            id: id.clone(),
            reference,
            standard: standard.clone(),
        });
    }
    if request.gate_mode == GateMode::Sweep {
        let order = |gate: &SelectedGate| match &gate.standard.enforcement {
            Enforcement::Drive { feature } => selection
                .features
                .get(feature)
                .map_or((String::from("~"), u32::MAX), |feature| {
                    (feature.area.clone(), feature.sweep_order)
                }),
            _ => (String::from("~"), u32::MAX),
        };
        let mut keyed = selection
            .gates
            .drain(..)
            .map(|gate| (order(&gate), gate))
            .collect::<Vec<_>>();
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        selection.gates = keyed.into_iter().map(|(_, gate)| gate).collect();
    }
    if let Some(wanted) = wanted {
        for name in wanted {
            if !selection.gate_ids.contains(name.as_str()) {
                return Err(format!("{name} is not a gate in this agreement."));
            }
        }
    }
    Ok(selection)
}

/// Owner acceptance of the exact gate revision is the execution grant: the
/// manifest digest is the accepted gate's content digest.
fn gate_execution_receipt(
    gate: &SelectedGate,
    outcome: &crate::gates::GateOutcome,
    manifest: &ContentDigest,
) -> crate::execution::ExecutionReceipt {
    use crate::execution::{CapturedOutput, ExecutionReceipt, ExecutionState, LimitEvidence};
    ExecutionReceipt {
        schema: crate::execution::RECEIPT_SCHEMA.into(),
        checker_id: gate.id.as_str().into(),
        checker_version: format!("v{}", gate.reference.revision),
        manifest_digest: manifest.as_str().into(),
        execution_grant_reference: Some(format!("accepted-gate:{}", manifest.as_str())),
        authority_revision: Some(gate.reference.revision),
        authority_checked_at: Some(utc_now()),
        state: match outcome.state {
            VerificationAxis::Pass => ExecutionState::Success,
            VerificationAxis::Fail => ExecutionState::Violated,
            VerificationAxis::Unknown => ExecutionState::Unknown,
        },
        reason_code: match outcome.state {
            VerificationAxis::Pass => "gate_passed",
            VerificationAxis::Fail => "gate_failed",
            VerificationAxis::Unknown => "gate_unknown",
        }
        .into(),
        summary: outcome.summary.clone(),
        process_started: outcome.exit_code.is_some() || outcome.program.is_some(),
        executable_digest: outcome.program_digest.clone(),
        consumed_configurations: Vec::new(),
        exit_code: outcome.exit_code,
        elapsed_ms: outcome.elapsed_ms,
        stdout: CapturedOutput::default(),
        stderr: CapturedOutput::default(),
        limits: LimitEvidence {
            timeout_ms: crate::gates::DEFAULT_GATE_TIMEOUT.as_millis() as u64,
            stdout_bytes: 2 * 1024 * 1024,
            stderr_bytes: 2 * 1024 * 1024,
            timeout_mechanism: "wall-clock bound with process-group termination".into(),
            output_mechanism: "bounded capture; overflow is unknown".into(),
            filesystem_network_memory_process_limits:
                "not sandboxed: repository-scoped working directory and allowlisted environment only"
                    .into(),
        },
        process_group_cleanup: "unix process group".into(),
        isolation_caveat: "Gate commands run as the local user without an OS sandbox.".into(),
    }
}

fn gate_finding(gate: &SelectedGate, failure: &crate::projection::ArtifactFailure) -> Finding {
    let (file, line) = match failure.location.rsplit_once(':') {
        Some((file, line))
            if line.chars().all(|character| character.is_ascii_digit())
                && !file.contains(' ')
                && !file.starts_with('/')
                && !file.contains("..") =>
        {
            (Some(file.to_string()), line.parse::<u32>().ok())
        }
        _ => (None, None),
    };
    Finding {
        file,
        line,
        column: None,
        rule_id: gate.id.as_str().into(),
        observed: failure.message.clone(),
        expected: gate.standard.statement.clone(),
        rationale: gate.standard.rationale.clone(),
        repair_direction: "Repair within the current task scope; never weaken the gate.".into(),
        permitted_next_action: format!("repair, then wh check --rule {}", gate.id.as_str()),
        verification_command: format!("wh check --rule {}", gate.id.as_str()),
    }
}

fn persist_gate_receipt(
    layout: &ProjectLayout,
    run_id: &str,
    gate: &SelectedGate,
    outcome: &crate::gates::GateOutcome,
    fingerprint: &str,
    checked_at: &str,
) -> Result<Option<crate::domain::RecordRef>, StorageError> {
    let store_path = layout.store_path(StoreKind::Private);
    if !store_path.join(".dolt").is_dir() {
        return Ok(None);
    }
    let repository = DoltRepository::initialize(&store_path, StoreKind::Private)?;
    let idempotency_key = format!("gate:{run_id}:{}", gate.id.as_str());
    if let Some(existing) = repository.by_idempotency_key(&idempotency_key)? {
        return existing.reference().map(Some).map_err(StorageError::Domain);
    }
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "whetstone:gate-runner".into(),
        display_name: None,
    };
    let key_digest = digest_bytes(idempotency_key.as_bytes());
    let suffix = &key_digest.as_str()["sha256:".len().."sha256:".len() + 24];
    let verification = outcome.state;
    let freshness = if verification == VerificationAxis::Pass && outcome.evidence.is_empty() {
        Freshness::Missing
    } else {
        Freshness::Fresh
    };
    let verification = if freshness == Freshness::Missing {
        VerificationAxis::Unknown
    } else {
        verification
    };
    let mut related = vec![gate.reference.clone()];
    if let Enforcement::Drive { feature } = &gate.standard.enforcement {
        if let Ok(Some(record)) = repository.latest(feature) {
            if let Ok(reference) = record.reference() {
                related.push(reference);
            }
        }
    }
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(format!("verification.gate_{suffix}")).map_err(StorageError::Domain)?,
        revision: 1,
        scope: Scope {
            organization: None,
            project: format!("project-{}", &layout.project_id()[..12]),
            component: None,
            environment: None,
        },
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::DeterministicCheck,
            recorded_by: principal,
            recorded_at: checked_at.into(),
            sources: vec![EvidenceRef {
                system: crate::projection::EVIDENCE_SYSTEM.into(),
                locator: run_id.into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes: None,
        idempotency_key,
        body: RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: format!(
                    "{}{}",
                    crate::projection::GATE_SUBJECT_PREFIX,
                    gate.id.as_str()
                ),
                revision: Some(fingerprint.into()),
            },
            code_digest: ContentDigest::new(if fingerprint.starts_with("sha256:") {
                fingerprint.to_string()
            } else {
                digest_bytes(fingerprint.as_bytes()).as_str().to_string()
            })
            .map_err(StorageError::Domain)?,
            policy_state: PolicyStateSnapshot {
                accepted: Some(gate.reference.clone()),
                required: None,
                installed: None,
                experimental: None,
            },
            verification,
            authorization: AuthorizationAxis::Unknown,
            freshness,
            checked_at: checked_at.into(),
            related_records: related,
            evidence: outcome.evidence.clone(),
        }),
    };
    repository.append(&record, None).map(Some)
}

fn digest_rule_inputs(project: &Path) -> Result<String, std::io::Error> {
    let mut files = WalkDir::new(project.join("whetstone"))
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    matches!(extension, "yaml" | "yml" | "toml" | "json" | "jsonc")
                })
        })
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    for candidate in [
        "Cargo.toml",
        "ruff.toml",
        ".ruff.toml",
        "pyproject.toml",
        "biome.json",
        "biome.jsonc",
        "rustfmt.toml",
        ".rustfmt.toml",
    ] {
        let path = project.join(candidate);
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(project).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(file)?);
        hasher.update([0]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
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
    fn unavailable_sync_never_claims_an_effect() {
        let response = CommandService.execute(ServiceRequest::Push(BasicRequest {
            project_dir: PathBuf::from("."),
            request_id: Some("push-1".into()),
        }));
        assert_eq!(response.state, ServiceState::Unavailable);
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
