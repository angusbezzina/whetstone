//! The decision log: an ordered, digest-checked read model over the records
//! already read from the private and shared Beads stores.
//!
//! It is not another persistence layer. Every record is validated and bound
//! to its content digest; the whole log has one snapshot digest so a view can
//! say when history changed under it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{AgreementRecord, ContentDigest, RecordBody, RecordRef};

/// At most this many items are returned by one inspection.
pub const MAX_HISTORY_ITEMS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryVisibility {
    Private,
    Team,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRecord {
    pub visibility: HistoryVisibility,
    pub record: AgreementRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Resolved(RecordRef),
    Missing(RecordRef),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryItem {
    pub reference: RecordRef,
    pub recorded_at: String,
    pub visibility: HistoryVisibility,
    pub record: Option<AgreementRecord>,
    pub redaction_reason: Option<String>,
    pub links: Vec<LinkState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryPage {
    pub as_of: String,
    pub snapshot_digest: ContentDigest,
    pub items: Vec<HistoryItem>,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryInspectionRequest {
    pub project: String,
    pub as_of: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    pub page_size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_snapshot: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryInspection {
    pub snapshot_digest: ContentDigest,
    pub decision_history: HistoryPage,
}

/// Read-only facade shared by the command and dashboard adapters.
#[derive(Debug, Clone)]
pub struct HistoryInspectionService {
    by_ref: BTreeMap<RecordRef, HistoryRecord>,
    ordered: Vec<RecordRef>,
    snapshot_digest: ContentDigest,
}

impl HistoryInspectionService {
    /// The log over records already read from both stores. A record present
    /// in the shared store is team-visible, never duplicated as private.
    pub fn from_records(
        private: Vec<AgreementRecord>,
        shared: Vec<AgreementRecord>,
    ) -> Result<Self, HistoryError> {
        let shared_refs = shared
            .iter()
            .map(AgreementRecord::reference)
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(HistoryError::Domain)?;
        let mut records = shared
            .into_iter()
            .map(|record| HistoryRecord {
                visibility: HistoryVisibility::Team,
                record,
            })
            .collect::<Vec<_>>();
        for record in private {
            let reference = record.reference().map_err(HistoryError::Domain)?;
            if !shared_refs.contains(&reference) {
                records.push(HistoryRecord {
                    visibility: HistoryVisibility::Private,
                    record,
                });
            }
        }
        Self::build(records)
    }

    pub fn build(records: Vec<HistoryRecord>) -> Result<Self, HistoryError> {
        let mut by_ref = BTreeMap::new();
        for stored in records {
            stored.record.validate().map_err(HistoryError::Domain)?;
            let reference = stored.record.reference().map_err(HistoryError::Domain)?;
            if let Some(existing) = by_ref.insert(reference.clone(), stored.clone()) {
                if existing != stored {
                    return Err(HistoryError::AmbiguousReference(reference));
                }
            }
        }
        let mut ordered = by_ref.keys().cloned().collect::<Vec<_>>();
        ordered.sort_by(|left, right| sort_key(&by_ref[left]).cmp(&sort_key(&by_ref[right])));
        let snapshot_digest = digest_records(&ordered, &by_ref)?;
        Ok(Self {
            by_ref,
            ordered,
            snapshot_digest,
        })
    }

    pub fn snapshot_digest(&self) -> &ContentDigest {
        &self.snapshot_digest
    }

    /// The first `page_size` items recorded at or before `as_of` that match
    /// the search, oldest first.
    pub fn inspect(
        &self,
        request: &HistoryInspectionRequest,
    ) -> Result<HistoryInspection, HistoryError> {
        if request.project.trim().is_empty()
            || request.as_of.len() < 20
            || !request.as_of.ends_with('Z')
            || request.page_size == 0
            || request.page_size > MAX_HISTORY_ITEMS
            || request.search.as_ref().is_some_and(|term| term.len() > 512)
        {
            return Err(HistoryError::InvalidQuery);
        }
        if request
            .expected_snapshot
            .as_ref()
            .is_some_and(|expected| expected != &self.snapshot_digest)
        {
            return Err(HistoryError::StaleSnapshot {
                expected: request.expected_snapshot.clone(),
                actual: self.snapshot_digest.clone(),
            });
        }
        let term = request
            .search
            .as_deref()
            .map(str::trim)
            .filter(|term| !term.is_empty())
            .map(str::to_lowercase);
        let mut items = Vec::new();
        let mut truncated = false;
        for reference in &self.ordered {
            let stored = &self.by_ref[reference];
            if stored.record.provenance.recorded_at.as_str() > request.as_of.as_str()
                || stored.record.scope.project != request.project
            {
                continue;
            }
            if let Some(term) = &term {
                let text = String::from_utf8(
                    stored
                        .record
                        .canonical_json()
                        .map_err(HistoryError::Domain)?,
                )
                .map_err(|error| HistoryError::Serialization(error.to_string()))?;
                if !text.to_lowercase().contains(term) {
                    continue;
                }
            }
            if items.len() == request.page_size {
                truncated = true;
                break;
            }
            items.push(self.render_item(stored)?);
        }
        Ok(HistoryInspection {
            snapshot_digest: self.snapshot_digest.clone(),
            decision_history: HistoryPage {
                as_of: request.as_of.clone(),
                snapshot_digest: self.snapshot_digest.clone(),
                items,
                truncated,
            },
        })
    }

    /// Every visible item as of the request, unfiltered, bounded at
    /// [`MAX_HISTORY_ITEMS`]; `true` when the bound stopped collection.
    pub fn all_decision_items(
        &self,
        base: &HistoryInspectionRequest,
    ) -> Option<(Vec<HistoryItem>, bool)> {
        let page = self
            .inspect(&HistoryInspectionRequest {
                search: None,
                page_size: MAX_HISTORY_ITEMS,
                expected_snapshot: None,
                ..base.clone()
            })
            .ok()?;
        Some((page.decision_history.items, page.decision_history.truncated))
    }

    fn render_item(&self, stored: &HistoryRecord) -> Result<HistoryItem, HistoryError> {
        let reference = stored.record.reference().map_err(HistoryError::Domain)?;
        let links = record_links(&stored.record)
            .into_iter()
            .map(|link| {
                if self.by_ref.contains_key(&link) {
                    LinkState::Resolved(link)
                } else {
                    LinkState::Missing(link)
                }
            })
            .collect();
        Ok(HistoryItem {
            reference,
            recorded_at: stored.record.provenance.recorded_at.clone(),
            visibility: stored.visibility,
            record: Some(stored.record.clone()),
            redaction_reason: None,
            links,
        })
    }
}

fn sort_key(stored: &HistoryRecord) -> (String, String, u64) {
    (
        stored.record.provenance.recorded_at.clone(),
        stored.record.id.as_str().to_string(),
        stored.record.revision,
    )
}

fn digest_records(
    ordered: &[RecordRef],
    by_ref: &BTreeMap<RecordRef, HistoryRecord>,
) -> Result<ContentDigest, HistoryError> {
    let bytes = serde_json::to_vec(
        &ordered
            .iter()
            .map(|reference| &by_ref[reference])
            .collect::<Vec<_>>(),
    )
    .map_err(|error| HistoryError::Serialization(error.to_string()))?;
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))).map_err(HistoryError::Domain)
}

fn record_links(record: &AgreementRecord) -> Vec<RecordRef> {
    let mut links = record.supersedes.iter().cloned().collect::<Vec<_>>();
    match &record.body {
        RecordBody::Proposal(body) => links.extend(body.proposed_records.clone()),
        RecordBody::VerificationReceipt(body) => links.extend(body.related_records.clone()),
        RecordBody::Retirement(body) => {
            links.push(body.target.clone());
            links.extend(body.replacement.clone());
        }
        RecordBody::LocalReview(body) => links.push(body.proposal.clone()),
        RecordBody::Judgment(body) => links.push(body.rule.clone()),
        RecordBody::Attestation(body) => links.push(body.rule.clone()),
        RecordBody::FlagDecision(body) => links.push(body.receipt.clone()),
        RecordBody::HandAnswer(body) => links.push(body.raise.clone()),
        _ => {}
    }
    links.sort();
    links.dedup();
    links
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    InvalidQuery,
    AmbiguousReference(RecordRef),
    StaleSnapshot {
        expected: Option<ContentDigest>,
        actual: ContentDigest,
    },
    Storage(String),
    Serialization(String),
    Domain(crate::domain::DomainError),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for HistoryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        EvidenceRef, Mission, PrincipalKind, PrincipalRef, Provenance, ProvenanceAuthority,
        ProvenanceKind, RecordId, Scope, SCHEMA_VERSION_V1,
    };

    fn owner() -> PrincipalRef {
        PrincipalRef {
            kind: PrincipalKind::LocalUser,
            stable_id: "local:owner".into(),
            display_name: None,
        }
    }

    fn mission(id: &str, at: &str, statement: &str) -> AgreementRecord {
        AgreementRecord {
            schema_version: SCHEMA_VERSION_V1,
            id: RecordId::new(id).expect("id"),
            revision: 1,
            scope: Scope::project("project-test"),
            owner: owner(),
            provenance: Provenance {
                kind: ProvenanceKind::HumanAuthored,
                recorded_by: owner(),
                recorded_at: at.into(),
                sources: vec![EvidenceRef {
                    system: "test".into(),
                    locator: "test".into(),
                    digest: None,
                }],
                authority: ProvenanceAuthority::OwnerAuthored,
            },
            supersedes: None,
            idempotency_key: format!("key-{id}"),
            body: RecordBody::Mission(Mission {
                statement: statement.into(),
                desired_outcomes: Vec::new(),
            }),
        }
    }

    fn request(as_of: &str, search: Option<&str>) -> HistoryInspectionRequest {
        HistoryInspectionRequest {
            project: "project-test".into(),
            as_of: as_of.into(),
            search: search.map(str::to_owned),
            page_size: 100,
            expected_snapshot: None,
        }
    }

    #[test]
    fn the_log_is_ordered_bounded_by_as_of_and_searchable() {
        let first = mission("mission.one", "2026-09-01T10:00:00Z", "Ship small");
        let second = mission("mission.two", "2026-09-02T10:00:00Z", "Prove it works");
        let service =
            HistoryInspectionService::from_records(vec![second, first], Vec::new()).expect("log");
        let all = service
            .inspect(&request("2026-09-30T00:00:00Z", None))
            .expect("inspect");
        let ids = all
            .decision_history
            .items
            .iter()
            .map(|item| item.reference.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["mission.one", "mission.two"]);
        let early = service
            .inspect(&request("2026-09-01T12:00:00Z", None))
            .expect("inspect");
        assert_eq!(early.decision_history.items.len(), 1);
        let found = service
            .inspect(&request("2026-09-30T00:00:00Z", Some("it works")))
            .expect("inspect");
        assert_eq!(found.decision_history.items.len(), 1);
    }

    #[test]
    fn a_changed_log_is_reported_stale_against_an_expected_snapshot() {
        let service = HistoryInspectionService::from_records(
            vec![mission("mission.one", "2026-09-01T10:00:00Z", "Ship small")],
            Vec::new(),
        )
        .expect("log");
        let mut stale = request("2026-09-30T00:00:00Z", None);
        stale.expected_snapshot =
            Some(ContentDigest::new(format!("sha256:{}", "0".repeat(64))).expect("digest"));
        assert!(matches!(
            service.inspect(&stale),
            Err(HistoryError::StaleSnapshot { .. })
        ));
    }

    #[test]
    fn a_shared_record_is_team_visible_and_never_duplicated() {
        let record = mission("mission.one", "2026-09-01T10:00:00Z", "Ship small");
        let service = HistoryInspectionService::from_records(vec![record.clone()], vec![record])
            .expect("log");
        let page = service
            .inspect(&request("2026-09-30T00:00:00Z", None))
            .expect("inspect");
        assert_eq!(page.decision_history.items.len(), 1);
        assert_eq!(
            page.decision_history.items[0].visibility,
            HistoryVisibility::Team
        );
    }
}
