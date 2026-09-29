//! Getting stricter and cheaper over time, deterministically: per-rule flag
//! statistics from real receipts and the owner's accept or dismiss labels,
//! and the drafts they suggest (demotion for noisy rules, promotion out of
//! shadow, hardening for recurring Jev flags).
//!
//! Nothing here changes force. Suggestions become drafts only through
//! `wh change --tune`, and the owner accepts every one.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::agreement::AgreementState;
use crate::domain::{
    AgreementRecord, Enforcer, FlagVerdict, JudgmentOutcome, RecordBody, RecordId, RecordRef, Rule,
    Strength, VerificationAxis,
};
use crate::proof::{GATE_SUBJECT_PREFIX, GIT_HEAD_SYSTEM};

/// TypeSafe's published input price: $0.042 per million input tokens.
pub const JEV_USD_PER_MILLION_INPUT_TOKENS: f64 = 0.042;
/// A rule is noisy above one false flag per this many commits.
pub const NOISY_COMMITS_PER_FALSE_FLAG: u64 = 10;
/// Recurring accepted flags that suggest a mechanical check instead.
pub const HARDENING_RECURRENCES: u64 = 3;

/// One flag a rule raised, and the owner's label on it if any.
#[derive(Debug, Clone, Serialize)]
pub struct Flag {
    pub receipt: RecordRef,
    pub at: String,
    pub unit: String,
    pub commit: Option<String>,
    pub shadow: bool,
    pub verdict: Option<FlagVerdict>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RuleStats {
    /// Checks this rule took part in (gate runs and Jev answers).
    pub checks: u64,
    pub commits: u64,
    pub flags: u64,
    pub accepted: u64,
    pub dismissed: u64,
    pub undecided: u64,
    /// Dismissed over labelled flags, as a percentage with one decimal.
    pub false_flag_rate: Option<String>,
    pub jev_answers: u64,
    pub jev_unavailable: u64,
    pub low_confidence: u64,
    pub input_tokens: u64,
    /// Average Jev cost per check in US dollars (mechanical checks cost 0).
    pub cost_per_check_usd: Option<String>,
}

fn rule_of(record: &AgreementRecord) -> Option<&RecordId> {
    match &record.body {
        RecordBody::Judgment(body) => Some(&body.rule.id),
        RecordBody::VerificationReceipt(body) => body
            .policy_state
            .accepted
            .as_ref()
            .map(|reference| &reference.id)
            .filter(|_| body.subject.stable_id.starts_with(GATE_SUBJECT_PREFIX)),
        _ => None,
    }
}

/// Every flag raised by `rule`, oldest first, with the latest label.
pub fn flags(state: &AgreementState, rule: &RecordId) -> Vec<Flag> {
    let mut labels = BTreeMap::<RecordRef, (String, FlagVerdict)>::new();
    for record in state.records() {
        if let RecordBody::FlagDecision(decision) = &record.body {
            if &decision.rule == rule {
                let newer = labels
                    .get(&decision.receipt)
                    .map_or(true, |(at, _)| at.as_str() <= decision.decided_at.as_str());
                if newer {
                    labels.insert(
                        decision.receipt.clone(),
                        (decision.decided_at.clone(), decision.verdict),
                    );
                }
            }
        }
    }
    let mut result = Vec::new();
    for record in state.records() {
        if rule_of(record) != Some(rule) {
            continue;
        }
        let Ok(reference) = record.reference() else {
            continue;
        };
        let (flagged, unit, commit, shadow, at) = match &record.body {
            RecordBody::Judgment(body) => (
                body.outcome == JudgmentOutcome::Flag,
                body.unit.clone(),
                body.commit.clone(),
                body.shadow,
                body.answered_at.clone(),
            ),
            RecordBody::VerificationReceipt(body) => (
                body.verification == VerificationAxis::Fail,
                body.subject.stable_id.clone(),
                body.evidence
                    .iter()
                    .find(|evidence| evidence.system == GIT_HEAD_SYSTEM)
                    .map(|evidence| evidence.locator.clone()),
                false,
                body.checked_at.clone(),
            ),
            _ => continue,
        };
        if flagged {
            result.push(Flag {
                verdict: labels.get(&reference).map(|(_, verdict)| *verdict),
                receipt: reference,
                at,
                unit,
                commit,
                shadow,
            });
        }
    }
    result.sort_by(|left, right| left.at.cmp(&right.at));
    result
}

pub fn rule_stats(state: &AgreementState, rule: &RecordId) -> RuleStats {
    let mut stats = RuleStats::default();
    let mut commits = BTreeSet::new();
    for record in state.records() {
        if rule_of(record) != Some(rule) {
            continue;
        }
        stats.checks += 1;
        match &record.body {
            RecordBody::Judgment(body) => {
                if let Some(commit) = &body.commit {
                    commits.insert(commit.clone());
                }
                match body.outcome {
                    JudgmentOutcome::Unavailable => stats.jev_unavailable += 1,
                    JudgmentOutcome::LowConfidence => {
                        stats.low_confidence += 1;
                        stats.jev_answers += 1;
                    }
                    _ => stats.jev_answers += 1,
                }
                stats.input_tokens += body.input_tokens.unwrap_or(0);
            }
            RecordBody::VerificationReceipt(body) => {
                if let Some(head) = body
                    .evidence
                    .iter()
                    .find(|evidence| evidence.system == GIT_HEAD_SYSTEM)
                {
                    commits.insert(head.locator.clone());
                }
            }
            _ => {}
        }
    }
    stats.commits = commits.len() as u64;
    for flag in flags(state, rule) {
        stats.flags += 1;
        match flag.verdict {
            Some(FlagVerdict::Accept) => stats.accepted += 1,
            Some(FlagVerdict::Dismiss) => stats.dismissed += 1,
            None => stats.undecided += 1,
        }
    }
    let labelled = stats.accepted + stats.dismissed;
    if labelled > 0 {
        stats.false_flag_rate = Some(format!(
            "{:.1}%",
            stats.dismissed as f64 * 100.0 / labelled as f64
        ));
    }
    if stats.checks > 0 {
        let dollars = stats.input_tokens as f64 * JEV_USD_PER_MILLION_INPUT_TOKENS / 1_000_000.0;
        stats.cost_per_check_usd = Some(format!("{:.6}", dollars / stats.checks as f64));
    }
    stats
}

/// A draft `wh change --tune` would record, and why.
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub kind: &'static str,
    pub rule: String,
    pub reason: String,
    /// The v2 rule body the draft would record, when the kernel can write
    /// it (demotion, promotion). Hardening needs the skill to write the check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft: Option<Rule>,
    /// Flagged units that recur, as labelled-example candidates.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<String>,
}

/// Deterministic tuning suggestions for every in-force rule.
pub fn suggestions(state: &AgreementState) -> Vec<Suggestion> {
    let mut result = Vec::new();
    for id in crate::proof::rule_ids(state) {
        let Some(record) = state.in_force(&id) else {
            continue;
        };
        if state.pending(&id).is_some() {
            // A draft is already waiting on this rule; do not stack another.
            continue;
        }
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        let rule = rule.into_owned();
        let stats = rule_stats(state, &id);
        let all_flags = flags(state, &id);
        // Noisy: more than one false flag per ten commits.
        if stats.dismissed >= 2
            && stats.dismissed * NOISY_COMMITS_PER_FALSE_FLAG > stats.commits.max(1)
            && rule.strength != Strength::Advisory
        {
            let mut draft = rule.clone();
            draft.strength = rule.strength.weaker();
            result.push(Suggestion {
                kind: "demotion",
                rule: id.as_str().into(),
                reason: format!(
                    "{} of {} labelled flags were dismissed over {} commit(s): more than one false flag per {NOISY_COMMITS_PER_FALSE_FLAG} commits. Demote from {} to {}, or reword the rule.",
                    stats.dismissed,
                    stats.accepted + stats.dismissed,
                    stats.commits,
                    rule.strength.label(),
                    draft.strength.label()
                ),
                draft: Some(draft),
                examples: Vec::new(),
            });
        }
        if let Enforcer::Question {
            shadow: true,
            promote_after,
            precision_bar_bp,
            ..
        } = &rule.enforcer
        {
            let shadow_flags = all_flags
                .iter()
                .filter(|flag| flag.shadow)
                .collect::<Vec<_>>();
            let accepted = shadow_flags
                .iter()
                .filter(|flag| flag.verdict == Some(FlagVerdict::Accept))
                .count() as u64;
            let labelled = shadow_flags
                .iter()
                .filter(|flag| flag.verdict.is_some())
                .count() as u64;
            let precision_bp = (accepted * 10_000).checked_div(labelled).unwrap_or(0);
            if stats.commits >= u64::from(*promote_after)
                && labelled > 0
                && precision_bp >= u64::from(*precision_bar_bp)
            {
                let mut draft = rule.clone();
                if let Enforcer::Question { shadow, .. } = &mut draft.enforcer {
                    *shadow = false;
                }
                result.push(Suggestion {
                    kind: "promotion",
                    rule: id.as_str().into(),
                    reason: format!(
                        "In shadow over {} commit(s), {accepted} of {labelled} labelled flags were right ({}.{:02}% precision, bar {}.{:02}%). Promote it so its answers are enforced.",
                        stats.commits,
                        precision_bp / 100,
                        precision_bp % 100,
                        precision_bar_bp / 100,
                        precision_bar_bp % 100
                    ),
                    draft: Some(draft),
                    examples: Vec::new(),
                });
            }
        }
        if rule.enforcer.family() == crate::domain::EnforcerFamily::Question
            && stats.accepted >= HARDENING_RECURRENCES
        {
            let examples = all_flags
                .iter()
                .filter(|flag| flag.verdict == Some(FlagVerdict::Accept))
                .map(|flag| flag.unit.clone())
                .collect::<Vec<_>>();
            result.push(Suggestion {
                kind: "hardening",
                rule: id.as_str().into(),
                reason: format!(
                    "Jev's flag on this rule was right {} times. Replace the question with a mechanical check: write it with wh change --kind rule --hardens {} and the flagged units as flag examples.",
                    stats.accepted,
                    id.as_str()
                ),
                draft: None,
                examples,
            });
        }
    }
    result
}
