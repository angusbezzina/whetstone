//! `wh check`: run every applicable rule the cheapest reliable way and
//! return actionable findings, or record a review attestation, a brief or a
//! raised hand.
//!
//! Modes: the staged index (pre-commit: mechanical content checks only), the
//! change since a base (pre-push and CI), everything, or a sweep of every
//! mapped feature. Mechanical enforcers run through `crate::gates`, question
//! enforcers through `crate::judgment`, and review enforcers are satisfied by
//! attestations bound to the commit and rule revision.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agreement::AgreementState;
use crate::check;
use crate::domain::{
    AgreementRecord, Attestation, AttestationVerdict, Brief, ContentDigest, Enforcer,
    EnforcerFamily, EvidenceRef, ExternalRef, ExternalSystem, Feature, Freshness, HandRaise,
    HandRaiseTrigger, HandTrigger, JudgmentOutcome, JudgmentReceipt, PolicyStateSnapshot,
    ProvenanceKind, RecordBody, RecordId, RecordRef, Rule, Strength, VerificationAxis,
    VerificationReceipt,
};
use crate::gates::{BriefView, FileSet, GateOutcome};
use crate::judgment::{self, Answer, UnitSource};
use crate::proof::ArtifactFailure;
use crate::storage::{ProjectLayout, RecordStore, StorageError, StoreKind};
use crate::verification::{
    self, AttestationState, EvidencePointer, Finding, RequirementKind, TrustedEvidenceSet,
    VerificationEvidence, VerificationPlan, VerificationRequirement,
};

use super::record::{
    append_private, array_len, compute_check_snapshot, count, digest_bytes, digest_json,
    drive_evidence, key_suffix, persist_verification_receipt, receipt_record,
    record_maintain_outcome, scan_findings, service_state, with_head, DriveBinding,
};
use super::{
    bounded_input, bounded_list, bounded_request_id, domain_response, load_records, storage_error,
    unix_now, unknown_response, utc_now, AttestRequest, BriefRequest, CheckRequest, GateMode,
    HandRequest, RequiredSnapshot, ServiceEvidence, ServiceResponse, ServiceState,
};

/// The repository's own privacy settings, applied to every question.
pub const PRIVACY_RELATIVE: &str = "whetstone/privacy.json";

mod persist;
mod question;
mod review;

use persist::*;
use question::*;
use review::*;

/// An in-force rule chosen for this check.
pub(crate) struct SelectedRule {
    pub(crate) id: RecordId,
    pub(crate) reference: RecordRef,
    pub(crate) rule: Rule,
}

#[derive(Default)]
pub(crate) struct Selection {
    pub(crate) rules: Vec<SelectedRule>,
    pub(crate) rule_ids: BTreeSet<String>,
    pub(crate) features: BTreeMap<RecordId, Feature>,
    pub(crate) skipped_drafts: Vec<String>,
    pub(crate) changed_paths: Vec<String>,
    pub(crate) features_affected: Vec<Value>,
}

fn invalid(summary: &str) -> ServiceResponse {
    unknown_response("check", "invalid-request".into(), summary.into())
}

pub(super) fn check(mut request: CheckRequest) -> ServiceResponse {
    if request.paths.len() > 64 || request.rules.len() > 64 || request.features.len() > 64 {
        return invalid("A check accepts at most 64 paths, 64 rule filters and 64 features.");
    }
    if request
        .paths
        .iter()
        .any(|path| path.as_os_str().to_string_lossy().len() > 4096)
        || request
            .rules
            .iter()
            .chain(request.features.iter())
            .any(|rule| rule.is_empty() || rule.len() > 256)
        || request.language.as_ref().is_some_and(|language| {
            language.is_empty()
                || language.len() > 32
                || !language
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
        || request
            .timeout_seconds
            .is_some_and(|seconds| seconds == 0 || seconds > 3_600)
    {
        return invalid("A check input exceeds its bound or contains an invalid language, rule, feature or timeout.");
    }
    if !request.steps.is_empty()
        && (request.features.len() != 1
            || request.steps.len() > 32
            || request
                .steps
                .iter()
                .any(|step| step.trim().is_empty() || step.len() > 1_000))
    {
        return invalid("Change-specific steps need exactly one --feature, at most 32 steps, each 1 to 1000 characters.");
    }
    // CI sends the all-zero sha for a branch's first push: no base.
    if request
        .base
        .as_deref()
        .is_some_and(|base| !base.is_empty() && base.chars().all(|character| character == '0'))
    {
        request.base = None;
    }
    if request.base.is_some() && request.gate_mode == GateMode::All {
        request.gate_mode = GateMode::Changed;
    }
    let project = match request.project_dir.canonicalize() {
        Ok(project) => project,
        Err(error) => {
            return unknown_response(
                "check",
                request.request_id.unwrap_or_else(|| "check".into()),
                format!("Project path is unavailable: {error}"),
            )
        }
    };
    let request_id = match bounded_request_id(
        request.request_id.clone(),
        format!(
            "check-{:x}",
            Sha256::digest(project.to_string_lossy().as_bytes())
        )[..22]
            .to_string(),
    ) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response("check", "invalid-request-id".into(), summary),
    };
    let layout = ProjectLayout::resolve(&project).ok();
    if let Some(layout) = layout.as_ref() {
        if let Some(outcome) = request.maintain_outcome {
            return record_maintain_outcome(
                layout,
                request_id,
                outcome,
                request.maintain_evidence.as_deref(),
            );
        }
        if let Some(attest) = request.attest.clone() {
            return record_attestation(layout, request_id, &attest);
        }
        if let Some(brief) = request.brief.clone() {
            return record_brief(layout, request_id, &brief);
        }
        if let Some(hand) = request.raise_hand.clone() {
            return raise_hand(layout, request_id, &hand);
        }
    } else if request.attest.is_some() || request.brief.is_some() || request.raise_hand.is_some() {
        return unknown_response(
            "check",
            request_id,
            "Recording an attestation, brief or raised hand needs a Git repository.".into(),
        );
    }
    // CI enforces what the team accepted and shared, never a contributor's
    // private drafts; a local check sees both stores.
    let agreement = match layout.as_ref().map(load_records) {
        Some(Ok(loaded)) if request.ci => loaded
            .shared
            .clone()
            .filter(|shared| !shared.is_empty())
            .map(AgreementState::from_records),
        Some(Ok(loaded)) if loaded.exists() => Some(AgreementState::from_records(loaded.union().0)),
        Some(Err(error)) => return storage_error("check", request_id, error),
        _ => None,
    };
    if request.ci && agreement.is_none() {
        return unknown_response(
            "check",
            request_id,
            "The CI check enforces the team's shared rules, and this checkout has none; run bd bootstrap, or share accepted rules with wh push.".into(),
        );
    }
    if let Some(pushed) = request.pushed.as_deref() {
        if let Some(reason) = pushed_mismatch(&project, pushed) {
            return unknown_response("check", request_id, reason);
        }
    }
    let staged = request.gate_mode == GateMode::Staged;
    let files = match request.gate_mode {
        GateMode::Staged => match FileSet::staged(&project) {
            Ok(files) => files,
            Err(error) => {
                return unknown_response(
                    "check",
                    request_id,
                    format!("The staged index could not be read: {error}"),
                )
            }
        },
        _ => FileSet::default(),
    };
    let selection = match select_rules(agreement.as_ref(), &project, &request, &files) {
        Ok(selection) => selection,
        Err(summary) => return unknown_response("check", request_id, summary),
    };
    let files = match request.gate_mode {
        GateMode::Staged => files,
        GateMode::Changed => FileSet::working(&project, &selection.changed_paths),
        _ => FileSet::repository(&project).unwrap_or_default(),
    }
    .retain(|path| !crate::skill::is_generated(path));
    if request.dry_run {
        return dry_run_response(request_id, &request, &selection);
    }
    let scanner_rules = request
        .rules
        .iter()
        .filter(|rule| !selection.rule_ids.contains(rule.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let rules_only_name_rules = !request.rules.is_empty() && scanner_rules.is_empty();
    let requested_paths = if request.paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        request.paths.clone()
    };
    let (scan_paths, _relative_paths, mut snapshot) = match compute_check_snapshot(
        &project,
        &requested_paths,
        request.language.as_deref(),
        &request.rules,
    ) {
        Ok(snapshot) => snapshot,
        Err(error) => return unknown_response("check", request_id, error),
    };
    if !selection.rules.is_empty() {
        snapshot.environment = digest_json(&json!({
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "command_validators": false,
            "owner_accepted_rules": true,
        }));
        snapshot.trust = digest_bytes(
            b"whetstone-trust-v2\0compiled-in-deterministic-scanner\0owner-accepted-rules-bounded-execution",
        );
    }
    // The compiled-in scanner reads the working tree, so it never runs at
    // pre-commit (which must ignore unstaged edits); a feature proof is
    // about that feature, not the repository-wide scan.
    let include_native = !staged
        && !rules_only_name_rules
        && request.features.is_empty()
        && request.gate_mode != GateMode::Sweep;
    let filter = (!scanner_rules.is_empty()).then_some(scanner_rules.as_slice());
    let result = if include_native {
        check::run(check::CheckOptions {
            project_dir: &project,
            rules_dir: None,
            scan_paths: &scan_paths,
            lang_filter: request.language.as_deref(),
            rule_filter: filter,
            execute_command_validators: false,
        })
    } else {
        json!({})
    };
    let violations = count(&result, "violations_count");
    let configuration_issues = count(&result, "config_issues_count");
    let rules_applied = count(&result, "rules_applied");
    let files_scanned = count(&result, "files_scanned");
    let delegated = result
        .get("skipped")
        .and_then(Value::as_array)
        .map_or(0, |items| {
            items
                .iter()
                .filter(|item| item.get("delegated_to").is_some())
                .count() as u64
        });
    // A rule delegated to the linter's verified configuration is enforced
    // there, not missing here; any other skip is incomplete evidence.
    let skipped = array_len(&result, "skipped") - delegated;
    let warnings = array_len(&result, "warnings");
    let native_state = if violations > 0 {
        AttestationState::Violated
    } else if configuration_issues > 0
        || rules_applied == 0
        || files_scanned == 0
        || skipped > 0
        || warnings > 0
    {
        AttestationState::Unknown
    } else {
        AttestationState::Success
    };
    // With accepted rules, a scanner that has no project rules is simply not
    // part of this check rather than an unknown requirement.
    let include_native =
        include_native && (selection.rules.is_empty() || rules_applied > 0 || violations > 0);
    let native_summary = match native_state {
        AttestationState::Success if delegated > 0 => format!(
            "The compiled-in scanner applied {rules_applied} rules to {files_scanned} files without violations; {delegated} rule signals are delegated to the linter configuration (binding verified, enforced by the linter, not run here)."
        ),
        AttestationState::Success => format!(
            "The compiled-in scanner applied {rules_applied} rules to {files_scanned} files without violations."
        ),
        AttestationState::Violated => format!(
            "The compiled-in scanner found {violations} violations; incomplete evidence remains visible in the findings."
        ),
        AttestationState::Unknown => format!(
            "The compiled-in scanner could not establish complete evidence ({configuration_issues} configuration issues, {skipped} skipped checks, {warnings} warnings)."
        ),
        AttestationState::Unavailable => "The compiled-in scanner was unavailable.".into(),
    };
    let findings = scan_findings(&project, &result);
    let scan_digest = digest_json(&result);
    let mut requirements = Vec::new();
    let mut evidence_items = Vec::new();
    if include_native {
        requirements.push(VerificationRequirement {
            id: "whetstone.native-scan".into(),
            kind: RequirementKind::NativeCheck,
            required: true,
            checker_manifest: Some(snapshot.checker_bundle.clone()),
            governing_records: Vec::new(),
            rationale: "Applicable accepted rules require deterministic local verification."
                .into(),
            repair_direction: "Repair reported source or checker configuration without changing policy, tests, or baselines to hide the result.".into(),
            permitted_next_action: "repair within the current task scope, then wh check".into(),
            verification_command: "wh check --json".into(),
            freshness_seconds: 300,
        });
        evidence_items.push(VerificationEvidence::NativeCheck {
            requirement_id: "whetstone.native-scan".into(),
            snapshot: snapshot.clone(),
            state: native_state,
            evidence: EvidencePointer {
                source: "whetstone-compiled-in-scanner".into(),
                locator: "local-project".into(),
                digest: scan_digest.clone(),
            },
            observed_at_unix: unix_now(),
            summary: native_summary,
            findings,
        });
    }
    let fingerprint = match crate::gates::workspace_fingerprint(&project) {
        Ok(value) => value,
        Err(error) if !selection.rules.is_empty() => {
            return unknown_response(
                "check",
                request_id,
                format!("The working tree could not be fingerprinted, so results could not be bound: {error}"),
            )
        }
        Err(_) => String::new(),
    };
    // Each check is its own run: the same workspace can get a different
    // verdict once an attestation, brief or accepted rule changes.
    let started_at = format!("{}:{}", utc_now(), std::process::id());
    let run_id = {
        let digest = digest_bytes(
            format!(
                "{request_id}\0{fingerprint}\0{started_at}\0{}\0{}\0{:?}",
                request.steps.join("\n"),
                selection
                    .rules
                    .iter()
                    .map(|rule| rule.reference.digest.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                request.gate_mode
            )
            .as_bytes(),
        );
        digest.as_str()["sha256:".len().."sha256:".len() + 16].to_string()
    };
    // Change-specific steps extend exactly one feature's drive, and only
    // when a drive rule for that feature is about to run.
    let change_steps = match request.features.as_slice() {
        [feature] if !request.steps.is_empty() => {
            let drives_it = selection.rules.iter().any(|selected| {
                matches!(&selected.rule.enforcer, Enforcer::Drive { feature: driven } if driven.as_str() == feature)
            });
            if !drives_it {
                return unknown_response(
                    "check",
                    request_id,
                    format!("No accepted drive rule proves {feature}, so change-specific steps have nothing to extend; record one with wh change --kind rule."),
                );
            }
            match RecordId::new(feature.clone()) {
                Ok(id) => Some((id, request.steps.clone())),
                Err(error) => return unknown_response("check", request_id, error.to_string()),
            }
        }
        _ => None,
    };
    let head = crate::gates::head_commit(&project);
    // What a review attestation must have reviewed: the index at pre-commit,
    // the pushed commit at pre-push (the tree is clean then), the working
    // tree otherwise.
    let attested = agreement.as_ref().is_some_and(|state| {
        state
            .records()
            .iter()
            .any(|record| matches!(record.body, RecordBody::Attestation(_)))
    });
    let checked_tree = if !attested {
        None
    } else if staged {
        crate::gates::files::index_tree(&project)
    } else if request.pushed.is_some() {
        crate::gates::files::head_tree(&project)
    } else {
        crate::gates::files::worktree_tree(&project)
    };
    // Public surfaces compare with what this commit builds on at pre-commit,
    // and with the last pushed revision otherwise (HEAD when never pushed).
    let surface_base = if staged {
        head.clone()
    } else {
        request
            .base
            .clone()
            .or_else(|| crate::gates::files::last_pushed_revision(&project))
            .or_else(|| head.clone())
    };
    let briefs = agreement.as_ref().map(brief_views).unwrap_or_default();
    let repository_privacy = read_privacy(&project);
    let mut outcomes = Vec::new();
    let mut judgment_records = Vec::new();
    let mut doctor = None;
    let mut not_applicable = Vec::new();
    let mut mutation_runs = Vec::new();
    let mut mutation_receipts = Vec::new();
    if let (Some(layout), false) = (layout.as_ref(), selection.rules.is_empty()) {
        let run_dir = crate::gates::evidence_root(layout).join(&run_id);
        if let Err(error) = fs::create_dir_all(&run_dir) {
            return unknown_response(
                "check",
                request_id,
                format!("The evidence directory could not be created: {error}"),
            );
        }
        if selection
            .rules
            .iter()
            .any(|selected| matches!(selected.rule.enforcer, Enforcer::Drive { .. }))
        {
            doctor = Some(crate::gates::run_doctor(&project, &run_dir));
        }
        let timeout = request.timeout_seconds.map_or(
            crate::gates::DEFAULT_GATE_TIMEOUT,
            std::time::Duration::from_secs,
        );
        // What the content gates and Jev read: the change without
        // Whetstone's own wiring.
        let content_changed = selection
            .changed_paths
            .iter()
            .filter(|path| !crate::skill::is_generated(path))
            .cloned()
            .collect::<Vec<_>>();
        let unit_source = UnitSource {
            project_root: &project,
            files: &files,
            staged,
            base: request.base.as_deref(),
            changed: &content_changed,
        };
        // Mutations run first, so a hollow proof they reveal decides this
        // check, not only the next one.
        if request.mutate {
            for selected in &selection.rules {
                let Enforcer::Drive { feature } = &selected.rule.enforcer else {
                    continue;
                };
                let Some(mapped) = selection.features.get(feature) else {
                    continue;
                };
                let context = crate::gates::GateContext {
                    project_root: &project,
                    run_dir: &run_dir,
                    run_id: &run_id,
                    features: &selection.features,
                    doctor: None,
                    timeout,
                    change_steps: None,
                    files: &files,
                    changed: &selection.changed_paths,
                    surface_base: surface_base.as_deref(),
                    briefs: &briefs,
                };
                for (index, mutation) in mapped.mutations.iter().enumerate() {
                    let run = crate::gates::run_mutation(
                        &context,
                        &selected.id,
                        &selected.rule,
                        feature,
                        mutation,
                        index,
                    );
                    if !request.ci {
                        match persist_mutation_receipt(layout, &run_id, index, selected, &run) {
                            Ok(Some(reference)) => mutation_receipts.push(reference),
                            Ok(None) => {}
                            Err(error) => return storage_error("check", request_id, error),
                        }
                    }
                    mutation_runs.push(run);
                }
            }
        }
        // Doctor runs before the first drive and again after any failed
        // drive; once it fails, the remaining drives are skipped, never run
        // against an instance nobody checked.
        let mut halted: Option<String> = None;
        for selected in &selection.rules {
            let rule = &selected.rule;
            let is_drive = matches!(rule.enforcer, Enforcer::Drive { .. });
            let family =
                if rule.privacy.local_only && rule.enforcer.family() == EnforcerFamily::Question {
                    EnforcerFamily::Review
                } else {
                    rule.enforcer.family()
                };
            let (outcome, requirement_kind, evidence) = match family {
                EnforcerFamily::Mechanical => {
                    let context = crate::gates::GateContext {
                        project_root: &project,
                        run_dir: &run_dir,
                        run_id: &run_id,
                        features: &selection.features,
                        doctor: doctor.as_ref(),
                        timeout,
                        change_steps: change_steps
                            .as_ref()
                            .map(|(id, steps)| (id, steps.as_slice())),
                        files: &files,
                        changed: &content_changed,
                        surface_base: surface_base.as_deref(),
                        briefs: &briefs,
                    };
                    let mut outcome = match (&halted, is_drive) {
                        (Some(reason), true) => {
                            crate::gates::skipped_outcome(&selected.id, rule, reason)
                        }
                        _ => crate::gates::run_gate(&context, &selected.id, rule),
                    };
                    // A proof that fails and then passes on one retry is
                    // flaky: quarantined with the reason, never a pass.
                    if is_drive && outcome.state == VerificationAxis::Fail && halted.is_none() {
                        let first = outcome.summary.clone();
                        let retry = crate::gates::run_gate(&context, &selected.id, rule);
                        if retry.state == VerificationAxis::Pass {
                            outcome = retry.unknown(format!(
                                "Flaky: the proof failed ({first}) and passed on one retry; it is quarantined until it passes first time."
                            ));
                            outcome.evidence.push(EvidenceRef {
                                system: crate::gates::QUARANTINE_EVIDENCE.into(),
                                locator: first.chars().take(300).collect(),
                                digest: None,
                            });
                        }
                    }
                    // A flag that is the owner's call passes once the owner
                    // attested this exact rule revision for the change.
                    if outcome.state == VerificationAxis::Fail
                        && rule.raises_hand_on(HandRaiseTrigger::Flag)
                    {
                        if let Some(attestation) = current_attestation(
                            agreement.as_ref(),
                            selected,
                            head.as_deref(),
                            checked_tree.as_deref(),
                        ) {
                            if attestation.verdict == AttestationVerdict::Pass {
                                outcome.state = VerificationAxis::Pass;
                                outcome.raise_hand = false;
                                outcome.summary = format!(
                                    "{} The owner approved it: {} attested at {} ({}).",
                                    outcome.summary,
                                    attestation.reviewer.label(),
                                    &attestation.commit[..attestation.commit.len().min(12)],
                                    attestation.notes
                                );
                                outcome.evidence.push(EvidenceRef {
                                    system: "whetstone_attestation".into(),
                                    locator: format!(
                                        "{}@{}",
                                        attestation.reviewer.label(),
                                        attestation.attested_at
                                    ),
                                    digest: None,
                                });
                            }
                        }
                    }
                    // A proof that survived its recorded mutation is hollow.
                    if let (Enforcer::Drive { feature }, VerificationAxis::Pass) =
                        (&rule.enforcer, outcome.state)
                    {
                        let fresh = mutation_runs
                            .iter()
                            .find(|run: &&crate::gates::MutationOutcome| {
                                run.feature == feature.as_str() && run.verdict == "hollow"
                            })
                            .map(|run| {
                                format!(
                                    "it still passed with \"{}\" applied; strengthen its drive steps",
                                    run.mutation
                                )
                            });
                        if let Some(detail) = fresh.or_else(|| {
                            agreement
                                .as_ref()
                                .and_then(|state| hollow_proof(state, feature))
                        }) {
                            outcome = outcome.unknown(format!("Hollow proof: {detail}"));
                        }
                    }
                    if is_drive
                        && halted.is_none()
                        && outcome.state != VerificationAxis::Pass
                        && !outcome
                            .summary
                            .starts_with(crate::gates::UNREACHABLE_PREFIX)
                        && doctor.as_ref().is_some_and(|result| result.ok)
                    {
                        let again = crate::gates::run_doctor(&project, &run_dir);
                        if !again.ok {
                            halted = Some(format!(
                                "Skipped: doctor failed after {} ({}); fix the instance and sweep again.",
                                selected.id.as_str(),
                                again.detail
                            ));
                        }
                        doctor = Some(again);
                    }
                    let evidence = VerificationEvidence::Execution {
                        requirement_id: format!("gate:{}", selected.id.as_str()),
                        snapshot: snapshot.clone(),
                        receipt: Box::new(gate_execution_receipt(selected, &outcome)),
                        evidence: EvidencePointer {
                            source: crate::proof::EVIDENCE_SYSTEM.into(),
                            locator: outcome.evidence.last().map_or_else(
                                || run_id.clone(),
                                |evidence| evidence.locator.clone(),
                            ),
                            digest: outcome
                                .evidence
                                .last()
                                .and_then(|evidence| evidence.digest.clone())
                                .unwrap_or_else(|| digest_bytes(outcome.summary.as_bytes())),
                        },
                        observed_at_unix: unix_now(),
                        findings: outcome
                            .failures
                            .iter()
                            .map(|failure| gate_finding(selected, failure))
                            .collect(),
                    };
                    (outcome, RequirementKind::NativeCheck, Some(evidence))
                }
                EnforcerFamily::Question => {
                    let run = run_question(
                        layout,
                        &run_dir,
                        &run_id,
                        selected,
                        &unit_source,
                        &repository_privacy,
                        head.as_deref(),
                    );
                    judgment_records.extend(run.receipts);
                    if run.units == 0 {
                        not_applicable.push(selected.id.as_str().to_string());
                        outcomes.push(run.outcome);
                        continue;
                    }
                    let attested = current_attestation(
                        agreement.as_ref(),
                        selected,
                        head.as_deref(),
                        checked_tree.as_deref(),
                    );
                    let evidence = match attested {
                        Some(attestation) => attestation_evidence(selected, attestation, &snapshot),
                        None => VerificationEvidence::Judgment {
                            requirement_id: format!("gate:{}", selected.id.as_str()),
                            snapshot: snapshot.clone(),
                            outcome: run.aggregate,
                            evidence: EvidencePointer {
                                source: "whetstone_judgment".into(),
                                locator: format!("{run_id}:{}", selected.id.as_str()),
                                digest: digest_bytes(run.outcome.summary.as_bytes()),
                            },
                            observed_at_unix: unix_now(),
                            summary: run.outcome.summary.clone(),
                            findings: run
                                .outcome
                                .failures
                                .iter()
                                .map(|failure| gate_finding(selected, failure))
                                .collect(),
                        },
                    };
                    // Shadow answers are recorded and shown, never enforced.
                    let evidence = (!rule.in_shadow()).then_some(evidence);
                    (run.outcome, RequirementKind::Judgment, evidence)
                }
                EnforcerFamily::Review => {
                    let mut outcome = GateOutcome::new(&selected.id, rule);
                    // A review scoped to some paths is owed only by a change
                    // that touches them.
                    let partial = matches!(request.gate_mode, GateMode::Staged | GateMode::Changed);
                    if partial
                        && !rule.paths.is_empty()
                        && !content_changed.iter().any(|path| rule.applies_to(path))
                    {
                        outcome.state = VerificationAxis::Pass;
                        outcome.summary =
                            "The change touches none of this rule's paths, so no review is owed."
                                .into();
                        not_applicable.push(selected.id.as_str().to_string());
                        outcomes.push(outcome);
                        continue;
                    }
                    let attested = current_attestation(
                        agreement.as_ref(),
                        selected,
                        head.as_deref(),
                        checked_tree.as_deref(),
                    );
                    match attested {
                        Some(attestation) => {
                            outcome.state = match attestation.verdict {
                                AttestationVerdict::Pass => VerificationAxis::Pass,
                                AttestationVerdict::Fail => VerificationAxis::Fail,
                            };
                            outcome.summary = format!(
                                "{} attested {} at {}: {}",
                                attestation.reviewer.label(),
                                match attestation.verdict {
                                    AttestationVerdict::Pass => "pass",
                                    AttestationVerdict::Fail => "fail",
                                },
                                &attestation.commit[..attestation.commit.len().min(12)],
                                attestation.notes
                            );
                            let evidence = attestation_evidence(selected, attestation, &snapshot);
                            (outcome, RequirementKind::Review, Some(evidence))
                        }
                        None => {
                            outcome = outcome.unknown(format!(
                                "No review attestation for this commit: run {} and record it with wh check --attest {} --verdict pass|fail{}. Missing review is not a pass.",
                                match &rule.enforcer {
                                    Enforcer::Review { reviewer } => reviewer.label(),
                                    _ => "/interrogate".into(),
                                },
                                selected.id.as_str(),
                                if rule.privacy.local_only {
                                    " (local-only: Jev is never called for this rule)"
                                } else {
                                    ""
                                }
                            ));
                            outcome.raise_hand = rule.strength == Strength::Must
                                && rule.raises_hand_on(HandRaiseTrigger::Unavailable);
                            (outcome, RequirementKind::Review, None)
                        }
                    }
                }
            };
            let enforced = !(rule.in_shadow() && requirement_kind == RequirementKind::Judgment);
            if enforced {
                let advisory = rule.strength == Strength::Advisory;
                requirements.push(VerificationRequirement {
                    id: format!("gate:{}", selected.id.as_str()),
                    kind: if advisory {
                        RequirementKind::AdvisoryGuidance
                    } else {
                        requirement_kind
                    },
                    required: rule.strength == Strength::Must,
                    checker_manifest: Some(selected.reference.digest.clone()),
                    governing_records: vec![selected.reference.clone()],
                    rationale: rule.rationale.clone(),
                    repair_direction: "Repair within the current task scope; never change or weaken the rule to pass it.".into(),
                    permitted_next_action: format!("repair, then wh check --rule {}", selected.id.as_str()),
                    verification_command: format!("wh check --rule {}", selected.id.as_str()),
                    freshness_seconds: 3_600,
                });
                if let Some(evidence) = evidence {
                    if advisory {
                        evidence_items.push(VerificationEvidence::Advisory {
                            requirement_id: format!("gate:{}", selected.id.as_str()),
                            evidence: EvidencePointer {
                                source: "whetstone_advisory".into(),
                                locator: selected.id.as_str().into(),
                                digest: digest_bytes(outcome.summary.as_bytes()),
                            },
                            summary: outcome.summary.clone(),
                        });
                    } else {
                        evidence_items.push(evidence);
                    }
                }
            }
            outcomes.push(outcome);
        }
    }
    let sweep = (request.gate_mode == GateMode::Sweep).then(|| {
        super::dash::sweep_report(agreement.as_ref(), &selection, &outcomes, layout.as_ref())
    });
    if requirements.is_empty() {
        return nothing_ran(
            request_id,
            &request,
            &selection,
            &outcomes,
            sweep,
            &not_applicable,
            layout.as_ref(),
            &judgment_records,
        );
    }
    let plan = VerificationPlan {
        subject: format!(
            "project:{:x}:check",
            Sha256::digest(project.to_string_lossy().as_bytes())
        ),
        snapshot: snapshot.clone(),
        requirements,
    };
    let evidence = TrustedEvidenceSet {
        items: evidence_items,
    };
    let report = match verification::aggregate(&plan, &evidence, unix_now(), &[]) {
        Ok(report) => report,
        Err(error) => {
            return unknown_response(
                "check",
                request_id,
                format!("Verification evidence could not be aggregated: {error}"),
            )
        }
    };
    let checked_at = utc_now();
    // CI records nothing, and neither does a staged check: the index is not
    // a commit yet, so a receipt would bind to the wrong one (pre-push
    // checks the same content again and records it).
    let records_receipts = !request.ci && request.gate_mode != GateMode::Staged;
    let (receipt_record, gate_receipts, judgment_receipts) = match layout.as_ref() {
        Some(layout) if records_receipts => {
            let native = if include_native {
                match persist_verification_receipt(layout, &request_id, &report, &checked_at) {
                    Ok(reference) => reference,
                    Err(error) => return storage_error("check", request_id, error),
                }
            } else {
                None
            };
            let mut gate_receipts = Vec::new();
            for (selected, outcome) in selection.rules.iter().zip(&outcomes) {
                if outcome.family != "mechanical" {
                    continue;
                }
                let feature_ref = match &selected.rule.enforcer {
                    Enforcer::Drive { feature } => agreement
                        .as_ref()
                        .and_then(|state| state.in_force(feature))
                        .and_then(|record| record.reference().ok()),
                    _ => None,
                };
                match persist_gate_receipt(
                    layout,
                    &run_id,
                    selected,
                    outcome,
                    &fingerprint,
                    &checked_at,
                    &DriveBinding {
                        feature: feature_ref,
                        doctor: doctor.as_ref(),
                    },
                ) {
                    Ok(Some(reference)) => gate_receipts.push(reference),
                    Ok(None) => {}
                    Err(error) => return storage_error("check", request_id, error),
                }
            }
            let judgment_receipts = match persist_all(layout, &judgment_records) {
                Ok(references) => references,
                Err(error) => return storage_error("check", request_id, error),
            };
            (native, gate_receipts, judgment_receipts)
        }
        _ => (None, Vec::new(), Vec::new()),
    };
    let mut state = service_state(report.state);
    let mut summary = report.human_summary();
    if !plan
        .requirements
        .iter()
        .any(|requirement| requirement.required)
    {
        // Only should and advisory rules ran: there is no must rule to hold,
        // so the check itself neither passes nor fails a must; its flags are
        // listed below for repair or labelling.
        state = ServiceState::Success;
        summary =
            "No must rule applies to this check; should and advisory results are listed.".into();
    }
    if let Some(sweep) = &sweep {
        let proven = sweep["tally"]["proven"].as_u64().unwrap_or(0);
        let total = sweep["features"].as_array().map_or(0, Vec::len) as u64;
        if state == ServiceState::Success && proven < total {
            // A sweep promises every mapped feature; anything short of
            // proven leaves the sweep unknown, not green.
            state = ServiceState::Unknown;
        }
        summary = format!(
            "Sweep: {proven} of {total} feature(s) proven, {} failed, {} unreachable, {} skipped. {summary}",
            sweep["tally"]["failed"],
            sweep["tally"]["unreachable"],
            sweep["tally"]["skipped"],
        );
    }
    let flags = outcomes
        .iter()
        .filter(|outcome| {
            outcome.state == VerificationAxis::Fail
                && outcome.strength != Strength::Must
                && !outcome.shadow
        })
        .map(|outcome| outcome.id.clone())
        .collect::<Vec<_>>();
    if !flags.is_empty() {
        summary.push_str(&format!(
            "\n{} should-rule flag(s) to repair, or for the owner to accept or dismiss: {}.",
            flags.len(),
            flags.join(", ")
        ));
    }
    let hands = hand_signals(&outcomes);
    let mut response = ServiceResponse::new(request_id, "check", state, summary);
    response.required_snapshot = Some(RequiredSnapshot {
        code_digest: Some(report.snapshot.code_tree.as_str().into()),
        policy_digest: Some(report.snapshot.policy.as_str().into()),
        checker_digest: Some(report.snapshot.checker_bundle.as_str().into()),
        scope_digest: Some(report.snapshot.scope.as_str().into()),
        environment_digest: Some(report.snapshot.environment.as_str().into()),
        trust_digest: Some(report.snapshot.trust.as_str().into()),
    });
    response.evidence = vec![ServiceEvidence {
        kind: "deterministic_scan".into(),
        locator: "local-project".into(),
        digest: Some(report.receipt_id.as_str().into()),
    }];
    for outcome in &outcomes {
        for evidence in &outcome.evidence {
            response.evidence.push(ServiceEvidence {
                kind: format!("gate:{}", outcome.id),
                locator: evidence.locator.clone(),
                digest: evidence
                    .digest
                    .as_ref()
                    .map(|digest| digest.as_str().to_string()),
            });
        }
    }
    response.permitted_actions = next_actions(state, &outcomes, &hands);
    response.data = json!({
        "report": report,
        "raw_scan": result,
        "scanner_included": include_native,
        "receipt_persisted": receipt_record.is_some() || !gate_receipts.is_empty() || !judgment_receipts.is_empty(),
        "receipt_record": receipt_record,
        "gate_receipts": gate_receipts,
        "judgment_receipts": judgment_receipts,
        "gates": outcomes,
        "flags": flags,
        "raise_hand": hands,
        "not_applicable": not_applicable,
        "mutations": mutation_runs,
        "mutation_receipts": mutation_receipts,
        "run_id": if selection.rules.is_empty() { None } else { Some(run_id) },
        "evidence_root": layout.as_ref().map(|layout| crate::gates::evidence_root(layout).display().to_string()),
        "doctor": doctor,
        "sweep": sweep,
        "ci": request.ci,
        "selection": selection_json(&request, &selection),
    });
    response
}

fn selection_json(request: &CheckRequest, selection: &Selection) -> Value {
    json!({
        "mode": format!("{:?}", request.gate_mode).to_ascii_lowercase(),
        "rules": selection.rules.iter().map(|rule| rule.id.as_str()).collect::<Vec<_>>(),
        "skipped_drafts": selection.skipped_drafts,
        "changed_paths": selection.changed_paths,
        "base": request.base,
        "features_affected": selection.features_affected,
        "change_steps": request.steps,
    })
}

fn next_actions(state: ServiceState, outcomes: &[GateOutcome], hands: &[Value]) -> Vec<String> {
    let mut actions = match state {
        ServiceState::Success => vec!["handoff the verified result".into()],
        ServiceState::Violated => {
            let failing = outcomes
                .iter()
                .filter(|outcome| outcome.state == VerificationAxis::Fail && !outcome.raise_hand)
                .map(|outcome| format!("repair, then wh check --rule {}", outcome.id))
                .collect::<Vec<_>>();
            if failing.is_empty() {
                vec!["repair within the current task scope, then wh check".into()]
            } else {
                failing
            }
        }
        _ => vec!["restore the rule's checker, driver or evidence, then wh check".into()],
    };
    for hand in hands {
        actions.push(format!(
            "raise a hand: wh check --raise-hand --question \"...\" --tried \"...\" --recommend \"...\" --trigger {} --rule {}",
            hand["trigger"].as_str().unwrap_or("rule_flag"),
            hand["rule"].as_str().unwrap_or_default()
        ));
    }
    actions
}

/// Results that are the owner's call, with the trigger to file them under.
fn hand_signals(outcomes: &[GateOutcome]) -> Vec<Value> {
    outcomes
        .iter()
        .filter(|outcome| outcome.raise_hand && !outcome.shadow)
        .map(|outcome| {
            let trigger =
                if outcome.state == VerificationAxis::Fail && outcome.family == "mechanical" {
                    HandTrigger::RuleFlag
                } else if outcome.summary.contains("low confidence") {
                    HandTrigger::LowConfidence
                } else if outcome.state == VerificationAxis::Fail {
                    HandTrigger::RuleFlag
                } else {
                    HandTrigger::Unavailable
                };
            json!({
                "rule": outcome.id,
                "trigger": trigger.label(),
                "summary": outcome.summary,
            })
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn nothing_ran(
    request_id: String,
    request: &CheckRequest,
    selection: &Selection,
    outcomes: &[GateOutcome],
    sweep: Option<Value>,
    not_applicable: &[String],
    layout: Option<&ProjectLayout>,
    judgments: &[AgreementRecord],
) -> ServiceResponse {
    // Shadow answers are still recorded when nothing else is enforced.
    let recorded = match (layout, request.ci || request.gate_mode == GateMode::Staged) {
        (Some(layout), false) => persist_all(layout, judgments).unwrap_or_default(),
        _ => Vec::new(),
    };
    let shadow = outcomes.iter().filter(|outcome| outcome.shadow).count();
    // Rules are in force, but none runs at pre-commit (question, review and
    // command rules run at pre-push): the commit is not blocked on them.
    let deferred = request.gate_mode == GateMode::Staged
        && selection.rules.is_empty()
        && !selection.rule_ids.is_empty()
        && selection.skipped_drafts.is_empty();
    let summary = if deferred {
        "No fast mechanical rule applies at pre-commit; the other rules in force run at pre-push and in CI."
            .to_string()
    } else if !not_applicable.is_empty() && shadow == 0 && selection.skipped_drafts.is_empty() {
        format!(
            "Nothing to check: no change is in scope for {}.",
            not_applicable.join(", ")
        )
    } else if shadow > 0 {
        format!(
            "Only shadow rules ran ({shadow}): their answers are recorded, not enforced, so nothing passed or failed."
        )
    } else if selection.skipped_drafts.is_empty() {
        "Nothing ran: no scanner rule or in-force rule applies, which is unknown, not a pass."
            .to_string()
    } else {
        format!(
            "Nothing ran: {} is a draft and cannot run until accepted; that is unknown, not a pass.",
            selection.skipped_drafts.join(", ")
        )
    };
    // Shadow answers are not enforced, so a check of shadow rules alone (or
    // of rules the change does not reach) has no must rule to hold: it
    // succeeds, and says why, rather than blocking a push.
    let state = if deferred
        || ((!not_applicable.is_empty() || shadow > 0) && selection.skipped_drafts.is_empty())
    {
        ServiceState::Success
    } else {
        ServiceState::Unknown
    };
    let mut response = ServiceResponse::new(request_id, "check", state, summary);
    response.permitted_actions = if selection.skipped_drafts.is_empty() {
        vec!["record a rule with wh change --kind rule, then accept it".into()]
    } else {
        vec!["accept the draft with wh change --accept <proposal>, then wh check".into()]
    };
    response.data = json!({
        "gates": outcomes,
        "judgment_receipts": recorded,
        "not_applicable": not_applicable,
        "sweep": sweep,
        "ci": request.ci,
        "selection": selection_json(request, selection),
    });
    response
}

fn dry_run_response(
    request_id: String,
    request: &CheckRequest,
    selection: &Selection,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "check",
        ServiceState::NeedsDecision,
        format!(
            "Dry run: {} rule(s) would run; nothing was executed, asked or recorded.",
            selection.rules.len()
        ),
    );
    response.permitted_actions = vec!["repeat without --dry-run to run them".into()];
    response.data = json!({
        "dry_run": true,
        "gates": selection.rules.iter().map(|selected| {
            let (mechanism, command) = crate::gates::mechanism_label(&selected.rule);
            json!({
                "id": selected.id.as_str(),
                "strength": selected.rule.strength.label(),
                "family": selected.rule.enforcer.family().label(),
                "mechanism": mechanism,
                "command": command,
                "shadow": selected.rule.in_shadow(),
                "local_only": selected.rule.privacy.local_only,
                "argv": match &selected.rule.enforcer {
                    Enforcer::Test { command } | Enforcer::Validator { command } => {
                        crate::gates::parse_command(command).ok()
                    }
                    _ => None,
                },
                "timeout_seconds": request.timeout_seconds.unwrap_or(crate::gates::DEFAULT_GATE_TIMEOUT.as_secs()),
            })
        }).collect::<Vec<_>>(),
        "doctor_required": selection.rules.iter().any(|selected| matches!(selected.rule.enforcer, Enforcer::Drive { .. })),
        "environment_allowlist": crate::gates::ENV_ALLOWLIST,
        "selection": selection_json(request, selection),
    });
    response
}

/// Choose the in-force rules this check runs.
pub(crate) fn select_rules(
    agreement: Option<&AgreementState>,
    project: &Path,
    request: &CheckRequest,
    staged: &FileSet,
) -> Result<Selection, String> {
    let mut selection = Selection::default();
    let Some(state) = agreement else {
        if !request.features.is_empty() {
            return Err("No agreement exists, so no feature can be proven yet.".into());
        }
        return Ok(selection);
    };
    for id in state.agreement_ids(|body| matches!(body, RecordBody::Feature(_))) {
        if let Some(record) = state.in_force(&id) {
            if let RecordBody::Feature(feature) = &record.body {
                selection.features.insert(id, feature.clone());
            }
        }
    }
    let rule_ids = crate::proof::rule_ids(state);
    for id in &rule_ids {
        selection.rule_ids.insert(id.as_str().to_string());
    }
    let named = request
        .rules
        .iter()
        .filter(|rule| selection.rule_ids.contains(rule.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut wanted: Option<BTreeSet<String>> = (!named.is_empty()).then_some(named);
    let mut feature_filter = request.features.clone();
    let rule_of = |id: &RecordId| -> Option<Rule> {
        state
            .in_force(id)
            .and_then(|record| record.body.rule_view())
            .map(std::borrow::Cow::into_owned)
    };
    match request.gate_mode {
        GateMode::Staged => {
            selection.changed_paths = staged.paths().map(str::to_owned).collect();
            selection
                .changed_paths
                .extend(staged.deleted.iter().cloned());
            let content = rule_ids
                .iter()
                .filter(|id| rule_of(id).is_some_and(|rule| rule.enforcer.runs_staged()))
                .map(|id| id.as_str().to_string())
                .collect::<BTreeSet<_>>();
            wanted = Some(match wanted {
                Some(named) => named.intersection(&content).cloned().collect(),
                None => content,
            });
        }
        GateMode::Changed => {
            let changed = crate::gates::changed_paths_between(
                project,
                request.base.as_deref(),
                request.pushed.is_none(),
            )?;
            for (id, feature) in &selection.features {
                let touched = changed
                    .iter()
                    .filter(|path| feature.covers_path(path))
                    .cloned()
                    .collect::<Vec<_>>();
                if !touched.is_empty() {
                    feature_filter.push(id.as_str().to_string());
                    selection.features_affected.push(json!({
                        "feature": id.as_str(),
                        "name": feature.name,
                        "paths": touched,
                        "map_review": "Behaviour in these paths changed; confirm the feature entry still describes it, or revise it with wh change --kind feature.",
                    }));
                }
            }
            selection.changed_paths = changed;
            // Repository-wide rules always apply to a change; drive rules
            // apply when the change touches their feature.
            let always = rule_ids
                .iter()
                .filter(|id| {
                    rule_of(id).is_some_and(|rule| !matches!(rule.enforcer, Enforcer::Drive { .. }))
                })
                .map(|id| id.as_str().to_string())
                .collect::<BTreeSet<_>>();
            if feature_filter.is_empty() && always.is_empty() {
                return Ok(selection);
            }
            wanted = Some(match wanted {
                Some(named) => named,
                None => always,
            });
        }
        GateMode::All | GateMode::Sweep => {}
    }
    if !feature_filter.is_empty() {
        let mut ids = wanted.take().unwrap_or_default();
        for feature_id in &feature_filter {
            let id = RecordId::new(feature_id.as_str())
                .map_err(|_| format!("{feature_id} is not a valid feature id."))?;
            let Some(feature) = selection.features.get(&id) else {
                return Err(format!(
                    "Feature {feature_id} is not in force; accept its draft before proving it."
                ));
            };
            ids.extend(
                crate::proof::proving_rules(state, &id, feature)
                    .iter()
                    .map(|rule| rule.as_str().to_string()),
            );
        }
        wanted = Some(ids);
    }
    for id in &rule_ids {
        if wanted
            .as_ref()
            .is_some_and(|wanted| !wanted.contains(id.as_str()))
        {
            continue;
        }
        let Some(record) = state.in_force(id) else {
            selection.skipped_drafts.push(id.as_str().to_string());
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        if request.gate_mode == GateMode::Sweep && !matches!(rule.enforcer, Enforcer::Drive { .. })
        {
            continue;
        }
        let reference = record.reference().map_err(|error| error.to_string())?;
        selection.rules.push(SelectedRule {
            id: id.clone(),
            reference,
            rule: rule.into_owned(),
        });
    }
    if request.gate_mode == GateMode::Sweep {
        let order = |selected: &SelectedRule| match &selected.rule.enforcer {
            Enforcer::Drive { feature } => selection
                .features
                .get(feature)
                .map_or((String::from("~"), u32::MAX), |feature| {
                    (feature.area.clone(), feature.sweep_order)
                }),
            _ => (String::from("~"), u32::MAX),
        };
        let mut keyed = selection
            .rules
            .drain(..)
            .map(|selected| (order(&selected), selected))
            .collect::<Vec<_>>();
        keyed.sort_by(|left, right| left.0.cmp(&right.0));
        selection.rules = keyed.into_iter().map(|(_, selected)| selected).collect();
    }
    if request.gate_mode != GateMode::Staged {
        if let Some(wanted) = wanted {
            for name in wanted {
                if !selection.rule_ids.contains(name.as_str()) {
                    return Err(format!("{name} is not a rule in this agreement."));
                }
            }
        }
    }
    Ok(selection)
}
