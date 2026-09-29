//! Proof state shared by gates, projections and the skill: gate evidence
//! artifacts, the latest receipt per rule, when each feature was last proven
//! and which of its entry points moved since.
//!
//! Receipts bind the commit they ran at (`git_head` evidence), so a proof is
//! stale exactly when a path under the feature's entry points changed after
//! that commit.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{AgreementRecord, Feature, RecordBody, RecordId, VerificationAxis};

/// Receipt subject prefix of one rule's mechanical run: `gate:<rule id>`.
pub const GATE_SUBJECT_PREFIX: &str = "gate:";
/// Evidence system of artifacts written under the private evidence root.
pub const EVIDENCE_SYSTEM: &str = "whetstone_evidence";
/// Evidence system carrying the Git commit a receipt ran at.
pub const GIT_HEAD_SYSTEM: &str = "git_head";

/// The machine-readable artifact every gate run writes next to its log.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct GateArtifact {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub failures: Vec<ArtifactFailure>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct ArtifactFailure {
    pub location: String,
    pub message: String,
}

/// Read one bounded gate artifact from the evidence root.
pub fn read_artifact(evidence_root: &Path, locator: &str) -> Option<GateArtifact> {
    if locator.contains("..") || locator.starts_with('/') {
        return None;
    }
    let path = evidence_root.join(locator);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// Latest per-rule gate receipt, if any.
pub fn latest_gate_receipt<'a>(
    state: &'a AgreementState,
    rule_id: &RecordId,
) -> Option<&'a AgreementRecord> {
    let subject = format!("{GATE_SUBJECT_PREFIX}{}", rule_id.as_str());
    state
        .records()
        .iter()
        .filter(|record| {
            matches!(&record.body, RecordBody::VerificationReceipt(body) if body.subject.stable_id == subject)
        })
        .max_by(|left, right| {
            left.provenance
                .recorded_at
                .cmp(&right.provenance.recorded_at)
                .then_with(|| left.id.cmp(&right.id))
        })
}

/// Ids of every rule record (v2 rules and earlier standards and guidance).
pub fn rule_ids(state: &AgreementState) -> Vec<RecordId> {
    state.agreement_ids(|body| body.rule_view().is_some())
}

/// The Git HEAD recorded by the latest receipt of each rule.
pub fn proof_heads(state: &AgreementState) -> BTreeMap<String, String> {
    let mut heads = BTreeMap::new();
    for id in rule_ids(state) {
        if let Some(RecordBody::VerificationReceipt(body)) =
            latest_gate_receipt(state, &id).map(|record| &record.body)
        {
            if let Some(head) = body
                .evidence
                .iter()
                .find(|evidence| evidence.system == GIT_HEAD_SYSTEM)
            {
                heads.insert(id.as_str().to_string(), head.locator.clone());
            }
        }
    }
    heads
}

/// Paths changed in the working tree, and since each commit a proof ran at.
#[derive(Debug, Clone, Default)]
pub struct Changes {
    pub working: Vec<String>,
    pub since: BTreeMap<String, Vec<String>>,
}

/// Gather the Git changes needed to judge feature drift, one call per commit.
pub fn changes_for(state: &AgreementState, project_root: &Path) -> Changes {
    let mut changes = Changes {
        working: crate::gates::changed_paths(project_root).unwrap_or_default(),
        since: BTreeMap::new(),
    };
    for head in proof_heads(state).into_values() {
        if let std::collections::btree_map::Entry::Vacant(slot) = changes.since.entry(head) {
            let paths = crate::gates::changed_since(project_root, slot.key()).unwrap_or_default();
            slot.insert(paths);
        }
    }
    changes
}

/// When a feature was last proven, and what moved under it since.
#[derive(Debug, Clone, Default, Serialize)]
pub struct FeatureProof {
    /// `{at, commit, gate}` of the latest passing proof, if any.
    pub last_proven: Option<Value>,
    pub drift: Vec<String>,
}

/// Proof state for every in-force feature.
pub fn feature_proofs(
    state: &AgreementState,
    changes: &Changes,
) -> BTreeMap<RecordId, FeatureProof> {
    let heads = proof_heads(state);
    let mut proofs = BTreeMap::new();
    for id in state.agreement_ids(|body| matches!(body, RecordBody::Feature(_))) {
        let Some(RecordBody::Feature(feature)) = state.in_force(&id).map(|record| &record.body)
        else {
            continue;
        };
        let last_proven = proving_rules(state, &id, feature)
            .iter()
            .filter_map(|gate| {
                let receipt = latest_gate_receipt(state, gate)?;
                let RecordBody::VerificationReceipt(body) = &receipt.body else {
                    return None;
                };
                (body.verification == VerificationAxis::Pass).then(|| {
                    json!({
                        "at": body.checked_at,
                        "gate": gate.as_str(),
                        "commit": heads.get(gate.as_str()),
                    })
                })
            })
            .max_by(|left, right| {
                left["at"]
                    .as_str()
                    .unwrap_or_default()
                    .cmp(right["at"].as_str().unwrap_or_default())
            });
        proofs.insert(
            id.clone(),
            FeatureProof {
                last_proven,
                drift: drifted_paths(state, &id, feature, &heads, changes),
            },
        );
    }
    proofs
}

/// The rules that prove a feature: those it lists, plus every drive rule that
/// names it.
pub fn proving_rules(state: &AgreementState, id: &RecordId, feature: &Feature) -> Vec<RecordId> {
    let mut rules = feature.proven_by.clone();
    for rule_id in rule_ids(state) {
        let Some(record) = state.in_force(&rule_id) else {
            continue;
        };
        if let Some(rule) = record.body.rule_view() {
            if matches!(&rule.enforcer, crate::domain::Enforcer::Drive { feature } if feature == id)
                && !rules.contains(&rule_id)
            {
                rules.push(rule_id);
            }
        }
    }
    rules
}

/// Paths under a feature's entry points that changed since its last proof,
/// or in the working tree when it has never been proven.
pub fn drifted_paths(
    state: &AgreementState,
    id: &RecordId,
    feature: &Feature,
    heads: &BTreeMap<String, String>,
    changes: &Changes,
) -> Vec<String> {
    let mut paths = changes
        .working
        .iter()
        .filter(|path| feature.covers_path(path))
        .cloned()
        .collect::<Vec<_>>();
    for gate in proving_rules(state, id, feature) {
        if let Some(committed) = heads
            .get(gate.as_str())
            .and_then(|head| changes.since.get(head))
        {
            paths.extend(
                committed
                    .iter()
                    .filter(|path| feature.covers_path(path))
                    .cloned(),
            );
        }
    }
    paths.sort();
    paths.dedup();
    paths
}
