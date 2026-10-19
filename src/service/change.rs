//! `wh change`: edit the rulebook file. A change or a retirement is a draft,
//! an edit to `whetstone/rulebook.json` in the working tree; accepting it is
//! committing the file, and withdrawing it restores the last commit.

use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    Mission, Principle, PrincipleSource, RecordId, Rule, RuleSource, RULE_SCHEMA_V2,
};
use crate::layout::ProjectLayout;
use crate::probe::ProbeReport;
use crate::rulebook::{
    self, PrincipleEntry, RetiredEntry, Revision, RuleEntry, Rulebook, RulebookError, RULEBOOK_PATH,
};

use super::{
    bounded_input, bounded_request_id, project_error, unknown_response, ChangeDefinition,
    ChangeKind, ChangeRequest, ServiceResponse, ServiceState,
};

pub(super) fn change(request: ChangeRequest) -> ServiceResponse {
    let layout = match ProjectLayout::resolve(&request.project_dir) {
        Ok(layout) => layout,
        Err(error) => return project_error("change", request.request_id, error),
    };
    let request_id = match bounded_request_id(
        request.request_id.clone(),
        default_request_id(&layout, &request),
    ) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response("change", "invalid-request-id".into(), summary),
    };
    let root = layout.project_root();
    if request.accept {
        return accept(root, request_id, &request);
    }
    if request.withdraw {
        return withdraw(root, request_id, &request);
    }
    if let Some(target) = request.retire.as_deref() {
        return retire(root, request_id, target, &request);
    }
    if request.from_text.is_some() || request.from_file.is_some() || request.from_finding.is_some()
    {
        return super::compile::compile(&layout, request_id, &request);
    }
    edit(root, request_id, &request)
}

/// Without `--request-id`, the id is derived from what the change says, so
/// two different changes never share one.
fn default_request_id(layout: &ProjectLayout, request: &ChangeRequest) -> String {
    let input = format!(
        "{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}",
        request.kind,
        request.record_id,
        request.content,
        request.definition,
        request.rationale,
        request.accept,
        request.withdraw,
        request.retire,
        layout.project_id(),
    ) + &format!(
        "\0{}\0{:?}\0{:?}\0{:?}\0{:?}",
        request.allow_unvalidated,
        request.from_text,
        request.from_file,
        request.from_finding,
        request.findings
    );
    format!(
        "change-{}",
        &format!("{:x}", Sha256::digest(input.as_bytes()))[..16]
    )
}

fn ask(request_id: String, question: impl Into<String>) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::NeedsInput,
        "More owner input is required; the rulebook was not changed.",
    );
    response.blocking_questions = vec![question.into()];
    response.permitted_actions = vec![
        "wh change --kind <mission|principle|rule> --record-id <id> --content <text> --rationale <why>"
            .into(),
    ];
    response
}

pub(super) fn rulebook_unreadable(request_id: String, error: &RulebookError) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Unknown,
        format!("The rulebook was not changed: {error}"),
    );
    response.permitted_actions = vec![format!("repair {RULEBOOK_PATH}, then retry")];
    response
}

/// A rule's or principle's entry as the file holds it, or the mission.
fn entry_json(book: &Rulebook, id: &str) -> Value {
    rulebook::entry(book, id).unwrap_or(Value::Null)
}

fn digest_of(book: &Rulebook, id: &str) -> Option<String> {
    book.rules
        .iter()
        .find(|entry| entry.id.as_str() == id)
        .map(|entry| entry.digest.as_str().to_string())
}

/// One rule's probe, kept with the rule it probed.
pub(super) struct Probed {
    pub(super) id: RecordId,
    pub(super) rule: Rule,
    pub(super) report: ProbeReport,
}

/// The probes as the envelope carries them.
pub(super) fn probes_json(probes: &[Probed]) -> Value {
    json!(probes
        .iter()
        .map(|probed| json!({"record_id": probed.id.as_str(), "probe": probed.report}))
        .collect::<Vec<_>>())
}

/// Probe each rule `after` adds or re-aims (a new enforcer or new paths),
/// compared with the rulebook on disk. A probe in `known` of the same rule is
/// reused rather than run again.
pub(super) fn probe_changed(root: &Path, after: &Rulebook, known: &[Probed]) -> Vec<Probed> {
    let before = rulebook::load(root).ok();
    let Ok(layout) = ProjectLayout::resolve(root) else {
        return Vec::new();
    };
    after
        .rules
        .iter()
        .filter(|entry| {
            !before.as_ref().is_some_and(|book| {
                book.rules.iter().any(|old| {
                    old.id == entry.id
                        && old.rule.enforcer == entry.rule.enforcer
                        && old.rule.paths == entry.rule.paths
                })
            })
        })
        .map(|entry| {
            let report = known
                .iter()
                .find(|probed| probed.id == entry.id && probed.rule == entry.rule)
                .map_or_else(
                    || crate::probe::probe(&layout, &entry.id, &entry.rule),
                    |probed| probed.report.clone(),
                );
            Probed {
                id: entry.id.clone(),
                rule: entry.rule.clone(),
                report,
            }
        })
        .collect()
}

/// A change refused because a rule's enforcer cannot be checked here.
fn unvalidated(request_id: String, probes: &[Probed]) -> ServiceResponse {
    let refused = probes
        .iter()
        .filter(|probed| !probed.report.validatable)
        .collect::<Vec<_>>();
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::NeedsInput,
        format!(
            "{}; the rulebook was not changed. Fix the enforcer and try again, or pass --allow-unvalidated to write it anyway.",
            refused
                .iter()
                .map(|probed| format!("{}: {}", probed.id.as_str(), probed.report.summary))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    );
    response.blocking_questions = refused
        .iter()
        .map(|probed| {
            format!(
                "{}: {}",
                probed.id.as_str(),
                probed
                    .report
                    .reason
                    .as_deref()
                    .unwrap_or(&probed.report.summary)
            )
        })
        .collect();
    response.permitted_actions = vec![
        "fix the enforcer or its paths, then repeat the change".into(),
        "repeat with --allow-unvalidated to write it without a validating probe".into(),
    ];
    response.data = json!({"written": false, "probes": probes_json(probes)});
    response
}

/// How [`draft`] treats the probes of the rules a change adds or re-aims.
#[derive(Default)]
pub(super) struct Probing<'a> {
    /// Write a rule even when its probe says it is not validatable here.
    pub(super) allow_unvalidated: bool,
    /// Probes this change already ran, reused for the same rule.
    pub(super) known: &'a [Probed],
}

/// Validate `after` strictly and probe every rule it adds or re-aims, then
/// write it unless this is a preview. A rule whose probe says it is not
/// validatable here is not written unless the probing allows it.
pub(super) fn draft(
    root: &Path,
    request_id: String,
    preview: bool,
    after: &Rulebook,
    probing: Probing<'_>,
    preview_summary: String,
    written_summary: String,
) -> Result<(Rulebook, ServiceResponse, Vec<Probed>), Box<ServiceResponse>> {
    let checked = rulebook::render(after)
        .and_then(|text| rulebook::parse(&text, Revision::Uncommitted))
        .map_err(|error| Box::new(invalid_change(request_id.clone(), &error)))?;
    let probes = probe_changed(root, &checked, probing.known);
    if !probing.allow_unvalidated && probes.iter().any(|probed| !probed.report.validatable) {
        return Err(Box::new(unvalidated(request_id, &probes)));
    }
    if preview {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            preview_summary,
        );
        response.blocking_questions =
            vec!["Does this exact before-and-after change express the intended decision?".into()];
        response.permitted_actions = vec![
            "repeat this exact request without --dry-run to write it".into(),
            "cancel without changing the rulebook".into(),
        ];
        return Ok((checked, response, probes));
    }
    let written = rulebook::write(root, after).map_err(|error| {
        Box::new(match error {
            RulebookError::Invalid(_) => invalid_change(request_id.clone(), &error),
            other => rulebook_unreadable(request_id.clone(), &other),
        })
    })?;
    let mut response =
        ServiceResponse::new(request_id, "change", ServiceState::Success, written_summary);
    response.permitted_actions = vec![
        "wh change --accept".into(),
        "wh change --withdraw".into(),
        "wh check".into(),
    ];
    Ok((written, response, probes))
}

fn invalid_change(request_id: String, error: &RulebookError) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Conflict,
        format!("The change would not make a valid rulebook: {error}"),
    );
    response.permitted_actions = vec!["correct the bounded input and submit a new request".into()];
    response
}

fn edit(root: &Path, request_id: String, request: &ChangeRequest) -> ServiceResponse {
    let Some(kind) = request.kind else {
        return ask(request_id, "Missing --kind (mission, principle or rule).");
    };
    let missing = [
        (
            "--record-id",
            kind != ChangeKind::Mission && request.record_id.is_none(),
        ),
        ("--content", request.content.is_none()),
        (
            "--rationale",
            kind == ChangeKind::Rule && request.rationale.is_none(),
        ),
    ]
    .into_iter()
    .filter_map(|(flag, absent)| absent.then_some(flag))
    .collect::<Vec<_>>();
    if !missing.is_empty() {
        return ask(request_id, format!("Missing {}.", missing.join(", ")));
    }
    let content = match bounded_input(request.content.clone(), "content", 16_000) {
        Ok(content) => content,
        Err(question) => return ask(request_id, question),
    };
    let before = match rulebook::load(root) {
        Ok(book) => Some(book),
        Err(RulebookError::Missing) => None,
        Err(error) => return rulebook_unreadable(request_id, &error),
    };
    let mut after = match (&before, kind) {
        (Some(book), _) => book.clone(),
        (None, ChangeKind::Mission) => Rulebook {
            mission: Mission {
                statement: String::new(),
            },
            principles: Vec::new(),
            rules: Vec::new(),
            retired: Vec::new(),
            revision: Revision::Uncommitted,
        },
        (None, _) => {
            return ask(
                request_id,
                format!("{RULEBOOK_PATH} does not exist yet. Set the mission first: wh change --kind mission --content <text>."),
            )
        }
    };
    let id = match kind {
        ChangeKind::Mission => {
            if request.rationale.is_some() {
                return ask(
                    request_id,
                    "The mission has no rationale in the rulebook; give the reason when you accept: wh change --accept --rationale <why>.",
                );
            }
            after.mission.statement = content;
            "mission".to_string()
        }
        ChangeKind::Principle | ChangeKind::Rule => {
            let id_text = request.record_id.clone().unwrap_or_default();
            let id = match RecordId::new(id_text.trim()) {
                Ok(id) => id,
                Err(error) => return ask(request_id, format!("--record-id: {error}")),
            };
            let (taken, other) = match kind {
                ChangeKind::Rule => (
                    after.principles.iter().any(|entry| entry.id == id),
                    "principle",
                ),
                _ => (after.rules.iter().any(|entry| entry.id == id), "rule"),
            };
            if taken {
                return ask(
                    request_id,
                    format!(
                        "{} is already a {other} in the rulebook; pick another id.",
                        id.as_str()
                    ),
                );
            }
            if kind == ChangeKind::Rule {
                let current = after.rules.iter().find(|entry| entry.id == id);
                let entry = match rule_of(request, content, current.map(|entry| &entry.rule)) {
                    Ok(rule) => RuleEntry::new(id.clone(), rule),
                    Err(question) => return ask(request_id, question),
                };
                match entry {
                    Ok(entry) => {
                        after.rules.retain(|entry| entry.id != id);
                        after.rules.push(entry);
                    }
                    Err(error) => return invalid_change(request_id, &error),
                }
            } else {
                let principle = match principle_of(root, request, content) {
                    Ok(principle) => principle,
                    Err(question) => return ask(request_id, question),
                };
                after.principles.retain(|entry| entry.id != id);
                after.principles.push(PrincipleEntry {
                    id: id.clone(),
                    principle,
                });
            }
            after.retired.retain(|entry| entry.id != id);
            id.as_str().to_string()
        }
    };
    let reinstated = before
        .as_ref()
        .is_some_and(|book| book.retired.iter().any(|entry| entry.id.as_str() == id));
    let diff = json!({
        "before": before.as_ref().map_or(Value::Null, |book| entry_json(book, &id)),
        "after": entry_json(&after, &id),
    });
    if before
        .as_ref()
        .is_some_and(|book| entry_json(book, &id) == diff["after"] && !reinstated)
    {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::Success,
            format!("The rulebook already says this about {id}; nothing was written."),
        );
        response.data = json!({
            "record_id": id,
            "path": RULEBOOK_PATH,
            "written": false,
            "diff": diff,
        });
        return response;
    }
    let (written, mut response, probes) = match draft(
        root,
        request_id,
        request.preview,
        &after,
        Probing {
            allow_unvalidated: request.allow_unvalidated,
            known: &[],
        },
        format!("Review the exact change to {id} before it is written to {RULEBOOK_PATH}."),
        format!("{RULEBOOK_PATH} now holds the change to {id}. It is a draft until you accept it, which commits the file."),
    ) {
        Ok(drafted) => drafted,
        Err(response) => return *response,
    };
    if let Some(probed) = probes.first() {
        response.summary = format!("{} {}", probed.report.summary, response.summary);
    }
    response.data = json!({
        "record_id": id,
        "path": RULEBOOK_PATH,
        "preview_only": request.preview,
        "written": !request.preview,
        "reinstated": reinstated,
        "revision": written.revision,
        "diff": diff,
        "digest": {
            "before": before.as_ref().and_then(|book| digest_of(book, &id)),
            "after": digest_of(&written, &id),
        },
        "probes": probes_json(&probes),
    });
    response
}

fn principle_of(
    root: &Path,
    request: &ChangeRequest,
    content: String,
) -> Result<Principle, String> {
    let (picked, rationale) = match request.definition.as_deref() {
        Some(ChangeDefinition::Principle {
            pstack,
            principle,
            rationale,
        }) => (
            principle
                .clone()
                .or_else(|| pstack.as_ref().map(|id| format!("pstack/{id}"))),
            rationale.clone(),
        ),
        None => (None, None),
        Some(_) => {
            return Err(
                "A principle's definition is {\"type\": \"principle\", \"principle\": \"<id or pack/id>\"}."
                    .into(),
            )
        }
    };
    let source = match picked {
        Some(spec) => {
            let catalogue = crate::catalogue::catalogue(root);
            let Some(entry) = catalogue.get(&spec) else {
                return Err(format!(
                    "{spec} is not a principle in any loaded pack ({}); name one as <id> or <pack>/<id>, or leave it out for your own.",
                    catalogue.describe()
                ));
            };
            entry.source()
        }
        None => PrincipleSource::Custom,
    };
    let rationale = rationale
        .or_else(|| request.rationale.clone())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    Ok(Principle {
        statement: content,
        source,
        rationale,
    })
}

fn rule_of(
    request: &ChangeRequest,
    content: String,
    current: Option<&Rule>,
) -> Result<Rule, String> {
    let Some(ChangeDefinition::Rule {
        strength,
        enforcer,
        examples,
        source,
        paths,
        sweep,
        sweep_priority,
        sweep_unsure,
    }) = request.definition.as_deref()
    else {
        return Err("Provide the rule's strength and its one enforcer as a rule definition: {\"type\": \"rule\", \"strength\": \"must\", \"enforcer\": {\"kind\": \"test\", \"command\": \"cargo test\"}}.".into());
    };
    if let crate::domain::Enforcer::Test { command }
    | crate::domain::Enforcer::Validator { command } = enforcer
    {
        crate::gates::parse_command(command)?;
    }
    let rule = Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: content,
        rationale: bounded_input(request.rationale.clone(), "rationale", 4_000)?,
        strength: *strength,
        enforcer: enforcer.clone(),
        examples: examples.clone(),
        source: source.clone().unwrap_or_else(RuleSource::owner),
        paths: paths.clone(),
        sweep: *sweep,
        sweep_priority: *sweep_priority,
        sweep_unsure: sweep_unsure.clone(),
    };
    // How far a sweep may reach over this rule's flags is a person's call:
    // an agent never widens it.
    if request.actor != crate::domain::ActorKind::Person {
        let current = current.map(Rule::sweep_class).unwrap_or_default();
        if rule.sweep_class().reach() > current.reach() {
            return Err(format!(
                "An agent cannot widen how a sweep works this rule (from {} to {}); a person sets the sweep class.",
                current.label(),
                rule.sweep_class().label()
            ));
        }
    }
    rule.validate()
        .map_err(|error| format!("The rule is not valid: {error}"))?;
    Ok(rule)
}

/// Take a rule or principle out of the rulebook and record why in its
/// `retired` list. The entry itself stays in Git history.
fn retire(
    root: &Path,
    request_id: String,
    target: &str,
    request: &ChangeRequest,
) -> ServiceResponse {
    let before = match rulebook::load(root) {
        Ok(book) => book,
        Err(error) => return rulebook_unreadable(request_id, &error),
    };
    let reason = match bounded_input(request.rationale.clone(), "rationale", 4_000) {
        Ok(reason) => reason,
        Err(question) => {
            return ask(
                request_id,
                format!("{question} Say why {target} is retired."),
            )
        }
    };
    let title = before
        .rules
        .iter()
        .find(|entry| entry.id.as_str() == target)
        .map(|entry| entry.rule.statement.clone())
        .or_else(|| {
            before
                .principles
                .iter()
                .find(|entry| entry.id.as_str() == target)
                .map(|entry| entry.principle.statement.clone())
        });
    let (Some(title), Ok(id)) = (title, RecordId::new(target)) else {
        let retired = before
            .retired
            .iter()
            .any(|entry| entry.id.as_str() == target);
        return ask(
            request_id,
            if retired {
                format!("{target} is already retired.")
            } else {
                format!("{target} is not a rule or principle in {RULEBOOK_PATH}, so there is nothing to retire.")
            },
        );
    };
    let mut after = before.clone();
    after.rules.retain(|entry| entry.id != id);
    after.principles.retain(|entry| entry.id != id);
    after.retired.push(RetiredEntry {
        id,
        reason: reason.clone(),
    });
    let (written, mut response, _) = match draft(
        root,
        request_id,
        request.preview,
        &after,
        Probing::default(),
        format!("Review the exact effect before {target} is retired."),
        format!("{target} is out of {RULEBOOK_PATH} and its retirement is recorded there. It is a draft until you accept it, which commits the file."),
    ) {
        Ok(drafted) => drafted,
        Err(response) => return *response,
    };
    response.data = json!({
        "retire": target,
        "title": title,
        "reason": reason,
        "path": RULEBOOK_PATH,
        "preview_only": request.preview,
        "written": !request.preview,
        "revision": written.revision,
        "diff": {"before": entry_json(&before, target), "after": Value::Null},
        "digest": {"before": digest_of(&before, target), "after": Value::Null},
    });
    response
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("git could not start: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let mut message = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if message.is_empty() {
            message = String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
        Err(format!("git {} failed: {message}", args[0]))
    }
}

fn git_failed(request_id: String, error: String) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Unknown,
        format!("The operation stopped without partial success: {error}"),
    );
    response.permitted_actions = vec!["fix what Git reported, then retry the same request".into()];
    response
}

/// The rulebook as last committed, if it was ever committed. `Err` holds the
/// reader's complaint about a committed file it cannot read.
pub(super) fn committed(root: &Path) -> Option<Result<Rulebook, RulebookError>> {
    let text = git(root, &["show", &format!("HEAD:{RULEBOOK_PATH}")]).ok()?;
    let commit = git(root, &["log", "-1", "--format=%H", "--", RULEBOOK_PATH])
        .map(|commit| commit.trim().to_string())
        .unwrap_or_default();
    Some(rulebook::parse(&text, Revision::Commit(commit)))
}

/// The edits from one rulebook to another (from nothing when `before` is
/// absent), each as the edit and its detail: `add rule.x` with the digest,
/// `retire rule.y` with the reason, and so on.
pub(super) fn changes(before: Option<&Rulebook>, after: &Rulebook) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    let Some(before) = before else {
        lines.push(("set the mission".into(), String::new()));
        let empty = Rulebook {
            principles: Vec::new(),
            rules: Vec::new(),
            retired: Vec::new(),
            ..after.clone()
        };
        lines.extend(changes(Some(&empty), after));
        return lines;
    };
    if before.mission.statement != after.mission.statement {
        lines.push(("change the mission".into(), String::new()));
    }
    let ids = |book: &Rulebook| {
        book.principles
            .iter()
            .map(|entry| entry.id.as_str().to_string())
            .chain(book.rules.iter().map(|entry| entry.id.as_str().to_string()))
            .collect::<std::collections::BTreeSet<_>>()
    };
    for id in ids(before).union(&ids(after)) {
        let (old, new) = (entry_json(before, id), entry_json(after, id));
        let digests = (digest_of(before, id), digest_of(after, id));
        if old.is_null() {
            let detail = digests
                .1
                .map(|digest| format!(" ({digest})"))
                .unwrap_or_default();
            lines.push((format!("add {id}"), detail));
        } else if new.is_null() {
            match after.retired.iter().find(|entry| entry.id.as_str() == id) {
                Some(retired) => {
                    lines.push((format!("retire {id}"), format!(": {}", retired.reason)))
                }
                None => lines.push((format!("remove {id}"), String::new())),
            }
        } else if old != new {
            let detail = match digests {
                (Some(old), Some(new)) => format!(" ({old} -> {new})"),
                _ => String::new(),
            };
            lines.push((format!("change {id}"), detail));
        }
    }
    lines
}

/// The commit message: a subject naming the edits and a body with the
/// owner's reason, each edit, the new digests and the retirement reasons.
fn message(edits: &[(String, String)], rationale: Option<&str>) -> (String, String) {
    let subject = match edits {
        [(only, _)] => match only.split_once(' ') {
            Some(("add", id)) => format!("Add {id} to the rulebook"),
            Some(("change", "the mission")) => "Change the rulebook's mission".into(),
            Some((verb @ ("retire" | "remove"), id)) => {
                format!("{} {id} from the rulebook", capitalized(verb))
            }
            _ => format!("{} in the rulebook", capitalized(only)),
        },
        _ => {
            let listed = format!(
                "Change the rulebook: {}",
                edits
                    .iter()
                    .map(|(edit, _)| edit.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if listed.len() <= 72 {
                listed
            } else {
                format!("Change the rulebook: {} edits", edits.len())
            }
        }
    };
    let mut body = rationale
        .map(str::trim)
        .filter(|rationale| !rationale.is_empty())
        .map(|rationale| format!("{rationale}\n\n"))
        .unwrap_or_default();
    for (edit, detail) in edits {
        body.push_str(&format!("{}{detail}\n", capitalized(edit)));
    }
    (subject, body.trim_end().to_string())
}

fn capitalized(text: &str) -> String {
    let mut characters = text.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

fn uncommitted(root: &Path) -> Result<bool, String> {
    git(
        root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=all",
            "--",
            RULEBOOK_PATH,
        ],
    )
    .map(|status| !status.trim().is_empty())
}

/// Accept the drafted edits: commit the rulebook file, and only it.
fn accept(root: &Path, request_id: String, request: &ChangeRequest) -> ServiceResponse {
    match uncommitted(root) {
        Ok(true) => {}
        Ok(false) => {
            return ask(
                request_id,
                format!("{RULEBOOK_PATH} has no uncommitted edit, so there is nothing to accept."),
            )
        }
        Err(error) => return git_failed(request_id, error),
    }
    let current = match rulebook::load(root) {
        Ok(book) => book,
        Err(error) => return rulebook_unreadable(request_id, &error),
    };
    let edits = match committed(root) {
        Some(Ok(head)) => changes(Some(&head), &current),
        Some(Err(_)) => vec![(
            "replace the unreadable committed rulebook".into(),
            String::new(),
        )],
        None => changes(None, &current),
    };
    let rationale = request.rationale.as_deref();
    if rationale.is_some_and(|value| value.len() > 4_000) {
        return ask(request_id, "rationale exceeds the 4000-byte limit.");
    }
    let (subject, body) = message(&edits, rationale);
    let listed = edits
        .iter()
        .map(|(edit, detail)| format!("{edit}{detail}"))
        .collect::<Vec<_>>();
    if request.preview {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            format!("Review the commit before {RULEBOOK_PATH} is accepted."),
        );
        response.permitted_actions = vec!["repeat without --dry-run to commit it".into()];
        response.data = json!({
            "preview_only": true,
            "path": RULEBOOK_PATH,
            "message": {"subject": subject, "body": body},
            "changes": listed,
        });
        return response;
    }
    let mut commit = vec!["commit", "--quiet", "-m", subject.as_str()];
    if !body.is_empty() {
        commit.extend(["-m", body.as_str()]);
    }
    commit.extend(["--", RULEBOOK_PATH]);
    let committed = git(root, &["add", "--", RULEBOOK_PATH])
        .and_then(|_| git(root, &commit))
        .and_then(|_| git(root, &["rev-parse", "HEAD"]));
    let commit = match committed {
        Ok(commit) => commit.trim().to_string(),
        Err(error) => return git_failed(request_id, error),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Success,
        format!(
            "Accepted: {RULEBOOK_PATH} was committed as {}. Share it the way you share any commit.",
            &commit[..12.min(commit.len())]
        ),
    );
    response.permitted_actions = vec!["wh check".into(), "git push".into()];
    response.data = json!({
        "path": RULEBOOK_PATH,
        "commit": commit,
        "revision": commit,
        "message": {"subject": subject, "body": body},
        "changes": listed,
    });
    response
}

/// Withdraw the drafted edits: restore the rulebook to its last commit.
fn withdraw(root: &Path, request_id: String, request: &ChangeRequest) -> ServiceResponse {
    let Some(head) = committed(root) else {
        return ask(
            request_id,
            format!("{RULEBOOK_PATH} was never committed, so there is no accepted rulebook to go back to."),
        );
    };
    match uncommitted(root) {
        Ok(true) => {}
        Ok(false) => {
            return ask(
                request_id,
                format!(
                    "{RULEBOOK_PATH} has no uncommitted edit, so there is nothing to withdraw."
                ),
            )
        }
        Err(error) => return git_failed(request_id, error),
    }
    let edits = match (head, rulebook::load(root)) {
        (Ok(head), Ok(current)) => changes(Some(&head), &current),
        _ => vec![("restore the rulebook as committed".into(), String::new())],
    };
    let listed = edits
        .into_iter()
        .map(|(edit, detail)| format!("{edit}{detail}"))
        .collect::<Vec<_>>();
    if request.preview {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            format!("Review the edits that withdrawing discards from {RULEBOOK_PATH}."),
        );
        response.permitted_actions = vec!["repeat without --dry-run to withdraw them".into()];
        response.data = json!({"preview_only": true, "path": RULEBOOK_PATH, "discards": listed});
        return response;
    }
    if let Err(error) = git(
        root,
        &[
            "restore",
            "--source=HEAD",
            "--staged",
            "--worktree",
            "--",
            RULEBOOK_PATH,
        ],
    ) {
        return git_failed(request_id, error);
    }
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Success,
        format!("The drafted edits were withdrawn; {RULEBOOK_PATH} is as last committed."),
    );
    response.permitted_actions = vec!["wh check".into()];
    response.data = json!({
        "path": RULEBOOK_PATH,
        "discarded": listed,
        "revision": rulebook::revision(root),
    });
    response
}
