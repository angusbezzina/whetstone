//! `wh change --import-pack <file|url|name>`, and the same from the page.
//!
//! Every import starts with an inert review: it reads the pack and this
//! repository and reports what each entry is, runs no command and writes
//! nothing. `--preview` stops there. Applying writes the principles and AST
//! rules as drafts. A pack's commands and files go further only with a
//! person's consent, because a draft's command runs on the next local check.
//! The built-in packs need no consent: no file can change them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{json, Value};

use crate::domain::{ActorKind, Enforcer, Principle, RecordId, Rule, RuleSource, RuleSourceKind};
use crate::gates::FileSet;
use crate::layout::ProjectLayout;
use crate::rulebook::{self, PrincipleEntry, RuleEntry, Rulebook, RulebookError, RULEBOOK_PATH};

use super::change::{draft, probes_json, rulebook_unreadable, Probed, Probing};
use super::pack_files::{through_symlink, PackedFile};
use super::packs::{
    install_principle_pack, needs, read_pack, PackedPrinciple, PackedRule, RulePack, BUILT_IN,
    MAX_PACK_BYTES,
};
use super::{unknown_response, ChangeRequest, ServiceResponse, ServiceState};

const CLAUSE_START: &str = " (imported from the ";

/// `text` with one provenance clause, naming the latest pack.
fn with_provenance(text: &str, provenance: &str, source: &str) -> String {
    let original = match text.rfind(CLAUSE_START) {
        Some(start) if text.ends_with(')') => &text[..start],
        _ => text,
    };
    format!("{original}{CLAUSE_START}{provenance} pack, {source})")
}

/// `<from>=<to>` prefix rewrites, in the order given.
fn path_maps(request: &ChangeRequest) -> Result<Vec<(String, String)>, String> {
    request
        .map_paths
        .iter()
        .map(|map| {
            let (from, to) = map
                .split_once('=')
                .ok_or_else(|| format!("--map-path {map} is not <from>=<to>"))?;
            let (from, to) = (from.trim_end_matches('/'), to.trim_end_matches('/'));
            if from.is_empty() || !crate::domain::safe_relative_path(from) {
                return Err(format!(
                    "--map-path {map}: {from:?} is not a repository path"
                ));
            }
            if !to.is_empty() && !crate::domain::safe_relative_path(to) {
                return Err(format!("--map-path {map}: {to:?} is not a repository path"));
            }
            Ok((from.to_string(), to.to_string()))
        })
        .collect()
}

fn mapped(path: &str, maps: &[(String, String)]) -> String {
    for (from, to) in maps {
        if path == from {
            return to.clone();
        }
        if let Some(rest) = path
            .strip_prefix(from.as_str())
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return if to.is_empty() {
                rest.to_string()
            } else {
                format!("{to}/{rest}")
            };
        }
    }
    path.to_string()
}

fn map_rule(rule: &mut Rule, maps: &[(String, String)]) {
    let map_all = |paths: &mut Vec<String>| {
        for path in paths.iter_mut() {
            *path = mapped(path, maps);
        }
    };
    map_all(&mut rule.paths);
    if let Some(exclude) = &mut rule.exclude {
        map_all(exclude);
    }
    for unsure in &mut rule.sweep_unsure {
        map_all(&mut unsure.paths);
        map_all(&mut unsure.exclude);
    }
    for example in &mut rule.examples {
        if let Some(path) = &mut example.path {
            *path = mapped(path, maps);
        }
    }
}

/// The directory a path pattern starts from: `apps/web/src/**` gives
/// `apps/web/src`.
fn literal_prefix(pattern: &str) -> String {
    pattern
        .split('/')
        .take_while(|segment| !segment.contains(['*', '?', '{', '[']))
        .collect::<Vec<_>>()
        .join("/")
}

/// The mapping that would aim a rule's paths at this repository: the path's
/// last directory found here, when there is one obvious match.
fn suggested_map(rule: &Rule, directories: &BTreeSet<String>) -> Option<Value> {
    rule.paths.iter().find_map(|pattern| {
        let from = literal_prefix(pattern);
        let name = from.rsplit('/').next().filter(|name| !name.is_empty())?;
        let mut matches = directories
            .iter()
            .filter(|directory| directory.rsplit('/').next() == Some(name) && **directory != from);
        match (matches.next(), matches.next()) {
            (Some(to), None) => {
                Some(json!({"from": from, "to": to, "flag": format!("--map-path {from}={to}")}))
            }
            _ => None,
        }
    })
}

/// How the rule's link to its principle lands in this rulebook.
enum Link {
    /// It serves this principle, drafted from the pack or already held.
    Serves(String),
    /// As `Serves`, but the id held here says something else.
    Conflict {
        id: String,
        here: String,
        pack: String,
    },
    /// Neither the pack nor the rulebook holds it, so the rule serves nothing.
    Missing(String),
    /// The rule never named a principle.
    None,
}

fn packed_principle_for<'a>(pack: &'a RulePack, reference: &str) -> Option<&'a PackedPrinciple> {
    let tail = reference.rsplit(':').next().unwrap_or(reference);
    pack.principles
        .iter()
        .find(|principle| principle.id == reference || principle.id == format!("principle.{tail}"))
}

fn link(rule: &Rule, pack: &RulePack, book: &Rulebook) -> Link {
    let reference = match (&rule.source.kind, rule.source.reference.as_deref()) {
        (RuleSourceKind::Principle, Some(reference)) => reference,
        _ => return Link::None,
    };
    let packed = packed_principle_for(pack, reference);
    let held = book
        .principle_named(reference)
        .or_else(|| packed.and_then(|principle| book.principle_named(&principle.id)));
    match (held, packed) {
        (Some(held), Some(packed)) if held.principle.statement != packed.statement => {
            Link::Conflict {
                id: held.id.as_str().into(),
                here: held.principle.statement.clone(),
                pack: packed.statement.clone(),
            }
        }
        (Some(held), _) => Link::Serves(held.id.as_str().into()),
        (None, Some(packed))
            if !book
                .retired
                .iter()
                .any(|entry| entry.id.as_str() == packed.id) =>
        {
            Link::Serves(packed.id.clone())
        }
        _ => Link::Missing(reference.to_string()),
    }
}

/// One rule of the pack, as this repository would receive it.
struct Candidate<'a> {
    id: Option<RecordId>,
    packed: &'a PackedRule,
    rule: Rule,
    link: Link,
    selected: bool,
    held: bool,
    conflicts: Vec<String>,
    /// Why static reading already says it cannot work here.
    unusable: Option<String>,
    suggestion: Option<Value>,
    covers_nothing: bool,
    probe: Option<crate::probe::ProbeReport>,
}

impl Candidate<'_> {
    fn runs_commands(&self) -> bool {
        !matches!(self.rule.enforcer, Enforcer::Ast { .. })
    }

    fn status(&self) -> &'static str {
        if self.held {
            "held"
        } else if !self.conflicts.is_empty() {
            "file_conflict"
        } else if self.covers_nothing {
            "needs_mapping"
        } else if self.unusable.is_some()
            || self.probe.as_ref().is_some_and(|probe| !probe.validatable)
        {
            "not_validatable"
        } else if self.runs_commands() {
            "needs_consent"
        } else {
            "ready"
        }
    }

    fn reason(&self) -> Option<String> {
        self.unusable
            .clone()
            .or_else(|| self.probe.as_ref().and_then(|probe| probe.reason.clone()))
    }
}

/// What each carried file meets in this repository.
fn file_states(root: &Path, pack: &RulePack) -> Result<BTreeMap<String, &'static str>, String> {
    let mut states = BTreeMap::new();
    for file in &pack.files {
        if let Some(link) = through_symlink(root, &file.path) {
            return Err(format!(
                "it would write {} through the symbolic link {link}",
                file.path
            ));
        }
        let state = match std::fs::read(root.join(&file.path)) {
            Ok(bytes) if bytes == file.text.as_bytes() => "present",
            Ok(_) => "conflict",
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "new",
            Err(_) => "conflict",
        };
        states.insert(file.path.clone(), state);
    }
    Ok(states)
}

/// What a person sees before anything happens: every entry and every file.
fn review_json(
    pack: &RulePack,
    book: &Rulebook,
    held: impl Fn(&str) -> bool,
    selected_principles: &BTreeSet<String>,
    candidates: &[Candidate<'_>],
    states: &BTreeMap<String, &'static str>,
) -> Value {
    let principles = pack.principles.iter().map(|principle| {
        let here = book
            .principles
            .iter()
            .find(|entry| entry.id.as_str() == principle.id);
        json!({
            "id": principle.id,
            "kind": "principle",
            "statement": principle.statement,
            "source": principle.source,
            "selected": selected_principles.contains(&principle.id),
            "status": match here {
                Some(entry) if entry.principle.statement != principle.statement => "conflict",
                Some(_) => "held",
                None if held(&principle.id) => "held",
                None => "ready",
            },
        })
    });
    let rules = candidates.iter().map(|candidate| {
        let rule = &candidate.rule;
        let (_, target) = crate::gates::mechanism_label(rule);
        json!({
            "id": candidate.packed.id,
            "kind": "rule",
            "statement": rule.statement,
            "strength": rule.strength.label(),
            "enforcer": rule.enforcer.kind(),
            "target": target,
            "runs_command": candidate.runs_commands(),
            "paths": rule.paths,
            "scoped_at_source": candidate.packed.scoped_at_source,
            "widened": candidate.packed.scoped_at_source && rule.paths.is_empty(),
            "serves": match &candidate.link {
                Link::Serves(id) | Link::Conflict { id, .. } => Some(id.as_str()),
                _ => None,
            },
            "principle_conflict": matches!(candidate.link, Link::Conflict { .. }),
            "principle_missing": matches!(candidate.link, Link::Missing(_)),
            "files": candidate.packed.files,
            "selected": candidate.selected,
            "status": candidate.status(),
            "reason": candidate.reason(),
            "suggestion": candidate.suggestion,
            "probe": candidate.probe.as_ref().map_or(json!({"status": "not_run"}), |probe| json!(probe)),
        })
    });
    let files = pack.files.iter().map(|file| {
        json!({
            "path": file.path,
            "bytes": file.text.len(),
            "executable": file.executable,
            "status": states.get(&file.path).copied().unwrap_or("new"),
            "needed_by": pack.rules.iter().filter(|rule| rule.files.contains(&file.path)).map(|rule| rule.id.as_str()).collect::<Vec<_>>(),
        })
    });
    json!({
        "pack": {"name": pack.name, "version": pack.version, "source": pack.source, "digest": pack.digest},
        "entries": principles.chain(rules).collect::<Vec<_>>(),
        "files": files.collect::<Vec<_>>(),
    })
}

/// Files written so far, for undoing a failed or partial import.
#[derive(Default)]
struct Written {
    files: Vec<String>,
}

impl Written {
    fn write(&mut self, root: &Path, file: &PackedFile) -> std::io::Result<()> {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.text)?;
        self.files.push(file.path.clone());
        #[cfg(unix)]
        if file.executable {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
        Ok(())
    }

    /// Remove the written files `keep` rejects, and the directories that
    /// leaves empty.
    fn undo(&mut self, root: &Path, keep: impl Fn(&str) -> bool) {
        self.files.retain(|path| {
            if keep(path) {
                return true;
            }
            let full = root.join(path);
            let _ = std::fs::remove_file(&full);
            let mut parent = full.parent();
            while let Some(directory) = parent.filter(|directory| *directory != root) {
                if std::fs::remove_dir(directory).is_err() {
                    break;
                }
                parent = directory.parent();
            }
            false
        });
    }
}

pub(super) fn import(
    layout: &ProjectLayout,
    request_id: String,
    request: &ChangeRequest,
) -> ServiceResponse {
    let root = layout.project_root();
    let source = request
        .import_pack
        .as_deref()
        .unwrap_or("the uploaded pack");
    let text = match &request.import_text {
        Some(text) if text.len() as u64 > MAX_PACK_BYTES => {
            return needs(request_id, "The pack is larger than 1 MiB.".into())
        }
        Some(text) => text.clone(),
        None => match read_pack(root, source) {
            Ok(text) => text,
            Err(error) => return needs(request_id, error),
        },
    };
    let trusted = request.import_text.is_none() && BUILT_IN.contains(&source);
    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => return needs(request_id, format!("{source} is not JSON: {error}")),
    };
    if value.get("schema").and_then(Value::as_str) == Some(crate::catalogue::PACK_SCHEMA) {
        return install_principle_pack(root, request_id, &text, request.preview);
    }
    let pack: RulePack = match serde_json::from_value(value) {
        Ok(pack) => pack,
        Err(error) => return needs(request_id, format!("{source} is not a rule pack: {error}")),
    };
    if let Err(error) = pack.verify() {
        return needs(request_id, format!("The pack was refused: {error}."));
    }
    if request.allow_commands && request.actor != ActorKind::Person {
        let mut response = needs(
            request_id,
            "Only a person can allow a pack's commands to run. Stop and ask them; nothing was changed.".into(),
        );
        response.data = json!({"written": false});
        return response;
    }
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
    let states = match file_states(root, &pack) {
        Ok(states) => states,
        Err(reason) => return needs(request_id, format!("The pack was refused: {reason}.")),
    };
    let maps = match path_maps(request) {
        Ok(maps) => maps,
        Err(reason) => return needs(request_id, reason),
    };
    let unknown = request
        .only
        .iter()
        .filter(|id| {
            !pack.rules.iter().any(|rule| rule.id == **id)
                && !pack.principles.iter().any(|principle| principle.id == **id)
        })
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return needs(
            request_id,
            format!(
                "Nothing was imported: the pack does not hold {}.",
                unknown.join(", ")
            ),
        );
    }
    let everything = request.only.is_empty();
    let chosen = |id: &str| everything || request.only.iter().any(|only| only == id);
    let held = |id: &str| {
        book.rules.iter().any(|entry| entry.id.as_str() == id)
            || book.principles.iter().any(|entry| entry.id.as_str() == id)
            || book.retired.iter().any(|entry| entry.id.as_str() == id)
    };
    let provenance = format!("{}@{}", pack.name, pack.version);
    let repository = match crate::probe::repository_files(root) {
        Ok(files) => files,
        Err(error) => return unknown_response("change", request_id, error),
    };
    let directories = directories_of(&repository);

    let mut selected_principles = pack
        .principles
        .iter()
        .filter(|principle| chosen(&principle.id))
        .map(|principle| principle.id.clone())
        .collect::<BTreeSet<_>>();
    let mut candidates = Vec::new();
    for packed in &pack.rules {
        let mut rule = packed.rule.clone();
        map_rule(&mut rule, &maps);
        rule.rationale = with_provenance(&rule.rationale, &provenance, &pack.source);
        let link = link(&rule, &pack, &book);
        let selected = chosen(&packed.id);
        if selected {
            if let Link::Serves(id) = &link {
                if pack.principles.iter().any(|principle| principle.id == *id) {
                    selected_principles.insert(id.clone());
                }
            }
        }
        rule.source = match &link {
            Link::Serves(id) | Link::Conflict { id, .. } => RuleSource {
                kind: RuleSourceKind::Principle,
                reference: Some(id.clone()),
                provenance: rule.source.provenance.clone(),
            },
            _ => RuleSource {
                kind: RuleSourceKind::Pack,
                reference: Some(provenance.clone()),
                provenance: Vec::new(),
            },
        };
        let id = RecordId::new(packed.id.as_str()).ok();
        let held_here = held(&packed.id);
        let conflicts = packed
            .files
            .iter()
            .filter(|path| states.get(*path) == Some(&"conflict"))
            .cloned()
            .collect::<Vec<_>>();
        let covers_nothing =
            !rule.paths.is_empty() && !repository.paths().any(|path| rule.applies_to(path));
        let unusable = rule.enforcer.command().and_then(|command| {
            let sequences = crate::gates::parse_command(command).ok()?;
            crate::probe::missing_file(&sequences, |path| {
                root.join(path).exists() || pack.files.iter().any(|file| file.path == path)
            })
            .map(|file| format!("its command runs {file}, which is neither here nor in the pack"))
        });
        // A tree-sitter query executes nothing, so an AST rule shows what it
        // would find even in the review.
        let probe = match (&id, &rule.enforcer) {
            (Some(id), Enforcer::Ast { .. }) if !held_here && !covers_nothing => {
                Some(crate::probe::probe(layout, id, &rule))
            }
            _ => None,
        };
        let suggestion = covers_nothing
            .then(|| suggested_map(&rule, &directories))
            .flatten();
        candidates.push(Candidate {
            id,
            packed,
            rule,
            link,
            selected,
            held: held_here,
            conflicts,
            unusable,
            suggestion,
            covers_nothing,
            probe,
        });
    }
    let review = review_json(
        &pack,
        &book,
        held,
        &selected_principles,
        &candidates,
        &states,
    );

    let consented = trusted || request.allow_commands;
    let mut after = book.clone();
    let mut drafted = Vec::new();
    let mut skipped = Vec::new();
    for principle in pack
        .principles
        .iter()
        .filter(|principle| selected_principles.contains(&principle.id))
    {
        let Ok(id) = RecordId::new(principle.id.as_str()) else {
            continue;
        };
        if held(&principle.id) {
            skipped
                .push(json!({"id": id.as_str(), "reason": "the rulebook already holds this id"}));
            continue;
        }
        drafted.push(json!({"id": id.as_str(), "kind": "principle"}));
        after.principles.push(PrincipleEntry {
            id,
            principle: Principle {
                statement: principle.statement.clone(),
                source: principle
                    .source
                    .clone()
                    .unwrap_or(crate::domain::PrincipleSource::Custom),
                rationale: Some(with_provenance(
                    principle
                        .rationale
                        .as_deref()
                        .unwrap_or("A principle the pack holds."),
                    &provenance,
                    &pack.source,
                )),
            },
        });
    }

    let mut needs_consent = Vec::new();
    let mut file_conflicts = Vec::new();
    let mut unvalidated = Vec::new();
    let mut principle_conflicts = Vec::new();
    let mut principle_missing = Vec::new();
    let mut widened = Vec::new();
    let mut to_probe = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if !candidate.selected {
            continue;
        }
        let name = candidate.packed.id.as_str();
        if candidate.held {
            skipped.push(json!({"id": name, "reason": "the rulebook already holds this id"}));
            continue;
        }
        if !candidate.conflicts.is_empty() {
            file_conflicts.push(json!({"id": name, "files": candidate.conflicts}));
            continue;
        }
        if matches!(candidate.status(), "needs_mapping" | "not_validatable")
            && !request.allow_unvalidated
        {
            unvalidated.push(json!({
                "id": name,
                "reason": candidate.reason().unwrap_or_else(|| "no file here matches the rule's paths".into()),
                "suggestion": candidate.suggestion,
                "probe": candidate.probe,
            }));
            continue;
        }
        if candidate.runs_commands() && !consented {
            needs_consent.push(json!({
                "id": name,
                "command": crate::gates::mechanism_label(&candidate.rule).1,
                "files": candidate.packed.files,
            }));
            continue;
        }
        match &candidate.link {
            Link::Conflict { id, here, pack } => principle_conflicts
                .push(json!({"rule": name, "principle": id, "here": here, "pack": pack})),
            Link::Missing(reference) => {
                principle_missing.push(json!({"rule": name, "principle": reference}))
            }
            _ => {}
        }
        if candidate.packed.scoped_at_source && candidate.rule.paths.is_empty() {
            widened.push(name.to_string());
        }
        to_probe.push(index);
    }

    let report = |verb: &str,
                  count: usize,
                  not_validatable: usize,
                  skipped: usize,
                  waiting: usize,
                  conflicts: usize| {
        format!(
            "The {provenance} pack: {count} draft(s) {verb}, {skipped} skipped because the rulebook already holds the id, {not_validatable} not validatable here{}{}. Nothing takes force until a person accepts it.",
            if waiting > 0 { format!(", {waiting} waiting for a person's consent to run its command") } else { String::new() },
            if conflicts > 0 { format!(", {conflicts} left out because a file here differs") } else { String::new() },
        )
    };
    let ask_consent = |response: &mut ServiceResponse, waiting: usize| {
        if waiting > 0 {
            response.blocking_questions = vec![format!(
                "{waiting} rule(s) in this pack run commands. A person reads them, then repeats the import with --allow-commands; an agent stops here and asks."
            )];
            response.permitted_actions.insert(
                0,
                "stop and ask a person: only a person can pass --allow-commands".into(),
            );
        }
    };

    if request.preview {
        // The inert review: every chosen rule, with the command rules marked
        // as not run. Nothing is written and no command starts.
        for index in &to_probe {
            let candidate = &candidates[*index];
            drafted.push(json!({
                "id": candidate.packed.id,
                "kind": "rule",
                "probe": candidate.probe.as_ref().map_or(json!({"status": "not_run"}), |probe| json!(probe)),
            }));
            if let Some(Ok(entry)) = candidate
                .id
                .clone()
                .map(|id| RuleEntry::new(id, candidate.rule.clone()))
            {
                after.rules.push(entry);
            }
        }
        let checked = rulebook::render(&after)
            .and_then(|text| rulebook::parse(&text, rulebook::Revision::Uncommitted));
        if let Err(error) = checked {
            return rulebook_unreadable(request_id, &error);
        }
        let waiting = needs_consent.len();
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            report(
                "would be written",
                drafted.len(),
                unvalidated.len(),
                skipped.len(),
                waiting,
                file_conflicts.len(),
            ),
        );
        response.permitted_actions = vec![
            "repeat without --preview to write the drafts".into(),
            "cancel without changing the rulebook".into(),
        ];
        ask_consent(&mut response, waiting);
        response.data = json!({
            "pack": review["pack"],
            "path": RULEBOOK_PATH,
            "review": review,
            "drafts": drafted,
            "skipped": skipped,
            "not_validatable": unvalidated,
            "needs_consent": needs_consent,
            "file_conflicts": file_conflicts,
            "principle_conflicts": principle_conflicts,
            "principle_missing": principle_missing,
            "widened": widened,
            "preview_only": true,
            "written": false,
        });
        return response;
    }

    // Applying. Files arrive only for the rules that passed the review, and
    // before those rules are probed, because the probe runs them.
    let mut written = Written::default();
    if consented {
        let wanted = to_probe
            .iter()
            .flat_map(|index| candidates[*index].packed.files.iter())
            .collect::<BTreeSet<_>>();
        for file in pack.files.iter().filter(|file| wanted.contains(&file.path)) {
            if states.get(&file.path) != Some(&"new") {
                continue;
            }
            if let Err(error) = written.write(root, file) {
                written.undo(root, |_| false);
                return unknown_response(
                    "change",
                    request_id,
                    format!(
                        "{} could not be written, so nothing was: {error}",
                        file.path
                    ),
                );
            }
        }
    }
    let mut probes = Vec::new();
    let mut fails_today = Vec::new();
    for index in &to_probe {
        let candidate = &candidates[*index];
        let Some(id) = candidate.id.clone() else {
            continue;
        };
        let report = match &candidate.probe {
            Some(report) => report.clone(),
            None => crate::probe::probe(layout, &id, &candidate.rule),
        };
        if !report.validatable && !request.allow_unvalidated {
            unvalidated.push(json!({
                "id": id.as_str(),
                "reason": report.reason,
                "suggestion": candidate.suggestion,
                "probe": report,
            }));
            continue;
        }
        if report.fails_today() {
            fails_today.push(json!({
                "id": id.as_str(),
                "strength": candidate.rule.strength.label(),
                "reason": report.samples.first().map(|sample| sample.message.as_str()).unwrap_or_default(),
            }));
        }
        match RuleEntry::new(id.clone(), candidate.rule.clone()) {
            Ok(entry) => after.rules.push(entry),
            Err(error) => {
                written.undo(root, |_| false);
                return rulebook_unreadable(request_id, &error);
            }
        }
        drafted.push(json!({"id": id.as_str(), "kind": "rule", "probe": report}));
        probes.push(Probed {
            id,
            rule: candidate.rule.clone(),
            report,
        });
    }
    // A file whose rules all fell away has no reason to stay.
    let needed = probes
        .iter()
        .filter_map(|probed| {
            candidates
                .iter()
                .find(|candidate| candidate.id.as_ref() == Some(&probed.id))
        })
        .flat_map(|candidate| candidate.packed.files.iter().cloned())
        .collect::<BTreeSet<_>>();
    written.undo(root, |path| needed.contains(path));

    let waiting = needs_consent.len();
    let summary = report(
        "written",
        drafted.len(),
        unvalidated.len(),
        skipped.len(),
        waiting,
        file_conflicts.len(),
    );
    let data = |written_any: bool, files: &[String]| {
        json!({
            "pack": review["pack"],
            "path": RULEBOOK_PATH,
            "review": review,
            "drafts": drafted,
            "skipped": skipped,
            "not_validatable": unvalidated,
            "needs_consent": needs_consent,
            "file_conflicts": file_conflicts,
            "principle_conflicts": principle_conflicts,
            "principle_missing": principle_missing,
            "fails_today": fails_today,
            "widened": widened,
            "files_written": files,
            "preview_only": false,
            "written": written_any,
        })
    };
    if drafted.is_empty() {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            if waiting > 0 {
                ServiceState::NeedsDecision
            } else {
                ServiceState::Success
            },
            summary,
        );
        ask_consent(&mut response, waiting);
        response.data = data(false, &[]);
        return response;
    }
    let (_, mut response, probed) = match draft(
        root,
        request_id,
        false,
        &after,
        Probing {
            allow_unvalidated: request.allow_unvalidated,
            known: &probes,
        },
        summary.clone(),
        summary,
    ) {
        Ok(drafted) => drafted,
        Err(response) => {
            written.undo(root, |_| false);
            return *response;
        }
    };
    response.permitted_actions = vec![
        "wh change --accept (a person accepts the drafts, which commits the file)".into(),
        "wh change --withdraw".into(),
        "wh dash".into(),
    ];
    let mut to_commit = written.files.clone();
    to_commit.sort();
    if !to_commit.is_empty() {
        response.permitted_actions.insert(
            0,
            format!(
                "git add {} && git commit -m \"Add the files the rules run\" (before accepting: accepting commits the rulebook only)",
                to_commit.join(" ")
            ),
        );
        let hooks = crate::wire::git_hooks(root);
        if !hooks.pre_commit || !hooks.pre_push {
            response.permitted_actions.push(
                "wh init --action wire (these rules take force at the gates once they are wired)"
                    .into(),
            );
        }
    }
    if waiting > 0 {
        response.state = ServiceState::NeedsDecision;
        ask_consent(&mut response, waiting);
    }
    response.data = data(true, &to_commit);
    response.data["probes"] = probes_json(&probed);
    response
}

/// Every directory that holds a repository file.
fn directories_of(files: &FileSet) -> BTreeSet<String> {
    let mut directories = BTreeSet::new();
    for path in files.paths() {
        let mut current = path;
        while let Some((parent, _)) = current.rsplit_once('/') {
            if !directories.insert(parent.to_string()) {
                break;
            }
            current = parent;
        }
    }
    directories
}
