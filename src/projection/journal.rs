//! The changelog (human journal entries grouped from the decision log) and the
//! decision trail in pstack show-me-your-work's TSV (ts, phase, decision,
//! why, evidence, result), which pstack's audit flow reads unchanged.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{
    AgreementRecord, LocalReviewVerdict, RecordBody, RecordId, RecordRef, VerificationAxis,
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
    if key.starts_with("check:") || key.starts_with("gate:") {
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
        RecordBody::Rule(_) | RecordBody::StoredRule(_) | RecordBody::Standard(_) => "rules",
        RecordBody::VerificationReceipt(_) => "checks",
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
    // Check runs: gate receipts and native scans.
    if records
        .iter()
        .all(|record| matches!(record.body, RecordBody::VerificationReceipt(_)))
    {
        let mut tally = Tally::default();
        for record in &records {
            if let RecordBody::VerificationReceipt(body) = &record.body {
                match body.verification {
                    VerificationAxis::Pass => tally.pass += 1,
                    VerificationAxis::Fail => tally.fail += 1,
                    VerificationAxis::Unknown => tally.unknown += 1,
                }
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
                _ => None,
            })
            .collect::<std::collections::BTreeSet<_>>();
        return base(
            "verification",
            format!(
                "Checks ran: {} pass · {} fail · {} unknown",
                tally.pass, tally.fail, tally.unknown
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
        if let RecordBody::Retired(retired) = &record.body {
            return base(
                "decision",
                record_title(record),
                StateLabel::new("muted", "retired kind"),
                "history",
                format!(
                    "A {} record of a kind Whetstone no longer writes; kept exactly as recorded.",
                    retired.record_type.replace('_', " ")
                ),
            );
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

/// A strength or enforcer change between a rule revision and the one it
/// supersedes, in words.
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
/// (retirements and strength changes included) and check receipts, in time
/// order.
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
                let (decision, why) = match &gate {
                    Some(gate) => {
                        let statement = body
                            .policy_state
                            .accepted
                            .as_ref()
                            .map(rule_title)
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
                    .filter(|evidence| evidence.system == EVIDENCE_SYSTEM)
                    .map(|evidence| evidence.locator.clone())
                    .take(3)
                    .collect::<Vec<_>>();
                rows.push((
                    body.checked_at.clone(),
                    id.clone(),
                    [
                        "checks".into(),
                        decision,
                        why,
                        if evidence.is_empty() {
                            format!("whetstone:{id}")
                        } else {
                            evidence.join(", ")
                        },
                        match body.verification {
                            VerificationAxis::Pass => "pass".into(),
                            VerificationAxis::Fail => "fail".into(),
                            VerificationAxis::Unknown => "unknown (not a pass)".into(),
                        },
                    ],
                ));
            }
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
