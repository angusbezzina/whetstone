//! `wh dash`: the read-only project view the dashboard and `--json` share,
//! and the decision trail (`--trail`).

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{Enforcer, VerificationAxis};
use crate::history::{HistoryError, HistoryInspectionRequest, HistoryInspectionService};
use crate::onboarding;
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
    let setup = match onboarding::inspect(layout.project_root()) {
        Ok(value) => value,
        Err(error) => {
            return unknown_response(
                "dash",
                request_id,
                format!("Project inspection failed without changing state: {error:?}"),
            )
        }
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
    let hands = crate::hands::statuses(layout.project_root());
    let (current, journal, trail) = match agreement_state {
        Ok(state) => {
            let fingerprint = crate::gates::workspace_fingerprint(layout.project_root()).ok();
            let evidence_root = crate::gates::evidence_root(&layout);
            let manifest = crate::skill::read_manifest(&layout);
            let mut view = projection::dashboard_view(
                &state,
                &projection::ProjectionInput {
                    project_label: project_label(layout.project_root()),
                    project_root: layout.project_root(),
                    evidence_root: &evidence_root,
                    agreement_complete: progress.agreement_complete,
                    onboarding: progress.onboarding.clone(),
                    fingerprint: fingerprint.as_deref(),
                    driver_path: crate::gates::driver_path(layout.project_root()),
                    skill: manifest.clone(),
                    changes: crate::proof::changes_for(&state, layout.project_root()),
                    hygiene: crate::hygiene::check(
                        &state,
                        layout.project_root(),
                        manifest.as_ref(),
                    ),
                    shared: shared_refs(loaded.as_ref().ok()),
                    hands,
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
        "required_workflow": layout.project_root().join(crate::skill::CI_WORKFLOW).is_file(),
    });
    response.data = json!({
        "setup": setup,
        "progress": progress,
        "progress_state": "available",
        "history": history,
        "history_state": history_state,
        "history_detail": history_detail,
        "changelog": journal,
        "changelog_truncated": changelog_truncated,
        "current": current,
        "workflows": [
            {"name": "init", "state": "available", "effect": "inspect, agree the mission, principles and rules, wire the gates"},
            {"name": "dash", "state": "available", "effect": "inspect rules, checks, requests and history"},
            {"name": "change", "state": "available", "effect": "draft a change, accept or withdraw it, label a flag, answer a hand"},
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

/// Whether an area is the closing multi-surface journeys group.
fn is_journey(area: &str) -> bool {
    area.to_ascii_lowercase().contains("journey")
}

/// Per-feature sweep outcomes in feature-map order (journeys last): proven,
/// failed at a named step, unreachable with its prerequisite, or skipped
/// with the reason. Map hygiene findings travel with it.
pub(crate) fn sweep_report(
    agreement: Option<&AgreementState>,
    selection: &super::check::Selection,
    outcomes: &[crate::gates::GateOutcome],
    layout: Option<&ProjectLayout>,
) -> Value {
    let mut features = selection.features.iter().collect::<Vec<_>>();
    features.sort_by(|left, right| {
        (
            is_journey(&left.1.area),
            left.1.area.as_str(),
            left.1.sweep_order,
            left.1.name.as_str(),
        )
            .cmp(&(
                is_journey(&right.1.area),
                right.1.area.as_str(),
                right.1.sweep_order,
                right.1.name.as_str(),
            ))
    });
    let mut tally = std::collections::BTreeMap::from([
        ("proven", 0_u64),
        ("failed", 0),
        ("unreachable", 0),
        ("skipped", 0),
        ("unknown", 0),
    ]);
    let mut rows = Vec::new();
    for (id, feature) in features {
        let ran = selection
            .rules
            .iter()
            .zip(outcomes)
            .find(|(selected, _)| matches!(&selected.rule.enforcer, Enforcer::Drive { feature } if feature == id));
        let (outcome, detail, step, gate, evidence) = match ran {
            Some((gate, outcome)) => {
                let evidence = outcome
                    .evidence
                    .iter()
                    .map(|evidence| evidence.locator.clone())
                    .collect::<Vec<_>>();
                let first = outcome.failures.first();
                let (kind, detail, step) = match outcome.state {
                    VerificationAxis::Pass => ("proven", outcome.summary.clone(), None),
                    VerificationAxis::Fail => (
                        "failed",
                        first.map_or_else(
                            || outcome.summary.clone(),
                            |failure| failure.message.clone(),
                        ),
                        first.map(|failure| failure.location.clone()),
                    ),
                    VerificationAxis::Unknown => {
                        if let Some(rest) = outcome
                            .summary
                            .strip_prefix(crate::gates::UNREACHABLE_PREFIX)
                        {
                            ("unreachable", rest.to_string(), None)
                        } else if outcome.summary.starts_with("Skipped:") {
                            ("skipped", outcome.summary.clone(), None)
                        } else {
                            ("unknown", outcome.summary.clone(), None)
                        }
                    }
                };
                (
                    kind,
                    detail,
                    step,
                    Some(gate.id.as_str().to_string()),
                    evidence,
                )
            }
            None => (
                "skipped",
                if feature.drive_steps.is_empty() {
                    "no drive steps are recorded, so it cannot be driven".to_string()
                } else {
                    "no accepted drive rule proves it".to_string()
                },
                None,
                None,
                Vec::new(),
            ),
        };
        *tally.entry(outcome).or_insert(0) += 1;
        rows.push(json!({
            "feature": id.as_str(),
            "name": feature.name,
            "area": feature.area,
            "outcome": outcome,
            "detail": detail,
            "step": step,
            "gate": gate,
            "evidence": evidence,
        }));
    }
    let hygiene = match (agreement, layout) {
        (Some(state), Some(layout)) => crate::hygiene::check(
            state,
            layout.project_root(),
            crate::skill::read_manifest(layout).as_ref(),
        ),
        _ => Vec::new(),
    };
    json!({
        "order": rows.iter().map(|row| row["feature"].clone()).collect::<Vec<_>>(),
        "features": rows,
        "tally": tally,
        "hygiene": hygiene,
        "edits_product_code": false,
        "edits_map_or_driver": false,
    })
}
