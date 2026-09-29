//! The records and receipts the workflows write, and the snapshot and
//! evidence helpers they bind to. Every receipt carries the commit it ran at,
//! the rule or feature revision it proved and where its evidence lives, so a
//! stale proof is detected the same way everywhere.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::agreement::AgreementState;
use crate::domain::{
    AgreementRecord, AuthorizationAxis, ContentDigest, EvidenceRef, ExternalRef, ExternalSystem,
    Freshness, PolicyStateSnapshot, PrincipalKind, PrincipalRef, Proposal, ProposalState,
    Provenance, ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, RecordRef, Scope,
    VerificationAxis, VerificationReceipt, SCHEMA_VERSION_V1,
};
use crate::storage::{ProjectLayout, RecordStore, StorageError, StoreKind};
use crate::verification::{self, Finding, SnapshotBinding, VerificationReport, VerificationState};

use super::{
    domain_response, load_records, storage_error, utc_now, MaintainOutcome, ServiceResponse,
    ServiceState, DOCTOR_EVIDENCE, DRIVER_CONFIG_EVIDENCE, DRIVER_EVIDENCE, MAINTAIN_SUBJECT,
    MAP_EVIDENCE,
};

/// Current UTC time in RFC 3339 form, second precision.
pub fn utc_timestamp() -> String {
    utc_now()
}

/// The scope every new record carries: this project only.
pub(crate) fn project_scope(layout: &ProjectLayout) -> Scope {
    Scope::project(format!("project-{}", &layout.project_id()[..12]))
}

/// A receipt envelope: candidate-only provenance recorded by a Whetstone
/// principal, never an agreement.
#[allow(clippy::too_many_arguments)]
pub(crate) fn receipt_record(
    layout: &ProjectLayout,
    id: String,
    idempotency_key: String,
    principal: &str,
    kind: ProvenanceKind,
    recorded_at: &str,
    source: EvidenceRef,
    body: RecordBody,
) -> Result<AgreementRecord, crate::domain::DomainError> {
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: principal.into(),
        display_name: None,
    };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id)?,
        revision: 1,
        scope: project_scope(layout),
        owner: principal.clone(),
        provenance: Provenance {
            kind,
            recorded_by: principal,
            recorded_at: recorded_at.into(),
            sources: vec![source],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes: None,
        idempotency_key,
        body,
    };
    record.validate()?;
    Ok(record)
}

/// Evidence plus the commit it was produced at, when the repository has one.
pub(crate) fn with_head(
    layout: &ProjectLayout,
    mut evidence: Vec<EvidenceRef>,
) -> Vec<EvidenceRef> {
    if let Some(head) = crate::gates::head_commit(layout.project_root()) {
        evidence.push(EvidenceRef {
            system: crate::proof::GIT_HEAD_SYSTEM.into(),
            locator: head,
            digest: None,
        });
    }
    evidence
}

/// A short, stable id suffix for a key.
pub(crate) fn key_suffix(key: &str, length: usize) -> String {
    let digest = digest_bytes(key.as_bytes());
    digest.as_str()["sha256:".len()..]
        .chars()
        .take(length)
        .collect()
}

/// Append to the private store, creating it on first use.
pub(crate) fn append_private(
    layout: &ProjectLayout,
    record: &AgreementRecord,
) -> Result<RecordRef, StorageError> {
    let store = RecordStore::initialize(&layout.private_store(), StoreKind::Private)?;
    if let Some(existing) = store.by_idempotency_key(&record.idempotency_key)? {
        return existing.reference().map_err(StorageError::Domain);
    }
    store.append(record, None)
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
    repository: &RecordStore,
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
                    RecordBody::Rule(_) | RecordBody::Standard(_) | RecordBody::Guidance(_) => {
                        "rule"
                    }
                    RecordBody::Feature(_) => "feature",
                    RecordBody::VerificationMap(_) => "map",
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

pub(crate) fn compute_check_snapshot(
    project: &Path,
    requested_paths: &[PathBuf],
    language: Option<&str>,
    rules: &[String],
) -> Result<(Vec<PathBuf>, Vec<PathBuf>, SnapshotBinding), String> {
    let mut scan_paths = Vec::new();
    for path in requested_paths {
        let joined = if path.is_absolute() {
            path.clone()
        } else {
            project.join(path)
        };
        let canonical = joined
            .canonicalize()
            .map_err(|error| format!("A requested check path is unavailable: {error}"))?;
        if !canonical.starts_with(project) {
            return Err("A requested path escapes the project root.".into());
        }
        scan_paths.push(canonical);
    }
    let relative_paths = scan_paths
        .iter()
        .map(|path| {
            let relative = path.strip_prefix(project).unwrap_or(path);
            if relative.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                relative.to_path_buf()
            }
        })
        .collect::<Vec<_>>();
    let code_tree = verification::code_tree_digest(project, &relative_paths)
        .map_err(|error| format!("The code snapshot could not be bound safely: {error}"))?;
    let policy = digest_rule_inputs(project)
        .map_err(|error| error.to_string())
        .and_then(|digest| ContentDigest::new(digest).map_err(|error| error.to_string()))
        .map_err(|error| format!("The policy snapshot could not be bound safely: {error}"))?;
    let checker_bundle = checker_bundle_digest()
        .map_err(|error| format!("The compiled checker could not be content-bound: {error}"))?;
    let scope = digest_json(&json!({
        "project": project.to_string_lossy(),
        "paths": relative_paths,
        "language": language,
        "rules": rules,
    }));
    let environment = digest_json(&json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "command_validators": false,
    }));
    let trust = digest_bytes(
        b"whetstone-trust-v2\0compiled-in-deterministic-scanner\0no-external-execution-receipt",
    );
    Ok((
        scan_paths,
        relative_paths,
        SnapshotBinding {
            code_tree,
            policy,
            checker_bundle,
            scope,
            environment,
            trust,
        },
    ))
}

pub(crate) fn count(value: &Value, field: &str) -> u64 {
    value.get(field).and_then(Value::as_u64).unwrap_or(0)
}

pub(crate) fn array_len(value: &Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(Value::as_array)
        .map_or(0, |items| items.len() as u64)
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
        "whetstone:{}:scanner-v1\0",
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

pub(crate) fn scan_findings(project: &Path, result: &Value) -> Vec<Finding> {
    let mut findings = Vec::new();
    if let Some(violations) = result.get("violations").and_then(Value::as_array) {
        for violation in violations {
            let rule_id = value_text(violation, "rule_id", "whetstone.unknown-rule");
            let description = value_text(
                violation,
                "description",
                "the accepted deterministic rule must be satisfied",
            );
            let source = value_text(violation, "source_url", "accepted project policy");
            let observed = value_text(violation, "match", "the rule matched this location");
            findings.push(Finding {
                file: finding_path(project, violation.get("file")),
                line: violation
                    .get("line")
                    .and_then(Value::as_u64)
                    .and_then(|line| u32::try_from(line).ok()),
                column: violation
                    .get("column")
                    .and_then(Value::as_u64)
                    .and_then(|column| u32::try_from(column).ok()),
                rule_id,
                observed,
                expected: description.clone(),
                rationale: format!("{description} Governing source: {source}."),
                repair_direction: "Change the reported source within the current task scope so the accepted rule no longer matches.".into(),
                permitted_next_action: "repair the reported source, then rerun the verification command".into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    if let Some(issues) = result.get("config_issues").and_then(Value::as_array) {
        for issue in issues {
            findings.push(Finding {
                file: finding_path(project, issue.get("path")),
                line: None,
                column: None,
                rule_id: value_text(issue, "rule_id", "whetstone.checker-configuration"),
                observed: value_text(issue, "issue", "checker configuration is incomplete"),
                expected: "Required rule and checker configuration must be available and valid."
                    .into(),
                rationale: "Unknown or unavailable required evidence cannot be treated as success."
                    .into(),
                repair_direction: value_text(
                    issue,
                    "fix",
                    "restore the required checker configuration without weakening the policy",
                ),
                permitted_next_action:
                    "repair checker configuration, then rerun the verification command".into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    if let Some(skipped) = result.get("skipped").and_then(Value::as_array) {
        for item in skipped {
            findings.push(Finding {
                file: None,
                line: None,
                column: None,
                rule_id: value_text(item, "rule_id", "whetstone.skipped-check"),
                observed: value_text(item, "reason", "a required check was skipped"),
                expected: "Every applicable required check must produce current evidence.".into(),
                rationale: "Skipped required evidence cannot be treated as success.".into(),
                repair_direction: "Restore a supported deterministic binding for the rule.".into(),
                permitted_next_action:
                    "restore the checker binding, then rerun the verification command".into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    if let Some(warnings) = result.get("warnings").and_then(Value::as_array) {
        for warning in warnings.iter().filter_map(Value::as_str) {
            findings.push(Finding {
                file: None,
                line: None,
                column: None,
                rule_id: "whetstone.scanner-warning".into(),
                observed: warning.into(),
                expected: "The scanner must inspect the complete requested scope without warnings."
                    .into(),
                rationale: "Partial scanner evidence cannot establish completion.".into(),
                repair_direction: "Resolve the warning without narrowing the requested scope."
                    .into(),
                permitted_next_action: "resolve the warning, then rerun the verification command"
                    .into(),
                verification_command: "wh check --json".into(),
            });
        }
    }
    findings
}

fn value_text(value: &Value, field: &str, fallback: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn finding_path(project: &Path, value: Option<&Value>) -> Option<String> {
    let text = value.and_then(Value::as_str)?;
    let path = Path::new(text);
    let relative = if path.is_absolute() {
        path.strip_prefix(project).ok()?
    } else {
        path
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        None
    } else {
        Some(relative.to_string_lossy().into_owned())
    }
}

pub(crate) fn service_state(state: VerificationState) -> ServiceState {
    match state {
        VerificationState::Success => ServiceState::Success,
        VerificationState::Violated => ServiceState::Violated,
        VerificationState::Unavailable => ServiceState::Unavailable,
        VerificationState::Unknown => ServiceState::Unknown,
    }
}

pub(crate) fn persist_verification_receipt(
    layout: &ProjectLayout,
    request_id: &str,
    report: &VerificationReport,
    checked_at: &str,
) -> Result<Option<crate::domain::RecordRef>, StorageError> {
    let store_path = layout.private_store();
    if !crate::beads::is_initialized(&store_path) {
        return Ok(None);
    }
    let repository = RecordStore::initialize(&store_path, StoreKind::Private)?;
    // Two checks of one snapshot that ran different gates are different
    // receipts; only an exact replay (same request, snapshot and results)
    // returns the existing one.
    let selection = report
        .results
        .iter()
        .map(|result| format!("{}={:?}", result.requirement_id, result.state))
        .collect::<Vec<_>>()
        .join(",");
    let identity = digest_bytes(
        format!(
            "{request_id}\0{selection}\0{}\0{}\0{}\0{}\0{}\0{}",
            report.snapshot.code_tree.as_str(),
            report.snapshot.policy.as_str(),
            report.snapshot.checker_bundle.as_str(),
            report.snapshot.scope.as_str(),
            report.snapshot.environment.as_str(),
            report.snapshot.trust.as_str(),
        )
        .as_bytes(),
    );
    let suffix = &identity.as_str()["sha256:".len().."sha256:".len() + 32];
    let idempotency_key = format!("check:{suffix}");
    if let Some(existing) = repository.by_idempotency_key(&idempotency_key)? {
        return existing.reference().map(Some).map_err(StorageError::Domain);
    }
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "whetstone:compiled-in-checker".into(),
        display_name: None,
    };
    let verification = match report.state {
        VerificationState::Success => VerificationAxis::Pass,
        VerificationState::Violated => VerificationAxis::Fail,
        VerificationState::Unknown | VerificationState::Unavailable => VerificationAxis::Unknown,
    };
    let freshness =
        if report.results.iter().any(|result| {
            result.required && result.freshness == verification::EvidenceFreshness::Stale
        }) {
            Freshness::Stale
        } else if report.results.iter().any(|result| {
            result.required && result.freshness == verification::EvidenceFreshness::Missing
        }) {
            Freshness::Missing
        } else {
            Freshness::Fresh
        };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(format!("verification.check_{suffix}")).map_err(StorageError::Domain)?,
        revision: 1,
        scope: project_scope(layout),
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::DeterministicCheck,
            recorded_by: principal,
            recorded_at: checked_at.into(),
            sources: vec![EvidenceRef {
                system: "whetstone_verification_report".into(),
                locator: report.receipt_id.as_str().into(),
                digest: Some(report.receipt_id.clone()),
            }],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes: None,
        idempotency_key,
        body: RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: format!("project:{}:check", layout.project_id()),
                revision: Some(report.receipt_id.as_str().into()),
            },
            code_digest: report.snapshot.code_tree.clone(),
            policy_state: PolicyStateSnapshot {
                accepted: None,
                required: None,
                installed: None,
                experimental: None,
            },
            verification,
            authorization: AuthorizationAxis::Unknown,
            freshness,
            checked_at: checked_at.into(),
            related_records: report
                .results
                .iter()
                .flat_map(|result| result.governing_records.clone())
                .collect(),
            evidence: with_head(
                layout,
                vec![EvidenceRef {
                    system: "whetstone_verification_report".into(),
                    locator: report.receipt_id.as_str().into(),
                    digest: Some(report.receipt_id.clone()),
                }],
            ),
        }),
    };
    repository.append(&record, None).map(Some)
}

/// Record which skill revision an agent host has on disk at this checkpoint.
/// Current only when the host's files match the manifest and the manifest
/// matches the accepted agreement; anything else is unknown, never assumed
/// delivered.
pub(crate) fn attach_host_checkpoint(
    response: &mut ServiceResponse,
    project_dir: &Path,
    host: &str,
) {
    if !crate::skill::HOSTS.iter().any(|(name, _)| *name == host) {
        response.data["host_checkpoint"] = json!({"host": host, "state": "unknown_host"});
        return;
    }
    let Ok(layout) = ProjectLayout::resolve(project_dir) else {
        return;
    };
    let state = match load_records(&layout) {
        Ok(loaded) if loaded.exists() => AgreementState::from_records(loaded.union().0),
        _ => return,
    };
    let manifest = crate::skill::read_manifest(&layout);
    let projections = crate::hosts::projections(
        layout.project_root(),
        manifest.as_ref(),
        &state.in_force_digest(),
    );
    let projection = projections
        .iter()
        .find(|projection| projection.host == host);
    let installed = crate::hosts::installed_skill_digest(&layout, host);
    let current = projection.is_some_and(|projection| projection.state == "current")
        && installed.is_some()
        && installed == projection.and_then(|projection| projection.skill_digest.clone());
    let checked_at = utc_now();
    let key = format!("host:{host}:{checked_at}:{}", response.request_id);
    let key_digest = digest_bytes(key.as_bytes());
    let suffix = &key_digest.as_str()["sha256:".len()..][..24];
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: format!("whetstone:host-{host}"),
        display_name: None,
    };
    let mut evidence = vec![EvidenceRef {
        system: "whetstone_adapter".into(),
        locator: crate::hosts::ADAPTER_VERSION.into(),
        digest: None,
    }];
    if let Some(installed) = &installed {
        if let Ok(digest) = ContentDigest::new(installed.clone()) {
            evidence.push(EvidenceRef {
                system: "whetstone_skill".into(),
                locator: format!("{host}/SKILL.md"),
                digest: Some(digest),
            });
        }
    }
    let Ok(id) = RecordId::new(format!("verification.host_{suffix}")) else {
        return;
    };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id,
        revision: 1,
        scope: project_scope(&layout),
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::DeterministicCheck,
            recorded_by: principal,
            recorded_at: checked_at.clone(),
            sources: vec![EvidenceRef {
                system: "agent_host".into(),
                locator: host.into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes: None,
        idempotency_key: key,
        body: RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: format!("{}{host}", crate::hosts::HOST_SUBJECT_PREFIX),
                revision: installed.clone(),
            },
            code_digest: digest_bytes(
                crate::gates::workspace_fingerprint(layout.project_root())
                    .unwrap_or_default()
                    .as_bytes(),
            ),
            policy_state: PolicyStateSnapshot {
                accepted: None,
                required: None,
                installed: None,
                experimental: None,
            },
            verification: if current {
                VerificationAxis::Pass
            } else {
                VerificationAxis::Unknown
            },
            authorization: AuthorizationAxis::Unknown,
            freshness: Freshness::Fresh,
            checked_at,
            related_records: Vec::new(),
            evidence,
        }),
    };
    let recorded = layout
        .private_store_exists()
        .then(|| RecordStore::open_existing(&layout.private_store(), StoreKind::Private))
        .and_then(Result::ok)
        .map(|store| store.append(&record, None).is_ok())
        .unwrap_or(false);
    response.data["host_checkpoint"] = json!({
        "host": host,
        "skill": projection.map_or("not installed", |projection| projection.state),
        "detail": projection.map(|projection| projection.detail.clone()),
        "installed_skill_digest": installed,
        "acknowledged": current,
        "recorded": recorded,
        "delivery": crate::hosts::delivery(host),
    });
    if !current {
        response.permitted_actions.push(format!(
            "wh init --action wire --host {host} (the projection for {host} is not current)"
        ));
    }
}

/// What a drive receipt binds besides the gate: the map revision it proved
/// and the doctor verdict it drove after.
pub(crate) struct DriveBinding<'a> {
    pub(crate) feature: Option<crate::domain::RecordRef>,
    pub(crate) doctor: Option<&'a crate::gates::DoctorResult>,
}

/// Driver script, driver configuration, map revision and doctor freshness as
/// receipt evidence, so a proof names exactly what drove it.
pub(crate) fn drive_evidence(
    project: &Path,
    binding: &DriveBinding<'_>,
    checked_at: &str,
) -> Vec<EvidenceRef> {
    let mut evidence = Vec::new();
    for (system, relative) in [
        (DRIVER_EVIDENCE, crate::gates::DRIVER_RELATIVE),
        (DRIVER_CONFIG_EVIDENCE, crate::skill::DRIVER_CONFIG_RELATIVE),
    ] {
        if let Ok(bytes) = fs::read(project.join(relative)) {
            evidence.push(EvidenceRef {
                system: system.into(),
                locator: relative.into(),
                digest: Some(digest_bytes(&bytes)),
            });
        }
    }
    if let Some(feature) = &binding.feature {
        evidence.push(EvidenceRef {
            system: MAP_EVIDENCE.into(),
            locator: format!("{}@r{}", feature.id.as_str(), feature.revision),
            digest: Some(feature.digest.clone()),
        });
    }
    if let Some(doctor) = binding.doctor {
        let verdict = format!(
            "{} at {checked_at}: {}",
            if doctor.ok { "ok" } else { "failed" },
            doctor.detail
        );
        evidence.push(EvidenceRef {
            system: DOCTOR_EVIDENCE.into(),
            locator: verdict.chars().take(400).collect(),
            digest: Some(digest_bytes(verdict.as_bytes())),
        });
    }
    evidence
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

/// Record a maintain pass's reported outcome as a receipt. Clean and changed
/// need evidence (the run notes or the PR); without it the receipt is
/// unknown. Blocked is always unknown: coverage did not finish.
pub(crate) fn record_maintain_outcome(
    layout: &ProjectLayout,
    request_id: String,
    outcome: MaintainOutcome,
    evidence: Option<&str>,
) -> ServiceResponse {
    let loaded = match load_records(layout) {
        Ok(loaded) => loaded,
        Err(error) => return storage_error("check", request_id, error),
    };
    let state = AgreementState::from_records(loaded.union().0);
    let related = state
        .in_force_matching(|body| {
            matches!(
                body,
                RecordBody::Feature(_) | RecordBody::VerificationMap(_)
            )
        })
        .into_iter()
        .filter_map(|record| record.reference().ok())
        .collect::<Vec<_>>();
    let evidence_ref = evidence.map(|locator| {
        let local = layout.project_root().join(locator);
        let digest = (!locator.contains("://"))
            .then(|| fs::read(&local).ok())
            .flatten()
            .map(|bytes| digest_bytes(&bytes));
        EvidenceRef {
            system: "maintain_run".into(),
            locator: locator.chars().take(500).collect(),
            digest,
        }
    });
    let local_missing = evidence.is_some_and(|locator| {
        !locator.contains("://") && !layout.project_root().join(locator).is_file()
    });
    let verification = match (outcome, &evidence_ref, local_missing) {
        (MaintainOutcome::Blocked, _, _) | (_, None, _) | (_, _, true) => VerificationAxis::Unknown,
        _ => VerificationAxis::Pass,
    };
    let checked_at = utc_now();
    // One receipt per reported pass: the default request id is per project,
    // so the outcome, evidence and time distinguish reports.
    let key = format!(
        "maintain:{request_id}:{outcome:?}:{}:{checked_at}",
        evidence
            .unwrap_or("none")
            .chars()
            .take(80)
            .collect::<String>()
    );
    let key_digest = digest_bytes(key.as_bytes());
    let suffix = &key_digest.as_str()["sha256:".len()..][..24];
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "whetstone:maintain-report".into(),
        display_name: None,
    };
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: match RecordId::new(format!("verification.maintain_{suffix}")) {
            Ok(id) => id,
            Err(error) => return domain_response("check", request_id, error),
        },
        revision: 1,
        scope: project_scope(layout),
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::ExternalObservation,
            recorded_by: principal,
            recorded_at: checked_at.clone(),
            sources: vec![EvidenceRef {
                system: "pstack".into(),
                locator: "maintain-verification-skill".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::CandidateOnly,
        },
        supersedes: None,
        idempotency_key: key,
        body: RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: MAINTAIN_SUBJECT.into(),
                revision: Some(
                    match outcome {
                        MaintainOutcome::Clean => "clean",
                        MaintainOutcome::Changed => "changed",
                        MaintainOutcome::Blocked => "blocked",
                    }
                    .into(),
                ),
            },
            code_digest: digest_bytes(
                crate::gates::workspace_fingerprint(layout.project_root())
                    .unwrap_or_default()
                    .as_bytes(),
            ),
            policy_state: PolicyStateSnapshot {
                accepted: None,
                required: None,
                installed: None,
                experimental: None,
            },
            verification,
            authorization: AuthorizationAxis::Unknown,
            freshness: Freshness::Fresh,
            checked_at,
            related_records: related,
            evidence: evidence_ref.into_iter().collect(),
        }),
    };
    let store = match RecordStore::initialize(&layout.private_store(), StoreKind::Private) {
        Ok(store) => store,
        Err(error) => return storage_error("check", request_id, error),
    };
    let reference = match store.append(&record, None) {
        Ok(reference) => reference,
        Err(error) => return storage_error("check", request_id, error),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "check",
        if verification == VerificationAxis::Pass {
            ServiceState::Success
        } else {
            ServiceState::Unknown
        },
        match (outcome, verification) {
            (MaintainOutcome::Blocked, _) => "The maintain pass was recorded as blocked; coverage did not finish, so the map is not verified.".to_string(),
            (_, VerificationAxis::Pass) => "The maintain pass outcome and its evidence were recorded.".to_string(),
            _ => "The maintain pass outcome was recorded without usable evidence, so it counts as unknown; pass --maintain-evidence <notes or PR>.".to_string(),
        },
    );
    response.permitted_actions = vec!["wh dash".into()];
    response.data = json!({"receipt": reference, "outcome": outcome, "verification": verification});
    response
}
