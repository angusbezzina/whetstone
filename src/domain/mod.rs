//! Storage-independent agreement records and lifecycle invariants.
//!
//! These types are the single semantic contract used by every adapter. They
//! deliberately contain no Beads, GitHub, CLI or dashboard concerns.
//!
//! Removed record kinds (values and philosophy, metrics, exceptions,
//! mandates, activation, observations, repair sessions, team decisions,
//! raised hands, review attestations, briefs, earlier guidance, Jev
//! judgments, flag labels, lessons, features, the feature map and check
//! receipts) stay readable as [`RetiredRecord`]s with their original bytes
//! and digests, so history is never rewritten; they can no longer be created.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::Error as _;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

mod rule;

pub use rule::*;

pub const SCHEMA_VERSION_V1: u16 = 1;

/// Who is acting: a person, or anything not clearly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Person,
    /// Anything not clearly a person.
    #[default]
    Agent,
}

impl ActorKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
        }
    }
}

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
            RecordBody::Retirement(body) => {
                links.push(body.target.clone());
                links.extend(body.replacement.clone());
            }
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

/// Where a record belongs. Whetstone writes the project only; earlier
/// records may carry an organization, component or environment, kept so their
/// digests still verify.
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
    /// The project scope every new record carries.
    pub fn project(project: impl Into<String>) -> Self {
        Self {
            organization: None,
            project: project.into(),
            component: None,
            environment: None,
        }
    }

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

/// Kinds the delegation plan removed. Their records stay readable, byte for
/// byte, as [`RetiredRecord`]s and can never be created again.
pub const RETIRED_KINDS: &[&str] = &[
    "core_value",
    "implementation_philosophy",
    "metric_definition",
    "source_snapshot",
    "decision",
    "mandate",
    "activation",
    "observation_receipt",
    "policy_exception",
    "repair_session",
    "repair_handoff",
    "repair_authority_reservation",
    "repair_operation_claim",
    "hand_raise",
    "hand_answer",
    "attestation",
    "brief",
    "guidance",
    "judgment",
    "flag_decision",
    "lesson",
    "lesson_check",
    "feature",
    "verification_map",
    "verification_receipt",
];

/// A record of a removed kind, kept exactly as stored so its digest still
/// verifies and it still appears in history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredRecord {
    pub record_type: String,
    pub record: serde_json::Value,
}

impl RetiredRecord {
    /// Text from a retired value or philosophy, for principle drafts.
    pub fn principle_text(&self) -> Option<(String, Option<String>)> {
        let text = |field: &str| {
            self.record
                .get(field)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        match self.record_type.as_str() {
            "core_value" => text("description").map(|statement| (statement, text("name"))),
            "implementation_philosophy" => {
                text("statement").map(|statement| (statement, text("rationale")))
            }
            _ => None,
        }
    }
}

/// A rule revision stored with fields the current model reads through a
/// mapping ([`Rule::from_stored`]), kept exactly as stored so its digest
/// still verifies. It is enforced as its mapped rule; revisions are written
/// as plain rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRule {
    pub rule: Rule,
    pub record: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordBody {
    Mission(Mission),
    Principle(Principle),
    Rule(Rule),
    StoredRule(StoredRule),
    Standard(Standard),
    Proposal(Proposal),
    LocalReview(LocalReview),
    Retirement(Retirement),
    Retired(RetiredRecord),
}

#[derive(Serialize)]
#[serde(tag = "record_type", content = "record", rename_all = "snake_case")]
enum BodyRef<'a> {
    Mission(&'a Mission),
    Principle(&'a Principle),
    Rule(&'a Rule),
    Standard(&'a Standard),
    Proposal(&'a Proposal),
    LocalReview(&'a LocalReview),
    Retirement(&'a Retirement),
}

#[derive(Deserialize)]
#[serde(tag = "record_type", content = "record", rename_all = "snake_case")]
enum BodyOwned {
    Mission(Mission),
    Principle(Principle),
    Rule(Rule),
    Standard(Standard),
    Proposal(Proposal),
    LocalReview(LocalReview),
    Retirement(Retirement),
}

impl Serialize for RecordBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let body = match self {
            Self::Retired(retired) => {
                let mut state = serializer.serialize_struct("RecordBody", 2)?;
                state.serialize_field("record_type", &retired.record_type)?;
                state.serialize_field("record", &retired.record)?;
                return state.end();
            }
            Self::StoredRule(stored) => {
                let mut state = serializer.serialize_struct("RecordBody", 2)?;
                state.serialize_field("record_type", "rule")?;
                state.serialize_field("record", &stored.record)?;
                return state.end();
            }
            Self::Mission(value) => BodyRef::Mission(value),
            Self::Principle(value) => BodyRef::Principle(value),
            Self::Rule(value) => BodyRef::Rule(value),
            Self::Standard(value) => BodyRef::Standard(value),
            Self::Proposal(value) => BodyRef::Proposal(value),
            Self::LocalReview(value) => BodyRef::LocalReview(value),
            Self::Retirement(value) => BodyRef::Retirement(value),
        };
        body.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for RecordBody {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            record_type: String,
            record: serde_json::Value,
        }
        let raw = Raw::deserialize(deserializer)?;
        if RETIRED_KINDS.contains(&raw.record_type.as_str()) {
            return Ok(Self::Retired(RetiredRecord {
                record_type: raw.record_type,
                record: raw.record,
            }));
        }
        // A standard that drove a feature is held by nothing any more.
        if raw.record_type == "standard"
            && raw.record.pointer("/enforcement/enforcement") == Some(&"drive".into())
        {
            return Ok(Self::Retired(RetiredRecord {
                record_type: raw.record_type,
                record: raw.record,
            }));
        }
        if raw.record_type == "rule" {
            return match serde_json::from_value::<Rule>(raw.record.clone()) {
                Ok(rule) => Ok(Self::Rule(rule)),
                Err(error) => match Rule::from_stored(&raw.record) {
                    Some(StoredRead::Mapped(rule)) => Ok(Self::StoredRule(StoredRule {
                        rule: *rule,
                        record: raw.record,
                    })),
                    Some(StoredRead::Retired) => Ok(Self::Retired(RetiredRecord {
                        record_type: raw.record_type,
                        record: raw.record,
                    })),
                    None => Err(D::Error::custom(error)),
                },
            };
        }
        let owned: BodyOwned = serde_json::from_value(serde_json::json!({
            "record_type": raw.record_type,
            "record": raw.record,
        }))
        .map_err(D::Error::custom)?;
        Ok(match owned {
            BodyOwned::Mission(value) => Self::Mission(value),
            BodyOwned::Principle(value) => Self::Principle(value),
            BodyOwned::Rule(value) => Self::Rule(value),
            BodyOwned::Standard(value) => Self::Standard(value),
            BodyOwned::Proposal(value) => Self::Proposal(value),
            BodyOwned::LocalReview(value) => Self::LocalReview(value),
            BodyOwned::Retirement(value) => Self::Retirement(value),
        })
    }
}

impl RecordBody {
    /// Stable snake_case name matching the serialized `record_type` tag.
    pub fn type_name(&self) -> &str {
        match self {
            Self::Mission(_) => "mission",
            Self::Principle(_) => "principle",
            Self::Rule(_) | Self::StoredRule(_) => "rule",
            Self::Standard(_) => "standard",
            Self::Proposal(_) => "proposal",
            Self::LocalReview(_) => "local_review",
            Self::Retirement(_) => "retirement",
            Self::Retired(retired) => &retired.record_type,
        }
    }

    /// The rule this body enforces: a v2 rule, or an earlier standard read
    /// through its migration.
    pub fn rule_view(&self) -> Option<std::borrow::Cow<'_, Rule>> {
        match self {
            Self::Rule(rule) => Some(std::borrow::Cow::Borrowed(rule)),
            Self::StoredRule(stored) => Some(std::borrow::Cow::Borrowed(&stored.rule)),
            Self::Standard(standard) => {
                Some(std::borrow::Cow::Owned(Rule::from_standard(standard)))
            }
            _ => None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Mission(value) => require_text(&value.statement),
            Self::Principle(value) => value.validate(),
            Self::Rule(value) => value.validate(),
            Self::StoredRule(value) => value.rule.validate(),
            Self::Standard(value) => value.validate(),
            Self::Proposal(value) => value.validate(),
            Self::LocalReview(value) => value.validate(),
            Self::Retirement(value) => {
                value.target.validate()?;
                require_text(&value.reason)
            }
            // History is kept as stored; its digest is verified on read.
            Self::Retired(_) => Ok(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    pub statement: String,
    /// Kept so earlier missions keep their digests; the plan records outcomes
    /// as principles and rules instead.
    #[serde(default)]
    pub desired_outcomes: Vec<String>,
}

/// Where a principle comes from: a pstack principle pinned to the pstack
/// version it was read from, or the owner's own wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrincipleSource {
    Pstack {
        id: String,
        version: String,
    },
    Custom,
    /// Migrated from an earlier value or implementation philosophy record.
    Migrated {
        record: RecordRef,
    },
}

/// A coding principle the owner picked; rules cite it as their source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principle {
    pub statement: String,
    pub source: PrincipleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Principle {
    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.statement)?;
        if let Some(rationale) = &self.rationale {
            require_text(rationale)?;
        }
        match &self.source {
            PrincipleSource::Pstack { id, version } => {
                require_text(id)?;
                require_text(version)?;
                if !id
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '-')
                {
                    return Err(DomainError::InvalidField(
                        "pstack principle ids are lowercase slugs",
                    ));
                }
                Ok(())
            }
            PrincipleSource::Custom => Ok(()),
            PrincipleSource::Migrated { record } => record.validate(),
        }
    }
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
    Ast { query: String },
    LintProxy { tool: String, code: String },
    Formatter { tool: String },
    Test { command_ref: String },
    Validator { command_ref: String },
}

/// An earlier deterministic rule (v1). It stays readable and in force, and
/// is enforced as [`Rule::from_standard`]; revisions are written as v2 rules.
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
        }
    }
}

pub const LOCAL_REVIEW_ASSURANCE: &str = "solo-local";

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
/// It is only valid for a draft proposal owned by the same principal. Team
/// review is the code host's branch protection, never a Whetstone record; the
/// `team_activation_permitted` field stays `false` and exists so earlier
/// reviews keep their digests.
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
    /// Evidence the decision rested on. Absent on earlier reviews, which keep
    /// their digests.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cites: Vec<EvidenceRef>,
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
        if self.cites.len() > 16
            || self
                .cites
                .iter()
                .any(|cite| cite.system.trim().is_empty() || cite.locator.trim().is_empty())
        {
            return Err(DomainError::InvalidField(
                "a review cites at most 16 named sources",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalState {
    Draft,
    Shared,
    Declined,
    Superseded,
}

/// The binding an earlier team proposal carried for PR-manifest activation.
/// Kept so those proposals keep their digests; nothing writes one now.
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
            return Err(DomainError::InvalidProposalBinding);
        }
        if !looks_like_utc_timestamp(&self.expires_at) {
            return Err(DomainError::InvalidTimestamp(self.expires_at.clone()));
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
        if matches!(record.body, RecordBody::Retired(_)) {
            return Err(DomainError::RetiredKind(
                record.body.type_name().to_string(),
            ));
        }
        self.replay(record, expected_revision)
    }

    /// Load a record that is already stored. Retired kinds are history: they
    /// are replayed beside new records, never written anew (that is
    /// [`Self::append`]'s guard).
    pub fn replay(
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
        if matches!(record.body, RecordBody::LocalReview(_)) {
            self.validate_local_review(&record)?;
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
        if proposal_record.scope.project != review.scope.project {
            return Err(DomainError::UnauthorizedScope);
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

pub(crate) fn require_text(value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::InvalidField("required text is empty"))
    } else {
        Ok(())
    }
}

pub(crate) fn looks_like_utc_timestamp(value: &str) -> bool {
    value.len() >= 20
        && value.ends_with('Z')
        && value.as_bytes().get(4) == Some(&b'-')
        && value.as_bytes().get(7) == Some(&b'-')
        && value.contains('T')
}

pub(crate) fn safe_relative_path(value: &str) -> bool {
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
    PrincipalMismatch,
    UnauthorizedScope,
    IllegalProposalTransition,
    InvalidLocalReview,
    /// A kind the delegation plan removed; it can be read, never created.
    RetiredKind(String),
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
