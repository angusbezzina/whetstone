//! `wh dash`: the read-only project view the dashboard and `--json` share,
//! from the rulebook file in the working tree, with the changelog and the
//! decision trail (`--trail`) from the file's Git log.

use serde_json::{json, Value};

use crate::layout::ProjectLayout;
use crate::projection::{diff, page};
use crate::rulebook::{self, Revision, RulebookError, RULEBOOK_PATH};

use super::change::{changes, committed, message};
use super::{
    bounded_request_id, decision_trail, project_error, project_label, rulebook_log, rulebook_texts,
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
    let history = match &log {
        Ok(log) => history(root, log),
        Err(_) => Vec::new(),
    };
    let current = match &book {
        Ok(book) => {
            let head = committed(root).and_then(Result::ok);
            let message = match book {
                Some(book) if book.revision == Revision::Uncommitted => {
                    let edits = changes(head.as_ref(), book);
                    (!edits.is_empty()).then(|| message(&edits, None))
                }
                _ => None,
            };
            let book_json = std::fs::read_to_string(root.join(RULEBOOK_PATH))
                .ok()
                .and_then(|text| serde_json::from_str::<Value>(&text).ok());
            let head_json = match &log {
                Ok(log) if !log.is_empty() => rulebook_texts(root, &["HEAD"])
                    .ok()
                    .and_then(|texts| texts.into_iter().next().flatten())
                    .and_then(|text| serde_json::from_str::<Value>(&text).ok()),
                _ => None,
            };
            if !book
                .as_ref()
                .is_some_and(|book| !book.mission.statement.trim().is_empty())
            {
                response
                    .blocking_questions
                    .push("What should the project's mission be?".into());
            }
            if !book.as_ref().is_some_and(|book| !book.rules.is_empty()) {
                response
                    .blocking_questions
                    .push("What should the project's rules be?".into());
            }
            let coverage = book
                .as_ref()
                .map(|book| crate::coverage::coverage(root, book));
            let view = page::page_view(
                book.as_ref(),
                page::PageInput {
                    project_label: project_label(root),
                    branch: branch(root),
                    head: head.as_ref(),
                    head_json: head_json.as_ref(),
                    book_json: book_json.as_ref(),
                    message,
                    hooks: crate::wire::git_hooks(root),
                    ci_wired: crate::wire::ci_wired(root),
                    coverage: coverage.clone(),
                    starters: crate::starter::starters(root),
                    catalogue: crate::catalogue::catalogue(root),
                    history,
                },
            );
            let mut value = serde_json::to_value(&view).unwrap_or(Value::Null);
            // The full coverage stays in the envelope for agents; the page
            // shows only its problems, in the health strip.
            if let (Some(coverage), Some(object)) = (coverage, value.as_object_mut()) {
                object.insert("coverage".into(), coverage);
            }
            Some(value)
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

/// Every commit that touched the rulebook, newest first, with what it
/// changed against the version before it.
fn history(root: &std::path::Path, log: &[RulebookCommit]) -> Vec<page::HistoryEntry> {
    let commits = log
        .iter()
        .map(|commit| commit.commit.as_str())
        .collect::<Vec<_>>();
    let versions = rulebook_texts(root, &commits)
        .unwrap_or_else(|_| vec![None; commits.len()])
        .into_iter()
        .map(|text| text.and_then(|text| serde_json::from_str::<Value>(&text).ok()))
        .collect::<Vec<_>>();
    log.iter()
        .enumerate()
        .map(|(index, commit)| {
            let after = versions[index].as_ref();
            let before = versions.get(index + 1).and_then(Option::as_ref);
            let changes = after
                .map(|after| diff::diff(before, after))
                .unwrap_or_default();
            page::HistoryEntry {
                commit: commit.commit.clone(),
                short: commit.commit[..12.min(commit.commit.len())].to_string(),
                author: commit.author.clone(),
                at: commit.date.clone(),
                subject: commit.subject.clone(),
                body: commit.body.clone(),
                summary: match after {
                    Some(_) => diff::summary(&changes),
                    None => format!("{RULEBOOK_PATH} was removed or is not JSON at this commit"),
                },
                changes,
                readable: after.is_some(),
            }
        })
        .collect()
}

/// The checked-out branch, `None` on a detached HEAD or outside Git.
fn branch(root: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !name.is_empty()).then_some(name)
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
