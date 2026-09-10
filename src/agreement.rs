//! Effective agreement state: which revision of each record is in force,
//! which is a pending local draft, and which was withdrawn.
//!
//! The store is append-only, so a draft revision becomes the latest stored
//! revision of its record. "Latest" is therefore not "in force". This module
//! is the single place that resolves the difference; projections, checks and
//! skill rendering all read through it.

use std::collections::{BTreeMap, BTreeSet};

use crate::domain::{
    AgreementRecord, DecisionVerdict, LocalReviewVerdict, ProposalState, ProvenanceAuthority,
    RecordBody, RecordId, RecordRef,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Lifecycle {
    /// Owner-agreed (onboarding), solo-accepted, or team-accepted.
    Accepted,
    /// A private proposal awaiting review.
    Draft,
    /// Explicitly withdrawn by its owner; kept in history only.
    Withdrawn,
    /// Receipts, sessions and governance records; not agreement content.
    Operational,
}

impl Lifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Draft => "draft",
            Self::Withdrawn => "withdrawn",
            Self::Operational => "operational",
        }
    }
}

/// A draft proposal together with the candidate revisions it proposes.
#[derive(Debug, Clone)]
pub struct PendingProposal<'a> {
    pub proposal: &'a AgreementRecord,
    pub candidates: Vec<&'a AgreementRecord>,
}

#[derive(Debug, Default)]
pub struct AgreementState {
    records: Vec<AgreementRecord>,
    by_ref: BTreeMap<RecordRef, usize>,
    lifecycle: BTreeMap<RecordRef, Lifecycle>,
    proposal_of: BTreeMap<RecordRef, RecordRef>,
}

pub fn is_agreement_body(body: &RecordBody) -> bool {
    matches!(
        body,
        RecordBody::Mission(_)
            | RecordBody::CoreValue(_)
            | RecordBody::ImplementationPhilosophy(_)
            | RecordBody::Standard(_)
            | RecordBody::Guidance(_)
            | RecordBody::MetricDefinition(_)
            | RecordBody::Feature(_)
    )
}

impl AgreementState {
    pub fn from_records(records: Vec<AgreementRecord>) -> Self {
        let mut by_ref = BTreeMap::new();
        for (index, record) in records.iter().enumerate() {
            if let Ok(reference) = record.reference() {
                by_ref.insert(reference, index);
            }
        }
        // Reviews and decisions on proposals, keyed by the exact proposal revision.
        let mut verdicts = BTreeMap::<RecordRef, Lifecycle>::new();
        for record in &records {
            match &record.body {
                RecordBody::LocalReview(review) => {
                    let outcome = match review.verdict {
                        LocalReviewVerdict::Accept => Lifecycle::Accepted,
                        LocalReviewVerdict::Withdraw => Lifecycle::Withdrawn,
                    };
                    verdicts.insert(review.proposal.clone(), outcome);
                }
                RecordBody::Decision(decision) => {
                    let outcome = match decision.verdict {
                        DecisionVerdict::Accept | DecisionVerdict::ApproveArchive => {
                            Lifecycle::Accepted
                        }
                        DecisionVerdict::Decline | DecisionVerdict::Redirect => {
                            Lifecycle::Withdrawn
                        }
                    };
                    verdicts.insert(decision.proposal.clone(), outcome);
                }
                _ => {}
            }
        }
        // The latest revision of each proposal decides its candidates.
        let mut latest_proposals = BTreeMap::<&RecordId, &AgreementRecord>::new();
        for record in &records {
            if matches!(record.body, RecordBody::Proposal(_))
                && latest_proposals
                    .get(&record.id)
                    .map_or(true, |current| current.revision < record.revision)
            {
                latest_proposals.insert(&record.id, record);
            }
        }
        let mut lifecycle = BTreeMap::new();
        let mut proposal_of = BTreeMap::new();
        for proposal in latest_proposals.values() {
            let RecordBody::Proposal(body) = &proposal.body else {
                continue;
            };
            let history = records
                .iter()
                .filter(|record| record.id == proposal.id)
                .filter_map(|record| record.reference().ok())
                .collect::<Vec<_>>();
            let verdict = history.iter().find_map(|reference| verdicts.get(reference));
            let state = match (verdict, &body.state) {
                (Some(outcome), _) => *outcome,
                (None, ProposalState::Declined | ProposalState::Superseded) => Lifecycle::Withdrawn,
                (None, ProposalState::Draft | ProposalState::Shared) => Lifecycle::Draft,
            };
            let Ok(proposal_ref) = proposal.reference() else {
                continue;
            };
            for candidate in &body.proposed_records {
                lifecycle.insert(candidate.clone(), state);
                proposal_of.insert(candidate.clone(), proposal_ref.clone());
            }
        }
        for record in &records {
            let Ok(reference) = record.reference() else {
                continue;
            };
            lifecycle.entry(reference).or_insert_with(|| {
                if !is_agreement_body(&record.body) {
                    Lifecycle::Operational
                } else if record.provenance.authority == ProvenanceAuthority::CandidateOnly {
                    Lifecycle::Draft
                } else {
                    // Owner-agreed onboarding records carry no proposal.
                    Lifecycle::Accepted
                }
            });
        }
        Self {
            records,
            by_ref,
            lifecycle,
            proposal_of,
        }
    }

    pub fn records(&self) -> &[AgreementRecord] {
        &self.records
    }

    pub fn get(&self, reference: &RecordRef) -> Option<&AgreementRecord> {
        self.by_ref
            .get(reference)
            .map(|index| &self.records[*index])
    }

    pub fn lifecycle_of(&self, record: &AgreementRecord) -> Lifecycle {
        record
            .reference()
            .ok()
            .and_then(|reference| self.lifecycle.get(&reference).copied())
            .unwrap_or(Lifecycle::Operational)
    }

    pub fn proposal_for(&self, record: &AgreementRecord) -> Option<&AgreementRecord> {
        let reference = record.reference().ok()?;
        self.proposal_of
            .get(&reference)
            .and_then(|proposal| self.get(proposal))
    }

    fn revisions(&self, id: &RecordId) -> impl Iterator<Item = &AgreementRecord> + '_ {
        let id = id.clone();
        self.records.iter().filter(move |record| record.id == id)
    }

    /// The highest accepted revision of a record.
    pub fn in_force(&self, id: &RecordId) -> Option<&AgreementRecord> {
        self.revisions(id)
            .filter(|record| self.lifecycle_of(record) == Lifecycle::Accepted)
            .max_by_key(|record| record.revision)
    }

    /// The latest stored revision regardless of lifecycle (the change base).
    pub fn latest(&self, id: &RecordId) -> Option<&AgreementRecord> {
        self.revisions(id).max_by_key(|record| record.revision)
    }

    /// A draft revision newer than the one in force, if any.
    pub fn pending(&self, id: &RecordId) -> Option<&AgreementRecord> {
        let in_force = self.in_force(id).map_or(0, |record| record.revision);
        self.revisions(id)
            .filter(|record| {
                record.revision > in_force && self.lifecycle_of(record) == Lifecycle::Draft
            })
            .max_by_key(|record| record.revision)
    }

    /// A draft or accepted revision older than the revision now in force.
    pub fn is_superseded(&self, record: &AgreementRecord) -> bool {
        self.in_force(&record.id)
            .is_some_and(|current| current.revision > record.revision)
    }

    /// Distinct agreement record ids whose body matches, in id order.
    pub fn agreement_ids(&self, predicate: impl Fn(&RecordBody) -> bool) -> Vec<RecordId> {
        self.records
            .iter()
            .filter(|record| is_agreement_body(&record.body) && predicate(&record.body))
            .map(|record| record.id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// In-force records of a kind, in id order.
    pub fn in_force_matching(
        &self,
        predicate: impl Fn(&RecordBody) -> bool,
    ) -> Vec<&AgreementRecord> {
        self.agreement_ids(predicate)
            .iter()
            .filter_map(|id| self.in_force(id))
            .collect()
    }

    /// Draft proposals that have not been reviewed, newest first.
    pub fn pending_proposals(&self) -> Vec<PendingProposal<'_>> {
        let mut latest = BTreeMap::<&RecordId, &AgreementRecord>::new();
        for record in &self.records {
            if matches!(record.body, RecordBody::Proposal(_))
                && latest
                    .get(&record.id)
                    .map_or(true, |current| current.revision < record.revision)
            {
                latest.insert(&record.id, record);
            }
        }
        let mut pending = latest
            .into_values()
            .filter_map(|proposal| {
                let RecordBody::Proposal(body) = &proposal.body else {
                    return None;
                };
                let candidates = body
                    .proposed_records
                    .iter()
                    .filter_map(|reference| self.get(reference))
                    .collect::<Vec<_>>();
                let all_draft = !candidates.is_empty()
                    && candidates.iter().all(|candidate| {
                        self.lifecycle_of(candidate) == Lifecycle::Draft
                            && !self.is_superseded(candidate)
                    });
                all_draft.then_some(PendingProposal {
                    proposal,
                    candidates,
                })
            })
            .collect::<Vec<_>>();
        pending.sort_by(|left, right| {
            right
                .proposal
                .provenance
                .recorded_at
                .cmp(&left.proposal.provenance.recorded_at)
                .then_with(|| left.proposal.id.cmp(&right.proposal.id))
        });
        pending
    }

    /// Stable digest of every in-force agreement revision; changes whenever
    /// accepted content changes. Used to stamp and detect stale projections.
    pub fn in_force_digest(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        for id in self.agreement_ids(|_| true) {
            if let Some(record) = self.in_force(&id) {
                if let Ok(reference) = record.reference() {
                    hasher.update(reference.id.as_str().as_bytes());
                    hasher.update(reference.revision.to_be_bytes());
                    hasher.update(reference.digest.as_str().as_bytes());
                }
            }
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        CoreValue, EvidenceRef, LocalReview, PrincipalKind, PrincipalRef, Proposal, Provenance,
        ProvenanceKind, Scope, LOCAL_REVIEW_ASSURANCE, SCHEMA_VERSION_V1,
    };

    fn owner() -> PrincipalRef {
        PrincipalRef {
            kind: PrincipalKind::LocalUser,
            stable_id: "local:owner".into(),
            display_name: Some("Owner".into()),
        }
    }

    fn record(id: &str, revision: u64, key: &str, body: RecordBody) -> AgreementRecord {
        AgreementRecord {
            schema_version: SCHEMA_VERSION_V1,
            id: RecordId::new(id).expect("id"),
            revision,
            scope: Scope {
                organization: None,
                project: "project-test".into(),
                component: None,
                environment: None,
            },
            owner: owner(),
            provenance: Provenance {
                kind: ProvenanceKind::HumanAuthored,
                recorded_by: owner(),
                recorded_at: format!("2026-09-10T10:00:0{revision}Z"),
                sources: vec![EvidenceRef {
                    system: "test".into(),
                    locator: "test".into(),
                    digest: None,
                }],
                authority: ProvenanceAuthority::OwnerAuthored,
            },
            supersedes: None,
            idempotency_key: key.into(),
            body,
        }
    }

    fn value(text: &str) -> RecordBody {
        RecordBody::CoreValue(CoreValue {
            name: "value".into(),
            description: text.into(),
        })
    }

    fn proposal(id: &str, candidate: &AgreementRecord) -> AgreementRecord {
        record(
            id,
            1,
            &format!("{id}:proposal"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Draft,
                title: "change".into(),
                rationale: "because".into(),
                proposed_records: vec![candidate.reference().expect("ref")],
                binding: None,
            }),
        )
    }

    fn review(
        id: &str,
        proposal: &AgreementRecord,
        verdict: LocalReviewVerdict,
    ) -> AgreementRecord {
        record(
            id,
            1,
            id,
            RecordBody::LocalReview(LocalReview {
                proposal: proposal.reference().expect("ref"),
                verdict,
                reviewer: owner(),
                assurance: LOCAL_REVIEW_ASSURANCE.into(),
                rationale: "explicit".into(),
                reviewed_at: "2026-09-10T11:00:00Z".into(),
                team_activation_permitted: false,
            }),
        )
    }

    #[test]
    fn a_draft_revision_never_displaces_the_accepted_one() {
        let v1 = record("value.evidence", 1, "init", value("Evidence first"));
        let mut v2 = record("value.evidence", 2, "change-1", value("Evidence always"));
        v2.supersedes = Some(v1.reference().expect("ref"));
        let p = proposal("proposal.a", &v2);
        let state = AgreementState::from_records(vec![v1.clone(), v2.clone(), p]);
        let id = RecordId::new("value.evidence").expect("id");
        assert_eq!(state.in_force(&id).map(|r| r.revision), Some(1));
        assert_eq!(state.pending(&id).map(|r| r.revision), Some(2));
        assert_eq!(state.latest(&id).map(|r| r.revision), Some(2));
        assert_eq!(state.pending_proposals().len(), 1);
    }

    #[test]
    fn accept_promotes_and_withdraw_retires_a_draft() {
        let v1 = record("value.evidence", 1, "init", value("Evidence first"));
        let mut v2 = record("value.evidence", 2, "change-1", value("Evidence always"));
        v2.supersedes = Some(v1.reference().expect("ref"));
        let p = proposal("proposal.a", &v2);
        let accepted = AgreementState::from_records(vec![
            v1.clone(),
            v2.clone(),
            p.clone(),
            review("review.a", &p, LocalReviewVerdict::Accept),
        ]);
        let id = RecordId::new("value.evidence").expect("id");
        assert_eq!(accepted.in_force(&id).map(|r| r.revision), Some(2));
        assert!(accepted.pending(&id).is_none());
        assert!(accepted.pending_proposals().is_empty());

        let withdrawn = AgreementState::from_records(vec![
            v1,
            v2.clone(),
            p.clone(),
            review("review.b", &p, LocalReviewVerdict::Withdraw),
        ]);
        assert_eq!(withdrawn.in_force(&id).map(|r| r.revision), Some(1));
        assert!(withdrawn.pending(&id).is_none());
        assert_eq!(withdrawn.lifecycle_of(&v2), Lifecycle::Withdrawn);
    }

    #[test]
    fn a_new_record_that_is_only_drafted_is_not_in_force() {
        let v1 = record("value.new", 1, "change-new", value("New"));
        let p = proposal("proposal.new", &v1);
        let state = AgreementState::from_records(vec![v1.clone(), p]);
        let id = RecordId::new("value.new").expect("id");
        assert!(state.in_force(&id).is_none());
        assert_eq!(state.pending(&id).map(|r| r.revision), Some(1));
        let before = state.in_force_digest();
        let accepted = AgreementState::from_records(vec![
            v1,
            state.records()[1].clone(),
            review(
                "review.new",
                &state.records()[1],
                LocalReviewVerdict::Accept,
            ),
        ]);
        assert_ne!(before, accepted.in_force_digest());
    }
}
