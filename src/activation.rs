//! Team policy activation over the shared Beads database (ADR-0002,
//! `whetstone-k5r.18`).
//!
//! `propose` records a shared proposal bound to the exact payload, the
//! team-active base and the authority revision, and writes the activation
//! manifest a pull request reviews. `activate` runs after the manifest
//! merges: it verifies the platform evidence (`crate::authority`) and only
//! then appends the independent decision and one activation per policy
//! record, pinning the exact checker files the activated gates run.
//! `team_active` is what `wh check --required` enforces: activated policy
//! only, rebuilt and validated from the shared records every time, with each
//! activation's platform evidence re-verified when asked.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::authority::{
    verify_activation, ActivationInputs, ActivationManifest, Authority, Denial, Platform, Role,
    MANIFEST_SCHEMA, PROPOSALS_DIR,
};
use crate::domain::{
    Activation, AgreementHistory, AgreementRecord, ContentDigest, Decision, DecisionVerdict,
    EvidenceRef, PrincipalKind, PrincipalRef, Proposal, ProposalBinding, ProposalState, Provenance,
    ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, RecordRef, SCHEMA_VERSION_V1,
};
use crate::storage::{AppendRequest, RecordStore};

pub const PULL_REQUEST_EVIDENCE: &str = "github_pull_request";
pub const MERGE_EVIDENCE: &str = "github_merge_commit";
pub const PIN_EVIDENCE: &str = "git_blob";

pub fn reference_text(reference: &RecordRef) -> String {
    format!(
        "{}@{}#{}",
        reference.id.as_str(),
        reference.revision,
        reference.digest.as_str()
    )
}

pub fn digest_of(parts: &[String]) -> String {
    let mut sorted = parts.to_vec();
    sorted.sort();
    let mut hasher = Sha256::new();
    for part in &sorted {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn sha256_file(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// One activated policy record and the activation that made it required.
#[derive(Debug, Clone)]
pub struct ActiveEntry {
    pub policy: AgreementRecord,
    pub activation: AgreementRecord,
    pub decision: Option<AgreementRecord>,
}

impl ActiveEntry {
    pub fn pins(&self) -> Vec<EvidenceRef> {
        match &self.activation.body {
            RecordBody::Activation(body) => body.pinned_checkers.clone(),
            _ => Vec::new(),
        }
    }
}

/// Team-active policy: the latest independent activation per policy id.
#[derive(Debug, Clone, Default)]
pub struct ActivePolicy {
    pub entries: BTreeMap<RecordId, ActiveEntry>,
    /// Records that failed validation (forged or corrupted); never active.
    pub rejected: Vec<String>,
}

impl ActivePolicy {
    pub fn digest(&self) -> String {
        digest_of(
            &self
                .entries
                .values()
                .filter_map(|entry| entry.policy.reference().ok())
                .map(|reference| reference_text(&reference))
                .collect::<Vec<_>>(),
        )
    }
}

fn stage(record: &AgreementRecord) -> u8 {
    match &record.body {
        RecordBody::Proposal(_) => 1,
        RecordBody::Decision(_) | RecordBody::LocalReview(_) => 2,
        RecordBody::Activation(_) => 3,
        _ => 0,
    }
}

/// Rebuild team-active policy from shared records, validating every
/// decision and activation structurally (independent reviewer, exact
/// binding, proposed policy). A record that fails is rejected, not trusted.
pub fn team_active(shared: &[AgreementRecord]) -> ActivePolicy {
    let mut ordered = shared.to_vec();
    ordered.sort_by(|left, right| {
        stage(left)
            .cmp(&stage(right))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.revision.cmp(&right.revision))
    });
    let mut history = AgreementHistory::default();
    let mut rejected = Vec::new();
    let mut accepted = Vec::new();
    for record in ordered {
        let expected = history.latest(&record.id).map(|current| current.revision);
        match history.append(record.clone(), expected) {
            Ok(_) => accepted.push(record),
            Err(error) => rejected.push(format!(
                "{} r{}: {error}",
                record.id.as_str(),
                record.revision
            )),
        }
    }
    let by_ref = accepted
        .iter()
        .filter_map(|record| record.reference().ok().map(|reference| (reference, record)))
        .collect::<BTreeMap<_, _>>();
    let mut entries = BTreeMap::<RecordId, (u64, ActiveEntry)>::new();
    for record in &accepted {
        let RecordBody::Activation(body) = &record.body else {
            continue;
        };
        if record.provenance.authority != ProvenanceAuthority::IndependentlyApproved {
            rejected.push(format!(
                "{} r{}: an activation without independent approval",
                record.id.as_str(),
                record.revision
            ));
            continue;
        }
        let Some(policy) = by_ref.get(&body.policy) else {
            continue;
        };
        let decision = by_ref
            .get(&body.acceptance_decision)
            .map(|record| (*record).clone());
        let newer = entries
            .get(&body.policy.id)
            .map_or(true, |(sequence, _)| body.activation_sequence > *sequence);
        if newer {
            entries.insert(
                body.policy.id.clone(),
                (
                    body.activation_sequence,
                    ActiveEntry {
                        policy: (*policy).clone(),
                        activation: record.clone(),
                        decision,
                    },
                ),
            );
        }
    }
    // An activated retirement takes its exact target out of team policy
    // (rollback is a new activation of an earlier revision's content).
    let retired = entries
        .values()
        .filter_map(|(_, entry)| match &entry.policy.body {
            RecordBody::Retirement(body) => Some(body.target.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for target in retired {
        let matches = entries
            .get(&target.id)
            .is_some_and(|(_, entry)| entry.policy.reference().ok().as_ref() == Some(&target));
        if matches {
            entries.remove(&target.id);
        }
    }
    ActivePolicy {
        entries: entries
            .into_iter()
            .map(|(id, (_, entry))| (id, entry))
            .collect(),
        rejected,
    }
}

/// Re-verify a recorded activation against the platform: the pull request it
/// names merged, and the decision's reviewer approved its exact head. Used
/// by required checks, so a record appended straight into Beads with a
/// forged approval never becomes enforceable policy.
pub fn verify_recorded(entry: &ActiveEntry, platform: &dyn Platform) -> Result<(), Denial> {
    let merge_commit = entry
        .activation
        .provenance
        .sources
        .iter()
        .find(|source| source.system == MERGE_EVIDENCE)
        .map(|source| source.locator.clone())
        .ok_or_else(|| {
            Denial::new(
                "unverifiable_activation",
                "the activation names no merge commit",
            )
        })?;
    let Some(decision) = &entry.decision else {
        return Err(Denial::new(
            "unverifiable_activation",
            "the activation's decision is missing",
        ));
    };
    let RecordBody::Decision(body) = &decision.body else {
        return Err(Denial::new(
            "unverifiable_activation",
            "the activation's decision is not a decision",
        ));
    };
    let reviewer =
        body.reviewer.stable_id.parse::<u64>().map_err(|_| {
            Denial::new("missing_identity", "the reviewer has no numeric GitHub id")
        })?;
    let evidence = platform.merge_evidence(&merge_commit)?;
    if !evidence.merged || evidence.repository_id != body.proposal_binding.repository_id {
        return Err(Denial::new(
            "unverifiable_activation",
            "the named pull request is not a merge in this repository",
        ));
    }
    let approved = evidence.reviews.iter().any(|review| {
        review.reviewer_id == reviewer
            && review.state.eq_ignore_ascii_case("APPROVED")
            && review.commit_id == evidence.head_sha
    });
    if !approved || reviewer == evidence.author_id {
        return Err(Denial::new(
            "unverifiable_activation",
            format!(
                "pull request #{} has no independent approval by the recorded reviewer",
                evidence.pull_request
            ),
        ));
    }
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Pin the repository files a gate's command runs, at the given commit.
pub fn pins_for(project_root: &Path, record: &AgreementRecord, commit: &str) -> Vec<EvidenceRef> {
    let RecordBody::Standard(standard) = &record.body else {
        return Vec::new();
    };
    let command = match &standard.enforcement {
        crate::domain::Enforcement::Test { command_ref }
        | crate::domain::Enforcement::Validator { command_ref } => command_ref.clone(),
        crate::domain::Enforcement::LintProxy { tool, .. }
        | crate::domain::Enforcement::Formatter { tool } => tool.clone(),
        _ => return Vec::new(),
    };
    let Ok(sequences) = crate::gates::parse_command(&command) else {
        return Vec::new();
    };
    let mut pins = Vec::new();
    for token in sequences.iter().flatten() {
        let relative = token.trim_start_matches("./");
        if relative.is_empty()
            || relative.contains("..")
            || Path::new(relative).is_absolute()
            || relative.starts_with('-')
        {
            continue;
        }
        let path = project_root.join(relative);
        if let Ok(bytes) = fs::read(&path) {
            if path.is_file() {
                pins.push(EvidenceRef {
                    system: PIN_EVIDENCE.into(),
                    locator: format!("{commit}:{relative}"),
                    digest: ContentDigest::new(sha256_file(&bytes)).ok(),
                });
            }
        }
    }
    pins.sort_by(|left, right| left.locator.cmp(&right.locator));
    pins.dedup();
    pins
}

/// What a required check did about a pinned checker file.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PinOutcome {
    pub path: String,
    pub state: &'static str,
    pub detail: String,
}

/// Compare pinned checker files with the checkout. In CI a changed checker
/// is restored to its activated bytes (the checkout is disposable); locally
/// it is reported and the gate cannot pass.
pub fn enforce_pins(project_root: &Path, pins: &[EvidenceRef], restore: bool) -> Vec<PinOutcome> {
    let mut outcomes = Vec::new();
    for pin in pins.iter().filter(|pin| pin.system == PIN_EVIDENCE) {
        let Some((commit, path)) = pin.locator.split_once(':') else {
            continue;
        };
        let current = fs::read(project_root.join(path))
            .ok()
            .map(|bytes| sha256_file(&bytes));
        let wanted = pin
            .digest
            .as_ref()
            .map(|digest| digest.as_str().to_string());
        if current == wanted {
            outcomes.push(PinOutcome {
                path: path.into(),
                state: "pinned",
                detail: "matches the activated checker".into(),
            });
            continue;
        }
        if !restore {
            outcomes.push(PinOutcome {
                path: path.into(),
                state: "changed",
                detail: format!("differs from the checker activated at {commit}; required checks run the activated version in CI"),
            });
            continue;
        }
        let restored = Command::new("git")
            .args(["show", &format!("{commit}:{path}")])
            .current_dir(project_root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| output.stdout)
            .filter(|bytes| Some(sha256_file(bytes)) == wanted);
        match restored {
            Some(bytes) => {
                let target = project_root.join(path);
                let mode = fs::metadata(&target)
                    .ok()
                    .map(|metadata| metadata.permissions());
                let written = fs::write(&target, &bytes).is_ok();
                if let Some(mode) = mode {
                    let _ = fs::set_permissions(&target, mode);
                }
                outcomes.push(PinOutcome {
                    path: path.into(),
                    state: if written { "restored" } else { "changed" },
                    detail: if written {
                        format!("this change modified the checker; ran the version activated at {commit}")
                    } else {
                        "the activated checker could not be restored".into()
                    },
                });
            }
            None => outcomes.push(PinOutcome {
                path: path.into(),
                state: "changed",
                detail: format!(
                    "the checker activated at {commit} is not available in this checkout"
                ),
            }),
        }
    }
    outcomes
}

fn github_principal(id: u64, display: &str) -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::GithubUser,
        stable_id: id.to_string(),
        display_name: Some(display.to_string()),
    }
}

fn activator_principal(repository_id: u64) -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::GithubActions,
        stable_id: repository_id.to_string(),
        display_name: Some("whetstone-policy workflow".into()),
    }
}

/// Result of proposing: the shared proposal and the manifest to review.
#[derive(Debug, Clone, Serialize)]
pub struct Proposed {
    pub proposal: RecordRef,
    pub manifest_path: String,
    pub manifest: ActivationManifest,
}

/// Record a shared team proposal for `policies` (already in the shared
/// store) and write its activation manifest into the working tree.
pub fn propose(
    project_root: &Path,
    shared: &RecordStore,
    platform: &dyn Platform,
    policies: &[RecordRef],
    now: &str,
    expires_at: &str,
) -> Result<Proposed, Denial> {
    if policies.is_empty() {
        return Err(Denial::new(
            "nothing_to_propose",
            "no shared policy record is waiting for team activation",
        ));
    }
    let authority = Authority::load(project_root)?;
    let actor = platform.actor()?;
    if !authority.may(actor.github_id, Role::Proposer, now) {
        return Err(Denial::new(
            "unauthorized_proposer",
            format!(
                "{} ({}) may not propose under authority revision {}",
                actor.login, actor.github_id, authority.revision
            ),
        ));
    }
    let repository_id = platform.repository_id()?;
    if repository_id != authority.repository_id {
        return Err(Denial::new(
            "repository_mismatch",
            "the authority file names another repository",
        ));
    }
    let records = shared
        .all_records()
        .map_err(|error| Denial::new("storage", error.to_string()))?;
    for policy in policies {
        if !records
            .iter()
            .any(|record| record.reference().ok().as_ref() == Some(policy))
        {
            return Err(Denial::new(
                "digest_mismatch",
                format!("{} is not in the shared database", reference_text(policy)),
            ));
        }
    }
    let base = team_active(&records).digest();
    let texts = policies.iter().map(reference_text).collect::<Vec<_>>();
    let payload = digest_of(&texts);
    let scope = records
        .iter()
        .find(|record| record.reference().ok().as_ref() == policies.first())
        .map(|record| record.scope.clone())
        .ok_or_else(|| Denial::new("digest_mismatch", "the first policy record is missing"))?;
    let suffix = &payload["sha256:".len()..]["sha256:".len() - 7..24];
    let principal = github_principal(actor.github_id, &actor.login);
    let proposal = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(format!("proposal.team_{suffix}"))
            .map_err(|error| Denial::new("manifest_invalid", error.to_string()))?,
        revision: 1,
        scope: scope.clone(),
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: principal,
            recorded_at: now.into(),
            sources: vec![EvidenceRef {
                system: "whetstone_push".into(),
                locator: format!("github:{repository_id}"),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: format!("propose:{payload}:{base}"),
        body: RecordBody::Proposal(Proposal {
            state: ProposalState::Shared,
            title: format!("Team policy: {} record(s) for activation", policies.len()),
            rationale: format!(
                "Rationale: proposed by {} for independent team review.",
                actor.login
            ),
            proposed_records: policies.to_vec(),
            binding: Some(ProposalBinding {
                repository_id,
                payload_digest: ContentDigest::new(payload.clone())
                    .map_err(|error| Denial::new("manifest_invalid", error.to_string()))?,
                base_active_digest: ContentDigest::new(base.clone())
                    .map_err(|error| Denial::new("manifest_invalid", error.to_string()))?,
                authority_revision: authority.revision,
                expires_at: expires_at.into(),
            }),
        }),
    };
    let reference = shared
        .append(&proposal, None)
        .map_err(|error| Denial::new("storage", error.to_string()))?;
    let manifest = ActivationManifest {
        schema: MANIFEST_SCHEMA.into(),
        repository_id,
        proposal_id: reference.id.as_str().into(),
        proposal_revision: reference.revision,
        proposal_digest: reference.digest.as_str().into(),
        payload_digest: payload,
        base_active_digest: base,
        scope: vec![scope.project.clone()],
        authority_revision: authority.revision,
        expires_at: expires_at.into(),
        records: texts,
    };
    let relative = format!("{PROPOSALS_DIR}/{}.json", reference.id.as_str());
    let path = project_root.join(&relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| Denial::new("storage", error.to_string()))?;
    }
    fs::write(
        &path,
        serde_json::to_string_pretty(&manifest)
            .map_err(|error| Denial::new("storage", error.to_string()))?
            + "\n",
    )
    .map_err(|error| Denial::new("storage", error.to_string()))?;
    Ok(Proposed {
        proposal: reference,
        manifest_path: relative,
        manifest,
    })
}

/// What an activation appended.
#[derive(Debug, Clone, Serialize)]
pub struct Activated {
    pub decision: RecordRef,
    pub activations: Vec<RecordRef>,
    pub reviewer_id: u64,
    pub pull_request: u64,
    pub replay: bool,
}

/// Verify a merged manifest against the platform and activate it.
pub fn activate(
    project_root: &Path,
    shared: &RecordStore,
    platform: &dyn Platform,
    manifest_path: &str,
    now: &str,
) -> Result<Activated, Denial> {
    let relative = manifest_path.trim_start_matches("./");
    if relative.contains("..") || !relative.starts_with(PROPOSALS_DIR) {
        return Err(Denial::new(
            "manifest_invalid",
            format!("activation manifests live under {PROPOSALS_DIR}/"),
        ));
    }
    let bytes = fs::read(project_root.join(relative))
        .map_err(|error| Denial::new("manifest_invalid", format!("{relative}: {error}")))?;
    let manifest: ActivationManifest = serde_json::from_slice(&bytes)
        .map_err(|error| Denial::new("manifest_invalid", format!("{relative}: {error}")))?;
    let authority = Authority::load(project_root)?;
    let records = shared
        .all_records()
        .map_err(|error| Denial::new("storage", error.to_string()))?;
    let proposal = records
        .iter()
        .find(|record| {
            record.id.as_str() == manifest.proposal_id
                && record.revision == manifest.proposal_revision
        })
        .cloned()
        .ok_or_else(|| {
            Denial::new(
                "digest_mismatch",
                format!("{} is not in the shared database", manifest.proposal_id),
            )
        })?;
    let proposal_ref = proposal
        .reference()
        .map_err(|error| Denial::new("digest_mismatch", error.to_string()))?;
    let RecordBody::Proposal(body) = &proposal.body else {
        return Err(Denial::new(
            "manifest_invalid",
            "the manifest does not name a proposal",
        ));
    };
    let binding = body.binding.clone().ok_or_else(|| {
        Denial::new(
            "manifest_invalid",
            "the proposal is not a shared, bound proposal",
        )
    })?;
    let texts = body
        .proposed_records
        .iter()
        .map(reference_text)
        .collect::<Vec<_>>();
    let stored_payload = digest_of(&texts);
    let mut listed = manifest.records.clone();
    let mut proposed = texts.clone();
    listed.sort();
    proposed.sort();
    if listed != proposed {
        return Err(Denial::new(
            "digest_mismatch",
            "the manifest lists different records than the proposal",
        ));
    }
    let present = body.proposed_records.iter().all(|reference| {
        records
            .iter()
            .any(|record| record.reference().ok().as_ref() == Some(reference))
    });
    let stored_payload = if present {
        stored_payload
    } else {
        "sha256:missing".into()
    };
    let proposer_id = (proposal.owner.kind == PrincipalKind::GithubUser)
        .then(|| proposal.owner.stable_id.parse::<u64>().ok())
        .flatten()
        .ok_or_else(|| {
            Denial::new(
                "missing_identity",
                "the proposal owner is not a GitHub user",
            )
        })?;
    // Replay: this exact proposal is already activated.
    if let Some(decision) = records.iter().find(|record| {
        matches!(&record.body, RecordBody::Decision(decision) if decision.proposal == proposal_ref)
    }) {
        let activations = records
            .iter()
            .filter(|record| matches!(&record.body, RecordBody::Activation(activation) if activation.proposal == proposal_ref))
            .filter_map(|record| record.reference().ok())
            .collect::<Vec<_>>();
        let reviewer_id = match &decision.body {
            RecordBody::Decision(decision) => decision.reviewer.stable_id.parse().unwrap_or(0),
            _ => 0,
        };
        return Ok(Activated {
            decision: decision.reference().map_err(|error| Denial::new("storage", error.to_string()))?,
            activations,
            reviewer_id,
            pull_request: 0,
            replay: true,
        });
    }
    let commit = git(project_root, &["log", "-1", "--format=%H", "--", relative])
        .filter(|commit| !commit.is_empty())
        .ok_or_else(|| {
            Denial::new(
                "manifest_not_reviewed",
                format!("{relative} is not committed"),
            )
        })?;
    let merge = platform.merge_evidence(&commit)?;
    let protection = platform.protection(&merge.base_ref)?;
    let active = team_active(&records);
    let verified = verify_activation(&ActivationInputs {
        manifest: &manifest,
        manifest_path: relative,
        authority: &authority,
        merge: &merge,
        protection: &protection,
        proposer_id,
        stored_proposal_digest: proposal_ref.digest.as_str(),
        stored_payload_digest: &stored_payload,
        current_active_digest: &active.digest(),
        now,
    })?;
    let activator = activator_principal(manifest.repository_id);
    let reviewer = PrincipalRef {
        kind: PrincipalKind::GithubUser,
        stable_id: verified.reviewer_id.to_string(),
        display_name: None,
    };
    let pull_locator = format!(
        "{}#{}@{}",
        manifest.repository_id, verified.pull_request, verified.head_sha
    );
    let provenance = |recorded_by: PrincipalRef| Provenance {
        kind: ProvenanceKind::HumanAuthored,
        recorded_by,
        recorded_at: now.into(),
        sources: vec![
            EvidenceRef {
                system: PULL_REQUEST_EVIDENCE.into(),
                locator: pull_locator.clone(),
                digest: None,
            },
            EvidenceRef {
                system: MERGE_EVIDENCE.into(),
                locator: verified.merge_commit_sha.clone(),
                digest: None,
            },
        ],
        authority: ProvenanceAuthority::IndependentlyApproved,
    };
    let suffix = &proposal_ref.digest.as_str()["sha256:".len()..][..24];
    let decision = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(format!("decision.team_{suffix}"))
            .map_err(|error| Denial::new("storage", error.to_string()))?,
        revision: 1,
        scope: proposal.scope.clone(),
        owner: reviewer.clone(),
        provenance: provenance(activator.clone()),
        supersedes: None,
        idempotency_key: format!("decide:{}", reference_text(&proposal_ref)),
        body: RecordBody::Decision(Decision {
            proposal: proposal_ref.clone(),
            proposal_binding: binding.clone(),
            verdict: DecisionVerdict::Accept,
            reviewer,
            rationale: format!(
                "Approved in pull request #{} on {} under authority revision {}.",
                verified.pull_request, verified.head_sha, authority.revision
            ),
            decided_at: now.into(),
        }),
    };
    let decision_ref = decision
        .reference()
        .map_err(|error| Denial::new("storage", error.to_string()))?;
    let last_sequence = records
        .iter()
        .filter_map(|record| match &record.body {
            RecordBody::Activation(body) => Some(body.activation_sequence),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut activations = Vec::new();
    for (offset, policy) in (1_u64..).zip(&body.proposed_records) {
        let sequence = last_sequence + offset;
        let policy_record = records
            .iter()
            .find(|record| record.reference().ok().as_ref() == Some(policy))
            .ok_or_else(|| Denial::new("digest_mismatch", "a proposed record is missing"))?;
        let id = RecordId::new(format!("activation.{}", policy.id.as_str()))
            .or_else(|_| {
                RecordId::new(format!(
                    "activation.{}",
                    &policy.digest.as_str()["sha256:".len()..][..32]
                ))
            })
            .map_err(|error| Denial::new("storage", error.to_string()))?;
        let previous = records
            .iter()
            .filter(|record| record.id == id)
            .max_by_key(|record| record.revision);
        activations.push(AgreementRecord {
            schema_version: SCHEMA_VERSION_V1,
            id,
            revision: previous.map_or(1, |record| record.revision + 1),
            scope: proposal.scope.clone(),
            owner: activator.clone(),
            provenance: provenance(activator.clone()),
            supersedes: previous.and_then(|record| record.reference().ok()),
            idempotency_key: format!(
                "activate:{}:{}",
                reference_text(&proposal_ref),
                reference_text(policy)
            ),
            body: RecordBody::Activation(Activation {
                proposal: proposal_ref.clone(),
                acceptance_decision: decision_ref.clone(),
                policy: policy.clone(),
                binding: binding.clone(),
                activation_sequence: sequence,
                activated_at: now.into(),
                pinned_checkers: pins_for(project_root, policy_record, &verified.merge_commit_sha),
            }),
        });
    }
    let mut requests = vec![AppendRequest {
        record: &decision,
        expected_revision: None,
    }];
    for activation in &activations {
        requests.push(AppendRequest {
            record: activation,
            expected_revision: activation
                .supersedes
                .as_ref()
                .map(|previous| previous.revision),
        });
    }
    let appended = shared
        .append_batch(&requests)
        .map_err(|error| Denial::new("storage", error.to_string()))?;
    Ok(Activated {
        decision: appended[0].clone(),
        activations: appended[1..].to_vec(),
        reviewer_id: verified.reviewer_id,
        pull_request: verified.pull_request,
        replay: false,
    })
}

/// Where the manifest for a proposal would be written.
pub fn manifest_path(project_root: &Path, proposal: &RecordId) -> PathBuf {
    project_root
        .join(PROPOSALS_DIR)
        .join(format!("{}.json", proposal.as_str()))
}
