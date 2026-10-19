//! `wh init`: from mission to agreed rules in five minutes.
//!
//! Inspect is read-only. Agree records the owner's mission, the principles
//! they picked from the catalogue and the starter rules they accepted. Setup
//! detects and installs the record store's tool. Wire lives in `crate::wire`,
//! and the export to the rulebook file in `super::export`.

use serde::Serialize;
use serde_json::json;

use crate::agreement::AgreementState;
use crate::domain::{
    AgreementRecord, Mission, Principle, PrincipleSource, RecordBody, RecordId, RecordRef,
};
use crate::projection::{LegacyItem, Onboarding, OnboardingStep, StarterView};
use crate::storage::{AppendRequest, ProjectLayout, RecordStore, StorageError};

use super::record::agreement_record;
use super::{
    bounded_input, bounded_request_id, domain_response, idempotency_conflict, latest_record,
    needs_input, owner_name, project_error, resume_token, stale_response, storage_error,
    unknown_response, InitAction, InitRequest, ServiceResponse, ServiceState,
};

/// Where onboarding stands, computed from the records.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Progress {
    pub(crate) agreement_revision: u64,
    pub(crate) agreement_complete: bool,
    pub(crate) setup_complete: bool,
    pub(crate) missing_decisions: Vec<String>,
    pub(crate) agreement_records: Vec<RecordRef>,
    pub(crate) hooks: crate::wire::GitHooks,
    pub(crate) shared: bool,
    #[serde(skip)]
    pub(crate) onboarding: Onboarding,
}

pub(crate) fn progress(layout: &ProjectLayout, records: Option<&[AgreementRecord]>) -> Progress {
    let root = layout.project_root();
    let state = AgreementState::from_records(records.map(<[_]>::to_vec).unwrap_or_default());
    let mission = RecordId::new(crate::projection::MISSION_ID)
        .ok()
        .and_then(|id| state.in_force(&id));
    let principles = state.in_force_matching(|body| matches!(body, RecordBody::Principle(_)));
    let rules = state.in_force_matching(|body| body.rule_view().is_some());
    let hooks = crate::wire::git_hooks(root);
    let tools = crate::setup::detect(root);
    let mut missing = Vec::new();
    if mission.is_none() {
        missing.push("mission".to_string());
    }
    if rules.is_empty() {
        missing.push("rules".to_string());
    }
    let agreement_records = mission
        .into_iter()
        .chain(principles.iter().copied())
        .chain(rules.iter().copied())
        .filter_map(|record| record.reference().ok())
        .collect::<Vec<_>>();
    let agreement_revision = agreement_records
        .iter()
        .map(|reference| reference.revision)
        .max()
        .unwrap_or(0);
    let agreement_complete = missing.is_empty();
    let setup_complete = agreement_complete && hooks.pre_commit && hooks.pre_push;
    let catalogue = crate::catalogue::catalogue();
    let starters = crate::starter::starters(root)
        .into_iter()
        .map(|starter| {
            let (mechanism, _) = crate::gates::mechanism_label(&starter.rule);
            StarterView {
                accepted: RecordId::new(starter.id)
                    .ok()
                    .is_some_and(|id| state.in_force(&id).is_some()),
                id: starter.id.into(),
                title: starter.title.into(),
                detected: starter.detected,
                strength: starter.rule.strength.label(),
                enforcer: mechanism,
                examples: starter
                    .rule
                    .examples
                    .iter()
                    .map(|example| json!({"input": example.input, "expected": example.expected, "reason": example.reason}))
                    .collect(),
            }
        })
        .collect();
    let migrated = state
        .records()
        .iter()
        .filter_map(|record| match &record.body {
            RecordBody::Principle(Principle {
                source: PrincipleSource::Migrated { record },
                ..
            }) => Some(record.id.clone()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut legacy = std::collections::BTreeMap::<RecordId, LegacyItem>::new();
    for record in state.records() {
        if let RecordBody::Retired(retired) = &record.body {
            if let Some((text, _)) = retired.principle_text() {
                legacy.insert(
                    record.id.clone(),
                    LegacyItem {
                        id: record.id.as_str().into(),
                        kind: retired.record_type.replace('_', " "),
                        text,
                        migrated: migrated.contains(&record.id),
                    },
                );
            }
        }
    }
    let onboarding = Onboarding {
        steps: vec![
            OnboardingStep {
                key: "tools",
                label: "Tools",
                hint: "the record store's tool is installed, so records work",
                done: tools.ready,
                optional: true,
            },
            OnboardingStep {
                key: "mission",
                label: "Mission",
                hint: "what this project exists to do, in one sentence",
                done: mission.is_some(),
                optional: false,
            },
            OnboardingStep {
                key: "principles",
                label: "Principles",
                hint: "the catalogue principles you hold, or your own",
                done: !principles.is_empty(),
                optional: true,
            },
            OnboardingStep {
                key: "rules",
                label: "Rules",
                hint: "accept the starter rules and any proposed ones",
                done: !rules.is_empty(),
                optional: false,
            },
            OnboardingStep {
                key: "gates",
                label: "Gates",
                hint: "pre-commit and pre-push hooks that run the rules",
                done: hooks.pre_commit && hooks.pre_push,
                optional: false,
            },
        ],
        missing: missing.clone(),
        command: "wh init",
        catalogue_version: catalogue.version.clone(),
        catalogue: catalogue.principles,
        starters,
        legacy: legacy.into_values().collect(),
        tools: tools.tools,
    };
    Progress {
        agreement_revision,
        agreement_complete,
        setup_complete,
        missing_decisions: missing,
        agreement_records,
        hooks,
        shared: false,
        onboarding,
    }
}

pub(crate) fn progress_for(layout: &ProjectLayout) -> Result<Progress, StorageError> {
    let loaded = super::load_records(layout)?;
    if !loaded.exists() {
        return Ok(progress(layout, None));
    }
    let records = loaded.union().0;
    Ok(progress(layout, Some(&records)))
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
        InitAction::Setup => return setup(&layout, request_id, &request),
        InitAction::Store => return store_switch(&layout, request_id, &request),
        InitAction::Export => return super::export::export(&layout, request_id, &request),
        _ => {}
    }
    let progress = match progress_for(&layout) {
        Ok(progress) => progress,
        Err(error) if request.action == InitAction::Cancel => {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::Success,
                "Onboarding was cancelled without changing project files, records, integrations or remotes.",
            );
            response.data =
                json!({"cancelled": true, "writes": [], "progress_detail": error.to_string()});
            return response;
        }
        Err(error) => return storage_error("init", request_id, error),
    };
    let current_revision = progress.agreement_revision;
    let resume = resume_token(layout.project_id(), "init", &request_id, current_revision);
    match request.action {
        InitAction::Cancel => {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::Success,
                "Onboarding was cancelled without changing project files, records, integrations or remotes.",
            );
            response.expected_revision = Some(current_revision);
            response.permitted_actions = vec!["wh init".into()];
            response.data = json!({
                "progress": progress,
                "cancelled": true,
                "writes": [],
                "shared": false,
            });
            response
        }
        InitAction::Inspect => {
            let mut response = inspection_response(request_id, resume, progress);
            propose_store(&layout, &mut response);
            response
        }
        _ => agree(&layout, request_id, &request, progress, resume),
    }
}

/// Bare `wh init` names the record store, and when no switch is written yet
/// on a checkout attached to Virgil, proposes the virgil store first and the
/// beads store second.
fn propose_store(layout: &ProjectLayout, response: &mut ServiceResponse) {
    use crate::storage::{SwitchSource, STORE_CONFIG};
    let switch = crate::storage::read_switch(layout.project_root());
    let unchosen = switch
        .as_ref()
        .is_ok_and(|switch| switch.source != SwitchSource::Store);
    let proposed = if unchosen {
        proposed_stores(layout.project_root())
    } else {
        Vec::new()
    };
    if proposed.first() == Some(&"virgil") {
        response.summary.push_str(&format!(
            " This checkout is attached to Virgil and {STORE_CONFIG} names no store yet: choose wh init --action setup --store virgil (proposed) or --store beads."
        ));
        let mut actions = proposed
            .iter()
            .map(|store| format!("wh init --action setup --store {store}"))
            .collect::<Vec<_>>();
        actions.append(&mut response.permitted_actions);
        response.permitted_actions = actions;
    }
    response.data["store"] = json!({
        "kind": switch.as_ref().ok().map(|switch| switch.config.backend()),
        "switch": STORE_CONFIG,
        "chosen": !unchosen && switch.is_ok(),
        "proposed": proposed,
    });
}

fn inspection_response(request_id: String, resume: String, progress: Progress) -> ServiceResponse {
    let (state, summary) = if progress.setup_complete {
        (
            ServiceState::Success,
            "The mission and rules are agreed, the Git gates are installed, and a rule has passed on the current workspace.",
        )
    } else if progress.agreement_complete {
        (
            ServiceState::NeedsInput,
            "The mission and rules are agreed. Install the gates (wh init --action wire --hooks) and run wh check to finish setup.",
        )
    } else {
        (
            ServiceState::NeedsInput,
            "Inspection is complete and read-only. State the mission, pick principles and accept starter rules.",
        )
    };
    let mut response = ServiceResponse::new(request_id, "init", state, summary);
    response.expected_revision = Some(progress.agreement_revision);
    if state != ServiceState::Success {
        response.resume_token = Some(resume);
    }
    response.blocking_questions = progress
        .missing_decisions
        .iter()
        .map(|decision| match decision.as_str() {
            "mission" => "What is the project's mission, in one sentence?".to_string(),
            _ => "Which starter rules do you accept (or which rule of your own)?".to_string(),
        })
        .collect();
    response.permitted_actions = if progress.agreement_complete {
        vec!["wh init --action wire --hooks".into(), "wh check".into()]
    } else {
        vec![
            "wh init --action agree --mission <text> --principle <id> --starter all with the returned revision and token".into(),
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

fn agree(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
    progress: Progress,
    resume: String,
) -> ServiceResponse {
    let current_revision = progress.agreement_revision;
    // Read what exists without creating anything: a stale or invalid request
    // must leave no trace.
    let existing_store = crate::storage::open_private(layout).ok();
    let stored = match existing_store
        .as_deref()
        .map(RecordStore::all_records)
        .transpose()
    {
        Ok(records) => records.unwrap_or_default(),
        Err(error) => return storage_error("init", request_id, error),
    };
    let prefix = format!("{request_id}:base-");
    let existing = stored
        .iter()
        .filter(|record| record.idempotency_key.starts_with(&prefix))
        .cloned()
        .collect::<Vec<_>>();
    let retrying = !existing.is_empty();
    let target_revision = if retrying {
        existing
            .iter()
            .filter_map(|record| {
                record
                    .idempotency_key
                    .strip_prefix(&prefix)
                    .and_then(|tail| tail.split_once(':'))
                    .and_then(|(revision, _)| revision.parse::<u64>().ok())
            })
            .min()
            .unwrap_or(current_revision)
    } else {
        current_revision
    };
    let expected_resume = resume_token(layout.project_id(), "init", &request_id, target_revision);
    if request.expected_revision != Some(target_revision)
        || request.resume_token.as_deref() != Some(expected_resume.as_str())
    {
        return stale_response(
            "init",
            request_id,
            current_revision,
            resume,
            "The onboarding answer does not target the current project revision; inspect again and use the returned revision and token.",
        );
    }
    let owner = owner_name(layout.project_root());
    // What is in force apart from this request's own earlier writes: an
    // exact retry must not mistake its own mission for someone else's.
    let state = AgreementState::from_records(
        stored
            .iter()
            .filter(|record| !record.idempotency_key.starts_with(&prefix))
            .cloned()
            .collect(),
    );
    let current_mission = RecordId::new(crate::projection::MISSION_ID)
        .ok()
        .and_then(|id| state.in_force(&id))
        .and_then(|record| match &record.body {
            RecordBody::Mission(mission) => Some(mission.statement.clone()),
            _ => None,
        });
    let mission_in_force = current_mission.is_some();
    // Resending the mission already in force (a form pre-filled after a
    // partial agreement) is not a change: record the rest.
    let same_mission = request
        .mission
        .as_deref()
        .is_some_and(|text| current_mission.as_deref() == Some(text.trim()));
    let mission = match (&request.mission, mission_in_force) {
        (Some(_), true) if same_mission => None,
        (Some(text), false) => match bounded_input(Some(text.clone()), "mission", 500) {
            Ok(text) => Some(text),
            Err(question) => {
                return needs_input("init", request_id, current_revision, resume, question)
            }
        },
        (Some(_), true) => {
            let mut response = ServiceResponse::new(
                request_id,
                "init",
                ServiceState::NeedsDecision,
                "A mission is already in force; change it with wh change --kind mission.",
            );
            response.expected_revision = Some(current_revision);
            response.permitted_actions =
                vec!["wh change --kind mission --record-id mission.project".into()];
            return response;
        }
        (None, false) => {
            return needs_input(
                "init",
                request_id,
                current_revision,
                resume,
                "What is the project's mission, in one sentence? (--mission)".into(),
            )
        }
        (None, true) => None,
    };
    let catalogue = crate::catalogue::catalogue();
    let base = format!("{request_id}:base-{target_revision}");
    let mut records = Vec::new();
    if let Some(statement) = mission {
        records.push(agreement_record(
            layout,
            crate::projection::MISSION_ID,
            format!("{base}:mission"),
            RecordBody::Mission(Mission {
                statement,
                desired_outcomes: Vec::new(),
            }),
            owner.as_deref(),
        ));
    }
    for id in &request.principles {
        let Some(entry) = catalogue.get(id) else {
            return needs_input(
                "init",
                request_id,
                current_revision,
                resume,
                format!(
                    "{id} is not a principle in the catalogue (version {}). Known ids include prove-it-works, laziness-protocol and boundary-discipline.",
                    catalogue.version
                ),
            );
        };
        let record_id = format!("principle.{id}");
        if RecordId::new(record_id.as_str())
            .ok()
            .is_some_and(|record_id| state.in_force(&record_id).is_some())
        {
            continue;
        }
        records.push(agreement_record(
            layout,
            &record_id,
            format!("{base}:principle:{id}"),
            RecordBody::Principle(Principle {
                statement: entry.rule.clone(),
                source: PrincipleSource::Pstack {
                    id: id.clone(),
                    version: catalogue.version.clone(),
                },
                rationale: None,
            }),
            owner.as_deref(),
        ));
    }
    for text in &request.custom_principles {
        let statement = match bounded_input(Some(text.clone()), "principle", 1_000) {
            Ok(statement) => statement,
            Err(question) => {
                return needs_input("init", request_id, current_revision, resume, question)
            }
        };
        let slug = super::record::key_suffix(&statement, 12);
        records.push(agreement_record(
            layout,
            &format!("principle.custom-{slug}"),
            format!("{base}:principle:custom-{slug}"),
            RecordBody::Principle(Principle {
                statement,
                source: PrincipleSource::Custom,
                rationale: None,
            }),
            owner.as_deref(),
        ));
    }
    let starters = crate::starter::starters(layout.project_root());
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
        return needs_input(
            "init",
            request_id,
            current_revision,
            resume,
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
        if RecordId::new(starter.id)
            .ok()
            .is_some_and(|id| state.in_force(&id).is_some())
        {
            continue;
        }
        records.push(agreement_record(
            layout,
            starter.id,
            format!("{base}:rule:{}", starter.id),
            RecordBody::Rule(starter.rule),
            owner.as_deref(),
        ));
    }
    let mut records = match records.into_iter().collect::<Result<Vec<_>, _>>() {
        Ok(records) => records,
        Err(error) => return domain_response("init", request_id, error),
    };
    if records.is_empty() && !retrying {
        return needs_input(
            "init",
            request_id,
            current_revision,
            resume,
            "Nothing new to record: add a principle (--principle <id>), your own (--custom-principle <text>) or a starter rule (--starter all).".into(),
        );
    }
    for record in &mut records {
        let current = latest_record(&stored, record.id.as_str());
        record.revision = current.map_or(1, |current| current.revision + 1);
        record.supersedes = match current.map(AgreementRecord::reference) {
            Some(Ok(reference)) => Some(reference),
            Some(Err(error)) => return domain_response("init", request_id, error),
            None => None,
        };
    }
    // On a retry, records this request already wrote must match exactly and
    // are returned as they are; any it did not get to are written now.
    let mut references = Vec::new();
    let mut missing = Vec::new();
    for record in records {
        match existing
            .iter()
            .find(|existing| existing.idempotency_key == record.idempotency_key)
        {
            Some(found) if found.body == record.body => match found.reference() {
                Ok(reference) => references.push(reference),
                Err(error) => return domain_response("init", request_id, error),
            },
            Some(_) => return idempotency_conflict("init", request_id, &record.idempotency_key),
            None => missing.push(record),
        }
    }
    if retrying {
        // A retry that names fewer records than it first wrote still
        // returns what it wrote, never a second copy.
        for record in &existing {
            if !references
                .iter()
                .any(|reference: &RecordRef| reference.id == record.id)
            {
                if let Ok(reference) = record.reference() {
                    references.push(reference);
                }
            }
        }
    }
    if request.dry_run {
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            ServiceState::NeedsDecision,
            "Dry run: these exact records would be written; nothing was written.",
        );
        response.expected_revision = Some(current_revision);
        response.resume_token = Some(resume);
        response.permitted_actions =
            vec!["repeat the same request without --dry-run to record the agreement".into()];
        response.data = json!({
            "dry_run": true,
            "records": missing,
            "effects": {
                "private_store_would_be_created": !layout.private_store().exists(),
                "private_record_write_on_confirm": !missing.is_empty(),
                "team_share": false,
                "platform_configuration_writes": [],
            },
        });
        return response;
    }
    if !missing.is_empty() {
        if let Err(error) = crate::storage::prepare_shared(layout) {
            return storage_error("init", request_id, error);
        }
        let private = match crate::storage::initialize_private(layout) {
            Ok(store) => store,
            Err(error) => return storage_error("init", request_id, error),
        };
        let requests = missing
            .iter()
            .map(|record| AppendRequest {
                record,
                expected_revision: record.supersedes.as_ref().map(|prior| prior.revision),
            })
            .collect::<Vec<_>>();
        match private.append_batch(&requests) {
            Ok(written) => references.extend(written),
            Err(error) => return storage_error("init", request_id, error),
        }
    }
    let progress = match progress_for(layout) {
        Ok(value) => value,
        Err(error) => return storage_error("init", request_id, error),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "init",
        if progress.setup_complete {
            ServiceState::Success
        } else {
            ServiceState::NeedsInput
        },
        if progress.agreement_complete {
            "The agreement is recorded. Next: install the gates with wh init --action wire --hooks, then wh check."
        } else {
            "Recorded. The agreement needs a mission and at least one rule before anything is checked."
        },
    );
    response.expected_revision = Some(progress.agreement_revision);
    response.permitted_actions = vec!["wh init --action wire --hooks".into(), "wh check".into()];
    response.data = json!({
        "records": references,
        "progress": progress,
        "onboarding": progress.onboarding,
        "shared": false,
        "platform_configuration_writes": [],
    });
    response
}

/// Detect the record store's tool, then (with `--yes`) install Beads.
fn setup(layout: &ProjectLayout, request_id: String, request: &InitRequest) -> ServiceResponse {
    // `--store` chooses the record store first (and moves records into it),
    // so the tools detected next are the ones that store needs: under
    // Virgil, the virgil CLI and never bd. Its `--yes` confirms the store
    // alone: the installs it detects are listed, never run, because the
    // person reviewed only the move; `wh init --action setup --yes` runs
    // them.
    let switched = match request.store {
        Some(_) => {
            let switched = store_switch(layout, request_id.clone(), request);
            if switched.state != ServiceState::Success || request.dry_run {
                return switched;
            }
            Some(switched)
        }
        None => None,
    };
    let tools_request = InitRequest {
        yes: request.yes && switched.is_none(),
        ..request.clone()
    };
    let mut response = setup_tools(layout, request_id, &tools_request);
    if let Some(switched) = switched {
        response.summary = format!("{} {}", switched.summary, response.summary);
        response.data["store"] = switched.data;
    }
    response
}

fn setup_tools(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
) -> ServiceResponse {
    let root = layout.project_root();
    let detected = crate::setup::detect(root);
    let mut ran = Vec::new();
    let mut failed = Vec::new();
    if request.yes && !request.dry_run {
        for fix in detected
            .tools
            .iter()
            .filter(|tool| tool.tool == "bd")
            .filter_map(|tool| tool.fix.as_ref())
        {
            match crate::setup::run_install(root, fix) {
                Ok(_) => ran.push(fix.clone()),
                Err(error) => failed.push(json!({"command": fix, "error": error})),
            }
        }
    }
    let after = if ran.is_empty() {
        detected
    } else {
        crate::setup::detect(root)
    };
    let state = if after.ready && failed.is_empty() {
        ServiceState::Success
    } else if request.yes {
        ServiceState::Unavailable
    } else {
        ServiceState::NeedsDecision
    };
    let mut response = ServiceResponse::new(
        request_id,
        "init",
        state,
        if after.ready {
            format!("{} is ready.", store_tool(root))
        } else if request.yes {
            "Some tools could not be installed here; run the listed commands yourself.".to_string()
        } else {
            "Some tools are missing. Review the exact commands, then repeat with --yes to run them (one confirmation).".to_string()
        },
    );
    response.permitted_actions = if after.ready {
        vec!["wh init".into()]
    } else {
        let mut actions = after.fixes.clone();
        if !request.yes {
            actions.push("wh init --action setup --yes".into());
        }
        actions
    };
    response.data = json!({
        "tools": after.tools,
        "fixes": after.fixes,
        "ran": ran,
        "failed": failed,
        "dry_run": request.dry_run,
    });
    response
}

/// The tool the record store the switch names needs, for people.
fn store_tool(root: &std::path::Path) -> &'static str {
    match crate::storage::read_config(root) {
        Ok(crate::storage::StoreConfig::Virgil) => "The virgil CLI",
        _ => "Beads",
    }
}

/// The stores to offer, best first: Virgil when this checkout is attached
/// to a Virgil repository (`.virgil/repo.toml`), then Beads.
pub(crate) fn proposed_stores(project_root: &std::path::Path) -> Vec<&'static str> {
    if project_root.join(".virgil/repo.toml").is_file() {
        vec!["virgil", "beads"]
    } else {
        vec!["beads"]
    }
}

/// Choose the record store for every record kind (`whetstone/store.json`).
/// Choosing Virgil needs the checkout attached to a Virgil repository and
/// the daemon answering; when Beads holds records, they are exported in the
/// shape `virgil rulebook import-beads` reads, and the switch flips only
/// when a person confirms (`--yes`) that every exported revision read back
/// with its digest. Drafts and receipts then move to the private scope.
/// Choosing Beads needs `bd`; records already in Virgil stay there. Nothing
/// in Beads or Git history is rewritten.
fn store_switch(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
) -> ServiceResponse {
    use crate::storage::{self, StoreConfig, SwitchSource, STORE_CONFIG};
    let root = layout.project_root();
    let current = storage::read_switch(root);
    let describe = |switch: &Result<storage::StoreSwitch, String>| match switch {
        Ok(switch) => json!({
            "kind": switch.config.backend(),
            "source": switch.source,
        }),
        Err(error) => json!({"kind": null, "error": error}),
    };
    let Some(name) = request.store.as_deref() else {
        let proposed = proposed_stores(root);
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            ServiceState::Success,
            format!(
                "Records live in the {} store. Choose with --store {}.",
                current
                    .as_ref()
                    .map_or("(unreadable)", |switch| switch.config.backend()),
                proposed.join(" or --store "),
            ),
        );
        response.data = json!({
            "store": describe(&current),
            "switch": STORE_CONFIG,
            "proposed": proposed,
            "read_only": true,
        });
        return response;
    };
    let wanted = match StoreConfig::parse(name) {
        Ok(wanted) => wanted,
        Err(message) => return needs_input("init", request_id, 0, String::new(), message),
    };
    let records_in_beads = current
        .as_ref()
        .map_or(true, |switch| switch.config == StoreConfig::Beads);
    let changed = current.as_ref().map_or(true, |switch| {
        switch.config != wanted || switch.source != SwitchSource::Store
    });
    let mut migration = json!(null);
    match wanted {
        StoreConfig::Virgil if changed || request.yes => {
            match virgil_store_migration(layout, &request_id, request, records_in_beads) {
                Ok(data) => migration = data,
                Err(response) => return *response,
            }
        }
        StoreConfig::Beads if changed && !crate::setup::bd_ready() => {
            let mut response = needs_input(
                "init",
                request_id,
                0,
                String::new(),
                format!(
                    "The beads store needs bd {} or later, which is not installed; {} was not changed.",
                    crate::setup::MIN_BD_VERSION,
                    STORE_CONFIG
                ),
            );
            response.permitted_actions = vec![
                crate::setup::bd_install_command(),
                "wh init --action setup --store beads".into(),
            ];
            return response;
        }
        _ => {}
    }
    let mut response = ServiceResponse::new(
        request_id,
        "init",
        if request.dry_run && changed {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        match (request.dry_run, changed) {
            (_, false) => format!("Records already live in the {} store; nothing changed.", wanted.backend()),
            (true, true) => format!(
                "{STORE_CONFIG} would name the {} store; nothing was written.",
                wanted.backend()
            ),
            (false, true) => format!(
                "Every record now lives in the {} store ({STORE_CONFIG} records the switch; commit it so the team shares it).{}",
                wanted.backend(),
                match wanted {
                    StoreConfig::Virgil => " Drafts live in your private scope of the repository's rulebook, visible to no one else; accepted records reach the group's Records when they are accepted; wh push and wh pull are the daemon's now.",
                    StoreConfig::Beads => " Records already in Virgil stay there; wh dash lists them.",
                }
            ),
        },
    );
    if changed && !request.dry_run {
        if let Err(error) = storage::write_config(root, wanted) {
            return unknown_response("init", response.request_id, error);
        }
    }
    response.permitted_actions = vec!["wh dash".into()];
    response.data = json!({
        "store": {"kind": wanted.backend()},
        "previous": describe(&current),
        "switch": STORE_CONFIG,
        "migrated": migration["rulebook"]["confirmed"].as_u64().unwrap_or(0),
        "migration": migration,
        "written": changed && !request.dry_run,
    });
    response
}

/// Before the switch names Virgil: the checkout must be attached and the
/// daemon must answer `virgil rulebook` in both scopes; records Beads holds
/// must have been imported and read back. `Ok(data)` lets the flip go ahead
/// (after the private records moved to the private scope).
fn virgil_store_migration(
    layout: &ProjectLayout,
    request_id: &str,
    request: &InitRequest,
    records_in_beads: bool,
) -> Result<serde_json::Value, Box<ServiceResponse>> {
    use crate::store::{export, virgil};
    let root = layout.project_root();
    let stop =
        |state: ServiceState, summary: String, actions: Vec<String>, data: serde_json::Value| {
            let mut response = ServiceResponse::new(request_id.to_string(), "init", state, summary);
            response.permitted_actions = actions;
            response.data = json!({
                "store": {"kind": "virgil"},
                "switch": crate::storage::STORE_CONFIG,
                "migration": data,
                "written": false,
            });
            Box::new(response)
        };
    let (group, repository) = match virgil::attachment(root) {
        Ok(Some(attached)) => attached,
        Ok(None) => {
            return Err(stop(
                ServiceState::NeedsInput,
                "This checkout is not attached to a Virgil repository, so records have nowhere to live; the switch was not flipped. An admin registers the repository, a developer binds it to the group's channel, then attach this checkout (it writes .virgil/repo.toml; commit it).".into(),
                vec![
                    "virgil repos register --group <group> --name <name> --origin-source <git URL> (an admin)".into(),
                    "virgil repos bind <repository> --group <group> --channel <channel>".into(),
                    "virgil repos attach <repository> --group <group>".into(),
                    "wh init --action setup --store virgil".into(),
                ],
                json!({"attached": null, "missing": ".virgil/repo.toml (virgil repos attach)"}),
            ))
        }
        Err(error) => {
            let code = match &error {
                StorageError::Virgil { code, .. } => code.as_str(),
                _ => "store_unavailable",
            };
            return Err(stop(
                if code == "store_unavailable" {
                    ServiceState::Unavailable
                } else {
                    ServiceState::Conflict
                },
                format!("Virgil could not say which repository this checkout is attached to ({code}): {error}; the switch was not flipped."),
                vec!["virgil status".into(), "virgil repos show".into()],
                json!({"refusal": {"code": code, "message": error.to_string()}, "missing": "a reachable virgil daemon"}),
            ))
        }
    };
    let probed = crate::store::virgil::VirgilStore::shared(root)
        .probe()
        .and_then(|()| crate::store::virgil::VirgilStore::private(root).probe());
    if let Err(error) = probed {
        let (state, missing) = match &error {
            StorageError::Virgil { code, .. } if code == "store_contract" => (
                ServiceState::Conflict,
                format!("virgil rulebook ({})", crate::store::virgil::RULEBOOK_API),
            ),
            _ => (
                ServiceState::Unavailable,
                "a reachable virgil daemon".to_string(),
            ),
        };
        return Err(stop(
            state,
            format!("The Virgil daemon does not answer virgil rulebook ({error}); the switch was not flipped. This needs {missing}."),
            vec!["virgil status".into(), "virgil rulebook --help".into()],
            json!({"missing": missing, "attached": {"group": group, "repository": repository}}),
        ));
    }
    let confirm = "wh init --action setup --store virgil --yes".to_string();
    let mut rulebook = json!({"exported": 0, "confirmed": 0});
    // What the person reviews before confirming: (summary, commands).
    let mut review: Vec<(String, Vec<String>)> = Vec::new();
    if records_in_beads {
        let built = export::build(layout).map_err(|error| {
            stop(
                ServiceState::Unavailable,
                format!("The records in Beads could not be read for the export: {error}; the switch was not flipped."),
                vec!["wh dash".into()],
                json!({"error": error.to_string()}),
            )
        })?;
        let file = export::path(layout)
            .map_err(|error| stop(ServiceState::Unknown, error, vec![], json!(null)))?;
        let import = |apply: bool| {
            format!(
                "echo '{{\"scope\":\"repository\",\"file\":\"{}\",\"apply\":{apply}}}' | virgil rulebook import-beads --input -",
                file.display()
            )
        };
        rulebook = json!({
            "file": file.display().to_string(),
            "exported": built.entries.len(),
            "records": built.references(),
            "blocked": built.blocked,
            "collisions": built.collisions,
            "confirmed": 0,
        });
        if !built.collisions.is_empty() {
            return Err(stop(
                ServiceState::Conflict,
                format!(
                    "{} accepted revision{} here collide{} with the shared Beads database's ({}); rebase {} with wh change before moving to Virgil. The switch was not flipped.",
                    built.collisions.len(),
                    if built.collisions.len() == 1 { "" } else { "s" },
                    if built.collisions.len() == 1 { "s" } else { "" },
                    built.collisions.join(", "),
                    if built.collisions.len() == 1 { "it" } else { "them" },
                ),
                vec!["wh pull".into(), "wh dash".into()],
                json!({"rulebook": rulebook}),
            ));
        }
        if !built.entries.is_empty()
            && request.yes
            && !request.dry_run
            && request.actor != crate::domain::ActorKind::Person
        {
            return Err(stop(
                ServiceState::Conflict,
                "Confirming the move to Virgil is a person's: it moves every record the team reads. A person runs this at a terminal.".into(),
                vec![confirm.clone()],
                json!({"rulebook": rulebook}),
            ));
        }
        if !built.entries.is_empty() {
            if !request.yes || request.dry_run {
                if !request.dry_run {
                    export::write(layout, &built, &repository).map_err(|error| {
                        stop(
                            ServiceState::Unknown,
                            format!("The export could not be written: {error}"),
                            vec![],
                            json!(null),
                        )
                    })?;
                }
                review.push((
                    format!(
                        "{} record revision{} in Beads must reach Virgil before the switch flips. {} the export at {}; review it with the dry run and apply it.",
                        built.entries.len(),
                        if built.entries.len() == 1 { "" } else { "s" },
                        if request.dry_run { "Nothing was written; it would write" } else { "Wrote" },
                        file.display(),
                    ),
                    vec![import(false), import(true)],
                ));
            } else if !export::written(layout, &built, &repository) {
                return Err(stop(
                    ServiceState::NeedsInput,
                    format!(
                        "No export of the records Beads holds now is at {}, so there is nothing to confirm; the switch was not flipped. Run wh init --action setup --store virgil to write it, import it, then confirm.",
                        file.display()
                    ),
                    vec!["wh init --action setup --store virgil".into()],
                    json!({"rulebook": rulebook}),
                ));
            }
        }
        if !built.entries.is_empty() && request.yes && !request.dry_run {
            let confirmation = export::confirm(root, &built).map_err(|error| {
                stop(
                    ServiceState::Unavailable,
                    format!("The export could not be read back from Virgil: {error}; the switch was not flipped."),
                    vec![import(true), confirm.clone()],
                    json!({"rulebook": rulebook}),
                )
            })?;
            if !confirmation.complete() {
                let mut named = confirmation
                    .missing
                    .iter()
                    .map(|id| format!("{id} (missing)"))
                    .collect::<Vec<_>>();
                named.extend(
                    confirmation
                        .mismatched
                        .iter()
                        .map(|id| format!("{id} (different digest)")),
                );
                rulebook["missing"] = json!(confirmation.missing);
                rulebook["mismatched"] = json!(confirmation.mismatched);
                return Err(stop(
                    if confirmation.mismatched.is_empty() {
                        ServiceState::NeedsInput
                    } else {
                        ServiceState::Conflict
                    },
                    format!(
                        "{} exported revision{} did not read back from Virgil as exported: {}; the switch was not flipped.",
                        named.len(),
                        if named.len() == 1 { "" } else { "s" },
                        named.join(", ")
                    ),
                    vec![import(true), confirm],
                    json!({"rulebook": rulebook}),
                ));
            }
            rulebook["confirmed"] = json!(confirmation.confirmed);
        }
    }
    if !review.is_empty() {
        let mut actions = Vec::new();
        let mut summary = Vec::new();
        for (text, commands) in review {
            summary.push(text);
            actions.extend(
                commands
                    .into_iter()
                    .filter(|command| !command.starts_with("wh init")),
            );
        }
        actions.push(confirm);
        summary.push("Confirm as a person with --yes once everything has read back; nothing flips until then.".into());
        return Err(stop(
            ServiceState::NeedsDecision,
            summary.join(" "),
            actions,
            json!({"rulebook": rulebook}),
        ));
    }
    let carried = if records_in_beads && !request.dry_run {
        export::carry_private(layout).map_err(|error| {
            stop(
                ServiceState::Unavailable,
                format!("The private drafts and receipts could not be moved to the private scope: {error}; the switch was not flipped."),
                vec![confirm.clone()],
                json!({"rulebook": rulebook}),
            )
        })?
    } else {
        0
    };
    Ok(json!({
        "attached": {"group": group, "repository": repository},
        "rulebook": rulebook,
        "carried_private": carried,
    }))
}
