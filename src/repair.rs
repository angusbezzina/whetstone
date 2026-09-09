//! Bounded, resumable repair-loop state shared by agent-host adapters.
//!
//! Whetstone never edits a project through this module. It records what the
//! already-authorized worker attempted and decides whether another attempt is
//! permitted, a deterministic recheck is required, or accountable input is
//! needed.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::ContentDigest;

pub const REPAIR_SCHEMA: &str = "whetstone.repair-session.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairAuthority {
    pub principal: String,
    pub task_id: String,
    pub allowed_paths: BTreeSet<String>,
    pub may_edit_source: bool,
    pub may_edit_policy: bool,
    pub may_publish: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairBudget {
    pub max_attempts: u16,
    pub max_repeated_finding: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairAttempt {
    pub number: u16,
    pub candidate_digest: ContentDigest,
    pub finding_ids: BTreeSet<String>,
    pub changed_paths: BTreeSet<String>,
    pub observed_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairState {
    Ready,
    MustRecheck,
    Verified,
    NeedsDecision,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairSession {
    pub schema: String,
    pub session_id: String,
    pub reviewed_snapshot: ContentDigest,
    pub policy_digest: ContentDigest,
    pub checker_digest: ContentDigest,
    pub authority: RepairAuthority,
    pub budget: RepairBudget,
    pub attempts: Vec<RepairAttempt>,
    pub state: RepairState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionHandoff {
    pub session_id: String,
    pub reviewed_snapshot: ContentDigest,
    pub stable_finding_ids: Vec<String>,
    pub question: String,
    pub recommendation: String,
    pub alternatives: Vec<String>,
    pub impact: String,
    pub evidence: Vec<String>,
    pub permitted_next_step: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepairError {
    MissingAuthority,
    InvalidBudget,
    Terminal,
    SnapshotChanged,
    OutOfScope(String),
    PolicyMutationForbidden,
    PublicationForbidden,
    AttemptOrder,
    NoProgress,
    Oscillation,
}

impl RepairSession {
    pub fn begin(
        session_id: impl Into<String>,
        reviewed_snapshot: ContentDigest,
        policy_digest: ContentDigest,
        checker_digest: ContentDigest,
        authority: RepairAuthority,
        budget: RepairBudget,
    ) -> Result<Self, RepairError> {
        let session_id = session_id.into();
        if authority.principal.trim().is_empty()
            || authority.task_id.trim().is_empty()
            || !authority.may_edit_source
        {
            return Err(RepairError::MissingAuthority);
        }
        if authority.may_edit_policy || authority.may_publish {
            return Err(RepairError::PolicyMutationForbidden);
        }
        if session_id.trim().is_empty()
            || budget.max_attempts == 0
            || budget.max_repeated_finding == 0
            || budget.max_repeated_finding > budget.max_attempts
        {
            return Err(RepairError::InvalidBudget);
        }
        Ok(Self {
            schema: REPAIR_SCHEMA.into(),
            session_id,
            reviewed_snapshot,
            policy_digest,
            checker_digest,
            authority,
            budget,
            attempts: Vec::new(),
            state: RepairState::Ready,
        })
    }

    /// Records one worker attempt. Persist the returned session before editing
    /// again so a process restart cannot reset the budget.
    pub fn record_attempt(&mut self, attempt: RepairAttempt) -> Result<(), RepairError> {
        if matches!(
            self.state,
            RepairState::Verified | RepairState::NeedsDecision | RepairState::Cancelled
        ) {
            return Err(RepairError::Terminal);
        }
        if attempt.number != self.attempts.len() as u16 + 1 {
            return Err(RepairError::AttemptOrder);
        }
        for path in &attempt.changed_paths {
            if is_policy_path(path) {
                return Err(RepairError::PolicyMutationForbidden);
            }
            if !self.authority.allowed_paths.contains(path) {
                return Err(RepairError::OutOfScope(path.clone()));
            }
        }

        if let Some(previous) = self.attempts.last() {
            if previous.candidate_digest == attempt.candidate_digest
                && previous.finding_ids == attempt.finding_ids
            {
                self.state = RepairState::NeedsDecision;
                return Err(RepairError::NoProgress);
            }
        }
        if self.attempts.len() >= 2 {
            let two_back = &self.attempts[self.attempts.len() - 2];
            if two_back.candidate_digest == attempt.candidate_digest
                && two_back.finding_ids == attempt.finding_ids
            {
                self.state = RepairState::NeedsDecision;
                return Err(RepairError::Oscillation);
            }
        }

        self.attempts.push(attempt);
        let counts = self.finding_counts();
        if self.attempts.len() >= usize::from(self.budget.max_attempts)
            || counts
                .values()
                .any(|count| *count >= self.budget.max_repeated_finding)
        {
            self.state = RepairState::NeedsDecision;
        } else {
            self.state = RepairState::MustRecheck;
        }
        Ok(())
    }

    pub fn accept_recheck(
        &mut self,
        snapshot: &ContentDigest,
        remaining_required_findings: &[String],
    ) -> Result<(), RepairError> {
        if self.state != RepairState::MustRecheck {
            return Err(RepairError::Terminal);
        }
        let last = self.attempts.last().ok_or(RepairError::AttemptOrder)?;
        if snapshot != &last.candidate_digest {
            return Err(RepairError::SnapshotChanged);
        }
        self.state = if remaining_required_findings.is_empty() {
            RepairState::Verified
        } else {
            RepairState::Ready
        };
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.state = RepairState::Cancelled;
    }

    pub fn handoff(
        &self,
        question: impl Into<String>,
        recommendation: impl Into<String>,
        alternatives: Vec<String>,
        impact: impl Into<String>,
        evidence: Vec<String>,
    ) -> Result<DecisionHandoff, RepairError> {
        if self.state != RepairState::NeedsDecision {
            return Err(RepairError::Terminal);
        }
        let finding_ids = self
            .attempts
            .iter()
            .flat_map(|attempt| attempt.finding_ids.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(DecisionHandoff {
            session_id: self.session_id.clone(),
            reviewed_snapshot: self.reviewed_snapshot.clone(),
            stable_finding_ids: finding_ids,
            question: question.into(),
            recommendation: recommendation.into(),
            alternatives,
            impact: impact.into(),
            evidence,
            permitted_next_step: "obtain an accountable owner decision, then start a new snapshot-bound repair session".into(),
        })
    }

    pub fn digest(&self) -> ContentDigest {
        let bytes = serde_json::to_vec(self).expect("repair session serializes");
        ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes)))
            .expect("sha256 is a valid content digest")
    }

    fn finding_counts(&self) -> BTreeMap<&str, u16> {
        let mut counts = BTreeMap::new();
        for finding in self
            .attempts
            .iter()
            .flat_map(|attempt| attempt.finding_ids.iter())
        {
            *counts.entry(finding.as_str()).or_default() += 1;
        }
        counts
    }
}

fn is_policy_path(path: &str) -> bool {
    path == "SKILL.md"
        || path.starts_with("whetstone/")
        || path.starts_with(".whetstone/")
        || path.contains("/whetstone/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn must<T, E: std::fmt::Debug>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(error) => panic!("expected success, got {error:?}"),
        }
    }

    fn digest(byte: char) -> ContentDigest {
        must(ContentDigest::new(format!(
            "sha256:{}",
            byte.to_string().repeat(64)
        )))
    }

    fn session() -> RepairSession {
        must(RepairSession::begin(
            "repair-1",
            digest('a'),
            digest('b'),
            digest('c'),
            RepairAuthority {
                principal: "user:owner".into(),
                task_id: "task-1".into(),
                allowed_paths: ["src/app.rs".into()].into_iter().collect(),
                may_edit_source: true,
                may_edit_policy: false,
                may_publish: false,
            },
            RepairBudget {
                max_attempts: 4,
                max_repeated_finding: 3,
            },
        ))
    }

    fn attempt(number: u16, byte: char, findings: &[&str]) -> RepairAttempt {
        RepairAttempt {
            number,
            candidate_digest: digest(byte),
            finding_ids: findings.iter().map(|value| (*value).into()).collect(),
            changed_paths: ["src/app.rs".into()].into_iter().collect(),
            observed_at_unix: u64::from(number),
        }
    }

    #[test]
    fn missing_or_expanded_authority_is_rejected() {
        let mut authority = session().authority;
        authority.principal.clear();
        assert_eq!(
            RepairSession::begin(
                "x",
                digest('a'),
                digest('b'),
                digest('c'),
                authority,
                RepairBudget {
                    max_attempts: 1,
                    max_repeated_finding: 1
                }
            ),
            Err(RepairError::MissingAuthority)
        );
        let mut active = session();
        let mut out = attempt(1, 'd', &["f1"]);
        out.changed_paths = ["src/other.rs".into()].into_iter().collect();
        assert_eq!(
            active.record_attempt(out),
            Err(RepairError::OutOfScope("src/other.rs".into()))
        );
    }

    #[test]
    fn policy_and_publication_authority_are_never_inherited() {
        let mut active = session();
        let mut policy = attempt(1, 'd', &["f1"]);
        policy.changed_paths = ["whetstone/rules/core.yaml".into()].into_iter().collect();
        assert_eq!(
            active.record_attempt(policy),
            Err(RepairError::PolicyMutationForbidden)
        );
        assert!(!active.authority.may_publish);
    }

    #[test]
    fn attempt_requires_exact_final_recheck_snapshot() {
        let mut active = session();
        must(active.record_attempt(attempt(1, 'd', &[])));
        assert_eq!(
            active.accept_recheck(&digest('e'), &[]),
            Err(RepairError::SnapshotChanged)
        );
        must(active.accept_recheck(&digest('d'), &[]));
        assert_eq!(active.state, RepairState::Verified);
    }

    #[test]
    fn restart_does_not_reset_budget_and_no_progress_stops() {
        let mut active = session();
        must(active.record_attempt(attempt(1, 'd', &["f1"])));
        must(active.accept_recheck(&digest('d'), &["f1".into()]));
        let bytes = must(serde_json::to_vec(&active));
        let mut restored: RepairSession = must(serde_json::from_slice(&bytes));
        assert_eq!(restored.attempts.len(), 1);
        assert_eq!(
            restored.record_attempt(attempt(2, 'd', &["f1"])),
            Err(RepairError::NoProgress)
        );
        assert_eq!(restored.state, RepairState::NeedsDecision);
        assert_eq!(
            must(restored.handoff(
                "Choose",
                "Keep policy",
                vec!["change implementation".into()],
                "blocked",
                vec!["receipt:1".into()]
            ))
            .stable_finding_ids,
            vec!["f1"]
        );
    }

    #[test]
    fn alternating_candidates_are_detected_as_oscillation() {
        let mut active = session();
        must(active.record_attempt(attempt(1, 'd', &["f1"])));
        must(active.accept_recheck(&digest('d'), &["f1".into()]));
        must(active.record_attempt(attempt(2, 'e', &["f2"])));
        must(active.accept_recheck(&digest('e'), &["f2".into()]));
        assert_eq!(
            active.record_attempt(attempt(3, 'd', &["f1"])),
            Err(RepairError::Oscillation)
        );
    }

    #[test]
    fn cancellation_is_terminal() {
        let mut active = session();
        active.cancel();
        assert_eq!(
            active.record_attempt(attempt(1, 'd', &[])),
            Err(RepairError::Terminal)
        );
    }
}
