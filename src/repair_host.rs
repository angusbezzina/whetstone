//! Durable host adapter for the bounded repair/check loop.
//!
//! The adapter never edits files. It authenticates the current task authority,
//! invokes the same deterministic check service as a human CLI call, persists
//! the result, and tells the existing worker whether it may make another
//! in-scope attempt or must hand control back to an accountable owner.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    AgreementRecord, ContentDigest, EvidenceRef, ExternalRef, PrincipalRef, Provenance,
    ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, RecordRef, RepairAttemptRecord,
    RepairAuthorityReservationRecord, RepairCheckPhaseRecord, RepairHandoffRecord,
    RepairOperationClaimRecord, RepairSessionRecord, RepairSessionStateRecord,
    RepairSnapshotRecord, RepairWorkspaceFileRecord, Scope, SCHEMA_VERSION_V1,
};
use crate::service::{CheckRequest, CommandService, ServiceRequest, ServiceResponse, ServiceState};
use crate::storage::{AppendRequest, DoltRepository, ProjectLayout, StorageError, StoreKind};
use crate::verification::{CheckState, VerificationReport, LEAN_BASELINE_REVISION};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairAuthorityEvidence {
    pub locator: String,
}

/// Task context authenticated by the host and persisted with the private
/// repair session. The worker may read it, but cannot expand it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairTaskContext {
    pub objective: String,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub applicable_guidance: Vec<RecordRef>,
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    pub check_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_language: Option<String>,
    #[serde(default)]
    pub required_rules: Vec<String>,
    pub final_check_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_check_language: Option<String>,
    #[serde(default)]
    pub final_required_rules: Vec<String>,
    pub budget: RepairBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairBudget {
    pub max_attempts: u16,
    pub max_repeated_finding: u16,
    pub max_elapsed_seconds: u64,
    pub max_resource_units: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairAuthorityTarget {
    pub session_id: String,
    pub task: ExternalRef,
    pub project: String,
    pub authority_revision: u64,
    pub context: RepairTaskContext,
    pub now_unix: u64,
}

/// Produced only by the configured trusted host adapter, never deserialized
/// from browser, agent, or imported project data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRepairAuthority {
    pub evidence_id: String,
    pub principal: PrincipalRef,
    pub task: ExternalRef,
    pub project: String,
    pub authority_revision: u64,
    pub expires_at: String,
    pub expires_at_unix: u64,
    pub context: RepairTaskContext,
    pub may_edit_source: bool,
    pub may_edit_policy: bool,
    pub may_edit_checks: bool,
    pub may_reset_baselines: bool,
    pub may_publish: bool,
    pub may_merge: bool,
    pub may_release: bool,
}

pub trait RepairAuthorityVerifier: Send + Sync {
    fn verify(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: &RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError>;

    fn verify_completion(
        &self,
        evidence: &RepairCompletionEvidence,
        target: &RepairCompletionTarget,
    ) -> Result<VerifiedRepairCompletion, RepairHostError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCheckpointKind {
    PostEditHook,
    ExplicitCheckpoint,
}

#[derive(Debug, Clone)]
pub struct BeginRepairRequest {
    pub session_id: String,
    pub request_id: String,
    pub task: ExternalRef,
    pub authority_revision: u64,
    pub authority_evidence: RepairAuthorityEvidence,
    pub context: RepairTaskContext,
    pub now: String,
}

#[derive(Debug, Clone)]
pub struct CheckpointRequest {
    pub session_id: String,
    pub request_id: String,
    pub expected_revision: u64,
    pub authority_evidence: RepairAuthorityEvidence,
    pub now: String,
    pub kind: HostCheckpointKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairCompletionEvidence {
    pub locator: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepairCompletionTarget {
    pub session_id: String,
    pub task: ExternalRef,
    pub project: String,
    pub authority_revision: u64,
    pub candidate_workspace: ContentDigest,
    pub check_snapshot: RepairSnapshotRecord,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRepairCompletion {
    pub evidence_id: String,
    pub session_id: String,
    pub task: ExternalRef,
    pub project: String,
    pub authority_revision: u64,
    pub candidate_workspace: ContentDigest,
    pub check_snapshot: RepairSnapshotRecord,
    pub task_acceptance_satisfied: bool,
    pub required_reviews_satisfied: bool,
}

#[derive(Debug, Clone)]
pub struct FinalizeRepairRequest {
    pub session_id: String,
    pub request_id: String,
    pub expected_revision: u64,
    pub authority_evidence: RepairAuthorityEvidence,
    pub completion_evidence: RepairCompletionEvidence,
    pub now: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairFeedback {
    pub session: RecordRef,
    pub lean_baseline_revision: String,
    pub state: RepairSessionStateRecord,
    pub checkpoint: HostCheckpointKind,
    pub check: ServiceResponse,
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<RepairHandoffRecord>,
    pub edit_authorized: bool,
    pub policy_change_authorized: bool,
    pub publish_authorized: bool,
    pub merge_authorized: bool,
    pub release_authorized: bool,
    pub outcome: String,
}

#[derive(Debug)]
pub enum RepairHostError {
    Storage(StorageError),
    Domain(crate::domain::DomainError),
    MissingAuthority,
    AuthorityMismatch,
    AuthorityExpired,
    ForbiddenCapability,
    InvalidPath(String),
    ProtectedPath(String),
    Stale { expected: u64, actual: u64 },
    SessionMissing,
    WrongRecordKind,
    Terminal,
    BudgetExhausted,
    InvalidCheckResponse,
    Workspace(String),
    WorkspaceTooLarge,
    WorkspaceChanged,
    CompletionRejected,
    DuplicateAuthoritySession,
    OperationInProgress,
}

impl From<StorageError> for RepairHostError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

impl From<crate::domain::DomainError> for RepairHostError {
    fn from(value: crate::domain::DomainError) -> Self {
        Self::Domain(value)
    }
}

pub trait RepairClock: Send + Sync {
    fn now_unix(&self) -> Result<u64, RepairHostError>;
}

/// Deterministic check boundary used by the repair host.
///
/// Keeping this injectable lets concurrency tests prove that a durable claim
/// prevents duplicate checker execution, rather than merely hiding duplicate
/// work behind idempotent persistence.
pub trait RepairCheckExecutor: Send + Sync {
    fn execute_check(&self, request: CheckRequest) -> ServiceResponse;
}

impl RepairCheckExecutor for CommandService {
    fn execute_check(&self, request: CheckRequest) -> ServiceResponse {
        self.execute(ServiceRequest::Check(request))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRepairClock;

impl RepairClock for SystemRepairClock {
    fn now_unix(&self) -> Result<u64, RepairHostError> {
        unix_now()
    }
}

pub struct RepairHost<V, C = SystemRepairClock, E = CommandService> {
    verifier: V,
    commands: E,
    clock: C,
}

impl<V: RepairAuthorityVerifier> RepairHost<V, SystemRepairClock> {
    pub fn new(verifier: V) -> Self {
        Self {
            verifier,
            commands: CommandService,
            clock: SystemRepairClock,
        }
    }
}

impl<V: RepairAuthorityVerifier, C: RepairClock> RepairHost<V, C, CommandService> {
    pub fn with_clock(verifier: V, clock: C) -> Self {
        Self {
            verifier,
            commands: CommandService,
            clock,
        }
    }
}

impl<V: RepairAuthorityVerifier, C: RepairClock, E: RepairCheckExecutor> RepairHost<V, C, E> {
    pub fn with_clock_and_executor(verifier: V, clock: C, commands: E) -> Self {
        Self {
            verifier,
            commands,
            clock,
        }
    }

    pub fn begin(
        &self,
        project_dir: &Path,
        request: BeginRepairRequest,
    ) -> Result<RepairFeedback, RepairHostError> {
        validate_session_id(&request.session_id)?;
        validate_external_request_id(&request.request_id)?;
        let layout = ProjectLayout::resolve(project_dir, None)?;
        let project = project_scope(&layout);
        let requested_task = request.task.clone();
        let requested_authority_revision = request.authority_revision;
        let requested_context = request.context.clone();
        let request_id = request.request_id.clone();
        let started_at_unix = self.clock.now_unix()?;
        let authority = self.authorize(
            &request.authority_evidence,
            RepairAuthorityTarget {
                session_id: request.session_id.clone(),
                task: request.task.clone(),
                project: project.clone(),
                authority_revision: request.authority_revision,
                context: request.context.clone(),
                now_unix: started_at_unix,
            },
        )?;
        if self.clock.now_unix()? >= authority.expires_at_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        let context = normalized_context(authority.context.clone())?;
        for path in context.check_paths.iter().chain(&context.final_check_paths) {
            validate_project_path(layout.project_root(), path)?;
        }
        let repository = DoltRepository::open_existing(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        )?;
        let id = session_record_id(&request.session_id)?;
        if let Some(replayed) = repository.by_idempotency_key(&request.request_id)? {
            if replayed.id != id || replayed.revision != 1 {
                return Err(RepairHostError::AuthorityMismatch);
            }
            let RecordBody::RepairSession(session) = &replayed.body else {
                return Err(RepairHostError::WrongRecordKind);
            };
            if session.task != request.task
                || session.authority_revision != request.authority_revision
                || session.authority_principal != authority.principal
                || session.authority_expires_at != authority.expires_at
                || session.authority_expires_at_unix != authority.expires_at_unix
                || !same_context(&session_context(session), &context)?
            {
                return Err(RepairHostError::AuthorityMismatch);
            }
            return self.replay_feedback(
                project_dir,
                &repository,
                replayed,
                HostCheckpointKind::ExplicitCheckpoint,
            );
        }
        if repository.latest(&id)?.is_some() {
            return Err(RepairHostError::Stale {
                expected: 0,
                actual: repository.latest(&id)?.map_or(0, |record| record.revision),
            });
        }
        let (reservation_id, reservation_key) =
            repair_reservation_identity(&project, &requested_task, requested_authority_revision)?;
        if let Some(existing) = repository.latest(&reservation_id)? {
            let RecordBody::RepairAuthorityReservation(reservation) = existing.body else {
                return Err(RepairHostError::AuthorityMismatch);
            };
            if reservation.project != project
                || reservation.task != requested_task
                || reservation.authority_revision != requested_authority_revision
            {
                return Err(RepairHostError::AuthorityMismatch);
            }
            return if reservation.begin_request_id == request_id
                && reservation.requested_session_id == request.session_id
                && reservation.session.is_none()
            {
                Err(RepairHostError::OperationInProgress)
            } else {
                Err(RepairHostError::DuplicateAuthoritySession)
            };
        }
        if repository.all_records()?.iter().any(|record| {
            matches!(
                &record.body,
                RecordBody::RepairSession(session)
                    if session.task == request.task
                        && session.authority_revision == request.authority_revision
            )
        }) {
            return Err(RepairHostError::DuplicateAuthoritySession);
        }
        let reservation_claim = operational_record(
            &layout,
            OperationalRecordWrite {
                id: reservation_id.clone(),
                revision: 1,
                supersedes: None,
                idempotency_key: reservation_key.clone(),
                owner: authority.principal.clone(),
                evidence_locator: "repair-authority-reservation".into(),
                recorded_at: request.now.clone(),
                body: RecordBody::RepairAuthorityReservation(Box::new(
                    RepairAuthorityReservationRecord {
                        project: project.clone(),
                        task: requested_task.clone(),
                        authority_revision: requested_authority_revision,
                        requested_session_id: request.session_id.clone(),
                        begin_request_id: request_id.clone(),
                        started_at_unix,
                        bootstrap_resource_units: 2,
                        session: None,
                    },
                )),
            },
        )?;
        if let Err(error) = repository.append_exclusive(&reservation_claim, None) {
            if let Some(existing) = repository.latest(&reservation_id)? {
                let RecordBody::RepairAuthorityReservation(reservation) = existing.body else {
                    return Err(RepairHostError::AuthorityMismatch);
                };
                if reservation.project == project
                    && reservation.task == requested_task
                    && reservation.authority_revision == requested_authority_revision
                {
                    return if reservation.begin_request_id == request_id
                        && reservation.requested_session_id == request.session_id
                    {
                        Err(RepairHostError::OperationInProgress)
                    } else {
                        Err(RepairHostError::DuplicateAuthoritySession)
                    };
                }
                return Err(RepairHostError::AuthorityMismatch);
            }
            return Err(RepairHostError::Storage(error));
        }
        let workspace_files = capture_workspace(layout.project_root(), &context)?;
        let check = self.run_check(
            project_dir,
            &request.request_id,
            &context.check_paths,
            context.check_language.clone(),
            context.required_rules.clone(),
        );
        let (snapshot, mut findings, receipt) = check_parts(&check)?;
        let final_baseline_check = self.run_check(
            project_dir,
            &format!("{}:final-baseline", request.request_id),
            &context.final_check_paths,
            context.final_check_language.clone(),
            context.final_required_rules.clone(),
        );
        let (final_baseline_snapshot, _, _) = check_parts(&final_baseline_check)?;
        if capture_workspace(layout.project_root(), &context)? != workspace_files {
            return Err(RepairHostError::WorkspaceChanged);
        }
        let completed_at_unix = self.clock.now_unix()?;
        let refreshed_authority = self.authorize(
            &request.authority_evidence,
            RepairAuthorityTarget {
                session_id: request.session_id.clone(),
                task: requested_task.clone(),
                project: project.clone(),
                authority_revision: requested_authority_revision,
                context: context.clone(),
                now_unix: completed_at_unix,
            },
        );
        let authority_rechecked_at_unix = self.clock.now_unix()?;
        let authority_still_valid = refreshed_authority.as_ref().is_ok_and(|refreshed| {
            refreshed.principal == authority.principal
                && refreshed.expires_at == authority.expires_at
                && refreshed.expires_at_unix == authority.expires_at_unix
                && authority_rechecked_at_unix < refreshed.expires_at_unix
        });
        let bootstrap_within_budget = authority_rechecked_at_unix.saturating_sub(started_at_unix)
            < context.budget.max_elapsed_seconds;
        if !authority_still_valid {
            findings.push("authority:expired-or-revoked-during-bootstrap".into());
        } else if !bootstrap_within_budget {
            findings.push("budget:exhausted-during-bootstrap".into());
        }
        let state = if authority_still_valid && bootstrap_within_budget {
            state_from_check(check.state)
        } else {
            RepairSessionStateRecord::NeedsDecision
        };
        let attempts_reserved = if state == RepairSessionStateRecord::Ready {
            1
        } else {
            0
        };
        let budget = context.budget;
        let body = RepairSessionRecord {
            session_id: request.session_id.clone(),
            lean_baseline_revision: LEAN_BASELINE_REVISION.into(),
            authority_principal: authority.principal.clone(),
            task: request.task,
            objective: context.objective,
            non_goals: context.non_goals,
            applicable_guidance: context.applicable_guidance,
            authority_revision: request.authority_revision,
            authority_expires_at: authority.expires_at,
            authority_expires_at_unix: authority.expires_at_unix,
            allowed_paths: context.allowed_paths,
            excluded_paths: context.excluded_paths,
            check_paths: context.check_paths,
            check_language: context.check_language,
            check_rules: context.required_rules,
            final_check_paths: context.final_check_paths,
            final_check_language: context.final_check_language,
            final_check_rules: context.final_required_rules,
            final_baseline_snapshot,
            reviewed_snapshot: snapshot,
            reviewed_phase: RepairCheckPhaseRecord::Fast,
            workspace_files,
            started_at_unix,
            last_checkpoint_at_unix: started_at_unix,
            max_attempts: budget.max_attempts,
            max_repeated_finding: budget.max_repeated_finding,
            max_elapsed_seconds: budget.max_elapsed_seconds,
            max_resource_units: budget.max_resource_units,
            attempts: Vec::new(),
            attempts_reserved,
            total_elapsed_seconds: 0,
            total_resource_units: 0,
            final_verification_elapsed_seconds: 0,
            final_verification_resource_units: 0,
            current_finding_ids: findings.clone(),
            state,
            updated_at: request.now.clone(),
            last_check_response: serde_json::to_value(&check)
                .map_err(|_| RepairHostError::InvalidCheckResponse)?,
            last_checkpoint_kind: checkpoint_kind_name(HostCheckpointKind::ExplicitCheckpoint)
                .into(),
            last_check_receipt: receipt,
        };
        let record = operational_record(
            &layout,
            OperationalRecordWrite {
                id,
                revision: 1,
                supersedes: None,
                idempotency_key: request.request_id,
                owner: authority.principal,
                evidence_locator: authority.evidence_id,
                recorded_at: request.now.clone(),
                body: RecordBody::RepairSession(Box::new(body)),
            },
        )?;
        let session_ref = record.reference()?;
        let reservation = operational_record(
            &layout,
            OperationalRecordWrite {
                id: reservation_id.clone(),
                revision: 2,
                supersedes: Some(reservation_claim.reference()?),
                idempotency_key: format!("{reservation_key}:complete"),
                owner: record.owner.clone(),
                evidence_locator: "repair-authority-reservation".into(),
                recorded_at: record.provenance.recorded_at.clone(),
                body: RecordBody::RepairAuthorityReservation(Box::new(
                    RepairAuthorityReservationRecord {
                        project: project.clone(),
                        task: requested_task.clone(),
                        authority_revision: requested_authority_revision,
                        requested_session_id: request.session_id.clone(),
                        begin_request_id: request_id.clone(),
                        started_at_unix,
                        bootstrap_resource_units: 2,
                        session: Some(session_ref),
                    },
                )),
            },
        )?;
        let handoff = (state == RepairSessionStateRecord::NeedsDecision)
            .then(|| {
                self.handoff_record(
                    &layout,
                    &record,
                    "Required verification is unavailable or unresolved",
                )
            })
            .transpose()?;
        let mut appends = vec![
            AppendRequest {
                record: &record,
                expected_revision: None,
            },
            AppendRequest {
                record: &reservation,
                expected_revision: Some(1),
            },
        ];
        if let Some(handoff) = &handoff {
            appends.push(AppendRequest {
                record: handoff,
                expected_revision: None,
            });
        }
        let reference = match repository.append_batch(&appends) {
            Ok(references) => references
                .first()
                .cloned()
                .ok_or(RepairHostError::InvalidCheckResponse)?,
            Err(error) => {
                // A concurrent identical request may have committed first.
                // Recover it by the caller key without relying on database
                // error strings or rerunning its verification work.
                if let Some(replayed) = repository.by_idempotency_key(&request_id)? {
                    let RecordBody::RepairSession(session) = &replayed.body else {
                        return Err(RepairHostError::AuthorityMismatch);
                    };
                    if replayed.id == record.id
                        && replayed.revision == 1
                        && session.task == requested_task
                        && session.authority_revision == requested_authority_revision
                        && session.authority_principal == record.owner
                        && same_context(&session_context(session), &requested_context)?
                    {
                        return self.replay_feedback(
                            project_dir,
                            &repository,
                            replayed,
                            HostCheckpointKind::ExplicitCheckpoint,
                        );
                    }
                    return Err(RepairHostError::AuthorityMismatch);
                }
                if let Some(existing) = repository.latest(&reservation_id)? {
                    let RecordBody::RepairAuthorityReservation(body) = existing.body else {
                        return Err(RepairHostError::AuthorityMismatch);
                    };
                    if body.project != project
                        || body.task != requested_task
                        || body.authority_revision != requested_authority_revision
                    {
                        return Err(RepairHostError::AuthorityMismatch);
                    }
                    return if body.begin_request_id == request_id && body.session.is_none() {
                        Err(RepairHostError::OperationInProgress)
                    } else {
                        Err(RepairHostError::DuplicateAuthoritySession)
                    };
                }
                return Err(RepairHostError::Storage(error));
            }
        };
        feedback(
            &repository,
            reference,
            state,
            HostCheckpointKind::ExplicitCheckpoint,
            check,
            findings,
        )
    }

    pub fn checkpoint(
        &self,
        project_dir: &Path,
        request: CheckpointRequest,
    ) -> Result<RepairFeedback, RepairHostError> {
        validate_external_request_id(&request.request_id)?;
        let layout = ProjectLayout::resolve(project_dir, None)?;
        let repository = DoltRepository::open_existing(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        )?;
        let id = session_record_id(&request.session_id)?;
        let current = repository
            .latest(&id)?
            .ok_or(RepairHostError::SessionMissing)?;
        if let Some(replayed) = repository.by_idempotency_key(&request.request_id)? {
            if replayed.id != id || replayed.revision != request.expected_revision.saturating_add(1)
            {
                return Err(RepairHostError::AuthorityMismatch);
            }
            let RecordBody::RepairSession(session) = &replayed.body else {
                return Err(RepairHostError::WrongRecordKind);
            };
            self.authorize_session(&layout, session, &request.authority_evidence)?;
            return self.replay_feedback(project_dir, &repository, replayed, request.kind);
        }
        if current.revision != request.expected_revision {
            return Err(RepairHostError::Stale {
                expected: request.expected_revision,
                actual: current.revision,
            });
        }
        let RecordBody::RepairSession(mut session) = current.body.clone() else {
            return Err(RepairHostError::WrongRecordKind);
        };
        if matches!(
            session.state,
            RepairSessionStateRecord::ReadyForFinalVerification
                | RepairSessionStateRecord::Verified
                | RepairSessionStateRecord::NeedsDecision
                | RepairSessionStateRecord::Cancelled
        ) {
            return Err(RepairHostError::Terminal);
        }
        let authority = self.authorize(
            &request.authority_evidence,
            RepairAuthorityTarget {
                session_id: request.session_id.clone(),
                task: session.task.clone(),
                project: project_scope(&layout),
                authority_revision: session.authority_revision,
                context: session_context(&session),
                now_unix: self.clock.now_unix()?,
            },
        )?;
        if authority.principal != session.authority_principal
            || authority.expires_at != session.authority_expires_at
            || authority.expires_at_unix != session.authority_expires_at_unix
        {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if self.clock.now_unix()? >= authority.expires_at_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        let attempt_number = session.attempts.len() as u16 + 1;
        if session.attempts_reserved != attempt_number {
            return Err(RepairHostError::BudgetExhausted);
        }
        let before_check_unix = self.clock.now_unix()?;
        if session.attempts.len() >= usize::from(session.max_attempts)
            || before_check_unix.saturating_sub(session.started_at_unix)
                >= session.max_elapsed_seconds
            || session.total_resource_units >= session.max_resource_units
        {
            return self.stop_for_budget(
                &layout,
                &repository,
                current,
                session,
                request.request_id,
                request.now,
                authority,
                request.kind,
            );
        }
        let context = session_context(&session);
        let workspace_files = capture_workspace(layout.project_root(), &context)?;
        let changed_paths = workspace_changes(&session.workspace_files, &workspace_files);
        for path in &changed_paths {
            validate_project_path(layout.project_root(), path)?;
            if protected_repair_path(path)
                || workspace_path_protected(&session.workspace_files, path)
                || workspace_path_protected(&workspace_files, path)
            {
                return Err(RepairHostError::ProtectedPath(path.clone()));
            }
            if session
                .excluded_paths
                .iter()
                .any(|excluded| path_is_within(path, excluded))
            {
                return Err(RepairHostError::InvalidPath(path.clone()));
            }
            if !session
                .allowed_paths
                .iter()
                .any(|allowed| path_is_within(path, allowed))
            {
                return Err(RepairHostError::InvalidPath(path.clone()));
            }
        }
        self.claim_operation(
            &layout,
            &repository,
            &current,
            &request.request_id,
            RepairCheckPhaseRecord::Fast,
            request.kind,
            before_check_unix,
            &request.now,
        )?;
        let check = self.run_check(
            project_dir,
            &request.request_id,
            &session.check_paths,
            session.check_language.clone(),
            session.check_rules.clone(),
        );
        let (snapshot, mut findings, receipt) = check_parts(&check)?;
        if capture_workspace(layout.project_root(), &context)? != workspace_files {
            return Err(RepairHostError::WorkspaceChanged);
        }
        if snapshot.policy != session.reviewed_snapshot.policy
            || snapshot.checker_bundle != session.reviewed_snapshot.checker_bundle
            || snapshot.scope != session.reviewed_snapshot.scope
            || snapshot.environment != session.reviewed_snapshot.environment
            || snapshot.trust != session.reviewed_snapshot.trust
        {
            return Err(RepairHostError::ProtectedPath(
                "required verification inputs changed".into(),
            ));
        }
        let refreshed_authority = self.authorize(
            &request.authority_evidence,
            RepairAuthorityTarget {
                session_id: request.session_id.clone(),
                task: session.task.clone(),
                project: project_scope(&layout),
                authority_revision: session.authority_revision,
                context: context.clone(),
                now_unix: self.clock.now_unix()?,
            },
        );
        let authority_rechecked_at_unix = self.clock.now_unix()?;
        let authority_still_valid = refreshed_authority.as_ref().is_ok_and(|refreshed| {
            refreshed.principal == session.authority_principal
                && refreshed.expires_at == session.authority_expires_at
                && refreshed.expires_at_unix == session.authority_expires_at_unix
                && authority_rechecked_at_unix < refreshed.expires_at_unix
        });
        if !authority_still_valid {
            findings.push("authority:expired-or-revoked-during-check".into());
            findings.sort();
        }
        let stop_reason = if !authority_still_valid {
            Some("Repair authority expired or was revoked during verification")
        } else if session.attempts.last().is_some_and(|previous| {
            previous.candidate == snapshot && previous.finding_ids == findings
        }) {
            Some("Repeated check made no progress")
        } else if session.attempts.len() >= 2 {
            let prior = &session.attempts[session.attempts.len() - 2];
            if prior.candidate == snapshot && prior.finding_ids == findings {
                Some("Repair attempts oscillated")
            } else {
                None
            }
        } else {
            None
        };
        let observed_at_unix = self.clock.now_unix()?;
        let elapsed_seconds = observed_at_unix
            .saturating_sub(session.last_checkpoint_at_unix)
            .max(1);
        let resource_units = 1;
        session.total_elapsed_seconds = session
            .total_elapsed_seconds
            .saturating_add(elapsed_seconds);
        session.total_resource_units = session.total_resource_units.saturating_add(resource_units);
        session.attempts.push(RepairAttemptRecord {
            number: attempt_number,
            candidate: snapshot.clone(),
            finding_ids: findings.clone(),
            changed_paths,
            elapsed_seconds,
            resource_units,
            observed_at_unix,
        });
        session.current_finding_ids = findings.clone();
        session.reviewed_snapshot = snapshot;
        session.reviewed_phase = RepairCheckPhaseRecord::Fast;
        session.last_check_response =
            serde_json::to_value(&check).map_err(|_| RepairHostError::InvalidCheckResponse)?;
        session.last_checkpoint_kind = checkpoint_kind_name(request.kind).into();
        session.last_check_receipt = receipt;
        session.workspace_files = workspace_files;
        session.last_checkpoint_at_unix = observed_at_unix;
        session.updated_at = request.now.clone();
        let repeats = repeated_findings(&session.attempts);
        let repair_budget_exhausted = attempt_number >= session.max_attempts
            || session.total_elapsed_seconds >= session.max_elapsed_seconds
            || session.total_resource_units >= session.max_resource_units
            || repeats
                .values()
                .any(|count| *count >= session.max_repeated_finding);
        // A green result on the final permitted repair attempt is still a
        // valid candidate for broader verification. The attempt cap limits
        // further edit/check cycles; it must not turn a successful last repair
        // into an owner interruption. Time/resource exhaustion still stops so
        // the separately authenticated final gate cannot exceed its budget.
        let final_verification_budget_exhausted = session.total_elapsed_seconds
            >= session.max_elapsed_seconds
            || session.total_resource_units >= session.max_resource_units;
        session.state = if check.state == ServiceState::Success
            && !final_verification_budget_exhausted
            && stop_reason.is_none()
        {
            RepairSessionStateRecord::ReadyForFinalVerification
        } else if check.state == ServiceState::Violated
            && !repair_budget_exhausted
            && stop_reason.is_none()
        {
            RepairSessionStateRecord::Ready
        } else {
            RepairSessionStateRecord::NeedsDecision
        };
        session.attempts_reserved = if session.state == RepairSessionStateRecord::Ready {
            attempt_number.saturating_add(1)
        } else {
            attempt_number
        };
        let state = session.state;
        let previous = current.reference()?;
        let record = operational_record(
            &layout,
            OperationalRecordWrite {
                id,
                revision: current.revision + 1,
                supersedes: Some(previous),
                idempotency_key: request.request_id,
                owner: authority.principal,
                evidence_locator: authority.evidence_id,
                recorded_at: request.now.clone(),
                body: RecordBody::RepairSession(session),
            },
        )?;
        let handoff_reason = (state == RepairSessionStateRecord::NeedsDecision).then(|| {
            stop_reason.unwrap_or(
                if repair_budget_exhausted || final_verification_budget_exhausted {
                    "Repair budget exhausted"
                } else {
                    "Required verification is unavailable or unresolved"
                },
            )
        });
        let reference = self.append_transition(
            &layout,
            &repository,
            &record,
            Some(current.revision),
            handoff_reason,
        )?;
        feedback(&repository, reference, state, request.kind, check, findings)
    }

    /// Run the broader task acceptance gate. A green fast repair check is not
    /// completion: this transition also requires the host to attest the task's
    /// acceptance criteria and required reviews against the exact workspace.
    pub fn finalize(
        &self,
        project_dir: &Path,
        request: FinalizeRepairRequest,
    ) -> Result<RepairFeedback, RepairHostError> {
        validate_external_request_id(&request.request_id)?;
        let layout = ProjectLayout::resolve(project_dir, None)?;
        let repository = DoltRepository::open_existing(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        )?;
        let id = session_record_id(&request.session_id)?;
        let current = repository
            .latest(&id)?
            .ok_or(RepairHostError::SessionMissing)?;
        if let Some(replayed) = repository.by_idempotency_key(&request.request_id)? {
            if replayed.id != id || replayed.revision != request.expected_revision.saturating_add(1)
            {
                return Err(RepairHostError::AuthorityMismatch);
            }
            let RecordBody::RepairSession(session) = &replayed.body else {
                return Err(RepairHostError::WrongRecordKind);
            };
            self.authorize_session(&layout, session, &request.authority_evidence)?;
            return self.replay_feedback(
                project_dir,
                &repository,
                replayed,
                HostCheckpointKind::ExplicitCheckpoint,
            );
        }
        if current.revision != request.expected_revision {
            return Err(RepairHostError::Stale {
                expected: request.expected_revision,
                actual: current.revision,
            });
        }
        let RecordBody::RepairSession(mut session) = current.body.clone() else {
            return Err(RepairHostError::WrongRecordKind);
        };
        if session.state != RepairSessionStateRecord::ReadyForFinalVerification {
            return Err(RepairHostError::Terminal);
        }
        let authority = self.authorize(
            &request.authority_evidence,
            RepairAuthorityTarget {
                session_id: request.session_id.clone(),
                task: session.task.clone(),
                project: project_scope(&layout),
                authority_revision: session.authority_revision,
                context: session_context(&session),
                now_unix: self.clock.now_unix()?,
            },
        )?;
        if authority.principal != session.authority_principal
            || authority.expires_at != session.authority_expires_at
            || authority.expires_at_unix != session.authority_expires_at_unix
        {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if self.clock.now_unix()? >= authority.expires_at_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        let before_check_unix = self.clock.now_unix()?;
        if before_check_unix.saturating_sub(session.started_at_unix) >= session.max_elapsed_seconds
            || session.total_resource_units >= session.max_resource_units
        {
            return self.stop_for_budget(
                &layout,
                &repository,
                current,
                session,
                request.request_id,
                request.now,
                authority,
                HostCheckpointKind::ExplicitCheckpoint,
            );
        }
        let context = session_context(&session);
        let workspace_files = capture_workspace(layout.project_root(), &context)?;
        if workspace_files != session.workspace_files {
            return Err(RepairHostError::WorkspaceChanged);
        }
        self.claim_operation(
            &layout,
            &repository,
            &current,
            &request.request_id,
            RepairCheckPhaseRecord::Final,
            HostCheckpointKind::ExplicitCheckpoint,
            before_check_unix,
            &request.now,
        )?;
        let check = self.run_check(
            project_dir,
            &request.request_id,
            &session.final_check_paths,
            session.final_check_language.clone(),
            session.final_check_rules.clone(),
        );
        let (snapshot, mut findings, receipt) = check_parts(&check)?;
        if capture_workspace(layout.project_root(), &context)? != workspace_files {
            return Err(RepairHostError::WorkspaceChanged);
        }
        if snapshot.policy != session.final_baseline_snapshot.policy
            || snapshot.checker_bundle != session.final_baseline_snapshot.checker_bundle
            || snapshot.scope != session.final_baseline_snapshot.scope
            || snapshot.environment != session.final_baseline_snapshot.environment
            || snapshot.trust != session.final_baseline_snapshot.trust
        {
            return Err(RepairHostError::ProtectedPath(
                "required final-verification inputs changed".into(),
            ));
        }
        let candidate_workspace = workspace_digest(&workspace_files)?;
        let completion = if check.state == ServiceState::Success && findings.is_empty() {
            self.verifier
                .verify_completion(
                    &request.completion_evidence,
                    &RepairCompletionTarget {
                        session_id: request.session_id.clone(),
                        task: session.task.clone(),
                        project: project_scope(&layout),
                        authority_revision: session.authority_revision,
                        candidate_workspace: candidate_workspace.clone(),
                        check_snapshot: snapshot.clone(),
                    },
                )
                .ok()
        } else {
            None
        };
        // Completion attestation is an external host operation. Recheck the
        // exact candidate and standing authority after it returns, and count
        // the entire interval before deciding whether Verified is possible.
        let reauthorized = self.authorize(
            &request.authority_evidence,
            RepairAuthorityTarget {
                session_id: request.session_id.clone(),
                task: session.task.clone(),
                project: project_scope(&layout),
                authority_revision: session.authority_revision,
                context: context.clone(),
                now_unix: self.clock.now_unix()?,
            },
        );
        let completed_at_unix = self.clock.now_unix()?;
        let final_workspace_unchanged =
            capture_workspace(layout.project_root(), &context)? == workspace_files;
        let authority_still_valid = reauthorized.as_ref().is_ok_and(|current_authority| {
            current_authority.principal == session.authority_principal
                && current_authority.expires_at == session.authority_expires_at
                && current_authority.expires_at_unix == session.authority_expires_at_unix
                && completed_at_unix < current_authority.expires_at_unix
        });
        let final_elapsed = completed_at_unix
            .saturating_sub(session.last_checkpoint_at_unix)
            .max(1);
        session.final_verification_elapsed_seconds = session
            .final_verification_elapsed_seconds
            .saturating_add(final_elapsed);
        session.final_verification_resource_units =
            session.final_verification_resource_units.saturating_add(1);
        session.total_elapsed_seconds = session.total_elapsed_seconds.saturating_add(final_elapsed);
        session.total_resource_units = session.total_resource_units.saturating_add(1);
        let within_budget = session.total_elapsed_seconds <= session.max_elapsed_seconds
            && session.total_resource_units <= session.max_resource_units;
        let accepted = completion.as_ref().is_some_and(|verified| {
            within_budget
                && authority_still_valid
                && final_workspace_unchanged
                && !verified.evidence_id.trim().is_empty()
                && verified.session_id == request.session_id
                && verified.task == session.task
                && verified.project == project_scope(&layout)
                && verified.authority_revision == session.authority_revision
                && verified.candidate_workspace == candidate_workspace
                && verified.check_snapshot == snapshot
                && verified.task_acceptance_satisfied
                && verified.required_reviews_satisfied
        });
        if !accepted && findings.is_empty() {
            findings.push(if !final_workspace_unchanged {
                "workspace:changed-during-completion".into()
            } else if !authority_still_valid {
                "authority:expired-or-revoked-during-completion".into()
            } else if !within_budget {
                "budget:exhausted-during-final-verification".into()
            } else {
                "completion:task-acceptance-or-review".into()
            });
        }
        session.state = if accepted {
            RepairSessionStateRecord::Verified
        } else {
            RepairSessionStateRecord::NeedsDecision
        };
        session.current_finding_ids = findings.clone();
        session.reviewed_snapshot = snapshot;
        session.reviewed_phase = RepairCheckPhaseRecord::Final;
        session.last_check_response =
            serde_json::to_value(&check).map_err(|_| RepairHostError::InvalidCheckResponse)?;
        session.last_checkpoint_kind =
            checkpoint_kind_name(HostCheckpointKind::ExplicitCheckpoint).into();
        session.last_check_receipt = receipt;
        session.workspace_files = workspace_files;
        session.last_checkpoint_at_unix = completed_at_unix;
        session.updated_at = request.now.clone();
        session.attempts_reserved = session.attempts.len() as u16;
        let state = session.state;
        let previous = current.reference()?;
        let record = operational_record(
            &layout,
            OperationalRecordWrite {
                id,
                revision: current.revision + 1,
                supersedes: Some(previous),
                idempotency_key: request.request_id,
                owner: authority.principal,
                evidence_locator: if accepted {
                    completion
                        .map(|value| value.evidence_id)
                        .unwrap_or_else(|| "completion-rejected".into())
                } else {
                    "completion-rejected".into()
                },
                recorded_at: request.now,
                body: RecordBody::RepairSession(session),
            },
        )?;
        let reference = self.append_transition(
            &layout,
            &repository,
            &record,
            Some(current.revision),
            (state == RepairSessionStateRecord::NeedsDecision)
                .then_some("Broader task acceptance or required review was not satisfied"),
        )?;
        feedback(
            &repository,
            reference,
            state,
            HostCheckpointKind::ExplicitCheckpoint,
            check,
            findings,
        )
    }

    pub fn cancel(
        &self,
        project_dir: &Path,
        session_id: &str,
        expected_revision: u64,
        request_id: String,
        now: String,
        evidence: &RepairAuthorityEvidence,
    ) -> Result<RecordRef, RepairHostError> {
        validate_external_request_id(&request_id)?;
        let layout = ProjectLayout::resolve(project_dir, None)?;
        let repository = DoltRepository::open_existing(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        )?;
        let id = session_record_id(session_id)?;
        let current = repository
            .latest(&id)?
            .ok_or(RepairHostError::SessionMissing)?;
        if let Some(replayed) = repository.by_idempotency_key(&request_id)? {
            if replayed.id != id || replayed.revision != expected_revision.saturating_add(1) {
                return Err(RepairHostError::AuthorityMismatch);
            }
            let RecordBody::RepairSession(session) = &replayed.body else {
                return Err(RepairHostError::WrongRecordKind);
            };
            self.authorize_session(&layout, session, evidence)?;
            if session.state != RepairSessionStateRecord::Cancelled {
                return Err(RepairHostError::AuthorityMismatch);
            }
            return Ok(replayed.reference()?);
        }
        if current.revision != expected_revision {
            return Err(RepairHostError::Stale {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let RecordBody::RepairSession(mut session) = current.body.clone() else {
            return Err(RepairHostError::WrongRecordKind);
        };
        if matches!(
            session.state,
            RepairSessionStateRecord::Verified
                | RepairSessionStateRecord::NeedsDecision
                | RepairSessionStateRecord::Cancelled
        ) {
            return Err(RepairHostError::Terminal);
        }
        let authority = self.authorize(
            evidence,
            RepairAuthorityTarget {
                session_id: session_id.into(),
                task: session.task.clone(),
                project: project_scope(&layout),
                authority_revision: session.authority_revision,
                context: session_context(&session),
                now_unix: self.clock.now_unix()?,
            },
        )?;
        if authority.principal != session.authority_principal
            || authority.expires_at != session.authority_expires_at
            || authority.expires_at_unix != session.authority_expires_at_unix
        {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if self.clock.now_unix()? >= authority.expires_at_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        session.state = RepairSessionStateRecord::Cancelled;
        session.attempts_reserved = session.attempts.len() as u16;
        session.updated_at = now.clone();
        let previous = current.reference()?;
        let record = operational_record(
            &layout,
            OperationalRecordWrite {
                id,
                revision: current.revision + 1,
                supersedes: Some(previous),
                idempotency_key: request_id,
                owner: authority.principal,
                evidence_locator: authority.evidence_id,
                recorded_at: now,
                body: RecordBody::RepairSession(session),
            },
        )?;
        Ok(repository.append(&record, Some(current.revision))?)
    }

    /// Handoff replies are deliberately not authority. This only verifies that
    /// a reply targets the exact current session; the owner's actual policy or
    /// scope decision must enter through `wh change` and a new repair session.
    pub fn validate_handoff_reply(
        &self,
        project_dir: &Path,
        handoff_ref: &RecordRef,
        authority_evidence: &RepairAuthorityEvidence,
        request_id: &str,
    ) -> Result<(), RepairHostError> {
        validate_external_request_id(request_id)?;
        let layout = ProjectLayout::resolve(project_dir, None)?;
        let repository = DoltRepository::open_existing(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        )?;
        let handoff = repository
            .get(handoff_ref)?
            .ok_or(RepairHostError::SessionMissing)?;
        let RecordBody::RepairHandoff(body) = handoff.body else {
            return Err(RepairHostError::WrongRecordKind);
        };
        let current = repository
            .get(&body.session)?
            .ok_or(RepairHostError::SessionMissing)?;
        let RecordBody::RepairSession(session) = &current.body else {
            return Err(RepairHostError::WrongRecordKind);
        };
        let latest = repository
            .latest(&current.id)?
            .ok_or(RepairHostError::SessionMissing)?;
        if latest.reference()? != body.session {
            return Err(RepairHostError::Stale {
                expected: body.expected_session_revision,
                actual: latest.revision,
            });
        }
        if session.state != RepairSessionStateRecord::NeedsDecision {
            return Err(RepairHostError::Terminal);
        }
        let authority = self.authorize(
            authority_evidence,
            RepairAuthorityTarget {
                session_id: session.session_id.clone(),
                task: session.task.clone(),
                project: project_scope(&layout),
                authority_revision: session.authority_revision,
                context: session_context(session),
                now_unix: self.clock.now_unix()?,
            },
        )?;
        if authority.principal != session.authority_principal
            || authority.expires_at_unix != session.authority_expires_at_unix
        {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if self.clock.now_unix()? >= authority.expires_at_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        let context = session_context(session);
        let workspace = capture_workspace(layout.project_root(), &context)?;
        if workspace != session.workspace_files {
            return Err(RepairHostError::WorkspaceChanged);
        }
        if body.reviewed_snapshot != session.reviewed_snapshot
            || body.stable_finding_ids != session.current_finding_ids
        {
            return Err(RepairHostError::InvalidCheckResponse);
        }
        Ok(())
    }

    fn authorize(
        &self,
        evidence: &RepairAuthorityEvidence,
        target: RepairAuthorityTarget,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        validate_context(&target.context)?;
        let verified = self.verifier.verify(evidence, &target)?;
        if verified.evidence_id.trim().is_empty()
            || verified.task != target.task
            || verified.project != target.project
            || verified.authority_revision != target.authority_revision
            || !same_context(&verified.context, &target.context)?
        {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if verified.expires_at_unix <= target.now_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        if !verified.may_edit_source
            || verified.may_edit_policy
            || verified.may_edit_checks
            || verified.may_reset_baselines
            || verified.may_publish
            || verified.may_merge
            || verified.may_release
        {
            return Err(RepairHostError::ForbiddenCapability);
        }
        Ok(verified)
    }

    #[allow(clippy::too_many_arguments)]
    fn stop_for_budget(
        &self,
        layout: &ProjectLayout,
        repository: &DoltRepository,
        current: AgreementRecord,
        mut session: Box<RepairSessionRecord>,
        request_id: String,
        recorded_at: String,
        authority: VerifiedRepairAuthority,
        checkpoint: HostCheckpointKind,
    ) -> Result<RepairFeedback, RepairHostError> {
        if !session
            .current_finding_ids
            .iter()
            .any(|finding| finding == "budget:exhausted")
        {
            session.current_finding_ids.push("budget:exhausted".into());
            session.current_finding_ids.sort();
        }
        let check = budget_stop_response(request_id.clone());
        session.state = RepairSessionStateRecord::NeedsDecision;
        session.attempts_reserved = session.attempts.len() as u16;
        session.last_checkpoint_at_unix = self.clock.now_unix()?;
        session.updated_at = recorded_at.clone();
        session.last_check_response =
            serde_json::to_value(&check).map_err(|_| RepairHostError::InvalidCheckResponse)?;
        session.last_checkpoint_kind = checkpoint_kind_name(checkpoint).into();
        let findings = session.current_finding_ids.clone();
        let previous = current.reference()?;
        let record = operational_record(
            layout,
            OperationalRecordWrite {
                id: current.id.clone(),
                revision: current.revision + 1,
                supersedes: Some(previous),
                idempotency_key: request_id.clone(),
                owner: authority.principal,
                evidence_locator: authority.evidence_id,
                recorded_at,
                body: RecordBody::RepairSession(session),
            },
        )?;
        let reference = self.append_transition(
            layout,
            repository,
            &record,
            Some(current.revision),
            Some("Repair budget expired before the next verification operation"),
        )?;
        feedback(
            repository,
            reference,
            RepairSessionStateRecord::NeedsDecision,
            checkpoint,
            check,
            findings,
        )
    }

    fn authorize_session(
        &self,
        layout: &ProjectLayout,
        session: &RepairSessionRecord,
        evidence: &RepairAuthorityEvidence,
    ) -> Result<VerifiedRepairAuthority, RepairHostError> {
        let authority = self.authorize(
            evidence,
            RepairAuthorityTarget {
                session_id: session.session_id.clone(),
                task: session.task.clone(),
                project: project_scope(layout),
                authority_revision: session.authority_revision,
                context: session_context(session),
                now_unix: self.clock.now_unix()?,
            },
        )?;
        if authority.principal != session.authority_principal
            || authority.expires_at != session.authority_expires_at
            || authority.expires_at_unix != session.authority_expires_at_unix
        {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if self.clock.now_unix()? >= authority.expires_at_unix {
            return Err(RepairHostError::AuthorityExpired);
        }
        Ok(authority)
    }

    fn replay_feedback(
        &self,
        project_dir: &Path,
        repository: &DoltRepository,
        record: AgreementRecord,
        checkpoint: HostCheckpointKind,
    ) -> Result<RepairFeedback, RepairHostError> {
        let RecordBody::RepairSession(session) = &record.body else {
            return Err(RepairHostError::WrongRecordKind);
        };
        let layout = ProjectLayout::resolve(project_dir, None)?;
        if repository
            .latest(&record.id)?
            .map(|latest| latest.reference())
            .transpose()?
            != Some(record.reference()?)
            || capture_workspace(layout.project_root(), &session_context(session))?
                != session.workspace_files
        {
            return Err(RepairHostError::WorkspaceChanged);
        }
        if session.last_checkpoint_kind != checkpoint_kind_name(checkpoint) {
            return Err(RepairHostError::AuthorityMismatch);
        }
        if session.state == RepairSessionStateRecord::Ready
            && (self
                .clock
                .now_unix()?
                .saturating_sub(session.started_at_unix)
                >= session.max_elapsed_seconds
                || session.total_resource_units >= session.max_resource_units)
        {
            return Err(RepairHostError::BudgetExhausted);
        }
        let check: ServiceResponse = serde_json::from_value(session.last_check_response.clone())
            .map_err(|_| RepairHostError::InvalidCheckResponse)?;
        if check
            .data
            .get("reason_code")
            .and_then(serde_json::Value::as_str)
            == Some("repair_budget_exhausted")
        {
            if session.state != RepairSessionStateRecord::NeedsDecision
                || !session
                    .current_finding_ids
                    .iter()
                    .any(|finding| finding.starts_with("budget:"))
            {
                return Err(RepairHostError::InvalidCheckResponse);
            }
        } else {
            let (snapshot, findings, _) = check_parts(&check)?;
            if snapshot != session.reviewed_snapshot
                || findings != deterministic_findings(&session.current_finding_ids)
            {
                return Err(RepairHostError::InvalidCheckResponse);
            }
        }
        feedback(
            repository,
            record.reference()?,
            session.state,
            checkpoint,
            check,
            session.current_finding_ids.clone(),
        )
    }

    fn run_check(
        &self,
        project_dir: &Path,
        request_id: &str,
        paths: &[String],
        language: Option<String>,
        rules: Vec<String>,
    ) -> ServiceResponse {
        self.commands.execute_check(CheckRequest {
            project_dir: project_dir.to_path_buf(),
            request_id: Some(format!("{request_id}:check")),
            paths: paths.iter().map(PathBuf::from).collect(),
            language,
            rules,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn claim_operation(
        &self,
        layout: &ProjectLayout,
        repository: &DoltRepository,
        session_record: &AgreementRecord,
        request_id: &str,
        phase: RepairCheckPhaseRecord,
        checkpoint: HostCheckpointKind,
        started_at_unix: u64,
        recorded_at: &str,
    ) -> Result<(), RepairHostError> {
        let session = session_record.reference()?;
        let bytes = serde_json::to_vec(&(session.id.as_str(), session.revision, phase))
            .map_err(|_| RepairHostError::InvalidCheckResponse)?;
        let digest = format!("{:x}", Sha256::digest(bytes));
        let id = RecordId::new(format!("repair.operation.{digest}"))?;
        if repository.latest(&id)?.is_some() {
            return Err(RepairHostError::OperationInProgress);
        }
        let record = operational_record(
            layout,
            OperationalRecordWrite {
                id: id.clone(),
                revision: 1,
                supersedes: None,
                idempotency_key: format!("internal:repair-operation:{digest}"),
                owner: session_record.owner.clone(),
                evidence_locator: "repair-operation-claim".into(),
                recorded_at: recorded_at.into(),
                body: RecordBody::RepairOperationClaim(Box::new(RepairOperationClaimRecord {
                    session: session.clone(),
                    request_id: request_id.into(),
                    expected_session_revision: session_record.revision,
                    phase,
                    checkpoint_kind: checkpoint_kind_name(checkpoint).into(),
                    started_at_unix,
                    resource_units: 1,
                })),
            },
        )?;
        match repository.append_exclusive_guarded(&record, None, &session) {
            Ok(_) => Ok(()),
            Err(StorageError::StaleRevision { expected, actual }) => Err(RepairHostError::Stale {
                expected: expected.unwrap_or(0),
                actual: actual.unwrap_or(0),
            }),
            Err(error) => {
                if repository.latest(&id)?.is_some() {
                    Err(RepairHostError::OperationInProgress)
                } else {
                    Err(RepairHostError::Storage(error))
                }
            }
        }
    }

    fn append_transition(
        &self,
        layout: &ProjectLayout,
        repository: &DoltRepository,
        session_record: &AgreementRecord,
        expected_revision: Option<u64>,
        handoff_reason: Option<&str>,
    ) -> Result<RecordRef, RepairHostError> {
        let Some(reason) = handoff_reason else {
            return Ok(repository.append(session_record, expected_revision)?);
        };
        let handoff = self.handoff_record(layout, session_record, reason)?;
        let references = repository.append_batch(&[
            AppendRequest {
                record: session_record,
                expected_revision,
            },
            AppendRequest {
                record: &handoff,
                expected_revision: None,
            },
        ])?;
        references
            .first()
            .cloned()
            .ok_or(RepairHostError::InvalidCheckResponse)
    }

    fn handoff_record(
        &self,
        layout: &ProjectLayout,
        session_record: &AgreementRecord,
        reason: &str,
    ) -> Result<AgreementRecord, RepairHostError> {
        let RecordBody::RepairSession(session) = &session_record.body else {
            return Err(RepairHostError::WrongRecordKind);
        };
        let session_ref = session_record.reference()?;
        let id = RecordId::new(format!(
            "repair.handoff.{}.{}",
            session.session_id, session_record.revision
        ))?;
        let body = RepairHandoffRecord {
            session: session_ref,
            expected_session_revision: session_record.revision,
            reviewed_snapshot: session.reviewed_snapshot.clone(),
            stable_finding_ids: session.current_finding_ids.clone(),
            question: "Which accountable change should resolve these unresolved findings?".into(),
            recommendation:
                "Keep required policy unchanged and choose an in-scope implementation approach."
                    .into(),
            alternatives: vec![
                "Authorize a separately reviewed scope change through wh change.".into(),
            ],
            impact: reason.into(),
            evidence: vec![EvidenceRef {
                system: "whetstone_check".into(),
                locator: session.last_check_receipt.as_ref().map_or_else(
                    || "unpersisted-check-response".into(),
                    |reference| format!("{}@{}", reference.id.as_str(), reference.revision),
                ),
                digest: session
                    .last_check_receipt
                    .as_ref()
                    .map(|reference| reference.digest.clone()),
            }],
            permitted_next_step:
                "record the owner decision with wh change, then begin a new repair session".into(),
        };
        let record = operational_record(
            layout,
            OperationalRecordWrite {
                id,
                revision: 1,
                supersedes: None,
                idempotency_key: format!(
                    "handoff:{}:{}",
                    session.session_id, session_record.revision
                ),
                owner: session.authority_principal.clone(),
                evidence_locator: "repair-budget".into(),
                recorded_at: session.updated_at.clone(),
                body: RecordBody::RepairHandoff(Box::new(body)),
            },
        )?;
        Ok(record)
    }
}

fn checkpoint_kind_name(kind: HostCheckpointKind) -> &'static str {
    match kind {
        HostCheckpointKind::PostEditHook => "post_edit_hook",
        HostCheckpointKind::ExplicitCheckpoint => "explicit_checkpoint",
    }
}

fn deterministic_findings(findings: &[String]) -> Vec<String> {
    findings
        .iter()
        .filter(|finding| {
            !finding.starts_with("completion:")
                && !finding.starts_with("budget:")
                && !finding.starts_with("authority:")
                && !finding.starts_with("workspace:")
        })
        .cloned()
        .collect()
}

fn check_parts(
    response: &ServiceResponse,
) -> Result<(RepairSnapshotRecord, Vec<String>, Option<RecordRef>), RepairHostError> {
    let report: VerificationReport = serde_json::from_value(
        response
            .data
            .get("report")
            .cloned()
            .ok_or(RepairHostError::InvalidCheckResponse)?,
    )
    .map_err(|_| RepairHostError::InvalidCheckResponse)?;
    if report.lean_baseline_revision != LEAN_BASELINE_REVISION {
        return Err(RepairHostError::InvalidCheckResponse);
    }
    let mut findings = report
        .results
        .iter()
        .flat_map(|check| check.findings.iter().map(stable_finding_id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for result in report.results.iter().filter(|result| {
        result.required
            && result.findings.is_empty()
            && !matches!(result.state, CheckState::Success)
    }) {
        findings.push(format!(
            "check:{}:{}",
            result.requirement_id, result.reason_code
        ));
    }
    findings.sort();
    findings.dedup();
    let receipt = response
        .data
        .get("receipt_record")
        .cloned()
        .filter(|value| !value.is_null())
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| RepairHostError::InvalidCheckResponse)?;
    Ok((
        RepairSnapshotRecord {
            code_tree: report.snapshot.code_tree,
            policy: report.snapshot.policy,
            checker_bundle: report.snapshot.checker_bundle,
            scope: report.snapshot.scope,
            environment: report.snapshot.environment,
            trust: report.snapshot.trust,
        },
        findings,
        receipt,
    ))
}

fn stable_finding_id(finding: &crate::verification::Finding) -> String {
    // Locations and rendered observations move when a worker edits nearby
    // code. Group on the violated requirement and remediation contract so a
    // line shift cannot reset no-progress accounting.
    let bytes = serde_json::to_vec(&(
        finding.file.as_deref(),
        finding.rule_id.as_str(),
        finding.expected.as_str(),
        finding.rationale.as_str(),
        finding.repair_direction.as_str(),
        finding.permitted_next_action.as_str(),
        finding.verification_command.as_str(),
    ))
    .expect("a validated verification finding is always serializable");
    format!("finding:{:x}", Sha256::digest(bytes))
}

fn state_from_check(state: ServiceState) -> RepairSessionStateRecord {
    match state {
        ServiceState::Success => RepairSessionStateRecord::ReadyForFinalVerification,
        ServiceState::Violated => RepairSessionStateRecord::Ready,
        _ => RepairSessionStateRecord::NeedsDecision,
    }
}

fn feedback(
    repository: &DoltRepository,
    reference: RecordRef,
    state: RepairSessionStateRecord,
    checkpoint: HostCheckpointKind,
    check: ServiceResponse,
    findings: Vec<String>,
) -> Result<RepairFeedback, RepairHostError> {
    let handoff = if state == RepairSessionStateRecord::NeedsDecision {
        let session_record = repository
            .get(&reference)?
            .ok_or(RepairHostError::SessionMissing)?;
        let RecordBody::RepairSession(session) = session_record.body else {
            return Err(RepairHostError::WrongRecordKind);
        };
        let handoff_id = RecordId::new(format!(
            "repair.handoff.{}.{}",
            session.session_id, reference.revision
        ))?;
        repository
            .latest(&handoff_id)?
            .map(|record| match record.body {
                RecordBody::RepairHandoff(body) if body.session == reference => Ok(*body),
                _ => Err(RepairHostError::InvalidCheckResponse),
            })
            .transpose()?
    } else {
        None
    };
    Ok(RepairFeedback {
        session: reference,
        lean_baseline_revision: LEAN_BASELINE_REVISION.into(),
        state,
        checkpoint,
        check,
        findings,
        handoff,
        edit_authorized: state == RepairSessionStateRecord::Ready,
        policy_change_authorized: false,
        publish_authorized: false,
        merge_authorized: false,
        release_authorized: false,
        outcome: "unknown".into(),
    })
}

fn budget_stop_response(request_id: String) -> ServiceResponse {
    ServiceResponse {
        schema: crate::service::RESPONSE_SCHEMA.into(),
        schema_version: 1,
        request_id,
        workflow: "check".into(),
        state: ServiceState::NeedsDecision,
        summary:
            "The durable repair budget expired before another verification operation could start."
                .into(),
        expected_revision: None,
        resume_token: None,
        required_snapshot: None,
        evidence: Vec::new(),
        blocking_questions: vec![
            "Should the accountable owner change the task, scope, or budget?".into(),
        ],
        permitted_actions: vec!["inspect the persisted repair handoff".into()],
        data: serde_json::json!({ "reason_code": "repair_budget_exhausted" }),
    }
}

struct OperationalRecordWrite {
    id: RecordId,
    revision: u64,
    supersedes: Option<RecordRef>,
    idempotency_key: String,
    owner: PrincipalRef,
    evidence_locator: String,
    recorded_at: String,
    body: RecordBody,
}

fn operational_record(
    layout: &ProjectLayout,
    write: OperationalRecordWrite,
) -> Result<AgreementRecord, RepairHostError> {
    let OperationalRecordWrite {
        id,
        revision,
        supersedes,
        idempotency_key,
        owner,
        evidence_locator,
        recorded_at,
        body,
    } = write;
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id,
        revision,
        scope: Scope {
            organization: None,
            project: project_scope(layout),
            component: None,
            environment: None,
        },
        owner: owner.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::ExternalObservation,
            recorded_by: owner,
            recorded_at,
            sources: vec![EvidenceRef {
                system: "repair_authority_adapter".into(),
                locator: evidence_locator,
                digest: None,
            }],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes,
        idempotency_key,
        body,
    };
    record.validate()?;
    Ok(record)
}

fn validate_budget(budget: RepairBudget) -> Result<(), RepairHostError> {
    if budget.max_attempts == 0
        || budget.max_repeated_finding == 0
        || budget.max_repeated_finding > budget.max_attempts
        || budget.max_elapsed_seconds == 0
        || budget.max_resource_units == 0
    {
        Err(RepairHostError::BudgetExhausted)
    } else {
        Ok(())
    }
}

fn validate_session_id(value: &str) -> Result<(), RepairHostError> {
    if !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        Ok(())
    } else {
        Err(RepairHostError::InvalidPath(value.into()))
    }
}

fn validate_external_request_id(value: &str) -> Result<(), RepairHostError> {
    if value.trim().is_empty() || value.len() > 256 || value.starts_with("internal:") {
        Err(RepairHostError::InvalidPath("request_id".into()))
    } else {
        Ok(())
    }
}

fn repair_reservation_identity(
    project: &str,
    task: &ExternalRef,
    authority_revision: u64,
) -> Result<(RecordId, String), RepairHostError> {
    let bytes = serde_json::to_vec(&(project, task, authority_revision))
        .map_err(|_| RepairHostError::AuthorityMismatch)?;
    let digest = format!("{:x}", Sha256::digest(bytes));
    Ok((
        RecordId::new(format!("repair.reservation.{digest}"))?,
        format!("internal:repair-reservation:{digest}"),
    ))
}

fn session_record_id(value: &str) -> Result<RecordId, RepairHostError> {
    validate_session_id(value)?;
    Ok(RecordId::new(format!("repair.session.{value}"))?)
}

fn project_scope(layout: &ProjectLayout) -> String {
    format!("project-{}", &layout.project_id()[..12])
}

fn normalized_paths(paths: Vec<String>) -> Result<Vec<String>, RepairHostError> {
    let mut result = BTreeSet::new();
    for path in paths {
        let path = path.trim().trim_start_matches("./").replace('\\', "/");
        let candidate = Path::new(&path);
        if path.is_empty()
            || candidate.is_absolute()
            || candidate
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(RepairHostError::InvalidPath(path));
        }
        result.insert(path);
    }
    Ok(result.into_iter().collect())
}

fn normalized_values(values: Vec<String>) -> Result<Vec<String>, RepairHostError> {
    let mut result = BTreeSet::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || value.len() > 256 {
            return Err(RepairHostError::InvalidPath("repair context value".into()));
        }
        result.insert(value.to_string());
    }
    Ok(result.into_iter().collect())
}

fn validate_context(context: &RepairTaskContext) -> Result<(), RepairHostError> {
    if context.objective.trim().is_empty()
        || context.objective.len() > 4_096
        || context.non_goals.len() > 32
        || context
            .non_goals
            .iter()
            .any(|value| value.trim().is_empty() || value.len() > 1_024)
        || context.applicable_guidance.len() > 64
        || context.allowed_paths.len() > 256
        || context.excluded_paths.len() > 256
        || context.check_paths.len() > 256
        || context.required_rules.len() > 256
        || context.final_check_paths.len() > 256
        || context.final_required_rules.len() > 256
        || context
            .check_language
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 64)
        || context
            .final_check_language
            .as_ref()
            .is_some_and(|value| value.trim().is_empty() || value.len() > 64)
    {
        return Err(RepairHostError::InvalidPath("repair task context".into()));
    }
    if normalized_paths(context.allowed_paths.clone())?.is_empty() {
        return Err(RepairHostError::InvalidPath("allowed_paths".into()));
    }
    normalized_paths(context.excluded_paths.clone())?;
    if normalized_paths(context.check_paths.clone())?.is_empty()
        || normalized_paths(context.final_check_paths.clone())?.is_empty()
    {
        return Err(RepairHostError::InvalidPath("check paths".into()));
    }
    normalized_values(context.required_rules.clone())?;
    normalized_values(context.final_required_rules.clone())?;
    validate_budget(context.budget)?;
    Ok(())
}

fn normalized_context(
    mut context: RepairTaskContext,
) -> Result<RepairTaskContext, RepairHostError> {
    validate_context(&context)?;
    context.objective = context.objective.trim().to_string();
    context.non_goals = context
        .non_goals
        .into_iter()
        .map(|value| value.trim().to_string())
        .collect();
    context.allowed_paths = normalized_paths(context.allowed_paths)?;
    context.excluded_paths = normalized_paths(context.excluded_paths)?;
    context.check_paths = normalized_paths(context.check_paths)?;
    context.check_language = context.check_language.map(|value| value.trim().to_string());
    context.required_rules = normalized_values(context.required_rules)?;
    context.final_check_paths = normalized_paths(context.final_check_paths)?;
    context.final_check_language = context
        .final_check_language
        .map(|value| value.trim().to_string());
    context.final_required_rules = normalized_values(context.final_required_rules)?;
    Ok(context)
}

fn same_context(
    left: &RepairTaskContext,
    right: &RepairTaskContext,
) -> Result<bool, RepairHostError> {
    validate_context(left)?;
    validate_context(right)?;
    Ok(left.objective == right.objective
        && left.non_goals == right.non_goals
        && left.applicable_guidance == right.applicable_guidance
        && normalized_paths(left.allowed_paths.clone())?
            == normalized_paths(right.allowed_paths.clone())?
        && normalized_paths(left.excluded_paths.clone())?
            == normalized_paths(right.excluded_paths.clone())?
        && normalized_paths(left.check_paths.clone())?
            == normalized_paths(right.check_paths.clone())?
        && left.check_language == right.check_language
        && normalized_values(left.required_rules.clone())?
            == normalized_values(right.required_rules.clone())?
        && normalized_paths(left.final_check_paths.clone())?
            == normalized_paths(right.final_check_paths.clone())?
        && left.final_check_language == right.final_check_language
        && normalized_values(left.final_required_rules.clone())?
            == normalized_values(right.final_required_rules.clone())?
        && left.budget == right.budget)
}

fn session_context(session: &RepairSessionRecord) -> RepairTaskContext {
    RepairTaskContext {
        objective: session.objective.clone(),
        non_goals: session.non_goals.clone(),
        applicable_guidance: session.applicable_guidance.clone(),
        allowed_paths: session.allowed_paths.clone(),
        excluded_paths: session.excluded_paths.clone(),
        check_paths: session.check_paths.clone(),
        check_language: session.check_language.clone(),
        required_rules: session.check_rules.clone(),
        final_check_paths: session.final_check_paths.clone(),
        final_check_language: session.final_check_language.clone(),
        final_required_rules: session.final_check_rules.clone(),
        budget: RepairBudget {
            max_attempts: session.max_attempts,
            max_repeated_finding: session.max_repeated_finding,
            max_elapsed_seconds: session.max_elapsed_seconds,
            max_resource_units: session.max_resource_units,
        },
    }
}

fn validate_project_path(project_root: &Path, relative: &str) -> Result<(), RepairHostError> {
    let root = project_root.canonicalize().map_err(StorageError::Io)?;
    let mut candidate = root.join(relative);
    while !candidate.exists() {
        if !candidate.pop() || candidate == root {
            break;
        }
    }
    let resolved = fs::canonicalize(&candidate).map_err(StorageError::Io)?;
    if resolved == root || resolved.starts_with(&root) {
        Ok(())
    } else {
        Err(RepairHostError::InvalidPath(relative.into()))
    }
}

fn protected_repair_path(path: &str) -> bool {
    let file_name = Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or(path);
    let components = path.split('/').collect::<Vec<_>>();
    path == "."
        || file_name == "SKILL.md"
        || file_name == "AGENTS.md"
        || path == "whetstone"
        || path == ".whetstone"
        || path.starts_with("whetstone/")
        || path.starts_with(".whetstone/")
        || path.starts_with("tests/")
        || path.starts_with("test/")
        || path.starts_with("__tests__/")
        || components
            .iter()
            .any(|component| matches!(*component, ".github" | ".githooks" | ".cargo"))
        || components
            .iter()
            .any(|component| matches!(*component, "tests" | "test" | "__tests__" | "fixtures"))
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
        || file_name.starts_with("test_")
        || file_name.contains("_test.")
        || file_name.ends_with(".snap")
        || file_name.starts_with("tsconfig") && file_name.ends_with(".json")
        || file_name.starts_with("jest.config.")
        || file_name.starts_with("vitest.config.")
        || file_name.starts_with("vite.config.")
        || file_name.starts_with("webpack.config.")
        || file_name.starts_with("eslint.config.")
        || file_name.starts_with(".eslintrc")
        || file_name.starts_with(".prettierrc")
        || file_name.contains(".config.")
        || matches!(
            file_name,
            "Cargo.toml"
                | "Cargo.lock"
                | "package.json"
                | "package-lock.json"
                | "pnpm-lock.yaml"
                | "yarn.lock"
                | "pyproject.toml"
                | "pytest.ini"
                | "tox.ini"
                | "setup.cfg"
                | "mypy.ini"
                | ".flake8"
                | "conftest.py"
                | "ruff.toml"
                | ".ruff.toml"
                | "biome.json"
                | "biome.jsonc"
                | "clippy.toml"
                | "deny.toml"
                | "rust-toolchain"
                | "rust-toolchain.toml"
                | "Taskfile.yml"
                | "Taskfile.yaml"
                | "Makefile"
        )
        || path.contains("baseline")
}

fn path_is_within(path: &str, allowed: &str) -> bool {
    allowed == "."
        || path == allowed
        || path
            .strip_prefix(allowed)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn repeated_findings(attempts: &[RepairAttemptRecord]) -> BTreeMap<&str, u16> {
    let mut counts = BTreeMap::new();
    for finding in attempts.iter().flat_map(|attempt| &attempt.finding_ids) {
        *counts.entry(finding.as_str()).or_default() += 1;
    }
    counts
}

fn unix_now() -> Result<u64, RepairHostError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| RepairHostError::Workspace(error.to_string()))
}

fn capture_workspace(
    project_root: &Path,
    _context: &RepairTaskContext,
) -> Result<Vec<RepairWorkspaceFileRecord>, RepairHostError> {
    const MAX_INDEX_BYTES: usize = 4 * 1024 * 1024;
    const MAX_FILES: usize = 20_000;
    let output = Command::new("git")
        .current_dir(project_root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
    if !output.status.success() {
        return Err(RepairHostError::Workspace(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    if output.stdout.len() > MAX_INDEX_BYTES {
        return Err(RepairHostError::WorkspaceTooLarge);
    }
    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|error| RepairHostError::Workspace(error.to_string()))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    // The project directory's permissions are part of the candidate identity
    // too: changing them can alter who may replace every protected control.
    paths.insert(".".to_string());
    let mut stack = vec![project_root.to_path_buf()];
    let mut visited_directories = 0_usize;
    while let Some(directory) = stack.pop() {
        visited_directories += 1;
        if visited_directories > 100_000 {
            return Err(RepairHostError::WorkspaceTooLarge);
        }
        for entry in fs::read_dir(&directory)
            .map_err(|error| RepairHostError::Workspace(error.to_string()))?
        {
            let entry = entry.map_err(|error| RepairHostError::Workspace(error.to_string()))?;
            let absolute = entry.path();
            let relative = absolute
                .strip_prefix(project_root)
                .map_err(|error| RepairHostError::Workspace(error.to_string()))?
                .to_string_lossy()
                .replace('\\', "/");
            let metadata = fs::symlink_metadata(&absolute)
                .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
            if metadata.is_dir() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if matches!(
                    name.as_ref(),
                    ".git" | ".beads" | "node_modules" | "target" | ".venv" | "venv" | ".cache"
                ) {
                    continue;
                }
                // Traverse nested repositories too. Their HEAD/status text is
                // not a content identity: editing an already-dirty file can
                // leave both unchanged. The nested `.git` directory is still
                // excluded above, while every worktree file is content-hashed.
                // Keep the directory itself too: its permission bits affect
                // whether protected controls can be replaced or traversed.
                paths.insert(relative);
                stack.push(absolute);
            } else {
                paths.insert(relative);
            }
            if paths.len() > MAX_FILES {
                return Err(RepairHostError::WorkspaceTooLarge);
            }
        }
    }
    let mut paths = paths.into_iter().collect::<Vec<_>>();
    paths.sort();
    if paths.len() > MAX_FILES {
        return Err(RepairHostError::WorkspaceTooLarge);
    }
    paths
        .into_iter()
        .map(|path| {
            let absolute = project_root.join(&path);
            let mut hasher = Sha256::new();
            let mut has_test_content = false;
            if let Ok(metadata) = fs::symlink_metadata(&absolute) {
                hash_workspace_permissions(&mut hasher, &metadata);
                if metadata.file_type().is_symlink() {
                    hasher.update(b"symlink\0");
                    let target = fs::read_link(&absolute)
                        .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
                    hasher.update(target.as_os_str().as_encoded_bytes());
                } else if metadata.is_file() {
                    hasher.update(b"file\0");
                    let mut file = File::open(&absolute)
                        .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
                    let mut buffer = [0_u8; 64 * 1024];
                    let mut overlap = Vec::new();
                    loop {
                        let read = file
                            .read(&mut buffer)
                            .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
                        if read == 0 {
                            break;
                        }
                        let mut searchable = overlap;
                        searchable.extend_from_slice(&buffer[..read]);
                        has_test_content |= test_content_markers(&searchable);
                        let keep = searchable.len().min(32);
                        overlap = searchable[searchable.len() - keep..].to_vec();
                        hasher.update(&buffer[..read]);
                    }
                } else if metadata.is_dir() && path != "." && absolute.join(".git").exists() {
                    hasher.update(b"gitlink\0");
                    let head = Command::new("git")
                        .current_dir(&absolute)
                        .args(["rev-parse", "HEAD"])
                        .output()
                        .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
                    let status = Command::new("git")
                        .current_dir(&absolute)
                        .args(["status", "--porcelain=v1", "-z"])
                        .output()
                        .map_err(|error| RepairHostError::Workspace(error.to_string()))?;
                    if !head.status.success() || !status.status.success() {
                        return Err(RepairHostError::Workspace(format!(
                            "unreadable nested repository: {path}"
                        )));
                    }
                    hasher.update(&head.stdout);
                    hasher.update(&status.stdout);
                } else if metadata.is_dir() {
                    hasher.update(b"directory\0");
                } else {
                    return Err(RepairHostError::Workspace(format!(
                        "unsupported workspace entry: {path}"
                    )));
                }
            } else {
                hasher.update(b"missing\0");
            }
            Ok(RepairWorkspaceFileRecord {
                protected: protected_repair_path(&path) || has_test_content,
                path,
                digest: ContentDigest::new(format!("sha256:{:x}", hasher.finalize()))?,
            })
        })
        .collect()
}

fn hash_workspace_permissions(hasher: &mut Sha256, metadata: &fs::Metadata) {
    hasher.update(b"permissions\0");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Permission changes can disable executable checks and hooks without
        // changing file bytes. Bind the stable permission bits into both
        // change detection and the final candidate identity.
        hasher.update((metadata.permissions().mode() & 0o7777).to_le_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update([u8::from(metadata.permissions().readonly())]);
    }
}

fn test_content_markers(bytes: &[u8]) -> bool {
    let inline_test_call = [b"describe".as_slice(), b"test".as_slice(), b"it".as_slice()]
        .iter()
        .any(|identifier| contains_unqualified_call(bytes, identifier));
    let rust_attribute_test = bytes.windows(2).enumerate().any(|(index, marker)| {
        marker == b"#["
            && bytes[index + 2..]
                .iter()
                .position(|byte| *byte == b']')
                .filter(|length| *length <= 256)
                .is_some_and(|length| {
                    bytes[index + 2..index + 2 + length]
                        .windows(4)
                        .any(|window| window == b"test")
                })
    });
    inline_test_call
        || rust_attribute_test
        || bytes.split(|byte| *byte == b'\n').any(|line| {
            let line = trim_ascii_whitespace(line);
            if line.starts_with(b"#[") {
                if let Some(end) = line.iter().position(|byte| *byte == b']') {
                    let attribute = &line[2..end];
                    if attribute.windows(4).any(|window| window == b"test") {
                        return true;
                    }
                }
            }
            [
                b"def test_".as_slice(),
                b"async def test_".as_slice(),
                b"mod tests".as_slice(),
                b"func Test".as_slice(),
                b"@Test".as_slice(),
                b"describe(".as_slice(),
                b"describe.".as_slice(),
                b"test(".as_slice(),
                b"test.".as_slice(),
                b"it(".as_slice(),
                b"it.".as_slice(),
            ]
            .iter()
            .any(|marker| line.starts_with(marker))
        })
}

fn contains_unqualified_call(bytes: &[u8], identifier: &[u8]) -> bool {
    bytes
        .windows(identifier.len())
        .enumerate()
        .any(|(index, window)| {
            if window != identifier
                || (index != 0 && {
                    let byte = bytes[index - 1];
                    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'$' || byte == b'.'
                })
            {
                return false;
            }
            bytes[index + identifier.len()..]
                .iter()
                .copied()
                .find(|byte| !byte.is_ascii_whitespace())
                == Some(b'(')
        })
}

fn trim_ascii_whitespace(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(u8::is_ascii_whitespace) {
        value = &value[1..];
    }
    while value.last().is_some_and(u8::is_ascii_whitespace) {
        value = &value[..value.len() - 1];
    }
    value
}

fn workspace_path_protected(files: &[RepairWorkspaceFileRecord], path: &str) -> bool {
    files
        .iter()
        .find(|file| file.path == path)
        .is_some_and(|file| file.protected)
}

fn workspace_changes(
    before: &[RepairWorkspaceFileRecord],
    after: &[RepairWorkspaceFileRecord],
) -> Vec<String> {
    let before = before
        .iter()
        .map(|file| (&file.path, &file.digest))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|file| (&file.path, &file.digest))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(*path) != after.get(*path))
        .map(|path| (*path).clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn workspace_digest(files: &[RepairWorkspaceFileRecord]) -> Result<ContentDigest, RepairHostError> {
    let bytes =
        serde_json::to_vec(files).map_err(|error| RepairHostError::Workspace(error.to_string()))?;
    Ok(ContentDigest::new(format!(
        "sha256:{:x}",
        Sha256::digest(bytes)
    ))?)
}
