//! The Checks view: the latest run of every rule.

use super::*;

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

pub(super) fn gate_runs(
    state: &AgreementState,
    input: &ProjectionInput<'_>,
    features: &[FeatureEntry],
    owner: &str,
) -> (Vec<GateRun>, Option<LastCheck>) {
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
        let view = mechanical_run(state, input, &id, shown, eligible);
        if let Some(at) = &view.last_run {
            if last_at.as_deref().map_or(true, |last| at.as_str() > last) {
                last_at = Some(at.clone());
            }
        }
        if eligible {
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
            mechanism,
            command,
            eligible,
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
