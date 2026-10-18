//! Receipts and decisions that record what happened around a rule: check
//! results, Jev answers and flag decisions. Each is its own record, never a
//! rewrite of another.

use serde::{Deserialize, Serialize};

use super::{
    looks_like_utc_timestamp, require_text, ContentDigest, DomainError, EvidenceRef, RecordId,
    RecordRef,
};

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
pub enum Freshness {
    Fresh,
    Stale,
    Missing,
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

/// Which accepted, installed or experimental policy a receipt ran under.
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

/// The result of one deterministic check, gate, drive proof or maintain pass.
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
    pub(super) fn validate(&self) -> Result<(), DomainError> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JudgmentOutcome {
    /// Jev answered yes above the rule's threshold: the rule looks broken.
    Flag,
    /// Jev answered no.
    Clear,
    /// The answer was too close to even odds to act on.
    LowConfidence,
    /// No key, no network, no driver, or an error: nothing was answered.
    Unavailable,
}

impl JudgmentOutcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::Clear => "clear",
            Self::LowConfidence => "low_confidence",
            Self::Unavailable => "unavailable",
        }
    }
}

/// One Jev answer to one rule's question about one unit of change. The key
/// is never recorded; the question and the exact text sent are recorded only
/// as digests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgmentReceipt {
    pub rule: RecordRef,
    /// The model that answered (or the pinned model when none answered).
    pub model: String,
    pub question_digest: ContentDigest,
    /// Digest of the exact text sent, after redaction.
    pub input_digest: ContentDigest,
    /// What was asked about, for example `src/app.css:12-30` or `commit`.
    pub unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub outcome: JudgmentOutcome,
    /// Probability of yes in basis points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probability_bp: Option<u16>,
    /// Distance from even odds in basis points.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_bp: Option<u16>,
    /// Recorded but not enforced.
    pub shadow: bool,
    /// Digest of what redaction replaced, when anything was replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction: Option<ContentDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub detail: String,
    pub answered_at: String,
}

impl JudgmentReceipt {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.model)?;
        require_text(&self.unit)?;
        require_text(&self.detail)?;
        if !looks_like_utc_timestamp(&self.answered_at) {
            return Err(DomainError::InvalidTimestamp(self.answered_at.clone()));
        }
        if self.probability_bp.is_some_and(|value| value > 10_000)
            || self.confidence_bp.is_some_and(|value| value > 10_000)
        {
            return Err(DomainError::InvalidField(
                "probabilities are basis points up to 10000",
            ));
        }
        let answered = self.probability_bp.is_some();
        if answered == (self.outcome == JudgmentOutcome::Unavailable) {
            return Err(DomainError::InvalidField(
                "an answered judgment has a probability; an unavailable one has none",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagVerdict {
    /// The flag was right: the rule was broken.
    Accept,
    /// The flag was wrong: a false flag.
    Dismiss,
}

impl FlagVerdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Accept => "accepted",
            Self::Dismiss => "dismissed",
        }
    }
}

/// The owner's label on one flag, which gives each rule a false-flag rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagDecision {
    /// The judgment or gate receipt that carried the flag.
    pub receipt: RecordRef,
    pub rule: RecordId,
    pub verdict: FlagVerdict,
    pub reason: String,
    pub decided_by: String,
    pub decided_at: String,
}

impl FlagDecision {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.reason)?;
        require_text(&self.decided_by)?;
        if !looks_like_utc_timestamp(&self.decided_at) {
            return Err(DomainError::InvalidTimestamp(self.decided_at.clone()));
        }
        Ok(())
    }
}
