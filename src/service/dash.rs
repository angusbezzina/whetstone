//! `wh dash`: the read-only project view the dashboard and `--json` share,
//! from the rulebook file in the working tree, with the changelog and the
//! decision trail (`--trail`) from the file's Git log.

use serde_json::{json, Value};

use crate::layout::ProjectLayout;
use crate::projection;
use crate::rulebook::{self, Revision, RulebookError, RULEBOOK_PATH};

use super::change::{changes, committed};
use super::{
    bounded_request_id, decision_trail, project_error, project_label, rulebook_log,
    unknown_response, utc_now, DashRequest, RulebookCommit, ServiceEvidence, ServiceResponse,
    ServiceState, TRAIL_HEADER,
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
    let root = layout.project_root();
    let as_of = request.as_of.unwrap_or_else(utc_now);
    let page_size = request.page_size.clamp(1, 200);
    let log = rulebook_log(root);
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
    let book = match rulebook::load(root) {
        Ok(book) => Ok(Some(book)),
        Err(RulebookError::Missing) => Ok(None),
        Err(error) => Err(error),
    };
    let current = match &book {
        Ok(book) => {
            let head = committed(root).and_then(Result::ok);
            let drafts = match book {
                Some(book) if book.revision == Revision::Uncommitted => {
                    changes(head.as_ref(), book)
                        .into_iter()
                        .map(|(edit, detail)| format!("{edit}{detail}"))
                        .collect()
                }
                _ => Vec::new(),
            };
            let mut view = projection::dashboard_view(
                book.as_ref(),
                projection::ProjectionInput {
                    project_label: project_label(root),
                    head: head.as_ref(),
                    drafts,
                    hooks: crate::wire::git_hooks(root),
                    starters: crate::starter::starters(root),
                },
            );
            view.latest_change = changelog.first().map(|entry| projection::LatestChange {
                title: entry["title"].as_str().unwrap_or_default().to_string(),
                at: entry["recorded_at"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            });
            response.blocking_questions = view
                .onboarding
                .missing
                .iter()
                .map(|decision| format!("What should the project's {decision} be?"))
                .collect();
            Some(serde_json::to_value(&view).unwrap_or(Value::Null))
        }
        Err(error) => {
            response.state = ServiceState::Unknown;
            response.summary = format!("{error}; no current state was inferred.");
            response.permitted_actions = vec![format!("repair {RULEBOOK_PATH}, then retry")];
            response.evidence.push(ServiceEvidence {
                kind: "rulebook_unreadable".into(),
                locator: RULEBOOK_PATH.into(),
                digest: None,
            });
            None
        }
    };
    response.data = json!({
        "rulebook": {
            "path": RULEBOOK_PATH,
            "state": match &book {
                Ok(Some(_)) => "available",
                Ok(None) => "missing",
                Err(_) => "invalid",
            },
            "revision": book.as_ref().ok().and_then(Option::as_ref).map(|book| book.revision.label().to_string()),
            "detail": book.as_ref().err().map(ToString::to_string),
        },
        "changelog": changelog,
        "changelog_truncated": changelog_truncated,
        "current": current,
        "workflows": [
            {"name": "init", "state": "available", "effect": "inspect the project and wire the Git gates"},
            {"name": "dash", "state": "available", "effect": "inspect the rules and their history"},
            {"name": "change", "state": "available", "effect": "draft an edit to the rulebook, accept or withdraw it, or retire a rule"},
            {"name": "check", "state": "available", "effect": "run the rules and return repair feedback"},
            {"name": "explain", "state": "available", "effect": "say why a rule or a finding exists and who accepted it"}
        ],
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
                "short": &commit.commit[..12.min(commit.commit.len())],
                "recorded_at": commit.date,
                "title": commit.subject,
                "summary": commit.body,
                "owner": commit.author,
            })
        })
        .collect();
    (entries, matching.len() > page_size)
}
