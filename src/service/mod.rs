//! Shared command services used by the CLI and the dashboard: one typed
//! request and one versioned response envelope per workflow.
//!
//! Each workflow lives in its own module: `init`, `dash`, `change` and
//! `check`; `snapshot` binds a check's report to what it read, and `trail`
//! reads the rulebook's Git log.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::{ActorKind, Enforcer, RuleExample, RuleSource, Strength, SweepClass};
use crate::layout::LayoutError;

mod change;
mod check;
mod dash;
mod init;
mod snapshot;
mod trail;

pub use trail::{decision_trail, rulebook_log, RulebookCommit, TRAIL_HEADER};

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
/// clearly a person (no terminal on stdin, or an agent host's marker in the
/// environment) is an agent, so an agent can never pass as a person by
/// leaving something out.
pub fn detect_actor() -> ActorKind {
    use std::io::IsTerminal;
    let marked = AGENT_MARKERS
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty()));
    if !marked && std::io::stdin().is_terminal() {
        ActorKind::Person
    } else {
        ActorKind::Agent
    }
}

pub const RESPONSE_SCHEMA: &str = "whetstone.command-response.v1";

#[derive(Debug, Clone)]
pub enum ServiceRequest {
    Orientation,
    Init(InitRequest),
    Dash(DashRequest),
    Change(ChangeRequest),
    Check(CheckRequest),
}

#[derive(Debug, Clone)]
pub struct DashRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub search: Option<String>,
    pub as_of: Option<String>,
    pub page_size: usize,
    /// Include the show-me-your-work TSV decision trail as `data.trail`.
    pub trail: bool,
}

impl DashRequest {
    pub fn basic(project_dir: PathBuf, request_id: Option<String>) -> Self {
        Self {
            project_dir,
            request_id,
            search: None,
            as_of: None,
            page_size: 100,
            trail: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InitAction {
    #[default]
    Inspect,
    /// Draft the owner's mission, principles and accepted starter rules
    /// into the rulebook.
    Agree,
    Cancel,
    /// Install the Git hooks or scaffold CI, as asked.
    Wire,
    /// Install the Git hooks and scaffold CI.
    Setup,
}

#[derive(Debug, Clone, Default)]
pub struct InitRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub action: InitAction,
    pub mission: Option<String>,
    /// Catalogue principle ids the owner picked (agree).
    pub principles: Vec<String>,
    /// The owner's own principles, in their words (agree).
    pub custom_principles: Vec<String>,
    /// Starter rule ids the owner accepts, or `all` (agree).
    pub starters: Vec<String>,
    /// Report exact writes without performing them.
    pub dry_run: bool,
    /// Install the Git pre-commit and pre-push hooks (wire).
    pub hooks: bool,
    /// Scaffold the workflow that runs the required whetstone/policy check (wire).
    pub ci: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Mission,
    Principle,
    Rule,
}

/// Always stored boxed (`Option<Box<ChangeDefinition>>`), so the rule
/// variant's size never inflates the requests that carry it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ChangeDefinition {
    Rule {
        strength: Strength,
        enforcer: Enforcer,
        #[serde(default)]
        examples: Vec<RuleExample>,
        #[serde(default)]
        source: Option<RuleSource>,
        #[serde(default)]
        paths: Vec<String>,
        /// How a night sweep works the rule's flags (discuss when absent).
        #[serde(default)]
        sweep: Option<SweepClass>,
        /// How urgent its sweep findings are, 0 (most) to 4 (2 when absent).
        #[serde(default)]
        sweep_priority: Option<u8>,
        /// Where its own flags are doubtful: findings there say `unsure`.
        #[serde(default)]
        sweep_unsure: Vec<crate::domain::SweepUnsure>,
    },
    Principle {
        /// A pstack principle id from the catalogue; absent means custom.
        #[serde(default)]
        pstack: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
    },
}

#[derive(Debug, Clone, Default)]
pub struct ChangeRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub kind: Option<ChangeKind>,
    pub record_id: Option<String>,
    pub content: Option<String>,
    pub definition: Option<Box<ChangeDefinition>>,
    /// The rule's or principle's rationale, the reason for a retirement, or
    /// the body of the commit that accepts the drafted edits.
    pub rationale: Option<String>,
    /// Show the exact effect without writing or committing anything.
    pub preview: bool,
    /// Commit the rulebook's drafted edits.
    pub accept: bool,
    /// Restore the rulebook to its last commit.
    pub withdraw: bool,
    /// Take a rule or principle out of the rulebook, recording why.
    pub retire: Option<String>,
    /// Who is asking: a person, or an agent (the default, so nothing passes
    /// as a person by omission).
    pub actor: ActorKind,
}

/// Which in-force rules a check runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GateMode {
    /// Every in-force rule (or those named by `rules`).
    #[default]
    All,
    /// Rules that apply to what changed.
    Changed,
    /// Pre-commit: mechanical content checks on the staged index only.
    Staged,
    /// Every rule below must, each flag reported as a finding.
    Sweep,
}

#[derive(Debug, Clone, Default)]
pub struct CheckRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub rules: Vec<String>,
    pub gate_mode: GateMode,
    pub timeout_seconds: Option<u64>,
    /// Report the selection and exact commands without executing anything.
    pub dry_run: bool,
    /// The required CI check: enforce the committed rulebook only, refusing
    /// one with uncommitted changes.
    pub ci: bool,
    /// Include paths changed since this revision (a committed change), not
    /// only the working tree.
    pub base: Option<String>,
    /// The commit a push sends (pre-push). The check reads the working tree,
    /// so it refuses unless that commit is checked out with no uncommitted
    /// tracked changes; changed paths then come from commits only.
    pub pushed: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Success,
    Violated,
    Unknown,
    Unavailable,
    NeedsDecision,
    NeedsInput,
    Stale,
    Conflict,
}

impl ServiceState {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Success => 0,
            Self::Violated => 1,
            Self::Unknown => 3,
            Self::Unavailable => 4,
            Self::NeedsDecision => 5,
            Self::NeedsInput => 6,
            Self::Stale => 7,
            Self::Conflict => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceEvidence {
    pub kind: String,
    pub locator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiredSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checker_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceResponse {
    pub schema: String,
    pub schema_version: u16,
    pub request_id: String,
    pub workflow: String,
    pub state: ServiceState,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_snapshot: Option<RequiredSnapshot>,
    #[serde(default)]
    pub evidence: Vec<ServiceEvidence>,
    #[serde(default)]
    pub blocking_questions: Vec<String>,
    #[serde(default)]
    pub permitted_actions: Vec<String>,
    pub data: Value,
}

impl ServiceResponse {
    /// Construct an envelope for workflows implemented outside this module.
    pub fn new_public(
        request_id: String,
        workflow: &str,
        state: ServiceState,
        summary: impl Into<String>,
    ) -> Self {
        Self::new(request_id, workflow, state, summary)
    }

    pub(crate) fn new(
        request_id: String,
        workflow: &str,
        state: ServiceState,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            schema: RESPONSE_SCHEMA.into(),
            schema_version: 1,
            request_id,
            workflow: workflow.into(),
            state,
            summary: summary.into(),
            required_snapshot: None,
            evidence: Vec::new(),
            blocking_questions: Vec::new(),
            permitted_actions: Vec::new(),
            data: json!({}),
        }
    }
}

#[derive(Debug, Default)]
pub struct CommandService;

impl CommandService {
    pub fn execute(&self, request: ServiceRequest) -> ServiceResponse {
        match request {
            ServiceRequest::Orientation => orientation(),
            ServiceRequest::Init(request) => init::init(request),
            ServiceRequest::Dash(request) => dash::dash(request),
            ServiceRequest::Change(request) => change::change(request),
            ServiceRequest::Check(request) => check::check(request),
        }
    }
}

/// Bare `wh`: honest, read-only orientation.
fn orientation() -> ServiceResponse {
    let mut response = ServiceResponse::new(
        "orientation".into(),
        "orientation",
        ServiceState::Success,
        "Whetstone keeps the rulebook in whetstone/rulebook.json and checks every rule at pre-commit, pre-push and CI.",
    );
    response.permitted_actions = vec![
        "wh init".into(),
        "wh dash".into(),
        "wh change".into(),
        "wh check".into(),
    ];
    response.data = json!({
        "workflows": ["init", "dash", "change", "check"],
        "available_now": ["init", "dash", "change", "check"],
        "rulebook": crate::rulebook::RULEBOOK_PATH,
        "read_only": true,
    });
    response
}

/// Current UTC time in RFC 3339 form, second precision.
pub(crate) fn utc_now() -> String {
    Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| value.ends_with('Z'))
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".into())
}

pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

pub(crate) fn bounded_input(
    value: Option<String>,
    name: &str,
    maximum: usize,
) -> Result<String, String> {
    let value = value.ok_or_else(|| format!("Provide {name}."))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("Provide non-empty {name}."))
    } else if trimmed.len() > maximum {
        Err(format!("{name} exceeds the {maximum}-byte limit."))
    } else {
        Ok(trimmed.to_string())
    }
}

pub(crate) fn bounded_request_id(
    value: Option<String>,
    fallback: String,
) -> Result<String, String> {
    let value = value.unwrap_or(fallback);
    if value.is_empty()
        || value.len() > 128
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':')
        })
    {
        Err("Request ID must be 1-128 ASCII letters, digits, dots, colons, underscores, or hyphens."
            .into())
    } else {
        Ok(value)
    }
}

pub(crate) fn unknown_response(
    workflow: &str,
    request_id: String,
    summary: String,
) -> ServiceResponse {
    let mut response = ServiceResponse::new(request_id, workflow, ServiceState::Unknown, summary);
    response.permitted_actions = vec![format!(
        "repair the input or environment, then wh {workflow}"
    )];
    response
}

pub(crate) fn project_error(
    workflow: &str,
    request_id: Option<String>,
    error: LayoutError,
) -> ServiceResponse {
    unknown_response(
        workflow,
        request_id.unwrap_or_else(|| workflow.into()),
        format!("Project identity could not be resolved: {error}"),
    )
}

pub(crate) fn project_label(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_states_have_stable_distinct_exit_codes() {
        let codes = [
            ServiceState::Success,
            ServiceState::Violated,
            ServiceState::Unknown,
            ServiceState::Unavailable,
            ServiceState::NeedsDecision,
            ServiceState::NeedsInput,
            ServiceState::Stale,
            ServiceState::Conflict,
        ]
        .map(ServiceState::exit_code);
        let unique = codes.into_iter().collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), codes.len());
    }
}
