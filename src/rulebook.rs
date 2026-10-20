//! The rulebook file (`whetstone/rulebook.json`, `whetstone.rulebook.v1`):
//! the mission, the chosen principles, the rules and the retired entries,
//! kept in the repository.
//!
//! A revision is the commit that last touched the file, and a rule's digest
//! is the sha256 of its canonical JSON. The reader is strict: an unknown
//! field, a missing one or an earlier shape is an error that names the field.
//! The writer keeps one layout (entries sorted by id, two-space indents,
//! arrays of plain values on one line), so an edit is a small diff.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::domain::{
    ContentDigest, DomainError, Enforcer, Mission, Principle, PrincipleSource, RecordId, Rule,
    RuleExample, RuleSource, RuleSourceKind, Strength, SweepClass, SweepUnsure, RULE_SCHEMA_V2,
};

pub const RULEBOOK_SCHEMA: &str = "whetstone.rulebook.v1";
/// Where the rulebook lives, relative to the project root.
pub const RULEBOOK_PATH: &str = "whetstone/rulebook.json";

/// The rulebook as read, with each rule's digest and the file's revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rulebook {
    pub mission: Mission,
    pub principles: Vec<PrincipleEntry>,
    pub rules: Vec<RuleEntry>,
    /// Rules and principles taken out of the rulebook, with the reason.
    pub retired: Vec<RetiredEntry>,
    /// Strict mode: `wh change --accept` refuses a rulebook in which a
    /// principle has no rule or a rule serves no principle.
    pub require_served: bool,
    pub revision: Revision,
}

/// Where the chain from principle to rule breaks: held principles no rule
/// serves, and rules that serve no held principle. Derived, never stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    pub unserved: Vec<RecordId>,
    pub unanchored: Vec<RecordId>,
}

impl Coverage {
    pub fn is_complete(&self) -> bool {
        self.unserved.is_empty() && self.unanchored.is_empty()
    }

    /// One sentence for the health strip and the check summary, or None
    /// when the chain holds.
    pub fn sentence(&self) -> Option<String> {
        let count =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let parts = [
            (!self.unserved.is_empty()).then(|| {
                count(
                    self.unserved.len(),
                    "principle has no rule",
                    "principles have no rule",
                )
            }),
            (!self.unanchored.is_empty()).then(|| {
                count(
                    self.unanchored.len(),
                    "rule serves no principle",
                    "rules serve no principle",
                )
            }),
        ];
        let parts = parts.into_iter().flatten().collect::<Vec<_>>();
        (!parts.is_empty()).then(|| parts.join("; "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipleEntry {
    pub id: RecordId,
    pub principle: Principle,
}

impl Rulebook {
    /// The principle a rule serves: the one its source names, as the record
    /// id (`principle.prove-it-works`), the bare id or a pack's
    /// `pstack:prove-it-works`. None when the source names nothing held here.
    pub fn principle_for(&self, rule: &Rule) -> Option<&PrincipleEntry> {
        if rule.source.kind != RuleSourceKind::Principle {
            return None;
        }
        let reference = rule.source.reference.as_deref()?;
        self.principle_named(reference)
    }

    /// The rules that serve a principle, in file order.
    pub fn served_by(&self, principle: &RecordId) -> Vec<&RuleEntry> {
        self.rules
            .iter()
            .filter(|entry| {
                self.principle_for(&entry.rule)
                    .is_some_and(|served| &served.id == principle)
            })
            .collect()
    }

    /// The breaks in the chain from principle to rule, ids in file order.
    /// Retired entries are not in the rulebook and never count; a rule
    /// that names a retired principle serves nothing.
    pub fn coverage(&self) -> Coverage {
        Coverage {
            unserved: self
                .principles
                .iter()
                .filter(|principle| self.served_by(&principle.id).is_empty())
                .map(|principle| principle.id.clone())
                .collect(),
            unanchored: self
                .rules
                .iter()
                .filter(|entry| self.principle_for(&entry.rule).is_none())
                .map(|entry| entry.id.clone())
                .collect(),
        }
    }

    /// The principle a reference names, by its record id, its bare id or the
    /// id it has in its pack.
    pub fn principle_named(&self, reference: &str) -> Option<&PrincipleEntry> {
        let tail = reference.rsplit(':').next().unwrap_or(reference);
        self.principles.iter().find(|entry| {
            let pack_id = match &entry.principle.source {
                PrincipleSource::Pstack { id, .. } | PrincipleSource::Pack { id, .. } => {
                    Some(id.as_str())
                }
                PrincipleSource::Custom => None,
            };
            let id = entry.id.as_str();
            id == reference || id == format!("principle.{tail}") || pack_id == Some(tail)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleEntry {
    pub id: RecordId,
    pub rule: Rule,
    /// sha256 of the rule's canonical JSON, its id included.
    pub digest: ContentDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetiredEntry {
    pub id: RecordId,
    pub reason: String,
}

/// The commit that last touched the rulebook, or `uncommitted` when the
/// working tree or the index differs from it (or it was never committed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revision {
    Commit(String),
    Uncommitted,
}

impl Revision {
    pub fn label(&self) -> &str {
        match self {
            Self::Commit(commit) => commit,
            Self::Uncommitted => "uncommitted",
        }
    }
}

impl Serialize for Revision {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.label())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulebookError {
    /// The project has no rulebook file.
    Missing,
    Unreadable(String),
    Unwritable(String),
    /// The file is not a rulebook; the message names the field.
    Invalid(String),
}

impl fmt::Display for RulebookError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(formatter, "{RULEBOOK_PATH} does not exist"),
            Self::Unreadable(error) => write!(formatter, "{RULEBOOK_PATH} cannot be read: {error}"),
            Self::Unwritable(error) => {
                write!(formatter, "{RULEBOOK_PATH} cannot be written: {error}")
            }
            Self::Invalid(error) => write!(formatter, "{RULEBOOK_PATH} is not a rulebook: {error}"),
        }
    }
}

impl std::error::Error for RulebookError {}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RulebookFile {
    schema: String,
    mission: MissionFile,
    principles: Vec<PrincipleFile>,
    rules: Vec<RuleFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    retired: Vec<RetiredEntry>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    require_served: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MissionFile {
    statement: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PrincipleFile {
    id: RecordId,
    statement: String,
    source: PrincipleOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rationale: Option<String>,
}

/// A principle's source in the file: a pstack principle from the pack
/// pinned in the binary, a principle from another pack (named with its
/// version), or the owner's own wording.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum PrincipleOrigin {
    Pstack {
        id: String,
        version: String,
    },
    Pack {
        pack: String,
        id: String,
        version: String,
    },
    Custom,
}

/// A rule in the file: the domain rule's fields under its id.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    id: RecordId,
    statement: String,
    rationale: String,
    strength: Strength,
    enforcer: Enforcer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    examples: Vec<RuleExample>,
    source: RuleSource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sweep: Option<SweepClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    sweep_priority: Option<u8>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sweep_unsure: Vec<SweepUnsure>,
}

impl RuleFile {
    fn into_rule(self) -> Rule {
        Rule {
            schema: RULE_SCHEMA_V2.into(),
            statement: self.statement,
            rationale: self.rationale,
            strength: self.strength,
            enforcer: self.enforcer,
            examples: self.examples,
            source: self.source,
            paths: self.paths,
            sweep: self.sweep,
            sweep_priority: self.sweep_priority,
            sweep_unsure: self.sweep_unsure,
        }
    }

    fn from_rule(id: &RecordId, rule: &Rule) -> Self {
        Self {
            id: id.clone(),
            statement: rule.statement.clone(),
            rationale: rule.rationale.clone(),
            strength: rule.strength,
            enforcer: rule.enforcer.clone(),
            examples: rule.examples.clone(),
            source: rule.source.clone(),
            paths: rule.paths.clone(),
            sweep: rule.sweep,
            sweep_priority: rule.sweep_priority,
            sweep_unsure: rule.sweep_unsure.clone(),
        }
    }
}

impl PrincipleFile {
    fn from_entry(entry: &PrincipleEntry) -> Self {
        let source = match &entry.principle.source {
            PrincipleSource::Pstack { id, version } => PrincipleOrigin::Pstack {
                id: id.clone(),
                version: version.clone(),
            },
            PrincipleSource::Pack { pack, id, version } => PrincipleOrigin::Pack {
                pack: pack.clone(),
                id: id.clone(),
                version: version.clone(),
            },
            PrincipleSource::Custom => PrincipleOrigin::Custom,
        };
        Self {
            id: entry.id.clone(),
            statement: entry.principle.statement.clone(),
            source,
            rationale: entry.principle.rationale.clone(),
        }
    }
}

impl RuleEntry {
    /// A rule under its id, with its digest.
    pub fn new(id: RecordId, rule: Rule) -> Result<Self, RulebookError> {
        let digest = digest(&RuleFile::from_rule(&id, &rule))?;
        Ok(Self { id, rule, digest })
    }
}

/// A rule as the file holds it.
pub fn rule_value(entry: &RuleEntry) -> Value {
    serde_json::to_value(RuleFile::from_rule(&entry.id, &entry.rule))
        .expect("a rule always encodes")
}

/// A principle as the file holds it.
pub fn principle_value(entry: &PrincipleEntry) -> Value {
    serde_json::to_value(PrincipleFile::from_entry(entry)).expect("a principle always encodes")
}

/// Read the project's rulebook and the commit that last touched it.
pub fn load(project_root: &Path) -> Result<Rulebook, RulebookError> {
    let text = match std::fs::read_to_string(project_root.join(RULEBOOK_PATH)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(RulebookError::Missing)
        }
        Err(error) => return Err(RulebookError::Unreadable(error.to_string())),
    };
    parse(&text, revision(project_root))
}

/// Read rulebook text strictly.
pub fn parse(text: &str, revision: Revision) -> Result<Rulebook, RulebookError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| RulebookError::Invalid(format!("not JSON: {error}")))?;
    check_schema(&value)?;
    let file: RulebookFile = serde_path_to_error::deserialize(
        &mut serde_json::Deserializer::from_str(text),
    )
    .map_err(|error| RulebookError::Invalid(format!("{}: {}", error.path(), error.inner())))?;
    let invalid = |field: String, error: DomainError| {
        RulebookError::Invalid(format!("{field}: {}", domain_message(&error)))
    };
    let mission = Mission {
        statement: file.mission.statement,
    };
    if mission.statement.trim().is_empty() {
        return Err(RulebookError::Invalid(
            "mission.statement: required text is empty".into(),
        ));
    }
    let mut ids = BTreeSet::new();
    let mut unique = |field: String, id: &RecordId| {
        if ids.insert(id.clone()) {
            Ok(())
        } else {
            Err(RulebookError::Invalid(format!(
                "{field}: {} appears more than once",
                id.as_str()
            )))
        }
    };
    let pstack = crate::catalogue::pinned_pack();
    let mut principles = Vec::with_capacity(file.principles.len());
    for (index, entry) in file.principles.into_iter().enumerate() {
        let field = format!("principles[{index}]");
        unique(format!("{field}.id"), &entry.id)?;
        let source = match entry.source {
            PrincipleOrigin::Pstack { id, version } => {
                if !pstack.principles.iter().any(|principle| principle.id == id) {
                    return Err(RulebookError::Invalid(format!(
                        "{field}.source.id: {id} is not a principle in the pinned pstack pack (version {})",
                        pstack.version
                    )));
                }
                PrincipleSource::Pstack { id, version }
            }
            // Another pack's principle keeps the pack's name and version;
            // the pack itself need not still be in the repository.
            PrincipleOrigin::Pack { pack, id, version } => {
                PrincipleSource::Pack { pack, id, version }
            }
            PrincipleOrigin::Custom => PrincipleSource::Custom,
        };
        let principle = Principle {
            statement: entry.statement,
            source,
            rationale: entry.rationale,
        };
        principle
            .validate()
            .map_err(|error| invalid(format!("{field} ({})", entry.id.as_str()), error))?;
        principles.push(PrincipleEntry {
            id: entry.id,
            principle,
        });
    }
    let mut rules = Vec::with_capacity(file.rules.len());
    for (index, entry) in file.rules.into_iter().enumerate() {
        let field = format!("rules[{index}]");
        unique(format!("{field}.id"), &entry.id)?;
        let digest = digest(&entry)?;
        let id = entry.id.clone();
        let rule = entry.into_rule();
        rule.validate()
            .map_err(|error| invalid(format!("{field} ({})", id.as_str()), error))?;
        rules.push(RuleEntry { id, rule, digest });
    }
    for (index, entry) in file.retired.iter().enumerate() {
        let field = format!("retired[{index}]");
        unique(format!("{field}.id"), &entry.id)?;
        if entry.reason.trim().is_empty() {
            return Err(RulebookError::Invalid(format!(
                "{field}.reason: required text is empty"
            )));
        }
    }
    Ok(Rulebook {
        mission,
        principles,
        rules,
        retired: file.retired,
        require_served: file.require_served,
        revision,
    })
}

/// The rulebook's text: principles, rules and retired entries sorted by id,
/// in the layout [`write`] keeps.
pub fn render(book: &Rulebook) -> Result<String, RulebookError> {
    let mut text = String::new();
    layout(&to_json(book)?, 0, &mut text);
    text.push('\n');
    Ok(text)
}

/// One entry as the file holds it: a principle or rule by id, or the
/// mission under `mission`.
pub fn entry(book: &Rulebook, id: &str) -> Option<Value> {
    if id == "mission" {
        return to_json(book).ok().map(|value| value["mission"].clone());
    }
    match book.rules.iter().find(|entry| entry.id.as_str() == id) {
        Some(rule) => Some(rule_value(rule)),
        None => book
            .principles
            .iter()
            .find(|entry| entry.id.as_str() == id)
            .map(principle_value),
    }
}

fn to_json(book: &Rulebook) -> Result<Value, RulebookError> {
    let mut principles = book
        .principles
        .iter()
        .map(PrincipleFile::from_entry)
        .collect::<Vec<_>>();
    principles.sort_by(|left, right| left.id.cmp(&right.id));
    let mut rules = book
        .rules
        .iter()
        .map(|entry| RuleFile::from_rule(&entry.id, &entry.rule))
        .collect::<Vec<_>>();
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    let mut retired = book.retired.clone();
    retired.sort_by(|left, right| left.id.cmp(&right.id));
    serde_json::to_value(RulebookFile {
        schema: RULEBOOK_SCHEMA.into(),
        mission: MissionFile {
            statement: book.mission.statement.clone(),
        },
        principles,
        rules,
        retired,
        require_served: book.require_served,
    })
    .map_err(|error| RulebookError::Invalid(format!("the rulebook cannot be encoded: {error}")))
}

/// Pretty JSON with two-space indents, except that an array of plain values
/// (strings, numbers, booleans) stays on one line.
fn layout(value: &Value, depth: usize, out: &mut String) {
    let indent = |depth: usize| "  ".repeat(depth);
    match value {
        Value::Object(object) if !object.is_empty() => {
            out.push_str("{\n");
            for (index, (key, item)) in object.iter().enumerate() {
                out.push_str(&indent(depth + 1));
                out.push_str(&Value::String(key.clone()).to_string());
                out.push_str(": ");
                layout(item, depth + 1, out);
                if index + 1 < object.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&indent(depth));
            out.push('}');
        }
        Value::Array(items)
            if items
                .iter()
                .any(|item| matches!(item, Value::Object(_) | Value::Array(_))) =>
        {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                out.push_str(&indent(depth + 1));
                layout(item, depth + 1, out);
                if index + 1 < items.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str(&indent(depth));
            out.push(']');
        }
        Value::Array(items) => {
            out.push('[');
            out.push_str(
                &items
                    .iter()
                    .map(Value::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// Write the rulebook into the working tree and read it back. Nothing is
/// written unless the strict reader accepts the text first.
pub fn write(project_root: &Path, book: &Rulebook) -> Result<Rulebook, RulebookError> {
    let text = render(book)?;
    parse(&text, Revision::Uncommitted)?;
    let path = project_root.join(RULEBOOK_PATH);
    let unwritable = |error: std::io::Error| RulebookError::Unwritable(error.to_string());
    let directory = path.parent().expect("the rulebook path has a directory");
    std::fs::create_dir_all(directory).map_err(unwritable)?;
    let staging = directory.join(".rulebook.json.tmp");
    std::fs::write(&staging, &text).map_err(unwritable)?;
    std::fs::rename(&staging, &path).map_err(unwritable)?;
    parse(&text, revision(project_root))
}

/// An earlier shape (a stored record, a bare rule, a later version) is named
/// before the fields are read.
fn check_schema(value: &Value) -> Result<(), RulebookError> {
    let Some(object) = value.as_object() else {
        return Err(RulebookError::Invalid(
            "the file is not a JSON object".into(),
        ));
    };
    match object.get("schema").and_then(Value::as_str) {
        Some(RULEBOOK_SCHEMA) => Ok(()),
        Some(other) => Err(RulebookError::Invalid(format!(
            "schema: {other} is not {RULEBOOK_SCHEMA}"
        ))),
        None if object.contains_key("record_type") || object.contains_key("schema_version") => {
            Err(RulebookError::Invalid(format!(
                "schema: missing; this is a stored agreement record, not a rulebook ({RULEBOOK_SCHEMA})"
            )))
        }
        None => Err(RulebookError::Invalid(format!(
            "schema: missing; a rulebook names {RULEBOOK_SCHEMA}"
        ))),
    }
}

fn domain_message(error: &DomainError) -> String {
    match error {
        DomainError::InvalidField(message) => (*message).to_string(),
        other => other.to_string(),
    }
}

fn digest(rule: &RuleFile) -> Result<ContentDigest, RulebookError> {
    let value = serde_json::to_value(rule)
        .map_err(|error| RulebookError::Invalid(format!("a rule cannot be encoded: {error}")))?;
    ContentDigest::new(format!(
        "sha256:{:x}",
        Sha256::digest(canonical_json(&value))
    ))
    .map_err(|error| RulebookError::Invalid(domain_message(&error)))
}

/// Canonical JSON: object keys sorted by their UTF-8 bytes, no insignificant
/// whitespace, strings and numbers as serde_json writes them. Fields at their
/// default are left out before this, so `"paths": []` and no paths are equal.
pub fn canonical_json(value: &Value) -> Vec<u8> {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(object) => {
                let mut keys = object.keys().collect::<Vec<_>>();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), sorted(&object[key])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_vec(&sorted(value)).expect("a JSON value always encodes")
}

/// The commit that last touched the rulebook, unless the working tree or the
/// index differs from it.
pub fn revision(project_root: &Path) -> Revision {
    let git = |args: &[&str]| {
        Command::new("git")
            .arg("-C")
            .arg(project_root)
            .args(args)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    };
    let changed = git(&[
        "status",
        "--porcelain",
        "--untracked-files=all",
        "--",
        RULEBOOK_PATH,
    ]);
    if changed.as_deref() != Some("") {
        return Revision::Uncommitted;
    }
    match git(&["log", "-1", "--format=%H", "--", RULEBOOK_PATH]) {
        Some(commit) if !commit.is_empty() => Revision::Commit(commit),
        _ => Revision::Uncommitted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rulebook() -> Value {
        json!({
            "schema": RULEBOOK_SCHEMA,
            "mission": {"statement": "Ship code people can trust."},
            "principles": [{
                "id": "principle.prove-it-works",
                "statement": "Prove it works.",
                "source": {"kind": "pstack", "id": "prove-it-works", "version": "0.15.5"}
            }],
            "rules": [{
                "id": "rule.no-dbg",
                "statement": "No dbg! is left in the source.",
                "rationale": "Debug output is unfinished work.",
                "strength": "must",
                "enforcer": {
                    "kind": "ast",
                    "query": "((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))",
                    "language": "rust"
                },
                "source": {"kind": "owner"},
                "paths": ["src/**"],
                "sweep_priority": 1
            }]
        })
    }

    fn read(value: &Value) -> Result<Rulebook, RulebookError> {
        parse(&value.to_string(), Revision::Uncommitted)
    }

    fn error(value: &Value) -> String {
        match read(value) {
            Err(RulebookError::Invalid(message)) => message,
            other => panic!("expected an invalid rulebook, got {other:?}"),
        }
    }

    #[test]
    fn a_rulebook_reads_into_the_domain_types() {
        let book = read(&rulebook()).expect("valid");
        assert_eq!(book.mission.statement, "Ship code people can trust.");
        assert_eq!(book.principles[0].id.as_str(), "principle.prove-it-works");
        let rule = &book.rules[0];
        assert_eq!(rule.id.as_str(), "rule.no-dbg");
        assert_eq!(rule.rule.schema, RULE_SCHEMA_V2);
        assert_eq!(rule.rule.strength, Strength::Must);
        assert_eq!(rule.rule.sweep_priority(), 1);
        assert!(rule.rule.applies_to("src/lib.rs"));
        assert!(rule.digest.as_str().starts_with("sha256:"));
        assert_eq!(book.revision.label(), "uncommitted");
    }

    #[test]
    fn digests_ignore_key_order_whitespace_and_defaults_but_not_content() {
        let first = read(&rulebook()).expect("valid").rules[0].digest.clone();
        let mut reordered = serde_json::Map::new();
        let original = rulebook()["rules"][0].clone();
        let mut keys = original
            .as_object()
            .expect("rule")
            .keys()
            .collect::<Vec<_>>();
        keys.reverse();
        for key in keys {
            reordered.insert(key.clone(), original[key].clone());
        }
        reordered.insert("examples".into(), json!([]));
        let mut value = rulebook();
        value["rules"][0] = Value::Object(reordered);
        let pretty = serde_json::to_string_pretty(&value).expect("pretty");
        let second = parse(&pretty, Revision::Uncommitted).expect("valid").rules[0]
            .digest
            .clone();
        assert_eq!(first, second);
        let mut changed = rulebook();
        changed["rules"][0]["strength"] = json!("should");
        assert_ne!(first, read(&changed).expect("valid").rules[0].digest);
        let mut renamed = rulebook();
        renamed["rules"][0]["id"] = json!("rule.no-dbg-macro");
        assert_ne!(first, read(&renamed).expect("valid").rules[0].digest);
    }

    fn ids(ids: &[RecordId]) -> Vec<&str> {
        ids.iter().map(RecordId::as_str).collect()
    }

    #[test]
    fn coverage_is_complete_when_every_principle_has_a_rule_and_every_rule_a_principle() {
        let mut value = rulebook();
        value["rules"][0]["source"] =
            json!({"kind": "principle", "reference": "principle.prove-it-works"});
        let book = read(&value).expect("valid");
        let coverage = book.coverage();
        assert!(coverage.is_complete(), "{coverage:?}");
        assert_eq!(coverage.sentence(), None);
        assert_eq!(book.served_by(&book.principles[0].id).len(), 1);
    }

    #[test]
    fn coverage_names_an_unserved_principle_and_an_unanchored_rule() {
        let book = read(&rulebook()).expect("valid");
        let coverage = book.coverage();
        assert_eq!(ids(&coverage.unserved), ["principle.prove-it-works"]);
        assert_eq!(ids(&coverage.unanchored), ["rule.no-dbg"]);
        assert_eq!(
            coverage.sentence().as_deref(),
            Some("1 principle has no rule; 1 rule serves no principle")
        );
    }

    #[test]
    fn a_rule_naming_a_retired_principle_serves_nothing() {
        let mut value = rulebook();
        value["principles"] = json!([]);
        value["retired"] =
            json!([{"id": "principle.prove-it-works", "reason": "Folded into the mission."}]);
        value["rules"][0]["source"] =
            json!({"kind": "principle", "reference": "principle.prove-it-works"});
        let coverage = read(&value).expect("valid").coverage();
        assert!(coverage.unserved.is_empty());
        assert_eq!(ids(&coverage.unanchored), ["rule.no-dbg"]);
    }

    #[test]
    fn require_served_round_trips_and_is_left_out_when_false() {
        let book = read(&rulebook()).expect("valid");
        assert!(!book.require_served);
        assert!(!render(&book).expect("render").contains("require_served"));
        let mut value = rulebook();
        value["require_served"] = json!(true);
        let strict = read(&value).expect("valid");
        assert!(strict.require_served);
        assert!(render(&strict)
            .expect("render")
            .contains("\"require_served\": true"));
    }

    #[test]
    fn canonical_json_sorts_keys_at_every_depth() {
        let value = json!({"b": 1, "a": {"d": [{"f": 1, "e": 2}], "c": "x"}});
        assert_eq!(
            String::from_utf8(canonical_json(&value)).expect("utf-8"),
            r#"{"a":{"c":"x","d":[{"e":2,"f":1}]},"b":1}"#
        );
    }

    #[test]
    fn an_unknown_field_is_named_with_its_path() {
        let mut value = rulebook();
        value["rules"][0]["shadow"] = json!(true);
        let message = error(&value);
        assert!(
            message.starts_with("rules[0].shadow: unknown field `shadow`"),
            "{message}"
        );
        let mut value = rulebook();
        value["owner"] = json!("someone");
        assert!(error(&value).contains("unknown field `owner`"));
    }

    #[test]
    fn earlier_shapes_are_errors_that_name_the_field() {
        let mut value = rulebook();
        value["rules"][0]["schema"] = json!(RULE_SCHEMA_V2);
        assert!(error(&value).starts_with("rules[0].schema: unknown field `schema`"));
        let mut value = rulebook();
        value["rules"][0]["strength"] = json!("advisory");
        assert!(error(&value).starts_with("rules[0].strength: unknown variant `advisory`"));
        let mut value = rulebook();
        value["principles"][0]["source"] = json!({"kind": "migrated", "record": {}});
        let message = error(&value);
        assert!(
            message.starts_with("principles[0].source.kind: unknown variant `migrated`"),
            "{message}"
        );
        let mut value = rulebook();
        value["rules"][0]["enforcer"] = json!({"kind": "review", "question": "Is it clear?"});
        let message = error(&value);
        assert!(message.starts_with("rules[0].enforcer"), "{message}");
        assert!(message.contains("`review`"), "{message}");
        let mut value = rulebook();
        value["mission"]["desired_outcomes"] = json!([]);
        assert!(
            error(&value).starts_with("mission.desired_outcomes: unknown field `desired_outcomes`")
        );
        let stored = json!({"schema_version": 1, "record_type": "rule", "record": {}});
        assert!(error(&stored).contains("stored agreement record"));
        let mut value = rulebook();
        value["schema"] = json!("whetstone.rulebook.v0");
        assert_eq!(
            error(&value),
            "schema: whetstone.rulebook.v0 is not whetstone.rulebook.v1"
        );
    }

    #[test]
    fn a_missing_field_is_named() {
        let mut value = rulebook();
        value["rules"][0]
            .as_object_mut()
            .expect("rule")
            .remove("enforcer");
        assert!(error(&value).starts_with("rules[0]: missing field `enforcer`"));
        let mut value = rulebook();
        value.as_object_mut().expect("book").remove("rules");
        assert!(error(&value).starts_with(".: missing field `rules`"));
    }

    #[test]
    fn domain_rules_hold_with_the_entry_named() {
        let mut value = rulebook();
        value["rules"][0]["sweep_priority"] = json!(7);
        assert_eq!(
            error(&value),
            "rules[0] (rule.no-dbg): a sweep priority is a whole number from 0 (most urgent) to 4"
        );
        let mut value = rulebook();
        value["rules"][0]["paths"] = json!(["../outside"]);
        assert!(error(&value).contains("repository-relative"));
        let mut value = rulebook();
        value["mission"]["statement"] = json!(" ");
        assert_eq!(error(&value), "mission.statement: required text is empty");
    }

    #[test]
    fn the_writer_keeps_the_hand_written_layout_byte_for_byte() {
        let text = include_str!("../whetstone/rulebook.json");
        let book = parse(text, Revision::Uncommitted).expect("Whetstone's rulebook");
        assert_eq!(render(&book).expect("render"), text);
        let mut shuffled = book.clone();
        shuffled.rules.reverse();
        assert_eq!(
            render(&shuffled).expect("render"),
            text,
            "entries are sorted by id"
        );
    }

    #[test]
    fn retired_entries_are_read_strictly_and_written_last() {
        let mut value = rulebook();
        value["retired"] = json!([{"id": "rule.old", "reason": "Superseded by rule.no-dbg."}]);
        let book = read(&value).expect("valid");
        assert_eq!(book.retired[0].id.as_str(), "rule.old");
        let text = render(&book).expect("render");
        assert!(text.ends_with(
            "  \"retired\": [\n    {\n      \"id\": \"rule.old\",\n      \"reason\": \"Superseded by rule.no-dbg.\"\n    }\n  ]\n}\n"
        ), "{text}");
        assert_eq!(
            parse(&text, Revision::Uncommitted).expect("round trip"),
            book
        );
        assert!(!render(&read(&rulebook()).expect("valid"))
            .expect("render")
            .contains("retired"));
        let mut value = rulebook();
        value["retired"] = json!([{"id": "rule.no-dbg", "reason": "Gone."}]);
        assert_eq!(
            error(&value),
            "retired[0].id: rule.no-dbg appears more than once"
        );
        value["retired"] = json!([{"id": "rule.old", "reason": " "}]);
        assert_eq!(error(&value), "retired[0].reason: required text is empty");
        value["retired"] = json!([{"id": "rule.old", "reason": "Gone.", "at": "today"}]);
        assert!(error(&value).starts_with("retired[0].at: unknown field `at`"));
    }

    #[test]
    fn an_entry_is_found_as_the_file_holds_it() {
        let book = read(&rulebook()).expect("valid");
        assert_eq!(
            entry(&book, "rule.no-dbg"),
            Some(rulebook()["rules"][0].clone())
        );
        assert_eq!(entry(&book, "mission"), Some(rulebook()["mission"].clone()));
        assert_eq!(entry(&book, "rule.absent"), None);
        let rebuilt =
            RuleEntry::new(book.rules[0].id.clone(), book.rules[0].rule.clone()).expect("entry");
        assert_eq!(
            rebuilt, book.rules[0],
            "a new entry's digest is the reader's"
        );
    }

    #[test]
    fn ids_are_unique_and_pstack_principles_come_from_the_catalogue() {
        let mut value = rulebook();
        let rule = value["rules"][0].clone();
        value["rules"].as_array_mut().expect("rules").push(rule);
        assert_eq!(
            error(&value),
            "rules[1].id: rule.no-dbg appears more than once"
        );
        let mut value = rulebook();
        value["principles"][0]["source"]["id"] = json!("not-a-principle");
        assert!(error(&value)
            .starts_with("principles[0].source.id: not-a-principle is not a principle"));
    }
}
