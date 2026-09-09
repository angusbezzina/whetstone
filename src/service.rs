//! Shared command services used by CLI and future HTTP adapters.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::check;
use crate::domain::{
    AgreementRecord, AuthorizationAxis, ContentDigest, CoreValue, EvidenceRef, ExternalRef,
    ExternalSystem, Freshness, Guidance, ImplementationPhilosophy, Mission, PolicyStateSnapshot,
    PrincipalKind, PrincipalRef, Proposal, ProposalState, Provenance, ProvenanceAuthority,
    ProvenanceKind, RecordBody, RecordId, Scope, VerificationAxis, VerificationReceipt,
    SCHEMA_VERSION_V1,
};
use crate::history::{AccessBoundary, HistoryInspectionRequest, HistoryInspectionService};
use crate::onboarding;
use crate::storage::{DoltRepository, ProjectLayout, StorageError, StoreKind};
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
    Dash(BasicRequest),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitAction {
    Inspect,
    Agree,
}

#[derive(Debug, Clone)]
pub struct InitRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub action: InitAction,
    pub expected_revision: Option<u64>,
    pub resume_token: Option<String>,
    pub mission: Option<String>,
    pub values: Option<String>,
    pub philosophy: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Mission,
    Value,
    Philosophy,
    Guidance,
    Standard,
}

#[derive(Debug, Clone)]
pub struct ChangeRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub kind: Option<ChangeKind>,
    pub record_id: Option<String>,
    pub content: Option<String>,
    pub rationale: Option<String>,
    pub source: Option<String>,
    pub expected_effect: Option<String>,
    pub impact: Option<String>,
    pub examples: Vec<String>,
    pub conflicts: Vec<String>,
    pub expected_revision: Option<u64>,
    pub resume_token: Option<String>,
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

#[derive(Debug, Clone)]
pub struct CheckRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub paths: Vec<PathBuf>,
    pub language: Option<String>,
    pub rules: Vec<String>,
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
            "available_now": ["init", "change", "check"],
            "unavailable_until_milestone": {"dash": "M1.10", "pull": "M2.1", "push": "M2.1"},
            "read_only": true,
            "lean_baseline_revision": "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48",
        });
        response
    }

    fn dash(&self, request: BasicRequest) -> ServiceResponse {
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
        let principal = PrincipalRef {
            kind: PrincipalKind::LocalUser,
            stable_id: format!(
                "local:{}",
                &format!("{:x}", Sha256::digest(layout.project_id().as_bytes()))[..20]
            ),
            display_name: None,
        };
        let history = HistoryInspectionService::open(&layout).and_then(|service| {
            service.inspect(&HistoryInspectionRequest {
                project: format!("project-{}", &layout.project_id()[..12]),
                as_of: utc_now(),
                access: AccessBoundary::Private { principal },
                search: None,
                history_after: None,
                page_size: 100,
                expected_snapshot: None,
                redact_private_before: None,
            })
        });
        let mut response = ServiceResponse::new(
            request_id,
            "dash",
            ServiceState::Success,
            "Read-only project shape and decision history are ready.",
        );
        response.permitted_actions = vec!["wh change".into(), "wh check".into()];
        response.data = match history {
            Ok(history) => json!({"setup": setup, "history": history, "read_only": true}),
            Err(error) => json!({
                "setup": setup,
                "history": null,
                "history_state": "not_initialized",
                "history_detail": error.to_string(),
                "read_only": true
            }),
        };
        response
    }

    fn init(&self, request: InitRequest) -> ServiceResponse {
        let layout = match ProjectLayout::resolve(&request.project_dir, None) {
            Ok(layout) => layout,
            Err(error) => return project_error("init", request.request_id, error),
        };
        let request_id = match bounded_request_id(
            request.request_id,
            format!("init-{}", &layout.project_id()[..16]),
        ) {
            Ok(request_id) => request_id,
            Err(summary) => return unknown_response("init", "invalid-request-id".into(), summary),
        };
        let private = match DoltRepository::initialize(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        ) {
            Ok(repository) => repository,
            Err(error) => return storage_error("init", request_id, error),
        };
        if let Err(error) = DoltRepository::initialize(
            &layout.store_path(StoreKind::Shareable),
            StoreKind::Shareable,
        ) {
            return storage_error("init", request_id, error);
        }
        let mission_id = match RecordId::new("mission.project") {
            Ok(value) => value,
            Err(error) => return domain_response("init", request_id, error),
        };
        let current_revision = match private.latest(&mission_id) {
            Ok(record) => record.map_or(0, |record| record.revision),
            Err(error) => return storage_error("init", request_id, error),
        };
        let resume = resume_token(layout.project_id(), "init", &request_id, current_revision);
        if request.action == InitAction::Inspect {
            if current_revision > 0 {
                let mut response = ServiceResponse::new(
                    request_id,
                    "init",
                    ServiceState::Success,
                    "A local project agreement already exists; initialization made no changes.",
                );
                response.expected_revision = Some(current_revision);
                response.permitted_actions = vec!["wh change".into(), "wh check".into()];
                response.data = json!({"initialized": true, "shared": false});
                return response;
            }
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::NeedsInput,
                "The local stores are ready; the project agreement still needs owner input.",
            );
            response.expected_revision = Some(current_revision);
            response.resume_token = Some(resume);
            response.blocking_questions = vec![
                "What is the project mission?".into(),
                "Which core values govern trade-offs?".into(),
                "What implementation philosophy should engineers and agents follow?".into(),
            ];
            response.permitted_actions = vec![
                "wh init --action agree --expected-revision <revision> --resume <token> --mission <text> --values <text> --philosophy <text>".into(),
            ];
            response.data = json!({
                "project_id": layout.project_id(),
                "project_root": layout.project_root(),
                "private_store": "ready",
                "shareable_store": "local-only",
                "remote": "unavailable",
            });
            return response;
        }
        let existing_records = match init_idempotent_records(&private, &request_id) {
            Ok(records) => records,
            Err(error) => return storage_error("init", request_id, error),
        };
        let retrying_same_request = !existing_records.is_empty();
        if current_revision > 0 && !retrying_same_request {
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
            0
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
        let (mission, values, philosophy) = match (
            bounded_input(request.mission, "mission", 250),
            bounded_input(request.values, "values", 500),
            bounded_input(request.philosophy, "philosophy", 1000),
        ) {
            (Ok(mission), Ok(values), Ok(philosophy)) => (mission, values, philosophy),
            (mission, values, philosophy) => {
                let question = mission
                    .err()
                    .or_else(|| values.err())
                    .or_else(|| philosophy.err())
                    .unwrap_or_else(|| "Required agreement input is missing.".into());
                return needs_input("init", request_id, current_revision, resume, question);
            }
        };
        let records = [
            agreement_record(
                &layout,
                "mission.project",
                format!("{request_id}:mission"),
                RecordBody::Mission(Mission {
                    statement: mission,
                    desired_outcomes: Vec::new(),
                }),
            ),
            agreement_record(
                &layout,
                "value.core",
                format!("{request_id}:values"),
                RecordBody::CoreValue(CoreValue {
                    name: "Core values".into(),
                    description: values,
                }),
            ),
            agreement_record(
                &layout,
                "philosophy.implementation",
                format!("{request_id}:philosophy"),
                RecordBody::ImplementationPhilosophy(ImplementationPhilosophy {
                    statement: philosophy,
                    rationale: "Confirmed by the project owner during onboarding.".into(),
                    review_triggers: vec![
                        "mission change".into(),
                        "material architecture change".into(),
                        "repeated rule friction".into(),
                    ],
                }),
            ),
        ];
        let mut references = Vec::new();
        for record in records {
            let record = match record {
                Ok(record) => record,
                Err(error) => return domain_response("init", request_id, error),
            };
            if let Some(existing) = match private.by_idempotency_key(&record.idempotency_key) {
                Ok(existing) => existing,
                Err(error) => return storage_error("init", request_id, error),
            } {
                if existing.id != record.id || existing.body != record.body {
                    return idempotency_conflict("init", request_id, &record.idempotency_key);
                }
                match existing.reference() {
                    Ok(reference) => references.push(reference),
                    Err(error) => return domain_response("init", request_id, error),
                }
                continue;
            }
            match private.append(&record, None) {
                Ok(reference) => references.push(reference),
                Err(error) => return storage_error("init", request_id, error),
            }
        }
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            ServiceState::Success,
            "The first local project agreement was recorded; nothing was shared.",
        );
        response.expected_revision = Some(1);
        response.permitted_actions = vec!["wh check".into(), "wh change".into()];
        response.data = json!({"records": references, "shared": false});
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
        if request.kind.is_none()
            || request.record_id.is_none()
            || request.content.is_none()
            || request.rationale.is_none()
            || request.source.is_none()
            || request.expected_effect.is_none()
            || request.impact.is_none()
            || request.expected_revision.is_none()
        {
            return needs_input(
                "change",
                request_id,
                current_revision,
                resume,
                "Provide kind, record ID, content, rationale, source, expected effect, impact, expected revision, and the returned resume token.".into(),
            );
        }
        let narrative = match change_narrative(&request) {
            Ok(narrative) => narrative,
            Err(question) => {
                return needs_input("change", request_id, current_revision, resume, question)
            }
        };
        let body = match change_body(
            request.kind,
            request.content.as_deref(),
            request.rationale.as_deref(),
            &request.examples,
            id_text,
        ) {
            Ok(body) => body,
            Err(question) => {
                return needs_input("change", request_id, current_revision, resume, question)
            }
        };
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
            let mut record = match agreement_record(&layout, id_text, request_id.clone(), body) {
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
        let standard = request.kind == Some(ChangeKind::Standard);
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            if standard {
                ServiceState::NeedsDecision
            } else {
                ServiceState::Success
            },
            if standard {
                "The local standard draft was recorded, but checker design and independent approval are still required."
            } else if idempotent_replay {
                "The existing local draft was returned; no duplicate was created."
            } else {
                "The local agreement draft was recorded; team policy is unchanged."
            },
        );
        response.expected_revision = Some(record.revision);
        response.permitted_actions = if standard {
            response.blocking_questions = vec![
                "Which trusted checker enforces this standard, and who independently approves it?"
                    .into(),
            ];
            vec!["design the checker and submit the draft for independent approval".into()]
        } else {
            vec!["wh check".into(), "wh push".into()]
        };
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
        if request.paths.len() > 64 || request.rules.len() > 64 {
            return unknown_response(
                "check",
                "invalid-request".into(),
                "A check accepts at most 64 paths and 64 rule filters.".into(),
            );
        }
        if request
            .paths
            .iter()
            .any(|path| path.as_os_str().to_string_lossy().len() > 4096)
            || request
                .rules
                .iter()
                .any(|rule| rule.is_empty() || rule.len() > 256)
            || request.language.as_ref().is_some_and(|language| {
                language.is_empty()
                    || language.len() > 32
                    || !language
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-')
            })
        {
            return unknown_response(
                "check",
                "invalid-request".into(),
                "A check input exceeds its bound or contains an invalid language/rule filter."
                    .into(),
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
            request.request_id,
            format!(
                "check-{:x}",
                Sha256::digest(project.to_string_lossy().as_bytes())
            )[..22]
                .to_string(),
        ) {
            Ok(request_id) => request_id,
            Err(summary) => return unknown_response("check", "invalid-request-id".into(), summary),
        };
        let requested_paths = if request.paths.is_empty() {
            vec![PathBuf::from(".")]
        } else {
            request.paths
        };
        let mut scan_paths = Vec::new();
        for path in requested_paths {
            let joined = if path.is_absolute() {
                path
            } else {
                project.join(path)
            };
            let canonical = match joined.canonicalize() {
                Ok(path) => path,
                Err(error) => {
                    return unknown_response(
                        "check",
                        request_id,
                        format!("A requested check path is unavailable: {error}"),
                    )
                }
            };
            if !canonical.starts_with(&project) {
                return unknown_response(
                    "check",
                    request_id,
                    "A requested path escapes the project root.".into(),
                );
            }
            scan_paths.push(canonical);
        }
        let filter = (!request.rules.is_empty()).then_some(request.rules.as_slice());
        let result = check::run(check::CheckOptions {
            project_dir: &project,
            rules_dir: None,
            scan_paths: &scan_paths,
            lang_filter: request.language.as_deref(),
            rule_filter: filter,
            execute_command_validators: false,
        });
        let relative_paths = scan_paths
            .iter()
            .map(|path| {
                let relative = path.strip_prefix(&project).unwrap_or(path);
                if relative.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    relative.to_path_buf()
                }
            })
            .collect::<Vec<_>>();
        let code_tree = match verification::code_tree_digest(&project, &relative_paths) {
            Ok(digest) => digest,
            Err(error) => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The code snapshot could not be bound safely: {error}"),
                )
            }
        };
        let policy = match digest_rule_inputs(&project)
            .map_err(|error| error.to_string())
            .and_then(|digest| ContentDigest::new(digest).map_err(|error| error.to_string()))
        {
            Ok(digest) => digest,
            Err(error) => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The policy snapshot could not be bound safely: {error}"),
                )
            }
        };
        let checker_bundle = match checker_bundle_digest() {
            Ok(digest) => digest,
            Err(error) => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The compiled checker could not be content-bound: {error}"),
                )
            }
        };
        let scope = digest_json(&json!({
            "project": project.to_string_lossy(),
            "paths": relative_paths,
            "language": request.language,
            "rules": request.rules,
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
        let snapshot = SnapshotBinding {
            code_tree,
            policy,
            checker_bundle,
            scope,
            environment,
            trust,
        };
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
        let evaluated_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());
        let plan = VerificationPlan {
            subject: format!(
                "project:{:x}:check",
                Sha256::digest(project.to_string_lossy().as_bytes())
            ),
            snapshot: snapshot.clone(),
            requirements: vec![VerificationRequirement {
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
            }],
        };
        let evidence = TrustedEvidenceSet {
            items: vec![VerificationEvidence::NativeCheck {
                requirement_id: "whetstone.native-scan".into(),
                snapshot,
                state: native_state,
                evidence: EvidencePointer {
                    source: "whetstone-compiled-in-scanner".into(),
                    locator: "local-project".into(),
                    digest: scan_digest,
                },
                observed_at_unix: evaluated_at_unix,
                summary: native_summary,
                findings,
            }],
        };
        let report = match verification::aggregate(&plan, &evidence, evaluated_at_unix, &[]) {
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
        let receipt_record = match ProjectLayout::resolve(&project, None) {
            Ok(layout) => {
                match persist_verification_receipt(&layout, &request_id, &report, &checked_at) {
                    Ok(reference) => reference,
                    Err(error) => return storage_error("check", request_id, error),
                }
            }
            Err(_) => None,
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
        response.permitted_actions = match state {
            ServiceState::Success => vec!["handoff the verified result".into()],
            ServiceState::Violated => {
                vec!["repair within the current task scope, then wh check".into()]
            }
            _ => vec!["restore the required policy/checker, then wh check".into()],
        };
        response.data = json!({
            "report": report,
            "raw_scan": result,
            "receipt_persisted": receipt_record.is_some(),
            "receipt_record": receipt_record,
        });
        response
    }
}

fn init_idempotent_records(
    repository: &DoltRepository,
    request_id: &str,
) -> Result<Vec<AgreementRecord>, StorageError> {
    let mut records = Vec::new();
    for suffix in ["mission", "values", "philosophy"] {
        if let Some(record) = repository.by_idempotency_key(&format!("{request_id}:{suffix}"))? {
            records.push(record);
        }
    }
    Ok(records)
}

fn change_narrative(request: &ChangeRequest) -> Result<ChangeNarrative, String> {
    Ok(ChangeNarrative {
        rationale: bounded_input(request.rationale.clone(), "rationale", 4_000)?,
        source: bounded_input(request.source.clone(), "source", 2_048)?,
        expected_effect: bounded_input(request.expected_effect.clone(), "expected effect", 4_000)?,
        impact: bounded_input(request.impact.clone(), "impact", 4_000)?,
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
            title: format!("Change {}", candidate.id.as_str()),
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

fn change_body(
    kind: Option<ChangeKind>,
    content: Option<&str>,
    rationale: Option<&str>,
    examples: &[String],
    id_text: &str,
) -> Result<RecordBody, String> {
    let kind = kind.ok_or_else(|| "Provide kind.".to_string())?;
    let content = bounded_input(content.map(str::to_string), "content", 16_000)?;
    let rationale = rationale
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Owner-authored local change.")
        .trim()
        .to_string();
    Ok(match kind {
        ChangeKind::Mission => RecordBody::Mission(Mission {
            statement: content,
            desired_outcomes: Vec::new(),
        }),
        ChangeKind::Value => RecordBody::CoreValue(CoreValue {
            name: id_text.into(),
            description: content,
        }),
        ChangeKind::Philosophy => RecordBody::ImplementationPhilosophy(ImplementationPhilosophy {
            statement: content,
            rationale,
            review_triggers: Vec::new(),
        }),
        ChangeKind::Guidance => RecordBody::Guidance(Guidance {
            statement: content,
            rationale,
            examples: examples.to_vec(),
        }),
        ChangeKind::Standard => RecordBody::Guidance(Guidance {
            statement: content,
            rationale,
            examples: examples.to_vec(),
        }),
    })
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
) -> Result<AgreementRecord, crate::domain::DomainError> {
    let stable_id = format!(
        "local:{}",
        &format!("{:x}", Sha256::digest(layout.project_id().as_bytes()))[..20]
    );
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id,
        display_name: None,
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
