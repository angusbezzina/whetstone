//! Typed, human-labelled projection of the agreement for the dashboard's
//! views: Home, Rules and Changelog, plus onboarding.
//!
//! The browser renders these fields verbatim. Every state carries a human
//! label and a tone so the UI never invents state or shows raw enum names,
//! and draft states can never read as accepted.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{AgreementRecord, PrincipleSource, RecordBody, RecordId, RecordRef, Rule};

mod journal;

pub use journal::{
    decision_trail, journal, journal_from_items, proposal_candidates, search_journal, JournalEntry,
    TRAIL_HEADER,
};

pub const MISSION_ID: &str = "mission.project";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StateLabel {
    pub tone: &'static str,
    pub label: String,
}

impl StateLabel {
    pub(crate) fn new(tone: &'static str, label: impl Into<String>) -> Self {
        Self {
            tone,
            label: label.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Header {
    pub project: String,
    pub agreement: String,
    pub visibility: &'static str,
    pub drafts: usize,
    pub team: &'static str,
}

/// One onboarding step: mission, principles, exemplars, starter rules, gates.
#[derive(Debug, Clone, Serialize)]
pub struct OnboardingStep {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub done: bool,
    pub optional: bool,
}

/// A starter rule the owner can accept or skip.
#[derive(Debug, Clone, Serialize)]
pub struct StarterView {
    pub id: String,
    pub title: String,
    pub detected: String,
    pub strength: &'static str,
    pub enforcer: String,
    pub examples: Vec<Value>,
    pub accepted: bool,
}

/// An earlier value or philosophy that becomes a principle draft.
#[derive(Debug, Clone, Serialize)]
pub struct LegacyItem {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub migrated: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Onboarding {
    pub steps: Vec<OnboardingStep>,
    pub missing: Vec<String>,
    pub command: &'static str,
    pub catalogue: Vec<crate::catalogue::CatalogueEntry>,
    pub catalogue_version: String,
    pub starters: Vec<StarterView>,
    pub legacy: Vec<LegacyItem>,
    pub tools: Vec<crate::setup::ToolState>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detail {
    pub key: &'static str,
    pub value: String,
    pub code: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    pub version: u64,
    pub title: String,
    pub proposal: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditBase {
    pub kind: &'static str,
    pub record_id: String,
    pub content: String,
    pub definition: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub detail: Vec<Detail>,
    pub version: u64,
    pub lifecycle: &'static str,
    pub pending: Option<Pending>,
    pub state: Option<StateLabel>,
    pub owner: Option<String>,
    pub changed_at: String,
    pub edit: EditBase,
}

/// A rule as the Rules view shows it: strength, enforcer, where it runs and
/// how a sweep works its flags.
#[derive(Debug, Clone, Serialize)]
pub struct RuleEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub strength: &'static str,
    pub enforcer: String,
    pub command: String,
    pub runs_at: &'static str,
    pub source: String,
    pub paths: Vec<String>,
    pub examples: Vec<Value>,
    /// How a night sweep works this rule's flags: draft_pr, confirm_first or
    /// discuss (the default).
    pub sweep_class: &'static str,
    /// How urgent its sweep findings are, 0 (most) to 4.
    pub sweep_priority: u8,
    /// Where its own flags are doubtful, with the reason each says.
    pub sweep_unsure: Vec<crate::domain::SweepUnsure>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attention {
    pub priority: u8,
    pub tone: &'static str,
    pub kind: &'static str,
    pub kind_label: &'static str,
    pub title: String,
    pub text: String,
    pub actor: String,
    pub next: String,
    pub route: &'static str,
    pub focus: Option<String>,
    pub action_label: &'static str,
    pub agent_instruction: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatestChange {
    pub title: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardView {
    pub established: bool,
    pub header: Header,
    pub onboarding: Onboarding,
    pub mission: Option<Entry>,
    pub principles: Vec<Entry>,
    pub rules: Vec<RuleEntry>,
    pub attention: Vec<Attention>,
    pub latest_change: Option<LatestChange>,
}

/// Inputs the service computes before projecting.
pub struct ProjectionInput {
    pub project_label: String,
    pub agreement_complete: bool,
    pub onboarding: Onboarding,
    /// Shared record refs (id -> `id@revision#digest`) in the team's store:
    /// what `wh push` already shared and CI enforces.
    pub shared: BTreeMap<String, String>,
}

pub(crate) fn snippet(text: &str, limit: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut cut = text.chars().take(limit).collect::<String>();
    if let Some(space) = cut.rfind(' ') {
        cut.truncate(space);
    }
    format!("{cut}…")
}

/// Where in the gate ladder a rule runs.
pub fn runs_at(rule: &Rule) -> &'static str {
    if rule.enforcer.runs_staged() {
        "pre-commit, pre-push and CI"
    } else {
        "pre-push and CI"
    }
}

pub(crate) fn kind_of(record: &AgreementRecord) -> &'static str {
    match &record.body {
        RecordBody::Mission(_) => "mission",
        RecordBody::Principle(_) => "principle",
        RecordBody::Rule(_) | RecordBody::StoredRule(_) | RecordBody::Standard(_) => "rule",
        RecordBody::Retirement(_) => "retirement",
        _ => "record",
    }
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "mission" => "Mission",
        "principle" => "Principle",
        "rule" => "Rule",
        "retirement" => "Retirement",
        _ => "Record",
    }
}

/// The human title of a record body.
pub fn record_title(record: &AgreementRecord) -> String {
    match &record.body {
        RecordBody::Mission(body) => body.statement.clone(),
        RecordBody::Principle(body) => body.statement.clone(),
        RecordBody::Rule(body) => body.statement.clone(),
        RecordBody::StoredRule(body) => body.rule.statement.clone(),
        RecordBody::Standard(body) => body.statement.clone(),
        RecordBody::Retirement(body) => format!("Retire {}", body.target.id.as_str()),
        RecordBody::Proposal(body) => body.title.clone(),
        RecordBody::Retired(retired) => match retired.principle_text() {
            Some((text, _)) => text,
            None => format!("{} (retired kind)", retired.record_type.replace('_', " ")),
        },
        other => other.type_name().replace('_', " "),
    }
}

fn source_label(rule: &Rule) -> String {
    match &rule.source.reference {
        Some(reference) => format!("{}: {reference}", rule.source.kind.label()),
        None => rule.source.kind.label().to_string(),
    }
}

fn rule_definition(rule: &Rule) -> Value {
    json!({
        "type": "rule",
        "strength": rule.strength,
        "enforcer": rule.enforcer,
        "examples": rule.examples,
        "source": rule.source,
        "paths": rule.paths,
        "sweep": rule.sweep,
        "sweep_priority": rule.sweep_priority,
        "sweep_unsure": rule.sweep_unsure,
    })
}

fn edit_base(record: &AgreementRecord) -> EditBase {
    let (kind, content, definition) = match &record.body {
        RecordBody::Mission(body) => ("mission", body.statement.clone(), Value::Null),
        RecordBody::Principle(body) => (
            "principle",
            body.statement.clone(),
            // The exact shape `wh change --kind principle --definition` takes.
            json!({
                "type": "principle",
                "pstack": match &body.source {
                    crate::domain::PrincipleSource::Pstack { id, .. } => Some(id.clone()),
                    _ => None,
                },
                "rationale": body.rationale,
            }),
        ),
        body @ (RecordBody::Rule(_) | RecordBody::StoredRule(_) | RecordBody::Standard(_)) => {
            let rule = body.rule_view().map(std::borrow::Cow::into_owned);
            (
                "rule",
                rule.as_ref()
                    .map(|rule| rule.statement.clone())
                    .unwrap_or_default(),
                rule.as_ref().map_or(Value::Null, rule_definition),
            )
        }
        _ => ("record", String::new(), Value::Null),
    };
    EditBase {
        kind,
        record_id: record.id.as_str().into(),
        content,
        definition,
    }
}

fn detail(key: &'static str, value: impl Into<String>, code: bool) -> Detail {
    Detail {
        key,
        value: value.into(),
        code,
    }
}

fn details_for(record: &AgreementRecord) -> Vec<Detail> {
    match &record.body {
        body @ (RecordBody::Rule(_) | RecordBody::StoredRule(_) | RecordBody::Standard(_)) => {
            let Some(rule) = body.rule_view() else {
                return Vec::new();
            };
            let (mechanism, command) = crate::gates::mechanism_label(&rule);
            vec![
                detail("strength", rule.strength.label(), false),
                detail("enforcer", mechanism, false),
                detail("command", command, true),
                detail("runs at", runs_at(&rule), false),
            ]
        }
        RecordBody::Principle(body) => {
            let mut details = vec![detail(
                "source",
                match &body.source {
                    PrincipleSource::Pstack { id, version } => format!("pstack {id} ({version})"),
                    PrincipleSource::Custom => "the owner's own".into(),
                    PrincipleSource::Migrated { record } => {
                        format!("migrated from {}", record.id.as_str())
                    }
                },
                false,
            )];
            if let Some(rationale) = &body.rationale {
                details.push(detail("why", rationale.clone(), false));
            }
            details
        }
        _ => Vec::new(),
    }
}

pub(crate) fn entry(state: &AgreementState, id: &RecordId) -> Option<Entry> {
    let in_force = state.in_force(id);
    let pending = state.pending(id);
    let shown = in_force.or(pending)?;
    let lifecycle = if in_force.is_some() {
        Lifecycle::Accepted
    } else {
        Lifecycle::Draft
    };
    Some(Entry {
        id: id.as_str().into(),
        kind: kind_of(shown),
        title: record_title(shown),
        detail: details_for(shown),
        version: shown.revision,
        lifecycle: lifecycle.label(),
        pending: pending.filter(|_| in_force.is_some()).map(|draft| Pending {
            version: draft.revision,
            title: record_title(draft),
            proposal: state
                .proposal_for(draft)
                .map(|proposal| proposal.id.as_str().into()),
        }),
        state: None,
        owner: shown.owner.display_name.clone(),
        changed_at: shown.provenance.recorded_at.clone(),
        // Edits target the newest revision so a second edit revises the draft.
        edit: edit_base(pending.unwrap_or(shown)),
    })
}

fn strength_order(strength: &str) -> u8 {
    u8::from(strength != "must")
}

/// `id@revision#digest`, the stable text form of a record reference.
pub fn reference_text(reference: &RecordRef) -> String {
    format!(
        "{}@{}#{}",
        reference.id.as_str(),
        reference.revision,
        reference.digest.as_str()
    )
}

fn rule_entries(state: &AgreementState) -> Vec<RuleEntry> {
    let mut rules = Vec::new();
    for id in state.rule_ids() {
        let Some(base) = entry(state, &id) else {
            continue;
        };
        let Some(record) = state.in_force(&id).or_else(|| state.pending(&id)) else {
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        let (mechanism, command) = crate::gates::mechanism_label(&rule);
        rules.push(RuleEntry {
            entry: base,
            strength: rule.strength.label(),
            enforcer: mechanism,
            command,
            runs_at: runs_at(&rule),
            source: source_label(&rule),
            paths: rule.paths.clone(),
            examples: rule
                .examples
                .iter()
                .map(|example| {
                    json!({
                        "input": example.input,
                        "expected": example.expected,
                        "reason": example.reason,
                        "path": example.path,
                    })
                })
                .collect(),
            sweep_class: rule.sweep_class().label(),
            sweep_priority: rule.sweep_priority(),
            sweep_unsure: rule.sweep_unsure.clone(),
        });
    }
    rules.sort_by(|left, right| {
        (strength_order(left.strength), left.entry.title.as_str())
            .cmp(&(strength_order(right.strength), right.entry.title.as_str()))
    });
    rules
}

/// Build the dashboard projection.
pub fn dashboard_view(state: &AgreementState, input: &ProjectionInput) -> DashboardView {
    let mission_id = RecordId::new(MISSION_ID).expect("constant id");
    let mission = entry(state, &mission_id);
    let owner = mission
        .as_ref()
        .and_then(|entry| entry.owner.clone())
        .unwrap_or_else(|| "the project owner".into());
    let established = input.agreement_complete;
    let principles = state
        .agreement_ids(|body| matches!(body, RecordBody::Principle(_)))
        .iter()
        .filter_map(|id| entry(state, id))
        .collect::<Vec<_>>();
    let rules = rule_entries(state);
    let pending = state.pending_proposals();
    let drafts = pending.len();
    let attention = attention(&owner, &rules, drafts);
    DashboardView {
        established,
        header: Header {
            project: input.project_label.clone(),
            agreement: if established {
                "owner approved".into()
            } else if mission.is_some() {
                "incomplete".into()
            } else {
                "not initialised".into()
            },
            visibility: "private",
            drafts,
            team: if input.shared.is_empty() {
                "not shared"
            } else {
                "shared through Beads"
            },
        },
        onboarding: input.onboarding.clone(),
        mission,
        principles,
        rules,
        attention,
        latest_change: None,
    }
}

fn attention(owner: &str, rules: &[RuleEntry], drafts: usize) -> Vec<Attention> {
    let mut items = Vec::new();
    if rules.iter().all(|rule| rule.entry.lifecycle != "accepted") {
        items.push(Attention {
            priority: 1,
            tone: "warn",
            kind: "owner_decision",
            kind_label: "owner decision",
            title: "No rule is in force yet".into(),
            text: "Nothing is checked until at least one rule is accepted. Start with the three starter rules.".into(),
            actor: owner.into(),
            next: "Accept starter rules in onboarding, or record one with wh change --kind rule.".into(),
            route: "rules",
            focus: None,
            action_label: "Open rules",
            agent_instruction: None,
        });
    }
    if drafts > 0 {
        items.push(Attention {
            priority: 2,
            tone: "muted",
            kind: "review",
            kind_label: "review",
            title: format!(
                "{drafts} draft{} await{} your review",
                if drafts == 1 { "" } else { "s" },
                if drafts == 1 { "s" } else { "" }
            ),
            text: "Drafts are private and not in force until you accept them. Nothing has been shared.".into(),
            actor: owner.into(),
            next: "Accept or withdraw each draft in the changelog.".into(),
            route: "changelog",
            focus: None,
            action_label: "Open the changelog",
            agent_instruction: None,
        });
    }
    items.sort_by_key(|item| item.priority);
    items
}
