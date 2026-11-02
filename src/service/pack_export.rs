//! `wh change --export-pack <file>`: a chosen part of the committed rulebook
//! as a rule pack. A chosen rule brings the principle it serves and the
//! files its command names, all read from the commit, never the working
//! tree, so one revision and one selection make one pack, byte for byte.
//! The dashboard's download and its picker read the same functions.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{json, Value};

use crate::domain::{PrincipleSource, RuleSourceKind};
use crate::layout::ProjectLayout;
use crate::projection::pack::{portability, PackPlan, PrinciplePlan, RulePlan, ServedPrinciple};
use crate::rulebook::{Revision, RuleEntry, Rulebook, RULEBOOK_PATH};

use super::change::{committed, rulebook_unreadable};
use super::pack_files::{refused_path, Tree};
use super::packs::{
    export_principles, needs, PackedPrinciple, PackedRule, RulePack, MAX_PACK_BYTES,
};
use super::{project_error, unknown_response, ChangeRequest, ServiceResponse, ServiceState};

/// What goes into a pack, whoever asks: the CLI's flags or the page's body.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackChoice {
    /// Rule or principle ids; empty means everything that can leave.
    pub only: Vec<String>,
    /// Every rule keeps its paths.
    pub include_paths: bool,
    /// These rules keep their paths.
    pub keep_paths: Vec<String>,
    /// Committed files to carry beyond those a command names.
    pub with_files: Vec<String>,
    pub name: Option<String>,
    pub version: Option<String>,
}

/// What travelling takes for one rule.
struct Needs {
    files: Vec<String>,
    suggested: Vec<String>,
    tools: Vec<String>,
    /// Why it cannot leave.
    blocked: Option<String>,
}

fn needs_of(root: &Path, tree: &Tree<'_>, entry: &RuleEntry) -> Needs {
    let mut found = Needs {
        files: Vec::new(),
        suggested: Vec::new(),
        tools: Vec::new(),
        blocked: None,
    };
    if entry.rule.source.kind == RuleSourceKind::Mission {
        found.blocked = Some("it is derived from this repository's mission".into());
        return found;
    }
    let Some(sequences) = entry
        .rule
        .enforcer
        .command()
        .and_then(|command| crate::gates::parse_command(command).ok())
    else {
        return found;
    };
    let required = crate::probe::required_files(&sequences);
    for path in crate::probe::named_paths(&sequences) {
        if !tree.tracked(&path) {
            if required.contains(&path) && root.join(&path).is_file() {
                found.blocked = Some(format!("it runs {path}, which is not committed"));
            }
            continue;
        }
        if let Some(reason) = refused_path(&path) {
            found.blocked = Some(format!("it runs {path}, and {reason}"));
        } else if let Err(reason) = tree.file(&path) {
            found.blocked = Some(format!("it runs a file that cannot travel: {reason}"));
        } else {
            found.files.push(path);
        }
    }
    found.suggested = tree.mentioned(&found.files);
    found.tools = sequences
        .iter()
        .filter_map(|argv| argv.first())
        .filter(|program| !program.contains('/'))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    found
}

fn principle_origin(source: &PrincipleSource) -> String {
    match source {
        PrincipleSource::Pstack { .. } => "pstack".into(),
        PrincipleSource::Pack { pack, .. } => pack.clone(),
        PrincipleSource::Custom => "custom".into(),
    }
}

/// A name or version a pack may carry: it ends up in a rule's source.
fn label_problem(what: &str, value: &str) -> Option<String> {
    let plain = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._+-".contains(character));
    (!plain).then(|| {
        format!("--pack-{what} is at most 64 letters, digits and . _ + -; {value:?} is not")
    })
}

fn default_name(root: &Path) -> String {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || "._+-".contains(character) {
                character
            } else {
                '-'
            }
        })
        .take(64)
        .collect::<String>();
    if name.is_empty() {
        "rules".into()
    } else {
        name
    }
}

fn default_version(book: &Rulebook) -> String {
    match &book.revision {
        Revision::Commit(hash) if !hash.is_empty() => hash.chars().take(12).collect(),
        _ => "1".into(),
    }
}

fn plan(root: &Path, book: &Rulebook, tree: &Tree<'_>, only: &BTreeSet<&str>) -> PackPlan {
    let rules = book
        .rules
        .iter()
        .filter(|entry| only.is_empty() || only.contains(entry.id.as_str()))
        .map(|entry| {
            let found = needs_of(root, tree, entry);
            let (_, target) = crate::gates::mechanism_label(&entry.rule);
            let language = match &entry.rule.enforcer {
                crate::domain::Enforcer::Ast { language, .. } => language.clone(),
                _ => None,
            };
            RulePlan {
                id: entry.id.as_str().into(),
                statement: entry.rule.statement.clone(),
                strength: entry.rule.strength.label(),
                enforcer: entry.rule.enforcer.kind(),
                language,
                target,
                can_leave: found.blocked.is_none(),
                portability: portability(
                    found.blocked.is_none(),
                    &found.files,
                    &found.tools,
                    &entry.rule.paths,
                ),
                reason: found.blocked,
                serves: book
                    .principle_for(&entry.rule)
                    .map(|served| ServedPrinciple {
                        id: served.id.as_str().into(),
                        statement: served.principle.statement.clone(),
                    }),
                files: found.files,
                suggested_files: found.suggested,
                tools: found.tools,
                paths: entry.rule.paths.clone(),
            }
        })
        .collect();
    let principles = book
        .principles
        .iter()
        .map(|entry| PrinciplePlan {
            id: entry.id.as_str().into(),
            statement: entry.principle.statement.clone(),
            origin: principle_origin(&entry.principle.source),
            rules: book
                .served_by(&entry.id)
                .iter()
                .map(|rule| rule.id.as_str().to_string())
                .collect(),
        })
        .collect();
    PackPlan {
        revision: match &book.revision {
            Revision::Commit(hash) => Some(hash.clone()),
            Revision::Uncommitted => None,
        },
        name: default_name(root),
        version: default_version(book),
        rules,
        principles,
    }
}

struct Built {
    pack: RulePack,
    text: String,
    added_by_closure: Vec<String>,
    left_out: Vec<Value>,
    suggested: Vec<String>,
    dirty: Vec<String>,
    plan: PackPlan,
}

fn packed_principle(book: &Rulebook, id: &str) -> Option<PackedPrinciple> {
    book.principles
        .iter()
        .find(|entry| entry.id.as_str() == id)
        .map(|entry| PackedPrinciple {
            id: id.into(),
            statement: entry.principle.statement.clone(),
            rationale: entry.principle.rationale.clone(),
            source: match entry.principle.source {
                PrincipleSource::Custom => None,
                ref source => Some(source.clone()),
            },
        })
}

/// The rule as it travels: its link to the principle by record id, its
/// paths only when asked for.
fn packed_rule(book: &Rulebook, entry: &RuleEntry, keep: bool) -> PackedRule {
    let mut rule = entry.rule.clone();
    if let Some(served) = book.principle_for(&entry.rule) {
        rule.source.reference = Some(served.id.as_str().into());
    }
    let scoped_at_source = !keep && !rule.paths.is_empty();
    if !keep {
        rule.paths.clear();
        rule.sweep_unsure.clear();
        rule.exclude = rule.exclude.filter(Vec::is_empty);
        for example in &mut rule.examples {
            example.path = None;
        }
    }
    PackedRule {
        id: entry.id.as_str().into(),
        rule,
        scoped_at_source,
        files: Vec::new(),
    }
}

fn dirty_files(root: &Path, files: &[String]) -> Vec<String> {
    files
        .iter()
        .filter(|path| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(["status", "--porcelain", "--", path])
                .output()
                .is_ok_and(|output| !output.stdout.is_empty())
        })
        .cloned()
        .collect()
}

/// The pack for `choice`, or the sentence that says why not.
fn build(root: &Path, book: &Rulebook, choice: &PackChoice) -> Result<Built, String> {
    let tree = Tree::at_head(root)?;
    for (what, value) in [("name", &choice.name), ("version", &choice.version)] {
        if let Some(problem) = value.as_deref().and_then(|text| label_problem(what, text)) {
            return Err(problem);
        }
    }
    let only = choice
        .only
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let is_rule = |id: &str| book.rules.iter().any(|entry| entry.id.as_str() == id);
    let is_principle = |id: &str| book.principles.iter().any(|entry| entry.id.as_str() == id);
    let mut refused = Vec::new();
    for id in &only {
        if book.retired.iter().any(|entry| entry.id.as_str() == *id) {
            refused.push(format!("{id} is retired"));
        } else if !is_rule(id) && !is_principle(id) {
            refused.push(format!("{id} is not in the accepted rulebook"));
        }
    }
    for id in &choice.keep_paths {
        if !is_rule(id) || !(only.is_empty() || only.contains(id.as_str())) {
            refused.push(format!("--keep-paths {id} is not a rule in this pack"));
        }
    }
    let mut left_out = Vec::new();
    let mut rules = Vec::new();
    let mut suggested_by: Vec<(usize, Vec<String>)> = Vec::new();
    let mut carried = BTreeSet::new();
    for entry in &book.rules {
        let chosen = only.contains(entry.id.as_str());
        if !only.is_empty() && !chosen {
            continue;
        }
        let found = needs_of(root, &tree, entry);
        if let Some(reason) = found.blocked {
            if chosen {
                refused.push(format!("{} cannot leave: {reason}", entry.id.as_str()));
            } else {
                left_out.push(json!({"rule": entry.id.as_str(), "reason": reason}));
            }
            continue;
        }
        let keep =
            choice.include_paths || choice.keep_paths.iter().any(|id| id == entry.id.as_str());
        let mut packed = packed_rule(book, entry, keep);
        carried.extend(found.files.iter().cloned());
        packed.files = found.files;
        suggested_by.push((rules.len(), found.suggested));
        rules.push(packed);
    }
    for path in &choice.with_files {
        if !tree.tracked(path) {
            refused.push(format!("--with-file {path} is not committed"));
        } else if let Some(reason) = refused_path(path) {
            refused.push(format!("--with-file {path}: {reason}"));
        } else if let Err(reason) = tree.file(path) {
            refused.push(format!("--with-file: {reason}"));
        }
    }
    if !refused.is_empty() {
        return Err(format!("Nothing was written: {}.", refused.join("; ")));
    }
    // A chosen file goes with the rules that lead to it. One nothing leads
    // to is what a script builds at run time, so every command rule gets it.
    for path in &choice.with_files {
        let mut placed = false;
        for (index, suggested) in &suggested_by {
            if suggested.contains(path) {
                rules[*index].files.push(path.clone());
                placed = true;
            }
        }
        if !placed {
            for rule in rules
                .iter_mut()
                .filter(|rule| rule.rule.enforcer.command().is_some())
            {
                rule.files.push(path.clone());
            }
        }
        carried.insert(path.clone());
    }
    for rule in &mut rules {
        rule.files.sort();
        rule.files.dedup();
    }
    let mut suggested = suggested_by
        .into_iter()
        .flat_map(|(_, suggested)| suggested)
        .filter(|path| !carried.contains(path))
        .collect::<Vec<_>>();
    suggested.sort();
    suggested.dedup();

    let mut added_by_closure = Vec::new();
    let mut principles = Vec::new();
    if only.is_empty() {
        principles.extend(
            book.principles
                .iter()
                .filter_map(|entry| packed_principle(book, entry.id.as_str())),
        );
    } else {
        let mut ids = only
            .iter()
            .filter(|id| is_principle(id))
            .map(|id| id.to_string())
            .collect::<BTreeSet<_>>();
        for packed in &rules {
            if packed.rule.source.kind == RuleSourceKind::Principle {
                if let Some(id) = packed.rule.source.reference.as_deref() {
                    if is_principle(id) && ids.insert(id.to_string()) {
                        added_by_closure.push(id.to_string());
                    }
                }
            }
        }
        principles.extend(ids.iter().filter_map(|id| packed_principle(book, id)));
    }
    if rules.is_empty() && (only.is_empty() || principles.is_empty()) {
        return Err(
            "No accepted rule can leave this repository yet; accept one first (wh change --accept)."
                .into(),
        );
    }
    let files = carried
        .iter()
        .map(|path| tree.file(path))
        .collect::<Result<Vec<_>, _>>()?;
    let dirty = dirty_files(root, &carried.iter().cloned().collect::<Vec<_>>());
    let name = choice.name.clone().unwrap_or_else(|| default_name(root));
    let version = choice
        .version
        .clone()
        .unwrap_or_else(|| default_version(book));
    let pack = RulePack::sealed(
        &name,
        &version,
        &super::check::findings::repository_identity(root),
        choice.include_paths,
        principles,
        rules,
        files,
    );
    let text = serde_json::to_string_pretty(&pack).unwrap_or_default() + "\n";
    if text.len() as u64 > MAX_PACK_BYTES {
        let mut largest = pack.files.iter().collect::<Vec<_>>();
        largest.sort_by_key(|file| std::cmp::Reverse(file.text.len()));
        return Err(format!(
            "The pack would be {} KiB, over the 1 MiB limit; choose fewer rules. Its largest files: {}.",
            text.len() / 1024,
            largest
                .iter()
                .take(5)
                .map(|file| format!("{} ({} KiB)", file.path, file.text.len() / 1024))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let plan = plan(root, book, &tree, &only);
    Ok(Built {
        pack,
        text,
        added_by_closure,
        left_out,
        suggested,
        dirty,
        plan,
    })
}

/// `wh change --export-pack <file>`.
pub(super) fn export(
    layout: &ProjectLayout,
    request_id: String,
    target: &str,
    request: &ChangeRequest,
) -> ServiceResponse {
    let root = layout.project_root();
    let book = match committed(root) {
        Some(Ok(book)) => book,
        Some(Err(error)) => return rulebook_unreadable(request_id, &error),
        None => {
            return needs(
                request_id,
                format!("{RULEBOOK_PATH} was never committed, so no rule is accepted yet; accept one first (wh change --accept)."),
            )
        }
    };
    if request.principles {
        return export_principles(root, request_id, target, request, &book);
    }
    let choice = PackChoice {
        only: request.only.clone(),
        include_paths: request.include_paths,
        keep_paths: request.keep_paths.clone(),
        with_files: request.with_files.clone(),
        name: request.pack_name.clone(),
        version: request.pack_version.clone(),
    };
    let built = match build(root, &book, &choice) {
        Ok(built) => built,
        Err(reason) => return needs(request_id, reason),
    };
    let pack = &built.pack;
    let path = if Path::new(target).is_absolute() {
        Path::new(target).to_path_buf()
    } else {
        root.join(target)
    };
    if !request.preview {
        if let Err(error) = std::fs::write(&path, &built.text) {
            return unknown_response(
                "change",
                request_id,
                format!("{} could not be written: {error}", path.display()),
            );
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        if request.preview {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "{} rule(s), {} principle(s) and {} file(s) {} {} (digest {}); the rulebook was not changed.",
            pack.rules.len(),
            pack.principles.len(),
            pack.files.len(),
            if request.preview {
                "would be written to"
            } else {
                "were written to"
            },
            path.display(),
            pack.digest
        ),
    );
    response.permitted_actions = vec![format!(
        "in another repository: wh change --import-pack {} --preview",
        path.display()
    )];
    response.data = json!({
        "pack": path.display().to_string(),
        "name": pack.name,
        "version": pack.version,
        "digest": pack.digest,
        "rules": pack.rules.iter().map(|rule| rule.id.as_str()).collect::<Vec<_>>(),
        "principles": pack.principles.iter().map(|principle| principle.id.as_str()).collect::<Vec<_>>(),
        "files": pack.files.iter().map(|file| file.path.as_str()).collect::<Vec<_>>(),
        "added_by_closure": built.added_by_closure,
        "suggested_files": built.suggested,
        "dirty_files": built.dirty,
        "includes_paths": request.include_paths,
        "left_out": built.left_out,
        "preview_only": request.preview,
    });
    if request.preview {
        response.data["plan"] = json!(built.plan);
    }
    response
}

fn committed_for_pack(
    project_dir: &Path,
    request_id: &str,
) -> Result<(ProjectLayout, Rulebook), Box<ServiceResponse>> {
    let layout = ProjectLayout::resolve(project_dir)
        .map_err(|error| Box::new(project_error("change", Some(request_id.into()), error)))?;
    match committed(layout.project_root()) {
        Some(Ok(book)) => Ok((layout, book)),
        Some(Err(error)) => Err(Box::new(rulebook_unreadable(request_id.into(), &error))),
        None => Err(Box::new(needs(
            request_id.into(),
            format!("{RULEBOOK_PATH} was never committed, so no rule is accepted yet."),
        ))),
    }
}

/// What every accepted rule needs to travel, for the page's picker.
pub fn pack_plan(project_dir: &Path) -> ServiceResponse {
    let request_id = "pack-plan";
    let (layout, book) = match committed_for_pack(project_dir, request_id) {
        Ok(found) => found,
        Err(response) => return *response,
    };
    let root = layout.project_root();
    let tree = match Tree::at_head(root) {
        Ok(tree) => tree,
        Err(error) => return unknown_response("change", request_id.into(), error),
    };
    let mut response = ServiceResponse::new(
        request_id.into(),
        "change",
        ServiceState::Success,
        "What each accepted rule needs to travel.",
    );
    response.data = json!(plan(root, &book, &tree, &BTreeSet::new()));
    response
}

/// The pack for `choice` as text and its name, for a browser to download.
/// Nothing is written.
pub fn pack_text(
    project_dir: &Path,
    choice: &PackChoice,
) -> Result<(String, String), Box<ServiceResponse>> {
    let request_id = "pack-export";
    let (layout, book) = committed_for_pack(project_dir, request_id)?;
    match build(layout.project_root(), &book, choice) {
        Ok(built) => Ok((built.text, built.pack.name)),
        Err(reason) => Err(Box::new(needs(request_id.into(), reason))),
    }
}
