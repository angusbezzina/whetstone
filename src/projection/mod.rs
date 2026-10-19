//! Typed, human-labelled projection of the rulebook file for the
//! dashboard's views: Home, Rules and Changelog, plus onboarding.
//!
//! The browser renders these fields verbatim. Every state carries a human
//! label and a tone so the UI never invents state or shows raw enum names,
//! and an edit not yet committed can never read as accepted.

use serde::Serialize;
use serde_json::{json, Value};

use crate::domain::{AgreementRecord, Principle, PrincipleSource, RecordBody, Rule};
use crate::rulebook::{self, RetiredEntry, Rulebook, RULEBOOK_PATH};

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

/// One onboarding step: tools, mission, principles, starter rules, gates.
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
    /// The rule as `wh change` takes it, so accepting a starter is a change.
    pub edit: EditBase,
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

/// What an edit form starts from and sends back to `wh change`: the kind,
/// the id (none for the mission), the text, the why and the definition.
#[derive(Debug, Clone, Serialize)]
pub struct EditBase {
    pub kind: &'static str,
    pub record_id: Option<String>,
    pub content: String,
    pub rationale: Option<String>,
    pub definition: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Header {
    pub project: String,
    pub agreement: &'static str,
    /// The commit that last touched the rulebook, or `uncommitted`.
    pub revision: String,
    pub drafts: usize,
}

/// The mission, a principle or a rule as the working tree's rulebook holds
/// it, and how that compares with the rulebook as last committed.
#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub detail: Vec<Detail>,
    /// `accepted` (as committed), `changed` (committed, edited since) or
    /// `new` (never committed).
    pub lifecycle: &'static str,
    pub state: StateLabel,
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
    pub rationale: String,
    pub paths: Vec<String>,
    pub examples: Vec<Value>,
    /// How a night sweep works this rule's flags: draft_pr, confirm_first or
    /// discuss (the default).
    pub sweep_class: &'static str,
    /// How urgent its sweep findings are, 0 (most) to 4.
    pub sweep_priority: u8,
    /// Where its own flags are doubtful, with the reason each says.
    pub sweep_unsure: Vec<crate::domain::SweepUnsure>,
    pub digest: String,
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

/// Onboarding on the rulebook file: the steps, the catalogue and the
/// starter rules, each accepted through `wh change`.
#[derive(Debug, Clone, Serialize)]
pub struct Start {
    pub steps: Vec<OnboardingStep>,
    pub missing: Vec<String>,
    pub command: &'static str,
    pub catalogue: Vec<crate::catalogue::CatalogueEntry>,
    pub catalogue_version: String,
    pub starters: Vec<StarterView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardView {
    pub established: bool,
    pub header: Header,
    pub onboarding: Start,
    pub mission: Option<Entry>,
    pub principles: Vec<Entry>,
    pub rules: Vec<RuleEntry>,
    /// The edits accepting would commit, as `wh change --accept` lists them.
    pub drafts: Vec<String>,
    pub retired: Vec<RetiredEntry>,
    pub attention: Vec<Attention>,
    pub latest_change: Option<LatestChange>,
}

/// Inputs the service computes before projecting.
pub struct ProjectionInput<'a> {
    pub project_label: String,
    /// The rulebook as last committed, when it ever was.
    pub head: Option<&'a Rulebook>,
    pub drafts: Vec<String>,
    pub hooks: crate::wire::GitHooks,
    pub starters: Vec<crate::starter::Starter>,
}

/// Where in the gate ladder a rule runs.
pub fn runs_at(rule: &Rule) -> &'static str {
    if rule.enforcer.runs_staged() {
        "pre-commit, pre-push and CI"
    } else {
        "pre-push and CI"
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

/// A rule's definition in the exact shape `wh change --definition` takes.
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

fn rule_edit(id: &str, rule: &Rule) -> EditBase {
    EditBase {
        kind: "rule",
        record_id: Some(id.into()),
        content: rule.statement.clone(),
        rationale: Some(rule.rationale.clone()),
        definition: rule_definition(rule),
    }
}

fn examples(rule: &Rule) -> Vec<Value> {
    rule.examples
        .iter()
        .map(|example| {
            json!({
                "input": example.input,
                "expected": example.expected,
                "reason": example.reason,
                "path": example.path,
            })
        })
        .collect()
}

/// A starter rule as onboarding offers it.
pub fn starter_view(starter: &crate::starter::Starter, accepted: bool) -> StarterView {
    let (mechanism, _) = crate::gates::mechanism_label(&starter.rule);
    StarterView {
        id: starter.id.into(),
        title: starter.title.into(),
        detected: starter.detected.clone(),
        strength: starter.rule.strength.label(),
        enforcer: mechanism,
        examples: starter
            .rule
            .examples
            .iter()
            .map(|example| json!({"input": example.input, "expected": example.expected, "reason": example.reason}))
            .collect(),
        accepted,
        edit: rule_edit(starter.id, &starter.rule),
    }
}

fn detail(key: &'static str, value: impl Into<String>, code: bool) -> Detail {
    Detail {
        key,
        value: value.into(),
        code,
    }
}

fn principle_details(principle: &Principle) -> Vec<Detail> {
    let mut details = vec![detail(
        "source",
        match &principle.source {
            PrincipleSource::Pstack { id, version } => format!("pstack {id} ({version})"),
            PrincipleSource::Custom => "the owner's own".into(),
            PrincipleSource::Migrated { record } => {
                format!("migrated from {}", record.id.as_str())
            }
        },
        false,
    )];
    if let Some(rationale) = &principle.rationale {
        details.push(detail("why", rationale.clone(), false));
    }
    details
}

fn state_of(lifecycle: &'static str) -> StateLabel {
    match lifecycle {
        "accepted" => StateLabel::new("pass", "accepted"),
        "changed" => StateLabel::new("draft", "edited · not accepted"),
        _ => StateLabel::new("draft", "new · not accepted"),
    }
}

fn strength_order(strength: &str) -> u8 {
    u8::from(strength != "must")
}

/// Build the dashboard projection from the working tree's rulebook (absent
/// before the mission is first written).
pub fn dashboard_view(book: Option<&Rulebook>, input: ProjectionInput<'_>) -> DashboardView {
    let lifecycle = |id: &str| -> &'static str {
        let now = book.and_then(|book| rulebook::entry(book, id));
        match input.head.and_then(|head| rulebook::entry(head, id)) {
            None => "new",
            Some(then) if Some(&then) == now.as_ref() => "accepted",
            Some(_) => "changed",
        }
    };
    let entry = |id: &str, kind: &'static str, title: String, detail, edit| {
        let lifecycle = lifecycle(id);
        Entry {
            id: id.into(),
            kind,
            title,
            detail,
            lifecycle,
            state: state_of(lifecycle),
            edit,
        }
    };
    let mission = book
        .filter(|book| !book.mission.statement.trim().is_empty())
        .map(|book| {
            entry(
                "mission",
                "mission",
                book.mission.statement.clone(),
                Vec::new(),
                EditBase {
                    kind: "mission",
                    record_id: None,
                    content: book.mission.statement.clone(),
                    rationale: None,
                    definition: Value::Null,
                },
            )
        });
    let principles = book
        .map(|book| book.principles.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|principle| {
            let id = principle.id.as_str();
            let body = &principle.principle;
            entry(
                id,
                "principle",
                body.statement.clone(),
                principle_details(body),
                EditBase {
                    kind: "principle",
                    record_id: Some(id.into()),
                    content: body.statement.clone(),
                    rationale: body.rationale.clone(),
                    definition: json!({
                        "type": "principle",
                        "pstack": match &body.source {
                            PrincipleSource::Pstack { id, .. } => Some(id.clone()),
                            _ => None,
                        },
                        "rationale": body.rationale,
                    }),
                },
            )
        })
        .collect::<Vec<_>>();
    let mut rules = book
        .map(|book| book.rules.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|item| {
            let id = item.id.as_str();
            let rule = &item.rule;
            let (mechanism, command) = crate::gates::mechanism_label(rule);
            RuleEntry {
                entry: entry(
                    id,
                    "rule",
                    rule.statement.clone(),
                    vec![
                        detail("strength", rule.strength.label(), false),
                        detail("enforcer", mechanism.clone(), false),
                        detail("command", command.clone(), true),
                        detail("runs at", runs_at(rule), false),
                    ],
                    rule_edit(id, rule),
                ),
                strength: rule.strength.label(),
                enforcer: mechanism,
                command,
                runs_at: runs_at(rule),
                source: source_label(rule),
                rationale: rule.rationale.clone(),
                paths: rule.paths.clone(),
                examples: examples(rule),
                sweep_class: rule.sweep_class().label(),
                sweep_priority: rule.sweep_priority(),
                sweep_unsure: rule.sweep_unsure.clone(),
                digest: item.digest.as_str().into(),
            }
        })
        .collect::<Vec<_>>();
    rules.sort_by(|left, right| {
        (strength_order(left.strength), left.entry.title.as_str())
            .cmp(&(strength_order(right.strength), right.entry.title.as_str()))
    });
    let established = mission.is_some() && !rules.is_empty();
    let mut missing = Vec::new();
    if mission.is_none() {
        missing.push("mission".to_string());
    }
    if rules.is_empty() {
        missing.push("rules".to_string());
    }
    let catalogue = crate::catalogue::catalogue();
    let onboarding = Start {
        steps: vec![
            OnboardingStep {
                key: "mission",
                label: "Mission",
                hint: "what this project exists to do, in one sentence",
                done: mission.is_some(),
                optional: false,
            },
            OnboardingStep {
                key: "principles",
                label: "Principles",
                hint: "the catalogue principles you hold, or your own",
                done: !principles.is_empty(),
                optional: true,
            },
            OnboardingStep {
                key: "rules",
                label: "Rules",
                hint: "accept the starter rules and any proposed ones",
                done: !rules.is_empty(),
                optional: false,
            },
            OnboardingStep {
                key: "gates",
                label: "Gates",
                hint: "pre-commit and pre-push hooks that run the rules",
                done: input.hooks.pre_commit && input.hooks.pre_push,
                optional: false,
            },
        ],
        missing,
        command: "wh change --kind mission --content \"…\"",
        catalogue_version: catalogue.version.clone(),
        catalogue: catalogue.principles,
        starters: input
            .starters
            .iter()
            .map(|starter| {
                starter_view(
                    starter,
                    rules.iter().any(|rule| rule.entry.id == starter.id),
                )
            })
            .collect(),
    };
    let drafts = input.drafts.len();
    let accepted = rules.iter().any(|rule| rule.entry.lifecycle != "new");
    let attention = if drafts > 0 {
        vec![Attention {
            priority: if accepted { 2 } else { 1 },
            tone: if accepted { "muted" } else { "warn" },
            kind: "review",
            kind_label: "review",
            title: format!(
                "{drafts} edit{} to {RULEBOOK_PATH} wait{} for you",
                if drafts == 1 { "" } else { "s" },
                if drafts == 1 { "s" } else { "" }
            ),
            text: "wh check already reads them on this machine; CI enforces only what is committed. Accepting commits the file.".into(),
            actor: "the project owner".into(),
            next: "Accept or withdraw them in Rules.".into(),
            route: "rules",
            focus: Some("drafts".into()),
            action_label: "Review drafts",
            agent_instruction: None,
        }]
    } else {
        Vec::new()
    };
    DashboardView {
        established,
        header: Header {
            project: input.project_label,
            agreement: match (book, input.head) {
                (None, _) => "not started",
                (Some(_), None) => "drafted, not accepted",
                (Some(_), Some(_)) if drafts > 0 => "accepted, with drafts",
                (Some(_), Some(_)) => "accepted",
            },
            revision: book.map_or("none", |book| book.revision.label()).into(),
            drafts,
        },
        onboarding,
        mission,
        principles,
        rules,
        drafts: input.drafts,
        retired: book.map(|book| book.retired.clone()).unwrap_or_default(),
        attention,
        latest_change: None,
    }
}
