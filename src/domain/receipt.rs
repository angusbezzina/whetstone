//! Verification receipts: what a check, gate, drive proof or maintain pass
//! found. Each is its own record, never a rewrite of another.

use serde::{Deserialize, Serialize};

use super::{
    looks_like_utc_timestamp, require_text, ContentDigest, DomainError, EvidenceRef, RecordRef,
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
