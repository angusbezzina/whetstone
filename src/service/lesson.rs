//! Lessons in the `wh change` family: remember (`--kind lesson`), admit,
//! supersede and forget, under the memory contract in [`crate::memory`].
//! Also the lesson views `wh dash` lists and the anchor receipts `wh check`
//! records.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{
    ActorKind, EvidenceRef, ForgetProposal, LessonCheck, LessonDecision, LessonStatus, LessonVerb,
    ProvenanceKind, RecordBody, RecordId, Rule, RuleSource, RuleSourceKind, Trust,
};
use crate::memory::{self, LessonDraft, LessonStore, Refusal, StoreConfig, StoredLesson};
use crate::storage::ProjectLayout;

use super::record::{append_private, key_suffix, receipt_record};
use super::{
    load_records, project_label, utc_now, ChangeKind, ChangeRequest, LessonDefinition, LessonOp,
    ServiceResponse, ServiceState,
};

/// A refusal as a response: the contract's code, never a partial write.
pub(crate) fn refused(request_id: String, refusal: Refusal) -> ServiceResponse {
    let state = match refusal.code {
        "store_unavailable" => ServiceState::Unavailable,
        "lesson_not_found" | "unattached" => ServiceState::NeedsInput,
        _ => ServiceState::Conflict,
    };
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        state,
        format!("Refused ({}): {}", refusal.code, refusal.message),
    );
    response.permitted_actions = match refusal.code {
        "agent_cannot_admit" => vec![
            "wh dash (a person admits it there)".into(),
            "take other ready work".into(),
        ],
        "read_again" => vec![
            "wh dash --json (read the lesson's current revision)".into(),
            "repeat the request with a new request id".into(),
        ],
        "unattached" => vec![
            "virgil repos show".into(),
            "virgil repos attach <repository> --group <group>".into(),
        ],
        "store_unavailable" => vec!["virgil status, then repeat the request".into()],
        _ => vec!["correct the request, then repeat it".into()],
    };
    response.data = json!({"refusal": refusal, "recorded": false});
    response
}

fn config(layout: &ProjectLayout) -> Result<StoreConfig, Refusal> {
    memory::read_config(layout.project_root())
        .map_err(|message| Refusal::new("memory_switch_invalid", message))
}

fn agreement(layout: &ProjectLayout) -> Option<AgreementState> {
    load_records(layout)
        .ok()
        .filter(|loaded| loaded.exists())
        .map(|loaded| AgreementState::from_records(loaded.union().0))
}

/// Writer-supplied fields the store owns are refused by name.
fn refuse_stamped(
    definition: Option<&LessonDefinition>,
    request: &ChangeRequest,
) -> Result<(), Refusal> {
    let mut supplied = definition
        .map(|definition| definition.other.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    if request.source.is_some() {
        supplied.push("source".into());
    }
    if request.new_owner.is_some() {
        supplied.push("new_owner".into());
    }
    let stamped = supplied
        .iter()
        .filter(|field| {
            memory::STAMPED_FIELDS.contains(&field.as_str())
                || matches!(field.as_str(), "source" | "new_owner")
        })
        .cloned()
        .collect::<Vec<_>>();
    if !stamped.is_empty() {
        return Err(Refusal::new(
            "provenance_is_stamped",
            format!(
                "A lesson request may not supply {}: who wrote it, in which session, task, branch and commit is stamped by the store, and trust, status and decisions are a person's. Remove {} and repeat.",
                stamped.join(", "),
                if stamped.len() == 1 { "it" } else { "them" }
            ),
        ));
    }
    if let Some(field) = supplied.first() {
        return Err(Refusal::new(
            "invalid_definition",
            format!("A lesson definition has key, about, concepts, audience and anchors; {field} is not one of them."),
        ));
    }
    Ok(())
}

fn lesson_definition(request: &ChangeRequest) -> Result<Option<&LessonDefinition>, Refusal> {
    match request.definition.as_deref() {
        None => Ok(None),
        Some(super::ChangeDefinition::Lesson(definition)) => Ok(Some(definition)),
        Some(_) => Err(Refusal::new(
            "invalid_definition",
            "A lesson's definition is {\"type\":\"lesson\",\"about\":[...],\"concepts\":[...],\"audience\":{...},\"anchors\":[...]}.",
        )),
    }
}

fn text_of(request: &ChangeRequest) -> Result<String, Refusal> {
    let text = request
        .content
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    if text.is_empty() {
        return Err(Refusal::new(
            "invalid_lesson",
            "Provide the lesson in words with --content.",
        ));
    }
    if text.len() > crate::domain::MAX_LESSON_TEXT {
        return Err(Refusal::new(
            "invalid_lesson",
            "A lesson is at most 4000 bytes.",
        ));
    }
    Ok(text.to_string())
}

/// The draft a writer asked for, before stamping.
fn draft(
    layout: &ProjectLayout,
    config: &StoreConfig,
    request: &ChangeRequest,
    base: Option<&crate::domain::Lesson>,
    supersedes: Option<String>,
) -> Result<LessonDraft, Refusal> {
    let definition = lesson_definition(request)?;
    refuse_stamped(definition, request)?;
    let text = text_of(request)?;
    let defaults = LessonDefinition::default();
    let definition = definition.unwrap_or(&defaults);
    let key = definition
        .key
        .clone()
        .or_else(|| {
            request
                .record_id
                .as_deref()
                .map(|id| id.trim_start_matches("lesson.").to_string())
        })
        .unwrap_or_else(|| memory::key_from_text(&text));
    if !crate::domain::valid_key(&key) {
        return Err(Refusal::new(
            "invalid_lesson",
            format!("{key} is not a lesson key: 3 to 80 lowercase letters, digits and hyphens."),
        ));
    }
    Ok(LessonDraft {
        key,
        text,
        about: inherit(&definition.about, base.map(|lesson| &lesson.about)),
        concepts: inherit(&definition.concepts, base.map(|lesson| &lesson.concepts)),
        audience: definition
            .audience
            .clone()
            .or_else(|| base.map(|lesson| lesson.audience.clone()))
            .unwrap_or_else(|| {
                memory::default_audience(
                    config,
                    layout.project_root(),
                    &project_label(layout.project_root()),
                )
            }),
        // A correction keeps the anchors of the lesson it corrects unless the
        // writer names others (inside Virgil a rewrite replaces them whole).
        anchor_paths: inherit(
            &definition.anchors,
            base.map(|lesson| {
                lesson
                    .anchors
                    .iter()
                    .map(|anchor| anchor.path.clone())
                    .collect::<Vec<_>>()
            })
            .as_ref(),
        ),
        supersedes,
    })
}

/// The writer's list, or the corrected lesson's when the writer gave none.
fn inherit<T: Clone>(own: &[T], base: Option<&Vec<T>>) -> Vec<T> {
    if own.is_empty() {
        base.cloned().unwrap_or_default()
    } else {
        own.to_vec()
    }
}

/// The stamped, anchored lesson for a draft.
fn stamped_lesson(
    layout: &ProjectLayout,
    actor: ActorKind,
    draft: &LessonDraft,
) -> Result<crate::domain::Lesson, Refusal> {
    let at = utc_now();
    let lesson = memory::remembered(
        draft,
        memory::stamp(layout.project_root(), actor, &at),
        memory::anchors(layout.project_root(), draft),
    );
    lesson.validate().map_err(|error| {
        Refusal::new(
            "invalid_lesson",
            format!("The lesson is not valid: {error}"),
        )
    })?;
    Ok(lesson)
}

/// `wh change` for lessons: remember, admit, supersede or forget.
pub(crate) fn change(
    layout: &ProjectLayout,
    request_id: String,
    request: &ChangeRequest,
) -> ServiceResponse {
    let config = match config(layout) {
        Ok(config) => config,
        Err(refusal) => return refused(request_id, refusal),
    };
    let store = memory::open_store_as(layout, &config, request.actor);
    let result = match (&request.lesson, request.kind) {
        (Some(LessonOp::Admit { key, trust }), _) => admit(
            layout,
            &config,
            store.as_ref(),
            &request_id,
            request,
            key,
            *trust,
        ),
        (Some(LessonOp::Supersede { key }), _) => {
            supersede(layout, &config, store.as_ref(), &request_id, request, key)
        }
        (Some(LessonOp::Forget { key }), _) => {
            forget(layout, store.as_ref(), &request_id, request, key)
        }
        (None, Some(ChangeKind::Lesson)) => {
            remember(layout, &config, store.as_ref(), &request_id, request)
        }
        (None, _) => Err(Refusal::new("invalid_request", "Not a lesson request.")),
    };
    match result {
        Ok(response) => response,
        Err(refusal) => refused(request_id, refusal),
    }
}

fn remember(
    layout: &ProjectLayout,
    config: &StoreConfig,
    store: &dyn LessonStore,
    request_id: &str,
    request: &ChangeRequest,
) -> Result<ServiceResponse, Refusal> {
    let draft = draft(layout, config, request, None, None)?;
    memory::scope_check(agreement(layout).as_ref(), layout.project_root(), &draft)?;
    let lesson = stamped_lesson(layout, request.actor, &draft)?;
    if request.preview {
        return Ok(preview(request_id, "remember", &lesson));
    }
    let stored = store.remember(&lesson, &format!("lesson:remember:{request_id}"))?;
    let mut response = ServiceResponse::new(
        request_id.to_string(),
        "change",
        ServiceState::Success,
        match config {
            StoreConfig::Beads => format!(
                "Lesson {} was proposed (inferred) in the beads store. A person admits it; nothing is delivered until then.",
                stored.lesson.key
            ),
            StoreConfig::Virgil => format!(
                "Lesson {} was proposed (inferred) and waits in Whetstone's store; nothing reaches Virgil until a person admits it, which writes it to the group's lessons.",
                stored.lesson.key
            ),
        },
    );
    response.permitted_actions = vec![
        format!("wh change --admit {} (a person)", stored.lesson.key),
        "wh dash".into(),
    ];
    response.data =
        json!({"lesson": view(layout, &stored, &[], &BTreeMap::new()), "recorded": true});
    Ok(response)
}

fn preview(request_id: &str, verb: &str, lesson: &crate::domain::Lesson) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id.to_string(),
        "change",
        ServiceState::NeedsDecision,
        format!("Review the lesson before you {verb} it; nothing was recorded."),
    );
    response.permitted_actions = vec!["repeat without --dry-run to record it".into()];
    response.data = json!({"preview_only": true, "lesson": lesson});
    response
}

/// A person's decision, stamped now. `{who}` in the default rationale names
/// the person.
fn person_decision(
    layout: &ProjectLayout,
    verb: LessonVerb,
    request: &ChangeRequest,
    default_rationale: &str,
) -> (LessonDecision, crate::domain::LessonStamp) {
    let at = utc_now();
    let stamp = memory::stamp(layout.project_root(), ActorKind::Person, &at);
    let rationale = request
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map_or_else(
            || default_rationale.replace("{who}", &stamp.who),
            |text| text.chars().take(2_000).collect(),
        );
    (
        LessonDecision {
            verb,
            by: stamp.who.clone(),
            at,
            rationale,
            receipt: None,
        },
        stamp,
    )
}

fn admit(
    layout: &ProjectLayout,
    config: &StoreConfig,
    store: &dyn LessonStore,
    request_id: &str,
    request: &ChangeRequest,
    key: &str,
    trust: Option<Trust>,
) -> Result<ServiceResponse, Refusal> {
    if request.actor != ActorKind::Person {
        return Err(Refusal::new(
            "agent_cannot_admit",
            format!("An agent proposes lessons and never admits them, by any route. {key} stays proposed (inferred) until a person admits it at a terminal or in wh dash."),
        ));
    }
    if let (StoreConfig::Virgil, Some(trust)) = (config, trust) {
        return Err(Refusal::new(
            "trust_not_chosen",
            format!("Inside Virgil trust is derived from the writer, never chosen: a person's admit lands validated and nothing writes authoritative. Repeat without --trust {}.", trust.label()),
        ));
    }
    let trust = trust.unwrap_or(Trust::Validated);
    if trust == Trust::Inferred {
        return Err(Refusal::new(
            "invalid_trust",
            "Admitting makes a lesson validated or authoritative.",
        ));
    }
    let Some(current) = store.get(key)? else {
        return Err(Refusal::new(
            "lesson_not_found",
            format!("No lesson {key} exists; see wh dash --json (current.lessons)."),
        ));
    };
    let (decision, stamp) =
        person_decision(layout, LessonVerb::Admit, request, "Admitted by {who}.");
    if request.preview {
        let next = memory::decided(&current.lesson, decision, stamp);
        return Ok(preview(request_id, "admit", &next));
    }
    let stored = store.admit(
        key,
        trust,
        &decision,
        &stamp,
        &format!("lesson:admit:{request_id}:{key}"),
    )?;
    let mut response = ServiceResponse::new(
        request_id.to_string(),
        "change",
        ServiceState::Success,
        format!(
            "Lesson {} was admitted ({}) in the {} store; it is delivered to agents and people from now on.{}",
            stored.lesson.key,
            stored.lesson.trust.label(),
            store.backend(),
            stored
                .lesson
                .supersedes
                .as_deref()
                .map(|old| format!(" It supersedes {old}."))
                .unwrap_or_default()
        ),
    );
    response.permitted_actions = vec![
        format!("wh change --kind rule --from-lesson {key} ... (draft a rule in shadow)"),
        "wh dash".into(),
    ];
    response.data =
        json!({"lesson": view(layout, &stored, &[], &BTreeMap::new()), "recorded": true});
    Ok(response)
}

fn supersede(
    layout: &ProjectLayout,
    config: &StoreConfig,
    store: &dyn LessonStore,
    request_id: &str,
    request: &ChangeRequest,
    key: &str,
) -> Result<ServiceResponse, Refusal> {
    let Some(old) = store.get(key)? else {
        return Err(Refusal::new(
            "lesson_not_found",
            format!("No lesson {key} exists to correct."),
        ));
    };
    if matches!(
        old.lesson.status,
        LessonStatus::Superseded | LessonStatus::Forgotten
    ) {
        return Err(Refusal::new(
            "lesson_not_live",
            format!(
                "{key} is {}; correct {} instead.",
                old.lesson.status.label(),
                old.lesson
                    .superseded_by
                    .as_deref()
                    .unwrap_or("the lesson that replaced it")
            ),
        ));
    }
    let named = lesson_definition(request)?
        .and_then(|definition| definition.key.clone())
        .or_else(|| {
            request
                .record_id
                .as_deref()
                .map(|id| id.trim_start_matches("lesson.").to_string())
        });
    // Inside Virgil a person may retire a lesson in favour of one that
    // already exists there, without writing new text.
    if let (ActorKind::Person, Some(successor)) = (request.actor, named.as_deref()) {
        if successor != key && !request.preview {
            let (decision, _) = person_decision(
                layout,
                LessonVerb::Supersede,
                request,
                &format!("{key} retired in favour of {successor} by {{who}}."),
            );
            if let Some(retired) = store.retire(
                key,
                successor,
                &decision,
                &format!("lesson:retire:{request_id}:{key}"),
            )? {
                let mut response = ServiceResponse::new(
                    request_id.to_string(),
                    "change",
                    ServiceState::Success,
                    format!("{key} was superseded by {successor}, which already held; no new text was written."),
                );
                response.permitted_actions = vec!["wh dash".into()];
                response.data = json!({
                    "lesson": view(layout, &retired, &[], &BTreeMap::new()),
                    "supersedes": key,
                    "applied": true,
                    "recorded": true,
                });
                return Ok(response);
            }
        }
    }
    let mut draft = draft(
        layout,
        config,
        request,
        Some(&old.lesson),
        Some(key.to_string()),
    )?;
    let explicit_key = named.is_some();
    if !explicit_key {
        // A correction gets its own key: the old key with the next free suffix.
        let existing = store.taken_keys()?;
        let stem = key.chars().take(72).collect::<String>();
        let stem =
            stem.trim_end_matches(|character: char| character.is_ascii_digit() || character == '-');
        let stem = if stem.len() < 3 { key } else { stem };
        draft.key = (2..)
            .map(|suffix| format!("{stem}-{suffix}"))
            .find(|candidate| !existing.contains(candidate))
            .unwrap_or_else(|| format!("{stem}-x"));
    }
    if draft.key == key {
        return Err(Refusal::new(
            "invalid_lesson",
            "A correction is a new lesson with its own key.",
        ));
    }
    memory::scope_check(agreement(layout).as_ref(), layout.project_root(), &draft)?;
    let correction = stamped_lesson(layout, request.actor, &draft)?;
    if request.preview {
        return Ok(preview(request_id, "supersede", &correction));
    }
    let proposed = store.supersede(
        key,
        &correction,
        &format!("lesson:supersede:{request_id}:{key}"),
    )?;
    // A person's correction is applied: the new lesson is admitted, which
    // supersedes the old one. An agent's waits for a person.
    let applied = if request.actor == ActorKind::Person {
        let (decision, stamp) = person_decision(
            layout,
            LessonVerb::Admit,
            request,
            &format!("Correction of {key} applied by {{who}}."),
        );
        Some(store.admit(
            &proposed.lesson.key,
            old.lesson.trust.max(Trust::Validated),
            &decision,
            &stamp,
            &format!("lesson:admit:{request_id}:{}", proposed.lesson.key),
        )?)
    } else {
        None
    };
    let current = applied.as_ref().unwrap_or(&proposed);
    let mut response = ServiceResponse::new(
        request_id.to_string(),
        "change",
        ServiceState::Success,
        if applied.is_some() && current.lesson.key == key {
            format!(
                "{key} was rewritten in place (revision {}) with the correction; its subjects and anchors were kept.",
                current.revision
            )
        } else if applied.is_some() {
            format!(
                "{key} was corrected by {} and is superseded.",
                current.lesson.key
            )
        } else {
            format!(
                "A correction of {key} was proposed as {}; {key} stays as it is until a person admits the correction.",
                current.lesson.key
            )
        },
    );
    response.permitted_actions = vec![
        format!("wh change --admit {} (a person)", current.lesson.key),
        "wh dash".into(),
    ];
    response.data = json!({
        "lesson": view(layout, current, &[], &BTreeMap::new()),
        "supersedes": key,
        "applied": applied.is_some(),
        "recorded": true,
    });
    Ok(response)
}

fn forget(
    layout: &ProjectLayout,
    store: &dyn LessonStore,
    request_id: &str,
    request: &ChangeRequest,
    key: &str,
) -> Result<ServiceResponse, Refusal> {
    let reason = request
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| Refusal::new("invalid_request", "Say why with --rationale."))?
        .chars()
        .take(2_000)
        .collect::<String>();
    let Some(current) = store.get(key)? else {
        return Err(Refusal::new(
            "lesson_not_found",
            format!("No lesson {key} exists to forget."),
        ));
    };
    let idempotency = format!("lesson:forget:{request_id}:{key}");
    let (stored, applied) = if request.actor == ActorKind::Person {
        let (decision, stamp) = person_decision(layout, LessonVerb::Forget, request, &reason);
        if request.preview {
            return Ok(preview(
                request_id,
                "forget",
                &memory::decided(&current.lesson, decision, stamp),
            ));
        }
        (
            store.forget(key, Some(&decision), None, &stamp, &idempotency)?,
            true,
        )
    } else {
        let at = utc_now();
        let stamp = memory::stamp(layout.project_root(), ActorKind::Agent, &at);
        let proposal = ForgetProposal {
            reason,
            by: stamp.who.clone(),
            at,
        };
        if request.preview {
            return Ok(preview(
                request_id,
                "forget",
                &memory::with_forget_proposal(&current.lesson, proposal, stamp),
            ));
        }
        (
            store.forget(key, None, Some(&proposal), &stamp, &idempotency)?,
            false,
        )
    };
    let mut response = ServiceResponse::new(
        request_id.to_string(),
        "change",
        ServiceState::Success,
        if applied {
            format!("{key} was forgotten: it is no longer delivered, and its history stays.")
        } else {
            format!("Forgetting {key} was proposed; it stays delivered until a person forgets it (wh change --forget {key}).")
        },
    );
    response.permitted_actions = vec!["wh dash".into()];
    response.data = json!({"lesson": view(layout, &stored, &[], &BTreeMap::new()), "applied": applied, "recorded": true});
    Ok(response)
}

/// How a lesson reads in `wh dash` and in responses: its fields, the anchor
/// comparison made now (no model call), its corrections and the rules drafted
/// from it.
pub(crate) fn view(
    layout: &ProjectLayout,
    stored: &StoredLesson,
    all: &[StoredLesson],
    derived: &BTreeMap<String, Vec<String>>,
) -> Value {
    let lesson = &stored.lesson;
    let comparisons = memory::anchor_state(layout.project_root(), stored);
    let by_key = all
        .iter()
        .map(|stored| (stored.lesson.key.clone(), &stored.lesson))
        .collect::<BTreeMap<_, _>>();
    let corrections = memory::corrections(&by_key, &lesson.key);
    json!({
        "key": lesson.key,
        "text": lesson.text,
        "trust": lesson.trust,
        "status": lesson.status,
        "about": lesson.about,
        "subjects": lesson.about.iter().map(crate::domain::Subject::label).collect::<Vec<_>>(),
        "concepts": lesson.concepts,
        "audience": lesson.audience,
        "audience_label": lesson.audience.label(),
        "stamp": lesson.stamp,
        "anchors": comparisons,
        "may_be_stale": memory::may_be_stale(&comparisons),
        "supersedes": lesson.supersedes,
        "superseded_by": lesson.superseded_by,
        "corrections": corrections,
        "needs_review": corrections >= memory::REVIEW_AFTER_CORRECTIONS && lesson.status != LessonStatus::Forgotten,
        "forget_proposed": lesson.forget_proposed,
        "decision": lesson.decision,
        "derived_rules": derived.get(&lesson.key).cloned().unwrap_or_default(),
        "revision": stored.revision,
        "locator": stored.locator,
        "admit_command": (lesson.status == LessonStatus::Proposed).then(|| format!("wh change --admit {}", lesson.key)),
    })
}

/// Rules (in force or drafted) whose source is each lesson key.
pub(crate) fn derived_rules(state: &AgreementState) -> BTreeMap<String, Vec<String>> {
    let mut derived = BTreeMap::<String, Vec<String>>::new();
    for id in crate::proof::rule_ids(state) {
        let record = state.in_force(&id).or_else(|| state.latest(&id));
        let Some(rule) = record.and_then(|record| record.body.rule_view()) else {
            continue;
        };
        if let Some(key) = memory::origin_lesson(&rule) {
            derived
                .entry(key.to_string())
                .or_default()
                .push(id.as_str().to_string());
        }
    }
    derived
}

/// The memory section of `wh dash`: which store is active, every lesson with
/// its staleness, the proposals waiting on a person, and lessons left in the
/// other store (flipping the switch migrates nothing).
pub(crate) fn dash_section(layout: &ProjectLayout, state: &AgreementState) -> Value {
    let config = match config(layout) {
        Ok(config) => config,
        Err(refusal) => {
            return json!({"state": "invalid", "refusal": refusal, "lessons": [], "proposals": []})
        }
    };
    let store = memory::open_store(layout, &config);
    let derived = derived_rules(state);
    let (lessons, store_state, detail) = match store.find() {
        Ok(lessons) => (lessons, "available", None),
        Err(refusal) => (Vec::new(), "unavailable", Some(refusal)),
    };
    let views = lessons
        .iter()
        .map(|stored| view(layout, stored, &lessons, &derived))
        .collect::<Vec<_>>();
    let proposals = views
        .iter()
        .filter(|view| {
            view["status"] == "proposed"
                || (view["status"] == "admitted" && !view["forget_proposed"].is_null())
        })
        .map(|view| view["key"].clone())
        .collect::<Vec<_>>();
    let stale = views
        .iter()
        .filter(|view| view["may_be_stale"] == true && view["status"] == "admitted")
        .map(|view| view["key"].clone())
        .collect::<Vec<_>>();
    let review = views
        .iter()
        .filter(|view| view["needs_review"] == true)
        .map(|view| view["key"].clone())
        .collect::<Vec<_>>();
    // Admitted lessons still only in Whetstone's own store while the switch
    // names Virgil: reported until the import's apply is confirmed.
    let left_behind = if store_state == "available" {
        store.left_behind(&lessons).unwrap_or_default()
    } else {
        Vec::new()
    };
    json!({
        "state": store_state,
        "detail": detail,
        "rules_to_review": if store_state == "available" { reviews_for(state, &lessons, config.backend()) } else { Vec::new() },
        "backend": config.backend(),
        "switch": crate::storage::STORE_CONFIG,
        "lessons": views,
        "proposals": proposals,
        "may_be_stale": stale,
        "needs_review": review,
        "not_migrated": (!left_behind.is_empty()).then(|| json!({
            "backend": "beads",
            "lessons": left_behind.len(),
            "keys": left_behind,
            "note": "Admitted lessons still only in Whetstone's Beads store. Export and import them with wh init --action store --store virgil (it prints the virgil lessons import-beads commands), then confirm with --yes.",
        })),
    })
}

/// Record the hash comparison of each anchored lesson as a `lesson_check`
/// receipt, once per distinct result (a repeated check records nothing new).
pub(crate) fn record_checks(layout: &ProjectLayout) -> Vec<Value> {
    let Ok(config) = config(layout) else {
        return Vec::new();
    };
    let store = memory::open_store(layout, &config);
    let Ok(lessons) = store.find() else {
        return Vec::new();
    };
    let head = crate::gates::head_commit(layout.project_root());
    let mut recorded = Vec::new();
    for stored in lessons {
        if stored.lesson.anchors.is_empty()
            || matches!(
                stored.lesson.status,
                LessonStatus::Superseded | LessonStatus::Forgotten
            )
        {
            continue;
        }
        let Some(reference) = memory::lesson_reference(&stored) else {
            continue;
        };
        let comparisons = memory::anchor_state(layout.project_root(), &stored);
        let fingerprint = serde_json::to_string(&comparisons).unwrap_or_default();
        let key = format!(
            "lesson-check:{}:{}",
            crate::projection::reference_text(&reference),
            key_suffix(&fingerprint, 24)
        );
        let checked_at = utc_now();
        let body = LessonCheck {
            lesson: reference.clone(),
            key: stored.lesson.key.clone(),
            may_be_stale: memory::may_be_stale(&comparisons),
            comparisons,
            commit: head.clone(),
            checked_at: checked_at.clone(),
        };
        let stale = body.may_be_stale;
        let Ok(record) = receipt_record(
            layout,
            format!("verification.lesson_{}", key_suffix(&key, 24)),
            key,
            "whetstone:lesson-check",
            ProvenanceKind::DeterministicCheck,
            &checked_at,
            EvidenceRef {
                system: "whetstone_lesson".into(),
                locator: stored.locator.clone(),
                digest: Some(reference.digest.clone()),
            },
            RecordBody::LessonCheck(body),
        ) else {
            continue;
        };
        if let Ok(receipt) = append_private(layout, &record) {
            recorded.push(
                json!({"lesson": stored.lesson.key, "may_be_stale": stale, "receipt": receipt}),
            );
        }
    }
    recorded
}

/// A rule drafted from a lesson: the lesson must be admitted, the rule
/// starts in shadow, and its source names the lesson where it lives.
pub(crate) fn rule_from_lesson(
    layout: &ProjectLayout,
    key: &str,
    rule: &mut Rule,
) -> Result<(), String> {
    let key = key.trim_start_matches("lesson.");
    let config = memory::read_config(layout.project_root())?;
    let store = memory::open_store(layout, &config);
    let stored = store
        .get(key)
        .map_err(|refusal| refusal.message)?
        .ok_or_else(|| format!("No lesson {key} exists to draft a rule from."))?;
    if stored.lesson.status != LessonStatus::Admitted {
        return Err(format!(
            "Lesson {key} is {}; only an admitted (validated or authoritative) lesson can be drafted into a rule.",
            stored.lesson.status.label()
        ));
    }
    let reference = memory::lesson_reference(&stored);
    rule.source = RuleSource {
        kind: RuleSourceKind::Lesson,
        reference: Some(key.to_string()),
        provenance: vec![EvidenceRef {
            system: format!("{}_lesson", store.backend()),
            locator: stored.locator.clone(),
            digest: reference.map(|reference| reference.digest),
        }],
    };
    rule.shadow = true;
    Ok(())
}

/// What an acceptance cites when it accepts a rule drafted from a lesson:
/// the lesson revision it rested on (and whether the lesson is still admitted).
pub(crate) fn citations(layout: &ProjectLayout, rule: &Rule) -> Result<Vec<EvidenceRef>, String> {
    let Some(key) = memory::origin_lesson(rule) else {
        return Ok(Vec::new());
    };
    let config = memory::read_config(layout.project_root())?;
    let store = memory::open_store(layout, &config);
    match store.get(key) {
        Ok(Some(stored)) if stored.lesson.status == LessonStatus::Admitted => {
            Ok(vec![EvidenceRef {
                system: format!("{}_lesson", store.backend()),
                locator: stored.locator.clone(),
                digest: memory::lesson_reference(&stored).map(|reference| reference.digest),
            }])
        }
        Ok(Some(stored)) => {
            Err(format!(
            "The rule's origin lesson {key} is now {}; withdraw this draft, or redraft it from {}.",
            stored.lesson.status.label(),
            stored.lesson.superseded_by.as_deref().unwrap_or("a lesson that is admitted")
        ))
        }
        Ok(None) => Err(format!(
            "The rule's origin lesson {key} no longer exists in the {} store.",
            store.backend()
        )),
        // The store cannot be read now: cite the lesson by key, unverified.
        Err(_) => Ok(vec![EvidenceRef {
            system: format!("{}_lesson_unverified", store.backend()),
            locator: format!("lesson.{key}"),
            digest: None,
        }]),
    }
}

/// The lineage `wh dash` walks from a rule: the review that accepted it, the
/// lesson that review cites, and the lesson's anchors now.
pub(crate) fn lineage(
    layout: &ProjectLayout,
    state: &AgreementState,
    rule_id: &RecordId,
    lessons: &[StoredLesson],
) -> Option<Value> {
    let record = state.in_force(rule_id).or_else(|| state.latest(rule_id))?;
    let rule = record.body.rule_view()?;
    let key = memory::origin_lesson(&rule)?.to_string();
    let proposal = state
        .proposal_for(record)
        .and_then(|proposal| proposal.reference().ok());
    let review = proposal.as_ref().and_then(|proposal| {
        state
            .records()
            .iter()
            .find_map(|candidate| match &candidate.body {
                RecordBody::LocalReview(review) if &review.proposal == proposal => {
                    Some((candidate, review))
                }
                _ => None,
            })
    });
    let lesson = lessons.iter().find(|stored| stored.lesson.key == key);
    Some(json!({
        "lesson": key,
        "accepted_by": review.map(|(record, review)| json!({
            "review": record.id.as_str(),
            "verdict": review.verdict,
            "by": review.reviewer.display_name,
            "at": review.reviewed_at,
            "rationale": review.rationale,
            "cites": review.cites,
        })),
        "origin": lesson.map(|stored| json!({
            "key": stored.lesson.key,
            "text": stored.lesson.text,
            "trust": stored.lesson.trust,
            "status": stored.lesson.status,
            "locator": stored.locator,
            "decision": stored.lesson.decision,
            "anchors": memory::anchor_state(layout.project_root(), stored),
        })),
    }))
}

/// Demotion mirrors promotion: a rule whose origin lesson was superseded,
/// forgotten or corrected twice goes back to a person, listed by
/// `wh change --tune` and `wh dash` (nothing changes force on its own).
pub(crate) fn rule_reviews(layout: &ProjectLayout, state: &AgreementState) -> Vec<Value> {
    if derived_rules(state).is_empty() {
        return Vec::new();
    }
    let Ok(config) = config(layout) else {
        return Vec::new();
    };
    match memory::open_store(layout, &config).find() {
        Ok(lessons) => reviews_for(state, &lessons, config.backend()),
        Err(_) => Vec::new(),
    }
}

fn reviews_for(state: &AgreementState, lessons: &[StoredLesson], backend: &str) -> Vec<Value> {
    let derived = derived_rules(state);
    let by_key = lessons
        .iter()
        .map(|stored| (stored.lesson.key.clone(), &stored.lesson))
        .collect::<BTreeMap<_, _>>();
    let mut reviews = Vec::new();
    for (key, rules) in &derived {
        let reason = match by_key.get(key) {
            None => format!("its origin lesson {key} is no longer in the {backend} store"),
            Some(lesson) if lesson.status == LessonStatus::Superseded => format!(
                "its origin lesson {key} was superseded by {}",
                lesson.superseded_by.as_deref().unwrap_or("a correction")
            ),
            Some(lesson) if lesson.status == LessonStatus::Forgotten => {
                format!("its origin lesson {key} was forgotten")
            }
            Some(_) if memory::corrections(&by_key, key) >= memory::REVIEW_AFTER_CORRECTIONS => {
                format!("its origin lesson {key} has been corrected twice")
            }
            Some(_) => continue,
        };
        for rule in rules {
            if RecordId::new(rule.as_str())
                .ok()
                .and_then(|id| state.in_force(&id))
                .is_none()
            {
                continue;
            }
            reviews.push(json!({
                "rule": rule,
                "lesson": key,
                "reason": format!("Review {rule}: {reason}. Redraft it from the lesson that holds now, demote it, or retire it (wh change --retire {rule})."),
            }));
        }
    }
    reviews
}
