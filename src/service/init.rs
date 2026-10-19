//! `wh init`: from mission to agreed rules in five minutes.
//!
//! Inspect is read-only: where onboarding stands, from the rulebook file and
//! the Git hooks. Agree drafts the owner's mission, the principles they
//! picked from the catalogue and the starter rules they accepted into
//! `whetstone/rulebook.json`, as `wh change` does; `wh change --accept`
//! commits it. Wire and setup live in `crate::wire`.

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::domain::{Mission, Principle, PrincipleSource, RecordId};
use crate::layout::ProjectLayout;
use crate::projection::{self, Start};
use crate::rulebook::{
    self, PrincipleEntry, Revision, RuleEntry, Rulebook, RulebookError, RULEBOOK_PATH,
};

use super::change::{committed, draft, rulebook_unreadable};
use super::{
    bounded_input, bounded_request_id, project_error, project_label, unknown_response, InitAction,
    InitRequest, ServiceResponse, ServiceState,
};

/// Where onboarding stands, from the rulebook in the working tree.
#[derive(Debug, Clone, Serialize)]
struct Progress {
    /// The commit that last touched the rulebook, `uncommitted`, or none.
    revision: Option<String>,
    /// A mission and at least one rule are in the rulebook.
    agreement_complete: bool,
    /// The agreement is committed and the Git gates are installed.
    setup_complete: bool,
    missing_decisions: Vec<String>,
    hooks: crate::wire::GitHooks,
    #[serde(skip)]
    onboarding: Start,
}

fn progress(layout: &ProjectLayout, book: Option<&Rulebook>) -> Progress {
    let root = layout.project_root();
    let head = committed(root).and_then(Result::ok);
    let hooks = crate::wire::git_hooks(root);
    let view = projection::dashboard_view(
        book,
        projection::ProjectionInput {
            project_label: project_label(root),
            head: head.as_ref(),
            drafts: Vec::new(),
            hooks: hooks.clone(),
            starters: crate::starter::starters(root),
        },
    );
    let committed = book.is_some_and(|book| book.revision != Revision::Uncommitted);
    Progress {
        revision: book.map(|book| book.revision.label().to_string()),
        agreement_complete: view.established,
        setup_complete: view.established && committed && hooks.pre_commit && hooks.pre_push,
        missing_decisions: view.onboarding.missing.clone(),
        hooks,
        onboarding: view.onboarding,
    }
}

/// The rulebook in the working tree, `None` when there is none yet.
fn load(root: &std::path::Path) -> Result<Option<Rulebook>, RulebookError> {
    match rulebook::load(root) {
        Ok(book) => Ok(Some(book)),
        Err(RulebookError::Missing) => Ok(None),
        Err(error) => Err(error),
    }
}

pub(super) fn init(request: InitRequest) -> ServiceResponse {
    let layout = match ProjectLayout::resolve(&request.project_dir) {
        Ok(layout) => layout,
        Err(error) => return project_error("init", request.request_id, error),
    };
    let request_id = match bounded_request_id(
        request.request_id.clone(),
        format!("init-{}", &layout.project_id()[..16]),
    ) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response("init", "invalid-request-id".into(), summary),
    };
    match request.action {
        InitAction::Wire => return crate::wire::wire(&layout, request_id, &request),
        InitAction::Setup => {
            let wiring = InitRequest {
                hooks: true,
                ci: true,
                ..request.clone()
            };
            return crate::wire::wire(&layout, request_id, &wiring);
        }
        _ => {}
    }
    let book = match load(layout.project_root()) {
        Ok(book) => book,
        Err(error) => {
            let mut response = rulebook_unreadable(request_id, &error);
            response.workflow = "init".into();
            return response;
        }
    };
    match request.action {
        InitAction::Cancel => {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::Success,
                "Onboarding was cancelled without changing project files, integrations or remotes.",
            );
            response.permitted_actions = vec!["wh init".into()];
            response.data = json!({
                "progress": progress(&layout, book.as_ref()),
                "cancelled": true,
                "writes": [],
            });
            response
        }
        InitAction::Inspect => inspection_response(request_id, progress(&layout, book.as_ref())),
        _ => agree(&layout, request_id, &request, book),
    }
}

fn inspection_response(request_id: String, progress: Progress) -> ServiceResponse {
    let (state, summary) = if progress.setup_complete {
        (
            ServiceState::Success,
            "The mission and rules are agreed and committed, and the Git gates are installed.",
        )
    } else if progress.agreement_complete {
        (
            ServiceState::NeedsInput,
            "The mission and rules are in the rulebook. Accept them (wh change --accept), install the gates (wh init --action wire --hooks) and run wh check to finish setup.",
        )
    } else {
        (
            ServiceState::NeedsInput,
            "Inspection is complete and read-only. State the mission, pick principles and accept starter rules.",
        )
    };
    let mut response = ServiceResponse::new(request_id, "init", state, summary);
    response.blocking_questions = progress
        .missing_decisions
        .iter()
        .map(|decision| match decision.as_str() {
            "mission" => "What is the project's mission, in one sentence?".to_string(),
            _ => "Which starter rules do you accept (or which rule of your own)?".to_string(),
        })
        .collect();
    response.permitted_actions = if progress.agreement_complete {
        vec![
            "wh change --accept".into(),
            "wh init --action wire --hooks".into(),
            "wh check".into(),
        ]
    } else {
        vec![
            "wh init --action agree --mission <text> --principle <id> --starter all".into(),
            "wh init --action cancel".into(),
        ]
    };
    response.data = json!({
        "progress": progress,
        "onboarding": progress.onboarding,
        "inspection_writes": [],
        "read_only": true,
    });
    response
}

fn ask(request_id: String, question: String) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "init",
        ServiceState::NeedsInput,
        "More owner input is required; the rulebook was not changed.",
    );
    response.blocking_questions = vec![question];
    response.permitted_actions =
        vec!["wh init --action agree --mission <text> --principle <id> --starter all".into()];
    response
}

/// Draft the mission, principles and starter rules the request names into
/// the rulebook, leaving out what it already holds.
fn agree(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
    book: Option<Rulebook>,
) -> ServiceResponse {
    let root = layout.project_root();
    let mut after = book.unwrap_or_else(|| Rulebook {
        mission: Mission {
            statement: String::new(),
        },
        principles: Vec::new(),
        rules: Vec::new(),
        retired: Vec::new(),
        revision: Revision::Uncommitted,
    });
    let mut drafted = Vec::new();
    let current = after.mission.statement.trim().to_string();
    match &request.mission {
        // Resending the mission already there (a form pre-filled after a
        // partial agreement) is not a change: draft the rest.
        Some(text) if !current.is_empty() && text.trim() == current => {}
        Some(_) if !current.is_empty() => {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::NeedsDecision,
                "The rulebook already has a mission; change it with wh change --kind mission.",
            );
            response.permitted_actions = vec!["wh change --kind mission --content <text>".into()];
            return response;
        }
        Some(text) => match bounded_input(Some(text.clone()), "mission", 500) {
            Ok(text) => {
                after.mission.statement = text;
                drafted.push("mission".to_string());
            }
            Err(question) => return ask(request_id, question),
        },
        None if current.is_empty() => {
            return ask(
                request_id,
                "What is the project's mission, in one sentence? (--mission)".into(),
            )
        }
        None => {}
    }
    let catalogue = crate::catalogue::catalogue();
    let has = |book: &Rulebook, id: &RecordId| {
        book.principles.iter().any(|entry| entry.id == *id)
            || book.rules.iter().any(|entry| entry.id == *id)
    };
    let mut principles = Vec::new();
    for id in &request.principles {
        let Some(entry) = catalogue.get(id) else {
            return ask(
                request_id,
                format!(
                    "{id} is not a principle in the catalogue (version {}). Known ids include prove-it-works, laziness-protocol and boundary-discipline.",
                    catalogue.version
                ),
            );
        };
        principles.push((
            format!("principle.{id}"),
            Principle {
                statement: entry.rule.clone(),
                source: PrincipleSource::Pstack {
                    id: id.clone(),
                    version: catalogue.version.clone(),
                },
                rationale: None,
            },
        ));
    }
    for text in &request.custom_principles {
        let statement = match bounded_input(Some(text.clone()), "principle", 1_000) {
            Ok(statement) => statement,
            Err(question) => return ask(request_id, question),
        };
        let slug = format!("{:x}", Sha256::digest(statement.as_bytes()));
        principles.push((
            format!("principle.custom-{}", &slug[..12]),
            Principle {
                statement,
                source: PrincipleSource::Custom,
                rationale: None,
            },
        ));
    }
    for (id, principle) in principles {
        let id = match RecordId::new(id) {
            Ok(id) => id,
            Err(error) => return ask(request_id, error.to_string()),
        };
        if has(&after, &id) {
            continue;
        }
        drafted.push(id.as_str().to_string());
        after.retired.retain(|entry| entry.id != id);
        after.principles.push(PrincipleEntry { id, principle });
    }
    let starters = crate::starter::starters(root);
    let wanted = |id: &str| {
        request
            .starters
            .iter()
            .any(|wanted| wanted == "all" || wanted == id || format!("rule.{wanted}") == id)
    };
    if let Some(unknown) = request.starters.iter().find(|wanted| {
        *wanted != "all"
            && !starters.iter().any(|starter| {
                starter.id == wanted.as_str() || starter.id == format!("rule.{wanted}")
            })
    }) {
        return ask(
            request_id,
            format!(
                "{unknown} is not a starter rule; choose from {}.",
                starters
                    .iter()
                    .map(|starter| starter.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }
    for starter in starters.into_iter().filter(|starter| wanted(starter.id)) {
        let entry = RecordId::new(starter.id)
            .map_err(|error| error.to_string())
            .and_then(|id| RuleEntry::new(id, starter.rule).map_err(|error| error.to_string()));
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => return unknown_response("init", request_id, error),
        };
        if has(&after, &entry.id) {
            continue;
        }
        drafted.push(entry.id.as_str().to_string());
        after.retired.retain(|retired| retired.id != entry.id);
        after.rules.push(entry);
    }
    if drafted.is_empty() {
        return ask(
            request_id,
            "Nothing new to draft: add a principle (--principle <id>), your own (--custom-principle <text>) or a starter rule (--starter all).".into(),
        );
    }
    let (written, mut response) = match draft(
        root,
        request_id,
        request.dry_run,
        &after,
        format!("Dry run: {RULEBOOK_PATH} would hold these exact entries; nothing was written."),
        format!("{RULEBOOK_PATH} now holds the agreement as a draft. Accept it with wh change --accept (one commit), then install the gates with wh init --action wire --hooks."),
    ) {
        Ok(drafted) => drafted,
        Err(mut response) => {
            response.workflow = "init".into();
            return *response;
        }
    };
    response.workflow = "init".into();
    if !request.dry_run {
        response.permitted_actions = vec![
            "wh change --accept".into(),
            "wh init --action wire --hooks".into(),
            "wh check".into(),
        ];
    }
    let progress = progress(layout, Some(&written));
    response.data = json!({
        "drafted": drafted,
        "entries": drafted
            .iter()
            .map(|id| rulebook::entry(&written, id))
            .collect::<Vec<_>>(),
        "path": RULEBOOK_PATH,
        "dry_run": request.dry_run,
        "written": !request.dry_run,
        "progress": progress,
        "onboarding": progress.onboarding,
    });
    response
}
