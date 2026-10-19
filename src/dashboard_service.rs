//! Typed dashboard adapter over the same command service used by the CLI.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;

use crate::dashboard::{BackendResponse, DashboardBackend};
use crate::domain::ContentDigest;
use crate::service::{
    ChangeDefinition, ChangeKind, ChangeRequest, CommandService, DashRequest, InitAction,
    InitRequest, ServiceRequest, ServiceResponse,
};

/// Keeps the dashboard's project authority fixed at process startup.
///
/// Request bodies deliberately have no `project_dir` field. This prevents an
/// authenticated browser session from turning the dashboard into an arbitrary
/// filesystem reader or writer.
pub struct CommandDashboardBackend {
    project_dir: PathBuf,
    inspect_request_id: Option<String>,
    service: CommandService,
    /// Who the dashboard acts for: a person when a person opened it, an agent
    /// (the default) otherwise, so automation never acts as a person.
    actor: crate::domain::ActorKind,
    /// Sees every mutation response after the service has answered it, so
    /// the foreground process can narrate what the dashboard recorded.
    /// Inspections are never reported.
    observer: Option<MutationObserver>,
}

/// Called with each mutation response; must not block the request thread.
pub type MutationObserver = Arc<dyn Fn(&ServiceResponse) + Send + Sync>;

impl std::fmt::Debug for CommandDashboardBackend {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommandDashboardBackend")
            .field("project_dir", &self.project_dir)
            .field("inspect_request_id", &self.inspect_request_id)
            .field("observer", &self.observer.is_some())
            .finish_non_exhaustive()
    }
}

impl CommandDashboardBackend {
    pub fn new(project_dir: PathBuf, inspect_request_id: Option<String>) -> Self {
        Self {
            project_dir,
            inspect_request_id,
            service: CommandService,
            actor: crate::domain::ActorKind::Agent,
            observer: None,
        }
    }

    /// Act for `actor`: only a dashboard a person opened (never one started
    /// for automation) acts for a person.
    pub fn acting_as(mut self, actor: crate::domain::ActorKind) -> Self {
        self.actor = actor;
        self
    }

    /// Report each mutation response to `observer`.
    pub fn observed(mut self, observer: MutationObserver) -> Self {
        self.observer = Some(observer);
        self
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
            page_size: query.page_size,
            expected_snapshot: query.expected_snapshot,
            trail: false,
        })))
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
        let mut request = match serde_json::from_slice::<DashboardCommand>(request_body) {
            Ok(request) => request.into_service_request(self.project_dir.clone()),
            Err(error) => return invalid_request(format!("invalid dashboard command: {error}")),
        };
        if let ServiceRequest::Change(change) = &mut request {
            change.actor = self.actor;
        }
        let response = self.service.execute(request);
        if let Some(observer) = &self.observer {
            observer(&response);
        }
        Self::response(response)
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct DashboardInspectRequest {
    search: Option<String>,
    as_of: Option<String>,
    page_size: usize,
    expected_snapshot: Option<ContentDigest>,
}

impl Default for DashboardInspectRequest {
    fn default() -> Self {
        Self {
            search: None,
            as_of: None,
            page_size: 100,
            expected_snapshot: None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "workflow", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
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
        principles: Vec<String>,
        #[serde(default)]
        custom_principles: Vec<String>,
        #[serde(default)]
        starters: Vec<String>,
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

impl DashboardCommand {
    fn into_service_request(self, project_dir: PathBuf) -> ServiceRequest {
        match self {
            Self::Init {
                request_id,
                action,
                expected_revision,
                resume_token,
                mission,
                principles,
                custom_principles,
                starters,
                dry_run,
            } => ServiceRequest::Init(InitRequest {
                project_dir,
                request_id,
                action: action.into(),
                expected_revision,
                resume_token,
                mission,
                principles,
                custom_principles,
                starters,
                dry_run,
                ..InitRequest::default()
            }),
            Self::Change {
                request_id,
                kind,
                record_id,
                content,
                definition,
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
                // The backend sets who is asking; a request cannot claim it.
                actor: crate::domain::ActorKind::Agent,
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
    Principle,
    Rule,
}

impl From<DashboardChangeKind> for ChangeKind {
    fn from(value: DashboardChangeKind) -> Self {
        match value {
            DashboardChangeKind::Mission => Self::Mission,
            DashboardChangeKind::Principle => Self::Principle,
            DashboardChangeKind::Rule => Self::Rule,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn observed_backend() -> (CommandDashboardBackend, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        let backend = CommandDashboardBackend::new(std::env::temp_dir(), None).observed(Arc::new(
            move |response: &ServiceResponse| {
                sink.lock()
                    .expect("observer lock")
                    .push(response.workflow.clone());
            },
        ));
        (backend, seen)
    }

    #[test]
    fn the_observer_sees_mutations_but_not_inspections_or_rejected_bodies() {
        let (backend, seen) = observed_backend();
        backend.inspect(b"");
        assert!(
            seen.lock().expect("observer lock").is_empty(),
            "inspections are not reported"
        );
        let rejected = backend.mutate(b"{\"workflow\":\"nope\"}");
        assert_eq!(rejected.status, 400);
        assert!(
            seen.lock().expect("observer lock").is_empty(),
            "rejected bodies never reach the service"
        );
        backend.mutate(b"{\"workflow\":\"init\",\"action\":\"inspect\"}");
        assert_eq!(seen.lock().expect("observer lock").as_slice(), ["init"]);
    }
}
