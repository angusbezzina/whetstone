//! `wh dash`: the read-only project view the dashboard and `--json` share,
//! the changelog and the decision trail (`--trail`), both from the Git log
//! of the rulebook file.

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::projection;
use crate::storage::{ProjectLayout, StorageError};

use super::init::progress;
use super::{
    bounded_request_id, decision_trail, load_records, project_error, project_label, rulebook_log,
    shared_refs, unknown_response, utc_now, DashRequest, RulebookCommit, ServiceEvidence,
    ServiceResponse, ServiceState, TRAIL_HEADER,
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
    // One read of each store serves progress and projection.
    let loaded = load_records(&layout);
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
    let log = rulebook_log(layout.project_root());
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
    let (changelog, changelog_truncated) = match &log {
        Ok(log) if valid_instant(&as_of) => {
            changelog(log, &as_of, request.search.as_deref(), page_size)
        }
        Ok(_) => {
            response.state = ServiceState::Unknown;
            response.summary = format!("{as_of} is not an RFC 3339 UTC instant (YYYY-MM-DDTHH:MM:SSZ); no changelog was inferred.");
            (Vec::new(), false)
        }
        Err(error) => {
            response.state = ServiceState::Unknown;
            response.summary = format!(
                "The rulebook's Git log is unavailable ({error}); no changelog was inferred."
            );
            (Vec::new(), false)
        }
    };
    let agreement_state = match records {
        Some(Ok(records)) => Ok(AgreementState::from_records(records)),
        Some(Err(error)) => Err(error),
        None => Ok(AgreementState::default()),
    };
    let current = match agreement_state {
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
            view.latest_change = changelog.first().map(|entry| projection::LatestChange {
                title: entry["title"].as_str().unwrap_or_default().to_string(),
                at: entry["recorded_at"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            });
            let view = serde_json::to_value(&view).unwrap_or(Value::Null);
            Some(view)
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
            None
        }
    };
    response.blocking_questions = progress
        .missing_decisions
        .iter()
        .map(|decision| format!("What should the project's {decision} be?"))
        .collect();
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
        "changelog": changelog,
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
        response.data["trail"] = json!(match &log {
            Ok(log) => decision_trail(log),
            Err(_) => format!("{TRAIL_HEADER}\n"),
        });
    }
    response
}

/// `YYYY-MM-DDTHH:MM:SSZ`, the form Git's log dates are written in here.
fn valid_instant(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}

/// The changelog, newest first: the commits that touched the rulebook at or
/// before `as_of`, matching `search` in their subject, body, author or
/// commit, at most `page_size` of them (`true` when more were left out).
fn changelog(
    log: &[RulebookCommit],
    as_of: &str,
    search: Option<&str>,
    page_size: usize,
) -> (Vec<Value>, bool) {
    let term = search
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase);
    let matching = log
        .iter()
        .filter(|commit| commit.date.as_str() <= as_of)
        .filter(|commit| {
            term.as_ref().map_or(true, |term| {
                [
                    &commit.subject,
                    &commit.body,
                    &commit.author,
                    &commit.commit,
                ]
                .iter()
                .any(|text| text.to_lowercase().contains(term))
            })
        })
        .collect::<Vec<_>>();
    let entries = matching
        .iter()
        .take(page_size)
        .map(|commit| {
            json!({
                "id": commit.commit,
                "commit": commit.commit,
                "recorded_at": commit.date,
                "kind": "decision",
                "title": commit.subject,
                "version": &commit.commit[..12.min(commit.commit.len())],
                "status": projection::StateLabel::new("pass", "committed"),
                "owner": commit.author,
                "area": "rules",
                "summary": commit.body,
                "note": null,
                "proposal": null,
                "records": [],
            })
        })
        .collect();
    (entries, matching.len() > page_size)
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
