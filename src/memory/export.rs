//! Flipping the switch to Virgil without stranding a team's lessons:
//! Whetstone writes its admitted lessons as exactly the file
//! `virgil lessons import-beads` reads (one JSON object of key to body, each
//! body a Beads memory header line, a newline, then the text). Proposals are
//! not lessons and stay in Whetstone's store, listed beside the export for a
//! person to admit after the switch.
//!
//! The importer refuses an export inside the repository (exports, manifests
//! and receipts are private), so the files live under the person's home, one
//! directory per project. The importer does not carry anchors; after the
//! reviewed apply, [`confirm`] reads every key back and re-anchors each
//! anchored lesson with one revision-checked `virgil lessons remember`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::{
    ActorKind, Audience, Lesson, LessonStatus, LessonVerb, RecordBody, Subject, Trust,
};
use crate::storage::ProjectLayout;

use super::store::{BeadsLessons, LessonStore};
use super::virgil::{self, Remember, VirgilCli};
use super::Refusal;

pub const EXPORT_FILE: &str = "lessons.json";
pub const PROPOSALS_FILE: &str = "lessons.proposals.json";

/// One exported lesson: what the importer reads, and what the read-back and
/// re-anchor need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub key: String,
    /// `CURRENT`, `SUPERSEDED` or `OBSOLETE`.
    pub status: &'static str,
    pub scope: &'static str,
    pub header: String,
    pub lesson: Lesson,
}

impl Entry {
    pub fn body(&self) -> String {
        format!("{}\n{}", self.header, self.lesson.text)
    }

    /// Imported entries (the importer excludes OBSOLETE ones).
    pub fn imported(&self) -> bool {
        self.status != "OBSOLETE"
    }
}

/// Everything Whetstone's own store holds, sorted for the flip.
#[derive(Debug, Clone, Default)]
pub struct Export {
    pub entries: BTreeMap<String, Entry>,
    pub proposals: BTreeMap<String, Lesson>,
    /// Keys the importer cannot name as a successor (reported, never rewritten).
    pub reported: Vec<String>,
}

impl Export {
    /// The file `virgil lessons import-beads --file` reads.
    pub fn file(&self) -> Value {
        Value::Object(
            self.entries
                .iter()
                .map(|(key, entry)| (key.clone(), Value::String(entry.body())))
                .collect(),
        )
    }

    pub fn proposals_file(&self) -> Value {
        Value::Object(
            self.proposals
                .iter()
                .map(|(key, lesson)| (key.clone(), json!(lesson)))
                .collect(),
        )
    }

    pub fn personal(&self) -> Vec<String> {
        self.entries
            .values()
            .filter(|entry| entry.imported() && entry.scope == "personal")
            .map(|entry| entry.key.clone())
            .collect()
    }

    pub fn imported(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| entry.imported())
            .count()
    }
}

/// Keys the importer accepts as a successor: lowercase letters, digits, `-`.
pub fn valid_successor_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

/// The Beads memory header line for a lesson.
pub fn header(
    status: &str,
    scope: &str,
    evidence: &str,
    verified: &str,
    successor: Option<&str>,
) -> String {
    let mut line = format!("{status} scope={scope} evidence={evidence} verified={verified}");
    if let Some(successor) = successor {
        line.push_str(&format!(" successor={successor}"));
    }
    line
}

/// `personal` for a person's lesson; `repo` for a group lesson about the
/// repository or a path in it; `team` for any other group lesson.
pub fn scope(lesson: &Lesson) -> &'static str {
    match lesson.audience {
        Audience::Person { .. } => "personal",
        Audience::Group { .. }
            if !lesson.anchors.is_empty()
                || lesson.about.iter().any(|subject| {
                    matches!(subject, Subject::Repo { .. } | Subject::Path { .. })
                }) =>
        {
            "repo"
        }
        Audience::Group { .. } => "team",
    }
}

/// Every import is a person's reviewed write, so it lands validated whatever
/// this says; `inference` keeps an inferred origin visible in the header.
pub fn evidence(trust: Trust) -> &'static str {
    match trust {
        Trust::Inferred => "inference",
        Trust::Validated | Trust::Authoritative => "feedback",
    }
}

/// Lessons already written to Virgil carry its receipt; they are not
/// exported again.
fn in_virgil(lesson: &Lesson) -> bool {
    lesson
        .decision
        .as_ref()
        .and_then(|decision| decision.receipt.as_deref())
        .is_some_and(|receipt| receipt.starts_with("virgil:"))
}

/// Build the export from Whetstone's own store: the latest revision of every
/// key, with `verified=` the date of the revision that admitted it.
pub fn build(local: &BeadsLessons) -> Result<Export, Refusal> {
    let latest = local
        .find()?
        .into_iter()
        .map(|stored| (stored.lesson.key.clone(), stored.lesson))
        .filter(|(_, lesson)| !in_virgil(lesson))
        .collect::<BTreeMap<_, _>>();
    let mut admitted_on = BTreeMap::<String, (u64, String)>::new();
    for record in local.revisions()? {
        let RecordBody::Lesson(lesson) = &record.body else {
            continue;
        };
        if let Some(decision) = lesson
            .decision
            .as_ref()
            .filter(|decision| decision.verb == LessonVerb::Admit)
        {
            let newer = admitted_on
                .get(&lesson.key)
                .map_or(true, |(revision, _)| *revision < record.revision);
            if newer {
                admitted_on.insert(lesson.key.clone(), (record.revision, decision.at.clone()));
            }
        }
    }
    let mut export = Export::default();
    for (key, lesson) in &latest {
        if lesson.status == LessonStatus::Proposed {
            export.proposals.insert(key.clone(), lesson.clone());
            continue;
        }
        if !valid_successor_key(key) {
            export.reported.push(key.clone());
        }
        let successor = lesson.superseded_by.as_deref().filter(|successor| {
            latest
                .get(*successor)
                .is_some_and(|next| next.status == LessonStatus::Admitted)
                && valid_successor_key(successor)
        });
        let status = match lesson.status {
            LessonStatus::Admitted => "CURRENT",
            LessonStatus::Superseded if successor.is_some() => "SUPERSEDED",
            _ => "OBSOLETE",
        };
        let verified = admitted_on
            .get(key)
            .map_or(lesson.stamp.at.as_str(), |(_, at)| at.as_str())
            .chars()
            .take(10)
            .collect::<String>();
        let scope = scope(lesson);
        export.entries.insert(
            key.clone(),
            Entry {
                key: key.clone(),
                status,
                scope,
                header: header(
                    status,
                    scope,
                    evidence(lesson.trust),
                    &verified,
                    successor.filter(|_| status == "SUPERSEDED"),
                ),
                lesson: lesson.clone(),
            },
        );
    }
    Ok(export)
}

/// Where the export lives: outside the repository, private to the person.
pub fn directory(layout: &ProjectLayout) -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or("HOME is not set, so there is nowhere private to write the export")?;
    Ok(home
        .join(".whetstone/exports")
        .join(&layout.project_id()[..20]))
}

fn write_private(path: &Path, value: &Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    text.push('\n');
    std::fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    Ok(())
}

/// Write the export and the proposals beside it; the same store writes the
/// same bytes. Returns the two paths.
pub fn write(layout: &ProjectLayout, export: &Export) -> Result<(PathBuf, PathBuf), String> {
    let directory = directory(layout)?;
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("{}: {error}", directory.display()))?;
    }
    let file = directory.join(EXPORT_FILE);
    let proposals = directory.join(PROPOSALS_FILE);
    write_private(&file, &export.file())?;
    write_private(&proposals, &export.proposals_file())?;
    Ok((file, proposals))
}

/// What confirming the apply found.
#[derive(Debug, Clone, Default)]
pub struct Confirmation {
    /// Imported keys that did not read back with their text.
    pub missing: Vec<String>,
    pub re_anchored: Vec<String>,
    /// Anchor paths dropped because they are gone at HEAD, by key.
    pub dropped: BTreeMap<String, Vec<String>>,
}

/// After the reviewed apply: read every imported key back from Virgil with
/// its text, then re-anchor each anchored current lesson as the person
/// running it. Nothing is re-anchored unless every key read back.
pub fn confirm(
    project_root: &Path,
    export: &Export,
    idempotency: &str,
) -> Result<Confirmation, Refusal> {
    let cli = VirgilCli::new(project_root, ActorKind::Person);
    let mut confirmation = Confirmation::default();
    let mut current = Vec::new();
    for entry in export.entries.values().filter(|entry| entry.imported()) {
        let read = cli.get(&entry.key, entry.scope == "personal")?;
        match read.as_ref().map(virgil::WireLesson::read).transpose()? {
            Some(wire) if wire.text.trim() == entry.lesson.text.trim() => {
                if entry.status == "CURRENT" {
                    current.push((entry, wire));
                }
            }
            _ => confirmation.missing.push(entry.key.clone()),
        }
    }
    if !confirmation.missing.is_empty() {
        return Ok(confirmation);
    }
    for (entry, wire) in current {
        let (mut about, anchors, dropped) = virgil::subjects(project_root, &entry.lesson);
        if !dropped.is_empty() {
            confirmation.dropped.insert(entry.key.clone(), dropped);
        }
        if anchors.is_empty() {
            continue;
        }
        // A rewrite replaces `about` whole: keep what the import gave it.
        for subject in &wire.about {
            if !about.contains(subject) {
                about.push(subject.clone());
            }
        }
        about.truncate(virgil::MAX_SUBJECTS.saturating_sub(anchors.len()));
        cli.remember(&Remember {
            key: &entry.key,
            text: &wire.text,
            audience: if wire.personal { "personal" } else { "channel" },
            about,
            anchors,
            if_revision: Some(wire.revision),
            reason: Some("re-anchored after import"),
            idempotency: virgil::idempotency_ulid(
                &format!("{idempotency}:re-anchor:{}", entry.key),
                virgil::millis(&entry.lesson.stamp.at),
            ),
        })?;
        confirmation.re_anchored.push(entry.key.clone());
    }
    Ok(confirmation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_follow_the_importer_grammar() {
        assert_eq!(
            header("CURRENT", "repo", "feedback", "2026-10-14", None),
            "CURRENT scope=repo evidence=feedback verified=2026-10-14"
        );
        assert_eq!(
            header(
                "SUPERSEDED",
                "team",
                "inference",
                "2026-10-14",
                Some("next-key")
            ),
            "SUPERSEDED scope=team evidence=inference verified=2026-10-14 successor=next-key"
        );
        assert!(valid_successor_key("values-are-floats-2"));
        assert!(!valid_successor_key("Values"));
    }
}
