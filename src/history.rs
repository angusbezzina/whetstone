//! Immutable history queries and the distinct present-day active projection.
//!
//! `HistoryIndex` is a deterministic read model over records loaded from the
//! existing private/shareable Dolt stores. It is not another persistence layer.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    AgreementRecord, ContentDigest, DecisionVerdict, PrincipalRef, ProposalState,
    ProvenanceAuthority, RecordBody, RecordId, RecordRef,
};
use crate::storage::{DoltRepository, ProjectLayout, StoreKind};

pub const ACCEPTED_LEAN_BASELINE: &str = "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48";
const SNAPSHOT_SCHEMA_VERSION: u16 = 1;

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
#[serde(tag = "visibility", rename_all = "snake_case", deny_unknown_fields)]
pub enum AccessBoundary {
    Team,
    /// The authenticated local-store owner may inspect all records in that
    /// private repository regardless of which accountable principal authored
    /// an individual record. Transport authentication must establish this
    /// boundary; serialized records can never grant it.
    PrivateStore,
    Private {
        principal: PrincipalRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryMode {
    Historical { as_of: String },
    Active { as_of: String },
}

impl QueryMode {
    fn as_of(&self) -> &str {
        match self {
            Self::Historical { as_of } | Self::Active { as_of } => as_of,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryCursor {
    pub recorded_at: String,
    pub record_id: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    pub project: String,
    pub mode: QueryMode,
    pub access: AccessBoundary,
    pub search: Option<String>,
    pub after: Option<HistoryCursor>,
    pub page_size: usize,
    pub expected_snapshot: Option<ContentDigest>,
    pub redact_private_before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    Resolved(RecordRef),
    Missing(RecordRef),
    Redacted {
        reference: RecordRef,
        reason: String,
    },
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
    pub mode: QueryMode,
    pub snapshot_digest: ContentDigest,
    pub items: Vec<HistoryItem>,
    pub next: Option<HistoryCursor>,
}

/// A single read-only dashboard/service request for the two deliberately
/// separate views of agreement state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryInspectionRequest {
    pub project: String,
    pub as_of: String,
    pub access: AccessBoundary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_after: Option<HistoryCursor>,
    pub page_size: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_snapshot: Option<ContentDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redact_private_before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryInspection {
    pub snapshot_digest: ContentDigest,
    pub active_context: HistoryPage,
    pub decision_history: HistoryPage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryTraceRequest {
    pub root: RecordRef,
    pub project: String,
    pub access: AccessBoundary,
    pub as_of: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySourceQuery {
    pub system: String,
    pub locator: String,
    pub project: String,
    pub access: AccessBoundary,
    pub as_of: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceResolution {
    Missing,
    Exact {
        stable_id: RecordId,
        lineage: Vec<RecordRef>,
    },
    Ambiguous {
        stable_ids: Vec<RecordId>,
    },
}

/// Logical transfer format only. The authoritative store remains Dolt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistorySnapshot {
    pub schema_version: u16,
    pub accepted_lean_baseline: String,
    pub records: Vec<HistoryRecord>,
    pub records_digest: ContentDigest,
}

#[derive(Debug, Clone)]
pub struct HistoryIndex {
    by_ref: BTreeMap<RecordRef, HistoryRecord>,
    ordered: Vec<RecordRef>,
    snapshot_digest: ContentDigest,
}

/// Typed read-only facade shared by command and dashboard adapters.
///
/// Construction opens only repositories that already exist. Inspection never
/// calls the storage initializer and therefore cannot silently create empty
/// history when state is missing or damaged.
#[derive(Debug, Clone)]
pub struct HistoryInspectionService {
    index: HistoryIndex,
}

impl HistoryInspectionService {
    pub fn open(layout: &ProjectLayout) -> Result<Self, HistoryError> {
        let private = DoltRepository::open_existing(
            &layout.store_path(StoreKind::Private),
            StoreKind::Private,
        )
        .map_err(|error| HistoryError::Storage(error.to_string()))?;
        let shareable_path = layout.store_path(StoreKind::Shareable);
        if !shareable_path.exists() {
            return Self::from_private_repository(&private);
        }
        let shareable = DoltRepository::open_existing(&shareable_path, StoreKind::Shareable)
            .map_err(|error| HistoryError::Storage(error.to_string()))?;
        Self::from_repositories(&private, &shareable)
    }

    /// Build from records already read from the private store, avoiding a
    /// second read when the caller holds them.
    pub fn from_private_records(records: Vec<AgreementRecord>) -> Result<Self, HistoryError> {
        Ok(Self {
            index: HistoryIndex::build(
                records
                    .into_iter()
                    .map(|record| HistoryRecord {
                        visibility: HistoryVisibility::Private,
                        record,
                    })
                    .collect(),
            )?,
        })
    }

    fn from_private_repository(private: &DoltRepository) -> Result<Self, HistoryError> {
        if private.kind() != StoreKind::Private {
            return Err(HistoryError::InvalidStoreBoundary);
        }
        let records = private
            .all_records()
            .map_err(|error| HistoryError::Storage(error.to_string()))?
            .into_iter()
            .map(|record| HistoryRecord {
                visibility: HistoryVisibility::Private,
                record,
            })
            .collect();
        Ok(Self {
            index: HistoryIndex::build(records)?,
        })
    }

    pub fn from_repositories(
        private: &DoltRepository,
        shareable: &DoltRepository,
    ) -> Result<Self, HistoryError> {
        Ok(Self {
            index: HistoryIndex::from_repositories(private, shareable)?,
        })
    }

    pub fn snapshot_digest(&self) -> &ContentDigest {
        self.index.snapshot_digest()
    }

    pub fn inspect(
        &self,
        request: &HistoryInspectionRequest,
    ) -> Result<HistoryInspection, HistoryError> {
        let base = HistoryQuery {
            project: request.project.clone(),
            mode: QueryMode::Historical {
                as_of: request.as_of.clone(),
            },
            access: request.access.clone(),
            search: request.search.clone(),
            after: request.history_after.clone(),
            page_size: request.page_size,
            expected_snapshot: request.expected_snapshot.clone(),
            redact_private_before: request.redact_private_before.clone(),
        };
        let decision_history = self.index.query(&base)?;
        let active_context = self.index.query(&HistoryQuery {
            mode: QueryMode::Active {
                as_of: request.as_of.clone(),
            },
            // A history cursor must not accidentally truncate the independent
            // current-context view.
            after: None,
            ..base
        })?;
        Ok(HistoryInspection {
            snapshot_digest: self.index.snapshot_digest().clone(),
            active_context,
            decision_history,
        })
    }

    pub fn query(&self, request: &HistoryQuery) -> Result<HistoryPage, HistoryError> {
        self.index.query(request)
    }

    pub fn trace(&self, request: &HistoryTraceRequest) -> Result<Vec<HistoryItem>, HistoryError> {
        validate_project_as_of(&request.project, &request.as_of)?;
        self.index.trace(
            &request.root,
            &request.project,
            &request.access,
            &request.as_of,
        )
    }

    pub fn resolve_source(
        &self,
        request: &HistorySourceQuery,
    ) -> Result<SourceResolution, HistoryError> {
        validate_project_as_of(&request.project, &request.as_of)?;
        if request.system.trim().is_empty() || request.locator.trim().is_empty() {
            return Err(HistoryError::InvalidQuery);
        }
        self.index.resolve_source(
            &request.system,
            &request.locator,
            &request.project,
            &request.access,
            &request.as_of,
        )
    }
}

impl HistoryIndex {
    /// Loads the immutable read model from the existing private and shareable
    /// Dolt repositories. A record already present in the shareable store is
    /// rendered as team-visible rather than duplicated as private ancestry.
    pub fn from_repositories(
        private: &DoltRepository,
        shareable: &DoltRepository,
    ) -> Result<Self, HistoryError> {
        if private.kind() != StoreKind::Private || shareable.kind() != StoreKind::Shareable {
            return Err(HistoryError::InvalidStoreBoundary);
        }
        let shared = shareable
            .all_records()
            .map_err(|error| HistoryError::Storage(error.to_string()))?;
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
        for record in private
            .all_records()
            .map_err(|error| HistoryError::Storage(error.to_string()))?
        {
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

    pub fn query(&self, request: &HistoryQuery) -> Result<HistoryPage, HistoryError> {
        validate_query(request)?;
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

        let selected = match &request.mode {
            QueryMode::Historical { .. } => self.historical_refs(request)?,
            QueryMode::Active { .. } => self.active_refs(request)?,
        };
        let mut items = Vec::new();
        let mut has_more = false;
        for reference in selected {
            let stored = &self.by_ref[&reference];
            let cursor = cursor_for(stored);
            if request.after.as_ref().is_some_and(|after| cursor <= *after) {
                continue;
            }
            if items.len() == request.page_size {
                has_more = true;
                break;
            }
            items.push(self.render_item(stored, request)?);
        }
        let next = has_more
            .then(|| {
                items.last().map(|item| HistoryCursor {
                    recorded_at: item.recorded_at.clone(),
                    record_id: item.reference.id.as_str().to_string(),
                    revision: item.reference.revision,
                })
            })
            .flatten();
        Ok(HistoryPage {
            mode: request.mode.clone(),
            snapshot_digest: self.snapshot_digest.clone(),
            items,
            next,
        })
    }

    pub fn trace(
        &self,
        root: &RecordRef,
        project: &str,
        access: &AccessBoundary,
        as_of: &str,
    ) -> Result<Vec<HistoryItem>, HistoryError> {
        let root_record = self
            .by_ref
            .get(root)
            .ok_or_else(|| HistoryError::UnknownReference(root.clone()))?;
        authorize(root_record, project, access)?;
        let mut queue = VecDeque::from([root.clone()]);
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        while let Some(reference) = queue.pop_front() {
            if !seen.insert(reference.clone()) {
                continue;
            }
            let stored = self
                .by_ref
                .get(&reference)
                .ok_or_else(|| HistoryError::UnknownReference(reference.clone()))?;
            if stored.record.provenance.recorded_at.as_str() > as_of {
                return Err(HistoryError::FutureReference(reference));
            }
            authorize(stored, project, access)?;
            let query = HistoryQuery {
                project: project.to_string(),
                mode: QueryMode::Historical {
                    as_of: as_of.to_string(),
                },
                access: access.clone(),
                search: None,
                after: None,
                page_size: usize::MAX,
                expected_snapshot: None,
                redact_private_before: None,
            };
            result.push(self.render_item(stored, &query)?);
            let mut links = record_links(&stored.record);
            links.sort();
            for link in links {
                let linked = self
                    .by_ref
                    .get(&link)
                    .ok_or_else(|| HistoryError::UnknownReference(link.clone()))?;
                authorize(linked, project, access)?;
                queue.push_back(link);
            }
        }
        Ok(result)
    }

    pub fn resolve_source(
        &self,
        system: &str,
        locator: &str,
        project: &str,
        access: &AccessBoundary,
        as_of: &str,
    ) -> Result<SourceResolution, HistoryError> {
        let mut ids = BTreeSet::new();
        for reference in &self.ordered {
            let stored = &self.by_ref[reference];
            if stored.record.provenance.recorded_at.as_str() > as_of
                || stored.record.scope.project != project
                || !is_visible(stored, access)
            {
                continue;
            }
            if stored
                .record
                .provenance
                .sources
                .iter()
                .any(|source| source.system == system && source.locator == locator)
            {
                ids.insert(stored.record.id.clone());
            }
        }
        match ids.len() {
            0 => Ok(SourceResolution::Missing),
            1 => {
                let stable_id = ids
                    .into_iter()
                    .next()
                    .ok_or(HistoryError::Invariant("missing sole source ID"))?;
                let lineage = self
                    .ordered
                    .iter()
                    .filter_map(|reference| {
                        let stored = &self.by_ref[reference];
                        (stored.record.id == stable_id
                            && stored.record.provenance.recorded_at.as_str() <= as_of
                            && stored.record.scope.project == project
                            && is_visible(stored, access))
                        .then_some(reference.clone())
                    })
                    .collect();
                Ok(SourceResolution::Exact { stable_id, lineage })
            }
            _ => Ok(SourceResolution::Ambiguous {
                stable_ids: ids.into_iter().collect(),
            }),
        }
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, HistoryError> {
        let records = self
            .ordered
            .iter()
            .map(|reference| self.by_ref[reference].clone())
            .collect();
        serde_json::to_vec(&HistorySnapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            accepted_lean_baseline: ACCEPTED_LEAN_BASELINE.to_string(),
            records,
            records_digest: self.snapshot_digest.clone(),
        })
        .map_err(|error| HistoryError::Serialization(error.to_string()))
    }

    pub fn restore_snapshot(bytes: &[u8]) -> Result<Self, HistoryError> {
        let snapshot: HistorySnapshot = serde_json::from_slice(bytes)
            .map_err(|error| HistoryError::Serialization(error.to_string()))?;
        if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION
            || snapshot.accepted_lean_baseline != ACCEPTED_LEAN_BASELINE
        {
            return Err(HistoryError::UnsupportedSnapshot);
        }
        let index = Self::build(snapshot.records)?;
        if index.snapshot_digest != snapshot.records_digest {
            return Err(HistoryError::SnapshotDigestMismatch);
        }
        Ok(index)
    }

    fn historical_refs(&self, request: &HistoryQuery) -> Result<Vec<RecordRef>, HistoryError> {
        let mut selected = Vec::new();
        for reference in &self.ordered {
            let stored = &self.by_ref[reference];
            if stored.record.provenance.recorded_at.as_str() > request.mode.as_of()
                || stored.record.scope.project != request.project
                || !is_visible(stored, &request.access)
                || !matches_search(stored, request.search.as_deref())?
            {
                continue;
            }
            selected.push(reference.clone());
        }
        Ok(selected)
    }

    fn active_refs(&self, request: &HistoryQuery) -> Result<Vec<RecordRef>, HistoryError> {
        let visible = self.historical_refs(&HistoryQuery {
            search: None,
            ..request.clone()
        })?;
        let mut active = BTreeMap::<RecordId, RecordRef>::new();
        let mut retired = BTreeSet::new();
        let mut superseded = BTreeSet::new();
        for reference in &visible {
            let transition = &self.by_ref[reference].record;
            if let Some(previous) = &transition.supersedes {
                superseded.insert(previous.clone());
            }
            match &transition.body {
                RecordBody::Activation(activation) => {
                    if transition.provenance.authority != ProvenanceAuthority::IndependentlyApproved
                    {
                        return Err(HistoryError::UnauthoritativeTransition(reference.clone()));
                    }
                    let policy = self.validate_activation_chain(reference, request)?;
                    if policy.record.provenance.recorded_at.as_str() > request.mode.as_of() {
                        return Err(HistoryError::FutureReference(activation.policy.clone()));
                    }
                    match active.get(&activation.policy.id) {
                        Some(current) if current.revision >= activation.policy.revision => {}
                        _ => {
                            active.insert(activation.policy.id.clone(), activation.policy.clone());
                        }
                    }
                }
                RecordBody::Retirement(retirement) => {
                    if transition.provenance.authority != ProvenanceAuthority::IndependentlyApproved
                    {
                        return Err(HistoryError::UnauthoritativeTransition(reference.clone()));
                    }
                    let target = self.by_ref.get(&retirement.target).ok_or_else(|| {
                        HistoryError::UnknownActiveReference(retirement.target.clone())
                    })?;
                    authorize(target, &request.project, &request.access)?;
                    retired.insert(retirement.target.clone());
                }
                _ => {}
            }
        }
        let mut result = active
            .into_values()
            .filter(|reference| !retired.contains(reference) && !superseded.contains(reference))
            .collect::<Vec<_>>();
        result.sort_by(|left, right| {
            sort_key(&self.by_ref[left]).cmp(&sort_key(&self.by_ref[right]))
        });
        if let Some(term) = request.search.as_deref() {
            result.retain(|reference| {
                matches_search(&self.by_ref[reference], Some(term)).unwrap_or(false)
            });
        }
        Ok(result)
    }

    fn validate_activation_chain(
        &self,
        activation_ref: &RecordRef,
        request: &HistoryQuery,
    ) -> Result<&HistoryRecord, HistoryError> {
        let activation_record = &self.by_ref[activation_ref].record;
        let RecordBody::Activation(activation) = &activation_record.body else {
            return Err(HistoryError::InvalidActivationEvidence(
                activation_ref.clone(),
            ));
        };
        let proposal_record = self
            .by_ref
            .get(&activation.proposal)
            .ok_or_else(|| HistoryError::UnknownActiveReference(activation.proposal.clone()))?;
        let decision_record = self
            .by_ref
            .get(&activation.acceptance_decision)
            .ok_or_else(|| {
                HistoryError::UnknownActiveReference(activation.acceptance_decision.clone())
            })?;
        let policy_record = self
            .by_ref
            .get(&activation.policy)
            .ok_or_else(|| HistoryError::UnknownActiveReference(activation.policy.clone()))?;
        if proposal_record.record.provenance.recorded_at.as_str() > request.mode.as_of()
            || decision_record.record.provenance.recorded_at.as_str() > request.mode.as_of()
            || policy_record.record.provenance.recorded_at.as_str() > request.mode.as_of()
        {
            return Err(HistoryError::FutureReference(activation_ref.clone()));
        }
        authorize(proposal_record, &request.project, &request.access)?;
        authorize(decision_record, &request.project, &request.access)?;
        authorize(policy_record, &request.project, &request.access)?;
        let (RecordBody::Proposal(proposal), RecordBody::Decision(decision)) =
            (&proposal_record.record.body, &decision_record.record.body)
        else {
            return Err(HistoryError::InvalidActivationEvidence(
                activation_ref.clone(),
            ));
        };
        if proposal.state != ProposalState::Shared
            || proposal.binding.as_ref() != Some(&activation.binding)
            || !proposal.proposed_records.contains(&activation.policy)
            || decision.verdict != DecisionVerdict::Accept
            || decision.proposal != activation.proposal
            || decision.proposal_binding != activation.binding
            || decision_record.record.provenance.authority
                != ProvenanceAuthority::IndependentlyApproved
            || proposal_record.record.scope != activation_record.scope
            || decision_record.record.scope != activation_record.scope
            || policy_record.record.scope != activation_record.scope
        {
            return Err(HistoryError::InvalidActivationEvidence(
                activation_ref.clone(),
            ));
        }
        Ok(policy_record)
    }

    fn render_item(
        &self,
        stored: &HistoryRecord,
        request: &HistoryQuery,
    ) -> Result<HistoryItem, HistoryError> {
        let reference = stored.record.reference().map_err(HistoryError::Domain)?;
        let redacted = stored.visibility == HistoryVisibility::Private
            && request
                .redact_private_before
                .as_deref()
                .is_some_and(|boundary| stored.record.provenance.recorded_at.as_str() < boundary);
        let links = record_links(&stored.record)
            .into_iter()
            .map(|link| match self.by_ref.get(&link) {
                None => LinkState::Missing(link),
                Some(linked) if !is_visible(linked, &request.access) => LinkState::Redacted {
                    reference: link,
                    reason: "outside authorized visibility boundary".into(),
                },
                Some(_) => LinkState::Resolved(link),
            })
            .collect();
        Ok(HistoryItem {
            reference,
            recorded_at: stored.record.provenance.recorded_at.clone(),
            visibility: stored.visibility,
            record: (!redacted).then(|| stored.record.clone()),
            redaction_reason: redacted.then(|| "retention display boundary".into()),
            links,
        })
    }
}

fn validate_query(request: &HistoryQuery) -> Result<(), HistoryError> {
    validate_project_as_of(&request.project, request.mode.as_of())?;
    if request.page_size == 0 || request.page_size > 500 {
        return Err(HistoryError::InvalidQuery);
    }
    if request.search.as_ref().is_some_and(|term| term.len() > 512) {
        return Err(HistoryError::InvalidQuery);
    }
    Ok(())
}

fn validate_project_as_of(project: &str, as_of: &str) -> Result<(), HistoryError> {
    if project.trim().is_empty() || as_of.len() < 20 || !as_of.ends_with('Z') {
        Err(HistoryError::InvalidQuery)
    } else {
        Ok(())
    }
}

fn authorize(
    stored: &HistoryRecord,
    project: &str,
    access: &AccessBoundary,
) -> Result<(), HistoryError> {
    if stored.record.scope.project != project {
        return Err(HistoryError::CrossProjectDenied);
    }
    if !is_visible(stored, access) {
        return Err(HistoryError::UnauthorizedHistory);
    }
    Ok(())
}

fn is_visible(stored: &HistoryRecord, access: &AccessBoundary) -> bool {
    match (stored.visibility, access) {
        (HistoryVisibility::Team, _) => true,
        (HistoryVisibility::Private, AccessBoundary::Team) => false,
        (HistoryVisibility::Private, AccessBoundary::PrivateStore) => true,
        (HistoryVisibility::Private, AccessBoundary::Private { principal }) => {
            stored.record.owner.kind == principal.kind
                && stored.record.owner.stable_id == principal.stable_id
        }
    }
}

fn matches_search(stored: &HistoryRecord, term: Option<&str>) -> Result<bool, HistoryError> {
    let Some(term) = term else { return Ok(true) };
    let haystack = String::from_utf8(
        stored
            .record
            .canonical_json()
            .map_err(HistoryError::Domain)?,
    )
    .map_err(|error| HistoryError::Serialization(error.to_string()))?;
    Ok(haystack.to_lowercase().contains(&term.to_lowercase()))
}

fn sort_key(stored: &HistoryRecord) -> (String, String, u64) {
    (
        stored.record.provenance.recorded_at.clone(),
        stored.record.id.as_str().to_string(),
        stored.record.revision,
    )
}

fn cursor_for(stored: &HistoryRecord) -> HistoryCursor {
    let (recorded_at, record_id, revision) = sort_key(stored);
    HistoryCursor {
        recorded_at,
        record_id,
        revision,
    }
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
        RecordBody::Decision(body) => links.push(body.proposal.clone()),
        RecordBody::Activation(body) => {
            links.push(body.proposal.clone());
            links.push(body.acceptance_decision.clone());
            links.push(body.policy.clone());
        }
        RecordBody::VerificationReceipt(body) => links.extend(body.related_records.clone()),
        RecordBody::ObservationReceipt(body) => {
            links.push(body.metric.clone());
            links.extend(body.related_records.clone());
        }
        RecordBody::Retirement(body) => {
            links.push(body.target.clone());
            links.extend(body.replacement.clone());
        }
        RecordBody::RepairSession(body) => links.extend(body.last_check_receipt.clone()),
        RecordBody::RepairHandoff(body) => links.push(body.session.clone()),
        RecordBody::RepairAuthorityReservation(body) => links.extend(body.session.clone()),
        RecordBody::RepairOperationClaim(body) => links.push(body.session.clone()),
        _ => {}
    }
    links.sort();
    links.dedup();
    links
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    InvalidQuery,
    UnknownReference(RecordRef),
    UnknownActiveReference(RecordRef),
    InvalidActivationEvidence(RecordRef),
    UnauthoritativeTransition(RecordRef),
    AmbiguousReference(RecordRef),
    FutureReference(RecordRef),
    UnauthorizedHistory,
    CrossProjectDenied,
    StaleSnapshot {
        expected: Option<ContentDigest>,
        actual: ContentDigest,
    },
    UnsupportedSnapshot,
    SnapshotDigestMismatch,
    InvalidStoreBoundary,
    Storage(String),
    Invariant(&'static str),
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
        Activation, CoreValue, Decision, DecisionVerdict, Enforcement, EvidenceRef, Guidance,
        MetricDefinition, MetricDirection, PrincipalKind, Proposal, ProposalBinding, Provenance,
        ProvenanceAuthority, ProvenanceKind, Retirement, Scope, Standard, StandardStrength,
        SCHEMA_VERSION_V1,
    };

    fn owner(id: &str) -> PrincipalRef {
        PrincipalRef {
            kind: PrincipalKind::GithubUser,
            stable_id: id.into(),
            display_name: None,
        }
    }

    fn digest(seed: char) -> ContentDigest {
        ContentDigest::new(format!("sha256:{}", seed.to_string().repeat(64))).expect("digest")
    }

    fn scope(project: &str) -> Scope {
        Scope {
            organization: None,
            project: project.into(),
            component: None,
            environment: None,
        }
    }

    fn record(
        id: &str,
        revision: u64,
        at: &str,
        owner: PrincipalRef,
        body: RecordBody,
        supersedes: Option<RecordRef>,
    ) -> AgreementRecord {
        AgreementRecord {
            schema_version: SCHEMA_VERSION_V1,
            id: RecordId::new(id).expect("id"),
            revision,
            scope: scope("whetstone"),
            owner: owner.clone(),
            provenance: Provenance {
                kind: ProvenanceKind::HumanAuthored,
                recorded_by: owner,
                recorded_at: at.into(),
                sources: vec![EvidenceRef {
                    system: "design".into(),
                    locator: format!("source-{id}-{revision}"),
                    digest: Some(digest('d')),
                }],
                authority: ProvenanceAuthority::IndependentlyApproved,
            },
            supersedes,
            idempotency_key: format!("{id}-{revision}"),
            body,
        }
    }

    fn stored(record: AgreementRecord, visibility: HistoryVisibility) -> HistoryRecord {
        HistoryRecord { visibility, record }
    }

    fn guidance(
        id: &str,
        revision: u64,
        at: &str,
        text: &str,
        previous: Option<RecordRef>,
    ) -> AgreementRecord {
        record(
            id,
            revision,
            at,
            owner("101"),
            RecordBody::Guidance(Guidance {
                statement: text.into(),
                rationale: "because".into(),
                examples: vec![],
            }),
            previous,
        )
    }

    fn query(mode: QueryMode) -> HistoryQuery {
        HistoryQuery {
            project: "whetstone".into(),
            mode,
            access: AccessBoundary::Team,
            search: None,
            after: None,
            page_size: 100,
            expected_snapshot: None,
            redact_private_before: None,
        }
    }

    #[test]
    fn as_of_reconstructs_old_definitions_and_rejected_alternatives_without_future_leakage() {
        let first = guidance(
            "guidance.api",
            1,
            "2026-01-01T00:00:00Z",
            "Old guidance",
            None,
        );
        let first_ref = first.reference().expect("ref");
        let second = guidance(
            "guidance.api",
            2,
            "2026-03-01T00:00:00Z",
            "New guidance",
            Some(first_ref),
        );
        let metric = record(
            "metric.latency",
            1,
            "2026-01-02T00:00:00Z",
            owner("101"),
            RecordBody::MetricDefinition(MetricDefinition {
                name: "Latency".into(),
                rationale: "Speed".into(),
                source: EvidenceRef {
                    system: "monitor".into(),
                    locator: "p95".into(),
                    digest: Some(digest('e')),
                },
                cohort: "all".into(),
                window: "7d".into(),
                direction: MetricDirection::Decrease,
                threshold: "100ms".into(),
                freshness_seconds: 60,
                expected_release: None,
            }),
            None,
        );
        let rejected = record(
            "proposal.alt",
            1,
            "2026-01-03T00:00:00Z",
            owner("101"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Declined,
                title: "Rejected approach".into(),
                rationale: "Too risky".into(),
                proposed_records: vec![metric.reference().expect("metric")],
                binding: Some(ProposalBinding {
                    repository_id: 1,
                    payload_digest: digest('f'),
                    base_active_digest: digest('a'),
                    authority_revision: 1,
                    expires_at: "2026-02-01T00:00:00Z".into(),
                }),
            }),
            None,
        );
        let index = HistoryIndex::build(vec![
            stored(second, HistoryVisibility::Team),
            stored(rejected, HistoryVisibility::Team),
            stored(first, HistoryVisibility::Team),
            stored(metric, HistoryVisibility::Team),
        ])
        .expect("index");
        let page = index
            .query(&query(QueryMode::Historical {
                as_of: "2026-01-31T00:00:00Z".into(),
            }))
            .expect("as of");
        assert_eq!(page.items.len(), 3);
        assert!(page.items.iter().any(|item| matches!(item.record.as_ref().map(|r| &r.body), Some(RecordBody::Proposal(body)) if body.state == ProposalState::Declined)));
        assert!(!page.items.iter().any(|item| item.reference.revision == 2));
    }

    #[test]
    fn active_projection_requires_activation_and_excludes_retired_advice() {
        let advice = guidance(
            "guidance.active",
            1,
            "2026-01-01T00:00:00Z",
            "Use safe APIs",
            None,
        );
        let advice_ref = advice.reference().expect("advice");
        let proposal = record(
            "proposal.active",
            1,
            "2026-01-02T00:00:00Z",
            owner("101"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Shared,
                title: "Activate".into(),
                rationale: "Safe".into(),
                proposed_records: vec![advice_ref.clone()],
                binding: Some(ProposalBinding {
                    repository_id: 1,
                    payload_digest: digest('b'),
                    base_active_digest: digest('a'),
                    authority_revision: 1,
                    expires_at: "2027-01-01T00:00:00Z".into(),
                }),
            }),
            None,
        );
        let proposal_ref = proposal.reference().expect("proposal");
        let decision = record(
            "decision.active",
            1,
            "2026-01-03T00:00:00Z",
            owner("202"),
            RecordBody::Decision(Decision {
                proposal: proposal_ref.clone(),
                proposal_binding: match &proposal.body {
                    RecordBody::Proposal(body) => body.binding.clone().expect("binding"),
                    _ => unreachable!(),
                },
                verdict: DecisionVerdict::Accept,
                reviewer: owner("202"),
                rationale: "Approved".into(),
                decided_at: "2026-01-03T00:00:00Z".into(),
            }),
            None,
        );
        let activation = record(
            "activation.active",
            1,
            "2026-01-04T00:00:00Z",
            owner("202"),
            RecordBody::Activation(Activation {
                proposal: proposal_ref,
                acceptance_decision: decision.reference().expect("decision"),
                policy: advice_ref.clone(),
                binding: match &proposal.body {
                    RecordBody::Proposal(body) => body.binding.clone().expect("binding"),
                    _ => unreachable!(),
                },
                activation_sequence: 1,
                activated_at: "2026-01-04T00:00:00Z".into(),
            }),
            None,
        );
        let correction = guidance(
            "guidance.active",
            2,
            "2026-01-15T00:00:00Z",
            "Use safer APIs",
            Some(advice_ref.clone()),
        );
        let retirement = record(
            "retirement.active",
            1,
            "2026-02-01T00:00:00Z",
            owner("202"),
            RecordBody::Retirement(Retirement {
                target: advice_ref,
                reason: "Replaced".into(),
                replacement: None,
            }),
            None,
        );
        let index = HistoryIndex::build(
            vec![
                advice.clone(),
                proposal.clone(),
                decision.clone(),
                activation.clone(),
                correction.clone(),
                retirement.clone(),
            ]
            .into_iter()
            .map(|r| stored(r, HistoryVisibility::Team))
            .collect(),
        )
        .expect("index");
        assert_eq!(
            index
                .query(&query(QueryMode::Active {
                    as_of: "2026-01-10T00:00:00Z".into()
                }))
                .expect("active")
                .items
                .len(),
            1
        );
        assert!(index
            .query(&query(QueryMode::Active {
                as_of: "2026-01-20T00:00:00Z".into()
            }))
            .expect("superseded but not reactivated")
            .items
            .is_empty());
        assert!(index
            .query(&query(QueryMode::Active {
                as_of: "2026-02-20T00:00:00Z".into()
            }))
            .expect("retired")
            .items
            .is_empty());
        assert_eq!(
            index
                .query(&query(QueryMode::Historical {
                    as_of: "2026-02-20T00:00:00Z".into()
                }))
                .expect("history")
                .items
                .len(),
            6
        );

        let mut forged_activation = activation.clone();
        forged_activation.provenance.authority = ProvenanceAuthority::OwnerAuthored;
        let forged_ref = forged_activation.reference().expect("forged activation");
        let forged = HistoryIndex::build(
            vec![
                advice.clone(),
                proposal.clone(),
                decision.clone(),
                forged_activation,
            ]
            .into_iter()
            .map(|record| stored(record, HistoryVisibility::Team))
            .collect(),
        )
        .expect("forged index remains inspectable");
        assert_eq!(
            forged.query(&query(QueryMode::Active {
                as_of: "2026-01-10T00:00:00Z".into()
            })),
            Err(HistoryError::UnauthoritativeTransition(forged_ref))
        );

        let mut declined_decision = decision;
        let RecordBody::Decision(body) = &mut declined_decision.body else {
            unreachable!()
        };
        body.verdict = DecisionVerdict::Decline;
        let mut invalid_activation = activation;
        let RecordBody::Activation(body) = &mut invalid_activation.body else {
            unreachable!()
        };
        body.acceptance_decision = declined_decision.reference().expect("declined decision");
        let invalid_activation_ref = invalid_activation.reference().expect("invalid activation");
        let invalid = HistoryIndex::build(
            vec![advice, proposal, declined_decision, invalid_activation]
                .into_iter()
                .map(|record| stored(record, HistoryVisibility::Team))
                .collect(),
        )
        .expect("invalid chain remains historical evidence");
        assert_eq!(
            invalid.query(&query(QueryMode::Active {
                as_of: "2026-01-10T00:00:00Z".into()
            })),
            Err(HistoryError::InvalidActivationEvidence(
                invalid_activation_ref
            ))
        );
    }

    #[test]
    fn source_lineage_survives_rename_and_content_revision_and_reports_missing_or_ambiguous() {
        let first = guidance("guidance.rename", 1, "2026-01-01T00:00:00Z", "First", None);
        let first_ref = first.reference().expect("first");
        let mut second = guidance(
            "guidance.rename",
            2,
            "2026-02-01T00:00:00Z",
            "Second",
            Some(first_ref),
        );
        second.provenance.sources[0].locator = "renamed-source".into();
        let other = guidance("guidance.other", 1, "2026-01-03T00:00:00Z", "Other", None);
        let mut same_source = other.clone();
        same_source.provenance.sources[0].locator = "renamed-source".into();
        let index = HistoryIndex::build(
            vec![first, second, same_source]
                .into_iter()
                .map(|r| stored(r, HistoryVisibility::Team))
                .collect(),
        )
        .expect("index");
        assert!(
            matches!(index.resolve_source("design", "source-guidance.rename-1", "whetstone", &AccessBoundary::Team, "2026-03-01T00:00:00Z").expect("source"), SourceResolution::Exact { lineage, .. } if lineage.len() == 2)
        );
        assert!(matches!(
            index
                .resolve_source(
                    "design",
                    "renamed-source",
                    "whetstone",
                    &AccessBoundary::Team,
                    "2026-03-01T00:00:00Z"
                )
                .expect("source"),
            SourceResolution::Ambiguous { .. }
        ));
        assert_eq!(
            index
                .resolve_source(
                    "design",
                    "missing",
                    "whetstone",
                    &AccessBoundary::Team,
                    "2026-03-01T00:00:00Z"
                )
                .expect("source"),
            SourceResolution::Missing
        );
    }

    #[test]
    fn pagination_is_deterministic_and_privacy_and_project_boundaries_fail_closed() {
        let private_owner = owner("101");
        let mut records = (0..120)
            .map(|number| {
                stored(
                    guidance(
                        &format!("guidance.item{number:03}"),
                        1,
                        "2026-01-01T00:00:00Z",
                        &format!("Item {number}"),
                        None,
                    ),
                    HistoryVisibility::Team,
                )
            })
            .collect::<Vec<_>>();
        let private = guidance(
            "guidance.private",
            1,
            "2026-01-01T00:00:00Z",
            "PRIVATE-CANARY",
            None,
        );
        let private_ref = private.reference().expect("private");
        records.push(stored(private, HistoryVisibility::Private));
        let index = HistoryIndex::build(records).expect("index");
        let mut request = query(QueryMode::Historical {
            as_of: "2026-02-01T00:00:00Z".into(),
        });
        request.page_size = 25;
        let first = index.query(&request).expect("page 1");
        request.after = first.next.clone();
        let second = index.query(&request).expect("page 2");
        assert_eq!(first.items.len(), 25);
        assert_eq!(second.items.len(), 25);
        assert!(first
            .items
            .iter()
            .chain(&second.items)
            .all(|item| item.reference != private_ref));
        assert_eq!(
            index.trace(
                &private_ref,
                "whetstone",
                &AccessBoundary::Team,
                "2026-02-01T00:00:00Z"
            ),
            Err(HistoryError::UnauthorizedHistory)
        );
        assert_eq!(
            index.trace(
                &private_ref,
                "other",
                &AccessBoundary::Private {
                    principal: private_owner
                },
                "2026-02-01T00:00:00Z"
            ),
            Err(HistoryError::CrossProjectDenied)
        );
    }

    #[test]
    fn team_trace_rejects_private_ancestry_and_stale_queries_are_not_success() {
        let private = guidance(
            "guidance.private-base",
            1,
            "2026-01-01T00:00:00Z",
            "Private",
            None,
        );
        let private_ref = private.reference().expect("private");
        let team = record(
            "proposal.team",
            1,
            "2026-01-02T00:00:00Z",
            owner("101"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Shared,
                title: "Team proposal".into(),
                rationale: "References selected private ancestry".into(),
                proposed_records: vec![private_ref],
                binding: Some(ProposalBinding {
                    repository_id: 1,
                    payload_digest: digest('b'),
                    base_active_digest: digest('a'),
                    authority_revision: 1,
                    expires_at: "2027-01-01T00:00:00Z".into(),
                }),
            }),
            None,
        );
        let team_ref = team.reference().expect("team");
        let index = HistoryIndex::build(vec![
            stored(private, HistoryVisibility::Private),
            stored(team, HistoryVisibility::Team),
        ])
        .expect("index");
        assert_eq!(
            index.trace(
                &team_ref,
                "whetstone",
                &AccessBoundary::Team,
                "2026-02-01T00:00:00Z"
            ),
            Err(HistoryError::UnauthorizedHistory)
        );
        let mut request = query(QueryMode::Historical {
            as_of: "2026-02-01T00:00:00Z".into(),
        });
        request.expected_snapshot = Some(digest('f'));
        assert!(matches!(
            index.query(&request),
            Err(HistoryError::StaleSnapshot { .. })
        ));
    }

    #[test]
    fn logical_backup_restore_preserves_rationale_source_hashes_and_snapshot_digest() {
        let standard = record(
            "standard.backup",
            1,
            "2026-01-01T00:00:00Z",
            owner("101"),
            RecordBody::Standard(Standard {
                statement: "No unsafe paths".into(),
                rationale: "Protect data".into(),
                strength: StandardStrength::Must,
                enforcement: Enforcement::Test {
                    command_ref: "gate:path".into(),
                },
                examples: vec![],
            }),
            None,
        );
        let index =
            HistoryIndex::build(vec![stored(standard, HistoryVisibility::Team)]).expect("index");
        let bytes = index.export_snapshot().expect("export");
        let restored = HistoryIndex::restore_snapshot(&bytes).expect("restore");
        assert_eq!(restored.snapshot_digest(), index.snapshot_digest());
        assert_eq!(restored.export_snapshot().expect("re-export"), bytes);
        assert!(String::from_utf8(bytes)
            .expect("json")
            .contains("Protect data"));
    }

    #[test]
    fn redaction_is_a_view_boundary_and_does_not_delete_history() {
        let private = guidance(
            "guidance.retained",
            1,
            "2026-01-01T00:00:00Z",
            "Sensitive rationale",
            None,
        );
        let index =
            HistoryIndex::build(vec![stored(private, HistoryVisibility::Private)]).expect("index");
        let mut request = query(QueryMode::Historical {
            as_of: "2026-02-01T00:00:00Z".into(),
        });
        request.access = AccessBoundary::Private {
            principal: owner("101"),
        };
        request.redact_private_before = Some("2026-01-15T00:00:00Z".into());
        let page = index.query(&request).expect("redacted view");
        assert!(page.items[0].record.is_none());
        assert_eq!(
            HistoryIndex::restore_snapshot(&index.export_snapshot().expect("backup"))
                .expect("restore")
                .ordered
                .len(),
            1
        );
    }

    #[test]
    fn accepted_or_declined_proposals_never_enter_active_context_without_activation() {
        let value = record(
            "value.safety",
            1,
            "2026-01-01T00:00:00Z",
            owner("101"),
            RecordBody::CoreValue(CoreValue {
                name: "Safety".into(),
                description: "Fail closed".into(),
            }),
            None,
        );
        let proposal = record(
            "proposal.only",
            1,
            "2026-01-02T00:00:00Z",
            owner("101"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Shared,
                title: "Value".into(),
                rationale: "Good".into(),
                proposed_records: vec![value.reference().expect("value")],
                binding: Some(ProposalBinding {
                    repository_id: 1,
                    payload_digest: digest('b'),
                    base_active_digest: digest('a'),
                    authority_revision: 1,
                    expires_at: "2027-01-01T00:00:00Z".into(),
                }),
            }),
            None,
        );
        let index = HistoryIndex::build(vec![
            stored(value, HistoryVisibility::Team),
            stored(proposal, HistoryVisibility::Team),
        ])
        .expect("index");
        assert!(index
            .query(&query(QueryMode::Active {
                as_of: "2026-02-01T00:00:00Z".into()
            }))
            .expect("active")
            .items
            .is_empty());
    }

    #[test]
    fn inspection_service_returns_distinct_active_history_trace_and_source_views() {
        let advice = guidance(
            "guidance.service",
            1,
            "2026-01-01T00:00:00Z",
            "Use bounded safe APIs",
            None,
        );
        let advice_ref = advice.reference().expect("advice ref");
        let binding = ProposalBinding {
            repository_id: 1,
            payload_digest: digest('b'),
            base_active_digest: digest('a'),
            authority_revision: 1,
            expires_at: "2027-01-01T00:00:00Z".into(),
        };
        let proposal = record(
            "proposal.service",
            1,
            "2026-01-02T00:00:00Z",
            owner("101"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Shared,
                title: "Activate service guidance".into(),
                rationale: "Reviewed policy".into(),
                proposed_records: vec![advice_ref.clone()],
                binding: Some(binding.clone()),
            }),
            None,
        );
        let proposal_ref = proposal.reference().expect("proposal ref");
        let decision = record(
            "decision.service",
            1,
            "2026-01-03T00:00:00Z",
            owner("202"),
            RecordBody::Decision(Decision {
                proposal: proposal_ref.clone(),
                proposal_binding: binding.clone(),
                verdict: DecisionVerdict::Accept,
                reviewer: owner("202"),
                rationale: "Independent acceptance".into(),
                decided_at: "2026-01-03T00:00:00Z".into(),
            }),
            None,
        );
        let activation = record(
            "activation.service",
            1,
            "2026-01-04T00:00:00Z",
            owner("202"),
            RecordBody::Activation(Activation {
                proposal: proposal_ref,
                acceptance_decision: decision.reference().expect("decision ref"),
                policy: advice_ref.clone(),
                binding,
                activation_sequence: 1,
                activated_at: "2026-01-04T00:00:00Z".into(),
            }),
            None,
        );
        let activation_ref = activation.reference().expect("activation ref");
        let service = HistoryInspectionService {
            index: HistoryIndex::build(
                [advice, proposal, decision, activation]
                    .into_iter()
                    .map(|record| stored(record, HistoryVisibility::Team))
                    .collect(),
            )
            .expect("index"),
        };
        let inspection = service
            .inspect(&HistoryInspectionRequest {
                project: "whetstone".into(),
                as_of: "2026-02-01T00:00:00Z".into(),
                access: AccessBoundary::Team,
                search: None,
                history_after: None,
                page_size: 100,
                expected_snapshot: Some(service.snapshot_digest().clone()),
                redact_private_before: None,
            })
            .expect("inspect");
        assert_eq!(inspection.active_context.items.len(), 1);
        assert_eq!(inspection.active_context.items[0].reference, advice_ref);
        assert_eq!(inspection.decision_history.items.len(), 4);

        let trace = service
            .trace(&HistoryTraceRequest {
                root: activation_ref,
                project: "whetstone".into(),
                access: AccessBoundary::Team,
                as_of: "2026-02-01T00:00:00Z".into(),
            })
            .expect("trace");
        assert_eq!(trace.len(), 4);
        assert!(matches!(
            service
                .resolve_source(&HistorySourceQuery {
                    system: "design".into(),
                    locator: "source-guidance.service-1".into(),
                    project: "whetstone".into(),
                    access: AccessBoundary::Team,
                    as_of: "2026-02-01T00:00:00Z".into(),
                })
                .expect("source"),
            SourceResolution::Exact { lineage, .. } if lineage.len() == 1
        ));

        let searched = service
            .inspect(&HistoryInspectionRequest {
                project: "whetstone".into(),
                as_of: "2026-02-01T00:00:00Z".into(),
                access: AccessBoundary::Team,
                search: Some("bounded safe APIs".into()),
                history_after: None,
                page_size: 100,
                expected_snapshot: None,
                redact_private_before: None,
            })
            .expect("searched inspection");
        assert_eq!(searched.active_context.items.len(), 1);
        assert_eq!(searched.decision_history.items.len(), 1);
    }

    #[test]
    fn inspection_service_preserves_private_boundary_and_rejects_invalid_as_of() {
        let private_owner = owner("101");
        let private = guidance(
            "guidance.service-private",
            1,
            "2026-01-01T00:00:00Z",
            "SERVICE-PRIVATE-CANARY",
            None,
        );
        let private_ref = private.reference().expect("private ref");
        let service = HistoryInspectionService {
            index: HistoryIndex::build(vec![stored(private, HistoryVisibility::Private)])
                .expect("index"),
        };
        let team = service
            .inspect(&HistoryInspectionRequest {
                project: "whetstone".into(),
                as_of: "2026-02-01T00:00:00Z".into(),
                access: AccessBoundary::Team,
                search: Some("SERVICE-PRIVATE-CANARY".into()),
                history_after: None,
                page_size: 100,
                expected_snapshot: None,
                redact_private_before: None,
            })
            .expect("team inspection");
        assert!(team.decision_history.items.is_empty());
        let private_view = service
            .query(&HistoryQuery {
                project: "whetstone".into(),
                mode: QueryMode::Historical {
                    as_of: "2026-02-01T00:00:00Z".into(),
                },
                access: AccessBoundary::Private {
                    principal: private_owner,
                },
                search: Some("SERVICE-PRIVATE-CANARY".into()),
                after: None,
                page_size: 100,
                expected_snapshot: None,
                redact_private_before: None,
            })
            .expect("owner inspection");
        assert_eq!(private_view.items[0].reference, private_ref);
        assert_eq!(
            service.resolve_source(&HistorySourceQuery {
                system: "design".into(),
                locator: "source-guidance.service-private-1".into(),
                project: "whetstone".into(),
                access: AccessBoundary::Team,
                as_of: "not-a-time".into(),
            }),
            Err(HistoryError::InvalidQuery)
        );
    }
}
