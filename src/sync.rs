//! `wh push` and `wh pull`: private-safe sharing through the repository's
//! shared Beads database.
//!
//! Push copies an exact, confirmed package of accepted records (with the
//! proposals and reviews that accepted them) from the private store into the
//! shared store and then runs `bd dolt push`. The package, destination and
//! base are bound into the confirmation token, so any change invalidates it.
//! A record whose revision chain contains a draft or withdrawn revision
//! carries private ancestry and is blocked unless its chain is explicitly
//! selected; a record whose links point at something unshared is blocked
//! until that dependency is shared too. Pull runs `bd dolt pull`, verifies
//! every incoming record, reports what arrived and any conflict with local
//! drafts, and never executes, activates or accepts anything.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agreement::{AgreementState, Lifecycle};
use crate::domain::{AgreementRecord, Enforcement, RecordBody, RecordId, RecordRef};
use crate::service::{ServiceResponse, ServiceState};
use crate::storage::{ProjectLayout, RecordStore, StorageError, StoreKind};

#[derive(Debug, Clone, Default)]
pub struct PushRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    /// Record ids to share; empty means every accepted record not yet shared.
    pub select: Vec<String>,
    /// Record ids whose draft or withdrawn ancestry the owner chose to share.
    pub include_ancestry: Vec<String>,
    /// Text that must never leave this machine; any match blocks the push.
    pub canaries: Vec<String>,
    pub confirm: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PullRequest {
    pub project_dir: PathBuf,
    pub request_id: Option<String>,
    pub dry_run: bool,
}

fn response(
    request_id: String,
    workflow: &str,
    state: ServiceState,
    summary: impl Into<String>,
) -> ServiceResponse {
    ServiceResponse::new_public(request_id, workflow, state, summary)
}

fn storage_failure(request_id: String, workflow: &str, error: StorageError) -> ServiceResponse {
    crate::service::storage_error(workflow, request_id, error)
}

fn reference_text(reference: &RecordRef) -> String {
    format!(
        "{}@{}#{}",
        reference.id.as_str(),
        reference.revision,
        reference.digest.as_str()
    )
}

/// Every record id a record depends on to make sense on another machine.
fn links(record: &AgreementRecord) -> Vec<RecordId> {
    match &record.body {
        RecordBody::Feature(feature) => feature
            .serves
            .iter()
            .chain(&feature.constrained_by)
            .chain(&feature.proven_by)
            .cloned()
            .collect(),
        RecordBody::Standard(standard) => match &standard.enforcement {
            Enforcement::Drive { feature } => vec![feature.clone()],
            _ => Vec::new(),
        },
        RecordBody::Retirement(retirement) => vec![retirement.target.id.clone()],
        _ => Vec::new(),
    }
}

struct Package {
    /// Records to copy, in the order they must be applied.
    records: Vec<RecordRef>,
    /// Human view of each shared record id.
    items: Vec<Value>,
    blocked: Vec<Value>,
}

/// The exact package for a push: accepted revisions with their acceptance
/// evidence, minus what the shared store already has.
fn build_package(
    state: &AgreementState,
    shared: &BTreeSet<RecordRef>,
    shared_ids: &BTreeSet<RecordId>,
    request: &PushRequest,
) -> Package {
    let include_ancestry = request
        .include_ancestry
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut ids = state
        .agreement_ids(|_| true)
        .into_iter()
        .filter(|id| state.in_force(id).is_some())
        .collect::<Vec<_>>();
    // Accepted retirements travel too; they are part of the history.
    for record in state.records() {
        if matches!(record.body, RecordBody::Retirement(_))
            && state.lifecycle_of(record) == Lifecycle::Accepted
            && !ids.contains(&record.id)
        {
            ids.push(record.id.clone());
        }
    }
    if !request.select.is_empty() {
        ids.retain(|id| {
            request
                .select
                .iter()
                .any(|selected| selected == id.as_str())
        });
    }
    let mut records = Vec::<RecordRef>::new();
    let mut seen = BTreeSet::<RecordRef>::new();
    let mut items = Vec::new();
    let mut blocked = Vec::new();
    let mut packaged_ids = BTreeSet::<RecordId>::new();
    for id in &ids {
        let top = state.in_force(id).or_else(|| {
            state
                .records()
                .iter()
                .filter(|record| &record.id == id)
                .max_by_key(|record| record.revision)
        });
        let Some(top) = top else {
            continue;
        };
        let chain = state
            .records()
            .iter()
            .filter(|record| &record.id == id && record.revision <= top.revision)
            .collect::<Vec<_>>();
        let private_ancestry = chain
            .iter()
            .filter(|record| state.lifecycle_of(record) != Lifecycle::Accepted)
            .map(|record| {
                format!(
                    "v{} ({})",
                    record.revision,
                    state.lifecycle_of(record).label()
                )
            })
            .collect::<Vec<_>>();
        if !private_ancestry.is_empty() && !include_ancestry.contains(id.as_str()) {
            blocked.push(json!({
                "record": id.as_str(),
                "reason": format!(
                    "its revision history includes unshared {}; share that ancestry explicitly with --include-ancestry {} or retire the record and author a shareable replacement",
                    private_ancestry.join(", "),
                    id.as_str()
                ),
            }));
            continue;
        }
        let mut added = Vec::new();
        for record in &chain {
            let Ok(reference) = record.reference() else {
                continue;
            };
            let mut evidence = Vec::new();
            if let Some(proposal) = state.proposal_for(record) {
                if let Ok(proposal_ref) = proposal.reference() {
                    evidence.push(proposal_ref.clone());
                    for review in state.records() {
                        if let RecordBody::LocalReview(body) = &review.body {
                            if body.proposal == proposal_ref {
                                if let Ok(review_ref) = review.reference() {
                                    evidence.push(review_ref);
                                }
                            }
                        }
                    }
                    // A proposal's own earlier revisions stay with it.
                    for revision in state.records().iter().filter(|candidate| {
                        candidate.id == proposal.id && candidate.revision < proposal.revision
                    }) {
                        if let Ok(revision_ref) = revision.reference() {
                            evidence.push(revision_ref);
                        }
                    }
                }
            }
            for item in std::iter::once(reference).chain(evidence) {
                if !shared.contains(&item) && seen.insert(item.clone()) {
                    added.push(item);
                }
            }
        }
        if added.is_empty() {
            continue;
        }
        packaged_ids.insert(id.clone());
        items.push(json!({
            "record": id.as_str(),
            "revision": top.revision,
            "title": crate::projection::record_title(top),
            "kind": top.body.type_name(),
            "includes_ancestry": !private_ancestry.is_empty(),
            "records": added.iter().map(reference_text).collect::<Vec<_>>(),
        }));
        records.extend(added);
    }
    // Links must resolve on the other side: shared already, or in this push.
    let mut unresolved = BTreeMap::<String, Vec<String>>::new();
    for reference in &records {
        let Some(record) = state.get(reference) else {
            continue;
        };
        for link in links(record) {
            if !shared_ids.contains(&link) && !packaged_ids.contains(&link) {
                unresolved
                    .entry(record.id.as_str().to_string())
                    .or_default()
                    .push(link.as_str().to_string());
            }
        }
    }
    if !unresolved.is_empty() {
        let dropped = unresolved.keys().cloned().collect::<BTreeSet<_>>();
        records.retain(|reference| {
            let owner = state
                .get(reference)
                .and_then(|record| match &record.body {
                    RecordBody::Proposal(body) => body
                        .proposed_records
                        .first()
                        .map(|candidate| candidate.id.as_str().to_string()),
                    RecordBody::LocalReview(body) => {
                        state
                            .get(&body.proposal)
                            .and_then(|proposal| match &proposal.body {
                                RecordBody::Proposal(body) => body
                                    .proposed_records
                                    .first()
                                    .map(|candidate| candidate.id.as_str().to_string()),
                                _ => None,
                            })
                    }
                    _ => Some(reference.id.as_str().to_string()),
                })
                .unwrap_or_default();
            !dropped.contains(&owner)
        });
        items.retain(|item| !dropped.contains(item["record"].as_str().unwrap_or_default()));
        for (record, missing) in unresolved {
            blocked.push(json!({
                "record": record,
                "reason": format!(
                    "it links to {} which the shared database does not have and this push does not include; share it too",
                    missing.join(", ")
                ),
            }));
        }
    }
    Package {
        records,
        items,
        blocked,
    }
}

fn digest(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    format!("sha256:{:x}", hasher.finalize())
}

pub fn push(request: PushRequest) -> ServiceResponse {
    let fallback = request.request_id.clone().unwrap_or_else(|| "push".into());
    let layout = match ProjectLayout::resolve(&request.project_dir, None) {
        Ok(layout) => layout,
        Err(error) => return storage_failure(fallback, "push", error),
    };
    let request_id = fallback;
    if !layout.private_store_exists() {
        let mut response = response(
            request_id,
            "push",
            ServiceState::NeedsInput,
            "There is no private agreement to share yet; nothing was published.",
        );
        response.permitted_actions = vec!["wh init".into()];
        return response;
    }
    let Some(shared_dir) = layout.shared_store() else {
        let mut response = response(
            request_id,
            "push",
            ServiceState::Unavailable,
            "This repository has no shared Beads database, so there is nowhere to share to; nothing was published.",
        );
        response.blocking_questions = vec![
            "Create the team's shared database with `bd init` in the repository root and add the team remote with `bd dolt remote add origin <url>`, then push again.".into(),
        ];
        response.permitted_actions = vec!["keep working locally".into()];
        return response;
    };
    let private = match RecordStore::open_existing(&layout.private_store(), StoreKind::Private) {
        Ok(store) => store,
        Err(error) => return storage_failure(request_id, "push", error),
    };
    let shared = match RecordStore::initialize(&shared_dir, StoreKind::Shareable) {
        Ok(store) => store,
        Err(error) => return storage_failure(request_id, "push", error),
    };
    let (private_records, shared_records) = match (private.all_records(), shared.all_records()) {
        (Ok(private), Ok(shared)) => (private, shared),
        (Err(error), _) | (_, Err(error)) => return storage_failure(request_id, "push", error),
    };
    let shared_refs = shared_records
        .iter()
        .filter_map(|record| record.reference().ok())
        .collect::<BTreeSet<_>>();
    let shared_ids = shared_records
        .iter()
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    let state = AgreementState::from_records(private_records);
    let package = build_package(&state, &shared_refs, &shared_ids, &request);
    let remotes = shared.remotes().unwrap_or_default();
    let destination = json!({
        "database": shared_dir.display().to_string(),
        "remotes": remotes,
    });
    let base = digest(
        &shared_refs
            .iter()
            .map(reference_text)
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let package_digest = digest(
        &package
            .records
            .iter()
            .map(reference_text)
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let token = digest(&[
        layout.project_id(),
        &package_digest,
        &destination.to_string(),
        &base,
    ])["sha256:".len()..][..32]
        .to_string();
    // Canaries: nothing selected may contain text that must stay private.
    for reference in &package.records {
        if let Some(record) = state.get(reference) {
            let canonical = record.canonical_json().unwrap_or_default();
            for canary in request.canaries.iter().filter(|canary| !canary.is_empty()) {
                if canonical
                    .windows(canary.len())
                    .any(|window| window == canary.as_bytes())
                {
                    let mut response = response(
                        request_id,
                        "push",
                        ServiceState::NeedsDecision,
                        format!(
                            "{} contains private text that must not leave this machine; nothing was published.",
                            record.id.as_str()
                        ),
                    );
                    response.permitted_actions = vec![format!(
                        "leave {} out with --select, or change it through wh change",
                        record.id.as_str()
                    )];
                    return response;
                }
            }
        }
    }
    let data = json!({
        "package": package.items,
        "package_digest": package_digest,
        "records": package.records.iter().map(reference_text).collect::<Vec<_>>(),
        "blocked": package.blocked,
        "destination": destination,
        "base": base,
        "private_store_has_remote": false,
    });
    if package.records.is_empty() {
        // Nothing new to copy, but an earlier push may have copied without
        // reaching the remote: transmitting again is idempotent.
        let transmitted = if remotes.is_empty() || request.dry_run {
            None
        } else {
            Some(shared.push_remote().map_err(|error| error.to_string()))
        };
        let mut response = response(
            request_id,
            "push",
            match (&transmitted, package.blocked.is_empty()) {
                (Some(Err(_)), _) => ServiceState::Unavailable,
                (_, true) => ServiceState::Success,
                (_, false) => ServiceState::NeedsDecision,
            },
            match (&transmitted, package.blocked.is_empty()) {
                (Some(Err(reason)), _) => format!(
                    "Nothing new to share, but the shared database could not reach its remote: {reason}"
                ),
                (_, true) => "Everything accepted is already shared and the shared database is pushed; nothing new was published.".into(),
                (_, false) => "Nothing can be shared as selected; the blocked records say why. Nothing was published.".into(),
            },
        );
        let mut data = data;
        data["transmitted"] = json!(matches!(transmitted, Some(Ok(_))));
        response.data = data;
        return response;
    }
    if request.dry_run || request.confirm.as_deref() != Some(token.as_str()) {
        let stale = request.confirm.is_some() && request.confirm.as_deref() != Some(token.as_str());
        let mut response = response(
            request_id,
            "push",
            if stale {
                ServiceState::Stale
            } else {
                ServiceState::NeedsDecision
            },
            if stale {
                "The package, destination or base changed since you reviewed it; review the new package before confirming. Nothing was published."
                    .to_string()
            } else {
                format!(
                    "Review the exact package: {} record revision(s) for {} record(s) would be shared. Nothing was published.",
                    package.records.len(),
                    package.items.len()
                )
            },
        );
        response.resume_token = Some(token.clone());
        response.blocking_questions =
            vec!["Is this exactly what the team should receive, at this destination?".into()];
        response.permitted_actions = vec![format!("wh push --confirm {token}")];
        response.data = data;
        return response;
    }
    let copied = match private.copy_to(&shared, &package.records, &[]) {
        Ok(copied) => copied,
        Err(error) => return storage_failure(request_id, "push", error),
    };
    // Prove the private canaries are absent from the whole shared store.
    if !request.canaries.is_empty() {
        if let Ok(records) = shared.all_records() {
            for record in records {
                let canonical = record.canonical_json().unwrap_or_default();
                if request
                    .canaries
                    .iter()
                    .filter(|canary| !canary.is_empty())
                    .any(|canary| {
                        canonical
                            .windows(canary.len())
                            .any(|window| window == canary.as_bytes())
                    })
                {
                    return response(
                        request_id,
                        "push",
                        ServiceState::Unknown,
                        format!("Private text was found in shared record {}; the remote push was not attempted.", record.id.as_str()),
                    );
                }
            }
        }
    }
    let transmitted = if remotes.is_empty() {
        Err("the shared database has no remote; the records are shared locally only".to_string())
    } else {
        shared.push_remote().map_err(|error| error.to_string())
    };
    let mut response = response(
        request_id,
        "push",
        if transmitted.is_ok() {
            ServiceState::Success
        } else {
            ServiceState::Unavailable
        },
        match &transmitted {
            Ok(_) => format!(
                "Shared {} record revision(s) and pushed the shared database to its remote.",
                copied.len()
            ),
            Err(reason) => format!(
                "Copied {} record revision(s) into the shared database, but {reason}. Repeat wh push once the remote is reachable; the copy is not repeated.",
                copied.len()
            ),
        },
    );
    response.permitted_actions = vec!["wh dash".into()];
    let mut data = data;
    data["copied"] = json!(copied.iter().map(reference_text).collect::<Vec<_>>());
    data["transmitted"] = json!(transmitted.is_ok());
    data["transmit_detail"] = json!(transmitted.err());
    response.data = data;
    response
}

pub fn pull(request: PullRequest) -> ServiceResponse {
    let request_id = request.request_id.clone().unwrap_or_else(|| "pull".into());
    let layout = match ProjectLayout::resolve(&request.project_dir, None) {
        Ok(layout) => layout,
        Err(error) => return storage_failure(request_id, "pull", error),
    };
    let Some(shared_dir) = layout.shared_store() else {
        let mut response = response(
            request_id,
            "pull",
            ServiceState::Unavailable,
            "This repository has no shared Beads database; there is nothing to pull. Run `bd bootstrap` on a fresh clone of a repository that shares one.",
        );
        response.permitted_actions = vec!["keep working locally".into()];
        return response;
    };
    let shared = match RecordStore::open_existing(&shared_dir, StoreKind::Shareable) {
        Ok(store) => store,
        Err(error) => return storage_failure(request_id, "pull", error),
    };
    let before = match shared.all_records() {
        Ok(records) => records,
        Err(error) => return storage_failure(request_id, "pull", error),
    };
    let remotes = shared.remotes().unwrap_or_default();
    if request.dry_run || remotes.is_empty() {
        let mut response = response(
            request_id,
            "pull",
            if remotes.is_empty() {
                ServiceState::Unavailable
            } else {
                ServiceState::NeedsDecision
            },
            if remotes.is_empty() {
                "The shared database has no remote to pull from; nothing was received.".to_string()
            } else {
                format!(
                    "Dry run: wh pull would run bd dolt pull from {}; nothing was received.",
                    remotes.join(", ")
                )
            },
        );
        response.data = json!({"remotes": remotes, "shared_records": before.len(), "executes": false, "activates": false});
        return response;
    }
    let pulled = shared.pull_remote();
    let after = match pulled.and_then(|_| shared.all_records()) {
        Ok(records) => records,
        Err(error) => return storage_failure(request_id, "pull", error),
    };
    let before_refs = before
        .iter()
        .filter_map(|record| record.reference().ok())
        .collect::<BTreeSet<_>>();
    let arrived = after
        .iter()
        .filter(|record| {
            record
                .reference()
                .is_ok_and(|reference| !before_refs.contains(&reference))
        })
        .map(|record| {
            json!({
                "record": record.id.as_str(),
                "revision": record.revision,
                "kind": record.body.type_name(),
                "title": crate::projection::record_title(record),
            })
        })
        .collect::<Vec<_>>();
    let conflicts = match crate::service::load_records(&layout) {
        Ok(loaded) => loaded.union().1,
        Err(_) => Vec::new(),
    };
    let mut response = response(
        request_id,
        "pull",
        if conflicts.is_empty() {
            ServiceState::Success
        } else {
            ServiceState::Conflict
        },
        if conflicts.is_empty() {
            format!(
                "Received {} record revision(s). Nothing was executed, activated or accepted; local drafts are unchanged.",
                arrived.len()
            )
        } else {
            format!(
                "Received {} record revision(s); {} local revision(s) now collide with shared ones and are held back until you rebase them with wh change. Nothing was executed or activated.",
                arrived.len(),
                conflicts.len()
            )
        },
    );
    response.permitted_actions = vec!["wh dash".into(), "wh check".into()];
    response.data = json!({
        "arrived": arrived,
        "conflicts": conflicts,
        "executes": false,
        "activates": false,
        "local_drafts_preserved": true,
    });
    response
}
