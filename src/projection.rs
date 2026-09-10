//! Typed, human-labelled projection of the local agreement for the four
//! dashboard views: Dashboard, Foundations, Checks and Changelog.
//!
//! The browser renders these fields verbatim. Every state carries a human
//! label and a tone so the UI never invents state or shows raw enum names,
//! and unknown, stale, not-run and draft states can never read as a pass.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::{json, Value};

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{
    AgreementRecord, Enforcement, Feature, Freshness, LocalReviewVerdict, RecordBody, RecordId,
    RecordRef, Standard, StandardStrength, VerificationAxis,
};
use crate::history::HistoryInspection;

pub const MISSION_ID: &str = "mission.project";
pub const VALUES_ID: &str = "value.core";
pub const PHILOSOPHY_ID: &str = "philosophy.implementation";
pub const SAFEGUARD_ID: &str = "guidance.initial-safeguard";
pub const INITIAL_GATE_ID: &str = "standard.initial-gate";
pub const GATE_SUBJECT_PREFIX: &str = "gate:";
pub const EVIDENCE_SYSTEM: &str = "whetstone_evidence";

/// The eight owner decisions `wh init` records, in the order they are asked.
pub const ONBOARDING_DECISIONS: [(&str, &str, &str); 8] = [
    ("mission", "Mission", "what this project exists to do"),
    (
        "desired outcome",
        "Desired outcome",
        "how you will know it is working",
    ),
    (
        "core values",
        "Core values",
        "what guides choices when they conflict",
    ),
    (
        "implementation philosophy",
        "Engineering philosophy",
        "how work gets done here",
    ),
    (
        "accountable owner",
        "Accountable owner",
        "who decides consequential changes",
    ),
    (
        "initial safeguard",
        "First gate",
        "one deterministic check you already trust",
    ),
    (
        "initial safeguard scope",
        "Gate scope",
        "where that check applies",
    ),
    (
        "revision triggers",
        "Review triggers",
        "when to revisit all of this",
    ),
];

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StateLabel {
    pub tone: &'static str,
    pub label: String,
}

impl StateLabel {
    fn new(tone: &'static str, label: impl Into<String>) -> Self {
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

#[derive(Debug, Clone, Serialize)]
pub struct OnboardingDecision {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    pub done: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Onboarding {
    pub decisions: Vec<OnboardingDecision>,
    pub missing: Vec<String>,
    pub command: &'static str,
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
    pub desired_outcome: Option<String>,
    pub review_triggers: Option<String>,
    pub definition: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub kind: &'static str,
    pub title: String,
    pub detail: Vec<Detail>,
    pub outcomes: Vec<String>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct Stages {
    pub mission: Vec<Entry>,
    pub values: Vec<Entry>,
    pub metrics: Vec<Entry>,
    pub rules: Vec<Entry>,
    pub gates: Vec<Entry>,
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
    pub mechanism: String,
    pub command: String,
    pub eligible: bool,
    pub result: StateLabel,
    pub last_run: Option<String>,
    pub current: bool,
    pub summary: Option<String>,
    pub failures: Vec<Failure>,
    pub recheck: String,
    pub brief: Option<String>,
    pub evidence: Vec<EvidenceSummary>,
    pub feature: Option<Link>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct DashboardView {
    pub established: bool,
    pub header: Header,
    pub onboarding: Onboarding,
    pub mission: Option<Entry>,
    pub stages: Stages,
    pub features: Vec<FeatureEntry>,
    pub checks: ChecksView,
    pub attention: Vec<Attention>,
    pub latest_change: Option<LatestChange>,
    pub skill: SkillState,
}

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

/// Inputs the service computes before projecting.
pub struct ProjectionInput<'a> {
    pub project_label: String,
    pub project_root: &'a Path,
    pub evidence_root: &'a Path,
    pub agreement_complete: bool,
    pub missing_decisions: &'a [String],
    pub fingerprint: Option<&'a str>,
    pub driver_path: Option<String>,
    pub skill: Option<SkillManifest>,
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

fn snippet(text: &str, limit: usize) -> String {
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

pub fn strength_label(strength: StandardStrength) -> &'static str {
    match strength {
        StandardStrength::Must => "must",
        StandardStrength::Should => "should",
        StandardStrength::May => "may",
    }
}

pub fn mechanism_label(standard: &Standard) -> (String, String) {
    match &standard.enforcement {
        Enforcement::Test { command_ref } => ("Test".into(), command_ref.clone()),
        Enforcement::Validator { command_ref } => ("Validator".into(), command_ref.clone()),
        Enforcement::Formatter { tool } => ("Formatter".into(), tool.clone()),
        Enforcement::LintProxy { tool, code } => ("Lint".into(), format!("{tool} ({code})")),
        Enforcement::Ast { query } => ("AST query".into(), query.clone()),
        Enforcement::Drive { feature } => ("Drive".into(), format!("prove {}", feature.as_str())),
    }
}

fn kind_of(record: &AgreementRecord) -> &'static str {
    match &record.body {
        RecordBody::Mission(_) => "mission",
        RecordBody::CoreValue(_) => "value",
        RecordBody::ImplementationPhilosophy(_) => "philosophy",
        RecordBody::Guidance(_) => "guidance",
        RecordBody::MetricDefinition(_) => "metric",
        RecordBody::Standard(_) => "standard",
        RecordBody::Feature(_) => "feature",
        _ => "record",
    }
}

pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "mission" => "Mission",
        "value" => "Core value",
        "philosophy" => "Engineering philosophy",
        "guidance" => "Guideline",
        "metric" => "Metric",
        "standard" => "Gate",
        "feature" => "Feature",
        _ => "Record",
    }
}

/// The human title of an agreement record body.
pub fn record_title(record: &AgreementRecord) -> String {
    match &record.body {
        RecordBody::Mission(body) => body.statement.clone(),
        RecordBody::CoreValue(body) => body.description.clone(),
        RecordBody::ImplementationPhilosophy(body) => body.statement.clone(),
        RecordBody::Guidance(body) => body.statement.clone(),
        RecordBody::MetricDefinition(body) => body.name.clone(),
        RecordBody::Standard(body) => body.statement.clone(),
        RecordBody::Feature(body) => body.name.clone(),
        RecordBody::Proposal(body) => body.title.clone(),
        other => other.type_name().replace('_', " "),
    }
}

fn edit_base(record: &AgreementRecord) -> EditBase {
    let (kind, content, desired_outcome, review_triggers, definition) = match &record.body {
        RecordBody::Mission(body) => (
            "mission",
            body.statement.clone(),
            body.desired_outcomes.first().cloned(),
            None,
            Value::Null,
        ),
        RecordBody::CoreValue(body) => ("value", body.description.clone(), None, None, Value::Null),
        RecordBody::ImplementationPhilosophy(body) => (
            "philosophy",
            body.statement.clone(),
            None,
            body.review_triggers.first().cloned(),
            Value::Null,
        ),
        RecordBody::Guidance(body) => ("guidance", body.statement.clone(), None, None, Value::Null),
        RecordBody::MetricDefinition(body) => (
            "metric",
            body.name.clone(),
            None,
            None,
            json!({
                "type": "metric",
                "source_system": body.source.system,
                "source_locator": body.source.locator,
                "cohort": body.cohort,
                "window": body.window,
                "direction": body.direction,
                "threshold": body.threshold,
                "freshness_seconds": body.freshness_seconds,
            }),
        ),
        RecordBody::Standard(body) => (
            "standard",
            body.statement.clone(),
            None,
            None,
            json!({
                "type": "standard",
                "strength": body.strength,
                "enforcement": body.enforcement,
            }),
        ),
        RecordBody::Feature(body) => (
            "feature",
            body.name.clone(),
            None,
            None,
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
            }),
        ),
        _ => ("record", String::new(), None, None, Value::Null),
    };
    EditBase {
        kind,
        record_id: record.id.as_str().into(),
        content,
        desired_outcome,
        review_triggers,
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
        RecordBody::MetricDefinition(body) => vec![
            detail(
                "target",
                format!(
                    "{} to {}",
                    format!("{:?}", body.direction).to_ascii_lowercase(),
                    body.threshold
                ),
                false,
            ),
            detail("window", body.window.clone(), false),
            detail(
                "source",
                format!("{}:{}", body.source.system, body.source.locator),
                true,
            ),
            detail("cohort", body.cohort.clone(), false),
        ],
        RecordBody::Standard(body) => {
            let (mechanism, command) = mechanism_label(body);
            vec![
                detail("strength", strength_label(body.strength), false),
                detail("mechanism", mechanism, false),
                detail("command", command, true),
            ]
        }
        RecordBody::ImplementationPhilosophy(body) => {
            let mut details = vec![detail("kind", "philosophy", false)];
            if let Some(trigger) = body.review_triggers.first() {
                details.push(detail("review when", trigger.clone(), false));
            }
            details
        }
        RecordBody::Guidance(body) => vec![
            detail("kind", "guidance", false),
            detail("why", body.rationale.clone(), false),
        ],
        RecordBody::Feature(body) => vec![
            detail("area", body.area.clone(), false),
            detail("summary", body.summary.clone(), false),
        ],
        _ => Vec::new(),
    }
}

fn entry(state: &AgreementState, id: &RecordId) -> Option<Entry> {
    let in_force = state.in_force(id);
    let pending = state.pending(id);
    let shown = in_force.or(pending)?;
    let lifecycle = if in_force.is_some() {
        Lifecycle::Accepted
    } else {
        Lifecycle::Draft
    };
    let outcomes = match &shown.body {
        RecordBody::Mission(body) => body.desired_outcomes.clone(),
        _ => Vec::new(),
    };
    Some(Entry {
        id: id.as_str().into(),
        kind: kind_of(shown),
        title: record_title(shown),
        detail: details_for(shown),
        outcomes,
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

/// Latest per-gate receipt for a standard id, if any.
fn latest_gate_receipt<'a>(
    state: &'a AgreementState,
    standard_id: &RecordId,
) -> Option<&'a AgreementRecord> {
    let subject = format!("{GATE_SUBJECT_PREFIX}{}", standard_id.as_str());
    state
        .records()
        .iter()
        .filter(|record| {
            matches!(&record.body, RecordBody::VerificationReceipt(body) if body.subject.stable_id == subject)
        })
        .max_by(|left, right| {
            left.provenance
                .recorded_at
                .cmp(&right.provenance.recorded_at)
                .then_with(|| left.id.cmp(&right.id))
        })
}

#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
pub struct GateArtifact {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub failures: Vec<ArtifactFailure>,
    #[serde(default)]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Default, serde::Deserialize, Serialize)]
pub struct ArtifactFailure {
    pub location: String,
    pub message: String,
}

fn read_artifact(evidence_root: &Path, locator: &str) -> Option<GateArtifact> {
    if locator.contains("..") || locator.starts_with('/') {
        return None;
    }
    let path = evidence_root.join(locator);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > 256 * 1024 {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
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
        format!("gate      {name} (v{version}, {strength})"),
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
    }
    lines.push("repair    within the current task; do not change or weaken the gate".into());
    lines.push(format!("recheck   {recheck}"));
    lines.push(format!(
        "stop      if the fix needs a policy or boundary change, hand back to {owner}"
    ));
    lines.join("\n")
}

fn gate_runs(
    state: &AgreementState,
    input: &ProjectionInput<'_>,
    features: &[FeatureEntry],
    owner: &str,
) -> (Vec<GateRun>, Option<LastCheck>) {
    let mut runs = Vec::new();
    let mut last_at: Option<String> = None;
    let mut tally = Tally::default();
    for id in state.agreement_ids(|body| matches!(body, RecordBody::Standard(_))) {
        let in_force = state.in_force(&id);
        let Some(shown) = in_force.or_else(|| state.pending(&id)) else {
            continue;
        };
        let RecordBody::Standard(standard) = &shown.body else {
            continue;
        };
        let (mechanism, command) = mechanism_label(standard);
        let feature = match &standard.enforcement {
            Enforcement::Drive { feature } => Some(link(state, feature)),
            _ => None,
        };
        let eligible = in_force.is_some();
        let receipt = latest_gate_receipt(state, &id);
        let (result, last_run, current, summary, failures, evidence) = match receipt
            .map(|record| (record, &record.body))
        {
            Some((_, RecordBody::VerificationReceipt(body))) => {
                let bound = shown.reference().ok().is_some_and(|reference| {
                    body.policy_state.accepted.as_ref() == Some(&reference)
                });
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
                let label = match (body.verification, current) {
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
                if last_at
                    .as_deref()
                    .map_or(true, |at| body.checked_at.as_str() > at)
                {
                    last_at = Some(body.checked_at.clone());
                }
                (
                    label,
                    Some(body.checked_at.clone()),
                    current,
                    artifact.as_ref().map(|artifact| artifact.summary.clone()),
                    failures,
                    body.evidence
                        .iter()
                        .map(|evidence| EvidenceSummary {
                            kind: evidence.system.clone(),
                            locator: evidence.locator.clone(),
                            digest: evidence.digest.as_ref().map(|d| d.as_str().to_string()),
                        })
                        .collect(),
                )
            }
            _ if !eligible => (
                StateLabel::new("draft", "draft · not run"),
                None,
                false,
                None,
                Vec::new(),
                Vec::new(),
            ),
            _ => (
                StateLabel::new("warn", "not run"),
                None,
                false,
                None,
                Vec::new(),
                Vec::new(),
            ),
        };
        if eligible {
            match result.tone {
                "pass" => tally.pass += 1,
                "fail" => tally.fail += 1,
                _ => tally.unknown += 1,
            }
        }
        let recheck = format!("wh check --rule {}", id.as_str());
        let feature_entry = feature
            .as_ref()
            .and_then(|link| features.iter().find(|entry| entry.entry.id == link.id));
        let brief = (result.tone == "fail").then(|| {
            gate_brief(
                &standard.statement,
                shown.revision,
                strength_label(standard.strength),
                &command,
                &failures,
                &recheck,
                owner,
                feature_entry,
            )
        });
        runs.push(GateRun {
            id: id.as_str().into(),
            name: standard.statement.clone(),
            strength: strength_label(standard.strength),
            mechanism,
            command,
            eligible,
            result,
            last_run,
            current,
            summary,
            failures,
            recheck,
            brief,
            evidence,
            feature,
        });
    }
    runs.sort_by(|left, right| {
        (!left.eligible, left.name.as_str()).cmp(&(!right.eligible, right.name.as_str()))
    });
    (runs, last_at.map(|at| LastCheck { at, tally }))
}

fn features(state: &AgreementState, gates: &BTreeMap<String, StateLabel>) -> Vec<FeatureEntry> {
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
        let proof_state = feature
            .proven_by
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
                if feature.proven_by.is_empty() {
                    StateLabel::new("warn", "no proving gate")
                } else {
                    StateLabel::new("warn", "not run")
                }
            });
        result.push(feature_entry(state, base, feature, proof_state));
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
    }
}

fn metric_state(state: &AgreementState, record: &AgreementRecord) -> StateLabel {
    let Ok(reference) = record.reference() else {
        return StateLabel::new("warn", "unknown");
    };
    let observation = state
        .records()
        .iter()
        .filter(|candidate| {
            matches!(&candidate.body, RecordBody::ObservationReceipt(body) if body.metric.id == reference.id)
        })
        .max_by_key(|candidate| candidate.provenance.recorded_at.as_str());
    match observation.map(|record| &record.body) {
        Some(RecordBody::ObservationReceipt(body)) => match (body.freshness, body.outcome) {
            (Freshness::Fresh, crate::domain::OutcomeAxis::Harmed) => StateLabel::new(
                "fail",
                format!("{} · off target", snippet(&body.summary, 24)),
            ),
            (Freshness::Fresh, crate::domain::OutcomeAxis::Unknown) => {
                StateLabel::new("warn", "observed · outcome unknown")
            }
            (Freshness::Fresh, _) => StateLabel::new(
                "pass",
                format!("{} · on target", snippet(&body.summary, 24)),
            ),
            (Freshness::Stale, _) => StateLabel::new("warn", "observation stale"),
            (Freshness::Missing, _) => StateLabel::new("warn", "not observed"),
        },
        _ => StateLabel::new("warn", "not observed"),
    }
}

/// Build the four-view projection. `None` when no private store exists.
pub fn dashboard_view(state: &AgreementState, input: &ProjectionInput<'_>) -> DashboardView {
    let mission_id = RecordId::new(MISSION_ID).expect("constant id");
    let mission = entry(state, &mission_id);
    let owner = mission
        .as_ref()
        .and_then(|entry| entry.owner.clone())
        .unwrap_or_else(|| "the project owner".into());
    let established = input.agreement_complete;

    // Values: every in-force or drafted core value record.
    let values = state
        .agreement_ids(|body| matches!(body, RecordBody::CoreValue(_)))
        .iter()
        .filter_map(|id| entry(state, id))
        .collect::<Vec<_>>();

    let metrics = state
        .agreement_ids(|body| matches!(body, RecordBody::MetricDefinition(_)))
        .iter()
        .filter_map(|id| {
            let mut entry = entry(state, id)?;
            let record = state.in_force(id).or_else(|| state.pending(id))?;
            entry.state = Some(if state.in_force(id).is_some() {
                metric_state(state, record)
            } else {
                StateLabel::new("draft", "draft · not observed")
            });
            Some(entry)
        })
        .collect::<Vec<_>>();

    let has_initial_gate = state
        .in_force(&RecordId::new(INITIAL_GATE_ID).expect("constant id"))
        .is_some();
    let mut rules = state
        .agreement_ids(|body| {
            matches!(
                body,
                RecordBody::ImplementationPhilosophy(_) | RecordBody::Guidance(_)
            )
        })
        .iter()
        .filter(|id| id.as_str() != SAFEGUARD_ID)
        .filter_map(|id| entry(state, id))
        .collect::<Vec<_>>();
    rules.sort_by_key(|entry| (entry.kind != "philosophy", entry.id.clone()));

    // Provisional: gate labels needed by features before gate runs exist.
    let placeholder = BTreeMap::new();
    let provisional_features = features(state, &placeholder);
    let (gate_runs, last_complete) = gate_runs(state, input, &provisional_features, &owner);
    let gate_labels = gate_runs
        .iter()
        .map(|run| (run.id.clone(), run.result.clone()))
        .collect::<BTreeMap<_, _>>();
    let features = features(state, &gate_labels);

    let mut gates = state
        .agreement_ids(|body| matches!(body, RecordBody::Standard(_)))
        .iter()
        .filter_map(|id| {
            let mut entry = entry(state, id)?;
            entry.state = gate_labels.get(id.as_str()).cloned();
            Some(entry)
        })
        .collect::<Vec<_>>();
    // The onboarding safeguard is the first gate. Without a mechanism it is
    // shown honestly as unenforced, never as a passing gate.
    if !has_initial_gate {
        if let Some(mut safeguard) = entry(state, &RecordId::new(SAFEGUARD_ID).expect("id")) {
            safeguard.kind = "guidance";
            safeguard.state = Some(StateLabel::new("warn", "no command yet"));
            safeguard.detail = vec![detail(
                "mechanism",
                "none recorded; advisory until a command is set",
                false,
            )];
            gates.insert(0, safeguard);
        }
    }

    let advisory = rules.len();
    let pending = state.pending_proposals();
    let drafts = pending.len();
    let skill = skill_state(state, input.skill.as_ref());
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
        .filter(|record| matches!(record.body, RecordBody::VerificationReceipt(_)))
        .take(6)
        .filter_map(|record| {
            let reference = record.reference().ok()?;
            Some(EvidenceSummary {
                kind: "verification receipt".into(),
                locator: format!("{} · {}", record.id.as_str(), record.provenance.recorded_at),
                digest: Some(reference.digest.as_str().into()),
            })
        })
        .collect();
    let attention = attention(
        state,
        input,
        &owner,
        &metrics,
        &gate_runs,
        &features,
        drafts,
        has_initial_gate,
        &skill,
    );
    let stages = Stages {
        mission: mission.iter().cloned().collect(),
        values,
        metrics,
        rules,
        gates,
    };
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
            team: "not configured",
        },
        onboarding: Onboarding {
            decisions: ONBOARDING_DECISIONS
                .iter()
                .map(|(key, label, hint)| OnboardingDecision {
                    key,
                    label,
                    hint,
                    done: !input.missing_decisions.iter().any(|missing| {
                        missing == key || missing.trim_start_matches("one ") == *key
                    }),
                })
                .collect(),
            missing: input.missing_decisions.to_vec(),
            command: "wh init",
        },
        mission,
        stages,
        features,
        checks: ChecksView {
            last_complete,
            gates: gate_runs,
            advisory,
            receipts,
            driver,
        },
        attention,
        latest_change: None,
        skill,
    }
}

fn skill_state(state: &AgreementState, manifest: Option<&SkillManifest>) -> SkillState {
    match manifest {
        None => SkillState {
            rendered: false,
            current: false,
            hosts: Vec::new(),
            rendered_at: None,
            label: StateLabel::new("warn", "not generated"),
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
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn attention(
    state: &AgreementState,
    input: &ProjectionInput<'_>,
    owner: &str,
    metrics: &[Entry],
    gates: &[GateRun],
    features: &[FeatureEntry],
    drafts: usize,
    has_initial_gate: bool,
    skill: &SkillState,
) -> Vec<Attention> {
    let mut items = Vec::new();
    let root = input.project_root.display().to_string();
    for gate in gates
        .iter()
        .filter(|gate| gate.eligible && gate.result.tone == "fail")
    {
        let count = gate.failures.len();
        items.push(Attention {
            priority: if gate.strength == "must" { 0 } else { 2 },
            tone: "fail",
            kind: "agent_repair",
            kind_label: "agent repair",
            title: format!("Failing gate: {}", gate.name.trim_end_matches('.')),
            text: format!(
                "{} in the latest check. The worker that made the change repairs it within task scope, then runs the exact check again.",
                if count == 1 {
                    "1 failure".to_string()
                } else if count > 1 {
                    format!("{count} failures")
                } else {
                    "The gate failed".to_string()
                }
            ),
            actor: "your coding agent".into(),
            next: "Hand the brief to the agent; return here when it has rerun the check.".into(),
            route: "checks",
            focus: Some(gate.id.clone()),
            action_label: "Open the failing gate",
            agent_instruction: gate.brief.clone(),
        });
    }
    if state
        .agreement_ids(|body| matches!(body, RecordBody::MetricDefinition(_)))
        .is_empty()
    {
        items.push(Attention {
            priority: 1,
            tone: "warn",
            kind: "owner_decision",
            kind_label: "owner decision",
            title: "No key metric is defined".into(),
            text: "Without a versioned metric and source, nobody can tell whether the mission is working.".into(),
            actor: owner.into(),
            next: "Define one mission-linked metric in Foundations.".into(),
            route: "foundations",
            focus: Some("st-metrics".into()),
            action_label: "Add a metric",
            agent_instruction: None,
        });
    }
    if !has_initial_gate && gates.iter().all(|gate| !gate.eligible) {
        items.push(Attention {
            priority: 1,
            tone: "warn",
            kind: "owner_decision",
            kind_label: "owner decision",
            title: "The first gate has no command".into(),
            text: "The onboarding safeguard is advisory until a deterministic command enforces it."
                .into(),
            actor: owner.into(),
            next: "Give the first gate a command in Foundations, then accept the draft.".into(),
            route: "foundations",
            focus: Some("st-gates".into()),
            action_label: "Set the gate command",
            agent_instruction: None,
        });
    }
    for metric in metrics
        .iter()
        .filter(|metric| metric.lifecycle == "accepted")
    {
        if let Some(label) = metric.state.as_ref().filter(|label| label.tone != "pass") {
            items.push(Attention {
                priority: 1,
                tone: label.tone,
                kind: "outcome_signal",
                kind_label: "outcome signal",
                title: format!("{} is {}", metric.title, label.label),
                text: "Without a fresh observation the mission's success is unknown, not fine."
                    .into(),
                actor: metric.owner.clone().unwrap_or_else(|| owner.into()),
                next: "Connect the source or revise the definition through review.".into(),
                route: "foundations",
                focus: Some(metric.id.clone()),
                action_label: "Open the metric",
                agent_instruction: None,
            });
        }
    }
    for gate in gates
        .iter()
        .filter(|gate| gate.eligible && gate.result.tone == "warn")
    {
        items.push(Attention {
            priority: 1,
            tone: "warn",
            kind: "stale_result",
            kind_label: "not current",
            title: if gate.last_run.is_some() {
                format!("Not current: {}", gate.name.trim_end_matches('.'))
            } else {
                format!("Never run: {}", gate.name.trim_end_matches('.'))
            },
            text: "A result that predates the current code, or no result at all, is not a pass."
                .into(),
            actor: "you or your agent".into(),
            next: format!("Run {} on the current revision.", gate.recheck),
            route: "checks",
            focus: Some(gate.id.clone()),
            action_label: "Open checks",
            agent_instruction: None,
        });
    }
    if features.is_empty() {
        items.push(Attention {
            priority: 2,
            tone: "muted",
            kind: "feature_map",
            kind_label: "feature map",
            title: "No feature is mapped yet".into(),
            text: "Agents need a feature map to reach, drive and prove the app without reading its source.".into(),
            actor: "your coding agent".into(),
            next: "Add features in Foundations, or ask your agent to map the top three.".into(),
            route: "foundations",
            focus: Some("st-features".into()),
            action_label: "Open features",
            agent_instruction: Some(format!(
                "In {root}, map the three most important user-facing features with `wh change --kind feature` (name, area, user path, drive steps, proof, entry points, and the gates that prove them). Record each as a private draft for the owner to accept."
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
            text: "The skill is the agent-facing projection of this agreement: how to launch, drive and prove the app, and why each feature exists.".into(),
            actor: "you or your agent".into(),
            next: "Run wh init --action wire to generate it for your agent hosts.".into(),
            route: "checks",
            focus: None,
            action_label: "Open checks",
            agent_instruction: Some(format!("In {root}, run `wh init --action wire --dry-run`, review the exact writes, then run it without --dry-run.")),
        });
    }
    if drafts > 0 {
        items.push(Attention {
            priority: 2,
            tone: "muted",
            kind: "review",
            kind_label: "review",
            title: format!(
                "{drafts} local draft{} await{} your review",
                if drafts == 1 { "" } else { "s" },
                if drafts == 1 { "s" } else { "" }
            ),
            text: "Drafts are private and inactive until you accept them. Nothing has been shared or installed.".into(),
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

fn area_of(record: &AgreementRecord) -> &'static str {
    match &record.body {
        RecordBody::Mission(_) => "mission",
        RecordBody::CoreValue(_) => "values",
        RecordBody::MetricDefinition(_) | RecordBody::ObservationReceipt(_) => "metrics",
        RecordBody::ImplementationPhilosophy(_) | RecordBody::Guidance(_) => "rules",
        RecordBody::Standard(_) => "gates",
        RecordBody::Feature(_) => "features",
        RecordBody::VerificationReceipt(_)
        | RecordBody::RepairSession(_)
        | RecordBody::RepairHandoff(_)
        | RecordBody::RepairAuthorityReservation(_)
        | RecordBody::RepairOperationClaim(_) => "checks",
        _ => "governance",
    }
}

/// Group history items into human journal entries, newest first.
pub fn journal(state: &AgreementState, history: Option<&HistoryInspection>) -> Vec<JournalEntry> {
    let Some(history) = history else {
        return Vec::new();
    };
    let mut groups = BTreeMap::<String, Vec<crate::history::HistoryItem>>::new();
    for item in &history.decision_history.items {
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
            "Private record withheld".into(),
            StateLabel::new("muted", "redacted"),
            "governance",
            "This history item is not visible from this boundary.".into(),
        );
    }
    // Verification runs: gate receipts and native scans.
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
        let gates = records
            .iter()
            .filter_map(|record| match &record.body {
                RecordBody::VerificationReceipt(body) => body
                    .subject
                    .stable_id
                    .strip_prefix(GATE_SUBJECT_PREFIX)
                    .map(str::to_owned),
                _ => None,
            })
            .collect::<Vec<_>>();
        return base(
            "verification",
            format!(
                "Checks ran: {} pass · {} fail · {} unknown",
                tally.pass, tally.fail, tally.unknown
            ),
            status,
            "checks",
            if gates.is_empty() {
                "Compiled-in scanner run.".into()
            } else {
                format!("Gates: {}", gates.join(", "))
            },
        );
    }
    if records.iter().all(|record| {
        matches!(
            record.body,
            RecordBody::RepairSession(_)
                | RecordBody::RepairHandoff(_)
                | RecordBody::RepairAuthorityReservation(_)
                | RecordBody::RepairOperationClaim(_)
        )
    }) {
        return base(
            "verification",
            "Agent repair session recorded".into(),
            StateLabel::new("muted", "recorded"),
            "checks",
            "A bounded repair attempt and its exact recheck were recorded.".into(),
        );
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
            "Foundations established".into(),
            StateLabel::new("pass", "accepted"),
            "mission",
            "Mission, outcome, core values, engineering philosophy, accountable owner and the first gate, recorded as one wh init operation.".into(),
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
            .map_or_else(|| "Governance record".into(), |record| record_title(record));
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
    entry.note = superseded_by.map(|revision| format!("superseded by v{revision}"));
    if lifecycle == Lifecycle::Draft && superseded_by.is_none() {
        entry.proposal = state
            .proposal_for(candidate)
            .map(|proposal| proposal.id.as_str().to_string());
    }
    entry
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
