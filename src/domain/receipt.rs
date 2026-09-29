//! Receipts and decisions that record what happened around a rule: check
//! results, Jev answers, review attestations, briefs, flag decisions and
//! raised hands. Each is its own record, never a rewrite of another.

use serde::{Deserialize, Serialize};

use super::{
    looks_like_utc_timestamp, require_text, ContentDigest, DomainError, EvidenceRef, RecordId,
    RecordRef, Reviewer,
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
pub enum AttestationVerdict {
    Pass,
    Fail,
}

/// A review verdict (pstack `/interrogate` or a named person) for one rule
/// revision at one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Attestation {
    pub rule: RecordRef,
    pub commit: String,
    /// The Git tree of exactly what was reviewed (the working tree at
    /// attestation). The attestation holds only while the checked content
    /// has this tree; absent on earlier attestations, which hold at their
    /// commit only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    pub reviewer: Reviewer,
    pub verdict: AttestationVerdict,
    pub notes: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<EvidenceRef>,
    pub attested_at: String,
}

impl Attestation {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        require_commit(&self.commit)?;
        if let Some(tree) = &self.tree {
            require_commit(tree)?;
        }
        require_text(&self.notes)?;
        if let Reviewer::Person { name } = &self.reviewer {
            require_text(name)?;
        }
        if !looks_like_utc_timestamp(&self.attested_at) {
            return Err(DomainError::InvalidTimestamp(self.attested_at.clone()));
        }
        Ok(())
    }
}

/// What an agent learned before building: the code, components and tokens it
/// will reuse and what could break, from pstack `/how`, `/why` and
/// `/blast-radius`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Brief {
    /// A feature id or a repository path or glob the brief covers.
    pub area: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reuse: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub risks: Vec<String>,
    pub notes: String,
    pub recorded_at: String,
}

impl Brief {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.area)?;
        require_text(&self.notes)?;
        if self.skills.is_empty() || self.skills.len() > 8 {
            return Err(DomainError::InvalidField(
                "a brief names one to eight skills it ran",
            ));
        }
        for item in self.skills.iter().chain(&self.reuse).chain(&self.risks) {
            require_text(item)?;
        }
        if self.reuse.len() > 32 || self.risks.len() > 32 {
            return Err(DomainError::InvalidField(
                "a brief lists at most 32 reuse and 32 risk items",
            ));
        }
        if let Some(commit) = &self.commit {
            require_commit(commit)?;
        }
        if !looks_like_utc_timestamp(&self.recorded_at) {
            return Err(DomainError::InvalidTimestamp(self.recorded_at.clone()));
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandTrigger {
    /// A Jev answer below the rule's confidence bar.
    LowConfidence,
    /// The same flag survived a second repair.
    SecondFailedRepair,
    /// The change touches an area a must rule reserves for the owner.
    MustRuleArea,
    /// The spec is too vague to satisfy the rules.
    VagueSpec,
    /// A rule whose flags are the owner's call.
    RuleFlag,
    /// A must rule's enforcer is unknown or unavailable.
    Unavailable,
}

impl HandTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::LowConfidence => "low_confidence",
            Self::SecondFailedRepair => "second_failed_repair",
            Self::MustRuleArea => "must_rule_area",
            Self::VagueSpec => "vague_spec",
            Self::RuleFlag => "rule_flag",
            Self::Unavailable => "unavailable",
        }
    }
}

/// A question an agent filed for the owner as a Beads issue labelled `human`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandRaise {
    /// The Beads issue that carries the question.
    pub issue: String,
    pub trigger: HandTrigger,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<RecordId>,
    pub question: String,
    pub tried: String,
    pub recommendation: String,
    pub raised_at: String,
}

impl HandRaise {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.issue)?;
        require_text(&self.question)?;
        require_text(&self.tried)?;
        require_text(&self.recommendation)?;
        if !looks_like_utc_timestamp(&self.raised_at) {
            return Err(DomainError::InvalidTimestamp(self.raised_at.clone()));
        }
        Ok(())
    }
}

/// The owner's answer to a raised hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandAnswer {
    pub raise: RecordRef,
    pub issue: String,
    pub answer: String,
    pub answered_by: String,
    pub answered_at: String,
}

impl HandAnswer {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.issue)?;
        require_text(&self.answer)?;
        require_text(&self.answered_by)?;
        if !looks_like_utc_timestamp(&self.answered_at) {
            return Err(DomainError::InvalidTimestamp(self.answered_at.clone()));
        }
        Ok(())
    }
}

fn require_commit(commit: &str) -> Result<(), DomainError> {
    if matches!(commit.len(), 40 | 64) && commit.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(DomainError::InvalidField(
            "a commit is a full hexadecimal object id",
        ))
    }
}
