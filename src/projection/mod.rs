//! Typed, human-labelled projections of the rulebook file: the one-page
//! dashboard (`page`), the field-level diffs it shows (`diff`), and the
//! onboarding `wh init` reports.
//!
//! The browser renders these fields verbatim, so the UI never invents state
//! or shows raw enum names, and an edit not yet committed can never read as
//! accepted.

pub mod diff;
pub mod page;

use serde::Serialize;
use serde_json::{json, Value};

use crate::domain::Rule;
use crate::rulebook::Rulebook;

/// One onboarding step: mission, principles, starter rules, gates.
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

/// Onboarding on the rulebook file: the steps, the catalogue and the
/// starter rules, each accepted through `wh change`.
#[derive(Debug, Clone, Serialize)]
pub struct Start {
    pub steps: Vec<OnboardingStep>,
    pub missing: Vec<String>,
    pub command: &'static str,
    pub catalogue: Vec<crate::catalogue::CatalogueEntry>,
    /// The loaded principle packs as `name version`, comma separated.
    pub catalogue_version: String,
    /// Every loaded principle pack, the default (pstack) first.
    pub packs: Vec<crate::catalogue::PackSummary>,
    /// Packs in `whetstone/principles/` that could not be loaded, and why.
    pub refused_packs: Vec<crate::catalogue::RefusedPack>,
    pub starters: Vec<StarterView>,
}

/// Inputs the service computes before projecting.
pub struct ProjectionInput {
    pub hooks: crate::wire::GitHooks,
    pub starters: Vec<crate::starter::Starter>,
    /// The principle packs the onboarding picker offers.
    pub catalogue: crate::catalogue::Catalogue,
}

/// Where in the gate ladder a rule runs.
pub fn runs_at(rule: &Rule) -> &'static str {
    if rule.enforcer.runs_staged() {
        "pre-commit, pre-push and CI"
    } else {
        "pre-push and CI"
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

pub(crate) fn rule_edit(id: &str, rule: &Rule) -> EditBase {
    EditBase {
        kind: "rule",
        record_id: Some(id.into()),
        content: rule.statement.clone(),
        rationale: Some(rule.rationale.clone()),
        definition: rule_definition(rule),
    }
}

pub(crate) fn examples(rule: &Rule) -> Vec<Value> {
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

/// Onboarding for `wh init`: whether the working tree's rulebook has a
/// mission and a rule, and the steps, catalogue and starters.
pub fn onboarding(book: Option<&Rulebook>, input: ProjectionInput) -> (bool, Start) {
    let has_mission = book.is_some_and(|book| !book.mission.statement.trim().is_empty());
    let has_principles = book.is_some_and(|book| !book.principles.is_empty());
    let rule_ids = book
        .map(|book| book.rules.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<Vec<_>>();
    let established = has_mission && !rule_ids.is_empty();
    let mut missing = Vec::new();
    if !has_mission {
        missing.push("mission".to_string());
    }
    if rule_ids.is_empty() {
        missing.push("rules".to_string());
    }
    let catalogue = input.catalogue;
    let onboarding = Start {
        steps: vec![
            OnboardingStep {
                key: "mission",
                label: "Mission",
                hint: "what this project exists to do, in one sentence",
                done: has_mission,
                optional: false,
            },
            OnboardingStep {
                key: "principles",
                label: "Principles",
                hint: "the principles you hold from a pack, or your own",
                done: has_principles,
                optional: true,
            },
            OnboardingStep {
                key: "rules",
                label: "Rules",
                hint: "accept the starter rules and any proposed ones",
                done: !rule_ids.is_empty(),
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
        catalogue_version: catalogue.describe(),
        packs: catalogue.packs,
        refused_packs: catalogue.refused,
        catalogue: catalogue.principles,
        starters: input
            .starters
            .iter()
            .map(|starter| starter_view(starter, rule_ids.contains(&starter.id)))
            .collect(),
    };
    (established, onboarding)
}
