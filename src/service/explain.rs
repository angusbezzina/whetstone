//! `wh explain <fingerprint | rule id>`: the rule behind a finding, its exact
//! enforcer, what it covers, how a sweep works it, the principle it serves
//! and the decisions that accepted and last changed it. Read-only.
//!
//! A sweep finding's fingerprint is resolved from a findings file when one is
//! given (or `findings.json` sits at the repository root), and otherwise by
//! recomputing fingerprints for every rule over the repository's files, so a
//! fingerprint copied from a pull request or an issue resolves on any clone.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{AgreementRecord, PrincipleSource, RecordBody, RecordId, Rule};
use crate::storage::ProjectLayout;

use super::check::findings;
use super::{
    bounded_request_id, load_records, project_error, storage_error, unknown_response,
    ServiceResponse, ServiceState,
};

#[derive(Debug, Clone)]
pub struct ExplainRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    /// A finding fingerprint (`sha256:...` or bare hex) or a rule id.
    pub target: String,
    /// A findings file to resolve the fingerprint from.
    pub findings: Option<PathBuf>,
}

/// The most `#n` suffixes tried when recomputing a fingerprint.
const MAX_SUBJECT_REPEATS: usize = 20;

pub(crate) fn explain(request: ExplainRequest) -> ServiceResponse {
    let layout = match ProjectLayout::resolve(&request.project_dir) {
        Ok(layout) => layout,
        Err(error) => return project_error("explain", request.request_id, error),
    };
    let request_id = match bounded_request_id(request.request_id.clone(), "explain".into()) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response("explain", "invalid-request-id".into(), summary),
    };
    let target = request.target.trim().to_string();
    if target.is_empty() || target.len() > 300 {
        return needs(
            request_id,
            "Name a finding fingerprint or a rule id to explain.".into(),
        );
    }
    let loaded = match load_records(&layout) {
        Ok(loaded) => loaded,
        Err(error) => return storage_error("explain", request_id, error),
    };
    let state = AgreementState::from_records(loaded.union().0);
    let root = layout.project_root();

    let rule_ids = state.agreement_ids(|body| body.rule_view().is_some());
    let (rule_id, resolved) = if let Some(id) = rule_ids.iter().find(|id| id.as_str() == target) {
        (id.clone(), json!({"kind": "rule", "rule": id.as_str()}))
    } else if looks_like_fingerprint(&target) {
        let fingerprint = normalize(&target);
        let found = match from_findings_file(root, request.findings.as_deref(), &fingerprint) {
            Ok(found) => found,
            Err(error) => return needs(request_id, error),
        };
        let found = found.or_else(|| recompute(root, &rule_ids, &fingerprint));
        let Some((rule, subject, source)) = found else {
            return needs(
                request_id,
                format!(
                    "No rule here produces the finding {fingerprint}. Pass the sweep's findings file with --findings, or check that this is the repository the sweep ran on."
                ),
            );
        };
        let Ok(id) = RecordId::new(rule.as_str()) else {
            return needs(
                request_id,
                format!("The finding names {rule}, which is not a rule id."),
            );
        };
        (
            id,
            json!({"kind": "fingerprint", "fingerprint": fingerprint, "subject": subject, "resolved_from": source}),
        )
    } else {
        return needs(
            request_id,
            format!(
                "{target} is neither a rule in this agreement nor a finding fingerprint (sha256:...). Rules here: {}.",
                rule_ids
                    .iter()
                    .map(RecordId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    };

    let Some(record) = state.in_force(&rule_id).or_else(|| state.latest(&rule_id)) else {
        return needs(
            request_id,
            format!("{} has no stored revision.", rule_id.as_str()),
        );
    };
    let Some(rule) = record.body.rule_view() else {
        return needs(request_id, format!("{} is not a rule.", rule_id.as_str()));
    };
    let lifecycle = if state.is_retired(&rule_id) {
        "retired"
    } else if state.in_force(&rule_id).is_some() {
        "in force"
    } else {
        state.lifecycle_of(record).label()
    };
    let principle = principle_of(&state, &rule);
    let decisions = decisions(&state, &rule_id);
    let created = decisions
        .iter()
        .find(|decision| decision["verdict"] == "accept" || decision["verdict"] == "agreed")
        .cloned();
    let last_changed = decisions
        .iter()
        .rev()
        .find(|decision| decision["verdict"] == "accept" || decision["verdict"] == "agreed")
        .cloned();
    let (mechanism, command) = crate::gates::mechanism_label(&rule);
    let rule_json = json!({
        "id": rule_id.as_str(),
        "revision": record.revision,
        "lifecycle": lifecycle,
        "statement": rule.statement,
        "rationale": rule.rationale,
        "strength": rule.strength.label(),
        "enforcer": rule.enforcer,
        "mechanism": mechanism,
        "command": command,
        "runs_at": crate::projection::runs_at(&rule),
        "paths": rule.paths,
        "sweep": {
            "class": rule.sweep_class().label(),
            "priority": rule.sweep_priority(),
            "unsure": rule.sweep_unsure,
        },
        "shadow": rule.in_shadow(),
        "source": rule.source,
    });
    let text = render_text(
        &rule_id,
        &rule,
        lifecycle,
        &resolved,
        principle.as_ref(),
        &decisions,
    );
    let mut response = ServiceResponse::new(
        request_id,
        "explain",
        ServiceState::Success,
        format!(
            "{} ({}, {}): {}",
            rule_id.as_str(),
            rule.strength.label(),
            lifecycle,
            rule.statement
        ),
    );
    response.permitted_actions = vec![
        "repair the code the finding names, then wh check".into(),
        format!(
            "wh change --kind rule --record-id {} ... --probe --rationale <why> (a person accepts)",
            rule_id.as_str()
        ),
    ];
    response.data = json!({
        "target": target,
        "resolved": resolved,
        "rule": rule_json,
        "principle": principle,
        "decisions": decisions,
        "created": created,
        "last_changed": last_changed,
        "text": text,
    });
    response
}

fn needs(request_id: String, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "explain",
        ServiceState::NeedsInput,
        summary.clone(),
    );
    response.blocking_questions = vec![summary];
    response
}

pub(crate) fn looks_like_fingerprint(target: &str) -> bool {
    let hex = target.strip_prefix("sha256:").unwrap_or(target);
    hex.len() >= 12 && hex.chars().all(|character| character.is_ascii_hexdigit())
}

pub(crate) fn normalize(target: &str) -> String {
    let hex = target.strip_prefix("sha256:").unwrap_or(target);
    format!("sha256:{}", hex.to_ascii_lowercase())
}

/// The finding with this fingerprint in a findings file: `(rule, subject
/// title, file)`. A given file must exist and parse; the default one is used
/// only when present.
pub(crate) fn from_findings_file(
    root: &Path,
    given: Option<&Path>,
    fingerprint: &str,
) -> Result<Option<(String, String, String)>, String> {
    let path = match given {
        Some(path) => path.to_path_buf(),
        None => {
            let default = root.join("findings.json");
            if !default.is_file() {
                return Ok(None);
            }
            default
        }
    };
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is not a findings file: {error}", path.display()))?;
    let items = match &value {
        Value::Array(items) => items.clone(),
        Value::Object(object) => object
            .get("findings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    // A short prefix of a fingerprint is enough when it is unambiguous.
    let matches = items
        .iter()
        .filter(|item| {
            item.get("fingerprint")
                .and_then(Value::as_str)
                .is_some_and(|candidate| candidate.starts_with(fingerprint))
        })
        .collect::<Vec<_>>();
    Ok(match matches.as_slice() {
        [item] => item
            .get("responsibility")
            .and_then(Value::as_str)
            .map(|rule| {
                (
                    rule.to_string(),
                    item.get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    path.display().to_string(),
                )
            }),
        _ => None,
    })
}

/// Recompute fingerprints for every rule over every subject a sweep could
/// have named, and return the one that matches.
pub(crate) fn recompute(
    root: &Path,
    rule_ids: &[RecordId],
    fingerprint: &str,
) -> Option<(String, String, String)> {
    let repository = findings::repository_identity(root);
    let mut subjects = vec!["repository".to_string()];
    if let Ok(files) = crate::gates::FileSet::repository(root) {
        subjects.extend(files.paths().map(str::to_owned));
    }
    let matches = |candidate: &str| candidate.starts_with(fingerprint);
    for repeat in 1..=MAX_SUBJECT_REPEATS {
        for rule in rule_ids {
            for subject in &subjects {
                let subject = if repeat == 1 {
                    subject.clone()
                } else {
                    format!("{subject}#{repeat}")
                };
                if matches(&findings::fingerprint(rule.as_str(), &repository, &subject)) {
                    return Some((rule.as_str().to_string(), subject, "recomputed".into()));
                }
            }
        }
    }
    None
}

/// The principle a rule serves, when its source names one held here.
fn principle_of(state: &AgreementState, rule: &Rule) -> Option<Value> {
    let reference = rule.source.reference.as_deref()?;
    let tail = reference.rsplit(':').next().unwrap_or(reference);
    state
        .in_force_matching(|body| matches!(body, RecordBody::Principle(_)))
        .into_iter()
        .find_map(|record| {
            let RecordBody::Principle(principle) = &record.body else {
                return None;
            };
            let source_id = match &principle.source {
                PrincipleSource::Pstack { id, .. } => Some(id.as_str()),
                PrincipleSource::Pack { id, .. } => Some(id.as_str()),
                _ => None,
            };
            let id = record.id.as_str();
            let named =
                id == reference || id == format!("principle.{tail}") || source_id == Some(tail);
            named.then(|| {
                json!({
                    "id": id,
                    "statement": principle.statement,
                    "source": principle.source,
                    "rationale": principle.rationale,
                })
            })
        })
}

/// Every decision on the rule's revisions, oldest first: who proposed each
/// revision and why, and who accepted or withdrew it, when and why.
fn decisions(state: &AgreementState, id: &RecordId) -> Vec<Value> {
    let mut revisions = state
        .records()
        .iter()
        .filter(|record| record.id == *id)
        .collect::<Vec<_>>();
    revisions.sort_by_key(|record| record.revision);
    revisions
        .into_iter()
        .map(|record| decision(state, record))
        .collect()
}

fn decision(state: &AgreementState, record: &AgreementRecord) -> Value {
    let recorded_by = record
        .provenance
        .recorded_by
        .display_name
        .clone()
        .unwrap_or_else(|| record.provenance.recorded_by.stable_id.clone());
    let Some(proposal) = state.proposal_for(record) else {
        let agreed = state.lifecycle_of(record) == Lifecycle::Accepted;
        return json!({
            "revision": record.revision,
            "verdict": if agreed { "agreed" } else { state.lifecycle_of(record).label() },
            "by": recorded_by,
            "at": record.provenance.recorded_at,
            "rationale": if agreed { "Agreed at onboarding." } else { "Recorded without a proposal." },
        });
    };
    let (title, proposed_rationale) = match &proposal.body {
        RecordBody::Proposal(body) => (body.title.clone(), body.rationale.clone()),
        _ => (String::new(), String::new()),
    };
    let review = state
        .records()
        .iter()
        .find_map(|candidate| match &candidate.body {
            RecordBody::LocalReview(review) if review.proposal.id == proposal.id => Some(review),
            _ => None,
        });
    let proposed_by = proposal
        .provenance
        .recorded_by
        .display_name
        .clone()
        .unwrap_or_else(|| proposal.provenance.recorded_by.stable_id.clone());
    match review {
        Some(review) => json!({
            "revision": record.revision,
            "proposal": proposal.id.as_str(),
            "title": title,
            "proposed_by": proposed_by,
            "proposed_at": proposal.provenance.recorded_at,
            "proposed_because": proposed_rationale,
            "verdict": match review.verdict {
                crate::domain::LocalReviewVerdict::Accept => "accept",
                crate::domain::LocalReviewVerdict::Withdraw => "withdraw",
            },
            "by": review.reviewer.display_name.clone().unwrap_or_else(|| review.reviewer.stable_id.clone()),
            "at": review.reviewed_at,
            "rationale": review.rationale,
        }),
        None => json!({
            "revision": record.revision,
            "proposal": proposal.id.as_str(),
            "title": title,
            "proposed_by": proposed_by,
            "proposed_at": proposal.provenance.recorded_at,
            "proposed_because": proposed_rationale,
            "verdict": state.lifecycle_of(record).label(),
            "by": proposed_by,
            "at": proposal.provenance.recorded_at,
            "rationale": proposed_rationale,
        }),
    }
}

fn render_text(
    id: &RecordId,
    rule: &Rule,
    lifecycle: &str,
    resolved: &Value,
    principle: Option<&Value>,
    decisions: &[Value],
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    if resolved["kind"] == "fingerprint" {
        let _ = writeln!(
            out,
            "Finding {} ({})",
            resolved["fingerprint"].as_str().unwrap_or_default(),
            resolved["subject"].as_str().unwrap_or_default()
        );
    }
    let _ = writeln!(
        out,
        "Rule {} · {} · {lifecycle}",
        id.as_str(),
        rule.strength.label()
    );
    let _ = writeln!(out, "  {}", rule.statement);
    let _ = writeln!(out, "Why: {}", rule.rationale);
    let (mechanism, command) = crate::gates::mechanism_label(rule);
    let _ = writeln!(out, "Enforcer: {mechanism}: {command}");
    let _ = writeln!(
        out,
        "Covers: {}",
        if rule.paths.is_empty() {
            "every file".to_string()
        } else {
            rule.paths.join(", ")
        }
    );
    let _ = writeln!(
        out,
        "Sweep: {}, priority {}",
        rule.sweep_class().label(),
        rule.sweep_priority()
    );
    if let Some(principle) = principle {
        let _ = writeln!(
            out,
            "Serves: {} ({})",
            principle["statement"].as_str().unwrap_or_default(),
            principle["id"].as_str().unwrap_or_default()
        );
    }
    let _ = writeln!(out, "Decisions:");
    for decision in decisions {
        let _ = writeln!(
            out,
            "  r{} {} by {} at {}: {}",
            decision["revision"],
            decision["verdict"].as_str().unwrap_or_default(),
            decision["by"].as_str().unwrap_or_default(),
            decision["at"].as_str().unwrap_or_default(),
            decision["rationale"].as_str().unwrap_or_default()
        );
    }
    out
}
