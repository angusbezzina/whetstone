//! `wh init --action import`: return edits to a verification skill (from
//! pstack's maintain pass, `/reflect` or a person) as private drafts.

use std::fs;
use std::path::Path;

use serde_json::json;

use crate::agreement::AgreementState;
use crate::domain::{RecordBody, RecordId};
use crate::feature_map;
use crate::service::{InitRequest, ServiceResponse, ServiceState};
use crate::storage::{ProjectLayout, RecordStore, StoreKind};

use super::render::map_conventions;
use super::{app_slug, digest, interview, DRIVER_CONFIG_RELATIVE, MAP_ID};

/// Every private record, or none when no private store exists yet.
fn private_records(layout: &ProjectLayout) -> Result<Vec<crate::domain::AgreementRecord>, String> {
    let store = layout.private_store();
    if !crate::beads::is_initialized(&store) {
        return Ok(Vec::new());
    }
    RecordStore::open_existing(&store, StoreKind::Private)
        .and_then(|repository| repository.all_records())
        .map_err(|error| error.to_string())
}

/// Whether this import request already recorded the record (a replay).
fn replayed(state: &AgreementState, request_id: &str, id: &str) -> bool {
    let key = format!("import:{request_id}:{id}");
    state
        .records()
        .iter()
        .any(|record| record.idempotency_key == key)
}

/// One record the importer would record as a private draft.
struct ImportCandidate {
    id: RecordId,
    body: RecordBody,
    source: String,
    source_digest: String,
}

fn import_error(request_id: String, state: ServiceState, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new_public(request_id, "init", state, summary);
    response.permitted_actions =
        vec!["wh init --action import --from <verify-skill directory> --dry-run".into()];
    response
}

/// Read a pstack verification skill directory (SKILL.md, features/README.md,
/// features/*.md, and Whetstone's rules/*.md) into private drafts: feature
/// records, the map's conventions and edited rules. Nothing is accepted; each
/// draft carries an exact before/after for the owner. Re-importing a
/// Whetstone-generated skill after pstack's maintain pass or `/reflect`
/// records only what changed.
pub fn import(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
) -> ServiceResponse {
    let root = layout.project_root();
    let Some(from) = request.import_from.as_ref() else {
        return import_error(
            request_id,
            ServiceState::NeedsInput,
            "Name the verification skill directory with --from <dir> (it holds SKILL.md and features/).".into(),
        );
    };
    let directory = if from.is_absolute() {
        from.clone()
    } else {
        root.join(from)
    };
    let directory = match directory.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => {
            return import_error(
                request_id,
                ServiceState::NeedsInput,
                format!("{} is not a readable directory.", from.display()),
            )
        }
    };
    let features_dir = directory.join("features");
    if !features_dir.is_dir() {
        return import_error(
            request_id,
            ServiceState::NeedsInput,
            format!(
                "{} has no features/ directory; it is not a pstack verification skill.",
                directory.display()
            ),
        );
    }
    let read = |path: &Path| -> Result<(String, String), String> {
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > 512 * 1024 {
            return Err(format!(
                "{} is not a regular file under 512 KiB",
                path.display()
            ));
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| format!("{} is not UTF-8", path.display()))?;
        Ok((text, digest(&bytes)))
    };
    let relative = |path: &Path| -> String {
        path.strip_prefix(root).map_or_else(
            |_| path.display().to_string(),
            |path| path.display().to_string(),
        )
    };
    let records = match private_records(layout) {
        Ok(records) => records,
        Err(error) => {
            return import_error(
                request_id,
                ServiceState::Unknown,
                format!("The private agreement could not be read: {error}"),
            )
        }
    };
    let state = AgreementState::from_records(records);
    let config = fs::read_to_string(root.join(DRIVER_CONFIG_RELATIVE))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_else(|| interview(root));
    let app = app_slug(root);

    let mut candidates = Vec::<ImportCandidate>::new();
    let mut unchanged = Vec::new();
    let mut preserved = Vec::new();
    let readme_path = features_dir.join("README.md");
    let readme = if readme_path.is_file() {
        match read(&readme_path).and_then(|(text, digest)| {
            feature_map::parse_readme(&text).map(|parsed| (parsed, digest))
        }) {
            Ok(readme) => Some(readme),
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        }
    } else {
        None
    };
    let mut skill_notes = Vec::new();
    let skill_path = directory.join("SKILL.md");
    if skill_path.is_file() {
        match read(&skill_path) {
            Ok((text, _)) => {
                let generated = text.contains("<!-- Generated by Whetstone from agreement");
                skill_notes = if generated {
                    feature_map::parse_imported_notes(&text)
                } else {
                    feature_map::parse_skill(&text).1
                };
            }
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        }
    }
    if let Some((parsed, readme_digest)) = &readme {
        let mut map = parsed.map.clone();
        if map.entry_contract.as_deref() == Some(feature_map::STANDARD_ENTRY_CONTRACT) {
            map.entry_contract = None;
        }
        map.skill_notes.extend(skill_notes.iter().cloned());
        preserved.extend(parsed.preserved.iter().cloned());
        let map_id = RecordId::new(MAP_ID).expect("constant");
        let effective = state
            .pending(&map_id)
            .or_else(|| state.in_force(&map_id))
            .and_then(|record| match &record.body {
                RecordBody::VerificationMap(map) => Some(map.clone()),
                _ => None,
            })
            .unwrap_or_else(|| map_conventions(&state, &app, &config));
        if map == effective && !replayed(&state, &request_id, MAP_ID) {
            unchanged.push(json!({"record": MAP_ID, "file": relative(&readme_path)}));
        } else {
            candidates.push(ImportCandidate {
                id: RecordId::new(MAP_ID).expect("constant"),
                body: RecordBody::VerificationMap(map),
                source: relative(&readme_path),
                source_digest: readme_digest.clone(),
            });
        }
    }
    let mut files = match fs::read_dir(&features_dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|extension| extension == "md")
                    && path.file_name().is_some_and(|name| name != "README.md")
            })
            .collect::<Vec<_>>(),
        Err(error) => return import_error(request_id, ServiceState::Unknown, error.to_string()),
    };
    files.sort();
    if files.len() > 200 {
        return import_error(
            request_id,
            ServiceState::NeedsInput,
            "The map has more than 200 feature files; import it in parts.".into(),
        );
    }
    for path in &files {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let (text, file_digest) = match read(path) {
            Ok(read) => read,
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        };
        let mut parsed = match feature_map::parse_feature(&stem, &text) {
            Ok(parsed) => parsed,
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        };
        if let Some((readme, _)) = &readme {
            if let Some((index, entry)) = readme
                .entries
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.0 == stem)
            {
                let default_line = format!("— {}", parsed.feature.summary.trim());
                if entry.2 != default_line && !entry.2.trim().is_empty() {
                    parsed.feature.index_summary = Some(entry.2.clone());
                }
                if parsed.feature.sweep_order == 0 {
                    parsed.feature.sweep_order = u32::try_from(index + 1).unwrap_or(u32::MAX);
                }
                if let Some(area) = &entry.3 {
                    parsed.feature.area = area.clone();
                }
            } else {
                preserved.push(format!("{stem}.md is not listed in features/README.md"));
            }
        }
        preserved.extend(
            parsed
                .preserved
                .iter()
                .map(|note| format!("{stem}.md: {note}")),
        );
        // A feature that names a proving gate nobody has recorded, and has
        // executable steps, gets a draft drive gate to review with it.
        if !parsed.feature.drive_steps.is_empty() {
            for gate in &parsed.feature.proven_by {
                let exists = state.in_force(gate).is_some()
                    || state.pending(gate).is_some()
                    || candidates.iter().any(|candidate| &candidate.id == gate);
                if exists && !replayed(&state, &request_id, gate.as_str()) {
                    continue;
                }
                let standard = RecordBody::Rule(crate::domain::Rule {
                    schema: crate::domain::RULE_SCHEMA_V2.into(),
                    statement: format!("{} is proven by driving it", parsed.feature.name),
                    rationale: format!(
                        "Proposed with the imported feature map: the drive steps in {stem}.md, run by the project's driver, prove the feature from the user's path."
                    ),
                    strength: crate::domain::Strength::Must,
                    enforcer: crate::domain::Enforcer::Drive {
                        feature: parsed.id.clone(),
                    },
                    examples: Vec::new(),
                    source: crate::domain::RuleSource {
                        kind: crate::domain::RuleSourceKind::Owner,
                        reference: Some(format!("import:{stem}.md")),
                        provenance: Vec::new(),
                    },
                    paths: Vec::new(),
                    hand_raise: Vec::new(),
                    privacy: crate::domain::Privacy::default(),
                });
                candidates.push(ImportCandidate {
                    id: gate.clone(),
                    body: standard,
                    source: relative(path),
                    source_digest: file_digest.clone(),
                });
            }
        }
        let body = RecordBody::Feature(parsed.feature);
        if let Err(error) = crate::domain::RecordBody::validate(&body) {
            return import_error(
                request_id,
                ServiceState::NeedsInput,
                format!("{stem}.md does not make a valid feature record: {error}"),
            );
        }
        let current = state
            .pending(&parsed.id)
            .or_else(|| state.in_force(&parsed.id));
        if current.is_some_and(|record| record.body == body)
            && !replayed(&state, &request_id, parsed.id.as_str())
        {
            unchanged.push(json!({"record": parsed.id.as_str(), "file": relative(path)}));
            continue;
        }
        candidates.push(ImportCandidate {
            id: parsed.id,
            body,
            source: relative(path),
            source_digest: file_digest,
        });
    }
    // Rule files: an edit by /reflect or a person is one draft; an
    // unchanged file records nothing.
    let rules_dir = directory.join("rules");
    if rules_dir.is_dir() {
        let mut rule_files = match fs::read_dir(&rules_dir) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
                .collect::<Vec<_>>(),
            Err(error) => {
                return import_error(request_id, ServiceState::Unknown, error.to_string())
            }
        };
        rule_files.sort();
        for path in rule_files.into_iter().take(200) {
            let (text, file_digest) = match read(&path) {
                Ok(read) => read,
                Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
            };
            let parsed = match super::rules::parse_rule_file(&text) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return import_error(
                        request_id,
                        ServiceState::NeedsInput,
                        format!("{} is not a valid rule file: {error}", relative(&path)),
                    )
                }
            };
            let current = state
                .pending(&parsed.id)
                .or_else(|| state.in_force(&parsed.id))
                .and_then(|record| record.body.rule_view().map(std::borrow::Cow::into_owned));
            if current.as_ref() == Some(&parsed.rule)
                && !replayed(&state, &request_id, parsed.id.as_str())
            {
                unchanged.push(json!({"record": parsed.id.as_str(), "file": relative(&path)}));
                continue;
            }
            candidates.push(ImportCandidate {
                id: parsed.id,
                body: RecordBody::Rule(parsed.rule),
                source: relative(&path),
                source_digest: file_digest,
            });
        }
    }
    let plan = candidates
        .iter()
        .map(|candidate| {
            let before = state
                .pending(&candidate.id)
                .or_else(|| state.in_force(&candidate.id))
                .map(|record| &record.body);
            json!({
                "record": candidate.id.as_str(),
                "file": candidate.source,
                "file_digest": candidate.source_digest,
                "diff": {"before": before, "after": &candidate.body},
            })
        })
        .collect::<Vec<_>>();
    if request.dry_run || candidates.is_empty() {
        let mut response = ServiceResponse::new_public(
            request_id,
            "init",
            if candidates.is_empty() {
                ServiceState::Success
            } else {
                ServiceState::NeedsDecision
            },
            if candidates.is_empty() {
                format!(
                    "Nothing to import: {} record(s) already match the imported files.",
                    unchanged.len()
                )
            } else {
                format!(
                    "Dry run: {} private draft(s) would be recorded; nothing was written or accepted.",
                    candidates.len()
                )
            },
        );
        response.permitted_actions = if candidates.is_empty() {
            vec!["wh init --action wire".into()]
        } else {
            vec!["repeat without --dry-run to record exactly these drafts".into()]
        };
        response.data = json!({
            "dry_run": request.dry_run,
            "drafts": plan,
            "unchanged": unchanged,
            "preserved_as_gotchas": preserved,
            "accepted": [],
        });
        return response;
    }
    let private = match crate::storage::RecordStore::initialize(
        &layout.private_store(),
        StoreKind::Private,
    ) {
        Ok(private) => private,
        Err(error) => {
            return crate::service::record_storage_error("init", request_id, error);
        }
    };
    let mut drafts = Vec::new();
    for candidate in &candidates {
        let key = format!("import:{request_id}:{}", candidate.id.as_str());
        let existing = match private.by_idempotency_key(&key) {
            Ok(existing) => existing,
            Err(error) => return crate::service::record_storage_error("init", request_id, error),
        };
        let (record, reference) = match existing {
            Some(existing) if existing.body == candidate.body => match existing.reference() {
                Ok(reference) => (existing, reference),
                Err(error) => return crate::service::record_domain_response("init", request_id, error),
            },
            Some(_) => {
                return import_error(
                    request_id,
                    ServiceState::Conflict,
                    format!("This import request id already recorded different content for {}; use a new --request-id.", candidate.id.as_str()),
                )
            }
            None => {
                let latest = match private.latest(&candidate.id) {
                    Ok(latest) => latest,
                    Err(error) => return crate::service::record_storage_error("init", request_id, error),
                };
                let owner = state
                    .in_force(&RecordId::new(crate::projection::MISSION_ID).expect("constant"))
                    .and_then(|record| record.owner.display_name.clone());
                let mut record = match crate::service::agreement_record(
                    layout,
                    candidate.id.as_str(),
                    key.clone(),
                    candidate.body.clone(),
                    owner.as_deref(),
                ) {
                    Ok(record) => record,
                    Err(error) => return crate::service::record_domain_response("init", request_id, error),
                };
                record.revision = latest.as_ref().map_or(1, |record| record.revision + 1);
                record.supersedes = match latest.as_ref().map(crate::domain::AgreementRecord::reference) {
                    Some(Ok(reference)) => Some(reference),
                    Some(Err(error)) => return crate::service::record_domain_response("init", request_id, error),
                    None => None,
                };
                record.provenance.kind = crate::domain::ProvenanceKind::ImportedNote;
                record.provenance.authority = crate::domain::ProvenanceAuthority::CandidateOnly;
                record.provenance.sources = vec![crate::domain::EvidenceRef {
                    system: "verification_skill_import".into(),
                    locator: candidate.source.clone(),
                    digest: crate::domain::ContentDigest::new(candidate.source_digest.clone()).ok(),
                }];
                let reference = match private.append(&record, latest.as_ref().map(|record| record.revision)) {
                    Ok(reference) => reference,
                    Err(error) => return crate::service::record_storage_error("init", request_id, error),
                };
                (record, reference)
            }
        };
        let narrative = crate::service::record::ChangeNarrative {
            rationale: format!("Imported from {} for review.", candidate.source),
            source: candidate.source.clone(),
            expected_effect: "The verification map matches the imported skill once accepted."
                .into(),
            impact: "not stated".into(),
            examples: Vec::new(),
            conflicts: Vec::new(),
        };
        let proposal = match crate::service::record::ensure_local_change_proposal(
            &private,
            &record,
            &reference,
            &narrative,
            record.revision.saturating_sub(1),
            &key,
        ) {
            Ok(proposal) => proposal,
            Err(error) => return crate::service::record_storage_error("init", request_id, error),
        };
        drafts.push(json!({
            "record": reference,
            "proposal": proposal,
            "file": candidate.source,
        }));
    }
    let mut response = ServiceResponse::new_public(
        request_id,
        "init",
        ServiceState::NeedsDecision,
        format!(
            "{} private draft(s) were recorded from the imported skill. Nothing is accepted until you review each one.",
            drafts.len()
        ),
    );
    response.permitted_actions = drafts
        .iter()
        .filter_map(|draft| draft["proposal"]["id"].as_str())
        .map(|proposal| format!("wh change --accept {proposal}"))
        .collect();
    response.data = json!({
        "drafts": drafts,
        "plan": plan,
        "unchanged": unchanged,
        "preserved_as_gotchas": preserved,
        "accepted": [],
        "shared": false,
    });
    response
}
