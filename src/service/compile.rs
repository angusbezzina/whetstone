//! `wh change --from-text|--from-file|--from-finding`: turn a team's prose,
//! or a finding that keeps coming back, into probed rule drafts in
//! `whetstone/rulebook.json`. What names its enforcer is probed on this
//! repository and written as a draft a person accepts; what does not is
//! returned as needing an enforcer, with its prose kept, for the user's agent
//! to complete.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::json;

use crate::compile::{self, Candidate};
use crate::domain::{
    Batch, Enforcer, ExampleVerdict, RecordId, Rule, RuleExample, RuleSource, RuleSourceKind,
    Strength, RULE_SCHEMA_V2,
};
use crate::layout::ProjectLayout;
use crate::rulebook::{self, RuleEntry, Rulebook, RulebookError, RULEBOOK_PATH};

use super::change::{draft, probes_json, rulebook_unreadable, Probed, Probing};
use super::check::findings;
use super::{ChangeKind, ChangeRequest, ServiceResponse, ServiceState};

/// Where the candidates came from, for rationales and provenance.
struct Origin {
    /// `CONTRIBUTING.md`, `the given text` or the finding.
    label: String,
    kind: RuleSourceKind,
    /// For a finding: the flagged line as `(path, text)`, kept as an example.
    flagged: Option<(String, String)>,
}

fn needs(request_id: String, question: String) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::NeedsInput,
        question.clone(),
    );
    response.blocking_questions = vec![question];
    response
}

pub(super) fn compile(
    layout: &ProjectLayout,
    request_id: String,
    request: &ChangeRequest,
) -> ServiceResponse {
    if request.kind.is_some_and(|kind| kind != ChangeKind::Rule) {
        return needs(
            request_id,
            "--from-text, --from-file and --from-finding draft rules; use them with --kind rule or alone.".into(),
        );
    }
    let root = layout.project_root();
    let book = match rulebook::load(root) {
        Ok(book) => book,
        Err(RulebookError::Missing) => {
            return needs(
                request_id,
                format!("{RULEBOOK_PATH} does not exist yet. Set the mission first: wh change --kind mission --content <text>."),
            )
        }
        Err(error) => return rulebook_unreadable(request_id, &error),
    };
    let (origin, candidates) = if let Some(finding) = request.from_finding.as_deref() {
        match from_finding(layout, &book, finding, request.findings.as_deref()) {
            Ok(found) => found,
            Err(question) => return needs(request_id, question),
        }
    } else {
        let (label, text) = match read_prose(root, request) {
            Ok(read) => read,
            Err(question) => return needs(request_id, question),
        };
        (
            Origin {
                label,
                kind: RuleSourceKind::Prose,
                flagged: None,
            },
            compile::candidates(root, &text),
        )
    };
    if candidates.is_empty() {
        return needs(
            request_id,
            format!(
                "{} holds no rule: no list item and no sentence that says must, never, should, avoid or prefer.",
                origin.label
            ),
        );
    }

    let mut after = book.clone();
    let mut taken = BTreeSet::new();
    let mut drafts = Vec::new();
    let mut probes = Vec::new();
    let mut unvalidated = Vec::new();
    let mut needs_enforcer = Vec::new();
    let mut already = Vec::new();
    for candidate in &candidates {
        let Some(enforcer) = candidate.enforcer.clone() else {
            let id = fresh_id(&after, &mut taken, &candidate.prose);
            needs_enforcer.push(json!({
                "line": candidate.line,
                "prose": candidate.prose,
                "strength": candidate.strength.label(),
                "paths": candidate.paths,
                "suggested_record_id": id.as_str(),
                "next": format!(
                    "Give it an enforcer: wh change --kind rule --record-id {} --content {:?} --rationale <why> --definition '{{\"type\":\"rule\",\"strength\":\"{}\",\"enforcer\":{{...}}}}' (--preview shows its probe). When no lint code or AST query fits, write a script under whetstone/checks (see whetstone/checks/README.md).",
                    id.as_str(),
                    candidate.prose,
                    candidate.strength.label()
                ),
            }));
            continue;
        };
        let rule = rule_of(candidate, enforcer, &origin);
        // Compiling the same prose again finds the rules it already wrote.
        if let Some(existing) = after.rules.iter().find(|entry| {
            entry.rule.statement == rule.statement && entry.rule.enforcer == rule.enforcer
        }) {
            already.push(json!({"line": candidate.line, "prose": candidate.prose, "record_id": existing.id.as_str()}));
            continue;
        }
        let id = fresh_id(&after, &mut taken, &candidate.prose);
        let report = crate::probe::probe(layout, &id, &rule);
        let summary = json!({
            "line": candidate.line,
            "prose": candidate.prose,
            "record_id": id.as_str(),
            "strength": candidate.strength.label(),
            "enforcer": rule.enforcer,
            "inferred": candidate.inferred,
            "paths": rule.paths,
            "probe": report,
        });
        if !report.validatable && !request.allow_unvalidated {
            unvalidated.push(summary);
            continue;
        }
        match RuleEntry::new(id.clone(), rule.clone()) {
            Ok(entry) => after.rules.push(entry),
            Err(error) => return rulebook_unreadable(request_id, &error),
        }
        probes.push(Probed { id, rule, report });
        drafts.push(summary);
    }
    let summary = format!(
        "{} rule(s) found in {}: {} probed draft(s) {}, {} not validatable here, {} need an enforcer, {} already in the rulebook. Nothing takes force until a person accepts it.",
        candidates.len(),
        origin.label,
        drafts.len(),
        if request.preview { "would be written" } else { "written" },
        unvalidated.len(),
        needs_enforcer.len(),
        already.len()
    );
    let data = |written: bool| {
        json!({
            "source": origin.label,
            "path": RULEBOOK_PATH,
            "candidates": candidates.len(),
            "drafts": drafts,
            "not_validatable": unvalidated,
            "needs_enforcer": needs_enforcer,
            "already": already,
            "preview_only": request.preview,
            "written": written,
        })
    };
    if drafts.is_empty() {
        let mut response =
            ServiceResponse::new(request_id, "change", ServiceState::NeedsInput, summary);
        response.blocking_questions =
            vec!["Name an enforcer for each rule that needs one, then draft it with wh change --kind rule.".into()];
        response.data = data(false);
        return response;
    }
    let (_, mut response, probed) = match draft(
        root,
        request_id,
        request.preview,
        &after,
        Probing {
            allow_unvalidated: request.allow_unvalidated,
            known: &probes,
        },
        summary.clone(),
        summary,
    ) {
        Ok(drafted) => drafted,
        Err(response) => return *response,
    };
    response.permitted_actions = vec![
        "wh change --accept (a person accepts the drafts, which commits the file)".into(),
        "wh change --withdraw".into(),
        "for each needs_enforcer entry: name an enforcer and draft it with wh change --kind rule"
            .into(),
    ];
    response.data = data(!request.preview);
    response.data["probes"] = probes_json(&probed);
    response
}

/// The prose to compile and what to call it.
fn read_prose(root: &Path, request: &ChangeRequest) -> Result<(String, String), String> {
    if let Some(text) = request.from_text.as_deref() {
        if text.trim().is_empty() {
            return Err("--from-text is empty.".into());
        }
        if text.len() > compile::MAX_INPUT_BYTES {
            return Err("--from-text is larger than 256 KiB; pass a file instead.".into());
        }
        return Ok(("the given text".into(), text.to_string()));
    }
    let path = request
        .from_file
        .as_deref()
        .ok_or_else(|| "Name the prose with --from-text or --from-file.".to_string())?;
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let metadata = std::fs::metadata(&full)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    if metadata.len() > compile::MAX_INPUT_BYTES as u64 {
        return Err(format!("{} is larger than 256 KiB.", path.display()));
    }
    let text = std::fs::read_to_string(&full)
        .map_err(|error| format!("{} is not text: {error}", path.display()))?;
    let label = full
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();
    Ok((label, text))
}

/// A rule id the rulebook does not hold yet (nor its retired list), from the
/// prose's first words.
fn fresh_id(book: &Rulebook, taken: &mut BTreeSet<String>, prose: &str) -> RecordId {
    let slug = prose
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .join("-");
    let base = if slug.is_empty() {
        "rule.drafted".to_string()
    } else {
        format!("rule.{}", &slug[..slug.len().min(60)])
    };
    let held = |id: &RecordId| {
        book.rules.iter().any(|entry| entry.id == *id)
            || book.principles.iter().any(|entry| entry.id == *id)
            || book.retired.iter().any(|entry| entry.id == *id)
    };
    let mut candidate = base.clone();
    let mut counter = 1;
    loop {
        let free = RecordId::new(candidate.as_str())
            .ok()
            .filter(|id| !held(id) && !taken.contains(id.as_str()));
        if let Some(id) = free {
            taken.insert(id.as_str().to_string());
            return id;
        }
        counter += 1;
        candidate = format!("{base}-{counter}");
    }
}

fn rule_of(candidate: &Candidate, enforcer: Enforcer, origin: &Origin) -> Rule {
    let statement = candidate.prose.chars().take(500).collect::<String>();
    let reference = match origin.kind {
        RuleSourceKind::Prose => format!("{}:{}", origin.label, candidate.line),
        _ => origin.label.clone(),
    };
    Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: statement.clone(),
        rationale: match origin.kind {
            RuleSourceKind::Finding => format!(
                "Drafted from a recurring finding ({}): {}",
                origin.label, candidate.prose
            ),
            _ => format!("From {reference}: \"{statement}\""),
        },
        strength: candidate.strength,
        enforcer,
        examples: origin
            .flagged
            .iter()
            .map(|(path, text)| RuleExample {
                input: text.clone(),
                expected: ExampleVerdict::Flag,
                reason: format!("{} flagged this line.", origin.label),
                path: Some(path.clone()),
            })
            .collect(),
        source: RuleSource {
            kind: origin.kind,
            reference: Some(reference.chars().take(300).collect()),
            provenance: Vec::new(),
        },
        paths: candidate.paths.clone(),
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
        exclude: None,
        mode: crate::domain::RuleMode::Tree,
        batch: None,
    }
}

/// A candidate from a finding: the rule that raised it, narrowed to where it
/// keeps flagging and raised to must, with the flagged line as an example.
fn from_finding(
    layout: &ProjectLayout,
    book: &Rulebook,
    finding: &str,
    findings_file: Option<&Path>,
) -> Result<(Origin, Vec<Candidate>), String> {
    let root = layout.project_root();
    // A batched finding names a file or a crate; a crate narrows the rule
    // to its directory.
    let mut crate_directory = None;
    let (rule_id, location) = if let Some((rule, location)) = finding.split_once('@') {
        (rule.to_string(), location.to_string())
    } else if super::explain::looks_like_fingerprint(finding) {
        let fingerprint = super::explain::normalize(finding);
        let found = super::explain::from_findings_file(root, findings_file, &fingerprint)?
            .or_else(|| super::explain::recompute(root, book, &fingerprint))
            .ok_or_else(|| format!("No rule here produces the finding {fingerprint}."))?;
        let subject = |found: &str| {
            if findings::batch_of(found).is_some() {
                found.to_string()
            } else {
                found.split('#').next().unwrap_or_default().to_string()
            }
        };
        // A findings file gives a title, not a path: recompute the subject.
        let subject = if root.join(subject(&found.1)).is_file() {
            subject(&found.1)
        } else {
            super::explain::recompute(root, book, &fingerprint)
                .map(|(_, recomputed, _)| subject(&recomputed))
                .unwrap_or_default()
        };
        match findings::batch_of(&subject) {
            Some((Batch::ByFile, file)) => (found.0, file.to_string()),
            Some((Batch::ByCrate, directory)) => {
                crate_directory = Some(directory.to_string());
                (found.0, String::new())
            }
            None => (found.0, subject),
        }
    } else {
        return Err(format!(
            "{finding} is neither a finding fingerprint nor <rule id>@<path[:line]>."
        ));
    };
    let entry = book
        .rules
        .iter()
        .find(|entry| entry.id.as_str() == rule_id)
        .ok_or_else(|| format!("{rule_id} is not a rule in {RULEBOOK_PATH}."))?;
    let rule = &entry.rule;
    let (path, mut line) = match location.split_once(':') {
        Some((path, line)) => (path.to_string(), line.parse::<usize>().ok()),
        None => (location.clone(), None),
    };
    // A fingerprint names a file, not a line: the rule's own enforcer says
    // where in the file it flags.
    if line.is_none() && !path.is_empty() {
        let report = crate::probe::probe(layout, &entry.id, rule);
        line = report.samples.iter().find_map(|sample| {
            let (flagged, at) = sample.location.split_once(':')?;
            (flagged == path).then(|| at.split(':').next()?.parse::<usize>().ok())?
        });
    }
    let flagged = line.and_then(|line| {
        std::fs::read_to_string(root.join(&path))
            .ok()
            .and_then(|text| text.lines().nth(line.saturating_sub(1)).map(str::to_owned))
    });
    let directory = Path::new(&path)
        .parent()
        .map(|parent| parent.display().to_string())
        .filter(|parent| !parent.is_empty());
    let paths = match (directory, crate_directory) {
        (Some(directory), _) if !path.is_empty() => vec![format!("{directory}/**")],
        (_, Some(directory)) if directory != "." => vec![format!("{directory}/**")],
        _ => rule.paths.clone(),
    };
    let candidate = Candidate {
        line: line.unwrap_or(0),
        prose: format!(
            "{}{}",
            rule.statement,
            if path.is_empty() {
                String::new()
            } else {
                format!(" (flagged in {location})")
            }
        ),
        strength: Strength::Must,
        enforcer: Some(rule.enforcer.clone()),
        inferred: Some(format!(
            "the enforcer of {rule_id}, which raised the finding"
        )),
        paths,
    };
    let origin = Origin {
        label: format!("{rule_id}@{location}"),
        kind: RuleSourceKind::Finding,
        flagged: flagged
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty())
            .map(|text| (path.clone(), text)),
    };
    Ok((origin, vec![candidate]))
}
