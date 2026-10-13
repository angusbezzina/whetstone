//! Lesson record v1 (`whetstone.lesson.v1`): the softest rung of the rule
//! ladder. A lesson is something learned about a repository, a record or a
//! path, in words, for a person or a group. It is proposed by anyone, admitted
//! only by a person, and anchored to the files it names so a cheap hash
//! comparison says when it may be stale. Rules keep their enforcers; lessons
//! never block anything.
//!
//! Who wrote a lesson, in which session, task, branch and commit, is stamped
//! by Whetstone (or by the Virgil daemon), never supplied by the writer.

use serde::{Deserialize, Serialize};

use super::{
    looks_like_utc_timestamp, require_text, safe_relative_path, ContentDigest, DomainError,
    RecordRef,
};

pub const LESSON_SCHEMA_V1: &str = "whetstone.lesson.v1";
pub const MAX_LESSON_TEXT: usize = 4_000;
pub const MAX_LESSON_ITEMS: usize = 32;

/// How far a lesson can be relied on. Names follow recall's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// Proposed by an agent or a person; nobody has admitted it.
    Inferred,
    /// Admitted by a person for the audience.
    Validated,
    /// A person's call that this is how things are.
    Authoritative,
}

impl Trust {
    pub fn label(self) -> &'static str {
        match self {
            Self::Inferred => "inferred",
            Self::Validated => "validated",
            Self::Authoritative => "authoritative",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonStatus {
    /// Waiting for a person to admit it.
    Proposed,
    /// Admitted: delivered to agents and people.
    Admitted,
    /// Replaced by a correction a person applied.
    Superseded,
    /// Taken out of delivery by a person; history stays.
    Forgotten,
}

impl LessonStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Admitted => "admitted",
            Self::Superseded => "superseded",
            Self::Forgotten => "forgotten",
        }
    }
}

/// What a lesson is about. Concepts are not subjects; they stay keys and
/// search text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Subject {
    /// A repository, by name (`whetstone`) or remote (`github.com/org/repo`).
    Repo { repo: String },
    /// A record by qualified locator: `<system>:<id>`, for example
    /// `whetstone:rule.no-unwrap` or `beads:whetstone-jpd`.
    Record { locator: String },
    /// A repository-relative code path.
    Path { path: String },
}

impl Subject {
    pub fn label(&self) -> String {
        match self {
            Self::Repo { repo } => format!("repo {repo}"),
            Self::Record { locator } => locator.clone(),
            Self::Path { path } => path.clone(),
        }
    }

    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Repo { repo } => bounded_line(repo, 300, "a repo subject is one short line"),
            Self::Record { locator } => {
                bounded_line(locator, 300, "a record locator is one short line")?;
                match locator.split_once(':') {
                    Some((system, id)) if !system.trim().is_empty() && !id.trim().is_empty() => {
                        Ok(())
                    }
                    _ => Err(DomainError::InvalidField(
                        "a record subject is a qualified locator: <system>:<id>",
                    )),
                }
            }
            Self::Path { path } => {
                if safe_relative_path(path.trim_start_matches("./")) && !path.contains('\\') {
                    Ok(())
                } else {
                    Err(DomainError::InvalidField(
                        "a path subject is repository-relative",
                    ))
                }
            }
        }
    }
}

/// Who a lesson is for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Audience {
    /// One person (a personal lesson).
    Person { name: String },
    /// A group: the repository's team standalone, a Virgil group inside Virgil.
    Group { name: String },
}

impl Audience {
    pub fn label(&self) -> String {
        match self {
            Self::Person { name } => format!("person {name}"),
            Self::Group { name } => format!("group {name}"),
        }
    }

    pub fn is_group(&self) -> bool {
        matches!(self, Self::Group { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Person,
    /// Anything not clearly a person.
    #[default]
    Agent,
}

impl ActorKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
        }
    }
}

/// Provenance the store stamps on every revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonStamp {
    pub who: String,
    pub actor: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub at: String,
}

/// A file a lesson depends on and its content hash when the lesson was written.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub path: String,
    /// sha256 of the file's bytes; absent when the file did not exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LessonVerb {
    Remember,
    Admit,
    Supersede,
    Forget,
}

impl LessonVerb {
    pub fn label(self) -> &'static str {
        match self {
            Self::Remember => "remember",
            Self::Admit => "admit",
            Self::Supersede => "supersede",
            Self::Forget => "forget",
        }
    }
}

/// The person's decision that moved a lesson out of `proposed`, or applied a
/// correction or a forget. Inside Virgil, `receipt` is the daemon's receipt
/// for `virgil lessons admit` (or supersede, forget).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonDecision {
    pub verb: LessonVerb,
    pub by: String,
    pub at: String,
    pub rationale: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<String>,
}

/// An agent's (or a person's) proposal to forget an admitted lesson; a
/// person applies it with `wh change --forget`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForgetProposal {
    pub reason: String,
    pub by: String,
    pub at: String,
}

/// Lesson record v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lesson {
    pub schema: String,
    /// A stable key: lowercase letters, digits and hyphens.
    pub key: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub about: Vec<Subject>,
    /// Concepts, as keys and search text only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub concepts: Vec<String>,
    pub audience: Audience,
    pub stamp: LessonStamp,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    pub trust: Trust,
    pub status: LessonStatus,
    /// The key of the lesson this one corrects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    /// The key of the lesson that corrected this one (status superseded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forget_proposed: Option<ForgetProposal>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<LessonDecision>,
}

impl Lesson {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema != LESSON_SCHEMA_V1 {
            return Err(DomainError::InvalidField(
                "lesson schema must be whetstone.lesson.v1",
            ));
        }
        if !valid_key(&self.key) {
            return Err(DomainError::InvalidField(
                "a lesson key is 3 to 80 lowercase letters, digits and hyphens",
            ));
        }
        require_text(&self.text)?;
        if self.text.len() > MAX_LESSON_TEXT {
            return Err(DomainError::InvalidField("a lesson is at most 4000 bytes"));
        }
        if self.about.len() > MAX_LESSON_ITEMS
            || self.concepts.len() > MAX_LESSON_ITEMS
            || self.anchors.len() > MAX_LESSON_ITEMS
        {
            return Err(DomainError::InvalidField(
                "a lesson lists at most 32 subjects, concepts and anchors",
            ));
        }
        for subject in &self.about {
            subject.validate()?;
        }
        for concept in &self.concepts {
            bounded_line(concept, 120, "a concept is one short line")?;
        }
        match &self.audience {
            Audience::Person { name } | Audience::Group { name } => {
                bounded_line(name, 200, "an audience is one short line")?
            }
        }
        require_text(&self.stamp.who)?;
        if !looks_like_utc_timestamp(&self.stamp.at) {
            return Err(DomainError::InvalidTimestamp(self.stamp.at.clone()));
        }
        for anchor in &self.anchors {
            if !safe_relative_path(&anchor.path) || anchor.path.contains('\\') {
                return Err(DomainError::InvalidField(
                    "an anchor is a repository-relative path",
                ));
            }
        }
        for key in [&self.supersedes, &self.superseded_by]
            .into_iter()
            .flatten()
        {
            if !valid_key(key) || key == &self.key {
                return Err(DomainError::InvalidField(
                    "a supersession names another lesson's key",
                ));
            }
        }
        // Only a person's decision moves a lesson past proposed/inferred.
        match (self.status, self.trust, &self.decision) {
            (LessonStatus::Proposed, Trust::Inferred, None) => {}
            (LessonStatus::Proposed, _, _) => {
                return Err(DomainError::InvalidField(
                    "a proposed lesson is inferred and carries no decision",
                ))
            }
            (_, _, None) => {
                return Err(DomainError::InvalidField(
                    "only a person's recorded decision moves a lesson past proposed",
                ))
            }
            (LessonStatus::Admitted, Trust::Inferred, _) => {
                return Err(DomainError::InvalidField(
                    "an admitted lesson is validated or authoritative",
                ))
            }
            _ => {}
        }
        if self.status == LessonStatus::Superseded && self.superseded_by.is_none() {
            return Err(DomainError::InvalidField(
                "a superseded lesson names the lesson that corrected it",
            ));
        }
        if let Some(decision) = &self.decision {
            require_text(&decision.by)?;
            require_text(&decision.rationale)?;
            if !looks_like_utc_timestamp(&decision.at) {
                return Err(DomainError::InvalidTimestamp(decision.at.clone()));
            }
            if decision.verb == LessonVerb::Remember {
                return Err(DomainError::InvalidField(
                    "remembering is a proposal, not a decision",
                ));
            }
        }
        if let Some(proposal) = &self.forget_proposed {
            require_text(&proposal.reason)?;
            require_text(&proposal.by)?;
            if !looks_like_utc_timestamp(&proposal.at) {
                return Err(DomainError::InvalidTimestamp(proposal.at.clone()));
            }
        }
        Ok(())
    }

    /// Delivered to agents and people: admitted and not taken out.
    pub fn is_live(&self) -> bool {
        self.status == LessonStatus::Admitted
    }

    /// Repository paths this lesson names as subjects.
    pub fn subject_paths(&self) -> impl Iterator<Item = &str> {
        self.about.iter().filter_map(|subject| match subject {
            Subject::Path { path } => Some(path.trim_start_matches("./")),
            _ => None,
        })
    }
}

/// One anchor's hash comparison at a read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorComparison {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded: Option<ContentDigest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<ContentDigest>,
    pub changed: bool,
}

/// The cheap staleness check of one lesson revision: every anchor's recorded
/// hash against the file now. No model is called.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LessonCheck {
    /// The lesson revision checked (a local record, or the Virgil lesson's
    /// key rendered as a record reference by the store adapter).
    pub lesson: RecordRef,
    pub key: String,
    pub comparisons: Vec<AnchorComparison>,
    pub may_be_stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub checked_at: String,
}

impl LessonCheck {
    pub(super) fn validate(&self) -> Result<(), DomainError> {
        if !valid_key(&self.key) {
            return Err(DomainError::InvalidField(
                "a lesson check names a lesson key",
            ));
        }
        if self.comparisons.is_empty() || self.comparisons.len() > MAX_LESSON_ITEMS {
            return Err(DomainError::InvalidField(
                "a lesson check compares one to 32 anchors",
            ));
        }
        let stale = self.comparisons.iter().any(|comparison| comparison.changed);
        if stale != self.may_be_stale {
            return Err(DomainError::InvalidField(
                "a lesson may be stale exactly when an anchor changed",
            ));
        }
        for comparison in &self.comparisons {
            let changed = comparison.recorded != comparison.current;
            if changed != comparison.changed || !safe_relative_path(&comparison.path) {
                return Err(DomainError::InvalidField(
                    "an anchor comparison is changed exactly when the hashes differ",
                ));
            }
        }
        if !looks_like_utc_timestamp(&self.checked_at) {
            return Err(DomainError::InvalidTimestamp(self.checked_at.clone()));
        }
        Ok(())
    }
}

/// Lesson keys: 3 to 80 lowercase letters, digits and hyphens, starting with
/// a letter or digit.
pub fn valid_key(key: &str) -> bool {
    (3..=80).contains(&key.len())
        && key.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !key.starts_with('-')
        && !key.ends_with('-')
}

fn bounded_line(value: &str, maximum: usize, message: &'static str) -> Result<(), DomainError> {
    if value.trim().is_empty() || value.len() > maximum || value.contains('\n') {
        Err(DomainError::InvalidField(message))
    } else {
        Ok(())
    }
}
