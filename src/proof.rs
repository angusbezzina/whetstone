//! Gate state shared by gates and projections: gate evidence artifacts and
//! the latest receipt per rule.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::agreement::AgreementState;
use crate::domain::{AgreementRecord, RecordBody, RecordId};

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
