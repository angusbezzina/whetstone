//! Rule record v2 (`whetstone.rule.v2`): one statement, one strength, exactly
//! one enforcer, labelled examples, a source and privacy.
//!
//! Earlier `Standard` records stay readable and are enforced through
//! [`Rule::from_standard`]; every new revision is written as a v2 rule.

use serde::{Deserialize, Serialize};

use super::{require_text, safe_relative_path, DomainError, RecordId};

pub const RULE_SCHEMA_V2: &str = "whetstone.rule.v2";
/// The Jev model pinned when a question does not name one.
pub const DEFAULT_JEV_MODEL: &str = "jev-1.13.0";
pub const MAX_RULE_EXAMPLES: usize = 32;
pub const MAX_RULE_LIST_ITEMS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Strength {
    /// Blocks: nothing but a mechanical pass, a proof or a review passes it.
    Must,
    /// Flags for repair; a person may accept the flag and continue.
    Should,
    /// Shown to agents and people; never blocks.
    Advisory,
}

impl Strength {
    pub fn label(self) -> &'static str {
        match self {
            Self::Must => "must",
            Self::Should => "should",
            Self::Advisory => "advisory",
        }
    }

    /// One step weaker, for demotion drafts; advisory stays advisory.
    pub fn weaker(self) -> Self {
        match self {
            Self::Must => Self::Should,
            Self::Should | Self::Advisory => Self::Advisory,
        }
    }
}

/// Which rung of the enforcement ladder an enforcer is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcerFamily {
    Mechanical,
    Question,
}

impl EnforcerFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mechanical => "mechanical",
            Self::Question => "question",
        }
    }
}

/// The smallest unit of change a question needs; the default is one hunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextUnit {
    /// One changed hunk with a few lines of context.
    #[default]
    Hunk,
    /// The whole changed file.
    File,
    /// A file the change adds (for example a new test file).
    AddedFile,
    /// The commit message and the list of changed paths.
    Commit,
}

impl ContextUnit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hunk => "hunk",
            Self::File => "file",
            Self::AddedFile => "added_file",
            Self::Commit => "commit",
        }
    }
}

/// The one thing that holds a rule, in order of preference: a mechanical
/// check or a literal yes/no Jev question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Enforcer {
    /// A tree-sitter query; a match is a violation.
    Ast {
        query: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        language: Option<String>,
    },
    /// A native linter code, read from the linter's structured output.
    Lint { tool: String, code: String },
    /// A formatter in check mode.
    Formatter { tool: String },
    /// A test command, run without a shell.
    Test { command: String },
    /// A validator command, run without a shell.
    Validator { command: String },
    /// Prove a mapped feature by driving the running app.
    Drive { feature: RecordId },
    /// Literal colours and sizes outside the token file are violations.
    DesignTokens {
        tokens: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stylesheets: Vec<String>,
        /// Also flag literal spacing and type sizes (colours are always checked).
        #[serde(default = "default_true", skip_serializing_if = "is_true")]
        sizes: bool,
    },
    /// Any change to exported items, CLI flags or JSON envelope fields,
    /// compared with the last pushed revision, is flagged for the owner.
    PublicSurface {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        surfaces: Vec<String>,
    },
    /// A literal yes/no question for Jev. Yes means the rule is broken.
    Question {
        question: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        yes_means: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        no_means: Option<String>,
        #[serde(default)]
        unit: ContextUnit,
        #[serde(default = "default_model")]
        model: String,
        /// Answers are recorded and shown, never enforced, until the owner
        /// accepts a promotion draft.
        #[serde(default = "default_true")]
        shadow: bool,
        /// Probability of yes, in basis points, at or above which the answer
        /// flags the rule.
        #[serde(default = "default_threshold")]
        threshold_bp: u16,
        /// Confidence (distance from even odds, in basis points) below which
        /// the answer is too uncertain to act on.
        #[serde(default = "default_confidence_bar")]
        confidence_bar_bp: u16,
        /// Distinct commits of shadow answers before promotion is considered.
        #[serde(default = "default_promote_after")]
        promote_after: u32,
        /// Precision (accepted over labelled flags) a promotion needs.
        #[serde(default = "default_precision_bar")]
        precision_bar_bp: u16,
    },
}

fn default_model() -> String {
    DEFAULT_JEV_MODEL.into()
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn default_threshold() -> u16 {
    5_000
}

fn default_confidence_bar() -> u16 {
    4_000
}

fn default_promote_after() -> u32 {
    50
}

fn default_precision_bar() -> u16 {
    9_000
}

impl Enforcer {
    pub fn family(&self) -> EnforcerFamily {
        match self {
            Self::Question { .. } => EnforcerFamily::Question,
            _ => EnforcerFamily::Mechanical,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ast { .. } => "ast",
            Self::Lint { .. } => "lint",
            Self::Formatter { .. } => "formatter",
            Self::Test { .. } => "test",
            Self::Validator { .. } => "validator",
            Self::Drive { .. } => "drive",
            Self::DesignTokens { .. } => "design_tokens",
            Self::PublicSurface { .. } => "public_surface",
            Self::Question { .. } => "question",
        }
    }

    /// Whether the enforcer reads only file contents, so pre-commit can run it
    /// on the staged index within its budget.
    pub fn runs_staged(&self) -> bool {
        matches!(
            self,
            Self::Ast { .. } | Self::DesignTokens { .. } | Self::PublicSurface { .. }
        )
    }

    fn validate(&self) -> Result<(), DomainError> {
        match self {
            Self::Ast { query, language } => {
                require_text(query)?;
                if let Some(language) = language {
                    require_text(language)?;
                }
                Ok(())
            }
            Self::Lint { tool, code } => {
                require_text(tool)?;
                require_text(code)
            }
            Self::Formatter { tool } => require_text(tool),
            Self::Test { command } | Self::Validator { command } => require_text(command),
            Self::Drive { .. } => Ok(()),
            Self::DesignTokens {
                tokens,
                stylesheets,
                ..
            } => {
                if !safe_relative_path(tokens) {
                    return Err(DomainError::InvalidField(
                        "design token file must be repository-relative",
                    ));
                }
                bounded_paths(stylesheets)
            }
            Self::PublicSurface { surfaces } => {
                for surface in surfaces {
                    if !matches!(surface.as_str(), "exports" | "cli" | "json") {
                        return Err(DomainError::InvalidField(
                            "public surfaces are exports, cli or json",
                        ));
                    }
                }
                Ok(())
            }
            Self::Question {
                question,
                yes_means,
                no_means,
                model,
                threshold_bp,
                confidence_bar_bp,
                precision_bar_bp,
                ..
            } => {
                require_text(question)?;
                require_text(model)?;
                for text in [yes_means, no_means].into_iter().flatten() {
                    require_text(text)?;
                }
                if question.len() > 1_000 {
                    return Err(DomainError::InvalidField(
                        "a Jev question must be one literal question under 1000 bytes",
                    ));
                }
                if *threshold_bp == 0
                    || *threshold_bp > 10_000
                    || *confidence_bar_bp > 10_000
                    || *precision_bar_bp > 10_000
                {
                    return Err(DomainError::InvalidField(
                        "question thresholds are basis points from 1 to 10000",
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExampleVerdict {
    /// The rule holds for this input.
    Pass,
    /// The rule is broken by this input.
    Flag,
}

/// A labelled example that defines the rule and that `wh eval` scores the
/// enforcer against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleExample {
    pub input: String,
    pub expected: ExampleVerdict,
    pub reason: String,
    /// The file name the input stands for (it picks the language and the
    /// token or surface parser), for example `src/app.css`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleSourceKind {
    /// Derived from the mission.
    Mission,
    /// Derived from a principle record.
    Principle,
    /// Distilled from an exemplar codebase.
    Exemplar,
    /// Written by the owner.
    Owner,
    /// One of Whetstone's starter rules.
    Starter,
    /// Replaces a recurring Jev flag with a mechanical check.
    Hardening,
    /// Migrated from an earlier record or rule file.
    Migration,
    /// Drafted from an admitted lesson (the reference is its key).
    Lesson,
}

impl RuleSourceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mission => "mission",
            Self::Principle => "principle",
            Self::Exemplar => "exemplar",
            Self::Owner => "owner",
            Self::Starter => "starter",
            Self::Hardening => "hardening",
            Self::Migration => "migration",
            Self::Lesson => "lesson",
        }
    }
}

/// Where a rule came from, with provenance for exemplars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSource {
    pub kind: RuleSourceKind,
    /// The principle id, exemplar locator, starter id or hardened rule id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Exact files or commits read to derive the rule (never executed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<super::EvidenceRef>,
}

impl RuleSource {
    pub fn owner() -> Self {
        Self {
            kind: RuleSourceKind::Owner,
            reference: None,
            provenance: Vec::new(),
        }
    }
}

/// What may leave the machine when a question runs.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Privacy {
    /// Paths whose text is never sent (replaced by a placeholder).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redact_globs: Vec<String>,
    /// Regular expressions whose matches are replaced before sending.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub redact_patterns: Vec<String>,
    /// Never call Jev for this rule; it routes to review instead.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub local_only: bool,
}

impl Privacy {
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// How a below-must rule's flags are worked by a night sweep (Messo's, inside
/// Virgil): the reach an agent has over them. Narrowest is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SweepClass {
    /// The agent only posts and asks.
    #[default]
    Discuss,
    /// A person approves the exact plan first.
    ConfirmFirst,
    /// A mechanical fix the agent may open as a draft PR.
    DraftPr,
}

impl SweepClass {
    pub fn label(self) -> &'static str {
        match self {
            Self::Discuss => "discuss",
            Self::ConfirmFirst => "confirm_first",
            Self::DraftPr => "draft_pr",
        }
    }

    /// The agent's reach, narrowest first.
    pub fn reach(self) -> u8 {
        match self {
            Self::Discuss => 0,
            Self::ConfirmFirst => 1,
            Self::DraftPr => 2,
        }
    }
}

/// How urgent a rule's sweep findings are when it names none: the middle of
/// 0 (most urgent) to 4.
pub const DEFAULT_SWEEP_PRIORITY: u8 = 2;
pub const MAX_SWEEP_PRIORITY: u8 = 4;
const MAX_SWEEP_UNSURE: usize = 16;

/// Where a rule's own flags are doubtful: a flag on one of `paths` (globs),
/// or whose flagged text matches `matching`, is reported `unsure` with the
/// reason, so a sweep parks it until a person raises it. It never covers
/// every flag; a rule that doubts all its flags is a rule to demote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SweepUnsure {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// A regular expression over the flagged line (or the failure message).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matching: Option<String>,
    /// One line: why the check cannot tell whether such a flag is real.
    pub reason: String,
}

impl SweepUnsure {
    fn validate(&self) -> Result<(), DomainError> {
        let reason = self.reason.trim();
        if reason.is_empty() || reason.len() > 200 || reason.contains('\n') {
            return Err(DomainError::InvalidField(
                "an unsure reason is one line of at most 200 bytes",
            ));
        }
        if self.paths.is_empty() && self.matching.is_none() {
            return Err(DomainError::InvalidField(
                "an unsure entry names paths or a matching pattern; one that covers every flag is a rule to demote",
            ));
        }
        bounded_paths(&self.paths)?;
        let everything = ["*", "**", "**/*", ".", "./", "./**"];
        if self
            .paths
            .iter()
            .any(|path| everything.contains(&path.trim()))
        {
            return Err(DomainError::InvalidField(
                "an unsure path covers every flag; a rule that doubts all its flags is a rule to demote",
            ));
        }
        if let Some(pattern) = &self.matching {
            let regex = regex::Regex::new(pattern).map_err(|_| {
                DomainError::InvalidField("an unsure pattern is not a valid regular expression")
            })?;
            if regex.is_match("") {
                return Err(DomainError::InvalidField(
                    "an unsure pattern matches every flag; a rule that doubts all its flags is a rule to demote",
                ));
            }
        }
        Ok(())
    }

    /// Whether a flag on `path` (when it names one) with `text` is doubtful.
    pub fn covers(&self, path: Option<&str>, text: &str) -> bool {
        let on_path = path.is_some_and(|path| {
            self.paths
                .iter()
                .any(|pattern| super::entry_point_matches(pattern.trim_start_matches("./"), path))
        });
        let matched = self
            .matching
            .as_deref()
            .and_then(|pattern| regex::Regex::new(pattern).ok())
            .is_some_and(|regex| regex.is_match(text));
        on_path || matched
    }
}

/// Rule record v2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub schema: String,
    pub statement: String,
    pub rationale: String,
    pub strength: Strength,
    pub enforcer: Enforcer,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub examples: Vec<RuleExample>,
    pub source: RuleSource,
    /// Repository paths or globs the rule applies to; empty means everywhere.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Privacy::is_default")]
    pub privacy: Privacy,
    /// Recorded and shown, never enforced, whatever the enforcer, until the
    /// owner accepts a promotion draft (a rule drafted from a lesson starts
    /// here). A question rule's own `shadow` works the same way.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shadow: bool,
    /// How a night sweep works this rule's flags; absent means discuss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep: Option<SweepClass>,
    /// How urgent this rule's sweep findings are, 0 (most) to 4; absent
    /// means 2. A person's priority on the task downstream still wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sweep_priority: Option<u8>,
    /// Where this rule's own flags are doubtful (see [`SweepUnsure`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sweep_unsure: Vec<SweepUnsure>,
}

impl Rule {
    pub fn validate(&self) -> Result<(), DomainError> {
        if self.schema != RULE_SCHEMA_V2 {
            return Err(DomainError::InvalidField(
                "rule schema must be whetstone.rule.v2",
            ));
        }
        require_text(&self.statement)?;
        require_text(&self.rationale)?;
        self.enforcer.validate()?;
        if self.examples.len() > MAX_RULE_EXAMPLES {
            return Err(DomainError::InvalidField("a rule has at most 32 examples"));
        }
        for example in &self.examples {
            require_text(&example.input)?;
            require_text(&example.reason)?;
            if let Some(path) = &example.path {
                if !safe_relative_path(path) {
                    return Err(DomainError::InvalidField(
                        "example paths must be repository-relative",
                    ));
                }
            }
        }
        bounded_paths(&self.paths)?;
        bounded_paths(&self.privacy.redact_globs)?;
        if self.privacy.redact_patterns.len() > MAX_RULE_LIST_ITEMS {
            return Err(DomainError::InvalidField("too many redaction patterns"));
        }
        for pattern in &self.privacy.redact_patterns {
            require_text(pattern)?;
            if regex::Regex::new(pattern).is_err() {
                return Err(DomainError::InvalidField(
                    "a redaction pattern is not a valid regular expression",
                ));
            }
        }
        if let Some(reference) = &self.source.reference {
            require_text(reference)?;
        }
        if self
            .sweep_priority
            .is_some_and(|priority| priority > MAX_SWEEP_PRIORITY)
        {
            return Err(DomainError::InvalidField(
                "a sweep priority is a whole number from 0 (most urgent) to 4",
            ));
        }
        if self.sweep_unsure.len() > MAX_SWEEP_UNSURE {
            return Err(DomainError::InvalidField(
                "a rule has at most 16 unsure entries",
            ));
        }
        for unsure in &self.sweep_unsure {
            unsure.validate()?;
        }
        Ok(())
    }

    /// Whether this rule applies to a repository-relative path.
    pub fn applies_to(&self, path: &str) -> bool {
        self.paths.is_empty()
            || self
                .paths
                .iter()
                .any(|pattern| super::entry_point_matches(pattern.trim_start_matches("./"), path))
    }

    /// Whether the rule is in shadow (recorded, not enforced): the rule's
    /// own flag, or a question rule's.
    pub fn in_shadow(&self) -> bool {
        self.shadow || matches!(self.enforcer, Enforcer::Question { shadow: true, .. })
    }

    /// How a sweep works this rule's flags (discuss when unset).
    pub fn sweep_class(&self) -> SweepClass {
        self.sweep.unwrap_or_default()
    }

    /// How urgent this rule's sweep findings are (2 when unset).
    pub fn sweep_priority(&self) -> u8 {
        self.sweep_priority.unwrap_or(DEFAULT_SWEEP_PRIORITY)
    }

    /// Why a flag on `path` with `text` is doubtful, when the rule says so.
    pub fn sweep_unsure_for(&self, path: Option<&str>, text: &str) -> Option<&str> {
        self.sweep_unsure
            .iter()
            .find(|unsure| unsure.covers(path, text))
            .map(|unsure| unsure.reason.trim())
    }

    /// An earlier `Standard` read as a v2 rule (`may` is advisory).
    pub fn from_standard(standard: &super::Standard) -> Self {
        use super::{Enforcement, StandardStrength};
        let enforcer = match &standard.enforcement {
            Enforcement::Ast { query } => Enforcer::Ast {
                query: query.clone(),
                language: None,
            },
            Enforcement::LintProxy { tool, code } => Enforcer::Lint {
                tool: tool.clone(),
                code: code.clone(),
            },
            Enforcement::Formatter { tool } => Enforcer::Formatter { tool: tool.clone() },
            Enforcement::Test { command_ref } => Enforcer::Test {
                command: command_ref.clone(),
            },
            Enforcement::Validator { command_ref } => Enforcer::Validator {
                command: command_ref.clone(),
            },
            Enforcement::Drive { feature } => Enforcer::Drive {
                feature: feature.clone(),
            },
        };
        let mut rationale = standard.rationale.clone();
        if !standard.examples.is_empty() {
            rationale.push_str(&format!(" Examples: {}.", standard.examples.join("; ")));
        }
        Self {
            schema: RULE_SCHEMA_V2.into(),
            statement: standard.statement.clone(),
            rationale,
            strength: match standard.strength {
                StandardStrength::Must => Strength::Must,
                StandardStrength::Should => Strength::Should,
                StandardStrength::May => Strength::Advisory,
            },
            enforcer,
            examples: Vec::new(),
            source: RuleSource {
                kind: RuleSourceKind::Migration,
                reference: Some("standard".into()),
                provenance: Vec::new(),
            },
            paths: Vec::new(),
            privacy: Privacy::default(),
            shadow: false,
            sweep: None,
            sweep_priority: None,
            sweep_unsure: Vec::new(),
        }
    }
}

/// Fields of stored rule revisions that the current model drops on read.
const DROPPED_RULE_FIELDS: &[&str] = &["hand_raise"];
/// Enforcer kinds of stored rule revisions that nothing holds any more.
const RETIRED_ENFORCERS: &[&str] = &["review", "brief"];

/// How [`Rule::from_stored`] reads a stored rule revision.
pub(super) enum StoredRead {
    /// Read as this rule.
    Mapped(Box<Rule>),
    /// Held by an enforcer that is gone: kept as history only.
    Retired,
}

impl Rule {
    /// Read a stored rule revision that does not parse as a current rule:
    /// one held by a retired enforcer is history, and otherwise the dropped
    /// fields are ignored. `None` when the revision is not of either shape
    /// (it is then malformed).
    pub(super) fn from_stored(record: &serde_json::Value) -> Option<StoredRead> {
        let kind = record
            .pointer("/enforcer/kind")
            .and_then(serde_json::Value::as_str);
        if kind.is_some_and(|kind| RETIRED_ENFORCERS.contains(&kind)) {
            return Some(StoredRead::Retired);
        }
        let mut fields = record.as_object()?.clone();
        let mut mapped = false;
        for field in DROPPED_RULE_FIELDS {
            mapped |= fields.remove(*field).is_some();
        }
        if !mapped {
            return None;
        }
        serde_json::from_value(serde_json::Value::Object(fields))
            .ok()
            .map(|rule| StoredRead::Mapped(Box::new(rule)))
    }
}

fn bounded_paths(paths: &[String]) -> Result<(), DomainError> {
    if paths.len() > MAX_RULE_LIST_ITEMS {
        return Err(DomainError::InvalidField("a rule lists at most 32 paths"));
    }
    for path in paths {
        if !safe_relative_path(path.trim_start_matches("./")) || path.contains('\\') {
            return Err(DomainError::InvalidField(
                "rule paths must be repository-relative",
            ));
        }
    }
    Ok(())
}
