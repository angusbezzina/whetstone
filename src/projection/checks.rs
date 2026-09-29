//! The Checks view: the latest run of every rule, by enforcer family.

use super::*;

/// The latest Jev answers for a rule in its most recent run.
pub(super) fn latest_judgments<'a>(
    state: &'a AgreementState,
    id: &RecordId,
) -> Vec<&'a AgreementRecord> {
    let mut by_run = BTreeMap::<String, Vec<&AgreementRecord>>::new();
    for record in state.records() {
        if let RecordBody::Judgment(body) = &record.body {
            if &body.rule.id == id {
                let run = record
                    .idempotency_key
                    .split(':')
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                by_run.entry(run).or_default().push(record);
            }
        }
    }
    by_run
        .into_values()
        .max_by(|left, right| {
            let latest = |records: &[&AgreementRecord]| {
                records
                    .iter()
                    .map(|record| record.provenance.recorded_at.clone())
                    .max()
                    .unwrap_or_default()
            };
            latest(left).cmp(&latest(right))
        })
        .unwrap_or_default()
}

pub(super) fn latest_attestation<'a>(
    state: &'a AgreementState,
    id: &RecordId,
) -> Option<&'a crate::domain::Attestation> {
    state
        .records()
        .iter()
        .filter_map(|record| match &record.body {
            RecordBody::Attestation(body) if &body.rule.id == id => Some(body),
            _ => None,
        })
        .max_by(|left, right| left.attested_at.cmp(&right.attested_at))
}

pub(super) struct RunView {
    pub(super) result: StateLabel,
    pub(super) last_run: Option<String>,
    pub(super) current: bool,
    pub(super) summary: Option<String>,
    pub(super) failures: Vec<Failure>,
    pub(super) evidence: Vec<EvidenceSummary>,
}

pub(super) fn mechanical_run(
    state: &AgreementState,
    input: &ProjectionInput<'_>,
    id: &RecordId,
    shown: &AgreementRecord,
    eligible: bool,
) -> RunView {
    let receipt = latest_gate_receipt(state, id);
    match receipt.map(|record| &record.body) {
        Some(RecordBody::VerificationReceipt(body)) => {
            let bound = shown
                .reference()
                .ok()
                .is_some_and(|reference| body.policy_state.accepted.as_ref() == Some(&reference));
            let same_code = input.fingerprint.is_some()
                && body.subject.revision.as_deref() == input.fingerprint;
            let current = bound && same_code && body.freshness == Freshness::Fresh;
            // The gate artifact is written last; driver evidence may also be JSON.
            let artifact = body
                .evidence
                .iter()
                .rev()
                .find(|evidence| {
                    evidence.system == EVIDENCE_SYSTEM && evidence.locator.ends_with(".json")
                })
                .and_then(|evidence| read_artifact(input.evidence_root, &evidence.locator));
            let failures = artifact
                .as_ref()
                .map(|artifact| {
                    artifact
                        .failures
                        .iter()
                        .take(20)
                        .map(|failure| Failure {
                            location: failure.location.clone(),
                            message: failure.message.clone(),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let quarantined = body
                .evidence
                .iter()
                .any(|evidence| evidence.system == crate::gates::QUARANTINE_EVIDENCE);
            let result = match (body.verification, current) {
                _ if quarantined => StateLabel::new("warn", "quarantined · flaky"),
                (VerificationAxis::Pass, true) => StateLabel::new("pass", "pass"),
                (VerificationAxis::Pass, false) => StateLabel::new("warn", "pass · stale"),
                (VerificationAxis::Fail, true) => {
                    let count: usize = artifact.as_ref().map_or(0, |a| a.failures.len());
                    StateLabel::new(
                        "fail",
                        if count > 0 {
                            format!("fail · {count}")
                        } else {
                            "fail".into()
                        },
                    )
                }
                (VerificationAxis::Fail, false) => StateLabel::new("fail", "fail · stale"),
                (VerificationAxis::Unknown, _) => StateLabel::new("warn", "unknown"),
            };
            RunView {
                result,
                last_run: Some(body.checked_at.clone()),
                current,
                // A quarantined run's artifact is the passing retry; the
                // reason it is not a pass is the first failure.
                summary: body
                    .evidence
                    .iter()
                    .find(|evidence| evidence.system == crate::gates::QUARANTINE_EVIDENCE)
                    .map(|evidence| {
                        format!(
                            "Flaky: the proof failed ({}) and passed on one retry; quarantined until it passes first time.",
                            evidence.locator
                        )
                    })
                    .or_else(|| artifact.as_ref().map(|artifact| artifact.summary.clone())),
                failures,
                evidence: body
                    .evidence
                    .iter()
                    .map(|evidence| EvidenceSummary {
                        kind: evidence.system.clone(),
                        locator: evidence.locator.clone(),
                        digest: evidence.digest.as_ref().map(|d| d.as_str().to_string()),
                    })
                    .collect(),
            }
        }
        _ => not_run(eligible),
    }
}

pub(super) fn not_run(eligible: bool) -> RunView {
    RunView {
        result: if eligible {
            StateLabel::new("warn", "not run")
        } else {
            StateLabel::new("draft", "draft · not run")
        },
        last_run: None,
        current: false,
        summary: None,
        failures: Vec::new(),
        evidence: Vec::new(),
    }
}

pub(super) fn question_run(
    state: &AgreementState,
    id: &RecordId,
    rule: &Rule,
    eligible: bool,
) -> RunView {
    let answers = latest_judgments(state, id);
    if answers.is_empty() {
        let mut view = not_run(eligible);
        if rule.privacy.local_only {
            view.result = StateLabel::new("muted", "local-only · review");
        }
        return view;
    }
    let mut counts = BTreeMap::<&'static str, usize>::new();
    let mut failures = Vec::new();
    let mut last = String::new();
    let mut redacted = false;
    for record in &answers {
        let RecordBody::Judgment(body) = &record.body else {
            continue;
        };
        *counts.entry(body.outcome.label()).or_default() += 1;
        redacted |= body.redaction.is_some();
        if body.answered_at > last {
            last = body.answered_at.clone();
        }
        if matches!(
            body.outcome,
            JudgmentOutcome::Flag | JudgmentOutcome::LowConfidence
        ) {
            failures.push(Failure {
                location: body.unit.clone(),
                message: format!(
                    "{} (p={})",
                    body.outcome.label().replace('_', " "),
                    body.probability_bp.map_or_else(
                        || "none".to_string(),
                        |bp| format!("{}.{:02}", bp / 10_000, bp % 10_000 / 100)
                    )
                ),
            });
        }
    }
    let flags = counts.get("flag").copied().unwrap_or(0);
    let unavailable = counts.get("unavailable").copied().unwrap_or(0);
    let low = counts.get("low_confidence").copied().unwrap_or(0);
    let shadow = rule.in_shadow();
    let result = if unavailable > 0 && flags == 0 {
        StateLabel::new("warn", "Jev unavailable")
    } else if flags > 0 && shadow {
        StateLabel::new("muted", format!("shadow · {flags} flag(s)"))
    } else if flags > 0 {
        StateLabel::new("fail", format!("flag · {flags}"))
    } else if low > 0 {
        StateLabel::new("warn", "low confidence · raise a hand")
    } else if shadow {
        StateLabel::new("muted", "shadow · clear")
    } else if rule.strength == crate::domain::Strength::Must {
        StateLabel::new("warn", "clear · needs review to pass")
    } else {
        StateLabel::new("pass", "clear")
    };
    RunView {
        result,
        last_run: Some(last),
        current: false,
        summary: Some(format!(
            "{} answer(s): {}{}",
            answers.len(),
            counts
                .iter()
                .map(|(label, count)| format!("{count} {}", label.replace('_', " ")))
                .collect::<Vec<_>>()
                .join(", "),
            if redacted {
                "; text was redacted before sending"
            } else {
                ""
            }
        )),
        failures,
        evidence: Vec::new(),
    }
}

pub(super) fn review_run(
    state: &AgreementState,
    id: &RecordId,
    shown: &AgreementRecord,
    head: Option<&str>,
    eligible: bool,
) -> RunView {
    let Some(attestation) = latest_attestation(state, id) else {
        let mut view = not_run(eligible);
        if eligible {
            view.result = StateLabel::new("warn", "not attested");
        }
        return view;
    };
    let bound = shown
        .reference()
        .ok()
        .is_some_and(|reference| reference == attestation.rule);
    let current = bound && head == Some(attestation.commit.as_str());
    RunView {
        result: match (attestation.verdict, current) {
            (AttestationVerdict::Pass, true) => StateLabel::new("pass", "attested"),
            (AttestationVerdict::Pass, false) => StateLabel::new("warn", "attested · stale"),
            (AttestationVerdict::Fail, _) => StateLabel::new("fail", "review failed"),
        },
        last_run: Some(attestation.attested_at.clone()),
        current,
        summary: Some(format!(
            "{} at {}: {}",
            attestation.reviewer.label(),
            &attestation.commit[..attestation.commit.len().min(12)],
            attestation.notes
        )),
        failures: Vec::new(),
        evidence: attestation
            .evidence
            .iter()
            .map(|evidence| EvidenceSummary {
                kind: evidence.system.clone(),
                locator: evidence.locator.clone(),
                digest: evidence.digest.as_ref().map(|d| d.as_str().to_string()),
            })
            .collect(),
    }
}

pub(super) fn gate_runs(
    state: &AgreementState,
    input: &ProjectionInput<'_>,
    features: &[FeatureEntry],
    owner: &str,
) -> (Vec<GateRun>, Option<LastCheck>) {
    let head = crate::gates::head_commit(input.project_root);
    let mut runs = Vec::new();
    let mut last_at: Option<String> = None;
    let mut tally = Tally::default();
    for id in rule_ids(state) {
        let in_force = state.in_force(&id);
        let Some(shown) = in_force.or_else(|| state.pending(&id)) else {
            continue;
        };
        let Some(rule) = shown.body.rule_view() else {
            continue;
        };
        let (mechanism, command) = crate::gates::mechanism_label(&rule);
        let feature = match &rule.enforcer {
            Enforcer::Drive { feature } => Some(link(state, feature)),
            _ => None,
        };
        let eligible = in_force.is_some();
        let view = match rule.enforcer.family() {
            crate::domain::EnforcerFamily::Mechanical => {
                mechanical_run(state, input, &id, shown, eligible)
            }
            crate::domain::EnforcerFamily::Question if !rule.privacy.local_only => {
                question_run(state, &id, &rule, eligible)
            }
            _ => review_run(state, &id, shown, head.as_deref(), eligible),
        };
        if let Some(at) = &view.last_run {
            if last_at.as_deref().map_or(true, |last| at.as_str() > last) {
                last_at = Some(at.clone());
            }
        }
        if eligible && !rule.in_shadow() {
            match view.result.tone {
                "pass" => tally.pass += 1,
                "fail" => tally.fail += 1,
                _ => tally.unknown += 1,
            }
        }
        let recheck = format!("wh check --rule {}", id.as_str());
        let feature_entry = feature
            .as_ref()
            .and_then(|link| features.iter().find(|entry| entry.entry.id == link.id));
        let brief = (view.result.tone == "fail").then(|| {
            gate_brief(
                &rule.statement,
                shown.revision,
                rule.strength.label(),
                &command,
                &view.failures,
                &recheck,
                owner,
                feature_entry,
            )
        });
        runs.push(GateRun {
            id: id.as_str().into(),
            name: rule.statement.clone(),
            strength: rule.strength.label(),
            family: rule.enforcer.family().label(),
            mechanism,
            command,
            eligible,
            shadow: rule.in_shadow(),
            result: view.result,
            last_run: view.last_run,
            current: view.current,
            summary: view.summary,
            failures: view.failures,
            recheck,
            brief,
            evidence: view.evidence,
            feature,
            team: match (
                input.shared.get(id.as_str()),
                in_force.and_then(|record| record.reference().ok()),
            ) {
                (Some(shared), Some(reference)) if *shared == reference_text(&reference) => {
                    "shared"
                }
                (Some(_), _) => "shared at another revision",
                (None, _) => "private",
            },
        });
    }
    runs.sort_by(|left, right| {
        (
            !left.eligible,
            strength_order(left.strength),
            left.name.as_str(),
        )
            .cmp(&(
                !right.eligible,
                strength_order(right.strength),
                right.name.as_str(),
            ))
    });
    (runs, last_at.map(|at| LastCheck { at, tally }))
}
