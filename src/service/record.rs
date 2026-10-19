//! The records the workflows write, and the snapshot and digest helpers a
//! check binds its report to.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::domain::{
    AgreementRecord, ContentDigest, EvidenceRef, PrincipalKind, PrincipalRef, Proposal,
    ProposalState, Provenance, ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, Scope,
    SCHEMA_VERSION_V1,
};
use crate::storage::{ProjectLayout, RecordStore, StorageError};

use super::check::SnapshotBinding;
use super::utc_now;

/// Current UTC time in RFC 3339 form, second precision.
pub fn utc_timestamp() -> String {
    utc_now()
}

/// The scope every new record carries: this project only.
pub(crate) fn project_scope(layout: &ProjectLayout) -> Scope {
    Scope::project(format!("project-{}", &layout.project_id()[..12]))
}

/// A short, stable id suffix for a key.
pub(crate) fn key_suffix(key: &str, length: usize) -> String {
    let digest = digest_bytes(key.as_bytes());
    digest.as_str()["sha256:".len()..]
        .chars()
        .take(length)
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct ChangeNarrative {
    pub(crate) rationale: String,
    pub(crate) source: String,
    pub(crate) expected_effect: String,
    pub(crate) impact: String,
    pub(crate) examples: Vec<String>,
    pub(crate) conflicts: Vec<String>,
}

impl ChangeNarrative {
    pub(crate) fn render(&self, base_revision: u64) -> String {
        format!(
            "Base revision: {base_revision}\nRationale: {}\nSource: {}\nExpected effect: {}\nImpact: {}\nExamples: {}\nConflicts: {}",
            self.rationale,
            self.source,
            self.expected_effect,
            self.impact,
            self.examples.join("; "),
            self.conflicts.join("; ")
        )
    }
}

pub(crate) fn ensure_local_change_proposal(
    repository: &dyn RecordStore,
    candidate: &AgreementRecord,
    candidate_reference: &crate::domain::RecordRef,
    narrative: &ChangeNarrative,
    base_revision: u64,
    request_id: &str,
) -> Result<crate::domain::RecordRef, StorageError> {
    let proposal_key = format!("{request_id}:proposal");
    let proposal_id = RecordId::new(format!(
        "proposal.change_{}",
        &format!("{:x}", Sha256::digest(request_id.as_bytes()))[..24]
    ))
    .map_err(StorageError::Domain)?;
    let proposal = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: proposal_id,
        revision: 1,
        scope: candidate.scope.clone(),
        owner: candidate.owner.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: candidate.owner.clone(),
            recorded_at: utc_now(),
            sources: vec![EvidenceRef {
                system: "owner_selected_source".into(),
                locator: narrative.source.clone(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: proposal_key.clone(),
        body: RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: format!(
                "{} {}: {}",
                crate::projection::kind_label(match &candidate.body {
                    RecordBody::Mission(_) => "mission",
                    RecordBody::Principle(_) => "principle",
                    RecordBody::Rule(_) | RecordBody::StoredRule(_) | RecordBody::Standard(_) => {
                        "rule"
                    }
                    RecordBody::Retirement(_) => "retirement",
                    _ => "record",
                }),
                if candidate.revision <= 1 {
                    "proposed"
                } else {
                    "revised"
                },
                crate::projection::record_title(candidate)
                    .chars()
                    .take(120)
                    .collect::<String>()
            ),
            rationale: narrative.render(base_revision),
            proposed_records: vec![candidate_reference.clone()],
            binding: None,
        }),
    };
    if let Some(existing) = repository.by_idempotency_key(&proposal_key)? {
        if existing.id != proposal.id || existing.body != proposal.body {
            return Err(StorageError::IdempotencyConflict(proposal_key));
        }
        return existing.reference().map_err(StorageError::Domain);
    }
    repository.append(&proposal, None)
}

pub fn agreement_record(
    layout: &ProjectLayout,
    id: &str,
    idempotency_key: String,
    body: RecordBody,
    owner_display: Option<&str>,
) -> Result<AgreementRecord, crate::domain::DomainError> {
    let stable_id = format!(
        "local:{}",
        &format!("{:x}", Sha256::digest(layout.project_id().as_bytes()))[..20]
    );
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id,
        display_name: owner_display.map(str::to_owned),
    };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id)?,
        revision: 1,
        scope: project_scope(layout),
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: principal,
            recorded_at: utc_now(),
            sources: vec![EvidenceRef {
                system: "whetstone_cli".into(),
                locator: "owner_input".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key,
        body,
    };
    record.validate()?;
    Ok(record)
}

/// What a check's report is bound to: Git's view of the code, the policy
/// inputs, this binary, the rules asked for and the platform.
pub(crate) fn compute_check_snapshot(
    project: &Path,
    rules: &[String],
) -> Result<SnapshotBinding, String> {
    let code_tree = super::check::code_tree_digest(project, &[PathBuf::from(".")])
        .map_err(|error| format!("The code snapshot could not be bound safely: {error}"))?;
    let policy = digest_rule_inputs(project)
        .map_err(|error| error.to_string())
        .and_then(|digest| ContentDigest::new(digest).map_err(|error| error.to_string()))
        .map_err(|error| format!("The policy snapshot could not be bound safely: {error}"))?;
    let checker_bundle = checker_bundle_digest()
        .map_err(|error| format!("The compiled checker could not be content-bound: {error}"))?;
    let scope = digest_json(&json!({
        "project": project.to_string_lossy(),
        "rules": rules,
    }));
    let environment = digest_json(&json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "owner_accepted_rules": true,
    }));
    let trust = digest_bytes(b"whetstone-trust-v3\0owner-accepted-rules-bounded-execution");
    Ok(SnapshotBinding {
        code_tree,
        policy,
        checker_bundle,
        scope,
        environment,
        trust,
    })
}

pub fn digest_bytes(bytes: &[u8]) -> ContentDigest {
    match ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))) {
        Ok(digest) => digest,
        Err(_) => unreachable!("SHA-256 formatting always satisfies the digest contract"),
    }
}

pub(crate) fn digest_json(value: &Value) -> ContentDigest {
    digest_bytes(&serde_json::to_vec(value).unwrap_or_default())
}

pub(crate) fn checker_bundle_digest() -> Result<ContentDigest, String> {
    let mut hasher = Sha256::new();
    hasher.update(format!(
        "whetstone:{}:checker-v1\0",
        env!("CARGO_PKG_VERSION")
    ));
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let bytes = fs::read(executable).map_err(|error| error.to_string())?;
    hasher.update(bytes);
    match ContentDigest::new(format!("sha256:{:x}", hasher.finalize())) {
        Ok(digest) => Ok(digest),
        Err(_) => unreachable!("SHA-256 formatting always satisfies the digest contract"),
    }
}

pub(crate) fn digest_rule_inputs(project: &Path) -> Result<String, std::io::Error> {
    let mut files = WalkDir::new(project.join("whetstone"))
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    matches!(extension, "yaml" | "yml" | "toml" | "json" | "jsonc")
                })
        })
        .map(|entry| entry.into_path())
        .collect::<Vec<_>>();
    for candidate in [
        "Cargo.toml",
        "ruff.toml",
        ".ruff.toml",
        "pyproject.toml",
        "biome.json",
        "biome.jsonc",
        "rustfmt.toml",
        ".rustfmt.toml",
    ] {
        let path = project.join(candidate);
        if path.is_file() {
            files.push(path);
        }
    }
    files.sort();
    files.dedup();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(project).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(file)?);
        hasher.update([0]);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}
