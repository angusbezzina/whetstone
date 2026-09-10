//! Storage-independent agreement records and lifecycle invariants.
//!
//! These types are the single semantic contract used by every future adapter.
//! They deliberately contain no Dolt, GitHub, CLI, or dashboard concerns.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCHEMA_VERSION_V1: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementRecord {
    pub schema_version: u16,
    pub id: RecordId,
    pub revision: u64,
    pub scope: Scope,
    pub owner: PrincipalRef,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<RecordRef>,
    pub idempotency_key: String,
    #[serde(flatten)]
    pub body: RecordBody,
}

impl AgreementRecord {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema_version != SCHEMA_VERSION_V1 {
            return Err(DomainError::UnsupportedSchema(self.schema_version));
        }
        if self.revision == 0 {
            return Err(DomainError::InvalidField("revision must be positive"));
        }
        if self.idempotency_key.trim().is_empty() {
            return Err(DomainError::InvalidField(
                "idempotency_key must not be empty",
            ));
        }
        self.scope.validate()?;
        self.owner.validate()?;
        self.provenance.validate()?;
        self.body.validate()?;
        if matches!(
            self.body,
            RecordBody::RepairSession(_)
                | RecordBody::RepairHandoff(_)
                | RecordBody::RepairAuthorityReservation(_)
                | RecordBody::RepairOperationClaim(_)
        ) && self.provenance.authority != ProvenanceAuthority::CandidateOnly
        {
            return Err(DomainError::OperationalRecordCannotGrantAuthority);
        }
        if let Some(previous) = &self.supersedes {
            previous.validate()?;
            if previous.id != self.id || previous.revision >= self.revision {
                return Err(DomainError::InvalidSupersession);
            }
        }
        Ok(())
    }

    pub fn canonical_json(&self) -> Result<Vec<u8>, DomainError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|error| DomainError::Serialization(error.to_string()))
    }

    pub fn digest(&self) -> Result<ContentDigest, DomainError> {
        let bytes = self.canonical_json()?;
        Ok(ContentDigest(format!("sha256:{:x}", Sha256::digest(bytes))))
    }

    pub fn reference(&self) -> Result<RecordRef, DomainError> {
        Ok(RecordRef {
            id: self.id.clone(),
            revision: self.revision,
            digest: self.digest()?,
        })
    }

    fn links(&self) -> Vec<RecordRef> {
        let mut links = Vec::new();
        if let Some(previous) = &self.supersedes {
            links.push(previous.clone());
        }
        match &self.body {
            RecordBody::Proposal(body) => links.extend(body.proposed_records.clone()),
            RecordBody::Decision(body) => links.push(body.proposal.clone()),
            RecordBody::Activation(body) => {
                links.push(body.proposal.clone());
                links.push(body.acceptance_decision.clone());
                links.push(body.policy.clone());
            }
            RecordBody::VerificationReceipt(body) => {
                links.extend(body.related_records.clone());
                links.extend(
                    [
                        body.policy_state.accepted.clone(),
                        body.policy_state.required.clone(),
                        body.policy_state.installed.clone(),
                        body.policy_state.experimental.clone(),
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
            RecordBody::ObservationReceipt(body) => {
                links.push(body.metric.clone());
                links.extend(body.related_records.clone());
            }
            RecordBody::Retirement(body) => {
                links.push(body.target.clone());
                links.extend(body.replacement.clone());
            }
            RecordBody::RepairSession(body) => {
                links.extend(body.last_check_receipt.clone());
            }
            RecordBody::RepairHandoff(body) => links.push(body.session.clone()),
            RecordBody::RepairAuthorityReservation(body) => {
                links.extend(body.session.clone());
            }
            RecordBody::RepairOperationClaim(body) => links.push(body.session.clone()),
            RecordBody::LocalReview(body) => links.push(body.proposal.clone()),
            _ => {}
        }
        links
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RecordId(String);

impl RecordId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = (3..=96).contains(&value.len())
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".:_-".contains(character));
        if valid {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidRecordId(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RecordId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RecordId> for String {
    fn from(value: RecordId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentDigest(String);

impl ContentDigest {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let suffix = value.strip_prefix("sha256:");
        if suffix.is_some_and(|hex| {
            hex.len() == 64 && hex.chars().all(|character| character.is_ascii_hexdigit())
        }) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(DomainError::InvalidDigest(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentDigest {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ContentDigest> for String {
    fn from(value: ContentDigest) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordRef {
    pub id: RecordId,
    pub revision: u64,
    pub digest: ContentDigest,
}

impl RecordRef {
    fn validate(&self) -> Result<(), DomainError> {
        if self.revision == 0 {
            Err(DomainError::InvalidField(
                "record reference revision must be positive",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    pub project: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
}

impl Scope {
    pub fn validate(&self) -> Result<(), DomainError> {
        validate_segment("project", &self.project)?;
        for (name, segment) in [
            ("organization", self.organization.as_deref()),
            ("component", self.component.as_deref()),
            ("environment", self.environment.as_deref()),
        ] {
            if let Some(segment) = segment {
                validate_segment(name, segment)?;
            }
        }
        Ok(())
    }

    pub fn contains(&self, other: &Self) -> bool {
        self.project == other.project
            && optional_scope_contains(&self.organization, &other.organization)
            && optional_scope_contains(&self.component, &other.component)
            && optional_scope_contains(&self.environment, &other.environment)
    }
}

fn optional_scope_contains(parent: &Option<String>, child: &Option<String>) -> bool {
    parent.is_none() || parent == child
}

fn validate_segment(name: &'static str, value: &str) -> Result<(), DomainError> {
    let valid = (1..=80).contains(&value.len())
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if valid {
        Ok(())
    } else {
        Err(DomainError::InvalidScope(name, value.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    LocalUser,
    GithubUser,
    GithubTeam,
    GithubApp,
    GithubActions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalRef {
    pub kind: PrincipalKind,
    pub stable_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl PrincipalRef {
    fn validate(&self) -> Result<(), DomainError> {
        if self.stable_id.trim().is_empty() {
            return Err(DomainError::MissingPrincipal);
        }
        if matches!(
            self.kind,
            PrincipalKind::GithubUser
                | PrincipalKind::GithubTeam
                | PrincipalKind::GithubApp
                | PrincipalKind::GithubActions
        ) && !self
            .stable_id
            .chars()
            .all(|character| character.is_ascii_digit())
        {
            return Err(DomainError::InvalidField(
                "GitHub principals require immutable numeric IDs",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    HumanAuthored,
    DependencyDocumentation,
    ImportedNote,
    InferredPreference,
    DeterministicCheck,
    ExternalObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub system: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub kind: ProvenanceKind,
    pub recorded_by: PrincipalRef,
    pub recorded_at: String,
    pub sources: Vec<EvidenceRef>,
    pub authority: ProvenanceAuthority,
}

impl Provenance {
    fn validate(&self) -> Result<(), DomainError> {
        self.recorded_by.validate()?;
        if !looks_like_utc_timestamp(&self.recorded_at) {
            return Err(DomainError::InvalidTimestamp(self.recorded_at.clone()));
        }
        if self.sources.is_empty()
            || self
                .sources
                .iter()
                .any(|source| source.system.trim().is_empty() || source.locator.trim().is_empty())
        {
            return Err(DomainError::MissingProvenance);
        }
        if matches!(
            self.kind,
            ProvenanceKind::ImportedNote | ProvenanceKind::InferredPreference
        ) && self.authority != ProvenanceAuthority::CandidateOnly
        {
            return Err(DomainError::ImportedContentCannotGrantAuthority);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceAuthority {
    CandidateOnly,
    OwnerAuthored,
    IndependentlyApproved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "record_type", content = "record", rename_all = "snake_case")]
pub enum RecordBody {
    Mission(Mission),
    CoreValue(CoreValue),
    ImplementationPhilosophy(ImplementationPhilosophy),
    Standard(Standard),
    Guidance(Guidance),
    MetricDefinition(MetricDefinition),
    SourceSnapshot(SourceSnapshot),
    Proposal(Proposal),
    Decision(Decision),
    Mandate(Mandate),
    Activation(Activation),
    VerificationReceipt(VerificationReceipt),
    ObservationReceipt(ObservationReceipt),
    Retirement(Retirement),
    RepairSession(Box<RepairSessionRecord>),
    RepairHandoff(Box<RepairHandoffRecord>),
    RepairAuthorityReservation(Box<RepairAuthorityReservationRecord>),
    RepairOperationClaim(Box<RepairOperationClaimRecord>),
    Feature(Feature),
    LocalReview(LocalReview),
}

impl RecordBody {
    /// Stable snake_case name matching the serialized `record_type` tag.
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Mission(_) => "mission",
            Self::CoreValue(_) => "core_value",
            Self::ImplementationPhilosophy(_) => "implementation_philosophy",
            Self::Standard(_) => "standard",
            Self::Guidance(_) => "guidance",
            Self::MetricDefinition(_) => "metric_definition",
            Self::SourceSnapshot(_) => "source_snapshot",
            Self::Proposal(_) => "proposal",
            Self::Decision(_) => "decision",
            Self::Mandate(_) => "mandate",
            Self::Activation(_) => "activation",
            Self::VerificationReceipt(_) => "verification_receipt",
            Self::ObservationReceipt(_) => "observation_receipt",
            Self::Retirement(_) => "retirement",
            Self::RepairSession(_) => "repair_session",
            Self::RepairHandoff(_) => "repair_handoff",
            Self::RepairAuthorityReservation(_) => "repair_authority_reservation",
            Self::RepairOperationClaim(_) => "repair_operation_claim",
            Self::Feature(_) => "feature",
            Self::LocalReview(_) => "local_review",
        }
    }

    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Mission(value) => require_text(&value.statement),
            Self::CoreValue(value) => {
                require_text(&value.name)?;
                require_text(&value.description)
            }
            Self::ImplementationPhilosophy(value) => {
                require_text(&value.statement)?;
                require_text(&value.rationale)
            }
            Self::Standard(value) => value.validate(),
            Self::Guidance(value) => {
                require_text(&value.statement)?;
                require_text(&value.rationale)
            }
            Self::MetricDefinition(value) => value.validate(),
            Self::SourceSnapshot(value) => {
                require_text(&value.locator)?;
                require_text(&value.media_type)?;
                if !looks_like_utc_timestamp(&value.captured_at) {
                    return Err(DomainError::InvalidTimestamp(value.captured_at.clone()));
                }
                Ok(())
            }
            Self::Proposal(value) => value.validate(),
            Self::Decision(value) => value.validate(),
            Self::Mandate(value) => value.validate(),
            Self::Activation(value) => value.validate(),
            Self::VerificationReceipt(value) => value.validate(),
            Self::ObservationReceipt(value) => value.validate(),
            Self::Retirement(value) => {
                value.target.validate()?;
                require_text(&value.reason)
            }
            Self::RepairSession(value) => value.validate(),
            Self::RepairHandoff(value) => value.validate(),
            Self::RepairAuthorityReservation(value) => value.validate(),
            Self::RepairOperationClaim(value) => value.validate(),
            Self::Feature(value) => value.validate(),
            Self::LocalReview(value) => value.validate(),
        }
    }
}

/// Exact deterministic snapshot bound to a repair session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairSnapshotRecord {
    pub code_tree: ContentDigest,
    pub policy: ContentDigest,
    pub checker_bundle: ContentDigest,
    pub scope: ContentDigest,
    pub environment: ContentDigest,
    pub trust: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairWorkspaceFileRecord {
    pub path: String,
    pub digest: ContentDigest,
    /// True when changing this file could weaken a check, test, policy, or
    /// baseline. Repair authority never permits such a change.
    pub protected: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairSessionStateRecord {
    Ready,
    ReadyForFinalVerification,
    Verified,
    NeedsDecision,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairCheckPhaseRecord {
    Fast,
    Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairAttemptRecord {
    pub number: u16,
    pub candidate: RepairSnapshotRecord,
    pub finding_ids: Vec<String>,
    pub changed_paths: Vec<String>,
    pub elapsed_seconds: u64,
    pub resource_units: u64,
    pub observed_at_unix: u64,
}

/// Operational state only. Serialized fields never grant authority; every
/// mutation requires the trusted host authority adapter to authenticate this
/// exact binding again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairSessionRecord {
    pub session_id: String,
    pub lean_baseline_revision: String,
    pub authority_principal: PrincipalRef,
    pub task: ExternalRef,
    pub objective: String,
    #[serde(default)]
    pub non_goals: Vec<String>,
    #[serde(default)]
    pub applicable_guidance: Vec<RecordRef>,
    pub authority_revision: u64,
    pub authority_expires_at: String,
    pub authority_expires_at_unix: u64,
    pub allowed_paths: Vec<String>,
    #[serde(default)]
    pub excluded_paths: Vec<String>,
    pub check_paths: Vec<String>,
    pub check_language: Option<String>,
    pub check_rules: Vec<String>,
    pub final_check_paths: Vec<String>,
    pub final_check_language: Option<String>,
    pub final_check_rules: Vec<String>,
    /// Immutable inputs for the broader final-check plan as observed before
    /// any repair edit. Its code digest may change; policy/checker/scope,
    /// environment, and trust may not.
    pub final_baseline_snapshot: RepairSnapshotRecord,
    pub reviewed_snapshot: RepairSnapshotRecord,
    pub reviewed_phase: RepairCheckPhaseRecord,
    pub workspace_files: Vec<RepairWorkspaceFileRecord>,
    pub started_at_unix: u64,
    pub last_checkpoint_at_unix: u64,
    pub max_attempts: u16,
    pub max_repeated_finding: u16,
    pub max_elapsed_seconds: u64,
    pub max_resource_units: u64,
    pub attempts: Vec<RepairAttemptRecord>,
    pub attempts_reserved: u16,
    pub total_elapsed_seconds: u64,
    pub total_resource_units: u64,
    pub final_verification_elapsed_seconds: u64,
    pub final_verification_resource_units: u64,
    pub current_finding_ids: Vec<String>,
    pub state: RepairSessionStateRecord,
    pub updated_at: String,
    /// Canonical response returned for an idempotent retry. Replays never run
    /// another checker outside the persisted operation budget.
    pub last_check_response: serde_json::Value,
    /// `post_edit_hook` or `explicit_checkpoint`; binds response presentation
    /// to the original request rather than the retrying caller.
    pub last_checkpoint_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check_receipt: Option<RecordRef>,
}

impl RepairSessionRecord {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.session_id)?;
        require_text(&self.lean_baseline_revision)?;
        self.authority_principal.validate()?;
        self.task.validate()?;
        require_text(&self.objective)?;
        if self.authority_revision == 0
            || !looks_like_utc_timestamp(&self.authority_expires_at)
            || self.authority_expires_at_unix <= self.started_at_unix
            || !looks_like_utc_timestamp(&self.updated_at)
            || self.allowed_paths.is_empty()
            || self.max_attempts == 0
            || self.max_repeated_finding == 0
            || self.max_repeated_finding > self.max_attempts
            || self.max_elapsed_seconds == 0
            || self.max_resource_units == 0
            || self.attempts.len() > usize::from(self.max_attempts)
            || self.attempts_reserved > self.max_attempts
            || usize::from(self.attempts_reserved) < self.attempts.len()
            || self.started_at_unix == 0
            || self.last_checkpoint_at_unix < self.started_at_unix
            || self.workspace_files.len() > 20_000
            || !self.last_check_response.is_object()
            || !matches!(
                self.last_checkpoint_kind.as_str(),
                "post_edit_hook" | "explicit_checkpoint"
            )
        {
            return Err(DomainError::InvalidRepairRecord);
        }
        let elapsed = self
            .attempts
            .iter()
            .fold(self.final_verification_elapsed_seconds, |total, attempt| {
                total.saturating_add(attempt.elapsed_seconds)
            });
        let resources = self
            .attempts
            .iter()
            .fold(self.final_verification_resource_units, |total, attempt| {
                total.saturating_add(attempt.resource_units)
            });
        if self
            .allowed_paths
            .iter()
            .any(|path| !safe_relative_path(path))
            || self
                .excluded_paths
                .iter()
                .any(|path| !safe_relative_path(path))
            || self
                .check_paths
                .iter()
                .any(|path| !safe_relative_path(path))
            || self
                .final_check_paths
                .iter()
                .any(|path| !safe_relative_path(path))
            || self
                .workspace_files
                .windows(2)
                .any(|pair| pair[0].path >= pair[1].path)
            || self
                .workspace_files
                .iter()
                .any(|file| !safe_relative_path(&file.path))
            || self.non_goals.iter().any(|value| value.trim().is_empty())
            || self
                .applicable_guidance
                .iter()
                .any(|reference| reference.validate().is_err())
            || self
                .current_finding_ids
                .iter()
                .any(|id| id.trim().is_empty())
            || elapsed != self.total_elapsed_seconds
            || resources != self.total_resource_units
            || self.attempts.iter().enumerate().any(|(index, attempt)| {
                attempt.number != index as u16 + 1
                    || attempt
                        .changed_paths
                        .iter()
                        .any(|path| !safe_relative_path(path))
                    || attempt.finding_ids.iter().any(|id| id.trim().is_empty())
                    || attempt.changed_paths.iter().any(|path| {
                        !self
                            .allowed_paths
                            .iter()
                            .any(|allowed| relative_path_contains(allowed, path))
                            || self
                                .excluded_paths
                                .iter()
                                .any(|excluded| relative_path_contains(excluded, path))
                    })
            })
            || match self.reviewed_phase {
                RepairCheckPhaseRecord::Fast => {
                    self.final_verification_elapsed_seconds != 0
                        || self.final_verification_resource_units != 0
                }
                RepairCheckPhaseRecord::Final => {
                    self.final_verification_elapsed_seconds == 0
                        || self.final_verification_resource_units == 0
                        || matches!(
                            self.state,
                            RepairSessionStateRecord::Ready
                                | RepairSessionStateRecord::ReadyForFinalVerification
                                | RepairSessionStateRecord::Cancelled
                        )
                }
            }
            || match self.state {
                RepairSessionStateRecord::Ready => {
                    self.current_finding_ids.is_empty()
                        || usize::from(self.attempts_reserved) != self.attempts.len() + 1
                }
                RepairSessionStateRecord::ReadyForFinalVerification
                | RepairSessionStateRecord::Verified => {
                    !self.current_finding_ids.is_empty()
                        || usize::from(self.attempts_reserved) != self.attempts.len()
                }
                RepairSessionStateRecord::NeedsDecision | RepairSessionStateRecord::Cancelled => {
                    usize::from(self.attempts_reserved) != self.attempts.len()
                }
            }
        {
            return Err(DomainError::InvalidRepairRecord);
        }
        Ok(())
    }
}

/// Permanent private reservation for one task-authority budget. Its
/// deterministic record ID is the database-level uniqueness key used to make
/// concurrent session starts contend inside one transaction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairAuthorityReservationRecord {
    pub project: String,
    pub task: ExternalRef,
    pub authority_revision: u64,
    pub requested_session_id: String,
    pub begin_request_id: String,
    pub started_at_unix: u64,
    pub bootstrap_resource_units: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<RecordRef>,
}

impl RepairAuthorityReservationRecord {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.project)?;
        self.task.validate()?;
        require_text(&self.requested_session_id)?;
        require_text(&self.begin_request_id)?;
        if let Some(session) = &self.session {
            session.validate()?;
        }
        if self.authority_revision == 0
            || self.started_at_unix == 0
            || self.bootstrap_resource_units == 0
        {
            return Err(DomainError::InvalidRepairRecord);
        }
        Ok(())
    }
}

/// Durable claim written before a checkpoint or finalization executes. A
/// process interruption therefore leaves an explicit, budgeted operation
/// rather than allowing silent unbounded reruns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairOperationClaimRecord {
    pub session: RecordRef,
    pub request_id: String,
    pub expected_session_revision: u64,
    pub phase: RepairCheckPhaseRecord,
    pub checkpoint_kind: String,
    pub started_at_unix: u64,
    pub resource_units: u64,
}

impl RepairOperationClaimRecord {
    fn validate(&self) -> Result<(), DomainError> {
        self.session.validate()?;
        require_text(&self.request_id)?;
        if self.expected_session_revision != self.session.revision
            || self.started_at_unix == 0
            || self.resource_units == 0
            || !matches!(
                self.checkpoint_kind.as_str(),
                "post_edit_hook" | "explicit_checkpoint"
            )
        {
            return Err(DomainError::InvalidRepairRecord);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairHandoffRecord {
    pub session: RecordRef,
    pub expected_session_revision: u64,
    pub reviewed_snapshot: RepairSnapshotRecord,
    pub stable_finding_ids: Vec<String>,
    pub question: String,
    pub recommendation: String,
    pub alternatives: Vec<String>,
    pub impact: String,
    pub evidence: Vec<EvidenceRef>,
    pub permitted_next_step: String,
}

impl RepairHandoffRecord {
    fn validate(&self) -> Result<(), DomainError> {
        self.session.validate()?;
        if self.expected_session_revision != self.session.revision
            || self.stable_finding_ids.is_empty()
            || self
                .stable_finding_ids
                .iter()
                .any(|id| id.trim().is_empty())
            || self.alternatives.is_empty()
            || self
                .alternatives
                .iter()
                .any(|value| value.trim().is_empty())
            || self.evidence.is_empty()
            || self
                .evidence
                .iter()
                .any(|value| value.system.trim().is_empty() || value.locator.trim().is_empty())
        {
            return Err(DomainError::InvalidRepairRecord);
        }
        require_text(&self.question)?;
        require_text(&self.recommendation)?;
        require_text(&self.impact)?;
        require_text(&self.permitted_next_step)
    }
}

fn safe_relative_path(value: &str) -> bool {
    let path = std::path::Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
}

fn relative_path_contains(parent: &str, child: &str) -> bool {
    parent == "."
        || child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    pub statement: String,
    #[serde(default)]
    pub desired_outcomes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreValue {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImplementationPhilosophy {
    pub statement: String,
    pub rationale: String,
    #[serde(default)]
    pub review_triggers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StandardStrength {
    Must,
    Should,
    May,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "enforcement", rename_all = "snake_case")]
pub enum Enforcement {
    Ast {
        query: String,
    },
    LintProxy {
        tool: String,
        code: String,
    },
    Formatter {
        tool: String,
    },
    Test {
        command_ref: String,
    },
    Validator {
        command_ref: String,
    },
    /// Prove a mapped feature by driving the running app with the project's
    /// verification driver. Evidence is mandatory; a drive without evidence
    /// is unknown, never a pass.
    Drive {
        feature: RecordId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Standard {
    pub statement: String,
    pub rationale: String,
    pub strength: StandardStrength,
    pub enforcement: Enforcement,
    #[serde(default)]
    pub examples: Vec<String>,
}

impl Standard {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.statement)?;
        require_text(&self.rationale)?;
        match &self.enforcement {
            Enforcement::Ast { query } => require_text(query),
            Enforcement::LintProxy { tool, code } => {
                require_text(tool)?;
                require_text(code)
            }
            Enforcement::Formatter { tool } => require_text(tool),
            Enforcement::Test { command_ref } | Enforcement::Validator { command_ref } => {
                require_text(command_ref)
            }
            Enforcement::Drive { .. } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guidance {
    pub statement: String,
    pub rationale: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

pub const MAX_FEATURE_LIST_ITEMS: usize = 64;
pub const LOCAL_REVIEW_ASSURANCE: &str = "solo-local";

/// A user-facing capability of the governed app: what it is, how a person
/// reaches it, how an agent drives and proves it, and why it exists.
///
/// The record is the source of truth; the agent runbook (four fixed headings
/// plus Why) and the human journal entry are projections of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feature {
    pub name: String,
    pub summary: String,
    /// Grouping used for the sweep order, for example "Dashboard".
    pub area: String,
    #[serde(default)]
    pub sweep_order: u32,
    #[serde(default)]
    pub sub_features: Vec<String>,
    /// How a person reaches the feature, written from the user's point of view.
    pub user_path: String,
    /// Driver commands executed in one session, in order.
    #[serde(default)]
    pub drive_steps: Vec<String>,
    /// The observable end state that proves the feature works.
    pub proof: String,
    #[serde(default)]
    pub gotchas: Vec<String>,
    /// Repository-relative path prefixes or globs whose changes affect this feature.
    #[serde(default)]
    pub entry_points: Vec<String>,
    /// Mission, outcome or metric records this feature exists to serve.
    #[serde(default)]
    pub serves: Vec<RecordId>,
    /// Values, philosophy and guidance that constrain how it is built.
    #[serde(default)]
    pub constrained_by: Vec<RecordId>,
    /// Standards (gates) that prove it.
    #[serde(default)]
    pub proven_by: Vec<RecordId>,
}

impl Feature {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.name)?;
        require_text(&self.summary)?;
        require_text(&self.area)?;
        require_text(&self.user_path)?;
        require_text(&self.proof)?;
        for list in [
            &self.sub_features,
            &self.drive_steps,
            &self.gotchas,
            &self.entry_points,
        ] {
            if list.len() > MAX_FEATURE_LIST_ITEMS {
                return Err(DomainError::InvalidField("feature list exceeds 64 items"));
            }
            for item in list {
                require_text(item)?;
            }
        }
        for entry in &self.entry_points {
            let path = std::path::Path::new(entry);
            if path.is_absolute()
                || entry.contains('\\')
                || path
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(DomainError::InvalidField(
                    "feature entry points must be repository-relative",
                ));
            }
        }
        for links in [&self.serves, &self.constrained_by, &self.proven_by] {
            if links.len() > MAX_FEATURE_LIST_ITEMS {
                return Err(DomainError::InvalidField("feature links exceed 64 items"));
            }
            let unique = links.iter().collect::<BTreeSet<_>>();
            if unique.len() != links.len() {
                return Err(DomainError::InvalidField("feature links must be unique"));
            }
        }
        Ok(())
    }

    /// Whether a repository-relative path is covered by this feature's entry points.
    pub fn covers_path(&self, path: &str) -> bool {
        let path = path.trim_start_matches("./");
        self.entry_points
            .iter()
            .any(|entry| entry_point_matches(entry.trim_start_matches("./"), path))
    }
}

/// Matches a path against a prefix (`src/`, `src/dashboard.rs`) or a simple
/// glob with `*` (one segment) and `**` (any depth).
pub fn entry_point_matches(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') {
        let pattern = pattern.trim_end_matches('/');
        return path == pattern
            || path
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/'));
    }
    glob_match(
        &pattern.split('/').collect::<Vec<_>>(),
        &path.split('/').collect::<Vec<_>>(),
    )
}

fn glob_match(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            glob_match(&pattern[1..], path) || (!path.is_empty() && glob_match(pattern, &path[1..]))
        }
        (Some(segment), Some(candidate)) => {
            segment_match(segment, candidate) && glob_match(&pattern[1..], &path[1..])
        }
        _ => false,
    }
}

fn segment_match(pattern: &str, candidate: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == candidate;
    }
    let mut rest = candidate;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            let Some(stripped) = rest.strip_prefix(part) else {
                return false;
            };
            rest = stripped;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(position) = rest.find(part) {
            rest = &rest[position + part.len()..];
        } else {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalReviewVerdict {
    Accept,
    Withdraw,
}

/// Explicit solo self-review of a private local draft.
///
/// This is not a team decision: it can never activate team policy, and it is
/// only valid for a draft proposal owned by the same principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalReview {
    pub proposal: RecordRef,
    pub verdict: LocalReviewVerdict,
    pub reviewer: PrincipalRef,
    pub assurance: String,
    pub rationale: String,
    pub reviewed_at: String,
    pub team_activation_permitted: bool,
}

impl LocalReview {
    fn validate(&self) -> Result<(), DomainError> {
        self.proposal.validate()?;
        self.reviewer.validate()?;
        require_text(&self.rationale)?;
        if self.assurance != LOCAL_REVIEW_ASSURANCE || self.team_activation_permitted {
            return Err(DomainError::InvalidLocalReview);
        }
        if !looks_like_utc_timestamp(&self.reviewed_at) {
            return Err(DomainError::InvalidTimestamp(self.reviewed_at.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDirection {
    Increase,
    Decrease,
    Maintain,
    Zero,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDefinition {
    pub name: String,
    pub rationale: String,
    pub source: EvidenceRef,
    pub cohort: String,
    pub window: String,
    pub direction: MetricDirection,
    pub threshold: String,
    pub freshness_seconds: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_release: Option<ExternalRef>,
}

impl MetricDefinition {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.name)?;
        require_text(&self.rationale)?;
        require_text(&self.source.system)?;
        require_text(&self.source.locator)?;
        require_text(&self.cohort)?;
        require_text(&self.window)?;
        require_text(&self.threshold)?;
        if self.freshness_seconds == 0 {
            return Err(DomainError::InvalidField(
                "metric freshness_seconds must be positive",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSnapshot {
    pub locator: String,
    pub content_digest: ContentDigest,
    pub media_type: String,
    pub captured_at: String,
    pub untrusted_content: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Draft,
    Shared,
    Declined,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposalBinding {
    pub repository_id: u64,
    pub payload_digest: ContentDigest,
    pub base_active_digest: ContentDigest,
    pub authority_revision: u64,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub state: ProposalState,
    pub title: String,
    pub rationale: String,
    pub proposed_records: Vec<RecordRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ProposalBinding>,
}

impl Proposal {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.title)?;
        require_text(&self.rationale)?;
        if self.proposed_records.is_empty() {
            return Err(DomainError::InvalidField(
                "proposal requires at least one proposed record",
            ));
        }
        for proposed in &self.proposed_records {
            proposed.validate()?;
        }
        match (&self.state, &self.binding) {
            (ProposalState::Draft, None) => Ok(()),
            (ProposalState::Shared, Some(binding))
            | (ProposalState::Declined, Some(binding))
            | (ProposalState::Superseded, Some(binding)) => binding.validate(),
            _ => Err(DomainError::InvalidProposalBinding),
        }
    }
}

impl ProposalBinding {
    fn validate(&self) -> Result<(), DomainError> {
        if self.repository_id == 0 || self.authority_revision == 0 {
            return Err(DomainError::InvalidAuthorityBinding);
        }
        if !looks_like_utc_timestamp(&self.expires_at) {
            return Err(DomainError::InvalidTimestamp(self.expires_at.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionVerdict {
    Accept,
    Decline,
    Redirect,
    ApproveArchive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub proposal: RecordRef,
    pub proposal_binding: ProposalBinding,
    pub verdict: DecisionVerdict,
    pub reviewer: PrincipalRef,
    pub rationale: String,
    pub decided_at: String,
}

impl Decision {
    fn validate(&self) -> Result<(), DomainError> {
        self.proposal.validate()?;
        self.proposal_binding.validate()?;
        self.reviewer.validate()?;
        require_text(&self.rationale)?;
        if !looks_like_utc_timestamp(&self.decided_at) {
            return Err(DomainError::InvalidTimestamp(self.decided_at.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mandate {
    pub objective: String,
    pub allowed_actions: Vec<String>,
    pub excluded_actions: Vec<String>,
    pub expires_at: String,
    pub maximum_attempts: u32,
    pub maximum_minutes: u32,
}

impl Mandate {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.objective)?;
        if self.allowed_actions.is_empty()
            || self.maximum_attempts == 0
            || self.maximum_minutes == 0
        {
            return Err(DomainError::InvalidMandate);
        }
        if !looks_like_utc_timestamp(&self.expires_at) {
            return Err(DomainError::InvalidTimestamp(self.expires_at.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyStateSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted: Option<RecordRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required: Option<RecordRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<RecordRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experimental: Option<RecordRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub proposal: RecordRef,
    pub acceptance_decision: RecordRef,
    pub policy: RecordRef,
    pub binding: ProposalBinding,
    pub activation_sequence: u64,
    pub activated_at: String,
}

impl Activation {
    fn validate(&self) -> Result<(), DomainError> {
        self.proposal.validate()?;
        self.acceptance_decision.validate()?;
        self.policy.validate()?;
        self.binding.validate()?;
        if self.activation_sequence == 0 {
            return Err(DomainError::InvalidActivation);
        }
        if !looks_like_utc_timestamp(&self.activated_at) {
            return Err(DomainError::InvalidTimestamp(self.activated_at.clone()));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationAxis {
    Pass,
    Fail,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationAxis {
    Authorized,
    Denied,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeAxis {
    Improved,
    Maintained,
    Harmed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReceipt {
    pub subject: ExternalRef,
    pub code_digest: ContentDigest,
    pub policy_state: PolicyStateSnapshot,
    pub verification: VerificationAxis,
    pub authorization: AuthorizationAxis,
    pub freshness: Freshness,
    pub checked_at: String,
    #[serde(default)]
    pub related_records: Vec<RecordRef>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

impl VerificationReceipt {
    fn validate(&self) -> Result<(), DomainError> {
        self.subject.validate()?;
        if !looks_like_utc_timestamp(&self.checked_at) {
            return Err(DomainError::InvalidTimestamp(self.checked_at.clone()));
        }
        if self.verification == VerificationAxis::Pass
            && (self.freshness != Freshness::Fresh || self.evidence.is_empty())
        {
            return Err(DomainError::PassRequiresFreshEvidence);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReceipt {
    pub metric: RecordRef,
    pub release: ExternalRef,
    pub observed_at: String,
    pub source_updated_at: String,
    pub freshness: Freshness,
    pub outcome: OutcomeAxis,
    pub sample_size: u64,
    pub summary: String,
    #[serde(default)]
    pub related_records: Vec<RecordRef>,
    #[serde(default)]
    pub evidence: Vec<EvidenceRef>,
}

impl ObservationReceipt {
    fn validate(&self) -> Result<(), DomainError> {
        self.metric.validate()?;
        self.release.validate()?;
        if !looks_like_utc_timestamp(&self.observed_at)
            || !looks_like_utc_timestamp(&self.source_updated_at)
        {
            return Err(DomainError::InvalidTimestamp(self.observed_at.clone()));
        }
        require_text(&self.summary)?;
        if self.freshness == Freshness::Missing && self.outcome != OutcomeAxis::Unknown {
            return Err(DomainError::MissingObservationCannotClaimOutcome);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retirement {
    pub target: RecordRef,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<RecordRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalSystem {
    Beads,
    GithubPullRequest,
    GithubRelease,
    GithubCheckRun,
    Incident,
    Deployment,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalRef {
    pub system: ExternalSystem,
    pub stable_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

impl ExternalRef {
    fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.stable_id)
    }
}

#[derive(Debug, Default)]
pub struct AgreementHistory {
    records: BTreeMap<RecordId, Vec<AgreementRecord>>,
    idempotency: BTreeMap<String, RecordRef>,
}

impl AgreementHistory {
    pub fn append(
        &mut self,
        record: AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, DomainError> {
        record.validate()?;
        if let Some(existing) = self.idempotency.get(&record.idempotency_key) {
            if existing == &record.reference()? {
                return Ok(existing.clone());
            }
            return Err(DomainError::IdempotencyConflict(
                record.idempotency_key.clone(),
            ));
        }

        match &record.body {
            RecordBody::Decision(_) => self.validate_decision(&record)?,
            RecordBody::Activation(_) => self.validate_activation(&record)?,
            RecordBody::ObservationReceipt(_) => self.validate_observation(&record)?,
            RecordBody::LocalReview(_) => self.validate_local_review(&record)?,
            _ => {}
        }

        let previous = self.latest(&record.id);
        validate_proposal_transition(previous, &record)?;

        let versions = self.records.entry(record.id.clone()).or_default();
        let current_revision = versions.last().map(|current| current.revision);
        if current_revision != expected_revision {
            return Err(DomainError::StaleRevision {
                expected: expected_revision,
                actual: current_revision,
            });
        }
        let required_revision = current_revision.map_or(1, |revision| revision + 1);
        if record.revision != required_revision {
            return Err(DomainError::IllegalRevision {
                expected: required_revision,
                actual: record.revision,
            });
        }
        match (versions.last(), &record.supersedes) {
            (None, None) => {}
            (Some(previous), Some(reference)) if previous.reference()? == *reference => {}
            _ => return Err(DomainError::InvalidSupersession),
        }

        let reference = record.reference()?;
        self.idempotency
            .insert(record.idempotency_key.clone(), reference.clone());
        versions.push(record);
        Ok(reference)
    }

    pub fn latest(&self, id: &RecordId) -> Option<&AgreementRecord> {
        self.records.get(id).and_then(|versions| versions.last())
    }

    pub fn by_ref(&self, reference: &RecordRef) -> Option<&AgreementRecord> {
        self.records
            .get(&reference.id)
            .and_then(|versions| {
                versions
                    .iter()
                    .find(|record| record.revision == reference.revision)
            })
            .filter(|record| record.digest().ok().as_ref() == Some(&reference.digest))
    }

    pub fn trace(&self, start: &RecordRef) -> Result<Vec<RecordRef>, DomainError> {
        let mut result = Vec::new();
        let mut queue = vec![start.clone()];
        let mut seen = BTreeSet::new();
        while let Some(reference) = queue.pop() {
            if !seen.insert(reference.clone()) {
                continue;
            }
            let record = self
                .by_ref(&reference)
                .ok_or_else(|| DomainError::UnknownReference(reference.id.clone()))?;
            result.push(reference);
            queue.extend(record.links());
        }
        Ok(result)
    }

    pub fn validate_decision(&self, decision: &AgreementRecord) -> Result<(), DomainError> {
        let RecordBody::Decision(body) = &decision.body else {
            return Err(DomainError::WrongRecordType("decision"));
        };
        let proposal_record = self
            .by_ref(&body.proposal)
            .ok_or_else(|| DomainError::UnknownReference(body.proposal.id.clone()))?;
        let RecordBody::Proposal(proposal) = &proposal_record.body else {
            return Err(DomainError::WrongRecordType("proposal"));
        };
        if proposal.state != ProposalState::Shared {
            return Err(DomainError::DraftCannotBeApproved);
        }
        if proposal.binding.as_ref() != Some(&body.proposal_binding) {
            return Err(DomainError::InvalidAuthorityBinding);
        }
        if decision.owner != body.reviewer {
            return Err(DomainError::PrincipalMismatch);
        }
        if proposal_record.owner.stable_id == body.reviewer.stable_id
            && proposal_record.owner.kind == body.reviewer.kind
        {
            return Err(DomainError::SelfApproval);
        }
        if !proposal_record.scope.contains(&decision.scope) {
            return Err(DomainError::UnauthorizedScope);
        }
        Ok(())
    }

    pub fn validate_activation(&self, activation: &AgreementRecord) -> Result<(), DomainError> {
        let RecordBody::Activation(body) = &activation.body else {
            return Err(DomainError::WrongRecordType("activation"));
        };
        let proposal_record = self
            .by_ref(&body.proposal)
            .ok_or_else(|| DomainError::UnknownReference(body.proposal.id.clone()))?;
        let RecordBody::Proposal(proposal) = &proposal_record.body else {
            return Err(DomainError::WrongRecordType("proposal"));
        };
        if proposal.state != ProposalState::Shared {
            return Err(DomainError::DraftCannotBeActivated);
        }
        let decision_record = self
            .by_ref(&body.acceptance_decision)
            .ok_or_else(|| DomainError::UnknownReference(body.acceptance_decision.id.clone()))?;
        let RecordBody::Decision(decision) = &decision_record.body else {
            return Err(DomainError::WrongRecordType("decision"));
        };
        self.validate_decision(decision_record)?;
        if decision.verdict != DecisionVerdict::Accept
            || decision.proposal != body.proposal
            || decision.proposal_binding != body.binding
        {
            return Err(DomainError::ActivationWithoutAcceptance);
        }
        if !proposal.proposed_records.contains(&body.policy) {
            return Err(DomainError::ActivationPolicyNotProposed);
        }
        if !proposal_record.scope.contains(&activation.scope) {
            return Err(DomainError::UnauthorizedScope);
        }
        Ok(())
    }

    /// A solo review binds an existing draft owned by the reviewer. It never
    /// substitutes for an independent team decision.
    pub fn validate_local_review(&self, review: &AgreementRecord) -> Result<(), DomainError> {
        let RecordBody::LocalReview(body) = &review.body else {
            return Err(DomainError::WrongRecordType("local_review"));
        };
        let proposal_record = self
            .by_ref(&body.proposal)
            .ok_or_else(|| DomainError::UnknownReference(body.proposal.id.clone()))?;
        let RecordBody::Proposal(proposal) = &proposal_record.body else {
            return Err(DomainError::WrongRecordType("proposal"));
        };
        if proposal.state != ProposalState::Draft {
            return Err(DomainError::InvalidLocalReview);
        }
        if review.owner != body.reviewer || proposal_record.owner != body.reviewer {
            return Err(DomainError::PrincipalMismatch);
        }
        if !proposal_record.scope.contains(&review.scope) {
            return Err(DomainError::UnauthorizedScope);
        }
        Ok(())
    }

    pub fn validate_observation(&self, observation: &AgreementRecord) -> Result<(), DomainError> {
        let RecordBody::ObservationReceipt(body) = &observation.body else {
            return Err(DomainError::WrongRecordType("observation_receipt"));
        };
        let metric_record = self
            .by_ref(&body.metric)
            .ok_or_else(|| DomainError::UnknownReference(body.metric.id.clone()))?;
        let RecordBody::MetricDefinition(metric) = &metric_record.body else {
            return Err(DomainError::WrongRecordType("metric_definition"));
        };
        if let Some(expected_release) = &metric.expected_release {
            if expected_release != &body.release {
                return Err(DomainError::ObservationWrongRelease);
            }
        }
        Ok(())
    }
}

fn validate_proposal_transition(
    previous: Option<&AgreementRecord>,
    next: &AgreementRecord,
) -> Result<(), DomainError> {
    let RecordBody::Proposal(next_proposal) = &next.body else {
        return Ok(());
    };
    let previous_state = previous.and_then(|record| match &record.body {
        RecordBody::Proposal(proposal) => Some(&proposal.state),
        _ => None,
    });
    let valid = matches!(
        (previous_state, &next_proposal.state),
        (None, ProposalState::Draft | ProposalState::Shared)
            | (Some(ProposalState::Draft), ProposalState::Shared)
            | (
                Some(ProposalState::Shared),
                ProposalState::Declined | ProposalState::Superseded
            )
    );
    if valid {
        Ok(())
    } else {
        Err(DomainError::IllegalProposalTransition)
    }
}

fn require_text(value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::InvalidField("required text is empty"))
    } else {
        Ok(())
    }
}

fn looks_like_utc_timestamp(value: &str) -> bool {
    value.len() >= 20
        && value.ends_with('Z')
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.contains('T')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    UnsupportedSchema(u16),
    InvalidRecordId(String),
    InvalidDigest(String),
    InvalidScope(&'static str, String),
    InvalidTimestamp(String),
    InvalidField(&'static str),
    MissingPrincipal,
    MissingProvenance,
    ImportedContentCannotGrantAuthority,
    InvalidSupersession,
    InvalidProposalBinding,
    InvalidAuthorityBinding,
    InvalidActivation,
    InvalidMandate,
    PassRequiresFreshEvidence,
    MissingObservationCannotClaimOutcome,
    IdempotencyConflict(String),
    StaleRevision {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    IllegalRevision {
        expected: u64,
        actual: u64,
    },
    UnknownReference(RecordId),
    WrongRecordType(&'static str),
    DraftCannotBeApproved,
    DraftCannotBeActivated,
    SelfApproval,
    PrincipalMismatch,
    UnauthorizedScope,
    ActivationWithoutAcceptance,
    ActivationPolicyNotProposed,
    IllegalProposalTransition,
    ObservationWrongRelease,
    InvalidRepairRecord,
    OperationalRecordCannotGrantAuthority,
    InvalidLocalReview,
    Serialization(String),
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests;
