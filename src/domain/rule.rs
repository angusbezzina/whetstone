//! Rule record v2 (`whetstone.rule.v2`): one statement, one strength, exactly
//! one enforcer, labelled examples, a source, hand-raise triggers and privacy.
//!
//! Earlier `Standard` and `Guidance` records stay readable and are enforced
//! through [`Rule::from_standard`] and [`Rule::from_guidance`]; every new
//! revision is written as a v2 rule.

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
    Review,
}

impl EnforcerFamily {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mechanical => "mechanical",
            Self::Question => "question",
            Self::Review => "review",
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

/// Who reviews a review-enforced rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reviewer {
    /// pstack's `/interrogate`, run by the agent, verdict attested.
    Interrogate,
    /// A named person.
    Person { name: String },
}

impl Reviewer {
    pub fn label(&self) -> String {
        match self {
            Self::Interrogate => "/interrogate".into(),
            Self::Person { name } => name.clone(),
        }
    }
}

/// The one thing that holds a rule, in order of preference: a mechanical
/// check, a literal yes/no Jev question, or a review.
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
    /// compared with the last pushed revision, raises a hand.
    PublicSurface {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        surfaces: Vec<String>,
    },
    /// A brief (pstack `/how`, `/why`, `/blast-radius`) must be recorded for a
    /// change that touches the rule's paths.
    Brief {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        skills: Vec<String>,
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
        /// the answer is too uncertain to act on and raises a hand.
        #[serde(default = "default_confidence_bar")]
        confidence_bar_bp: u16,
        /// Distinct commits of shadow answers before promotion is considered.
        #[serde(default = "default_promote_after")]
        promote_after: u32,
        /// Precision (accepted over labelled flags) a promotion needs.
        #[serde(default = "default_precision_bar")]
        precision_bar_bp: u16,
    },
    /// pstack's `/interrogate` or a named person attests the verdict.
    Review { reviewer: Reviewer },
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
            Self::Review { .. } => EnforcerFamily::Review,
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
            Self::Brief { .. } => "brief",
            Self::Question { .. } => "question",
            Self::Review { .. } => "review",
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
            Self::Brief { skills } => {
                for skill in skills {
                    require_text(skill)?;
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
            Self::Review { reviewer } => match reviewer {
                Reviewer::Interrogate => Ok(()),
                Reviewer::Person { name } => require_text(name),
            },
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

/// When a result stops the agent and asks the owner instead of repairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandRaiseTrigger {
    /// Any flag from this rule is the owner's call (a public API, say).
    Flag,
    /// A Jev answer below the rule's confidence bar.
    LowConfidence,
    /// The enforcer could not run.
    Unavailable,
}

impl HandRaiseTrigger {
    pub fn label(self) -> &'static str {
        match self {
            Self::Flag => "flag",
            Self::LowConfidence => "low_confidence",
            Self::Unavailable => "unavailable",
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hand_raise: Vec<HandRaiseTrigger>,
    #[serde(default, skip_serializing_if = "Privacy::is_default")]
    pub privacy: Privacy,
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

    /// Whether a question rule is still in shadow (recorded, not enforced).
    pub fn in_shadow(&self) -> bool {
        matches!(self.enforcer, Enforcer::Question { shadow: true, .. })
    }

    pub fn raises_hand_on(&self, trigger: HandRaiseTrigger) -> bool {
        self.hand_raise.contains(&trigger)
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
            hand_raise: Vec::new(),
            privacy: Privacy::default(),
        }
    }

    /// Earlier advisory `Guidance` read as an advisory rule reviewed by
    /// `/interrogate`.
    pub fn from_guidance(guidance: &super::Guidance) -> Self {
        let mut rationale = guidance.rationale.clone();
        if !guidance.examples.is_empty() {
            rationale.push_str(&format!(" Examples: {}.", guidance.examples.join("; ")));
        }
        Self {
            schema: RULE_SCHEMA_V2.into(),
            statement: guidance.statement.clone(),
            rationale,
            strength: Strength::Advisory,
            enforcer: Enforcer::Review {
                reviewer: Reviewer::Interrogate,
            },
            examples: Vec::new(),
            source: RuleSource {
                kind: RuleSourceKind::Migration,
                reference: Some("guidance".into()),
                provenance: Vec::new(),
            },
            paths: Vec::new(),
            hand_raise: Vec::new(),
            privacy: Privacy::default(),
        }
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
