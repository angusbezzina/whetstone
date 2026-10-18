//! `wh change`: propose a bounded change as a private draft the owner
//! accepts, and the owner's other decisions: accepting or withdrawing a
//! draft, retiring a record, and migrating earlier values and philosophy.

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::agreement::AgreementState;
use crate::domain::{
    AgreementRecord, EvidenceRef, Principle, PrincipleSource, Provenance, ProvenanceAuthority,
    ProvenanceKind, RecordBody, RecordId, RecordRef, Rule, RuleSource, RULE_SCHEMA_V2,
    SCHEMA_VERSION_V1,
};
use crate::storage::{ProjectLayout, RecordStore, StorageError};

use super::record::{agreement_record, ensure_local_change_proposal, ChangeNarrative};
use super::{
    bounded_input, bounded_list, bounded_request_id, domain_response, idempotency_conflict,
    load_records, needs_input, optional_input, owner_name, project_error, resume_token,
    stale_response, storage_error, unknown_response, utc_now, ChangeDefinition, ChangeKind,
    ChangeRequest, ReviewRequest, ServiceResponse, ServiceState,
};

pub(super) fn change(request: ChangeRequest) -> ServiceResponse {
    let layout = match ProjectLayout::resolve(&request.project_dir) {
        Ok(layout) => layout,
        Err(error) => return project_error("change", request.request_id, error),
    };
    let request_id = match bounded_request_id(
        request.request_id.clone(),
        default_request_id(&layout, &request),
    ) {
        Ok(request_id) => request_id,
        Err(summary) => return unknown_response("change", "invalid-request-id".into(), summary),
    };
    let private = match crate::storage::initialize_private(&layout) {
        Ok(repository) => repository,
        Err(error) => return storage_error("change", request_id, error),
    };
    let private: &dyn RecordStore = &*private;
    if let Some(review) = request.review.clone() {
        return review_draft(private, request_id, &review, &request);
    }
    if let Some(target) = request.retire.clone() {
        return retire_record(&layout, private, request_id, &target, &request);
    }
    if request.migrate {
        return migrate(&layout, private, request_id, &request);
    }
    propose(&layout, private, request_id, &request)
}

/// Without `--request-id`, the id is derived from what the change says, so the
/// inspect-then-confirm round trip shares one id and two different changes
/// never collide on it. The revision, token and preview flag are left out:
/// they differ between the inspection and the confirmation.
fn default_request_id(layout: &ProjectLayout, request: &ChangeRequest) -> String {
    let input =
        format!(
        "{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{:?}\0{}\0{:?}\0{:?}\0{:?}",
        request.kind,
        request.record_id,
        request.content,
        request.definition,
        request.new_owner,
        request.rationale,
        request.source,
        request.expected_effect,
        request.impact,
        request.review.as_ref().map(|review| (&review.proposal, review.verdict)),
        request.retire,
        request.migrate,
        request.examples,
        request.conflicts,
        layout.project_id(),
    );
    format!(
        "change-{}",
        &format!("{:x}", Sha256::digest(input.as_bytes()))[..16]
    )
}

fn propose(
    layout: &ProjectLayout,
    private: &dyn RecordStore,
    request_id: String,
    request: &ChangeRequest,
) -> ServiceResponse {
    let id_text = request.record_id.as_deref().unwrap_or("change.pending");
    let record_id = match RecordId::new(id_text) {
        Ok(id) => id,
        Err(error) => return domain_response("change", request_id, error),
    };
    let current = match private.latest(&record_id) {
        Ok(current) => current,
        Err(error) => return storage_error("change", request_id, error),
    };
    let current_revision = current.as_ref().map_or(0, |record| record.revision);
    let resume = resume_token(
        layout.project_id(),
        "change",
        &format!("{request_id}:{id_text}"),
        current_revision,
    );
    let missing = [
        ("--kind", request.kind.is_none()),
        ("--record-id", request.record_id.is_none()),
        ("--content", request.content.is_none()),
        ("--rationale", request.rationale.is_none()),
    ]
    .into_iter()
    .filter_map(|(flag, absent)| absent.then_some(flag))
    .collect::<Vec<_>>();
    if !missing.is_empty() || request.expected_revision.is_none() {
        let question = if missing.is_empty() {
            format!(
                "Base revision {current_revision} is ready. Repeat the same request with --expected-revision {current_revision} --resume {resume} to record it (add --dry-run to preview)."
            )
        } else {
            format!(
                "Missing {}. Then repeat with --expected-revision {current_revision} --resume {resume}.",
                missing.join(", ")
            )
        };
        let mut response = needs_input(
            "change",
            request_id,
            current_revision,
            resume.clone(),
            question,
        );
        response.permitted_actions = vec![format!(
            "wh change --request-id {} --kind <mission|principle|rule> --record-id {} --content <text> --rationale <why> --expected-revision {current_revision} --resume {resume}",
            response.request_id, id_text
        )];
        response.data = json!({
            "current_record": current,
            "base_revision": current_revision,
            "effects": {
                "private_record_write": true,
                "private_proposal_write": true,
                "team_share": false,
                "platform_configuration_write": false,
                "requires_later_review_to_share": true
            }
        });
        return response;
    }
    let narrative = match change_narrative(request) {
        Ok(narrative) => narrative,
        Err(question) => {
            return needs_input("change", request_id, current_revision, resume, question)
        }
    };
    let body = match change_body(layout, request) {
        Ok(body) => body,
        Err(question) => {
            return needs_input("change", request_id, current_revision, resume, question)
        }
    };
    let existing = match private.by_idempotency_key(&request_id) {
        Ok(existing) => existing,
        Err(error) => return storage_error("change", request_id, error),
    };
    if let Some(existing) = existing.as_ref() {
        if existing.id != record_id || existing.body != body {
            return idempotency_conflict("change", request_id.clone(), &request_id);
        }
    }
    if existing.is_none()
        && (request.expected_revision != Some(current_revision)
            || request.resume_token.as_deref() != Some(resume.as_str()))
    {
        let summary = if request.expected_revision == Some(current_revision) {
            "The revision is current, but this exact change has not been confirmed: repeat the same command (same --request-id and content) with the returned --resume token."
        } else if request.expected_revision.is_none() {
            "Confirm this exact change: repeat the same command (same --request-id and content) with --expected-revision and --resume from this response."
        } else {
            "The change targets a stale agreement revision."
        };
        return stale_response("change", request_id, current_revision, resume, summary);
    }
    if request.preview {
        if existing.is_some() {
            return idempotency_conflict("change", request_id.clone(), &request_id);
        }
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            "Review the exact draft and its bound base revision before recording it.",
        );
        response.expected_revision = Some(current_revision);
        response.resume_token = Some(resume);
        response.blocking_questions = vec![
            "Does this exact before-and-after change express the intended decision and consequences?"
                .into(),
        ];
        response.permitted_actions = vec![
            "repeat this exact request without --dry-run to record it".into(),
            "cancel without changing project state".into(),
        ];
        response.data = json!({
            "preview_only": true,
            "record_id": record_id,
            "base_revision": current_revision,
            "diff": {
                "before": current.as_ref().map(|record| &record.body),
                "after": &body,
            },
            "explanation": narrative_json(&narrative, current.as_ref().map(|record| &record.owner)),
            "effects": {
                "private_record_write_on_confirm": true,
                "private_proposal_write_on_confirm": true,
                "team_share": false,
                "platform_configuration_write": false,
                "requires_later_review_to_share": true
            }
        });
        return response;
    }
    let idempotent_replay = existing.is_some();
    let base_revision = existing
        .as_ref()
        .map_or(current_revision, |record| record.revision.saturating_sub(1));
    let (record, reference) = if let Some(existing) = existing {
        let reference = match existing.reference() {
            Ok(reference) => reference,
            Err(error) => return domain_response("change", request_id, error),
        };
        (existing, reference)
    } else {
        let new_owner = request
            .new_owner
            .as_deref()
            .map(|owner| bounded_input(Some(owner.to_string()), "accountable owner", 200))
            .transpose();
        let new_owner = match new_owner {
            Ok(owner) => owner,
            Err(question) => {
                return needs_input("change", request_id, current_revision, resume, question)
            }
        };
        let owner_display = new_owner
            .or_else(|| {
                current
                    .as_ref()
                    .and_then(|record| record.owner.display_name.clone())
            })
            .or_else(|| mission_owner(private))
            .or_else(|| owner_name(layout.project_root()));
        match write_draft(
            layout,
            private,
            &record_id,
            request_id.clone(),
            body,
            owner_display.as_deref(),
            current.as_ref(),
            &narrative.source,
        ) {
            Ok(written) => written,
            Err(error) => return storage_error("change", request_id, error),
        }
    };
    let proposal = match ensure_local_change_proposal(
        private,
        &record,
        &reference,
        &narrative,
        base_revision,
        &request_id,
    ) {
        Ok(reference) => reference,
        Err(error) => return storage_error("change", request_id, error),
    };
    let before = match record.supersedes.as_ref() {
        Some(previous) => match private.get(previous) {
            Ok(previous) => previous.map(|record| record.body),
            Err(error) => return storage_error("change", request_id, error),
        },
        None => None,
    };
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Success,
        if idempotent_replay {
            "The existing draft was returned; no duplicate was created."
        } else {
            "The private draft was recorded. It is not in force until you accept it; nothing was shared."
        },
    );
    response.expected_revision = Some(record.revision);
    response.permitted_actions = vec![
        format!("wh change --accept {}", proposal.id.as_str()),
        format!("wh change --withdraw {}", proposal.id.as_str()),
    ];
    response.data = json!({
        "record": reference,
        "proposal": proposal,
        "shared": false,
        "recorded": true,
        "idempotent_replay": idempotent_replay,
        "base_revision": base_revision,
        "diff": {"before": before, "after": record.body},
        "explanation": narrative_json(&narrative, Some(&record.owner)),
    });
    response
}

fn narrative_json(
    narrative: &ChangeNarrative,
    owner: Option<&crate::domain::PrincipalRef>,
) -> serde_json::Value {
    json!({
        "rationale": narrative.rationale,
        "source": narrative.source,
        "expected_effect": narrative.expected_effect,
        "impact": narrative.impact,
        "examples": narrative.examples,
        "conflicts": narrative.conflicts,
        "owner": owner,
    })
}

fn mission_owner(private: &dyn RecordStore) -> Option<String> {
    RecordId::new(crate::projection::MISSION_ID)
        .ok()
        .and_then(|id| private.latest(&id).ok().flatten())
        .and_then(|record| record.owner.display_name)
}

/// Append a new draft revision of `record_id` on top of `current`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn write_draft(
    layout: &ProjectLayout,
    private: &dyn RecordStore,
    record_id: &RecordId,
    idempotency_key: String,
    body: RecordBody,
    owner_display: Option<&str>,
    current: Option<&AgreementRecord>,
    source: &str,
) -> Result<(AgreementRecord, RecordRef), StorageError> {
    let current_revision = current.map_or(0, |record| record.revision);
    let mut record = agreement_record(
        layout,
        record_id.as_str(),
        idempotency_key,
        body,
        owner_display,
    )
    .map_err(StorageError::Domain)?;
    record.revision = current_revision + 1;
    record.supersedes = current
        .map(AgreementRecord::reference)
        .transpose()
        .map_err(StorageError::Domain)?;
    record.provenance.sources = vec![EvidenceRef {
        system: "owner_selected_source".into(),
        locator: source.chars().take(2_000).collect(),
        digest: None,
    }];
    let reference = private.append(&record, (current_revision > 0).then_some(current_revision))?;
    Ok((record, reference))
}

/// Record one draft with its proposal in a single call, idempotently by key.
pub(crate) fn draft_with_proposal(
    layout: &ProjectLayout,
    private: &dyn RecordStore,
    key: &str,
    record_id: &RecordId,
    body: RecordBody,
    narrative: &ChangeNarrative,
) -> Result<(RecordRef, RecordRef), StorageError> {
    let existing = private.by_idempotency_key(key)?;
    let (record, reference) = match existing {
        Some(existing) => {
            let reference = existing.reference().map_err(StorageError::Domain)?;
            (existing, reference)
        }
        None => {
            let current = private.latest(record_id)?;
            let owner = current
                .as_ref()
                .and_then(|record| record.owner.display_name.clone())
                .or_else(|| mission_owner(private))
                .or_else(|| owner_name(layout.project_root()));
            write_draft(
                layout,
                private,
                record_id,
                key.to_string(),
                body,
                owner.as_deref(),
                current.as_ref(),
                &narrative.source,
            )?
        }
    };
    let base = record.revision.saturating_sub(1);
    let proposal =
        ensure_local_change_proposal(private, &record, &reference, narrative, base, key)?;
    Ok((reference, proposal))
}

fn change_narrative(request: &ChangeRequest) -> Result<ChangeNarrative, String> {
    Ok(ChangeNarrative {
        rationale: bounded_input(request.rationale.clone(), "rationale", 4_000)?,
        source: optional_input(request.source.clone(), "source", 2_048, "owner")?,
        expected_effect: optional_input(
            request.expected_effect.clone(),
            "expected effect",
            4_000,
            "not stated",
        )?,
        impact: optional_input(request.impact.clone(), "impact", 4_000, "not stated")?,
        examples: bounded_list(&request.examples, "examples", 16, 2_000)?,
        conflicts: bounded_list(&request.conflicts, "conflicts", 16, 2_000)?,
    })
}

fn change_body(layout: &ProjectLayout, request: &ChangeRequest) -> Result<RecordBody, String> {
    let kind = request.kind.ok_or_else(|| "Provide kind.".to_string())?;
    let content = bounded_input(request.content.clone(), "content", 16_000)?;
    let rationale = request
        .rationale
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Owner-authored change.")
        .trim()
        .to_string();
    Ok(match kind {
        ChangeKind::Mission => RecordBody::Mission(crate::domain::Mission {
            statement: content,
            desired_outcomes: Vec::new(),
        }),
        ChangeKind::Principle => {
            let (pstack, principle_rationale) = match request.definition.as_deref() {
                Some(ChangeDefinition::Principle { pstack, rationale }) => {
                    (pstack.clone(), rationale.clone())
                }
                None => (None, None),
                Some(_) => return Err(
                    "A principle's definition is {\"type\": \"principle\", \"pstack\": \"<id>\"}."
                        .into(),
                ),
            };
            let source = match pstack {
                Some(id) => {
                    let catalogue = crate::catalogue::catalogue();
                    if catalogue.get(&id).is_none() {
                        return Err(format!(
                            "{id} is not a principle in the catalogue (version {}); pick one of its ids or leave pstack out for your own.",
                            catalogue.version
                        ));
                    }
                    PrincipleSource::Pstack {
                        id,
                        version: catalogue.version,
                    }
                }
                None => PrincipleSource::Custom,
            };
            let principle = Principle {
                statement: content,
                source,
                rationale: principle_rationale.or(Some(rationale)),
            };
            RecordBody::Principle(principle)
        }
        ChangeKind::Rule => {
            let Some(ChangeDefinition::Rule {
                strength,
                enforcer,
                examples,
                source,
                paths,
                sweep,
                sweep_priority,
                sweep_unsure,
            }) = request.definition.as_deref()
            else {
                return Err("Provide the rule's strength and its one enforcer as a rule definition: {\"type\": \"rule\", \"strength\": \"must\", \"enforcer\": {\"kind\": \"test\", \"command\": \"cargo test\"}}.".into());
            };
            if let crate::domain::Enforcer::Test { command }
            | crate::domain::Enforcer::Validator { command } = enforcer
            {
                crate::gates::parse_command(command)?;
            }
            let source = source.clone().unwrap_or_else(RuleSource::owner);
            let rule = Rule {
                schema: RULE_SCHEMA_V2.into(),
                statement: content,
                rationale,
                strength: *strength,
                enforcer: enforcer.clone(),
                examples: examples.clone(),
                source,
                paths: paths.clone(),
                sweep: *sweep,
                sweep_priority: *sweep_priority,
                sweep_unsure: sweep_unsure.clone(),
            };
            // How far a sweep may reach over this rule's flags is a person's
            // call: an agent never widens it.
            if request.actor != crate::domain::ActorKind::Person {
                let current = request
                    .record_id
                    .as_deref()
                    .and_then(|id| RecordId::new(id).ok())
                    .and_then(|id| {
                        let state = state_of(layout).ok()?;
                        let class = state.in_force(&id)?.body.rule_view()?.sweep_class();
                        Some(class)
                    })
                    .unwrap_or_default();
                if rule.sweep_class().reach() > current.reach() {
                    return Err(format!(
                        "An agent cannot widen how a sweep works this rule (from {} to {}); a person sets the sweep class.",
                        current.label(),
                        rule.sweep_class().label()
                    ));
                }
            }
            rule.validate()
                .map_err(|error| format!("The rule is not valid: {error}"))?;
            RecordBody::Rule(rule)
        }
    })
}

fn state_of(layout: &ProjectLayout) -> Result<AgreementState, StorageError> {
    Ok(AgreementState::from_records(
        load_records(layout)?.union().0,
    ))
}

/// Earlier values and philosophy become principle drafts the owner reviews;
/// nothing is rewritten.
fn migrate(
    layout: &ProjectLayout,
    private: &dyn RecordStore,
    request_id: String,
    request: &ChangeRequest,
) -> ServiceResponse {
    let state = match state_of(layout) {
        Ok(state) => state,
        Err(error) => return storage_error("change", request_id, error),
    };
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
    let mut latest = std::collections::BTreeMap::<RecordId, &AgreementRecord>::new();
    for record in state.records() {
        if let RecordBody::Retired(retired) = &record.body {
            if retired.principle_text().is_some()
                && latest
                    .get(&record.id)
                    .map_or(true, |current| current.revision < record.revision)
            {
                latest.insert(record.id.clone(), record);
            }
        }
    }
    let candidates = latest
        .into_values()
        .filter(|record| !migrated.contains(&record.id))
        .collect::<Vec<_>>();
    // Rules from `whetstone/rules` become v2 rule drafts, once.
    let (legacy_rules, legacy_skipped) = crate::rules::legacy_rules_as_v2(layout.project_root());
    let legacy_rules = legacy_rules
        .into_iter()
        .filter(|(id, _, _)| {
            RecordId::new(id.as_str())
                .ok()
                .is_some_and(|id| state.latest(&id).is_none())
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() && legacy_rules.is_empty() {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::Success,
            "Nothing is waiting to migrate: no earlier value or philosophy, and every rule file is already a rule draft.",
        );
        response.data = json!({"drafts": [], "skipped": legacy_skipped});
        return response;
    }
    let mut drafts = Vec::new();
    for (id, rule, source) in legacy_rules {
        let Ok(record_id) = RecordId::new(id.as_str()) else {
            continue;
        };
        let body = RecordBody::Rule(rule);
        if request.preview {
            drafts.push(json!({"record": record_id, "after": body}));
            continue;
        }
        let narrative = ChangeNarrative {
            rationale: "Rules move from whetstone/rules into Beads; review the migrated rule and its examples before accepting it.".into(),
            source: source.clone(),
            expected_effect: "The rule is enforced from Beads once accepted; the rule file can then be retired.".into(),
            impact: "none until accepted".into(),
            examples: Vec::new(),
            conflicts: Vec::new(),
        };
        let key = format!("migrate:{request_id}:{id}");
        match draft_with_proposal(layout, private, &key, &record_id, body, &narrative) {
            Ok((reference, proposal)) => {
                drafts.push(json!({"record": reference, "proposal": proposal, "from": source}))
            }
            Err(error) => return storage_error("change", request_id, error),
        }
    }
    for record in candidates {
        let RecordBody::Retired(retired) = &record.body else {
            continue;
        };
        let Some((statement, note)) = retired.principle_text() else {
            continue;
        };
        let Ok(reference) = record.reference() else {
            continue;
        };
        let slug = record.id.as_str().replace(['.', ':'], "-");
        let Ok(id) = RecordId::new(format!("principle.{slug}")) else {
            continue;
        };
        let body = RecordBody::Principle(Principle {
            statement,
            source: PrincipleSource::Migrated { record: reference },
            rationale: note,
        });
        if request.preview {
            drafts.push(json!({"record": id, "after": body}));
            continue;
        }
        let narrative = ChangeNarrative {
            rationale: format!(
                "The delegation plan replaces {} with principles; review this wording before accepting it.",
                retired.record_type.replace('_', " ")
            ),
            source: format!("migration from {}", record.id.as_str()),
            expected_effect: "A principle rules can cite as their source.".into(),
            impact: "none until accepted".into(),
            examples: Vec::new(),
            conflicts: Vec::new(),
        };
        let key = format!("migrate:{request_id}:{}", record.id.as_str());
        match draft_with_proposal(layout, private, &key, &id, body, &narrative) {
            Ok((reference, proposal)) => {
                drafts.push(json!({"record": reference, "proposal": proposal}))
            }
            Err(error) => return storage_error("change", request_id, error),
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        if request.preview {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "{} earlier record(s) (values, philosophy or whetstone/rules YAML) {} drafts (principles and rules); accept, edit or withdraw each.",
            drafts.len(),
            if request.preview { "would become" } else { "became" }
        ),
    );
    response.permitted_actions = vec!["wh dash".into(), "wh change --accept <proposal>".into()];
    response.data = json!({"drafts": drafts, "preview_only": request.preview});
    response
}

pub(crate) fn review_draft(
    private: &dyn RecordStore,
    request_id: String,
    review: &ReviewRequest,
    request: &ChangeRequest,
) -> ServiceResponse {
    use crate::domain::{LocalReview, LocalReviewVerdict, LOCAL_REVIEW_ASSURANCE};
    let key = format!("review:{request_id}:{}", review.proposal);
    match private.by_idempotency_key(&key) {
        Ok(Some(existing)) => {
            let mut response = ServiceResponse::new(
                request_id,
                "change",
                ServiceState::Success,
                "The existing review was returned; no duplicate was created.",
            );
            response.data = json!({"review": existing.reference().ok(), "idempotent_replay": true});
            return response;
        }
        Ok(None) => {}
        Err(error) => return storage_error("change", request_id, error),
    }
    let records = match private.all_records() {
        Ok(records) => records,
        Err(error) => return storage_error("change", request_id, error),
    };
    let state = AgreementState::from_records(records);
    let pending = state.pending_proposals();
    let Some(target) = pending
        .iter()
        .find(|pending| pending.proposal.id.as_str() == review.proposal)
    else {
        let mut response = needs_input(
            "change",
            request_id,
            0,
            String::new(),
            if pending.is_empty() {
                format!(
                    "{} is not a pending draft; nothing awaits review.",
                    review.proposal
                )
            } else {
                format!(
                    "{} is not a pending draft. Pending: {}.",
                    review.proposal,
                    pending
                        .iter()
                        .map(|pending| pending.proposal.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            },
        );
        response.expected_revision = None;
        response.resume_token = None;
        response.data = json!({
            "pending": pending.iter().map(|pending| json!({
                "proposal": pending.proposal.id.as_str(),
                "title": crate::projection::record_title(pending.proposal),
            })).collect::<Vec<_>>(),
        });
        return response;
    };
    let proposal_ref = match target.proposal.reference() {
        Ok(reference) => reference,
        Err(error) => return domain_response("change", request_id, error),
    };
    let rationale = request
        .rationale
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(
            || match review.verdict {
                LocalReviewVerdict::Accept => "Explicit solo acceptance by the owner.".to_string(),
                LocalReviewVerdict::Withdraw => "Withdrawn by the owner.".to_string(),
            },
            |value| value.chars().take(4_000).collect(),
        );
    let reviewer = target.proposal.owner.clone();
    let suffix = &format!("{:x}", Sha256::digest(key.as_bytes()))[..24];
    let record = AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: match RecordId::new(format!("review.{suffix}")) {
            Ok(id) => id,
            Err(error) => return domain_response("change", request_id, error),
        },
        revision: 1,
        scope: target.proposal.scope.clone(),
        owner: reviewer.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: reviewer.clone(),
            recorded_at: utc_now(),
            sources: vec![EvidenceRef {
                system: "whetstone_cli".into(),
                locator: "explicit_owner_review".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: key,
        body: RecordBody::LocalReview(LocalReview {
            proposal: proposal_ref.clone(),
            verdict: review.verdict,
            reviewer,
            assurance: LOCAL_REVIEW_ASSURANCE.into(),
            rationale: rationale.clone(),
            reviewed_at: utc_now(),
            team_activation_permitted: false,
            cites: Vec::new(),
        }),
    };
    let candidates = target
        .candidates
        .iter()
        .map(|candidate| {
            json!({
                "record": candidate.id.as_str(),
                "version": candidate.revision,
                "title": crate::projection::record_title(candidate),
                "replaces": state.in_force(&candidate.id).map(|current| current.revision),
            })
        })
        .collect::<Vec<_>>();
    let verb = match review.verdict {
        LocalReviewVerdict::Accept => "accept",
        LocalReviewVerdict::Withdraw => "withdraw",
    };
    if request.preview {
        let mut response = ServiceResponse::new(
            request_id,
            "change",
            ServiceState::NeedsDecision,
            format!("Review the exact effect before you {verb} this draft."),
        );
        response.permitted_actions = vec!["repeat without --dry-run to record the review".into()];
        response.data = json!({
            "preview_only": true,
            "proposal": proposal_ref,
            "verdict": verb,
            "candidates": candidates,
            "effects": {
                "private_record_write_on_confirm": true,
                "becomes_in_force": review.verdict == LocalReviewVerdict::Accept,
                "team_share": false,
            },
        });
        return response;
    }
    let reference = match private.append(&record, None) {
        Ok(reference) => reference,
        Err(error) => return storage_error("change", request_id, error),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Success,
        match review.verdict {
            LocalReviewVerdict::Accept => {
                "The draft was accepted and is now in force for this agreement; nothing was shared."
            }
            LocalReviewVerdict::Withdraw => {
                "The draft was withdrawn; the previously accepted record stays in force."
            }
        },
    );
    response.permitted_actions = vec!["wh check".into(), "wh init --action wire".into()];
    response.data = json!({
        "review": reference,
        "proposal": proposal_ref,
        "verdict": verb,
        "candidates": candidates,
        "rationale": rationale,
        "shared": false,
    });
    response
}

/// Record a private draft that retires an accepted record. Accepting it takes
/// the record out of force; every revision stays in history.
pub(crate) fn retire_record(
    layout: &ProjectLayout,
    private: &dyn RecordStore,
    request_id: String,
    target: &str,
    request: &ChangeRequest,
) -> ServiceResponse {
    let target_id = match RecordId::new(target) {
        Ok(id) => id,
        Err(error) => return domain_response("change", request_id, error),
    };
    let records = match private.all_records() {
        Ok(records) => records,
        Err(error) => return storage_error("change", request_id, error),
    };
    let state = AgreementState::from_records(records);
    let Some(current) = state.in_force(&target_id) else {
        return needs_input(
            "change",
            request_id,
            0,
            String::new(),
            format!("{target} is not an accepted record in force, so there is nothing to retire."),
        );
    };
    let rationale = match bounded_input(request.rationale.clone(), "rationale", 4_000) {
        Ok(rationale) => rationale,
        Err(question) => return needs_input("change", request_id, 0, String::new(), question),
    };
    let target_ref = match current.reference() {
        Ok(reference) => reference,
        Err(error) => return domain_response("change", request_id, error),
    };
    let retirement_id = {
        let candidate = format!("retirement.{target}");
        if candidate.len() <= 96 {
            candidate
        } else {
            format!(
                "retirement.{}",
                &format!("{:x}", Sha256::digest(target.as_bytes()))[..32]
            )
        }
    };
    let retirement_id = match RecordId::new(retirement_id) {
        Ok(id) => id,
        Err(error) => return domain_response("change", request_id, error),
    };
    let existing = match private.by_idempotency_key(&request_id) {
        Ok(existing) => existing,
        Err(error) => return storage_error("change", request_id, error),
    };
    let body = RecordBody::Retirement(crate::domain::Retirement {
        target: target_ref.clone(),
        reason: rationale.clone(),
        replacement: None,
    });
    let previous = state.latest(&retirement_id).cloned();
    let (record, reference) = match existing {
        Some(existing) if existing.id == retirement_id && existing.body == body => {
            match existing.reference() {
                Ok(reference) => (existing, reference),
                Err(error) => return domain_response("change", request_id, error),
            }
        }
        Some(_) => return idempotency_conflict("change", request_id.clone(), &request_id),
        None => {
            let mut record = match agreement_record(
                layout,
                retirement_id.as_str(),
                request_id.clone(),
                body,
                current.owner.display_name.as_deref(),
            ) {
                Ok(record) => record,
                Err(error) => return domain_response("change", request_id, error),
            };
            record.revision = previous.as_ref().map_or(1, |record| record.revision + 1);
            record.supersedes = match previous.as_ref().map(AgreementRecord::reference) {
                Some(Ok(reference)) => Some(reference),
                Some(Err(error)) => return domain_response("change", request_id, error),
                None => None,
            };
            if request.preview {
                let mut response = ServiceResponse::new(
                    request_id,
                    "change",
                    ServiceState::NeedsDecision,
                    format!("Review the exact effect before you retire {target}."),
                );
                response.permitted_actions =
                    vec!["repeat without --dry-run to record the retirement draft".into()];
                response.data = json!({
                    "preview_only": true,
                    "retire": target_ref,
                    "title": crate::projection::record_title(current),
                    "effects": {
                        "private_record_write_on_confirm": true,
                        "leaves_force_on_accept": true,
                        "history_preserved": true,
                        "team_share": false,
                    },
                });
                return response;
            }
            let reference =
                match private.append(&record, previous.as_ref().map(|record| record.revision)) {
                    Ok(reference) => reference,
                    Err(error) => return storage_error("change", request_id, error),
                };
            (record, reference)
        }
    };
    let narrative = ChangeNarrative {
        rationale,
        source: "owner".into(),
        expected_effect: format!("{target} leaves force and the sweep; its history stays."),
        impact: "not stated".into(),
        examples: Vec::new(),
        conflicts: Vec::new(),
    };
    let proposal = match ensure_local_change_proposal(
        private,
        &record,
        &reference,
        &narrative,
        current.revision,
        &request_id,
    ) {
        Ok(reference) => reference,
        Err(error) => return storage_error("change", request_id, error),
    };
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::Success,
        format!(
            "A private draft to retire {target} was recorded. It stays in force until you accept the retirement; history is never deleted."
        ),
    );
    response.permitted_actions = vec![
        format!("wh change --accept {}", proposal.id.as_str()),
        format!("wh change --withdraw {}", proposal.id.as_str()),
    ];
    response.data = json!({
        "record": reference,
        "proposal": proposal,
        "retires": target_ref,
        "title": crate::projection::record_title(current),
        "shared": false,
    });
    response
}
