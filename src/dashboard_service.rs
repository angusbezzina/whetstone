//! Typed dashboard adapter over the same command service used by the CLI.

use std::path::PathBuf;

use serde::Deserialize;

use crate::dashboard::{BackendResponse, DashboardBackend};
use crate::domain::ContentDigest;
use crate::history::HistoryCursor;
use crate::service::{
    ChangeKind, ChangeRequest, CheckRequest, CommandService, DashRequest, InitAction, InitRequest,
    ServiceRequest, ServiceResponse,
};

/// Keeps the dashboard's project authority fixed at process startup.
///
/// Request bodies deliberately have no `project_dir` field. This prevents an
/// authenticated browser session from turning the dashboard into an arbitrary
/// filesystem reader or writer.
#[derive(Debug)]
pub struct CommandDashboardBackend {
    project_dir: PathBuf,
    inspect_request_id: Option<String>,
    service: CommandService,
}

impl CommandDashboardBackend {
    pub fn new(project_dir: PathBuf, inspect_request_id: Option<String>) -> Self {
        Self {
            project_dir,
            inspect_request_id,
            service: CommandService,
        }
    }

    fn response(response: ServiceResponse) -> BackendResponse {
        match serde_json::to_value(response) {
            Ok(value) => BackendResponse::json(200, &value),
            Err(error) => {
                invalid_request(format!("service response serialization failed: {error}"))
            }
        }
    }
}

impl DashboardBackend for CommandDashboardBackend {
    fn inspect(&self, request_body: &[u8]) -> BackendResponse {
        let query = if request_body.is_empty() {
            DashboardInspectRequest::default()
        } else {
            match serde_json::from_slice(request_body) {
                Ok(query) => query,
                Err(error) => {
                    return invalid_request(format!("invalid dashboard inspection: {error}"));
                }
            }
        };
        Self::response(self.service.execute(ServiceRequest::Dash(DashRequest {
            project_dir: self.project_dir.clone(),
            request_id: self.inspect_request_id.clone(),
            search: query.search,
            as_of: query.as_of,
            history_after: query.history_after,
            page_size: query.page_size,
            expected_snapshot: query.expected_snapshot,
        })))
    }

    fn mutate(&self, request_body: &[u8]) -> BackendResponse {
        let request = match serde_json::from_slice::<DashboardCommand>(request_body) {
            Ok(request) => request.into_service_request(self.project_dir.clone()),
            Err(error) => return invalid_request(format!("invalid dashboard command: {error}")),
        };
        Self::response(self.service.execute(request))
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DashboardInspectRequest {
    search: Option<String>,
    as_of: Option<String>,
    history_after: Option<HistoryCursor>,
    page_size: usize,
    expected_snapshot: Option<ContentDigest>,
}

impl Default for DashboardInspectRequest {
    fn default() -> Self {
        Self {
            search: None,
            as_of: None,
            history_after: None,
            page_size: 100,
            expected_snapshot: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "workflow", rename_all = "snake_case", deny_unknown_fields)]
enum DashboardCommand {
    Init {
        #[serde(default)]
        request_id: Option<String>,
        action: DashboardInitAction,
        #[serde(default)]
        expected_revision: Option<u64>,
        #[serde(default)]
        resume_token: Option<String>,
        #[serde(default)]
        mission: Option<String>,
        #[serde(default)]
        desired_outcome: Option<String>,
        #[serde(default)]
        values: Option<String>,
        #[serde(default)]
        philosophy: Option<String>,
        #[serde(default)]
        owner: Option<String>,
        #[serde(default)]
        initial_safeguard: Option<String>,
        #[serde(default)]
        safeguard_scope: Option<String>,
        #[serde(default)]
        revision_triggers: Option<String>,
    },
    Change {
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        kind: Option<DashboardChangeKind>,
        #[serde(default)]
        record_id: Option<String>,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        expected_effect: Option<String>,
        #[serde(default)]
        impact: Option<String>,
        #[serde(default)]
        examples: Vec<String>,
        #[serde(default)]
        conflicts: Vec<String>,
        #[serde(default)]
        expected_revision: Option<u64>,
        #[serde(default)]
        resume_token: Option<String>,
        #[serde(default)]
        preview: bool,
    },
    Check {
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        paths: Vec<PathBuf>,
        #[serde(default)]
        language: Option<String>,
        #[serde(default)]
        rules: Vec<String>,
    },
}

impl DashboardCommand {
    fn into_service_request(self, project_dir: PathBuf) -> ServiceRequest {
        match self {
            Self::Init {
                request_id,
                action,
                expected_revision,
                resume_token,
                mission,
                desired_outcome,
                values,
                philosophy,
                owner,
                initial_safeguard,
                safeguard_scope,
                revision_triggers,
            } => ServiceRequest::Init(InitRequest {
                project_dir,
                request_id,
                action: action.into(),
                expected_revision,
                resume_token,
                mission,
                desired_outcome,
                values,
                philosophy,
                owner,
                initial_safeguard,
                safeguard_scope,
                revision_triggers,
            }),
            Self::Change {
                request_id,
                kind,
                record_id,
                content,
                rationale,
                source,
                expected_effect,
                impact,
                examples,
                conflicts,
                expected_revision,
                resume_token,
                preview,
            } => ServiceRequest::Change(ChangeRequest {
                project_dir,
                request_id,
                kind: kind.map(Into::into),
                record_id,
                content,
                rationale,
                source,
                expected_effect,
                impact,
                examples,
                conflicts,
                expected_revision,
                resume_token,
                preview,
            }),
            Self::Check {
                request_id,
                paths,
                language,
                rules,
            } => ServiceRequest::Check(CheckRequest {
                project_dir,
                request_id,
                paths,
                language,
                rules,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DashboardInitAction {
    Inspect,
    Agree,
    Cancel,
}

impl From<DashboardInitAction> for InitAction {
    fn from(value: DashboardInitAction) -> Self {
        match value {
            DashboardInitAction::Inspect => Self::Inspect,
            DashboardInitAction::Agree => Self::Agree,
            DashboardInitAction::Cancel => Self::Cancel,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DashboardChangeKind {
    Mission,
    Value,
    Philosophy,
    Guidance,
    Standard,
}

impl From<DashboardChangeKind> for ChangeKind {
    fn from(value: DashboardChangeKind) -> Self {
        match value {
            DashboardChangeKind::Mission => Self::Mission,
            DashboardChangeKind::Value => Self::Value,
            DashboardChangeKind::Philosophy => Self::Philosophy,
            DashboardChangeKind::Guidance => Self::Guidance,
            DashboardChangeKind::Standard => Self::Standard,
        }
    }
}

fn invalid_request(summary: String) -> BackendResponse {
    BackendResponse::json(
        400,
        &serde_json::json!({
            "schema": "whetstone.command-response.v1",
            "schema_version": 1,
            "request_id": "dashboard-invalid-request",
            "workflow": "dash",
            "state": "unknown",
            "summary": summary,
            "evidence": [],
            "blocking_questions": [],
            "permitted_actions": ["correct the bounded request and retry"],
            "data": {},
        }),
    )
}
