//! The rule contract and the identifiers the rulebook file is built from:
//! who acts, record ids, content digests, the mission and principles, and
//! rules ([`Rule`]).

use std::fmt;

use serde::{Deserialize, Serialize};

mod rule;

pub use rule::*;

/// Who is acting: a person, or anything not clearly one.
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RecordId(String);

impl RecordId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let valid = (3..=96).contains(&value.len())
            && value
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ".:_-".contains(character));
        if valid {
            Ok(Self(value))
        } else {
            Err(DomainError::InvalidRecordId(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for RecordId {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RecordId> for String {
    fn from(value: RecordId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentDigest(String);

impl ContentDigest {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        let suffix = value.strip_prefix("sha256:");
        if suffix.is_some_and(|hex| {
            hex.len() == 64 && hex.chars().all(|character| character.is_ascii_hexdigit())
        }) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(DomainError::InvalidDigest(value))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentDigest {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ContentDigest> for String {
    fn from(value: ContentDigest) -> Self {
        value.0
    }
}

/// Where a piece of evidence lives, and its digest when it was taken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef {
    pub system: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<ContentDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mission {
    pub statement: String,
}

/// Where a principle comes from: a pstack principle pinned to the pstack
/// version it was read from, or the owner's own wording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrincipleSource {
    Pstack { id: String, version: String },
    Custom,
}

/// A coding principle the owner picked; rules cite it as their source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principle {
    pub statement: String,
    pub source: PrincipleSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Principle {
    pub(crate) fn validate(&self) -> Result<(), DomainError> {
        require_text(&self.statement)?;
        if let Some(rationale) = &self.rationale {
            require_text(rationale)?;
        }
        match &self.source {
            PrincipleSource::Pstack { id, version } => {
                require_text(id)?;
                require_text(version)?;
                if !id
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || character == '-')
                {
                    return Err(DomainError::InvalidField(
                        "pstack principle ids are lowercase slugs",
                    ));
                }
                Ok(())
            }
            PrincipleSource::Custom => Ok(()),
        }
    }
}

/// Matches a path against a prefix (`src/`, `src/dashboard.rs`) or a simple
/// glob with `*` (one segment) and `**` (any depth).
pub fn entry_point_matches(pattern: &str, path: &str) -> bool {
    if !pattern.contains('*') {
        let pattern = pattern.trim_end_matches('/');
        return path == pattern
            || path
                .strip_prefix(pattern)
                .is_some_and(|rest| rest.starts_with('/'));
    }
    glob_match(
        &pattern.split('/').collect::<Vec<_>>(),
        &path.split('/').collect::<Vec<_>>(),
    )
}

fn glob_match(pattern: &[&str], path: &[&str]) -> bool {
    match (pattern.first(), path.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            glob_match(&pattern[1..], path) || (!path.is_empty() && glob_match(pattern, &path[1..]))
        }
        (Some(segment), Some(candidate)) => {
            segment_match(segment, candidate) && glob_match(&pattern[1..], &path[1..])
        }
        _ => false,
    }
}

fn segment_match(pattern: &str, candidate: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return pattern == candidate;
    }
    let mut rest = candidate;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            let Some(stripped) = rest.strip_prefix(part) else {
                return false;
            };
            rest = stripped;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else if let Some(position) = rest.find(part) {
            rest = &rest[position + part.len()..];
        } else {
            return false;
        }
    }
    true
}

pub(crate) fn require_text(value: &str) -> Result<(), DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::InvalidField("required text is empty"))
    } else {
        Ok(())
    }
}

pub(crate) fn safe_relative_path(value: &str) -> bool {
    let path = std::path::Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    InvalidRecordId(String),
    InvalidDigest(String),
    InvalidField(&'static str),
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for DomainError {}

#[cfg(test)]
mod tests;
