//! Memory as the softest rung of the rule ladder: Whetstone owns the lesson
//! contract (who may admit, what a lesson may be about, how it is anchored
//! and when it may be stale); where lessons live is a backend behind the
//! record store switch (`whetstone/store.json`).
//!
//! - Standalone (the default): Whetstone's own private Beads store.
//! - Inside Virgil: the Records `lesson` collection, reached only through the
//!   `virgil lessons` commands on the daemon socket ([`virgil`]).
//!
//! The switch changes where admitted lessons live, never what they mean.
//! Proposals always stay in Whetstone's own store until a person admits them;
//! inside Virgil the admit is that person's `virgil lessons remember`.
//! Flipping to Virgil exports the admitted lessons in the shape
//! `virgil lessons import-beads` reads ([`export`]) and flips only once a
//! person confirms the import read back. Agents propose; only a person
//! admits, supersedes or forgets. Nothing here calls a model.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{
    ActorKind, Anchor, AnchorComparison, Audience, ContentDigest, ForgetProposal, Lesson,
    LessonDecision, LessonStamp, LessonStatus, LessonVerb, RecordId, RecordRef, Subject, Trust,
    LESSON_SCHEMA_V1, MAX_LESSON_ITEMS,
};

pub mod export;
pub mod store;
pub mod virgil;

pub use store::{open_store, open_store_as, LessonStore, StoredLesson};

/// The switch that names where lessons live is the record store switch
/// ([`crate::storage::StoreConfig`], `whetstone/store.json`): lessons live
/// wherever every other record does (a legacy `memory.json` moved lessons
/// alone; see [`crate::storage::StoreSwitch`]).
pub use crate::storage::{read_lesson_config as read_config, StoreConfig};

/// Environment variables agent hosts set in the shells they run. Any of them
/// makes the caller an agent, whatever else is true.
pub const AGENT_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CODEX_SANDBOX",
    "CODEX_THREAD_ID",
    "CURSOR_AGENT",
    "GEMINI_CLI",
];

/// Who is calling: a person at a terminal, or an agent. Anything that is not
/// clearly a person (no terminal on stdin, an agent host's marker in the
/// environment, or a launcher's `VIRGIL_VIA=agent:<client>`) is an agent, so
/// an agent can never pass as a person by leaving something out.
pub fn detect_actor() -> ActorKind {
    use std::io::IsTerminal;
    let marked = AGENT_MARKERS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()))
        || std::env::var("VIRGIL_VIA").is_ok_and(|via| via.starts_with("agent:"));
    if !marked && std::io::stdin().is_terminal() {
        ActorKind::Person
    } else {
        ActorKind::Agent
    }
}

/// The agent host running this process, as Virgil's client name: a
/// launcher's own `VIRGIL_VIA=agent:<client>` first, then the host markers
/// (claude, codex, cursor, gemini), else `whetstone`.
pub fn agent_host() -> String {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    if let Some(client) = std::env::var("VIRGIL_VIA")
        .ok()
        .and_then(|via| via.strip_prefix("agent:").map(str::to_owned))
    {
        return virgil_client(&client);
    }
    let host = if set("CLAUDECODE") || set("CLAUDE_CODE_ENTRYPOINT") {
        "claude"
    } else if set("CODEX_SANDBOX") || set("CODEX_THREAD_ID") {
        "codex"
    } else if set("CURSOR_AGENT") {
        "cursor"
    } else if set("GEMINI_CLI") {
        "gemini"
    } else {
        "whetstone"
    };
    host.to_string()
}

/// A host name as Virgil accepts a client (`WriterAttribution::new`): 1 to
/// 64 bytes, the first an ASCII letter or digit, the rest lowercase letters,
/// digits, `-`, `_` or `.`. Anything else becomes `whetstone`, so a write
/// is never refused for its marker.
pub fn virgil_client(host: &str) -> String {
    let client = host
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || "-_.".contains(character)
            {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    let client = client.trim_start_matches(|character: char| !character.is_ascii_alphanumeric());
    if client.is_empty() || client.len() > 64 {
        "whetstone".into()
    } else {
        client.to_string()
    }
}

/// Why a lesson request was refused, with a stable code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Refusal {
    pub code: &'static str,
    pub message: String,
}

impl Refusal {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

/// Fields a writer may never supply: the store stamps provenance, and only a
/// person's decision sets trust, status or the decision itself.
pub const STAMPED_FIELDS: &[&str] = &[
    "provenance",
    "stamp",
    "who",
    "session",
    "task",
    "branch",
    "commit",
    "trust",
    "status",
    "decision",
];

/// What a writer asks to remember, before Whetstone stamps and anchors it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LessonDraft {
    pub key: String,
    pub text: String,
    pub about: Vec<Subject>,
    pub concepts: Vec<String>,
    pub audience: Audience,
    /// Paths the writer adds as anchors beyond the paths the lesson names.
    pub anchor_paths: Vec<String>,
    /// The key of the lesson this one corrects.
    pub supersedes: Option<String>,
}

/// A key from free text: the first words, lowercase, hyphenated.
pub fn key_from_text(text: &str) -> String {
    let mut key = String::new();
    for word in text
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(8)
    {
        if key.len() + word.len() + 1 > 60 {
            break;
        }
        if !key.is_empty() {
            key.push('-');
        }
        key.push_str(&word.to_ascii_lowercase());
    }
    if key.len() < 3 {
        key = format!(
            "lesson-{}",
            &format!("{:x}", Sha256::digest(text.as_bytes()))[..12]
        );
    }
    key
}

/// The record id a lesson key is stored under.
pub fn record_id(key: &str) -> Result<RecordId, crate::domain::DomainError> {
    RecordId::new(format!("lesson.{key}"))
}

/// The default audience: the attached Virgil group inside Virgil, the
/// repository's team standalone.
pub fn default_audience(
    config: &StoreConfig,
    project_root: &Path,
    project_label: &str,
) -> Audience {
    let group = match config {
        StoreConfig::Beads => None,
        StoreConfig::Virgil => virgil::attached_group(project_root),
    };
    Audience::Group {
        name: group.unwrap_or_else(|| project_label.to_string()),
    }
}

/// Stamp provenance from the environment the store sees, never from the
/// request: who (Git's user, or the agent), the agent host's session, the one
/// Beads task in progress, and the branch and commit.
pub fn stamp(project_root: &Path, actor: ActorKind, at: &str) -> LessonStamp {
    let git = |args: &[&str]| -> Option<String> {
        Command::new("git")
            .arg("-C")
            .arg(project_root)
            .args(args)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let user = git(&["config", "user.name"]);
    let who = match actor {
        ActorKind::Person => user.unwrap_or_else(|| "a person".into()),
        ActorKind::Agent => match user {
            Some(user) => format!("an agent for {user}"),
            None => "an agent".into(),
        },
    };
    let session = [
        "CLAUDE_SESSION_ID",
        "CLAUDE_CODE_SESSION_ID",
        "CODEX_THREAD_ID",
        "CURSOR_SESSION_ID",
    ]
    .iter()
    .find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
    .map(|value| value.chars().take(120).collect());
    LessonStamp {
        who: who.chars().take(200).collect(),
        actor,
        session,
        task: in_progress_task(project_root),
        branch: git(&["rev-parse", "--abbrev-ref", "HEAD"]).filter(|branch| branch != "HEAD"),
        commit: git(&["rev-parse", "--verify", "--quiet", "HEAD"]),
        at: at.to_string(),
    }
}

/// The Beads issue in progress, when exactly one is.
fn in_progress_task(project_root: &Path) -> Option<String> {
    if !project_root.join(".beads").is_dir() {
        return None;
    }
    let output = Command::new("bd")
        .args(["list", "--status", "in_progress", "--json", "--limit", "3"])
        .current_dir(project_root)
        .env("BD_NON_INTERACTIVE", "1")
        .env_remove("BEADS_DIR")
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let items = serde_json::from_slice::<serde_json::Value>(&output.stdout).ok()?;
    match items.as_array()?.as_slice() {
        [only] => only["id"].as_str().map(str::to_owned),
        _ => None,
    }
}

/// sha256 of a file's bytes, or of a directory's tracked files (path and
/// content, in order); `None` when the path does not exist.
pub fn path_digest(project_root: &Path, path: &str) -> Option<ContentDigest> {
    let relative = path.trim_start_matches("./").trim_end_matches('/');
    let full = project_root.join(relative);
    let metadata = std::fs::symlink_metadata(&full).ok()?;
    if metadata.is_file() {
        let bytes = std::fs::read(&full).ok()?;
        return ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))).ok();
    }
    if !metadata.is_dir() {
        return None;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["ls-files", "-z", "--", relative])
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let mut hasher = Sha256::new();
    let mut files = String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|file| !file.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    files.sort();
    for file in files.iter().take(4_096) {
        hasher.update(file.as_bytes());
        hasher.update([0]);
        if let Ok(bytes) = std::fs::read(project_root.join(file)) {
            hasher.update(Sha256::digest(bytes));
        }
    }
    ContentDigest::new(format!("sha256:{:x}", hasher.finalize())).ok()
}

/// Anchors: the paths a lesson names plus the writer's additions, never
/// every file a session changed.
pub fn anchors(project_root: &Path, draft: &LessonDraft) -> Vec<Anchor> {
    let paths = draft
        .about
        .iter()
        .filter_map(|subject| match subject {
            Subject::Path { path } => Some(path.as_str()),
            _ => None,
        })
        .chain(draft.anchor_paths.iter().map(String::as_str))
        .map(|path| {
            path.trim_start_matches("./")
                .trim_end_matches('/')
                .to_string()
        })
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>();
    paths
        .into_iter()
        .take(MAX_LESSON_ITEMS)
        .map(|path| Anchor {
            digest: path_digest(project_root, &path),
            path,
        })
        .collect()
}

/// The cheap staleness check: each anchor's recorded hash against the file
/// now. A changed or vanished anchor means the lesson may be stale.
pub fn compare(project_root: &Path, lesson: &Lesson) -> Vec<AnchorComparison> {
    lesson
        .anchors
        .iter()
        .map(|anchor| {
            let current = path_digest(project_root, &anchor.path);
            AnchorComparison {
                path: anchor.path.clone(),
                changed: current != anchor.digest,
                recorded: anchor.digest.clone(),
                current,
            }
        })
        .collect()
}

/// A stored lesson's anchors now: what its store reported (Virgil hashes at
/// the writer's commit and says which anchors are stale), else Whetstone's
/// own hash comparison (the Beads backend).
pub fn anchor_state(project_root: &Path, stored: &StoredLesson) -> Vec<AnchorComparison> {
    stored
        .anchor_state
        .clone()
        .unwrap_or_else(|| compare(project_root, &stored.lesson))
}

pub fn may_be_stale(comparisons: &[AnchorComparison]) -> bool {
    comparisons.iter().any(|comparison| comparison.changed)
}

/// A group lesson may not be about something only one person has: a draft
/// or withdrawn record, a `personal:` or `local:` locator, or a path Git
/// ignores (it exists only on the writer's machine).
pub fn scope_check(
    state: Option<&AgreementState>,
    project_root: &Path,
    draft: &LessonDraft,
) -> Result<(), Refusal> {
    if !draft.audience.is_group() {
        return Ok(());
    }
    for subject in &draft.about {
        let narrower = match subject {
            Subject::Repo { .. } => None,
            Subject::Record { locator } => {
                let (system, id) = locator.split_once(':').unwrap_or(("", locator));
                if matches!(system, "personal" | "local" | "private") {
                    Some(format!("{locator} is a personal record"))
                } else if system == "whetstone" {
                    let lifecycle = RecordId::new(id).ok().and_then(|id| {
                        let state = state?;
                        if state.in_force(&id).is_some() {
                            return Some(Lifecycle::Accepted);
                        }
                        state.latest(&id).map(|record| state.lifecycle_of(record))
                    });
                    matches!(lifecycle, Some(Lifecycle::Draft | Lifecycle::Withdrawn))
                        .then(|| format!("{locator} is a private draft, not an accepted record"))
                } else {
                    None
                }
            }
            Subject::Path { path } => {
                let ignored = Command::new("git")
                    .arg("-C")
                    .arg(project_root)
                    .args(["check-ignore", "-q", "--", path.trim_start_matches("./")])
                    .status()
                    .is_ok_and(|status| status.success());
                ignored.then(|| format!("{path} is ignored by Git, so only this machine has it"))
            }
        };
        if let Some(reason) = narrower {
            return Err(Refusal::new(
                "scope_exceeds_authority",
                format!(
                    "This lesson is for the {} but is about something narrower: {reason}. Make it a personal lesson (audience person) or name a subject the whole audience has.",
                    draft.audience.label()
                ),
            ));
        }
    }
    Ok(())
}

/// A newly remembered lesson: proposed and inferred, whoever wrote it.
pub fn remembered(draft: &LessonDraft, stamp: LessonStamp, anchors: Vec<Anchor>) -> Lesson {
    Lesson {
        schema: LESSON_SCHEMA_V1.into(),
        key: draft.key.clone(),
        text: draft.text.clone(),
        about: draft.about.clone(),
        concepts: draft.concepts.clone(),
        audience: draft.audience.clone(),
        stamp,
        anchors,
        trust: Trust::Inferred,
        status: LessonStatus::Proposed,
        supersedes: draft.supersedes.clone(),
        superseded_by: None,
        forget_proposed: None,
        decision: None,
    }
}

/// The next revision of a lesson after a person's decision.
pub fn decided(current: &Lesson, decision: LessonDecision, stamp: LessonStamp) -> Lesson {
    let mut next = current.clone();
    next.stamp = stamp;
    match decision.verb {
        LessonVerb::Admit => {
            next.status = LessonStatus::Admitted;
            if next.trust == Trust::Inferred {
                next.trust = Trust::Validated;
            }
        }
        LessonVerb::Forget => {
            next.status = LessonStatus::Forgotten;
            next.forget_proposed = None;
        }
        LessonVerb::Supersede => next.status = LessonStatus::Superseded,
        LessonVerb::Remember => {}
    }
    next.decision = Some(decision);
    next
}

/// An agent's proposal to forget a lesson: recorded on the lesson, applied
/// only by a person.
pub fn with_forget_proposal(
    current: &Lesson,
    proposal: ForgetProposal,
    stamp: LessonStamp,
) -> Lesson {
    let mut next = current.clone();
    next.stamp = stamp;
    next.forget_proposed = Some(proposal);
    next
}

/// How many corrections a lesson's lineage has had: the supersessions behind
/// it, following `supersedes` keys back.
pub fn corrections(by_key: &BTreeMap<String, &Lesson>, key: &str) -> u32 {
    let mut count = 0;
    let mut seen = BTreeSet::new();
    let mut cursor = by_key.get(key).and_then(|lesson| lesson.supersedes.clone());
    while let Some(previous) = cursor {
        if !seen.insert(previous.clone()) {
            break;
        }
        count += 1;
        cursor = by_key
            .get(&previous)
            .and_then(|lesson| lesson.supersedes.clone());
    }
    count
}

/// A lesson corrected this many times goes back to a person for review.
pub const REVIEW_AFTER_CORRECTIONS: u32 = 2;

/// The lesson a rule was drafted from, when its source names one.
pub fn origin_lesson(rule: &crate::domain::Rule) -> Option<&str> {
    (rule.source.kind == crate::domain::RuleSourceKind::Lesson)
        .then_some(rule.source.reference.as_deref())
        .flatten()
        .map(|reference| reference.trim_start_matches("lesson."))
}

/// A stable reference to a lesson revision, for receipts and citations: the
/// local record's, or one computed from the lesson Virgil returned.
pub fn lesson_reference(stored: &StoredLesson) -> Option<RecordRef> {
    if let Some(reference) = &stored.reference {
        return Some(reference.clone());
    }
    let id = record_id(&stored.lesson.key).ok()?;
    let bytes = serde_json::to_vec(&stored.lesson).ok()?;
    Some(RecordRef {
        id,
        revision: stored.revision.max(1),
        digest: ContentDigest::new(format!("sha256:{:x}", Sha256::digest(bytes))).ok()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_come_from_the_first_words() {
        assert_eq!(
            key_from_text("Never call unwrap() in src/check — use ?"),
            "never-call-unwrap-in-src-check-use"
        );
        assert!(crate::domain::valid_key(&key_from_text("!!")));
    }

    #[test]
    fn a_missing_switch_is_standalone_and_a_broken_one_fails_closed() {
        let temp = tempfile::tempdir().expect("temp");
        assert_eq!(read_config(temp.path()), Ok(StoreConfig::Beads));
        std::fs::create_dir_all(temp.path().join("whetstone")).expect("dir");
        std::fs::write(
            temp.path().join(crate::storage::STORE_CONFIG),
            "{\"store\":\"recall\"}",
        )
        .expect("write");
        let refused = read_config(temp.path()).expect_err("unknown store");
        assert!(
            refused.contains("\"recall\" is not a record store"),
            "{refused}"
        );
        crate::storage::write_config(temp.path(), StoreConfig::Virgil).expect("write");
        assert_eq!(read_config(temp.path()), Ok(StoreConfig::Virgil));
    }

    #[test]
    fn corrections_follow_the_supersession_chain() {
        let lesson = |key: &str, supersedes: Option<&str>| Lesson {
            schema: LESSON_SCHEMA_V1.into(),
            key: key.into(),
            text: "text".into(),
            about: Vec::new(),
            concepts: Vec::new(),
            audience: Audience::Group {
                name: "team".into(),
            },
            stamp: LessonStamp {
                who: "someone".into(),
                actor: ActorKind::Agent,
                session: None,
                task: None,
                branch: None,
                commit: None,
                at: "2026-10-13T00:00:00Z".into(),
            },
            anchors: Vec::new(),
            trust: Trust::Inferred,
            status: LessonStatus::Proposed,
            supersedes: supersedes.map(str::to_owned),
            superseded_by: None,
            forget_proposed: None,
            decision: None,
        };
        let a = lesson("first", None);
        let b = lesson("second", Some("first"));
        let c = lesson("third", Some("second"));
        let by_key = [&a, &b, &c]
            .into_iter()
            .map(|lesson| (lesson.key.clone(), lesson))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(corrections(&by_key, "first"), 0);
        assert_eq!(corrections(&by_key, "second"), 1);
        assert_eq!(corrections(&by_key, "third"), REVIEW_AFTER_CORRECTIONS);
    }
}
