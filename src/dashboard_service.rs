//! Typed dashboard adapter over the same command service used by the CLI.

use std::path::PathBuf;

use serde::Deserialize;

use crate::dashboard::{BackendResponse, DashboardBackend};
use crate::domain::ContentDigest;
use crate::history::HistoryCursor;
use crate::service::{
    ChangeDefinition, ChangeKind, ChangeRequest, CheckRequest, CommandService, DashRequest,
    InitAction, InitRequest, ServiceRequest, ServiceResponse,
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
            trail: false,
        })))
    }

    fn evidence(&self, locator: &str) -> BackendResponse {
        let not_found = || BackendResponse {
            status: 404,
            content_type: "text/plain; charset=utf-8",
            body: b"not_found".to_vec(),
        };
        let safe = !locator.is_empty()
            && locator.len() <= 300
            && !locator.starts_with('/')
            && !locator.contains("..")
            && locator.split('/').count() <= 4
            && locator
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "._-/".contains(character));
        if !safe {
            return not_found();
        }
        let content_type = match locator.rsplit('.').next() {
            Some("png") => "image/png",
            Some("json") => "application/json; charset=utf-8",
            Some("log" | "txt") => "text/plain; charset=utf-8",
            _ => return not_found(),
        };
        let Ok(layout) = crate::storage::ProjectLayout::resolve(&self.project_dir, None) else {
            return not_found();
        };
        let root = crate::gates::evidence_root(&layout);
        let path = root.join(locator);
        let (Ok(root), Ok(real)) = (root.canonicalize(), path.canonicalize()) else {
            return not_found();
        };
        let is_file = std::fs::symlink_metadata(&path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= 8 * 1024 * 1024);
        if !real.starts_with(&root) || !is_file {
            return not_found();
        }
        match std::fs::read(&real) {
            Ok(body) => BackendResponse {
                status: 200,
                content_type,
                body,
            },
            Err(_) => not_found(),
        }
    }

    fn trail(&self) -> BackendResponse {
        let mut request =
            DashRequest::basic(self.project_dir.clone(), self.inspect_request_id.clone());
        request.trail = true;
        request.page_size = 1;
        let response = self.service.execute(ServiceRequest::Dash(request));
        match response
            .data
            .get("trail")
            .and_then(serde_json::Value::as_str)
        {
            Some(trail) => BackendResponse {
                status: 200,
                content_type: "text/tab-separated-values; charset=utf-8",
                body: trail.as_bytes().to_vec(),
            },
            None => invalid_request(response.summary),
        }
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
        #[serde(default)]
        gate_command: Option<String>,
        #[serde(default)]
        dry_run: bool,
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
        definition: Option<Box<ChangeDefinition>>,
        #[serde(default)]
        desired_outcome: Option<String>,
        #[serde(default)]
        review_triggers: Option<String>,
        #[serde(default)]
        new_owner: Option<String>,
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
        #[serde(default)]
        review: Option<DashboardReview>,
        #[serde(default)]
        retire: Option<String>,
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
        #[serde(default)]
        features: Vec<String>,
        #[serde(default)]
        mode: DashboardCheckMode,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DashboardReview {
    proposal: String,
    verdict: DashboardVerdict,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DashboardVerdict {
    Accept,
    Withdraw,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DashboardCheckMode {
    #[default]
    All,
    Changed,
    Sweep,
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
                gate_command,
                dry_run,
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
                gate_command,
                dry_run,
                hosts: Vec::new(),
                regenerate_driver: false,
                import_from: None,
                hooks: false,
                ci: false,
                reviewers: Vec::new(),
            }),
            Self::Change {
                request_id,
                kind,
                record_id,
                content,
                definition,
                desired_outcome,
                review_triggers,
                new_owner,
                rationale,
                source,
                expected_effect,
                impact,
                examples,
                conflicts,
                expected_revision,
                resume_token,
                preview,
                review,
                retire,
            } => ServiceRequest::Change(ChangeRequest {
                project_dir,
                request_id,
                kind: kind.map(Into::into),
                record_id,
                content,
                definition,
                desired_outcome,
                review_triggers,
                new_owner,
                rationale,
                source,
                expected_effect,
                impact,
                examples,
                conflicts,
                expected_revision,
                resume_token,
                preview,
                review: review.map(|review| crate::service::ReviewRequest {
                    proposal: review.proposal,
                    verdict: match review.verdict {
                        DashboardVerdict::Accept => crate::domain::LocalReviewVerdict::Accept,
                        DashboardVerdict::Withdraw => crate::domain::LocalReviewVerdict::Withdraw,
                    },
                }),
                retire,
                activate: None,
            }),
            Self::Check {
                request_id,
                paths,
                language,
                rules,
                features,
                mode,
            } => ServiceRequest::Check(CheckRequest {
                project_dir,
                request_id,
                paths,
                language,
                rules,
                features,
                gate_mode: match mode {
                    DashboardCheckMode::All => crate::service::GateMode::All,
                    DashboardCheckMode::Changed => crate::service::GateMode::Changed,
                    DashboardCheckMode::Sweep => crate::service::GateMode::Sweep,
                },
                timeout_seconds: None,
                dry_run: false,
                maintain_outcome: None,
                maintain_evidence: None,
                required: false,
                host: None,
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
    Metric,
    Guidance,
    Standard,
    Feature,
    Map,
}

impl From<DashboardChangeKind> for ChangeKind {
    fn from(value: DashboardChangeKind) -> Self {
        match value {
            DashboardChangeKind::Mission => Self::Mission,
            DashboardChangeKind::Value => Self::Value,
            DashboardChangeKind::Philosophy => Self::Philosophy,
            DashboardChangeKind::Metric => Self::Metric,
            DashboardChangeKind::Guidance => Self::Guidance,
            DashboardChangeKind::Standard => Self::Standard,
            DashboardChangeKind::Feature => Self::Feature,
            DashboardChangeKind::Map => Self::Map,
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
