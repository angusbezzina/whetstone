//! The changelog (human journal entries grouped from the decision log) and the
//! decision trail in pstack show-me-your-work's TSV (ts, phase, decision,
//! why, evidence, result), which pstack's audit flow reads unchanged.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{
    AgreementRecord, AttestationVerdict, Enforcer, JudgmentOutcome, LocalReviewVerdict, RecordBody,
    RecordId, RecordRef, VerificationAxis,
};
use crate::history::HistoryInspection;
use crate::proof::{EVIDENCE_SYSTEM, GATE_SUBJECT_PREFIX};

use super::{kind_label, kind_of, record_title, snippet, StateLabel, Tally};

#[derive(Debug, Clone, Serialize)]
pub struct JournalEntry {
    pub id: String,
    pub recorded_at: String,
    pub kind: &'static str,
    pub title: String,
    pub version: Option<String>,
    pub status: StateLabel,
    pub owner: Option<String>,
    pub area: &'static str,
    pub summary: String,
    pub note: Option<String>,
    /// Pending draft proposal this entry can be accepted or withdrawn through.
    pub proposal: Option<String>,
    pub records: Vec<crate::history::HistoryItem>,
}

fn short_version(record: &AgreementRecord) -> String {
    if record.revision <= 1 {
        format!("v{}", record.revision)
    } else {
        format!("v{} → v{}", record.revision - 1, record.revision)
    }
}

fn operation_key(record: &AgreementRecord) -> String {
    let key = record
        .idempotency_key
        .strip_suffix(":proposal")
        .unwrap_or(&record.idempotency_key);
    if let Some(rest) = key.strip_prefix("review:") {
        return format!("review:{rest}");
    }
    if key.starts_with("check:") || key.starts_with("gate:") || key.starts_with("judgment:") {
        let run = key.split(':').nth(1).unwrap_or(key);
        return format!("check:{run}");
    }
    key.find(":base-")
        .map_or_else(|| key.to_string(), |index| key[..index].to_string())
}

pub(crate) fn area_of(record: &AgreementRecord) -> &'static str {
    match &record.body {
        RecordBody::Mission(_) => "mission",
        RecordBody::Principle(_) => "principles",
        RecordBody::Rule(_) | RecordBody::Standard(_) | RecordBody::Guidance(_) => "rules",
        RecordBody::Feature(_) | RecordBody::VerificationMap(_) => "features",
        RecordBody::VerificationReceipt(_)
        | RecordBody::Judgment(_)
        | RecordBody::Attestation(_)
        | RecordBody::Brief(_) => "checks",
        RecordBody::FlagDecision(_) => "rules",
        RecordBody::HandRaise(_) | RecordBody::HandAnswer(_) => "requests",
        RecordBody::Retired(_) => "history",
        _ => "governance",
    }
}

/// Group history items into human journal entries, newest first.
pub fn journal(state: &AgreementState, history: Option<&HistoryInspection>) -> Vec<JournalEntry> {
    history.map_or_else(Vec::new, |history| {
        journal_from_items(state, &history.decision_history.items)
    })
}

/// Journal entries from any set of history items (the dashboard passes the
/// whole visible history, so the newest entries are never paged away).
pub fn journal_from_items(
    state: &AgreementState,
    items: &[crate::history::HistoryItem],
) -> Vec<JournalEntry> {
    let mut groups = BTreeMap::<String, Vec<crate::history::HistoryItem>>::new();
    for item in items {
        let key = item.record.as_ref().map_or_else(
            || {
                format!(
                    "redacted:{}:{}",
                    item.reference.id.as_str(),
                    item.reference.revision
                )
            },
            operation_key,
        );
        groups.entry(key).or_default().push(item.clone());
    }
    let mut entries = groups
        .into_iter()
        .map(|(id, mut items)| {
            items.sort_by(|left, right| left.recorded_at.cmp(&right.recorded_at));
            journal_entry(state, id, items)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .recorded_at
            .cmp(&left.recorded_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    entries
}

/// Keep the entries whose visible text or underlying records contain
/// `term`, case-insensitively.
pub fn search_journal(entries: Vec<JournalEntry>, term: Option<&str>) -> Vec<JournalEntry> {
    let Some(term) = term.map(str::trim).filter(|term| !term.is_empty()) else {
        return entries;
    };
    let term = term.to_lowercase();
    entries
        .into_iter()
        .filter(|entry| {
            let visible = [
                entry.title.as_str(),
                entry.summary.as_str(),
                entry.area,
                entry.kind,
                entry.owner.as_deref().unwrap_or_default(),
                entry.note.as_deref().unwrap_or_default(),
                entry.version.as_deref().unwrap_or_default(),
            ];
            visible
                .iter()
                .any(|text| text.to_lowercase().contains(&term))
                || entry.records.iter().any(|item| {
                    item.record
                        .as_ref()
                        .and_then(|record| serde_json::to_string(record).ok())
                        .is_some_and(|text| text.to_lowercase().contains(&term))
                })
        })
        .collect()
}

fn journal_entry(
    state: &AgreementState,
    id: String,
    items: Vec<crate::history::HistoryItem>,
) -> JournalEntry {
    let recorded_at = items
        .iter()
        .map(|item| item.recorded_at.clone())
        .max()
        .unwrap_or_default();
    let records = items
        .iter()
        .filter_map(|item| item.record.as_ref())
        .collect::<Vec<_>>();
    let owner = records
        .iter()
        .find_map(|record| record.owner.display_name.clone());
    let base = |kind, title: String, status, area, summary: String| JournalEntry {
        id: id.clone(),
        recorded_at: recorded_at.clone(),
        kind,
        title,
        version: None,
        status,
        owner: owner.clone(),
        area,
        summary,
        note: None,
        proposal: None,
        records: items.clone(),
    };
    if records.is_empty() {
        return base(
            "decision",
            "Record withheld".into(),
            StateLabel::new("muted", "redacted"),
            "governance",
            "This history item is not visible from this boundary.".into(),
        );
    }
    // A reported maintain pass (pstack's maintain-verification-skill).
    if let Some(body) = records.iter().find_map(|record| match &record.body {
        RecordBody::VerificationReceipt(body)
            if body.subject.stable_id == crate::service::MAINTAIN_SUBJECT =>
        {
            Some(body)
        }
        _ => None,
    }) {
        let outcome = body
            .subject
            .revision
            .clone()
            .unwrap_or_else(|| "reported".into());
        return base(
            "verification",
            format!("Maintain pass: {outcome}"),
            match body.verification {
                VerificationAxis::Pass => StateLabel::new("pass", outcome.as_str()),
                _ => StateLabel::new("warn", "unknown"),
            },
            "features",
            body.evidence.first().map_or_else(
                || "No evidence was supplied, so the pass counts as unknown.".to_string(),
                |evidence| format!("Evidence: {}", evidence.locator),
            ),
        );
    }
    // An agent host checked in with the skill revision it had.
    if let Some(body) = records.iter().find_map(|record| match &record.body {
        RecordBody::VerificationReceipt(body)
            if body
                .subject
                .stable_id
                .starts_with(crate::hosts::HOST_SUBJECT_PREFIX) =>
        {
            Some(body)
        }
        _ => None,
    }) {
        let host = body
            .subject
            .stable_id
            .trim_start_matches(crate::hosts::HOST_SUBJECT_PREFIX)
            .to_string();
        return base(
            "verification",
            format!("Host checkpoint: {host}"),
            match body.verification {
                VerificationAxis::Pass => StateLabel::new("pass", "current skill"),
                _ => StateLabel::new("warn", "skill not current"),
            },
            "checks",
            format!(
                "{host} had skill revision {} at this checkpoint.",
                body.subject.revision.as_deref().unwrap_or("none")
            ),
        );
    }
    // Check runs: gate receipts, native scans and Jev answers.
    if records.iter().all(|record| {
        matches!(
            record.body,
            RecordBody::VerificationReceipt(_) | RecordBody::Judgment(_)
        )
    }) {
        let mut tally = Tally::default();
        let mut shadow = 0;
        for record in &records {
            match &record.body {
                RecordBody::VerificationReceipt(body) => match body.verification {
                    VerificationAxis::Pass => tally.pass += 1,
                    VerificationAxis::Fail => tally.fail += 1,
                    VerificationAxis::Unknown => tally.unknown += 1,
                },
                RecordBody::Judgment(body) if body.shadow => shadow += 1,
                RecordBody::Judgment(body) => match body.outcome {
                    JudgmentOutcome::Flag => tally.fail += 1,
                    JudgmentOutcome::Clear => tally.pass += 1,
                    _ => tally.unknown += 1,
                },
                _ => {}
            }
        }
        let status = if tally.fail > 0 {
            StateLabel::new("fail", "fail")
        } else if tally.unknown > 0 {
            StateLabel::new("warn", "unknown")
        } else {
            StateLabel::new("pass", "pass")
        };
        let rules = records
            .iter()
            .filter_map(|record| match &record.body {
                RecordBody::VerificationReceipt(body) => body
                    .subject
                    .stable_id
                    .strip_prefix(GATE_SUBJECT_PREFIX)
                    .map(str::to_owned),
                RecordBody::Judgment(body) => Some(format!("{} (Jev)", body.rule.id.as_str())),
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        let redacted = records
            .iter()
            .filter(|record| {
                matches!(&record.body, RecordBody::Judgment(body) if body.redaction.is_some())
            })
            .count();
        return base(
            "verification",
            format!(
                "Checks ran: {} pass · {} fail · {} unknown{}{}",
                tally.pass,
                tally.fail,
                tally.unknown,
                if shadow > 0 {
                    format!(" · {shadow} shadow")
                } else {
                    String::new()
                },
                if redacted > 0 {
                    format!(" · redacted before {redacted} Jev question(s)")
                } else {
                    String::new()
                }
            ),
            status,
            "checks",
            if rules.is_empty() {
                "Compiled-in scanner run.".into()
            } else {
                format!(
                    "Rules: {}",
                    rules.into_iter().collect::<Vec<_>>().join(", ")
                )
            },
        );
    }
    if let Some(record) = records.first().filter(|_| records.len() == 1) {
        match &record.body {
            RecordBody::Attestation(body) => {
                return base(
                    "verification",
                    format!("Review attested: {}", body.rule.id.as_str()),
                    match body.verdict {
                        AttestationVerdict::Pass => StateLabel::new("pass", "pass"),
                        AttestationVerdict::Fail => StateLabel::new("fail", "fail"),
                    },
                    "checks",
                    format!("{}: {}", body.reviewer.label(), body.notes),
                );
            }
            RecordBody::Brief(body) => {
                return base(
                    "verification",
                    format!("Brief recorded: {}", body.area),
                    StateLabel::new("muted", "recorded"),
                    "checks",
                    format!(
                        "Ran {}. Reuse: {}. Could break: {}.",
                        body.skills.join(", "),
                        if body.reuse.is_empty() {
                            "nothing named".into()
                        } else {
                            body.reuse.join("; ")
                        },
                        if body.risks.is_empty() {
                            "nothing named".into()
                        } else {
                            body.risks.join("; ")
                        }
                    ),
                );
            }
            RecordBody::FlagDecision(body) => {
                return base(
                    "decision",
                    format!("Flag {} on {}", body.verdict.label(), body.rule.as_str()),
                    StateLabel::new("muted", body.verdict.label()),
                    "rules",
                    format!("{} ({})", body.reason, body.decided_by),
                );
            }
            RecordBody::HandRaise(body) => {
                return base(
                    "decision",
                    format!("Raised a hand: {}", snippet(&body.question, 90)),
                    StateLabel::new("warn", "open"),
                    "requests",
                    format!(
                        "{} · recommends: {}",
                        body.trigger.label(),
                        body.recommendation
                    ),
                );
            }
            RecordBody::HandAnswer(body) => {
                return base(
                    "decision",
                    format!("Answered {}", body.issue),
                    StateLabel::new("pass", "answered"),
                    "requests",
                    format!("{} ({})", body.answer, body.answered_by),
                );
            }
            RecordBody::Retired(retired) => {
                return base(
                    "decision",
                    record_title(record),
                    StateLabel::new("muted", "retired kind"),
                    "history",
                    format!(
                        "A {} record from before the delegation plan; kept exactly as recorded.",
                        retired.record_type.replace('_', " ")
                    ),
                );
            }
            _ => {}
        }
    }
    if let Some(review) = records.iter().find_map(|record| match &record.body {
        RecordBody::LocalReview(review) => Some(review),
        _ => None,
    }) {
        let candidate = state
            .get(&review.proposal)
            .and_then(|proposal| match &proposal.body {
                RecordBody::Proposal(body) => body
                    .proposed_records
                    .first()
                    .and_then(|reference| state.get(reference)),
                _ => None,
            });
        let (kind, title) = candidate.map_or(("Draft", "a draft".to_string()), |record| {
            (kind_label(kind_of(record)), record_title(record))
        });
        let (verb, status) = match review.verdict {
            LocalReviewVerdict::Accept => ("accepted", StateLabel::new("pass", "accepted")),
            LocalReviewVerdict::Withdraw => ("withdrawn", StateLabel::new("muted", "withdrawn")),
        };
        return base(
            "decision",
            format!("{kind} {verb}: {}", snippet(&title, 90)),
            status,
            "governance",
            review.rationale.clone(),
        );
    }
    if let Some((retirement, body)) = records.iter().find_map(|record| match &record.body {
        RecordBody::Retirement(body) => Some((*record, body)),
        _ => None,
    }) {
        let target = state.get(&body.target);
        let title = target.map_or_else(|| body.target.id.as_str().to_string(), record_title);
        let lifecycle = state.lifecycle_of(retirement);
        let status = match lifecycle {
            Lifecycle::Accepted => StateLabel::new("pass", "retired"),
            Lifecycle::Draft => StateLabel::new("draft", "draft"),
            Lifecycle::Withdrawn => StateLabel::new("muted", "withdrawn"),
            Lifecycle::Operational => StateLabel::new("muted", "recorded"),
        };
        let mut entry = base(
            "decision",
            format!(
                "{} retirement: {}",
                kind_label(target.map_or("record", kind_of)),
                snippet(&title, 90)
            ),
            status,
            target.map_or("governance", area_of),
            body.reason.clone(),
        );
        entry.version = Some(format!("v{} retired", body.target.revision));
        if lifecycle == Lifecycle::Draft {
            entry.proposal = state
                .proposal_for(retirement)
                .map(|proposal| proposal.id.as_str().to_string());
        }
        return entry;
    }
    let candidates = records
        .iter()
        .filter(|record| crate::agreement::is_agreement_body(&record.body))
        .copied()
        .collect::<Vec<_>>();
    let proposal = records.iter().find_map(|record| match &record.body {
        RecordBody::Proposal(proposal) => Some(proposal),
        _ => None,
    });
    let onboarding = proposal.is_none() && candidates.len() > 1;
    if onboarding {
        let mut entry = base(
            "decision",
            "Agreement established".into(),
            StateLabel::new("pass", "accepted"),
            "mission",
            format!(
                "Recorded as one wh init operation: {}.",
                candidates
                    .iter()
                    .map(|record| format!(
                        "{} {}",
                        kind_label(kind_of(record)).to_lowercase(),
                        record.id.as_str()
                    ))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        entry.version = Some(format!(
            "revision {}",
            candidates
                .iter()
                .map(|record| record.revision)
                .max()
                .unwrap_or(1)
        ));
        return entry;
    }
    let Some(candidate) = candidates.first() else {
        let title = records
            .last()
            .map_or_else(|| "Record".into(), |record| record_title(record));
        return base(
            "decision",
            snippet(&title, 90),
            StateLabel::new("muted", "recorded"),
            "governance",
            "The exact records are available below.".into(),
        );
    };
    let lifecycle = state.lifecycle_of(candidate);
    let superseded_by = state
        .in_force(&candidate.id)
        .filter(|current| {
            current.revision > candidate.revision
                && matches!(lifecycle, Lifecycle::Accepted | Lifecycle::Draft)
        })
        .map(|current| current.revision);
    let status = match (lifecycle, superseded_by) {
        (_, Some(_)) => StateLabel::new("muted", "superseded"),
        (Lifecycle::Accepted, None) => StateLabel::new("pass", "accepted"),
        (Lifecycle::Draft, _) => StateLabel::new("draft", "draft"),
        (Lifecycle::Withdrawn, _) => StateLabel::new("muted", "withdrawn"),
        (Lifecycle::Operational, _) => StateLabel::new("muted", "recorded"),
    };
    let verb = if candidate.revision <= 1 {
        "added"
    } else {
        "revised"
    };
    let summary = proposal
        .map(|proposal| {
            proposal
                .rationale
                .lines()
                .find_map(|line| line.strip_prefix("Rationale: "))
                .unwrap_or(&proposal.rationale)
                .to_string()
        })
        .unwrap_or_else(|| "Recorded by the owner.".into());
    let mut entry = base(
        "decision",
        format!(
            "{} {verb}: {}",
            kind_label(kind_of(candidate)),
            snippet(&record_title(candidate), 90)
        ),
        status,
        area_of(candidate),
        summary,
    );
    entry.version = Some(short_version(candidate));
    entry.note = superseded_by
        .map(|revision| format!("superseded by v{revision}"))
        .or_else(|| rule_transition(state, candidate));
    if lifecycle == Lifecycle::Draft && superseded_by.is_none() {
        entry.proposal = state
            .proposal_for(candidate)
            .map(|proposal| proposal.id.as_str().to_string());
    }
    entry
}

/// A strength change or a promotion out of shadow between a rule revision
/// and the one it supersedes, in words.
fn rule_transition(state: &AgreementState, record: &AgreementRecord) -> Option<String> {
    let after = record.body.rule_view()?;
    let previous = state.get(record.supersedes.as_ref()?)?;
    let before = previous.body.rule_view()?;
    let mut changes = Vec::new();
    if before.strength != after.strength {
        changes.push(format!(
            "strength {} → {}",
            before.strength.label(),
            after.strength.label()
        ));
    }
    match (&before.enforcer, &after.enforcer) {
        (Enforcer::Question { shadow: true, .. }, Enforcer::Question { shadow: false, .. }) => {
            changes.push("promoted from shadow".into());
        }
        (Enforcer::Question { .. }, next)
            if next.family() == crate::domain::EnforcerFamily::Mechanical =>
        {
            changes.push(format!(
                "hardened into a {} check",
                next.kind().replace('_', " ")
            ));
        }
        (left, right) if left.kind() != right.kind() => {
            changes.push(format!("enforcer {} → {}", left.kind(), right.kind()));
        }
        _ => {}
    }
    (!changes.is_empty()).then(|| changes.join("; "))
}

/// Candidate references of one pending proposal, for review routes.
pub fn proposal_candidates(state: &AgreementState, proposal_id: &str) -> Vec<RecordRef> {
    state
        .pending_proposals()
        .into_iter()
        .filter(|pending| pending.proposal.id.as_str() == proposal_id)
        .flat_map(|pending| {
            pending
                .candidates
                .into_iter()
                .filter_map(|candidate| candidate.reference().ok())
        })
        .collect()
}

/// pstack show-me-your-work's header row, verbatim.
pub const TRAIL_HEADER: &str = "ts\tphase\tdecision\twhy\tevidence\tresult";

/// One single-line TSV cell: tabs and newlines become spaces, and a leading
/// `=`, `+`, `-` or `@` is quoted so spreadsheets never evaluate it.
fn trail_cell(value: &str) -> String {
    let flat = value
        .chars()
        .map(|character| {
            if matches!(character, '\t' | '\n' | '\r') {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let flat = flat.trim().to_string();
    if flat.starts_with(['=', '+', '-', '@']) {
        format!("'{flat}")
    } else if flat.is_empty() {
        "none".into()
    } else {
        flat
    }
}

fn record_evidence(record: &AgreementRecord) -> String {
    format!(
        "whetstone:{}@r{} {}",
        record.id.as_str(),
        record.revision,
        record
            .digest()
            .map(|digest| digest.as_str()[..19].to_string())
            .unwrap_or_default()
    )
}

/// The decision history as a show-me-your-work TSV: accepted changes
/// (retirements, strength changes and shadow promotions included), check
/// receipts, Jev answers (with redactions), review attestations, briefs,
/// flag decisions and raised hands with their answers, in time order.
pub fn decision_trail(state: &AgreementState) -> String {
    let mut rows = Vec::<(String, String, [String; 5])>::new();
    let acceptance = |record: &AgreementRecord| -> Option<(String, String)> {
        let proposal = state.proposal_for(record);
        let reviewed = proposal.and_then(|proposal| {
            let reference = proposal.reference().ok()?;
            state
                .records()
                .iter()
                .find_map(|candidate| match &candidate.body {
                    RecordBody::LocalReview(review)
                        if review.proposal == reference
                            && review.verdict == LocalReviewVerdict::Accept =>
                    {
                        Some((review.reviewed_at.clone(), review.rationale.clone()))
                    }
                    _ => None,
                })
        });
        let why = proposal
            .and_then(|proposal| match &proposal.body {
                RecordBody::Proposal(body) => body
                    .rationale
                    .lines()
                    .find_map(|line| line.strip_prefix("Rationale: "))
                    .map(str::to_owned),
                _ => None,
            })
            .unwrap_or_else(|| "Agreed by the owner during wh init.".into());
        match reviewed {
            Some((at, _)) => Some((at, why)),
            None if proposal.is_none() => Some((record.provenance.recorded_at.clone(), why)),
            None => None,
        }
    };
    let rule_title = |reference: &RecordRef| {
        state
            .get(reference)
            .or_else(|| state.in_force(&reference.id))
            .map_or_else(|| reference.id.as_str().to_string(), record_title)
    };
    for record in state.records() {
        let accepted = state.lifecycle_of(record) == Lifecycle::Accepted;
        let id = record.id.as_str().to_string();
        match &record.body {
            body if crate::agreement::is_agreement_body(body) && accepted => {
                let Some((at, why)) = acceptance(record) else {
                    continue;
                };
                let result = if state.is_retired(&record.id) && state.in_force(&record.id).is_none()
                {
                    "accepted, later retired".to_string()
                } else if let Some(current) = state
                    .in_force(&record.id)
                    .filter(|current| current.revision > record.revision)
                {
                    format!("accepted, superseded by v{}", current.revision)
                } else {
                    "accepted, in force".to_string()
                };
                let transition = rule_transition(state, record)
                    .map(|change| format!(" ({change})"))
                    .unwrap_or_default();
                rows.push((
                    at,
                    id,
                    [
                        area_of(record).into(),
                        format!(
                            "{} v{}: {}{transition}",
                            kind_label(kind_of(record)),
                            record.revision,
                            snippet(&record_title(record), 160)
                        ),
                        why,
                        record_evidence(record),
                        result,
                    ],
                ));
            }
            RecordBody::Retirement(body) if accepted => {
                let Some((at, _)) = acceptance(record) else {
                    continue;
                };
                let title = state
                    .get(&body.target)
                    .map_or_else(|| body.target.id.as_str().to_string(), record_title);
                rows.push((
                    at,
                    id,
                    [
                        state.get(&body.target).map_or("governance", area_of).into(),
                        format!(
                            "Retired {} v{}: {}",
                            body.target.id.as_str(),
                            body.target.revision,
                            snippet(&title, 160)
                        ),
                        body.reason.clone(),
                        format!("whetstone:{}@r{}", record.id.as_str(), record.revision),
                        "retired; history kept".into(),
                    ],
                ));
            }
            RecordBody::VerificationReceipt(body) => {
                let gate = body
                    .subject
                    .stable_id
                    .strip_prefix(GATE_SUBJECT_PREFIX)
                    .map(str::to_owned);
                let maintain = body.subject.stable_id == crate::service::MAINTAIN_SUBJECT;
                if let Some(host) = body
                    .subject
                    .stable_id
                    .strip_prefix(crate::hosts::HOST_SUBJECT_PREFIX)
                {
                    rows.push((
                        body.checked_at.clone(),
                        id,
                        [
                            "hosts".into(),
                            format!("Host checkpoint: {host}"),
                            "An agent host reported the skill revision it had.".into(),
                            body.subject
                                .revision
                                .clone()
                                .unwrap_or_else(|| "no skill installed".into()),
                            if body.verification == VerificationAxis::Pass {
                                "current skill".into()
                            } else {
                                "skill not current".into()
                            },
                        ],
                    ));
                    continue;
                }
                let quarantine = body
                    .evidence
                    .iter()
                    .find(|evidence| evidence.system == crate::gates::QUARANTINE_EVIDENCE);
                if let Some(feature) = body
                    .subject
                    .stable_id
                    .strip_prefix(crate::gates::MUTATION_SUBJECT_PREFIX)
                {
                    rows.push((
                        body.checked_at.clone(),
                        id,
                        [
                            "proofs".into(),
                            format!("Mutated {feature} to test its proof"),
                            body.evidence
                                .iter()
                                .find(|evidence| evidence.system == "whetstone_mutation")
                                .map_or_else(
                                    || "a recorded mutation".to_string(),
                                    |evidence| evidence.locator.clone(),
                                ),
                            format!("whetstone:{}", record.id.as_str()),
                            match body.verification {
                                VerificationAxis::Pass => {
                                    "proof failed under mutation: real".into()
                                }
                                VerificationAxis::Fail => "hollow: the proof still passed".into(),
                                VerificationAxis::Unknown => "unknown (not a pass)".into(),
                            },
                        ],
                    ));
                    continue;
                }
                let (decision, why) = match &gate {
                    _ if maintain => (
                        format!(
                            "Recorded the maintain pass outcome: {}",
                            body.subject.revision.as_deref().unwrap_or("reported")
                        ),
                        "pstack's maintain-verification-skill kept the feature map honest.".into(),
                    ),
                    Some(gate) => {
                        let statement = body
                            .policy_state
                            .accepted
                            .as_ref()
                            .map(&rule_title)
                            .or_else(|| {
                                RecordId::new(gate.as_str())
                                    .ok()
                                    .and_then(|id| state.in_force(&id))
                                    .map(record_title)
                            })
                            .unwrap_or_else(|| gate.clone());
                        (format!("Ran rule {gate}"), statement)
                    }
                    None => (
                        "Ran the compiled-in scanner".into(),
                        "Applicable deterministic rules.".into(),
                    ),
                };
                let evidence = body
                    .evidence
                    .iter()
                    .filter(|evidence| {
                        evidence.system == EVIDENCE_SYSTEM || evidence.system == "maintain_run"
                    })
                    .map(|evidence| evidence.locator.clone())
                    .take(3)
                    .collect::<Vec<_>>();
                rows.push((
                    body.checked_at.clone(),
                    id.clone(),
                    [
                        if maintain {
                            "maintain".into()
                        } else {
                            "checks".into()
                        },
                        decision,
                        why,
                        if evidence.is_empty() {
                            format!("whetstone:{id}")
                        } else {
                            evidence.join(", ")
                        },
                        match (body.verification, quarantine) {
                            (_, Some(first)) => {
                                format!("quarantined as flaky (first run: {})", first.locator)
                            }
                            (VerificationAxis::Pass, None) => "pass".into(),
                            (VerificationAxis::Fail, None) => "fail".into(),
                            (VerificationAxis::Unknown, None) => "unknown (not a pass)".into(),
                        },
                    ],
                ));
            }
            RecordBody::Judgment(body) => {
                let mut evidence = format!(
                    "model {} · question {} · input {}",
                    body.model,
                    &body.question_digest.as_str()[..19],
                    &body.input_digest.as_str()[..19]
                );
                if let Some(redaction) = &body.redaction {
                    evidence.push_str(&format!(" · redacted {}", &redaction.as_str()[..19]));
                }
                rows.push((
                    body.answered_at.clone(),
                    id,
                    [
                        "checks".into(),
                        format!(
                            "Asked Jev about {} for {}",
                            body.unit,
                            body.rule.id.as_str()
                        ),
                        rule_title(&body.rule),
                        evidence,
                        format!(
                            "{}{}{}",
                            body.outcome.label().replace('_', " "),
                            body.probability_bp
                                .map(|bp| format!(" (p {}.{:02})", bp / 10_000, bp % 10_000 / 100))
                                .unwrap_or_default(),
                            if body.shadow {
                                ", shadow: not enforced"
                            } else {
                                ""
                            }
                        ),
                    ],
                ));
            }
            RecordBody::Attestation(body) => rows.push((
                body.attested_at.clone(),
                id,
                [
                    "review".into(),
                    format!(
                        "Review of {} by {}",
                        body.rule.id.as_str(),
                        body.reviewer.label()
                    ),
                    body.notes.clone(),
                    format!("commit {}", &body.commit[..body.commit.len().min(12)]),
                    match body.verdict {
                        AttestationVerdict::Pass => "pass".into(),
                        AttestationVerdict::Fail => "fail".into(),
                    },
                ],
            )),
            RecordBody::Brief(body) => rows.push((
                body.recorded_at.clone(),
                id,
                [
                    "brief".into(),
                    format!("Brief for {}", body.area),
                    body.notes.clone(),
                    format!(
                        "reuse: {}; risks: {}",
                        if body.reuse.is_empty() {
                            "none named".into()
                        } else {
                            body.reuse.join(", ")
                        },
                        if body.risks.is_empty() {
                            "none named".into()
                        } else {
                            body.risks.join(", ")
                        }
                    ),
                    format!("ran {}", body.skills.join(", ")),
                ],
            )),
            RecordBody::FlagDecision(body) => rows.push((
                body.decided_at.clone(),
                id,
                [
                    "rules".into(),
                    format!("Flag {} on {}", body.verdict.label(), body.rule.as_str()),
                    body.reason.clone(),
                    format!(
                        "whetstone:{}@r{} by {}",
                        body.receipt.id.as_str(),
                        body.receipt.revision,
                        body.decided_by
                    ),
                    match body.verdict {
                        crate::domain::FlagVerdict::Accept => "true flag".into(),
                        crate::domain::FlagVerdict::Dismiss => "false flag".into(),
                    },
                ],
            )),
            RecordBody::HandRaise(body) => rows.push((
                body.raised_at.clone(),
                id,
                [
                    "hands".into(),
                    format!(
                        "Raised a hand ({}): {}",
                        body.trigger.label(),
                        body.question
                    ),
                    format!("Tried: {} Recommends: {}", body.tried, body.recommendation),
                    format!("beads:{}", body.issue),
                    "waiting for the owner".into(),
                ],
            )),
            RecordBody::HandAnswer(body) => rows.push((
                body.answered_at.clone(),
                id,
                [
                    "hands".into(),
                    format!("Answered {}", body.issue),
                    body.answer.clone(),
                    format!("beads:{} by {}", body.issue, body.answered_by),
                    "answered".into(),
                ],
            )),
            _ => {}
        }
    }
    rows.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let mut out = String::from(TRAIL_HEADER);
    out.push('\n');
    for (at, _, cells) in rows {
        out.push_str(&trail_cell(&at));
        for cell in cells {
            out.push('\t');
            out.push_str(&trail_cell(&cell));
        }
        out.push('\n');
    }
    out
}
