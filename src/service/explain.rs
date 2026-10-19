//! `wh explain <fingerprint | rule id>`: the rule behind a finding, its exact
//! enforcer, what it covers, how a sweep works it, the principle it serves
//! and the decisions that accepted and last changed it: the commits that
//! changed the rule in `whetstone/rulebook.json`. Read-only.
//!
//! A sweep finding's fingerprint is resolved from a findings file when one is
//! given (or `findings.json` sits at the repository root), and otherwise by
//! recomputing fingerprints for every rule over the repository's files, so a
//! fingerprint copied from a pull request or an issue resolves on any clone.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

use crate::domain::{PrincipleSource, Rule};
use crate::layout::ProjectLayout;
use crate::rulebook::{self, RuleEntry, Rulebook, RulebookError, RULEBOOK_PATH};

use super::check::findings;
use super::{
    bounded_request_id, project_error, rulebook_log, unknown_response, RulebookCommit,
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
    let root = layout.project_root();
    let book = match rulebook::load(root) {
        Ok(book) => book,
        Err(error) => return unreadable(request_id, &error),
    };
    let rule_ids = book
        .rules
        .iter()
        .map(|entry| entry.id.as_str().to_string())
        .collect::<Vec<_>>();
    let (rule_id, resolved) = if rule_ids.contains(&target) {
        (target.clone(), json!({"kind": "rule", "rule": target}))
    } else if let Some(retired) = book
        .retired
        .iter()
        .find(|entry| entry.id.as_str() == target)
    {
        return needs(
            request_id,
            format!(
                "{target} was retired from {RULEBOOK_PATH}: {}",
                retired.reason
            ),
        );
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
        if !rule_ids.contains(&rule) {
            return needs(
                request_id,
                format!("The finding names {rule}, which is not a rule in {RULEBOOK_PATH}."),
            );
        }
        (
            rule,
            json!({"kind": "fingerprint", "fingerprint": fingerprint, "subject": subject, "resolved_from": source}),
        )
    } else {
        return needs(
            request_id,
            format!(
                "{target} is neither a rule in {RULEBOOK_PATH} nor a finding fingerprint (sha256:...). Rules here: {}.",
                rule_ids.join(", ")
            ),
        );
    };
    let entry = book
        .rules
        .iter()
        .find(|entry| entry.id.as_str() == rule_id)
        .expect("the resolved rule is in the rulebook");
    let rule = &entry.rule;
    let decisions = match decisions(root, &rule_id) {
        Ok(decisions) => decisions,
        Err(error) => return unknown_response("explain", request_id, error),
    };
    let lifecycle = lifecycle(root, entry);
    let principle = principle_of(&book, rule);
    let (mechanism, command) = crate::gates::mechanism_label(rule);
    let rule_json = json!({
        "id": rule_id,
        "digest": entry.digest,
        "lifecycle": lifecycle,
        "statement": rule.statement,
        "rationale": rule.rationale,
        "strength": rule.strength.label(),
        "enforcer": rule.enforcer,
        "mechanism": mechanism,
        "command": command,
        "runs_at": crate::projection::runs_at(rule),
        "paths": rule.paths,
        "sweep": {
            "class": rule.sweep_class().label(),
            "priority": rule.sweep_priority(),
            "unsure": rule.sweep_unsure,
        },
        "source": rule.source,
    });
    let text = render_text(
        &rule_id,
        rule,
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
            "{rule_id} ({}, {lifecycle}): {}",
            rule.strength.label(),
            rule.statement
        ),
    );
    response.permitted_actions = vec![
        "repair the code the finding names, then wh check".into(),
        format!(
            "wh change --kind rule --record-id {rule_id} ... --rationale <why> (a person accepts)"
        ),
    ];
    response.data = json!({
        "target": target,
        "resolved": resolved,
        "rule": rule_json,
        "principle": principle,
        "decisions": decisions,
        "created": decisions.first(),
        "last_changed": decisions.last(),
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

fn unreadable(request_id: String, error: &RulebookError) -> ServiceResponse {
    let mut response = unknown_response(
        "explain",
        request_id,
        format!("Nothing can be explained: {error}"),
    );
    response.permitted_actions = vec![format!("repair {RULEBOOK_PATH}, then wh explain")];
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
    // A sweep writes one finding per line; a JSON array reads the same.
    let items = match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(items)) => items,
        Ok(Value::Object(object)) => object
            .get("findings")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        _ => text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<Value>)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("{} is not a findings file: {error}", path.display()))?,
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
/// have named, and return the one that matches: `(rule, subject, how)`.
pub(crate) fn recompute(
    root: &Path,
    rule_ids: &[String],
    fingerprint: &str,
) -> Option<(String, String, String)> {
    let repository = findings::repository_identity(root);
    let mut subjects = vec!["repository".to_string()];
    if let Ok(files) = crate::gates::FileSet::repository(root) {
        subjects.extend(files.paths().map(str::to_owned));
    }
    for repeat in 1..=MAX_SUBJECT_REPEATS {
        for rule in rule_ids {
            for subject in &subjects {
                let subject = if repeat == 1 {
                    subject.clone()
                } else {
                    format!("{subject}#{repeat}")
                };
                if findings::fingerprint(rule, &repository, &subject).starts_with(fingerprint) {
                    return Some((rule.clone(), subject, "recomputed".into()));
                }
            }
        }
    }
    None
}

/// Whether the rule as the file holds it is the accepted one (committed with
/// the same digest) or a draft in the working tree.
fn lifecycle(root: &Path, entry: &RuleEntry) -> &'static str {
    match super::change::committed(root) {
        Some(Ok(head))
            if head
                .rules
                .iter()
                .any(|accepted| accepted.id == entry.id && accepted.digest == entry.digest) =>
        {
            "accepted"
        }
        _ => "draft",
    }
}

/// The principle a rule serves, when its source names one in the rulebook.
fn principle_of(book: &Rulebook, rule: &Rule) -> Option<Value> {
    let reference = rule.source.reference.as_deref()?;
    let tail = reference.rsplit(':').next().unwrap_or(reference);
    book.principles.iter().find_map(|entry| {
        let source_id = match &entry.principle.source {
            PrincipleSource::Pstack { id, .. } | PrincipleSource::Pack { id, .. } => {
                Some(id.as_str())
            }
            PrincipleSource::Custom => None,
        };
        let id = entry.id.as_str();
        let named = id == reference || id == format!("principle.{tail}") || source_id == Some(tail);
        named.then(|| rulebook::principle_value(entry))
    })
}

/// The rule's entry in the rulebook at a revision, as plain JSON (an older
/// file may not read strictly), and whether it is retired there.
fn entry_at(root: &Path, revision: &str, id: &str) -> (Option<Value>, bool) {
    let text = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("{revision}:{RULEBOOK_PATH}")])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| output.stdout);
    let Some(value) = text.and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok()) else {
        return (None, false);
    };
    let find = |list: &str| {
        value[list]
            .as_array()
            .and_then(|items| items.iter().find(|item| item["id"] == id).cloned())
    };
    (find("rules"), find("retired").is_some())
}

/// Every commit that changed the rule, oldest first: who changed it, when,
/// how (added, changed, retired, reinstated) and why.
fn decisions(root: &Path, id: &str) -> Result<Vec<Value>, String> {
    let log = rulebook_log(root)?;
    let mut decisions = Vec::new();
    for commit in log.iter().rev() {
        let (after, retired) = entry_at(root, &commit.commit, id);
        let (before, was_retired) = entry_at(root, &format!("{}^", commit.commit), id);
        let change = match (&before, &after) {
            (None, Some(_)) if was_retired => "reinstated",
            (None, Some(_)) => "added",
            (Some(_), None) if retired => "retired",
            (Some(_), None) => "removed",
            (Some(old), Some(new)) if old != new => "changed",
            _ => continue,
        };
        decisions.push(decision(commit, change));
    }
    Ok(decisions)
}

fn decision(commit: &RulebookCommit, change: &str) -> Value {
    json!({
        "commit": commit.commit,
        "change": change,
        "by": commit.author,
        "at": commit.date,
        "subject": commit.subject,
        "rationale": commit.body.split("\n\n").next().unwrap_or_default(),
    })
}

fn render_text(
    id: &str,
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
    let _ = writeln!(out, "Rule {id} · {} · {lifecycle}", rule.strength.label());
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
    if decisions.is_empty() {
        let _ = writeln!(
            out,
            "  none yet: the rule is not committed, so no one has accepted it"
        );
    }
    for decision in decisions {
        let text = |field: &str| decision[field].as_str().unwrap_or_default().to_string();
        let commit = text("commit");
        let _ = writeln!(
            out,
            "  {} {} by {} at {}: {}",
            &commit[..commit.len().min(12)],
            text("change"),
            text("by"),
            text("at"),
            if text("rationale").is_empty() {
                text("subject")
            } else {
                text("rationale")
            }
        );
    }
    out
}
