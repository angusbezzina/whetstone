//! Typed, human-labelled projection of the agreement for the dashboard's
//! views: Home, Rules, Checks, Changelog and Requests, plus onboarding.
//!
//! The browser renders these fields verbatim. Every state carries a human
//! label and a tone so the UI never invents state or shows raw enum names,
//! and unknown, stale, not-run, shadow and draft states can never read as a
//! pass.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{
    AgreementRecord, AttestationVerdict, Enforcer, Feature, Freshness, JudgmentOutcome,
    PrincipleSource, RecordBody, RecordId, RecordRef, Rule, VerificationAxis,
};
use crate::learning::RuleStats;
use crate::proof::{latest_gate_receipt, read_artifact, rule_ids, Changes, EVIDENCE_SYSTEM};

mod checks;
mod journal;

use checks::*;

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
    pub family: &'static str,
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

#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub area: String,
    pub sweep_order: u32,
    pub summary: String,
    pub user_path: String,
    pub proof: String,
    pub sub_features: Vec<String>,
    pub drive_steps: Vec<String>,
    pub gotchas: Vec<String>,
    pub entry_points: Vec<String>,
    pub serves: Vec<Link>,
    pub constrained_by: Vec<Link>,
    pub proven_by: Vec<Link>,
    pub proof_state: StateLabel,
    /// Entry-point paths changed since the feature was last proven.
    pub drift: Vec<String>,
}

/// A rule as the Rules view shows it: strength, enforcer, shadow status and
/// its record of flags.
#[derive(Debug, Clone, Serialize)]
pub struct RuleEntry {
    #[serde(flatten)]
    pub entry: Entry,
    pub strength: &'static str,
    pub family: &'static str,
    pub enforcer: String,
    pub command: String,
    pub shadow: bool,
    pub local_only: bool,
    pub runs_at: &'static str,
    pub source: String,
    pub paths: Vec<String>,
    pub examples: Vec<Value>,
    pub hand_raise: Vec<&'static str>,
    pub stats: RuleStats,
    /// Flags still waiting for the owner's label, newest first (at most 20):
    /// `receipt` is what `wh change --accept-flag|--dismiss-flag` takes.
    pub unlabelled_flags: Vec<Value>,
    pub result: StateLabel,
}

#[derive(Debug, Clone, Serialize)]
pub struct Failure {
    pub location: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceSummary {
    pub kind: String,
    pub locator: String,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateRun {
    pub id: String,
    pub name: String,
    pub strength: &'static str,
    pub family: &'static str,
    pub mechanism: String,
    pub command: String,
    pub eligible: bool,
    pub shadow: bool,
    pub result: StateLabel,
    pub last_run: Option<String>,
    pub current: bool,
    pub summary: Option<String>,
    pub failures: Vec<Failure>,
    pub recheck: String,
    pub brief: Option<String>,
    pub evidence: Vec<EvidenceSummary>,
    pub feature: Option<Link>,
    /// Accepted here versus shared with the team through `wh push`:
    /// "shared", "shared at another revision" or "private".
    pub team: &'static str,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Tally {
    pub pass: usize,
    pub fail: usize,
    pub unknown: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LastCheck {
    pub at: String,
    pub tally: Tally,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChecksView {
    pub last_complete: Option<LastCheck>,
    pub gates: Vec<GateRun>,
    pub advisory: usize,
    pub shadow: usize,
    pub receipts: Vec<EvidenceSummary>,
    pub driver: DriverState,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriverState {
    pub configured: bool,
    pub path: Option<String>,
    pub label: String,
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

/// A raised hand: the question an agent filed for the owner.
#[derive(Debug, Clone, Serialize)]
pub struct RequestEntry {
    pub id: String,
    pub issue: String,
    pub trigger: &'static str,
    pub rule: Option<String>,
    pub question: String,
    pub tried: String,
    pub recommendation: String,
    pub raised_at: String,
    pub status: StateLabel,
    pub answer: Option<String>,
    pub answered_by: Option<String>,
    pub answered_at: Option<String>,
    pub answer_command: String,
}

/// What the tracker says about a hand's Beads issue right now.
#[derive(Debug, Clone, Default)]
pub struct HandStatus {
    pub status: String,
    pub answer: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LatestChange {
    pub title: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillState {
    pub rendered: bool,
    pub current: bool,
    pub hosts: Vec<String>,
    pub rendered_at: Option<String>,
    pub label: StateLabel,
    /// Each host's copy compared with the manifest and the agreement.
    pub projections: Vec<crate::hosts::HostProjection>,
    /// The latest checkpoint per host: the skill revision it actually had.
    pub acknowledged: Vec<Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardView {
    pub established: bool,
    pub header: Header,
    pub onboarding: Onboarding,
    pub mission: Option<Entry>,
    pub principles: Vec<Entry>,
    pub rules: Vec<RuleEntry>,
    pub features: Vec<FeatureEntry>,
    pub checks: ChecksView,
    pub requests: Vec<RequestEntry>,
    pub attention: Vec<Attention>,
    pub latest_change: Option<LatestChange>,
    pub skill: SkillState,
    /// Deterministic map hygiene findings (see `crate::hygiene`).
    pub hygiene: Vec<crate::hygiene::HygieneFinding>,
    /// Drafts `wh change --tune` would record, with why.
    pub suggestions: Vec<crate::learning::Suggestion>,
}

/// Inputs the service computes before projecting.
pub struct ProjectionInput<'a> {
    pub project_label: String,
    pub project_root: &'a Path,
    pub evidence_root: &'a Path,
    pub agreement_complete: bool,
    pub onboarding: Onboarding,
    pub fingerprint: Option<&'a str>,
    pub driver_path: Option<String>,
    pub skill: Option<SkillManifest>,
    pub changes: Changes,
    pub hygiene: Vec<crate::hygiene::HygieneFinding>,
    /// Shared record refs (id -> `id@revision#digest`) in the team's store:
    /// what `wh push` already shared and CI enforces.
    pub shared: BTreeMap<String, String>,
    /// Beads issue id -> tracker status, for raised hands.
    pub hands: BTreeMap<String, HandStatus>,
}

/// Written by skill rendering; read to detect stale projections.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SkillManifest {
    pub agreement_digest: String,
    pub rendered_at: String,
    pub hosts: Vec<String>,
    pub files: Vec<SkillFile>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct SkillFile {
    pub path: String,
    pub digest: String,
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
    if rule.privacy.local_only && rule.enforcer.family() == crate::domain::EnforcerFamily::Question
    {
        return "review (local-only: Jev is never called)";
    }
    match rule.enforcer.family() {
        crate::domain::EnforcerFamily::Mechanical if rule.enforcer.runs_staged() => {
            "pre-commit, pre-push and CI"
        }
        crate::domain::EnforcerFamily::Mechanical => "pre-push and CI",
        crate::domain::EnforcerFamily::Question if rule.in_shadow() => {
            "pre-push and CI, in shadow (recorded, not enforced)"
        }
        crate::domain::EnforcerFamily::Question => "pre-push and CI",
        crate::domain::EnforcerFamily::Review => "pre-push and CI, once attested",
    }
}

pub(crate) fn kind_of(record: &AgreementRecord) -> &'static str {
    match &record.body {
        RecordBody::Mission(_) => "mission",
        RecordBody::Principle(_) => "principle",
        RecordBody::Rule(_) | RecordBody::Standard(_) | RecordBody::Guidance(_) => "rule",
        RecordBody::Feature(_) => "feature",
        RecordBody::VerificationMap(_) => "map",
        RecordBody::Retirement(_) => "retirement",
        _ => "record",
    }
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "mission" => "Mission",
        "principle" => "Principle",
        "rule" => "Rule",
        "feature" => "Feature",
        "map" => "Feature map",
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
        RecordBody::Guidance(body) => body.statement.clone(),
        RecordBody::Standard(body) => body.statement.clone(),
        RecordBody::Feature(body) => body.name.clone(),
        RecordBody::VerificationMap(body) => body.title.clone(),
        RecordBody::Retirement(body) => format!("Retire {}", body.target.id.as_str()),
        RecordBody::Proposal(body) => body.title.clone(),
        RecordBody::Judgment(body) => {
            format!("Jev {} on {}", body.outcome.label(), body.rule.id.as_str())
        }
        RecordBody::Attestation(body) => format!(
            "Review of {} by {}",
            body.rule.id.as_str(),
            body.reviewer.label()
        ),
        RecordBody::Brief(body) => format!("Brief for {}", body.area),
        RecordBody::FlagDecision(body) => {
            format!("Flag {} on {}", body.verdict.label(), body.rule.as_str())
        }
        RecordBody::HandRaise(body) => format!("Raised hand: {}", body.question),
        RecordBody::HandAnswer(body) => format!("Answered {}", body.issue),
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
        "hand_raise": rule.hand_raise,
        "privacy": rule.privacy,
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
        body @ (RecordBody::Rule(_) | RecordBody::Standard(_) | RecordBody::Guidance(_)) => {
            let rule = body.rule_view().map(std::borrow::Cow::into_owned);
            (
                "rule",
                rule.as_ref()
                    .map(|rule| rule.statement.clone())
                    .unwrap_or_default(),
                rule.as_ref().map_or(Value::Null, rule_definition),
            )
        }
        RecordBody::Feature(body) => (
            "feature",
            body.name.clone(),
            json!({
                "type": "feature",
                "summary": body.summary,
                "area": body.area,
                "sweep_order": body.sweep_order,
                "sub_features": body.sub_features,
                "user_path": body.user_path,
                "drive_steps": body.drive_steps,
                "proof": body.proof,
                "gotchas": body.gotchas,
                "entry_points": body.entry_points,
                "serves": body.serves,
                "constrained_by": body.constrained_by,
                "proven_by": body.proven_by,
                "index_summary": body.index_summary,
                "harness": body.harness,
                "preconditions": body.preconditions,
                "drive_recipe": body.drive_recipe,
                "mutations": body.mutations,
            }),
        ),
        RecordBody::VerificationMap(body) => (
            "map",
            body.title.clone(),
            json!({
                "type": "map",
                "intro": body.intro,
                "baseline_preconditions": body.baseline_preconditions,
                "driving_conventions": body.driving_conventions,
                "proof_reporting": body.proof_reporting,
                "entry_contract": body.entry_contract,
            }),
        ),
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
        body @ (RecordBody::Rule(_) | RecordBody::Standard(_) | RecordBody::Guidance(_)) => {
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
        RecordBody::Feature(body) => vec![
            detail("area", body.area.clone(), false),
            detail("summary", body.summary.clone(), false),
        ],
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

fn link(state: &AgreementState, id: &RecordId) -> Link {
    let record = state.in_force(id).or_else(|| state.pending(id));
    Link {
        id: id.as_str().into(),
        kind: record.map_or("record", kind_of),
        title: record.map_or_else(
            || id.as_str().into(),
            |record| snippet(&record_title(record), 80),
        ),
        resolved: record.is_some(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn gate_brief(
    name: &str,
    version: u64,
    strength: &str,
    command: &str,
    failures: &[Failure],
    recheck: &str,
    owner: &str,
    feature: Option<&FeatureEntry>,
) -> String {
    let mut lines = vec![
        format!("rule      {name} (v{version}, {strength})"),
        format!("command   {command}"),
    ];
    if !failures.is_empty() {
        lines.push(format!(
            "failures  {}",
            failures
                .iter()
                .take(8)
                .map(|failure| failure.location.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let Some(feature) = feature {
        lines.push(format!(
            "feature   {} — {}",
            feature.entry.title, feature.user_path
        ));
        lines.push(format!("prove     {}", feature.proof));
        if let Ok(id) = RecordId::new(feature.entry.id.as_str()) {
            lines.push(format!(
                "map       features/{}.md in the verify skill (read it before driving)",
                crate::feature_map::feature_slug(&id)
            ));
        }
        lines.push("proof bar the real user path, the action and its resulting state, side effects checked, evidence that survives cleanup; no evidence is unknown".into());
    }
    lines.push("repair    within the current task; do not change or weaken the rule".into());
    lines.push(format!("recheck   {recheck}"));
    lines.push(format!(
        "stop      if the same finding survives a second repair, or the fix needs a rule change, raise a hand for {owner} (wh check --raise-hand)"
    ));
    lines.join("\n")
}

fn strength_order(strength: &str) -> u8 {
    match strength {
        "must" => 0,
        "should" => 1,
        _ => 2,
    }
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

fn features(
    state: &AgreementState,
    gates: &BTreeMap<String, StateLabel>,
    changes: &Changes,
) -> Vec<FeatureEntry> {
    let heads = crate::proof::proof_heads(state);
    let mut result = Vec::new();
    for id in state.agreement_ids(|body| matches!(body, RecordBody::Feature(_))) {
        let Some(base) = entry(state, &id) else {
            continue;
        };
        let Some(record) = state.in_force(&id).or_else(|| state.pending(&id)) else {
            continue;
        };
        let RecordBody::Feature(feature) = &record.body else {
            continue;
        };
        let proving = crate::proof::proving_rules(state, &id, feature);
        let proof_state = proving
            .iter()
            .filter_map(|gate| gates.get(gate.as_str()))
            .min_by_key(|label| match label.tone {
                "fail" => 0,
                "warn" => 1,
                "draft" => 2,
                "muted" => 3,
                _ => 4,
            })
            .cloned()
            .unwrap_or_else(|| {
                if proving.is_empty() {
                    StateLabel::new("warn", "no proving rule")
                } else {
                    StateLabel::new("warn", "not run")
                }
            });
        let drift = if state.pending(&id).is_some() {
            Vec::new()
        } else {
            crate::proof::drifted_paths(state, &id, feature, &heads, changes)
        };
        let mut entry = feature_entry(state, base, feature, proof_state);
        if !drift.is_empty() && entry.proof_state.tone != "fail" {
            entry.entry.state = Some(StateLabel::new("warn", "changed · re-prove"));
        }
        entry.drift = drift;
        result.push(entry);
    }
    result.sort_by(|left, right| {
        (
            left.area.as_str(),
            left.sweep_order,
            left.entry.title.as_str(),
        )
            .cmp(&(
                right.area.as_str(),
                right.sweep_order,
                right.entry.title.as_str(),
            ))
    });
    result
}

fn feature_entry(
    state: &AgreementState,
    mut base: Entry,
    feature: &Feature,
    proof_state: StateLabel,
) -> FeatureEntry {
    base.state = Some(proof_state.clone());
    FeatureEntry {
        entry: base,
        area: feature.area.clone(),
        sweep_order: feature.sweep_order,
        summary: feature.summary.clone(),
        user_path: feature.user_path.clone(),
        proof: feature.proof.clone(),
        sub_features: feature.sub_features.clone(),
        drive_steps: feature.drive_steps.clone(),
        gotchas: feature.gotchas.clone(),
        entry_points: feature.entry_points.clone(),
        serves: feature.serves.iter().map(|id| link(state, id)).collect(),
        constrained_by: feature
            .constrained_by
            .iter()
            .map(|id| link(state, id))
            .collect(),
        proven_by: feature.proven_by.iter().map(|id| link(state, id)).collect(),
        proof_state,
        drift: Vec::new(),
    }
}

fn rule_entries(state: &AgreementState, runs: &[GateRun]) -> Vec<RuleEntry> {
    let mut rules = Vec::new();
    for id in rule_ids(state) {
        let Some(mut base) = entry(state, &id) else {
            continue;
        };
        let Some(record) = state.in_force(&id).or_else(|| state.pending(&id)) else {
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        let (mechanism, command) = crate::gates::mechanism_label(&rule);
        let result = runs.iter().find(|run| run.id == id.as_str()).map_or_else(
            || StateLabel::new("warn", "not run"),
            |run| run.result.clone(),
        );
        base.state = Some(result.clone());
        rules.push(RuleEntry {
            entry: base,
            strength: rule.strength.label(),
            family: rule.enforcer.family().label(),
            enforcer: mechanism,
            command,
            shadow: rule.in_shadow(),
            local_only: rule.privacy.local_only,
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
            hand_raise: rule
                .hand_raise
                .iter()
                .map(|trigger| trigger.label())
                .collect(),
            stats: crate::learning::rule_stats(state, &id),
            unlabelled_flags: crate::learning::flags(state, &id)
                .into_iter()
                .rev()
                .filter(|flag| flag.verdict.is_none())
                .take(20)
                .map(|flag| {
                    json!({
                        "receipt": flag.receipt.id.as_str(),
                        "at": flag.at,
                        "unit": flag.unit,
                        "commit": flag.commit,
                        "shadow": flag.shadow,
                    })
                })
                .collect(),
            result,
        });
    }
    rules.sort_by(|left, right| {
        (strength_order(left.strength), left.entry.title.as_str())
            .cmp(&(strength_order(right.strength), right.entry.title.as_str()))
    });
    rules
}

fn requests(state: &AgreementState, hands: &BTreeMap<String, HandStatus>) -> Vec<RequestEntry> {
    let mut answers = BTreeMap::<RecordRef, &crate::domain::HandAnswer>::new();
    for record in state.records() {
        if let RecordBody::HandAnswer(answer) = &record.body {
            answers.insert(answer.raise.clone(), answer);
        }
    }
    let mut result = Vec::new();
    for record in state.records() {
        let RecordBody::HandRaise(raise) = &record.body else {
            continue;
        };
        let Ok(reference) = record.reference() else {
            continue;
        };
        let answer = answers.get(&reference);
        let tracker = hands.get(&raise.issue);
        let status = match (answer, tracker.map(|status| status.status.as_str())) {
            (Some(_), _) => StateLabel::new("pass", "answered"),
            (None, Some("closed")) => {
                StateLabel::new("warn", "closed in Beads · answer not recorded")
            }
            (None, Some(_)) => StateLabel::new("warn", "waiting for the owner"),
            (None, None) => StateLabel::new("warn", "waiting · issue not found in Beads"),
        };
        result.push(RequestEntry {
            id: record.id.as_str().into(),
            issue: raise.issue.clone(),
            trigger: raise.trigger.label(),
            rule: raise.rule.as_ref().map(|rule| rule.as_str().to_string()),
            question: raise.question.clone(),
            tried: raise.tried.clone(),
            recommendation: raise.recommendation.clone(),
            raised_at: raise.raised_at.clone(),
            status,
            answer: answer
                .map(|answer| answer.answer.clone())
                .or_else(|| tracker.and_then(|status| status.answer.clone())),
            answered_by: answer.map(|answer| answer.answered_by.clone()),
            answered_at: answer.map(|answer| answer.answered_at.clone()),
            answer_command: format!(
                "wh change --answer {} --content \"<your answer>\"",
                raise.issue
            ),
        });
    }
    result.sort_by(|left, right| {
        (left.answer.is_some(), right.raised_at.as_str())
            .cmp(&(right.answer.is_some(), left.raised_at.as_str()))
    });
    result
}

/// Build the dashboard projection.
pub fn dashboard_view(state: &AgreementState, input: &ProjectionInput<'_>) -> DashboardView {
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
    // Provisional: rule labels needed by features before runs exist.
    let placeholder = BTreeMap::new();
    let provisional_features = features(state, &placeholder, &Changes::default());
    let (gate_runs, last_complete) = gate_runs(state, input, &provisional_features, &owner);
    let gate_labels = gate_runs
        .iter()
        .map(|run| (run.id.clone(), run.result.clone()))
        .collect::<BTreeMap<_, _>>();
    let features = features(state, &gate_labels, &input.changes);
    let rules = rule_entries(state, &gate_runs);
    let requests = requests(state, &input.hands);
    let pending = state.pending_proposals();
    let drafts = pending.len();
    let mut skill = skill_state(state, input.skill.as_ref());
    skill.projections = crate::hosts::projections(
        input.project_root,
        input.skill.as_ref(),
        &state.in_force_digest(),
    );
    skill.acknowledged = host_acknowledgements(state, &skill.projections);
    let driver = DriverState {
        configured: input.driver_path.is_some(),
        label: if input.driver_path.is_some() {
            "verification driver present".into()
        } else {
            "no verification driver yet".into()
        },
        path: input.driver_path.clone(),
    };
    let receipts = state
        .records()
        .iter()
        .rev()
        .filter(|record| record.body.is_receipt())
        .take(8)
        .filter_map(|record| {
            let reference = record.reference().ok()?;
            Some(EvidenceSummary {
                kind: record.body.type_name().replace('_', " "),
                locator: format!("{} · {}", record.id.as_str(), record.provenance.recorded_at),
                digest: Some(reference.digest.as_str().into()),
            })
        })
        .collect();
    let suggestions = crate::learning::suggestions(state);
    let attention = attention(
        state,
        input,
        &owner,
        &gate_runs,
        &features,
        &rules,
        &requests,
        &suggestions,
        drafts,
        &skill,
    );
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
        features,
        checks: ChecksView {
            last_complete,
            advisory: gate_runs
                .iter()
                .filter(|run| run.eligible && run.strength == "advisory")
                .count(),
            shadow: gate_runs
                .iter()
                .filter(|run| run.eligible && run.shadow)
                .count(),
            gates: gate_runs,
            receipts,
            driver,
        },
        requests,
        attention,
        latest_change: None,
        skill,
        hygiene: input.hygiene.clone(),
        suggestions,
    }
}

/// The latest host checkpoint receipts, and whether the host has checked in
/// since its skill or the adapter changed (if not, rerun the regression set).
fn host_acknowledgements(
    state: &AgreementState,
    projections: &[crate::hosts::HostProjection],
) -> Vec<Value> {
    let mut latest = BTreeMap::<String, &AgreementRecord>::new();
    for record in state.records() {
        if let RecordBody::VerificationReceipt(body) = &record.body {
            if let Some(host) = body
                .subject
                .stable_id
                .strip_prefix(crate::hosts::HOST_SUBJECT_PREFIX)
            {
                let newer = latest.get(host).map_or(true, |current| {
                    current.provenance.recorded_at < record.provenance.recorded_at
                });
                if newer {
                    latest.insert(host.to_string(), record);
                }
            }
        }
    }
    let mut result = Vec::new();
    for projection in projections {
        let receipt = latest
            .get(&projection.host)
            .and_then(|record| match &record.body {
                RecordBody::VerificationReceipt(body) => Some(body),
                _ => None,
            });
        let acknowledged_digest = receipt.and_then(|body| body.subject.revision.clone());
        let adapter = receipt.and_then(|body| {
            body.evidence
                .iter()
                .find(|evidence| evidence.system == "whetstone_adapter")
                .map(|evidence| evidence.locator.clone())
        });
        let in_step = acknowledged_digest.is_some()
            && acknowledged_digest == projection.skill_digest
            && adapter.as_deref() == Some(crate::hosts::ADAPTER_VERSION);
        result.push(json!({
            "host": projection.host,
            "last_checkpoint": receipt.map(|body| body.checked_at.clone()),
            "acknowledged_skill_digest": acknowledged_digest,
            "adapter": adapter,
            "state": if receipt.is_none() {
                "not acknowledged"
            } else if in_step {
                "acknowledged"
            } else {
                "changed since last checkpoint"
            },
            "regression": (!in_step && receipt.is_some())
                .then_some("The skill or the adapter changed since this host last checked in: run wh check --sweep before relying on earlier results."),
        }));
    }
    result
}

fn skill_state(state: &AgreementState, manifest: Option<&SkillManifest>) -> SkillState {
    match manifest {
        None => SkillState {
            rendered: false,
            current: false,
            hosts: Vec::new(),
            rendered_at: None,
            label: StateLabel::new("warn", "not generated"),
            projections: Vec::new(),
            acknowledged: Vec::new(),
        },
        Some(manifest) => {
            let current = manifest.agreement_digest == state.in_force_digest();
            SkillState {
                rendered: true,
                current,
                hosts: manifest.hosts.clone(),
                rendered_at: Some(manifest.rendered_at.clone()),
                label: if current {
                    StateLabel::new("pass", "current")
                } else {
                    StateLabel::new("warn", "stale · regenerate")
                },
                projections: Vec::new(),
                acknowledged: Vec::new(),
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn attention(
    state: &AgreementState,
    input: &ProjectionInput<'_>,
    owner: &str,
    gates: &[GateRun],
    features: &[FeatureEntry],
    rules: &[RuleEntry],
    requests: &[RequestEntry],
    suggestions: &[crate::learning::Suggestion],
    drafts: usize,
    skill: &SkillState,
) -> Vec<Attention> {
    let mut items = Vec::new();
    let root = input.project_root.display().to_string();
    for request in requests.iter().filter(|request| request.answer.is_none()) {
        items.push(Attention {
            priority: 0,
            tone: "warn",
            kind: "raised_hand",
            kind_label: "raised hand",
            title: format!("An agent asks: {}", snippet(&request.question, 90)),
            text: format!(
                "Tried: {} Recommends: {}",
                snippet(&request.tried, 160),
                snippet(&request.recommendation, 160)
            ),
            actor: owner.into(),
            next: format!("Answer it: {}", request.answer_command),
            route: "requests",
            focus: Some(request.id.clone()),
            action_label: "Open the request",
            agent_instruction: None,
        });
    }
    for gate in gates
        .iter()
        .filter(|gate| gate.eligible && !gate.shadow && gate.result.tone == "fail")
    {
        let count = gate.failures.len();
        items.push(Attention {
            priority: if gate.strength == "must" { 0 } else { 2 },
            tone: "fail",
            kind: "agent_repair",
            kind_label: "agent repair",
            title: format!("Failing rule: {}", gate.name.trim_end_matches('.')),
            text: format!(
                "{} in the latest check. The worker that made the change repairs it within task scope, then runs the exact check again.",
                if count == 1 {
                    "1 failure".to_string()
                } else if count > 1 {
                    format!("{count} failures")
                } else {
                    "The rule failed".to_string()
                }
            ),
            actor: "your coding agent".into(),
            next: "Hand the brief to the agent; return here when it has rerun the check.".into(),
            route: "checks",
            focus: Some(gate.id.clone()),
            action_label: "Open the failing rule",
            agent_instruction: gate.brief.clone(),
        });
    }
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
    for gate in gates
        .iter()
        .filter(|gate| gate.eligible && !gate.shadow && gate.result.tone == "warn")
    {
        items.push(Attention {
            priority: 1,
            tone: "warn",
            kind: "stale_result",
            kind_label: "not current",
            title: if gate.family == "review" {
                // A check cannot run a review: someone attests it.
                format!("Not attested yet: {}", gate.name.trim_end_matches('.'))
            } else if gate.last_run.is_some() {
                format!("Not current: {}", gate.name.trim_end_matches('.'))
            } else {
                format!("Never run: {}", gate.name.trim_end_matches('.'))
            },
            text: "A result that predates the current code, or no result at all, is not a pass."
                .into(),
            actor: "you or your agent".into(),
            next: if gate.family == "review" {
                format!(
                    "Review the change (/interrogate or the named reviewer), then record it with wh check --attest {} --verdict pass|fail.",
                    gate.id
                )
            } else {
                format!("Run {} on the current revision.", gate.recheck)
            },
            route: "checks",
            focus: Some(gate.id.clone()),
            action_label: "Open checks",
            agent_instruction: None,
        });
    }
    for suggestion in suggestions {
        items.push(Attention {
            priority: 2,
            tone: "muted",
            kind: "tuning",
            kind_label: suggestion.kind,
            title: format!(
                "Suggested {}: {}",
                suggestion.kind,
                state
                    .in_force(
                        &RecordId::new(suggestion.rule.as_str()).unwrap_or_else(|_| {
                            RecordId::new("unknown.rule").expect("constant id")
                        })
                    )
                    .map_or_else(|| suggestion.rule.clone(), record_title)
            ),
            text: suggestion.reason.clone(),
            actor: owner.into(),
            next: "Record it as a draft with wh change --tune, then accept or withdraw it.".into(),
            route: "rules",
            focus: Some(suggestion.rule.clone()),
            action_label: "Open the rule",
            agent_instruction: None,
        });
    }
    for feature in features.iter().filter(|feature| !feature.drift.is_empty()) {
        items.push(Attention {
            priority: 1,
            tone: "warn",
            kind: "feature_drift",
            kind_label: "map review",
            title: format!("Behaviour may have moved: {}", feature.entry.title),
            text: format!(
                "{} changed under this feature since it was last proven: {}.",
                if feature.drift.len() == 1 { "One file" } else { "Files" },
                feature
                    .drift
                    .iter()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            actor: "you or your agent".into(),
            next: format!(
                "Re-prove with wh check --feature {}; if the behaviour moved, revise the feature entry in the same change.",
                feature.entry.id
            ),
            route: "rules",
            focus: Some(feature.entry.id.clone()),
            action_label: "Open the feature",
            agent_instruction: Some(format!(
                "In {root}, run `wh check --feature {id}`. If it fails because the behaviour intentionally moved, update the feature record with `wh change --kind feature --record-id {id}` (user path, drive steps, proof) as a draft for the owner; if it fails because the product regressed, repair the product instead. Never edit the map to hide a regression.",
                id = feature.entry.id
            )),
        });
    }
    let mut by_feature = BTreeMap::<&str, Vec<&crate::hygiene::HygieneFinding>>::new();
    for finding in &input.hygiene {
        by_feature
            .entry(finding.feature.as_str())
            .or_default()
            .push(finding);
    }
    for (feature, found) in by_feature.into_iter().take(6) {
        let title = state
            .in_force(
                &RecordId::new(feature)
                    .unwrap_or_else(|_| RecordId::new("unknown").expect("constant")),
            )
            .map_or_else(|| feature.to_string(), record_title);
        items.push(Attention {
            priority: 2,
            tone: "warn",
            kind: "map_hygiene",
            kind_label: "map hygiene",
            title: format!("Feature map: {}", snippet(&title, 70)),
            text: found
                .iter()
                .map(|finding| finding.detail.clone())
                .collect::<Vec<_>>()
                .join(" "),
            actor: "you or your agent".into(),
            next: found[0].next.clone(),
            route: "rules",
            focus: RecordId::new(feature).ok().map(|id| id.as_str().to_string()),
            action_label: "Open the feature",
            agent_instruction: Some(format!(
                "In {root}: {} Change records only through wh change; never edit product code to satisfy the map.",
                found.iter().map(|finding| finding.next.clone()).collect::<Vec<_>>().join(" ")
            )),
        });
    }
    if !skill.rendered || !skill.current {
        items.push(Attention {
            priority: 2,
            tone: "muted",
            kind: "verification_skill",
            kind_label: "verification skill",
            title: if skill.rendered {
                "The verification skill is stale".into()
            } else {
                "Agents have no verification skill yet".into()
            },
            text: "The skill is the agent-facing projection of the rules: how to brief, build, prove and raise a hand.".into(),
            actor: "you or your agent".into(),
            next: "Run wh init --action wire --hooks to generate it and install the Git hooks.".into(),
            route: "checks",
            focus: None,
            action_label: "Open checks",
            agent_instruction: Some(format!("In {root}, run `wh init --action wire --hooks --dry-run`, review the exact writes, then run it without --dry-run.")),
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
