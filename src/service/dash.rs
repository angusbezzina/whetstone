//! `wh dash`: the read-only project view the dashboard and `--json` share,
//! and the decision trail (`--trail`).

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::history::{HistoryError, HistoryInspectionRequest, HistoryInspectionService};
use crate::projection;
use crate::storage::{ProjectLayout, StorageError};

use super::init::progress;
use super::{
    bounded_request_id, load_records, project_error, project_label, shared_refs, unknown_response,
    utc_now, DashRequest, LoadedRecords, ServiceEvidence, ServiceResponse, ServiceState,
};

pub(super) fn dash(request: DashRequest) -> ServiceResponse {
    let layout = match ProjectLayout::resolve(&request.project_dir) {
        Ok(layout) => layout,
        Err(error) => return project_error("dash", request.request_id, error),
    };
    let request_id = match bounded_request_id(
        request.request_id,
        format!("dash-{}", &layout.project_id()[..16]),
    ) {
        Ok(value) => value,
        Err(summary) => return unknown_response("dash", "invalid-request-id".into(), summary),
    };
    let as_of = request.as_of.unwrap_or_else(utc_now);
    let page_size = request.page_size.clamp(1, 200);
    // One read of each store serves progress, projection and history.
    let loaded = load_records(&layout);
    let store_exists = loaded.as_ref().is_ok_and(LoadedRecords::exists);
    let (records, sync_conflicts) = match &loaded {
        Ok(loaded) if loaded.exists() => {
            let (records, conflicts) = loaded.union();
            (Some(Ok(records)), conflicts)
        }
        Ok(_) => (None, Vec::new()),
        Err(error) => (
            Some(Err(StorageError::UnexpectedData(error.to_string()))),
            Vec::new(),
        ),
    };
    let history_service = match &loaded {
        Ok(loaded) if loaded.exists() => HistoryInspectionService::from_records(
            loaded.private.clone().unwrap_or_default(),
            loaded.shared.clone().unwrap_or_default(),
        ),
        Ok(_) => Err(HistoryError::Storage(
            "no private or shared records exist yet".into(),
        )),
        Err(error) => Err(HistoryError::Storage(error.to_string())),
    };
    let history_request = HistoryInspectionRequest {
        project: format!("project-{}", &layout.project_id()[..12]),
        as_of,
        search: request.search.clone(),
        page_size,
        expected_snapshot: request.expected_snapshot,
    };
    let (history, journal_items) = match history_service {
        Ok(service) => {
            let page = service.inspect(&history_request);
            // The journal groups the whole visible history (bounded), so its
            // newest entries are never cut off by paging.
            let all = service.all_decision_items(&history_request);
            (page, all)
        }
        Err(error) => (Err(error), None),
    };
    let progress = match &records {
        Some(Ok(records)) => progress(&layout, Some(records)),
        _ => progress(&layout, None),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "dash",
        ServiceState::Success,
        "Read-only project shape and decision history are ready.",
    );
    response.permitted_actions = vec!["wh change".into(), "wh check".into()];
    let (history, history_state, history_detail) = match history {
        Ok(history) => (Some(history), "available", None),
        Err(error) if !store_exists => (None, "not_initialized", Some(error.to_string())),
        Err(error @ HistoryError::StaleSnapshot { .. }) => {
            response.state = ServiceState::Stale;
            response.summary =
                "Decision history changed; refresh before continuing this exact view.".into();
            (None, "stale", Some(error.to_string()))
        }
        Err(error) => {
            response.state = ServiceState::Unknown;
            response.summary =
                "Decision history is unavailable; no current-state claim was inferred.".into();
            (None, "unavailable", Some(error.to_string()))
        }
    };
    let agreement_state = match records {
        Some(Ok(records)) => Ok(AgreementState::from_records(records)),
        Some(Err(error)) => Err(error),
        None => Ok(AgreementState::default()),
    };
    let (current, journal, trail) = match agreement_state {
        Ok(state) => {
            let mut view = projection::dashboard_view(
                &state,
                &projection::ProjectionInput {
                    project_label: project_label(layout.project_root()),
                    agreement_complete: progress.agreement_complete,
                    onboarding: progress.onboarding.clone(),
                    shared: shared_refs(loaded.as_ref().ok()),
                },
            );
            let journal = match &journal_items {
                Some((items, _)) => projection::search_journal(
                    projection::journal_from_items(&state, items),
                    request.search.as_deref(),
                ),
                None => projection::journal(&state, history.as_ref()),
            };
            view.latest_change =
                journal
                    .iter()
                    .find(|entry| entry.kind == "decision")
                    .map(|entry| projection::LatestChange {
                        title: entry.title.clone(),
                        at: entry.recorded_at.clone(),
                    });
            let trail = request.trail.then(|| projection::decision_trail(&state));
            let view = serde_json::to_value(&view).unwrap_or(Value::Null);
            (Some(view), journal, trail)
        }
        Err(_error) => {
            response.state = ServiceState::Unknown;
            response.summary =
                "The current agreement projection is unavailable; no state was inferred.".into();
            response.evidence.push(ServiceEvidence {
                kind: "current_projection_unavailable".into(),
                locator: layout.private_store().display().to_string(),
                digest: None,
            });
            (None, Vec::new(), None)
        }
    };
    response.blocking_questions = progress
        .missing_decisions
        .iter()
        .map(|decision| format!("What should the project's {decision} be?"))
        .collect();
    let changelog_truncated = journal_items
        .as_ref()
        .is_some_and(|(_, truncated)| *truncated);
    let shared_exists = matches!(&loaded, Ok(loaded) if loaded.shared.is_some());
    let sync = json!({
        "private_store": loaded.as_ref().is_ok_and(|loaded| loaded.private.is_some()),
        "shared_store": layout.shared_store().map(|dir| dir.display().to_string()),
        "shared_records": loaded.as_ref().ok().and_then(|loaded| loaded.shared.as_ref()).map_or(0, Vec::len),
        "conflicts": sync_conflicts,
        "shared_refs": shared_refs(loaded.as_ref().ok()),
        "required_workflow": layout.project_root().join(crate::wire::CI_WORKFLOW).is_file(),
    });
    response.data = json!({
        "progress": progress,
        "progress_state": "available",
        "history": history,
        "history_state": history_state,
        "history_detail": history_detail,
        "changelog": journal,
        "changelog_truncated": changelog_truncated,
        "current": current,
        "store": store_section(&layout),
        "workflows": [
            {"name": "init", "state": "available", "effect": "inspect, agree the mission, principles and rules, wire the gates"},
            {"name": "dash", "state": "available", "effect": "inspect the rules and their history"},
            {"name": "change", "state": "available", "effect": "draft a change, accept or withdraw it, or retire a record"},
            {"name": "check", "state": "available", "effect": "run the rules and return repair feedback"},
            {"name": "pull", "state": if shared_exists { "available" } else { "unavailable" }, "effect": "receive shared records; nothing runs or is accepted"},
            {"name": "push", "state": if shared_exists { "available" } else { "unavailable" }, "effect": "share an exact, confirmed package of accepted records"}
        ],
        "sync": sync,
        "read_only": true
    });
    if request.trail {
        response.data["trail"] =
            json!(trail.unwrap_or_else(|| format!("{}\n", projection::TRAIL_HEADER)));
    }
    response
}

/// Which store holds the records, how the switch was read, where Virgil is
/// attached, and any records left in the store the switch no longer names.
fn store_section(layout: &ProjectLayout) -> Value {
    let switch = crate::storage::read_switch(layout.project_root());
    let mut section = match &switch {
        Ok(switch) => json!({
            "kind": switch.config.backend(),
            "source": switch.source,
            "switch": crate::storage::STORE_CONFIG,
        }),
        Err(error) => {
            return json!({"kind": null, "state": "invalid", "detail": error, "switch": crate::storage::STORE_CONFIG})
        }
    };
    let virgil = switch
        .as_ref()
        .is_ok_and(|switch| switch.config == crate::storage::StoreConfig::Virgil);
    if virgil {
        section["attachment"] = match crate::store::virgil::attachment(layout.project_root()) {
            Ok(Some((group, repository))) => json!({"group": group, "repository": repository}),
            Ok(None) => json!(null),
            Err(error) => json!({"state": "unavailable", "detail": error.to_string()}),
        };
    }
    let left = crate::store::export::left_behind(layout);
    section["left_behind"] = json!(left);
    section
}
