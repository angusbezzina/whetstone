//! The Virgil adapter: every record in the attached repository's `rulebook`
//! Records collection, reached only through `virgil rulebook` on the daemon
//! socket. The daemon stamps provenance and syncs; Whetstone never talks to a
//! door or a database, and never needs `bd`.
//!
//! One adapter serves both store kinds through the collection's two scopes:
//! `repository` (accepted records and their decisions, shared with the group
//! through the repository attachment) and `private` (drafts and receipts,
//! owned by the signed-in principal for this repository and visible to no one
//! else). Accepting a draft copies it from `private` to `repository`; nothing
//! copies the other way except migration.
//!
//! The contract Whetstone pins (`virgil.rulebook/v1`): every verb reads one
//! JSON request on `--input -` and answers one JSON line,
//! `{"api_version","result"}` or `{"api_version","error":{"code","message",
//! "hint"}}`, with Virgil's exit codes (0 ok, 1 internal, 2 invalid, 3 read
//! again, 4 refused, 5 not permitted, 6 not found or unattached, 7
//! unavailable). Every request names its `"scope"` (`repository` or
//! `private`); the repository is the attachment found from the checkout
//! (`.virgil/repo.toml`), and no verb names a group.
//!
//! - `put` `{"scope","record","expected_revision"?,"copy"?}`: one revision,
//!   unique on `record_id`+`revision` and on `idempotency_key` per scope,
//!   answered `{"id","record_id","revision","digest","replayed",
//!   "replication_pending"}`; Whetstone requires `id` and the stored `digest`
//!   and treats a landed write whose digest differs as a mismatch.
//!   `expected_revision` is the latest revision the writer saw, `null`
//!   meaning none may exist yet (it is never "skip the check"); a stale one
//!   is exit 3 with `error.code` `stale_revision` and the current revision as
//!   `error.actual` (a number or `null`). A repeated key answers what it
//!   recorded (`replayed`); a reused key or revision with other content is
//!   exit 4 `idempotency_conflict` or `conflicting_revision`. `copy: true`
//!   writes an exact revision without the expected-revision check
//!   (accepting, sharing and migration) and is the only way to skip it.
//! - `get` `{"scope","record_id","revision"}`, `latest`
//!   `{"scope","record_id"}`: one `record`, exit 6 `not_found` when there is
//!   none.
//! - `history` `{"scope","record_id"}`: `records`, by revision.
//! - `list` `{"scope","limit","cursor","idempotency_key"?}`: a page of
//!   `records` and the next `cursor`; the first request sends `"cursor": ""`
//!   and an empty cursor in the answer ends the listing.
//! - `mark` `{"scope","marker","members"}` and `markers` `{"scope"}`: batch
//!   completion markers.
//! - `lifecycle` `{"scope","record_id","revision","lifecycle"}`: the
//!   lifecycle view.
//! - `import-beads` `{"scope":"repository","file","apply"}`: the export
//!   Whetstone writes ([`crate::store::export`]), answered with one entry per
//!   revision.
//!
//! A record on the wire is `{"id","record_id","revision","digest",
//! "supersedes","idempotency_key","kind","type","lifecycle","wh_record"}`;
//! `wh_record` is the ASCII-escaped canonical JSON, verified against
//! `digest` on every read. Exit codes are mapped to Whetstone's errors, never
//! passed through. A write whose result carries a record id landed, even at
//! exit 7 (replication still arriving) or 3 to 5 (a follow-up refused).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::domain::{AgreementRecord, RecordId, RecordRef};
use crate::storage::{
    batch_marker_key, check_append, check_batch, decode_metadata, initial_lifecycle,
    pending_batches, record_class, reference_text, resolved_lifecycle, validate_for_append,
    AppendRequest, IncompleteBatch, RecordStore, StorageError, StoreBackend, StoreKind,
    METADATA_SCHEMA,
};

pub const RULEBOOK_API: &str = "virgil.rulebook/v1";
const PAGE: u64 = 100;
const MAX_PAGES: usize = 100;

/// One rulebook record as Virgil holds it, verified.
#[derive(Debug, Clone)]
pub struct WireRecord {
    /// Virgil's record id.
    pub id: String,
    pub record: AgreementRecord,
    pub reference: RecordRef,
    pub lifecycle: String,
}

/// The wire form of one revision, for `put` and the export.
pub fn wire(record: &AgreementRecord, lifecycle: &str) -> Result<Value, StorageError> {
    let reference = record.reference().map_err(StorageError::Domain)?;
    let metadata = crate::storage::record_metadata(record, &reference)?;
    Ok(json!({
        "record_id": record.id.as_str(),
        "revision": record.revision,
        "digest": reference.digest.as_str(),
        "supersedes": record.supersedes.as_ref().map(reference_text),
        "idempotency_key": record.idempotency_key,
        "kind": record_class(&record.body),
        "type": record.body.type_name(),
        "lifecycle": lifecycle,
        "wh_record": metadata["wh_record"],
    }))
}

/// Read and verify one wire record.
pub fn read_wire(value: &Value) -> Result<WireRecord, StorageError> {
    let id = value["id"].as_str().unwrap_or_default().to_string();
    let locator = if id.is_empty() {
        format!(
            "virgil:{}@{}",
            value["record_id"].as_str().unwrap_or("?"),
            value["revision"]
        )
    } else {
        format!("virgil:{id}")
    };
    let malformed = |reason: String| StorageError::MalformedRecord {
        locator: locator.clone(),
        reason,
    };
    let metadata = json!({
        "wh_schema": METADATA_SCHEMA,
        "wh_id": value["record_id"],
        "wh_revision": value["revision"],
        "wh_digest": value["digest"],
        "wh_key": value["idempotency_key"],
        "wh_record": value["wh_record"],
    });
    let record = decode_metadata(&metadata).map_err(malformed)?;
    let supersedes = record.supersedes.as_ref().map(reference_text);
    if value["supersedes"].as_str() != supersedes.as_deref() {
        return Err(malformed(
            "the supersedes field disagrees with the record".into(),
        ));
    }
    let reference = record.reference().map_err(StorageError::Domain)?;
    Ok(WireRecord {
        id,
        lifecycle: value["lifecycle"]
            .as_str()
            .map_or_else(|| initial_lifecycle(&record).to_string(), str::to_owned),
        record,
        reference,
    })
}

/// A failed answer as a storage error.
fn failure(op: &str, answer: &crate::memory::virgil::Answer) -> StorageError {
    let error = &answer.line["error"];
    let code = error["code"].as_str().unwrap_or_default();
    let message = error["message"]
        .as_str()
        .filter(|message| !message.trim().is_empty())
        .map_or_else(
            || {
                if answer.stderr.is_empty() {
                    "no detail".to_string()
                } else {
                    answer.stderr.clone()
                }
            },
            str::to_owned,
        );
    let hint = error["hint"]
        .as_str()
        .filter(|hint| !hint.trim().is_empty())
        .map(|hint| format!(" ({hint})"))
        .unwrap_or_default();
    let virgil = |code: &str, message: String| StorageError::Virgil {
        code: code.to_string(),
        message,
    };
    match (answer.exit, code) {
        (6, "unattached" | "repository_missing") => virgil(
            "unattached",
            format!("This checkout is not attached to a Virgil repository ({message}), and the store switch names Virgil. Attach it (virgil repos attach <repository> --group <group> writes .virgil/repo.toml), or switch back with wh init --action setup --store beads. Nothing was written."),
        ),
        (6, _) => virgil("not_found", format!("virgil rulebook {op}: {message}")),
        (2, _) => virgil("invalid_request", format!("virgil rulebook {op}: {message}{hint}")),
        (3, _) => virgil(
            "read_again",
            format!("virgil rulebook {op}: the record moved since it was read ({message}); read it again and repeat. Nothing was retried."),
        ),
        (4, _) => virgil("refused_content", format!("virgil rulebook {op}: {message}{hint}")),
        (5, _) => virgil("not_permitted", format!("virgil rulebook {op}: {message}{hint}")),
        (7, _) => virgil("store_unavailable", format!("virgil rulebook {op}: {message}{hint}")),
        _ => virgil("store_refused", format!("virgil rulebook {op} failed: {message}{hint}")),
    }
}

/// Which scope of the `rulebook` collection a store reads and writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirgilScope {
    /// Accepted records and decisions, shared through the attachment.
    Repository,
    /// Drafts and receipts, the signed-in principal's own.
    Private,
}

impl VirgilScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Repository => "repository",
            Self::Private => "private",
        }
    }
}

/// One scope of the attached repository's `rulebook` collection.
#[derive(Debug, Clone)]
pub struct VirgilStore {
    root: PathBuf,
    scope: VirgilScope,
}

impl VirgilStore {
    /// The shared store: the repository scope.
    pub fn shared(project_root: &Path) -> Self {
        Self {
            root: project_root.to_path_buf(),
            scope: VirgilScope::Repository,
        }
    }

    /// The private store: the signed-in principal's scope.
    pub fn private(project_root: &Path) -> Self {
        Self {
            root: project_root.to_path_buf(),
            scope: VirgilScope::Private,
        }
    }

    pub fn scope(&self) -> VirgilScope {
        self.scope
    }

    /// `VIRGIL_VIA` for an agent's writes, none for a person's.
    fn via() -> Option<String> {
        (crate::memory::detect_actor() == crate::domain::ActorKind::Agent).then(|| {
            format!(
                "agent:{}",
                crate::memory::virgil_client(&crate::memory::agent_host())
            )
        })
    }

    /// Run one verb: `Ok(answer)` whatever the exit, so writes can tell a
    /// landed write from a refused one.
    fn invoke(
        &self,
        op: &str,
        request: &Value,
        write: bool,
    ) -> Result<crate::memory::virgil::Answer, StorageError> {
        let args = [
            "rulebook".to_string(),
            op.to_string(),
            "--input".into(),
            "-".into(),
        ];
        let via = if write { Self::via() } else { None };
        let mut request = request.clone();
        request["scope"] = json!(self.scope.as_str());
        let answer = crate::memory::virgil::invoke(
            &self.root,
            &args,
            Some(&request.to_string()),
            via.as_deref(),
        )
        .map_err(|refusal| StorageError::Virgil {
            code: refusal.code.to_string(),
            message: refusal.message,
        })?;
        if !answer.line.is_object() && answer.exit != 0 {
            return Err(StorageError::Virgil {
                code: "store_contract".into(),
                message: format!(
                    "virgil rulebook {op} answered no JSON line (exit {}: {}); this Virgil may not have the rulebook collection ({RULEBOOK_API})",
                    answer.exit, answer.stderr
                ),
            });
        }
        if answer.line.is_object() && answer.line["api_version"].as_str() != Some(RULEBOOK_API) {
            return Err(StorageError::Virgil {
                code: "store_contract".into(),
                message: format!(
                    "virgil rulebook {op} answered {} where Whetstone pins {RULEBOOK_API}",
                    answer.line["api_version"]
                ),
            });
        }
        Ok(answer)
    }

    /// A read: the result, `None` when Virgil answers not found.
    fn read(&self, op: &str, request: &Value) -> Result<Option<Value>, StorageError> {
        let answer = self.invoke(op, request, false)?;
        if answer.exit != 0 || answer.line.get("error").is_some() {
            let error = failure(op, &answer);
            return match &error {
                StorageError::Virgil { code, .. } if code == "not_found" => Ok(None),
                _ => Err(error),
            };
        }
        answer
            .line
            .get("result")
            .cloned()
            .map(Some)
            .ok_or_else(|| StorageError::Virgil {
                code: "store_contract".into(),
                message: format!("virgil rulebook {op} answered without a result line"),
            })
    }

    /// Every record in scope (all pages), optionally one idempotency key.
    pub fn listing(&self, key: Option<&str>) -> Result<Vec<WireRecord>, StorageError> {
        let mut agreements = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_PAGES {
            let mut request = json!({"limit": PAGE, "cursor": cursor});
            if let Some(key) = key {
                request["idempotency_key"] = json!(key);
            }
            let page = self.read("list", &request)?.unwrap_or_else(|| json!({}));
            let items = page["records"]
                .as_array()
                .ok_or_else(|| StorageError::Virgil {
                    code: "store_contract".into(),
                    message: "virgil rulebook list returned no records".into(),
                })?;
            for item in items {
                agreements.push(read_wire(item)?);
            }
            cursor = page["cursor"].as_str().unwrap_or_default().to_string();
            if cursor.is_empty() {
                agreements.sort_by(|left, right| {
                    left.record
                        .id
                        .cmp(&right.record.id)
                        .then_with(|| left.record.revision.cmp(&right.record.revision))
                });
                // One revision of one record is one Virgil record.
                for pair in agreements.windows(2) {
                    if pair[0].record.id == pair[1].record.id
                        && pair[0].record.revision == pair[1].record.revision
                    {
                        return Err(StorageError::ConflictingRevision {
                            id: pair[0].record.id.clone(),
                            revision: pair[0].record.revision,
                            beads: vec![pair[0].id.clone(), pair[1].id.clone()],
                        });
                    }
                }
                return Ok(agreements);
            }
        }
        Err(StorageError::Virgil {
            code: "store_contract".into(),
            message: format!("more than {MAX_PAGES} pages of rulebook records; the list stopped"),
        })
    }

    fn one(&self, op: &str, request: &Value) -> Result<Option<WireRecord>, StorageError> {
        match self.read(op, request)? {
            Some(result) if !result["record"].is_null() => read_wire(&result["record"]).map(Some),
            _ => Ok(None),
        }
    }

    /// One revision as Virgil confirms it.
    pub fn fetch(&self, id: &RecordId, revision: u64) -> Result<Option<WireRecord>, StorageError> {
        self.one(
            "get",
            &json!({"record_id": id.as_str(), "revision": revision}),
        )
    }

    /// Put one revision; the landed record's Virgil id.
    fn put(
        &self,
        record: &AgreementRecord,
        reference: &RecordRef,
        expected_revision: Option<u64>,
        copy: bool,
    ) -> Result<String, StorageError> {
        let mut request = json!({"record": wire(record, initial_lifecycle(record))?});
        if copy {
            request["copy"] = json!(true);
        } else {
            request["expected_revision"] = json!(expected_revision);
        }
        let answer = self.invoke("put", &request, true)?;
        let result = &answer.line["result"];
        if let Some(id) = result["id"].as_str().filter(|id| !id.trim().is_empty()) {
            if result["digest"].as_str() != Some(reference.digest.as_str()) {
                return Err(StorageError::DigestMismatch(record.id.clone()));
            }
            return Ok(id.to_string());
        }
        if answer.exit == 0 {
            return Err(StorageError::Virgil {
                code: "store_contract".into(),
                message: "virgil rulebook put answered without a record id".into(),
            });
        }
        let error = &answer.line["error"];
        Err(match (answer.exit, error["code"].as_str()) {
            (3, Some("stale_revision")) => StorageError::StaleRevision {
                expected: expected_revision,
                actual: error["actual"].as_u64(),
            },
            (4, Some("idempotency_conflict")) => {
                StorageError::IdempotencyConflict(record.idempotency_key.clone())
            }
            (4, Some("conflicting_revision")) => StorageError::ConflictingRevision {
                id: record.id.clone(),
                revision: record.revision,
                beads: Vec::new(),
            },
            _ => failure("put", &answer),
        })
    }

    /// A follow-up write (`mark`, `lifecycle`): one that answers a result
    /// landed, even at exit 7 (replication still arriving).
    fn write_answer(&self, op: &str, request: &Value) -> Result<(), StorageError> {
        let answer = self.invoke(op, request, true)?;
        let landed = answer.line.get("error").is_none()
            && answer
                .line
                .get("result")
                .is_some_and(|result| !result.is_null());
        if landed && matches!(answer.exit, 0 | 7) {
            return Ok(());
        }
        Err(failure(op, &answer))
    }

    /// Whether the daemon answers the pinned rulebook contract.
    pub fn probe(&self) -> Result<(), StorageError> {
        self.markers().map(|_| ())
    }

    fn markers(&self) -> Result<BTreeSet<String>, StorageError> {
        let result = self
            .read("markers", &json!({}))?
            .unwrap_or_else(|| json!({}));
        Ok(result["markers"]
            .as_array()
            .map(|markers| {
                markers
                    .iter()
                    .filter_map(|marker| marker["marker"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `virgil rulebook import-beads`: what the export would do (or did,
    /// with `apply`), one entry per revision.
    pub fn import_beads(&self, file: &Path, apply: bool) -> Result<Vec<Value>, StorageError> {
        if self.scope != VirgilScope::Repository {
            return Err(StorageError::InvalidProjectionBoundary);
        }
        let request = json!({"file": file.display().to_string(), "apply": apply});
        let answer = self.invoke("import-beads", &request, apply)?;
        if answer.exit != 0 || answer.line.get("error").is_some() {
            return Err(failure("import-beads", &answer));
        }
        Ok(answer.line["result"]["entries"]
            .as_array()
            .cloned()
            .unwrap_or_default())
    }
}

impl RecordStore for VirgilStore {
    fn backend(&self) -> StoreBackend {
        StoreBackend::Virgil
    }

    fn kind(&self) -> StoreKind {
        match self.scope {
            VirgilScope::Repository => StoreKind::Shareable,
            VirgilScope::Private => StoreKind::Private,
        }
    }

    fn location(&self) -> String {
        format!("virgil:{}:{}", self.scope.as_str(), self.root.display())
    }

    fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        Ok(self
            .listing(None)?
            .into_iter()
            .map(|agreement| agreement.record)
            .collect())
    }

    fn get(&self, reference: &RecordRef) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .fetch(&reference.id, reference.revision)?
            .filter(|agreement| &agreement.reference == reference)
            .map(|agreement| agreement.record))
    }

    fn by_idempotency_key(&self, key: &str) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .listing(Some(key))?
            .into_iter()
            .map(|agreement| agreement.record)
            .find(|record| record.idempotency_key == key))
    }

    fn latest(&self, id: &RecordId) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .one("latest", &json!({"record_id": id.as_str()}))?
            .map(|agreement| agreement.record))
    }

    fn history(&self, id: &RecordId) -> Result<Vec<AgreementRecord>, StorageError> {
        let result = self
            .read("history", &json!({"record_id": id.as_str()}))?
            .unwrap_or_else(|| json!({"records": []}));
        let mut records = result["records"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|item| read_wire(item).map(|agreement| agreement.record))
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by_key(|record| record.revision);
        Ok(records)
    }

    fn locator(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        Ok(self
            .fetch(&reference.id, reference.revision)?
            .filter(|agreement| &agreement.reference == reference)
            .map(|agreement| format!("virgil:{}", agreement.id)))
    }

    fn incomplete_batches(&self) -> Result<Vec<IncompleteBatch>, StorageError> {
        let records = self.all_records()?;
        Ok(pending_batches(&records, &self.markers()?))
    }

    fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        let reference = validate_for_append(record)?;
        // check_append reads only the key's record and the id's history, so
        // a write never lists the whole scope.
        let mut existing = self
            .listing(Some(&record.idempotency_key))?
            .into_iter()
            .map(|item| item.record)
            .filter(|item| item.idempotency_key == record.idempotency_key)
            .collect::<Vec<_>>();
        existing.extend(self.history(&record.id)?);
        if let Some(already) = check_append(&existing, record, &reference, expected_revision)? {
            return Ok(already);
        }
        // Virgil checks the expected revision and both uniqueness rules
        // again, so a writer racing this one is refused, never merged.
        self.put(record, &reference, expected_revision, false)?;
        Ok(reference)
    }

    fn append_batch(&self, requests: &[AppendRequest<'_>]) -> Result<Vec<RecordRef>, StorageError> {
        if requests.is_empty() {
            return Err(StorageError::EmptyBatch);
        }
        for request in requests {
            validate_for_append(request.record)?;
        }
        let marker_key = batch_marker_key(requests);
        let mut existing = self.all_records()?;
        check_batch(&existing, requests)?;
        let mut references = Vec::new();
        for request in requests {
            let reference = request.record.reference().map_err(StorageError::Domain)?;
            if check_append(
                &existing,
                request.record,
                &reference,
                request.expected_revision,
            )?
            .is_none()
            {
                self.put(request.record, &reference, request.expected_revision, false)?;
                existing.push(request.record.clone());
            }
            references.push(reference);
        }
        if !self.markers()?.contains(&marker_key) {
            self.write_answer(
                "mark",
                &json!({
                    "marker": marker_key,
                    "members": requests.iter().map(|request| request.record.idempotency_key.clone()).collect::<Vec<_>>(),
                }),
            )?;
        }
        Ok(references)
    }

    fn sync_lifecycle_labels(
        &self,
        state: &crate::agreement::AgreementState,
    ) -> Result<usize, StorageError> {
        let mut changed = 0;
        for agreement in self.listing(None)? {
            let wanted = resolved_lifecycle(state, &agreement.record);
            if agreement.lifecycle != wanted {
                self.write_answer(
                    "lifecycle",
                    &json!({
                        "record_id": agreement.record.id.as_str(),
                        "revision": agreement.record.revision,
                        "lifecycle": wanted,
                    }),
                )?;
                changed += 1;
            }
        }
        Ok(changed)
    }

    fn lifecycle_view(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        Ok(self
            .fetch(&reference.id, reference.revision)?
            .filter(|agreement| &agreement.reference == reference)
            .map(|agreement| agreement.lifecycle))
    }

    fn receive(&self, records: &[AgreementRecord]) -> Result<Vec<RecordRef>, StorageError> {
        let mut present = self.listing(None)?;
        let mut copied = Vec::new();
        for record in records {
            record.validate().map_err(StorageError::Domain)?;
            let reference = record.reference().map_err(StorageError::Domain)?;
            if present.iter().any(|item| item.reference == reference) {
                continue;
            }
            if present
                .iter()
                .any(|item| item.record.id == record.id && item.record.revision == record.revision)
            {
                return Err(StorageError::ConflictingRevision {
                    id: record.id.clone(),
                    revision: record.revision,
                    beads: Vec::new(),
                });
            }
            let id = self.put(record, &reference, None, true)?;
            present.push(WireRecord {
                id,
                record: record.clone(),
                reference: reference.clone(),
                lifecycle: initial_lifecycle(record).to_string(),
            });
            copied.push(reference);
        }
        Ok(copied)
    }
}
