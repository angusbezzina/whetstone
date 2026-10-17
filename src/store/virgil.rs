//! The Virgil adapter: accepted records and decisions in the attached
//! group's Records `agreement` collection, reached only through
//! `virgil agreements` on the daemon socket. The daemon stamps provenance and
//! syncs; Whetstone never talks to a door or a database.
//!
//! The contract Whetstone pins (`virgil.agreements/v1`): every verb reads one
//! JSON request on `--input -` and answers one JSON line,
//! `{"api_version","result"}` or `{"api_version","error":{"code","message",
//! "hint"}}`, with Virgil's exit codes (0 ok, 1 internal, 2 invalid, 3 read
//! again, 4 refused, 5 not permitted, 6 not found or unattached, 7
//! unavailable). The scope is the repository attachment found from the
//! checkout (`.virgil/repo.toml`); no verb names a group.
//!
//! - `put` `{"agreement", "expected_revision"?, "copy"?}`: one revision,
//!   unique on `record_id`+`revision` and on `idempotency_key` per
//!   repository, answered `{"id","record_id","revision","digest",
//!   "replayed","replication_pending"}`; Whetstone requires `id` and the
//!   stored `digest` and treats a landed write whose digest differs as a
//!   mismatch. `expected_revision` is the latest revision the writer saw,
//!   `null` meaning none may exist yet (it is never "skip the check"); a
//!   stale one is exit 3 with `error.code` `stale_revision` and the current
//!   revision as `error.actual` (a number or `null`). A repeated key answers
//!   what it recorded (`replayed`); a reused key or revision with other
//!   content is exit 4 `idempotency_conflict` or `conflicting_revision`.
//!   `copy: true` writes an exact revision without the expected-revision
//!   check (migration and sharing) and is the only way to skip it.
//! - `get` `{"record_id","revision"}`, `latest` `{"record_id"}`: one
//!   `agreement`, exit 6 `not_found` when there is none.
//! - `history` `{"record_id"}`: `agreements`, by revision.
//! - `list` `{"limit","cursor","idempotency_key"?}`: a page of `agreements`
//!   and the next `cursor`; the first request sends `"cursor": ""` and an
//!   empty cursor in the answer ends the listing.
//! - `mark` `{"marker","members"}` and `markers` `{}`: batch completion
//!   markers.
//! - `lifecycle` `{"record_id","revision","lifecycle"}`: the lifecycle view.
//! - `import-beads` `{"file","apply"}`: the export Whetstone writes
//!   ([`crate::store::export`]), answered with one entry per revision.
//!
//! An agreement on the wire is `{"id","record_id","revision","digest",
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

pub const AGREEMENTS_API: &str = "virgil.agreements/v1";
const PAGE: u64 = 100;
const MAX_PAGES: usize = 100;

/// One agreement as Virgil holds it, verified.
#[derive(Debug, Clone)]
pub struct WireAgreement {
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

/// Read and verify one wire agreement.
pub fn read_wire(value: &Value) -> Result<WireAgreement, StorageError> {
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
    Ok(WireAgreement {
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
            format!("This checkout is not attached to a Virgil repository ({message}), and the store switch names Virgil. Attach it (virgil repos attach <repository> --group <group> writes .virgil/repo.toml), or switch back with wh init --action store --store beads. Nothing was written."),
        ),
        (6, _) => virgil("not_found", format!("virgil agreements {op}: {message}")),
        (2, _) => virgil("invalid_request", format!("virgil agreements {op}: {message}{hint}")),
        (3, _) => virgil(
            "read_again",
            format!("virgil agreements {op}: the record moved since it was read ({message}); read it again and repeat. Nothing was retried."),
        ),
        (4, _) => virgil("refused_content", format!("virgil agreements {op}: {message}{hint}")),
        (5, _) => virgil("not_permitted", format!("virgil agreements {op}: {message}{hint}")),
        (7, _) => virgil("store_unavailable", format!("virgil agreements {op}: {message}{hint}")),
        _ => virgil("store_refused", format!("virgil agreements {op} failed: {message}{hint}")),
    }
}

/// The record store in the attached group's Records.
#[derive(Debug, Clone)]
pub struct VirgilStore {
    root: PathBuf,
}

impl VirgilStore {
    pub fn new(project_root: &Path) -> Self {
        Self {
            root: project_root.to_path_buf(),
        }
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
            "agreements".to_string(),
            op.to_string(),
            "--input".into(),
            "-".into(),
        ];
        let via = if write { Self::via() } else { None };
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
                    "virgil agreements {op} answered no JSON line (exit {}: {}); this Virgil may not have the agreements collection ({AGREEMENTS_API})",
                    answer.exit, answer.stderr
                ),
            });
        }
        if answer.line.is_object() && answer.line["api_version"].as_str() != Some(AGREEMENTS_API) {
            return Err(StorageError::Virgil {
                code: "store_contract".into(),
                message: format!(
                    "virgil agreements {op} answered {} where Whetstone pins {AGREEMENTS_API}",
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
                message: format!("virgil agreements {op} answered without a result line"),
            })
    }

    /// Every agreement in scope (all pages), optionally one idempotency key.
    pub fn listing(&self, key: Option<&str>) -> Result<Vec<WireAgreement>, StorageError> {
        let mut agreements = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_PAGES {
            let mut request = json!({"limit": PAGE, "cursor": cursor});
            if let Some(key) = key {
                request["idempotency_key"] = json!(key);
            }
            let page = self.read("list", &request)?.unwrap_or_else(|| json!({}));
            let items = page["agreements"]
                .as_array()
                .ok_or_else(|| StorageError::Virgil {
                    code: "store_contract".into(),
                    message: "virgil agreements list returned no agreements".into(),
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
                // One revision of one record is one agreement.
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
            message: format!("more than {MAX_PAGES} pages of agreements; the list stopped"),
        })
    }

    fn one(&self, op: &str, request: &Value) -> Result<Option<WireAgreement>, StorageError> {
        match self.read(op, request)? {
            Some(result) if !result["agreement"].is_null() => {
                read_wire(&result["agreement"]).map(Some)
            }
            _ => Ok(None),
        }
    }

    /// One revision as Virgil confirms it.
    pub fn fetch(
        &self,
        id: &RecordId,
        revision: u64,
    ) -> Result<Option<WireAgreement>, StorageError> {
        self.one(
            "get",
            &json!({"record_id": id.as_str(), "revision": revision}),
        )
    }

    /// Put one revision; the landed agreement's Virgil id.
    fn put(
        &self,
        record: &AgreementRecord,
        reference: &RecordRef,
        expected_revision: Option<u64>,
        copy: bool,
    ) -> Result<String, StorageError> {
        let mut request = json!({"agreement": wire(record, initial_lifecycle(record))?});
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
                message: "virgil agreements put answered without a record id".into(),
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

    fn write_answer(&self, op: &str, request: &Value) -> Result<(), StorageError> {
        let answer = self.invoke(op, request, true)?;
        if answer.exit != 0 || answer.line.get("error").is_some() {
            return Err(failure(op, &answer));
        }
        Ok(())
    }

    /// Whether the daemon answers the pinned agreements contract.
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

    /// `virgil agreements import-beads`: what the export would do (or did,
    /// with `apply`), one entry per revision.
    pub fn import_beads(&self, file: &Path, apply: bool) -> Result<Vec<Value>, StorageError> {
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
        StoreKind::Shareable
    }

    fn location(&self) -> String {
        format!("virgil:{}", self.root.display())
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
            .unwrap_or_else(|| json!({"agreements": []}));
        let mut records = result["agreements"]
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
        let existing = self.all_records()?;
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
            present.push(WireAgreement {
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
