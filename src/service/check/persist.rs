//! Receipts a check records: one per gate run.

use super::*;

/// Owner acceptance of the exact rule revision is the execution grant: the
/// manifest digest is the accepted rule's content digest.
pub(super) fn gate_execution_receipt(
    selected: &SelectedRule,
    outcome: &GateOutcome,
) -> crate::execution::ExecutionReceipt {
    use crate::execution::{CapturedOutput, ExecutionReceipt, ExecutionState, LimitEvidence};
    let manifest = &selected.reference.digest;
    ExecutionReceipt {
        schema: crate::execution::RECEIPT_SCHEMA.into(),
        checker_id: selected.id.as_str().into(),
        checker_version: format!("v{}", selected.reference.revision),
        manifest_digest: manifest.as_str().into(),
        execution_grant_reference: Some(format!("accepted-rule:{}", manifest.as_str())),
        authority_revision: Some(selected.reference.revision),
        authority_checked_at: Some(utc_now()),
        state: match outcome.state {
            VerificationAxis::Pass => ExecutionState::Success,
            VerificationAxis::Fail => ExecutionState::Violated,
            VerificationAxis::Unknown => ExecutionState::Unknown,
        },
        reason_code: match outcome.state {
            VerificationAxis::Pass => "gate_passed",
            VerificationAxis::Fail => "gate_failed",
            VerificationAxis::Unknown => "gate_unknown",
        }
        .into(),
        summary: outcome.summary.clone(),
        // Content checks run in-process; they are trusted like the scanner.
        process_started: true,
        executable_digest: outcome
            .program_digest
            .clone()
            .or_else(|| Some(format!("in-process:{}", selected.rule.enforcer.kind()))),
        consumed_configurations: Vec::new(),
        exit_code: outcome.exit_code,
        elapsed_ms: outcome.elapsed_ms,
        stdout: CapturedOutput::default(),
        stderr: CapturedOutput::default(),
        limits: LimitEvidence {
            timeout_ms: crate::gates::DEFAULT_GATE_TIMEOUT.as_millis() as u64,
            stdout_bytes: 2 * 1024 * 1024,
            stderr_bytes: 2 * 1024 * 1024,
            timeout_mechanism: "wall-clock bound with process-group termination".into(),
            output_mechanism: "bounded capture; overflow is unknown".into(),
            filesystem_network_memory_process_limits:
                "not sandboxed: repository-scoped working directory and allowlisted environment only"
                    .into(),
        },
        process_group_cleanup: "unix process group".into(),
        isolation_caveat: "Rule commands run as the local user without an OS sandbox.".into(),
    }
}

pub(super) fn gate_finding(selected: &SelectedRule, failure: &ArtifactFailure) -> Finding {
    let (file, line) = match failure.location.rsplit_once(':') {
        Some((file, line))
            if line.chars().all(|character| character.is_ascii_digit())
                && !file.contains(' ')
                && !file.starts_with('/')
                && !file.contains("..") =>
        {
            (Some(file.to_string()), line.parse::<u32>().ok())
        }
        _ if !failure.location.contains(' ')
            && !failure.location.starts_with('/')
            && !failure.location.contains("..")
            && failure.location.contains('.') =>
        {
            (Some(failure.location.clone()), None)
        }
        _ => (None, None),
    };
    Finding {
        file,
        line,
        column: None,
        rule_id: selected.id.as_str().into(),
        observed: failure.message.clone(),
        expected: selected.rule.statement.clone(),
        rationale: selected.rule.rationale.clone(),
        repair_direction: "Repair within the current task scope; never weaken the rule.".into(),
        permitted_next_action: format!("repair, then wh check --rule {}", selected.id.as_str()),
        verification_command: format!("wh check --rule {}", selected.id.as_str()),
    }
}

pub(super) fn persist_gate_receipt(
    layout: &ProjectLayout,
    run_id: &str,
    selected: &SelectedRule,
    outcome: &GateOutcome,
    fingerprint: &str,
    checked_at: &str,
) -> Result<Option<RecordRef>, StorageError> {
    if !layout.private_store_exists() {
        return Ok(None);
    }
    let repository = crate::storage::initialize_private(layout)?;
    let idempotency_key = format!("gate:{run_id}:{}", selected.id.as_str());
    if let Some(existing) = repository.by_idempotency_key(&idempotency_key)? {
        return existing.reference().map(Some).map_err(StorageError::Domain);
    }
    let suffix = key_suffix(&idempotency_key, 24);
    let verification = outcome.state;
    let freshness = if verification == VerificationAxis::Pass && outcome.evidence.is_empty() {
        Freshness::Missing
    } else {
        Freshness::Fresh
    };
    let verification = if freshness == Freshness::Missing {
        VerificationAxis::Unknown
    } else {
        verification
    };
    let evidence = with_head(layout, outcome.evidence.clone());
    let record = receipt_record(
        layout,
        format!("verification.gate_{suffix}"),
        idempotency_key,
        "whetstone:gate-runner",
        ProvenanceKind::DeterministicCheck,
        checked_at,
        EvidenceRef {
            system: crate::proof::EVIDENCE_SYSTEM.into(),
            locator: run_id.into(),
            digest: None,
        },
        RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: format!(
                    "{}{}",
                    crate::proof::GATE_SUBJECT_PREFIX,
                    selected.id.as_str()
                ),
                revision: Some(fingerprint.into()),
            },
            code_digest: ContentDigest::new(if fingerprint.starts_with("sha256:") {
                fingerprint.to_string()
            } else {
                digest_bytes(fingerprint.as_bytes()).as_str().to_string()
            })
            .map_err(StorageError::Domain)?,
            policy_state: PolicyStateSnapshot {
                accepted: Some(selected.reference.clone()),
                required: None,
                installed: None,
                experimental: None,
            },
            verification,
            authorization: crate::domain::AuthorizationAxis::Unknown,
            freshness,
            checked_at: checked_at.into(),
            related_records: vec![selected.reference.clone()],
            evidence,
        }),
    )
    .map_err(StorageError::Domain)?;
    repository.append(&record, None).map(Some)
}
