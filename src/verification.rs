//! Snapshot-bound aggregation of trusted verification evidence.
//!
//! This module never runs a checker, repairs source, changes policy, or grants
//! authority. It converts already-trusted execution/attestation observations
//! into a compact, deterministic receipt and rejects replay against a changed
//! or stale snapshot.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{ContentDigest, RecordRef};
use crate::execution::{ExecutionReceipt, ExecutionState, RECEIPT_SCHEMA as EXECUTION_SCHEMA};

pub const VERIFICATION_SCHEMA: &str = "whetstone.verification-receipt.v1";
pub const LEAN_BASELINE_REVISION: &str = "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48";
const MAX_REQUIREMENTS: usize = 256;
const MAX_FINDINGS: usize = 256;
const MAX_TEXT_BYTES: usize = 2_048;
const MAX_CODE_FILES: usize = 4_096;
const MAX_CODE_BYTES: u64 = 64 * 1024 * 1024;
const SNAPSHOT_SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".venv",
    "venv",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotBinding {
    /// Content digest of relevant paths, modes, and bytes, not merely Git HEAD.
    pub code_tree: ContentDigest,
    pub policy: ContentDigest,
    pub checker_bundle: ContentDigest,
    pub scope: ContentDigest,
    pub environment: ContentDigest,
    pub trust: ContentDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementKind {
    NativeCheck,
    TaskAcceptance,
    RequiredReview,
    StageSafeguard,
    AdvisoryGuidance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRequirement {
    pub id: String,
    pub kind: RequirementKind,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checker_manifest: Option<ContentDigest>,
    #[serde(default)]
    pub governing_records: Vec<RecordRef>,
    pub rationale: String,
    pub repair_direction: String,
    pub permitted_next_action: String,
    pub verification_command: String,
    pub freshness_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationPlan {
    pub subject: String,
    pub snapshot: SnapshotBinding,
    pub requirements: Vec<VerificationRequirement>,
}

/// Compact evidence reference. Raw credentials and checker output are never
/// retained by this aggregation layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidencePointer {
    pub source: String,
    pub locator: String,
    pub digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    pub rule_id: String,
    pub observed: String,
    pub expected: String,
    pub rationale: String,
    pub repair_direction: String,
    pub permitted_next_action: String,
    pub verification_command: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationState {
    Success,
    Violated,
    Unknown,
    Unavailable,
}

/// Authenticated evidence returned by a trusted task/review adapter.
///
/// It intentionally has no `Deserialize` implementation: untrusted JSON cannot
/// assert reviewer identity, acceptance, or trust. Its fields and the aggregate
/// entrypoint are crate-private, so browser/plugin input must first pass a
/// trusted same-process adapter. Adapter implementations are part of
/// Whetstone's trust boundary, like execution authority adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifiedAttestation {
    pub(crate) requirement_id: String,
    pub(crate) snapshot: SnapshotBinding,
    pub(crate) state: AttestationState,
    pub(crate) evidence: EvidencePointer,
    pub(crate) observed_at_unix: u64,
    pub(crate) summary: String,
    pub(crate) findings: Vec<Finding>,
}

/// Same-process evidence accepted by the aggregator.
///
/// This type is deliberately crate-private and non-deserializable. In
/// particular, a restored public `ExecutionReceipt` is data, not trusted input;
/// only the in-process execution adapter may wrap it here after it has checked
/// the execution grant and exact manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
// These variants are constructed by the crate's execution, acceptance, and
// review adapters when `wh check` is wired to this module. Keeping the type
// crate-private is the security property; the temporary allowance disappears
// once those adapters are connected.
#[allow(dead_code)]
pub(crate) enum VerificationEvidence {
    /// Evidence produced directly by a deterministic checker compiled into
    /// this process. It is distinct from `Execution`: no external process ran
    /// and therefore no execution grant or executable receipt is invented.
    NativeCheck {
        requirement_id: String,
        snapshot: SnapshotBinding,
        state: AttestationState,
        evidence: EvidencePointer,
        observed_at_unix: u64,
        summary: String,
        findings: Vec<Finding>,
    },
    Execution {
        requirement_id: String,
        snapshot: SnapshotBinding,
        receipt: Box<ExecutionReceipt>,
        evidence: EvidencePointer,
        observed_at_unix: u64,
        findings: Vec<Finding>,
    },
    Attestation(VerifiedAttestation),
    Skipped {
        requirement_id: String,
        reason: String,
    },
    Advisory {
        requirement_id: String,
        evidence: EvidencePointer,
        summary: String,
    },
}

/// Opaque evidence batch produced only by trusted in-process adapters.
///
/// External callers can neither deserialize nor populate this value. This is
/// intentional: a public/restored `ExecutionReceipt` is inspectable data until
/// the trusted execution path wraps it in this crate-private collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedEvidenceSet {
    pub(crate) items: Vec<VerificationEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    Success,
    Violated,
    Unknown,
    Unavailable,
    NeedsExecutionApproval,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Success,
    Violated,
    Unknown,
    Unavailable,
    NeedsExecutionApproval,
    Stale,
    Skipped,
    Advisory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceFreshness {
    Fresh,
    Stale,
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub requirement_id: String,
    pub kind: RequirementKind,
    pub required: bool,
    pub state: CheckState,
    pub freshness: EvidenceFreshness,
    pub reason_code: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidencePointer>,
    #[serde(default)]
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub governing_records: Vec<RecordRef>,
    pub repair_direction: String,
    pub permitted_next_action: String,
    pub verification_command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_unix: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until_unix: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    pub schema: String,
    pub receipt_id: ContentDigest,
    pub lean_baseline_revision: String,
    pub subject: String,
    pub snapshot: SnapshotBinding,
    pub evaluated_at_unix: u64,
    pub state: VerificationState,
    pub summary: String,
    pub results: Vec<CheckResult>,
    pub next_actions: Vec<String>,
}

impl VerificationReport {
    pub fn human_summary(&self) -> String {
        let mut lines = vec![format!(
            "{}: {} ({})",
            self.subject,
            self.summary,
            state_name(self.state)
        )];
        for result in &self.results {
            if result.state == CheckState::Success {
                lines.push(format!(
                    "{} {}: {}",
                    check_state_name(result.state),
                    result.requirement_id,
                    result.summary
                ));
            } else {
                lines.push(format!(
                    "{} {}: {} — next: {}",
                    check_state_name(result.state),
                    result.requirement_id,
                    result.summary,
                    result.permitted_next_action
                ));
            }
        }
        lines.join("\n")
    }
}

pub fn aggregate(
    plan: &VerificationPlan,
    evidence: &TrustedEvidenceSet,
    evaluated_at_unix: u64,
    redactions: &[String],
) -> Result<VerificationReport, VerificationError> {
    validate_plan(plan)?;
    let mut indexed: BTreeMap<&str, Vec<&VerificationEvidence>> = BTreeMap::new();
    for item in &evidence.items {
        let id = evidence_id(item);
        indexed.entry(id).or_default().push(item);
    }
    let known: BTreeSet<&str> = plan
        .requirements
        .iter()
        .map(|requirement| requirement.id.as_str())
        .collect();
    if let Some(unknown) = indexed.keys().find(|id| !known.contains(**id)) {
        return Err(VerificationError::UnknownRequirement((*unknown).into()));
    }

    let mut requirements = plan.requirements.clone();
    requirements.sort_by(|left, right| left.id.cmp(&right.id));
    let mut results = Vec::with_capacity(requirements.len());
    for requirement in requirements {
        let observations = indexed
            .get(requirement.id.as_str())
            .cloned()
            .unwrap_or_default();
        let result = if observations.len() > 1 {
            failed_result(
                &requirement,
                CheckState::Unknown,
                EvidenceFreshness::Missing,
                "duplicate_evidence",
                "Multiple observations claim the same requirement; evidence is ambiguous.",
            )
        } else if let Some(observation) = observations.first() {
            resolve_observation(
                &requirement,
                &plan.snapshot,
                observation,
                evaluated_at_unix,
                redactions,
            )?
        } else if requirement.kind == RequirementKind::AdvisoryGuidance {
            failed_result(
                &requirement,
                CheckState::Advisory,
                EvidenceFreshness::Missing,
                "advisory_not_observed",
                "Advisory guidance has no current observation and does not affect completion.",
            )
        } else {
            failed_result(
                &requirement,
                CheckState::Unknown,
                EvidenceFreshness::Missing,
                "required_evidence_missing",
                "Required evidence is missing; a successful subset is not completion.",
            )
        };
        results.push(result);
    }

    let state = aggregate_state(&results);
    let summary = match state {
        VerificationState::Success => {
            "All applicable required checks, acceptance, and reviews are fresh and successful."
        }
        VerificationState::Violated => {
            "One or more required checks or attestations reported a violation."
        }
        VerificationState::NeedsExecutionApproval => {
            "A required checker is not approved for this exact execution snapshot."
        }
        VerificationState::Unavailable => "A required checker or authority source is unavailable.",
        VerificationState::Unknown => {
            "Required evidence is missing, stale, skipped, ambiguous, or untrusted."
        }
    };
    let mut next_actions: Vec<String> = results
        .iter()
        .filter(|result| result.required && result.state != CheckState::Success)
        .map(|result| result.permitted_next_action.clone())
        .collect();
    next_actions.sort();
    next_actions.dedup();
    let mut report = VerificationReport {
        schema: VERIFICATION_SCHEMA.into(),
        receipt_id: empty_digest()?,
        lean_baseline_revision: LEAN_BASELINE_REVISION.into(),
        subject: bounded(&plan.subject, redactions),
        snapshot: plan.snapshot.clone(),
        evaluated_at_unix,
        state,
        summary: summary.into(),
        results,
        next_actions,
    };
    report.receipt_id = report_digest(&report)?;
    Ok(report)
}

pub fn replay(
    report: &VerificationReport,
    current_snapshot: &SnapshotBinding,
    now_unix: u64,
) -> Result<VerificationState, VerificationError> {
    if report.schema != VERIFICATION_SCHEMA
        || report.lean_baseline_revision != LEAN_BASELINE_REVISION
    {
        return Err(VerificationError::UnsupportedSchema);
    }
    if report_digest(report)? != report.receipt_id {
        return Err(VerificationError::ReceiptTampered);
    }
    if &report.snapshot != current_snapshot {
        return Ok(VerificationState::Unknown);
    }
    if report
        .results
        .iter()
        .filter(|result| result.required)
        .any(|result| match result.valid_until_unix {
            Some(until) => now_unix > until,
            None => true,
        })
    {
        return Ok(VerificationState::Unknown);
    }
    Ok(report.state)
}

/// Hash relevant repository paths including uncommitted file bytes. Symlinks
/// and paths outside the canonical project root are rejected.
pub fn code_tree_digest(
    project_root: &Path,
    relevant_paths: &[PathBuf],
) -> Result<ContentDigest, VerificationError> {
    let root = project_root
        .canonicalize()
        .map_err(|error| VerificationError::Io(error.to_string()))?;
    if !root.is_dir() || relevant_paths.is_empty() {
        return Err(VerificationError::InvalidPlan(
            "code snapshot requires a project directory and at least one path".into(),
        ));
    }
    let mut files = Vec::new();
    for relative in relevant_paths {
        validate_relative_path(relative)?;
        let path = root.join(relative);
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| VerificationError::Io(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(VerificationError::UnsafePath(relative.clone()));
        }
        let canonical = path
            .canonicalize()
            .map_err(|error| VerificationError::Io(error.to_string()))?;
        if !canonical.starts_with(&root) {
            return Err(VerificationError::UnsafePath(relative.clone()));
        }
        if metadata.is_file() {
            files.push(canonical);
        } else if metadata.is_dir() {
            for entry in walkdir::WalkDir::new(&canonical)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    entry.depth() == 0
                        || !SNAPSHOT_SKIP_DIRS
                            .iter()
                            .any(|name| entry.file_name() == *name)
                })
            {
                let entry = entry.map_err(|error| VerificationError::Io(error.to_string()))?;
                let metadata = std::fs::symlink_metadata(entry.path())
                    .map_err(|error| VerificationError::Io(error.to_string()))?;
                if metadata.file_type().is_symlink() {
                    return Err(VerificationError::UnsafePath(entry.path().to_path_buf()));
                }
                if metadata.is_file() {
                    files.push(entry.path().to_path_buf());
                }
            }
        } else {
            return Err(VerificationError::UnsafePath(relative.clone()));
        }
    }
    files.sort();
    files.dedup();
    let bytes = files.iter().try_fold(0_u64, |total, file| {
        let size = file
            .metadata()
            .map_err(|error| VerificationError::Io(error.to_string()))?
            .len();
        total
            .checked_add(size)
            .ok_or_else(|| VerificationError::InvalidPlan("code snapshot is too large".into()))
    })?;
    if files.len() > MAX_CODE_FILES || bytes > MAX_CODE_BYTES {
        return Err(VerificationError::InvalidPlan(
            "code snapshot exceeds file or byte bounds".into(),
        ));
    }
    let mut hash = Sha256::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .map_err(|_| VerificationError::UnsafePath(file.clone()))?;
        let name = relative.to_string_lossy();
        hash.update((name.len() as u64).to_be_bytes());
        hash.update(name.as_bytes());
        let metadata = file
            .metadata()
            .map_err(|error| VerificationError::Io(error.to_string()))?;
        hash.update(metadata.len().to_be_bytes());
        let mut reader =
            File::open(&file).map_err(|error| VerificationError::Io(error.to_string()))?;
        let mut chunk = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut chunk)
                .map_err(|error| VerificationError::Io(error.to_string()))?;
            if read == 0 {
                break;
            }
            hash.update(&chunk[..read]);
        }
    }
    ContentDigest::new(format!("sha256:{:x}", hash.finalize())).map_err(VerificationError::Domain)
}

fn resolve_observation(
    requirement: &VerificationRequirement,
    expected: &SnapshotBinding,
    evidence: &VerificationEvidence,
    evaluated_at_unix: u64,
    redactions: &[String],
) -> Result<CheckResult, VerificationError> {
    match evidence {
        VerificationEvidence::NativeCheck {
            snapshot,
            state,
            evidence,
            observed_at_unix,
            summary,
            findings,
            ..
        } => {
            if requirement.kind != RequirementKind::NativeCheck {
                return Ok(failed_result(
                    requirement,
                    CheckState::Unknown,
                    EvidenceFreshness::Missing,
                    "evidence_kind_mismatch",
                    "Compiled-in native check evidence cannot substitute for acceptance, review, safeguards, or advisory guidance.",
                ));
            }
            let (state, reason) = match state {
                AttestationState::Success => (CheckState::Success, "check_succeeded"),
                AttestationState::Violated => (CheckState::Violated, "check_violated"),
                AttestationState::Unknown => (CheckState::Unknown, "checker_unknown"),
                AttestationState::Unavailable => (CheckState::Unavailable, "checker_unavailable"),
            };
            observed_result(
                requirement,
                expected,
                snapshot,
                state,
                reason,
                summary,
                evidence,
                *observed_at_unix,
                findings,
                evaluated_at_unix,
                redactions,
            )
        }
        VerificationEvidence::Skipped { reason, .. } => Ok(failed_result(
            requirement,
            CheckState::Skipped,
            EvidenceFreshness::Missing,
            "required_check_skipped",
            &bounded(reason, redactions),
        )),
        VerificationEvidence::Advisory {
            evidence, summary, ..
        } => {
            if requirement.kind != RequirementKind::AdvisoryGuidance {
                return Ok(failed_result(
                    requirement,
                    CheckState::Unknown,
                    EvidenceFreshness::Missing,
                    "evidence_kind_mismatch",
                    "Advisory evidence cannot satisfy a required check, acceptance, review, or safeguard.",
                ));
            }
            let mut result = failed_result(
                requirement,
                CheckState::Advisory,
                EvidenceFreshness::Fresh,
                "advisory",
                &bounded(summary, redactions),
            );
            result.evidence = Some(sanitize_pointer(evidence, redactions)?);
            Ok(result)
        }
        VerificationEvidence::Attestation(attestation) => {
            if !matches!(
                requirement.kind,
                RequirementKind::TaskAcceptance
                    | RequirementKind::RequiredReview
                    | RequirementKind::StageSafeguard
            ) {
                return Ok(failed_result(
                    requirement,
                    CheckState::Unknown,
                    EvidenceFreshness::Missing,
                    "evidence_kind_mismatch",
                    "An authenticated attestation cannot substitute for native checker execution or advisory guidance.",
                ));
            }
            let (state, reason) = match attestation.state {
                AttestationState::Success => (CheckState::Success, "attestation_succeeded"),
                AttestationState::Violated => (CheckState::Violated, "attestation_violated"),
                AttestationState::Unknown => (CheckState::Unknown, "attestation_unknown"),
                AttestationState::Unavailable => {
                    (CheckState::Unavailable, "attestation_unavailable")
                }
            };
            observed_result(
                requirement,
                expected,
                &attestation.snapshot,
                state,
                reason,
                &attestation.summary,
                &attestation.evidence,
                attestation.observed_at_unix,
                &attestation.findings,
                evaluated_at_unix,
                redactions,
            )
        }
        VerificationEvidence::Execution {
            snapshot,
            receipt,
            evidence,
            observed_at_unix,
            findings,
            ..
        } => {
            if requirement.kind != RequirementKind::NativeCheck {
                return Ok(failed_result(
                    requirement,
                    CheckState::Unknown,
                    EvidenceFreshness::Missing,
                    "evidence_kind_mismatch",
                    "Native execution evidence cannot substitute for task acceptance, required review, stage safeguards, or advisory guidance.",
                ));
            }
            let expected_manifest = requirement.checker_manifest.as_ref();
            let manifest_matches =
                expected_manifest.is_some_and(|digest| digest.as_str() == receipt.manifest_digest);
            let trusted_success = receipt.schema == EXECUTION_SCHEMA
                && receipt.process_started
                && receipt.execution_grant_reference.is_some()
                && receipt.authority_revision.is_some()
                && receipt.authority_checked_at.is_some()
                && receipt.executable_digest.is_some()
                && manifest_matches;
            let (state, reason, summary) = match receipt.state {
                ExecutionState::Success if trusted_success => {
                    (CheckState::Success, "check_succeeded", receipt.summary.as_str())
                }
                ExecutionState::Violated if trusted_success => {
                    (CheckState::Violated, "check_violated", receipt.summary.as_str())
                }
                ExecutionState::NeedsExecutionApproval => (
                    CheckState::NeedsExecutionApproval,
                    "execution_approval_required",
                    receipt.summary.as_str(),
                ),
                ExecutionState::Unavailable => (
                    CheckState::Unavailable,
                    "checker_unavailable",
                    receipt.summary.as_str(),
                ),
                ExecutionState::Unknown => {
                    (CheckState::Unknown, "checker_unknown", receipt.summary.as_str())
                }
                _ => (
                    CheckState::Unknown,
                    "untrusted_execution_receipt",
                    "Execution claimed a result without complete trusted grant, manifest, executable, and process evidence.",
                ),
            };
            observed_result(
                requirement,
                expected,
                snapshot,
                state,
                reason,
                summary,
                evidence,
                *observed_at_unix,
                findings,
                evaluated_at_unix,
                redactions,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn observed_result(
    requirement: &VerificationRequirement,
    expected_snapshot: &SnapshotBinding,
    observed_snapshot: &SnapshotBinding,
    mut state: CheckState,
    mut reason: &str,
    summary: &str,
    evidence: &EvidencePointer,
    observed_at_unix: u64,
    findings: &[Finding],
    evaluated_at_unix: u64,
    redactions: &[String],
) -> Result<CheckResult, VerificationError> {
    let valid_until = observed_at_unix.saturating_add(requirement.freshness_seconds);
    let mut freshness = EvidenceFreshness::Fresh;
    if observed_snapshot != expected_snapshot {
        state = CheckState::Stale;
        freshness = EvidenceFreshness::Stale;
        reason = "snapshot_mismatch";
    } else if evaluated_at_unix > valid_until {
        state = CheckState::Stale;
        freshness = EvidenceFreshness::Stale;
        reason = "evidence_expired";
    }
    let mut findings = findings
        .iter()
        .take(MAX_FINDINGS)
        .map(|finding| sanitize_finding(finding, redactions))
        .collect::<Result<Vec<_>, _>>()?;
    findings.sort_by(|left, right| {
        (&left.file, left.line, left.column, &left.rule_id).cmp(&(
            &right.file,
            right.line,
            right.column,
            &right.rule_id,
        ))
    });
    Ok(CheckResult {
        requirement_id: requirement.id.clone(),
        kind: requirement.kind,
        required: requirement.required,
        state,
        freshness,
        reason_code: reason.into(),
        summary: bounded(summary, redactions),
        evidence: Some(sanitize_pointer(evidence, redactions)?),
        findings,
        governing_records: sorted_refs(&requirement.governing_records),
        repair_direction: bounded(&requirement.repair_direction, redactions),
        permitted_next_action: bounded(&requirement.permitted_next_action, redactions),
        verification_command: bounded(&requirement.verification_command, redactions),
        observed_at_unix: Some(observed_at_unix),
        valid_until_unix: Some(valid_until),
    })
}

fn failed_result(
    requirement: &VerificationRequirement,
    state: CheckState,
    freshness: EvidenceFreshness,
    reason: &str,
    summary: &str,
) -> CheckResult {
    CheckResult {
        requirement_id: requirement.id.clone(),
        kind: requirement.kind,
        required: requirement.required,
        state,
        freshness,
        reason_code: reason.into(),
        summary: summary.into(),
        evidence: None,
        findings: Vec::new(),
        governing_records: sorted_refs(&requirement.governing_records),
        repair_direction: requirement.repair_direction.clone(),
        permitted_next_action: requirement.permitted_next_action.clone(),
        verification_command: requirement.verification_command.clone(),
        observed_at_unix: None,
        valid_until_unix: None,
    }
}

fn aggregate_state(results: &[CheckResult]) -> VerificationState {
    let required: Vec<&CheckResult> = results.iter().filter(|result| result.required).collect();
    if required
        .iter()
        .any(|result| result.state == CheckState::Violated)
    {
        VerificationState::Violated
    } else if required
        .iter()
        .any(|result| result.state == CheckState::NeedsExecutionApproval)
    {
        VerificationState::NeedsExecutionApproval
    } else if required
        .iter()
        .any(|result| result.state == CheckState::Unavailable)
    {
        VerificationState::Unavailable
    } else if required.is_empty()
        || required
            .iter()
            .any(|result| result.state != CheckState::Success)
    {
        VerificationState::Unknown
    } else {
        VerificationState::Success
    }
}

fn validate_plan(plan: &VerificationPlan) -> Result<(), VerificationError> {
    if plan.subject.trim().is_empty()
        || plan.requirements.is_empty()
        || plan.requirements.len() > MAX_REQUIREMENTS
    {
        return Err(VerificationError::InvalidPlan(
            "subject and one to 256 requirements are required".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    for requirement in &plan.requirements {
        if requirement.id.trim().is_empty()
            || !ids.insert(requirement.id.clone())
            || requirement.freshness_seconds == 0
            || requirement.rationale.trim().is_empty()
            || requirement.permitted_next_action.trim().is_empty()
            || requirement.verification_command.trim().is_empty()
        {
            return Err(VerificationError::InvalidPlan(format!(
                "invalid or duplicate requirement {}",
                requirement.id
            )));
        }
        if requirement.kind == RequirementKind::NativeCheck
            && requirement.checker_manifest.is_none()
        {
            return Err(VerificationError::InvalidPlan(format!(
                "native check {} lacks a checker manifest digest",
                requirement.id
            )));
        }
        if requirement.kind == RequirementKind::AdvisoryGuidance && requirement.required {
            return Err(VerificationError::InvalidPlan(format!(
                "advisory {} cannot be required",
                requirement.id
            )));
        }
    }
    if !plan
        .requirements
        .iter()
        .any(|requirement| requirement.required)
    {
        return Err(VerificationError::InvalidPlan(
            "at least one required verification is necessary".into(),
        ));
    }
    Ok(())
}

fn evidence_id(evidence: &VerificationEvidence) -> &str {
    match evidence {
        VerificationEvidence::NativeCheck { requirement_id, .. }
        | VerificationEvidence::Execution { requirement_id, .. }
        | VerificationEvidence::Skipped { requirement_id, .. }
        | VerificationEvidence::Advisory { requirement_id, .. } => requirement_id,
        VerificationEvidence::Attestation(attestation) => &attestation.requirement_id,
    }
}

fn sanitize_pointer(
    pointer: &EvidencePointer,
    redactions: &[String],
) -> Result<EvidencePointer, VerificationError> {
    if pointer.source.trim().is_empty() || pointer.locator.trim().is_empty() {
        return Err(VerificationError::InvalidEvidence(
            "evidence source and locator are required".into(),
        ));
    }
    Ok(EvidencePointer {
        source: bounded(&pointer.source, redactions),
        locator: bounded(&pointer.locator, redactions),
        digest: pointer.digest.clone(),
    })
}

fn sanitize_finding(
    finding: &Finding,
    redactions: &[String],
) -> Result<Finding, VerificationError> {
    if finding.rule_id.trim().is_empty() {
        return Err(VerificationError::InvalidEvidence(
            "finding rule ID is required".into(),
        ));
    }
    let file = finding
        .file
        .as_ref()
        .map(|file| {
            let path = PathBuf::from(file);
            validate_relative_path(&path)?;
            Ok::<String, VerificationError>(bounded(file, redactions))
        })
        .transpose()?;
    Ok(Finding {
        file,
        line: finding.line,
        column: finding.column,
        rule_id: bounded(&finding.rule_id, redactions),
        observed: bounded(&finding.observed, redactions),
        expected: bounded(&finding.expected, redactions),
        rationale: bounded(&finding.rationale, redactions),
        repair_direction: bounded(&finding.repair_direction, redactions),
        permitted_next_action: bounded(&finding.permitted_next_action, redactions),
        verification_command: bounded(&finding.verification_command, redactions),
    })
}

fn bounded(value: &str, redactions: &[String]) -> String {
    let mut result = value.to_string();
    for secret in redactions.iter().filter(|secret| !secret.is_empty()) {
        result = result.replace(secret, "[REDACTED]");
    }
    if result.len() <= MAX_TEXT_BYTES {
        return result;
    }
    let mut boundary = MAX_TEXT_BYTES;
    while !result.is_char_boundary(boundary) {
        boundary -= 1;
    }
    result.truncate(boundary);
    result.push_str("...[TRUNCATED]");
    result
}

fn report_digest(report: &VerificationReport) -> Result<ContentDigest, VerificationError> {
    let mut canonical = report.clone();
    canonical.receipt_id = empty_digest()?;
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|error| VerificationError::Serialization(error.to_string()))?;
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes)))
        .map_err(VerificationError::Domain)
}

fn empty_digest() -> Result<ContentDigest, VerificationError> {
    ContentDigest::new(format!("sha256:{}", "0".repeat(64))).map_err(VerificationError::Domain)
}

fn sorted_refs(references: &[RecordRef]) -> Vec<RecordRef> {
    let mut references = references.to_vec();
    references.sort();
    references.dedup();
    references
}

fn validate_relative_path(path: &Path) -> Result<(), VerificationError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        Err(VerificationError::UnsafePath(path.to_path_buf()))
    } else {
        Ok(())
    }
}

fn state_name(state: VerificationState) -> &'static str {
    match state {
        VerificationState::Success => "success",
        VerificationState::Violated => "violated",
        VerificationState::Unknown => "unknown",
        VerificationState::Unavailable => "unavailable",
        VerificationState::NeedsExecutionApproval => "needs_execution_approval",
    }
}

fn check_state_name(state: CheckState) -> &'static str {
    match state {
        CheckState::Success => "PASS",
        CheckState::Violated => "FAIL",
        CheckState::Unknown => "UNKNOWN",
        CheckState::Unavailable => "UNAVAILABLE",
        CheckState::NeedsExecutionApproval => "NEEDS_APPROVAL",
        CheckState::Stale => "STALE",
        CheckState::Skipped => "SKIPPED",
        CheckState::Advisory => "ADVISORY",
    }
}

#[derive(Debug)]
pub enum VerificationError {
    InvalidPlan(String),
    UnknownRequirement(String),
    InvalidEvidence(String),
    UnsafePath(PathBuf),
    UnsupportedSchema,
    ReceiptTampered,
    Serialization(String),
    Io(String),
    Domain(crate::domain::DomainError),
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPlan(reason) => write!(formatter, "invalid verification plan: {reason}"),
            Self::UnknownRequirement(id) => write!(formatter, "unknown requirement: {id}"),
            Self::InvalidEvidence(reason) => write!(formatter, "invalid evidence: {reason}"),
            Self::UnsafePath(path) => {
                write!(formatter, "unsafe code or finding path: {}", path.display())
            }
            Self::UnsupportedSchema => {
                formatter.write_str("unsupported verification receipt schema or baseline")
            }
            Self::ReceiptTampered => {
                formatter.write_str("verification receipt digest does not match its content")
            }
            Self::Serialization(reason) => {
                write!(formatter, "verification serialization failed: {reason}")
            }
            Self::Io(reason) => write!(formatter, "verification snapshot I/O failed: {reason}"),
            Self::Domain(error) => write!(formatter, "verification domain error: {error}"),
        }
    }
}

impl std::error::Error for VerificationError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::execution::{CapturedOutput, LimitEvidence};
    use tempfile::TempDir;

    fn aggregate(
        plan: &VerificationPlan,
        evidence: &[VerificationEvidence],
        evaluated_at_unix: u64,
        redactions: &[String],
    ) -> Result<VerificationReport, VerificationError> {
        super::aggregate(
            plan,
            &TrustedEvidenceSet {
                items: evidence.to_vec(),
            },
            evaluated_at_unix,
            redactions,
        )
    }

    fn digest(character: char) -> ContentDigest {
        ContentDigest::new(format!("sha256:{}", character.to_string().repeat(64)))
            .expect("test digest")
    }

    fn snapshot() -> SnapshotBinding {
        SnapshotBinding {
            code_tree: digest('1'),
            policy: digest('2'),
            checker_bundle: digest('3'),
            scope: digest('4'),
            environment: digest('5'),
            trust: digest('6'),
        }
    }

    fn requirement(id: &str, kind: RequirementKind, required: bool) -> VerificationRequirement {
        VerificationRequirement {
            id: id.into(),
            kind,
            required,
            checker_manifest: (kind == RequirementKind::NativeCheck).then(|| digest('a')),
            governing_records: Vec::new(),
            rationale: "required by the accepted project agreement".into(),
            repair_direction: "repair the reported condition without changing policy or tests"
                .into(),
            permitted_next_action: format!("resolve {id}, then recheck"),
            verification_command: "wh check --json".into(),
            freshness_seconds: 60,
        }
    }

    fn plan() -> VerificationPlan {
        VerificationPlan {
            subject: "task:T-042/revision:4".into(),
            snapshot: snapshot(),
            requirements: vec![
                requirement("native", RequirementKind::NativeCheck, true),
                requirement("acceptance", RequirementKind::TaskAcceptance, true),
                requirement("review", RequirementKind::RequiredReview, true),
                requirement("advice", RequirementKind::AdvisoryGuidance, false),
            ],
        }
    }

    fn pointer(id: &str) -> EvidencePointer {
        EvidencePointer {
            source: "trusted-adapter".into(),
            locator: format!("receipt:{id}"),
            digest: digest('b'),
        }
    }

    fn execution(state: ExecutionState, trusted: bool) -> ExecutionReceipt {
        ExecutionReceipt {
            schema: EXECUTION_SCHEMA.into(),
            checker_id: "native".into(),
            checker_version: "1".into(),
            manifest_digest: digest('a').as_str().into(),
            execution_grant_reference: trusted.then(|| "grant:1".into()),
            authority_revision: trusted.then_some(1),
            authority_checked_at: trusted.then(|| "2026-09-09T00:00:00Z".into()),
            state,
            reason_code: "fixture".into(),
            summary: "native checker observation".into(),
            process_started: trusted,
            executable_digest: trusted.then(|| digest('c').as_str().into()),
            consumed_configurations: Vec::new(),
            exit_code: trusted.then_some(0),
            elapsed_ms: 1,
            stdout: CapturedOutput::default(),
            stderr: CapturedOutput::default(),
            limits: LimitEvidence {
                timeout_ms: 100,
                stdout_bytes: 1_024,
                stderr_bytes: 1_024,
                timeout_mechanism: "wall-clock".into(),
                output_mechanism: "bounded".into(),
                filesystem_network_memory_process_limits: "adapter caveat".into(),
            },
            process_group_cleanup: "complete".into(),
            isolation_caveat: "not an OS boundary".into(),
        }
    }

    fn native(state: ExecutionState, trusted: bool) -> VerificationEvidence {
        VerificationEvidence::Execution {
            requirement_id: "native".into(),
            snapshot: snapshot(),
            receipt: Box::new(execution(state, trusted)),
            evidence: pointer("native"),
            observed_at_unix: 100,
            findings: Vec::new(),
        }
    }

    fn compiled_native(state: AttestationState) -> VerificationEvidence {
        VerificationEvidence::NativeCheck {
            requirement_id: "native".into(),
            snapshot: snapshot(),
            state,
            evidence: pointer("compiled-native"),
            observed_at_unix: 100,
            summary: "compiled-in deterministic scanner observation".into(),
            findings: Vec::new(),
        }
    }

    fn attestation(id: &str, state: AttestationState) -> VerificationEvidence {
        VerificationEvidence::Attestation(VerifiedAttestation {
            requirement_id: id.into(),
            snapshot: snapshot(),
            state,
            evidence: pointer(id),
            observed_at_unix: 100,
            summary: format!("{id} was evaluated by its trusted adapter"),
            findings: Vec::new(),
        })
    }

    fn complete_evidence() -> Vec<VerificationEvidence> {
        vec![
            native(ExecutionState::Success, true),
            attestation("acceptance", AttestationState::Success),
            attestation("review", AttestationState::Success),
            VerificationEvidence::Advisory {
                requirement_id: "advice".into(),
                evidence: pointer("advice"),
                summary: "consider a smaller interface".into(),
            },
        ]
    }

    #[test]
    fn full_snapshot_bound_checks_acceptance_and_reviews_are_required_for_success() {
        let report = aggregate(&plan(), &complete_evidence(), 120, &[]).expect("complete report");
        assert_eq!(report.state, VerificationState::Success);
        assert_eq!(report.schema, VERIFICATION_SCHEMA);
        assert_eq!(report.lean_baseline_revision, LEAN_BASELINE_REVISION);
        assert_eq!(report.results.len(), 4);
        assert_eq!(report.results[0].state, CheckState::Success);
        assert_eq!(report.results[1].state, CheckState::Advisory);
        assert_eq!(report.results[2].state, CheckState::Success);
        assert_eq!(report.results[3].state, CheckState::Success);
        assert!(report.human_summary().contains("(success)"));

        let partial = aggregate(&plan(), &[native(ExecutionState::Success, true)], 120, &[])
            .expect("partial is represented");
        assert_eq!(partial.state, VerificationState::Unknown);
        assert_eq!(
            partial
                .results
                .iter()
                .filter(|result| result.reason_code == "required_evidence_missing")
                .count(),
            2
        );
    }

    #[test]
    fn compiled_native_check_is_trusted_without_fabricating_external_execution() {
        let plan = VerificationPlan {
            subject: "local deterministic scan".into(),
            snapshot: snapshot(),
            requirements: vec![requirement("native", RequirementKind::NativeCheck, true)],
        };
        let report = aggregate(
            &plan,
            &[compiled_native(AttestationState::Success)],
            120,
            &[],
        )
        .expect("compiled native evidence aggregates");
        assert_eq!(report.state, VerificationState::Success);
        assert_eq!(report.results[0].reason_code, "check_succeeded");
        assert_eq!(
            report.results[0]
                .evidence
                .as_ref()
                .map(|evidence| evidence.source.as_str()),
            Some("trusted-adapter")
        );
    }

    #[test]
    fn every_snapshot_dimension_and_freshness_invalidates_replay() {
        let report = aggregate(&plan(), &complete_evidence(), 120, &[]).expect("complete report");
        assert_eq!(
            replay(&report, &snapshot(), 150).expect("fresh replay"),
            VerificationState::Success
        );
        let mutations: Vec<fn(&mut SnapshotBinding)> = vec![
            |value| value.code_tree = digest('7'),
            |value| value.policy = digest('7'),
            |value| value.checker_bundle = digest('7'),
            |value| value.scope = digest('7'),
            |value| value.environment = digest('7'),
            |value| value.trust = digest('7'),
        ];
        for mutation in mutations {
            let mut changed = snapshot();
            mutation(&mut changed);
            assert_eq!(
                replay(&report, &changed, 150).expect("changed replay"),
                VerificationState::Unknown
            );
        }
        assert_eq!(
            replay(&report, &snapshot(), 161).expect("expired replay"),
            VerificationState::Unknown
        );
    }

    #[test]
    fn skipped_unknown_unavailable_and_execution_approval_never_pass() {
        let cases = [
            (
                VerificationEvidence::Skipped {
                    requirement_id: "native".into(),
                    reason: "too slow".into(),
                },
                VerificationState::Unknown,
                CheckState::Skipped,
            ),
            (
                native(ExecutionState::Unknown, true),
                VerificationState::Unknown,
                CheckState::Unknown,
            ),
            (
                native(ExecutionState::Unavailable, false),
                VerificationState::Unavailable,
                CheckState::Unavailable,
            ),
            (
                native(ExecutionState::NeedsExecutionApproval, false),
                VerificationState::NeedsExecutionApproval,
                CheckState::NeedsExecutionApproval,
            ),
        ];
        for (native_evidence, expected_report, expected_check) in cases {
            let evidence = vec![
                native_evidence,
                attestation("acceptance", AttestationState::Success),
                attestation("review", AttestationState::Success),
            ];
            let report = aggregate(&plan(), &evidence, 120, &[]).expect("failure is represented");
            assert_eq!(report.state, expected_report);
            assert_eq!(
                report
                    .results
                    .iter()
                    .find(|result| result.requirement_id == "native")
                    .expect("native result")
                    .state,
                expected_check
            );
        }
    }

    #[test]
    fn fabricated_execution_success_without_trust_is_unknown() {
        let evidence = vec![
            native(ExecutionState::Success, false),
            attestation("acceptance", AttestationState::Success),
            attestation("review", AttestationState::Success),
        ];
        let report = aggregate(&plan(), &evidence, 120, &[]).expect("malice is represented");
        assert_eq!(report.state, VerificationState::Unknown);
        let native = report
            .results
            .iter()
            .find(|result| result.requirement_id == "native")
            .expect("native result");
        assert_eq!(native.reason_code, "untrusted_execution_receipt");
    }

    #[test]
    fn evidence_variants_cannot_substitute_for_another_requirement_kind() {
        let substitutions = vec![
            (
                "acceptance",
                VerificationEvidence::Execution {
                    requirement_id: "acceptance".into(),
                    snapshot: snapshot(),
                    receipt: Box::new(execution(ExecutionState::Success, true)),
                    evidence: pointer("wrong-execution"),
                    observed_at_unix: 100,
                    findings: Vec::new(),
                },
            ),
            (
                "native",
                VerificationEvidence::Attestation(VerifiedAttestation {
                    requirement_id: "native".into(),
                    snapshot: snapshot(),
                    state: AttestationState::Success,
                    evidence: pointer("wrong-attestation"),
                    observed_at_unix: 100,
                    summary: "self-reported native success".into(),
                    findings: Vec::new(),
                }),
            ),
            (
                "review",
                VerificationEvidence::Advisory {
                    requirement_id: "review".into(),
                    evidence: pointer("wrong-advisory"),
                    summary: "an advisory is not a review".into(),
                },
            ),
        ];
        for (target, substitution) in substitutions {
            let evidence = complete_evidence()
                .into_iter()
                .map(|item| {
                    if evidence_id(&item) == target {
                        substitution.clone()
                    } else {
                        item
                    }
                })
                .collect::<Vec<_>>();
            let report = aggregate(&plan(), &evidence, 120, &[])
                .expect("substitution is represented as unknown");
            assert_eq!(report.state, VerificationState::Unknown);
            let result = report
                .results
                .iter()
                .find(|result| result.requirement_id == target)
                .expect("substituted result");
            assert_eq!(result.state, CheckState::Unknown);
            assert_eq!(result.reason_code, "evidence_kind_mismatch");
        }
    }

    #[test]
    fn stale_or_wrong_release_observation_is_rejected() {
        let mut stale = complete_evidence();
        if let VerificationEvidence::Attestation(attestation) = &mut stale[1] {
            attestation.observed_at_unix = 1;
        }
        let report = aggregate(&plan(), &stale, 120, &[]).expect("stale represented");
        assert_eq!(report.state, VerificationState::Unknown);
        assert!(report.results.iter().any(|result| {
            result.requirement_id == "acceptance" && result.reason_code == "evidence_expired"
        }));

        let mut wrong_release = complete_evidence();
        if let VerificationEvidence::Attestation(attestation) = &mut wrong_release[2] {
            attestation.snapshot.code_tree = digest('9');
        }
        let report =
            aggregate(&plan(), &wrong_release, 120, &[]).expect("wrong release represented");
        assert_eq!(report.state, VerificationState::Unknown);
        assert!(report.results.iter().any(|result| {
            result.requirement_id == "review" && result.reason_code == "snapshot_mismatch"
        }));
    }

    #[test]
    fn violated_findings_are_actionable_bounded_and_redacted() {
        let secret = "highly-sensitive-token";
        let finding = Finding {
            file: Some("src/checkout.rs".into()),
            line: Some(18),
            column: Some(4),
            rule_id: "ARCH-01".into(),
            observed: format!("private import using {secret}"),
            expected: "public module interface".into(),
            rationale: "D-002 requires public boundaries".into(),
            repair_direction: "use payments/public".into(),
            permitted_next_action: "repair current task scope".into(),
            verification_command: "npm run test:architecture && wh check".into(),
        };
        let mut evidence = complete_evidence();
        if let VerificationEvidence::Execution {
            receipt, findings, ..
        } = &mut evidence[0]
        {
            receipt.state = ExecutionState::Violated;
            findings.push(finding);
        }
        let report =
            aggregate(&plan(), &evidence, 120, &[secret.into()]).expect("violation represented");
        assert_eq!(report.state, VerificationState::Violated);
        let json = serde_json::to_string(&report).expect("serialize report");
        assert!(!json.contains(secret));
        assert!(json.contains("[REDACTED]"));
        let finding = &report
            .results
            .iter()
            .find(|result| result.requirement_id == "native")
            .expect("native result")
            .findings[0];
        assert_eq!(finding.line, Some(18));
        assert!(!finding.repair_direction.is_empty());
        assert!(!finding.verification_command.is_empty());
    }

    #[test]
    fn canonical_receipts_are_order_independent_idempotent_and_tamper_evident() {
        let evidence = complete_evidence();
        let first = aggregate(&plan(), &evidence, 120, &[]).expect("first report");
        let mut reversed = evidence;
        reversed.reverse();
        let second = aggregate(&plan(), &reversed, 120, &[]).expect("second report");
        assert_eq!(first, second);

        let restored: VerificationReport =
            serde_json::from_slice(&serde_json::to_vec(&first).expect("serialize receipt"))
                .expect("restore receipt");
        assert_eq!(
            replay(&restored, &snapshot(), 120).expect("replay receipt"),
            VerificationState::Success
        );
        let mut tampered = restored;
        tampered.summary.push_str(" altered");
        assert!(matches!(
            replay(&tampered, &snapshot(), 120),
            Err(VerificationError::ReceiptTampered)
        ));
    }

    #[test]
    fn code_tree_digest_changes_for_dirty_bytes_and_rejects_symlinks() {
        let project = TempDir::new().expect("temp project");
        std::fs::create_dir(project.path().join("src")).expect("create src");
        let file = project.path().join("src/lib.rs");
        std::fs::write(&file, "pub fn value() -> u8 { 1 }\n").expect("write source");
        let clean =
            code_tree_digest(project.path(), &[PathBuf::from("src")]).expect("clean snapshot");
        std::fs::write(&file, "pub fn value() -> u8 { 2 }\n").expect("dirty source");
        let dirty =
            code_tree_digest(project.path(), &[PathBuf::from("src")]).expect("dirty snapshot");
        assert_ne!(clean, dirty);

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(project.path().join("src"), project.path().join("linked"))
                .expect("create symlink");
            assert!(matches!(
                code_tree_digest(project.path(), &[PathBuf::from("linked")]),
                Err(VerificationError::UnsafePath(_))
            ));
        }
    }
}
