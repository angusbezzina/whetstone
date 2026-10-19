//! Typed dashboard adapter over the same command service used by the CLI.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;

use crate::dashboard::{BackendResponse, DashboardBackend};
use crate::service::{
    ChangeDefinition, ChangeKind, ChangeRequest, CommandService, DashRequest, ServiceRequest,
    ServiceResponse,
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
        let request = match serde_json::from_slice::<DashboardCommand>(request_body) {
            Ok(request) => request.into_service_request(self.project_dir.clone(), self.actor),
            Err(error) => return invalid_request(format!("invalid dashboard command: {error}")),
        };
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
}

impl Default for DashboardInspectRequest {
    fn default() -> Self {
        Self {
            search: None,
            as_of: None,
            page_size: 100,
        }
    }
}

/// The one workflow the dashboard writes through: `wh change`, on the
/// rulebook file.
#[derive(Debug, Deserialize)]
#[serde(tag = "workflow", rename_all = "snake_case", deny_unknown_fields)]
enum DashboardCommand {
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
        rationale: Option<String>,
        #[serde(default)]
        preview: bool,
        #[serde(default)]
        accept: bool,
        #[serde(default)]
        withdraw: bool,
        #[serde(default)]
        retire: Option<String>,
        /// Write a rule even when its probe says it cannot be checked here.
        #[serde(default)]
        allow_unvalidated: bool,
    },
}

impl DashboardCommand {
    /// The backend sets who is asking; a request body cannot claim it.
    fn into_service_request(
        self,
        project_dir: PathBuf,
        actor: crate::domain::ActorKind,
    ) -> ServiceRequest {
        let Self::Change {
            request_id,
            kind,
            record_id,
            content,
            definition,
            rationale,
            preview,
            accept,
            withdraw,
            retire,
            allow_unvalidated,
        } = self;
        ServiceRequest::Change(ChangeRequest {
            project_dir,
            request_id,
            kind: kind.map(Into::into),
            record_id,
            content,
            definition,
            rationale,
            preview,
            accept,
            withdraw,
            retire,
            actor,
            allow_unvalidated,
        })
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
        backend.mutate(b"{\"workflow\":\"change\",\"withdraw\":true}");
        assert_eq!(seen.lock().expect("observer lock").as_slice(), ["change"]);
    }
}
