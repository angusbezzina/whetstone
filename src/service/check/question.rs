//! Jev questions: units, redaction, the ask and the aggregate answer.

use super::*;

/// The repository's privacy settings (`whetstone/privacy.json`), if any.
pub(crate) fn read_privacy(project: &Path) -> crate::domain::Privacy {
    fs::read_to_string(project.join(PRIVACY_RELATIVE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub(super) struct QuestionRun {
    pub(super) outcome: GateOutcome,
    pub(super) aggregate: JudgmentOutcome,
    pub(super) receipts: Vec<AgreementRecord>,
    pub(super) units: usize,
}

pub(super) fn run_question(
    layout: &ProjectLayout,
    run_dir: &Path,
    run_id: &str,
    selected: &SelectedRule,
    source: &UnitSource<'_>,
    repository_privacy: &crate::domain::Privacy,
    head: Option<&str>,
) -> QuestionRun {
    let rule = &selected.rule;
    let mut outcome = GateOutcome::new(&selected.id, rule);
    let Enforcer::Question {
        unit,
        threshold_bp,
        confidence_bar_bp,
        model,
        ..
    } = &rule.enforcer
    else {
        return QuestionRun {
            outcome: outcome.unknown("Not a question rule."),
            aggregate: JudgmentOutcome::Unavailable,
            receipts: Vec::new(),
            units: 0,
        };
    };
    let units = judgment::units(rule, *unit, source);
    if units.is_empty() {
        outcome.state = VerificationAxis::Pass;
        outcome.summary = format!(
            "No {} in scope for this rule; nothing was asked.",
            unit.label().replace('_', " ")
        );
        return QuestionRun {
            outcome,
            aggregate: JudgmentOutcome::Clear,
            receipts: Vec::new(),
            units: 0,
        };
    }
    let answered_at = utc_now();
    let mut receipts = Vec::new();
    let mut tally = BTreeMap::<&'static str, usize>::new();
    let mut redactions = 0;
    let mut aggregate = JudgmentOutcome::Clear;
    for (index, unit) in units.iter().enumerate() {
        let redacted = judgment::redact(
            &[&rule.privacy, repository_privacy],
            unit.path.as_deref(),
            &unit.text,
        );
        redactions += redacted.replaced;
        let Some(request) = judgment::question_request(rule, &redacted.text) else {
            continue;
        };
        let name = format!("{}-{index}", selected.id.as_str().replace(['.', ':'], "_"));
        let answer = judgment::ask(source.project_root, run_dir, &name, &request);
        let (unit_outcome, probability, confidence, answer_model, request_id, tokens, detail) =
            match answer {
                Answer::Answered {
                    probability_bp,
                    model: answered_by,
                    request_id,
                    input_tokens,
                } => {
                    let (unit_outcome, confidence) =
                        judgment::outcome(probability_bp, *threshold_bp, *confidence_bar_bp);
                    (
                        unit_outcome,
                        Some(probability_bp),
                        Some(confidence),
                        if answered_by.is_empty() {
                            model.clone()
                        } else {
                            answered_by
                        },
                        request_id,
                        input_tokens,
                        format!("Jev answered {}", unit_outcome.label().replace('_', " ")),
                    )
                }
                Answer::Unavailable(reason) => (
                    JudgmentOutcome::Unavailable,
                    None,
                    None,
                    model.clone(),
                    None,
                    None,
                    reason,
                ),
            };
        *tally.entry(unit_outcome.label()).or_default() += 1;
        aggregate = worse(aggregate, unit_outcome);
        if matches!(
            unit_outcome,
            JudgmentOutcome::Flag | JudgmentOutcome::LowConfidence
        ) {
            outcome.failures.push(ArtifactFailure {
                location: unit.locator.clone(),
                message: format!(
                    "{}{}",
                    unit_outcome.label().replace('_', " "),
                    probability
                        .map(|bp| format!(" (p {}.{:02})", bp / 10_000, bp % 10_000 / 100))
                        .unwrap_or_default()
                ),
            });
        }
        let key = format!(
            "judgment:{run_id}:{}:{}",
            selected.id.as_str(),
            judgment::input_digest(&redacted.text).as_str()
        );
        if let Ok(record) = receipt_record(
            layout,
            format!("verification.judgment_{}", key_suffix(&key, 24)),
            key,
            "whetstone:jev",
            ProvenanceKind::ExternalObservation,
            &answered_at,
            EvidenceRef {
                system: "typesafe_system_one".into(),
                locator: request_id.clone().unwrap_or_else(|| "no request".into()),
                digest: None,
            },
            RecordBody::Judgment(JudgmentReceipt {
                rule: selected.reference.clone(),
                model: answer_model,
                question_digest: judgment::question_digest(&request),
                input_digest: judgment::input_digest(&redacted.text),
                unit: unit.locator.clone(),
                commit: head.map(str::to_owned),
                outcome: unit_outcome,
                probability_bp: probability,
                confidence_bp: confidence,
                shadow: rule.in_shadow(),
                redaction: redacted.digest.clone(),
                input_tokens: tokens,
                request_id,
                detail: detail.chars().take(300).collect(),
                answered_at: answered_at.clone(),
            }),
        ) {
            receipts.push(record);
        }
    }
    let summary = format!(
        "Asked Jev about {} unit(s): {}{}{}",
        units.len(),
        tally
            .iter()
            .map(|(label, count)| format!("{count} {}", label.replace('_', " ")))
            .collect::<Vec<_>>()
            .join(", "),
        if redactions > 0 {
            format!("; {redactions} redaction(s) before sending")
        } else {
            String::new()
        },
        if rule.in_shadow() {
            "; shadow: recorded, not enforced"
        } else {
            ""
        }
    );
    outcome.summary = summary;
    outcome.state = match aggregate {
        JudgmentOutcome::Flag => VerificationAxis::Fail,
        JudgmentOutcome::Clear if rule.strength != Strength::Must => VerificationAxis::Pass,
        _ => VerificationAxis::Unknown,
    };
    outcome.raise_hand = match aggregate {
        // Low confidence on an enforced question always raises a hand (the
        // plan's first trigger); a shadow rule raises one only if it opts in.
        JudgmentOutcome::LowConfidence => {
            !rule.in_shadow() || rule.raises_hand_on(HandRaiseTrigger::LowConfidence)
        }
        JudgmentOutcome::Unavailable => rule.raises_hand_on(HandRaiseTrigger::Unavailable),
        JudgmentOutcome::Flag => rule.raises_hand_on(HandRaiseTrigger::Flag),
        JudgmentOutcome::Clear => false,
    };
    if aggregate == JudgmentOutcome::LowConfidence {
        outcome.summary.push_str("; low confidence");
    }
    QuestionRun {
        outcome,
        aggregate,
        receipts,
        units: units.len(),
    }
}

pub(super) fn worse(current: JudgmentOutcome, next: JudgmentOutcome) -> JudgmentOutcome {
    let rank = |outcome: JudgmentOutcome| match outcome {
        JudgmentOutcome::Flag => 3,
        JudgmentOutcome::LowConfidence => 2,
        JudgmentOutcome::Unavailable => 1,
        JudgmentOutcome::Clear => 0,
    };
    if rank(next) > rank(current) {
        next
    } else {
        current
    }
}
