//! `wh init`: from mission to agreed rules in five minutes.
//!
//! Inspect is read-only. Agree records the owner's mission, the principles
//! they picked and the starter rules they accepted. Setup detects, installs
//! and pins Beads and pstack. Exemplar reads a codebase the owner admires and
//! drafts rules from it. Wire and import live in `crate::skill`.

use serde::Serialize;
use serde_json::json;

use crate::agreement::AgreementState;
use crate::domain::{
    AgreementRecord, Mission, Principle, PrincipleSource, RecordBody, RecordId, RecordRef,
};
use crate::onboarding;
use crate::projection::{LegacyItem, Onboarding, OnboardingStep, StarterView};
use crate::storage::{AppendRequest, ProjectLayout, RecordStore, StorageError, StoreKind};

use super::record::{agreement_record, ChangeNarrative};
use super::{
    bounded_input, bounded_request_id, domain_response, idempotency_conflict, latest_record,
    needs_input, owner_name, project_error, resume_token, stale_response, storage_error,
    unknown_response, InitAction, InitRequest, ServiceEvidence, ServiceResponse, ServiceState,
};

/// Where onboarding stands, computed from the records.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Progress {
    pub(crate) agreement_revision: u64,
    pub(crate) agreement_complete: bool,
    pub(crate) setup_complete: bool,
    pub(crate) missing_decisions: Vec<String>,
    pub(crate) agreement_records: Vec<RecordRef>,
    /// A passing rule run at the current workspace: the loop is proven.
    pub(crate) first_proof: Option<RecordRef>,
    pub(crate) proof_status: String,
    pub(crate) hooks: crate::hosts::GitHooks,
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
    let exemplar_rules = rules
        .iter()
        .filter(|record| {
            record
                .body
                .rule_view()
                .is_some_and(|rule| rule.source.kind == crate::domain::RuleSourceKind::Exemplar)
        })
        .count();
    let hooks = crate::hosts::git_hooks(root);
    let tools = crate::setup::detect(root, &[]);
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
    let fingerprint = crate::gates::workspace_fingerprint(root).ok();
    let mut first_proof = None;
    let mut stale = false;
    for record in state.records().iter().rev() {
        let RecordBody::VerificationReceipt(receipt) = &record.body else {
            continue;
        };
        if !receipt
            .subject
            .stable_id
            .starts_with(crate::proof::GATE_SUBJECT_PREFIX)
            || receipt.verification != crate::domain::VerificationAxis::Pass
        {
            continue;
        }
        if receipt.subject.revision.as_deref() == fingerprint.as_deref() {
            first_proof = record.reference().ok();
            break;
        }
        stale = true;
    }
    let agreement_complete = missing.is_empty();
    let setup_complete =
        agreement_complete && hooks.pre_commit && hooks.pre_push && first_proof.is_some();
    let catalogue = crate::catalogue::catalogue(root);
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
                family: starter.rule.enforcer.family().label(),
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
                label: "Beads and pstack",
                hint: "installed and pinned, so records and briefs work",
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
                hint: "the pstack principles you hold, or your own",
                done: !principles.is_empty(),
                optional: true,
            },
            OnboardingStep {
                key: "exemplars",
                label: "Exemplars",
                hint: "a codebase you admire; rules are drafted from it",
                done: exemplar_rules > 0,
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
        proof_status: if first_proof.is_some() {
            "a rule ran and passed against the current workspace".into()
        } else if stale {
            "a rule passed before, but the workspace changed since; run wh check".into()
        } else if agreement_complete {
            "no rule has passed yet; run wh check".into()
        } else {
            "no agreement yet, so nothing has been checked".into()
        },
        first_proof,
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
        InitAction::Wire => return crate::skill::wire(&layout, request_id, &request),
        InitAction::Import => return crate::skill::import(&layout, request_id, &request),
        InitAction::Setup => return setup(&layout, request_id, &request),
        InitAction::Exemplar => return exemplar(&layout, request_id, &request),
        _ => {}
    }
    let setup_plan = match onboarding::inspect(layout.project_root()) {
        Ok(value) => value,
        Err(error) => {
            return unknown_response(
                "init",
                request_id,
                format!("Project inspection failed without changing state: {error:?}"),
            )
        }
    };
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
                "setup": setup_plan,
                "progress": progress,
                "cancelled": true,
                "writes": [],
                "shared": false,
            });
            response
        }
        InitAction::Inspect => inspection_response(request_id, resume, setup_plan, progress),
        _ => agree(&layout, request_id, &request, progress, resume),
    }
}

fn inspection_response(
    request_id: String,
    resume: String,
    setup: onboarding::SetupPlan,
    progress: Progress,
) -> ServiceResponse {
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
    response.evidence = setup
        .facts
        .iter()
        .map(|fact| ServiceEvidence {
            kind: format!("detected_{:?}", fact.kind).to_ascii_lowercase(),
            locator: fact.path.clone(),
            digest: Some(fact.digest.as_str().into()),
        })
        .collect();
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
            "wh init --action wire --hooks".into(),
            "wh check".into(),
            "wh init --action exemplar --from <path or Git URL>".into(),
        ]
    } else {
        vec![
            "wh init --action agree --mission <text> --principle <id> --starter all with the returned revision and token".into(),
            "wh init --action cancel".into(),
        ]
    };
    response.data = json!({
        "setup": setup,
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
    let existing_store =
        RecordStore::open_existing(&layout.private_store(), StoreKind::Private).ok();
    let stored = match existing_store
        .as_ref()
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
    let catalogue = crate::catalogue::catalogue(layout.project_root());
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
                    "{id} is not a pstack principle in the catalogue (pstack {}). Known ids include prove-it-works, laziness-protocol and boundary-discipline.",
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
        if let Some(shared) = layout.shared_store() {
            if let Err(error) = RecordStore::initialize(&shared, StoreKind::Shareable) {
                return storage_error("init", request_id, error);
            }
        }
        let private = match RecordStore::initialize(&layout.private_store(), StoreKind::Private) {
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
    response.permitted_actions = vec![
        "wh init --action wire --hooks".into(),
        "wh check".into(),
        "wh init --action exemplar --from <path or Git URL>".into(),
    ];
    response.data = json!({
        "records": references,
        "progress": progress,
        "onboarding": progress.onboarding,
        "shared": false,
        "platform_configuration_writes": [],
    });
    response
}

/// Detect, then (with `--yes`) install and pin, Beads and pstack.
fn setup(layout: &ProjectLayout, request_id: String, request: &InitRequest) -> ServiceResponse {
    let root = layout.project_root();
    let detected = crate::setup::detect(root, &request.hosts);
    let mut ran = Vec::new();
    let mut failed = Vec::new();
    if request.yes && !request.dry_run {
        for fix in detected
            .fixes
            .iter()
            .filter(|fix| !fix.starts_with("wh init"))
        {
            match crate::setup::run_install(root, fix) {
                Ok(_) => ran.push(fix.clone()),
                Err(error) => failed.push(json!({"command": fix, "error": error})),
            }
        }
    }
    let after = if ran.is_empty() {
        detected.clone()
    } else {
        crate::setup::detect(root, &request.hosts)
    };
    let lock = crate::setup::lock_for(&after);
    let lock_written = match (&lock, request.dry_run) {
        (Some(lock), false) if after.lock.as_ref() != Some(lock) => {
            match crate::setup::write_lock(root, lock) {
                Ok(()) => true,
                Err(error) => {
                    failed.push(json!({"command": "write the lock", "error": error}));
                    false
                }
            }
        }
        _ => false,
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
            if lock_written {
                format!(
                    "Beads and pstack are ready; versions are pinned in {}.",
                    crate::setup::LOCK_RELATIVE
                )
            } else {
                "Beads and pstack are ready.".to_string()
            }
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
        "skills": after.skills,
        "fixes": after.fixes,
        "ran": ran,
        "failed": failed,
        "lock": lock,
        "lock_path": crate::setup::LOCK_RELATIVE,
        "lock_written": lock_written,
        "dry_run": request.dry_run,
    });
    response
}

/// Read an exemplar and record the rules it demonstrates as drafts.
fn exemplar(layout: &ProjectLayout, request_id: String, request: &InitRequest) -> ServiceResponse {
    let locator = match (&request.exemplar_url, &request.import_from) {
        (Some(url), _) => url.clone(),
        (None, Some(path)) => path.display().to_string(),
        (None, None) => {
            return needs_input(
                "init",
                request_id,
                0,
                String::new(),
                "Name the exemplar: --from <local path> or --url <Git URL>.".into(),
            )
        }
    };
    let cache = layout.state_root().join("exemplars");
    let mut opened = match crate::exemplar::open(&locator, &cache) {
        Ok(opened) => opened,
        Err(error) => return unknown_response("init", request_id, error),
    };
    let proposals = crate::exemplar::propose(&mut opened);
    if proposals.is_empty() || request.dry_run {
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            if proposals.is_empty() {
                ServiceState::Success
            } else {
                ServiceState::NeedsDecision
            },
            if proposals.is_empty() {
                format!("{locator} shows no configuration Whetstone can turn into a mechanical rule; ask your agent to read it and propose rules with wh change --kind rule --source \"exemplar:{locator}\".")
            } else {
                format!("Dry run: {} rule draft(s) would be recorded from {locator}; nothing was written.", proposals.len())
            },
        );
        response.data =
            json!({"exemplar": opened, "proposals": proposals, "dry_run": request.dry_run});
        return response;
    }
    let private = match RecordStore::initialize(&layout.private_store(), StoreKind::Private) {
        Ok(store) => store,
        Err(error) => return storage_error("init", request_id, error),
    };
    let mut drafts = Vec::new();
    for proposal in &proposals {
        let Ok(id) = RecordId::new(proposal.id.as_str()) else {
            continue;
        };
        let narrative = ChangeNarrative {
            rationale: proposal.why.clone(),
            source: format!(
                "exemplar {}@{}",
                opened.locator,
                opened.commit.as_deref().unwrap_or("working-tree")
            ),
            expected_effect: "The exemplar's practice becomes a rule here once accepted.".into(),
            impact: "none until accepted".into(),
            examples: Vec::new(),
            conflicts: Vec::new(),
        };
        let key = format!("exemplar:{request_id}:{}", proposal.id);
        match super::change::draft_with_proposal(
            layout,
            &private,
            &key,
            &id,
            RecordBody::Rule(proposal.rule.clone()),
            &narrative,
        ) {
            Ok((record, proposal)) => drafts.push(json!({"record": record, "proposal": proposal})),
            Err(error) => return storage_error("init", request_id, error),
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "init",
        ServiceState::Success,
        format!(
            "{} rule draft(s) from {locator} were recorded with the files they came from; nothing is in force until you accept each.",
            drafts.len()
        ),
    );
    response.permitted_actions = vec!["wh dash".into(), "wh change --accept <proposal>".into()];
    response.data = json!({"exemplar": opened, "drafts": drafts, "proposals": proposals});
    response
}
