//! Project layout, the record store port, the store switch and storage
//! errors.
//!
//! Every record revision lives behind one port, [`RecordStore`]. Which
//! adapters sit behind it is chosen once per repository by the committed
//! switch `whetstone/store.json` ([`StoreConfig`]), for every record kind
//! together:
//!
//! - **Beads** (the default): a private Beads database under `.git/whetstone`
//!   that never has a remote, and the repository's shared Beads database
//!   ([`crate::beads`]). `wh push` and `wh pull` wrap `bd dolt push` and
//!   `bd dolt pull`.
//! - **Virgil**: every record in the attached repository's `rulebook`
//!   Records collection, reached only through `virgil rulebook`
//!   ([`crate::store::virgil`]): drafts and receipts in its private scope
//!   (the signed-in principal's own), accepted records and decisions in its
//!   repository scope. Sync is the daemon's, so `wh push` and `wh pull` are
//!   refused by name and `bd` is never needed.
//!
//! The kernel (in-force resolution, digests, the draft-never-displaces-
//! accepted invariant) is the same whichever adapter is underneath; adapters
//! only keep exact revisions and refuse what would break them.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{
    AgreementHistory, AgreementRecord, DomainError, RecordBody, RecordId, RecordRef,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    Private,
    Shareable,
}

impl StoreKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shareable => "shareable",
        }
    }
}

/// Which adapter holds a store's records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreBackend {
    Beads,
    Virgil,
}

impl StoreBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Beads => "beads",
            Self::Virgil => "virgil",
        }
    }
}

/// A batch whose completion marker is missing: some of its records exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteBatch {
    pub marker_key: String,
    pub present: Vec<String>,
    pub missing: Vec<String>,
}

/// One record in a multi-record append.
///
/// `expected_revision` is the latest revision the caller observed for this
/// record ID before this item is applied. For two consecutive revisions of the
/// same ID in one batch, the second item therefore expects the first item's
/// revision.
#[derive(Debug, Clone, Copy)]
pub struct AppendRequest<'a> {
    pub record: &'a AgreementRecord,
    pub expected_revision: Option<u64>,
}

/// The record store port: exact, digest-verified, append-only revisions.
///
/// Every adapter keeps the same invariants: a record is read back exactly as
/// it was written and its digest is verified on every read (a body that does
/// not verify is reported with where it lives and never repaired); an append
/// names the revision it expects and repeats of the same idempotency key
/// return what was recorded; a multi-record append is a sequence of
/// idempotent appends closed by a completion marker, so repeating it after an
/// interruption writes only what is missing; writes are serialized by the
/// adapter.
pub trait RecordStore: std::fmt::Debug {
    fn backend(&self) -> StoreBackend;

    fn kind(&self) -> StoreKind;

    /// Where the records live, for people (a directory or a Virgil scope);
    /// two handles on the same store have the same location.
    fn location(&self) -> String;

    /// Every record revision, by id then revision.
    fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError>;

    fn get(&self, reference: &RecordRef) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .all_records()?
            .into_iter()
            .find(|record| record.reference().is_ok_and(|found| &found == reference)))
    }

    fn by_idempotency_key(&self, key: &str) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .all_records()?
            .into_iter()
            .find(|record| record.idempotency_key == key))
    }

    fn latest(&self, id: &RecordId) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .all_records()?
            .into_iter()
            .filter(|record| &record.id == id)
            .max_by_key(|record| record.revision))
    }

    fn history(&self, id: &RecordId) -> Result<Vec<AgreementRecord>, StorageError> {
        Ok(self
            .all_records()?
            .into_iter()
            .filter(|record| &record.id == id)
            .collect())
    }

    /// Where one revision lives inside the store (a bead id, a file, a
    /// Virgil record id), for people and the adapter's own tools.
    fn locator(&self, reference: &RecordRef) -> Result<Option<String>, StorageError>;

    /// Batches that started but never wrote their completion marker.
    fn incomplete_batches(&self) -> Result<Vec<IncompleteBatch>, StorageError>;

    fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError>;

    /// Append several records as one logical operation (see the trait).
    fn append_batch(&self, requests: &[AppendRequest<'_>]) -> Result<Vec<RecordRef>, StorageError>;

    /// Bring the stored lifecycle view in line with the kernel's resolution.
    /// The view is for people browsing the store; nothing reads it back.
    fn sync_lifecycle_labels(
        &self,
        state: &crate::agreement::AgreementState,
    ) -> Result<usize, StorageError>;

    /// The lifecycle view stored beside a revision (`pending`, `accepted`,
    /// `superseded`, ...), as [`RecordStore::sync_lifecycle_labels`] left it.
    fn lifecycle_view(&self, reference: &RecordRef) -> Result<Option<String>, StorageError>;

    /// Write exact revisions copied from another store, in the order given:
    /// a revision already present is skipped, a different revision under the
    /// same id and number is a conflict. Only [`RecordStore::copy_to`] and
    /// migration call this.
    fn receive(&self, records: &[AgreementRecord]) -> Result<Vec<RecordRef>, StorageError>;

    /// Remote transport, when the adapter has its own (Beads' `bd dolt`).
    /// `None` means sync is not Whetstone's to run.
    fn remote(&self) -> Option<&dyn RemoteSync> {
        None
    }

    /// Copy exact record revisions into a shareable store (the sharing
    /// path). Each copy keeps its canonical bytes; records already present
    /// are skipped. Operational records never cross, and no canary may
    /// appear in any copied record.
    fn copy_to(
        &self,
        destination: &dyn RecordStore,
        selected: &[RecordRef],
        private_canaries: &[&str],
    ) -> Result<Vec<RecordRef>, StorageError> {
        if self.location() == destination.location() || destination.kind() != StoreKind::Shareable {
            return Err(StorageError::InvalidProjectionBoundary);
        }
        let mut records = Vec::new();
        for reference in selected {
            let record = self
                .get(reference)?
                .ok_or_else(|| StorageError::UnknownReference(reference.clone()))?;
            if record_class(&record.body) == "receipt" {
                return Err(StorageError::InvalidProjectionBoundary);
            }
            // Retired kinds stay where they were; they are never written anew.
            if matches!(record.body, RecordBody::Retired(_)) {
                return Err(StorageError::Domain(DomainError::RetiredKind(
                    record.body.type_name().to_string(),
                )));
            }
            let canonical = record.canonical_json().map_err(StorageError::Domain)?;
            for canary in private_canaries {
                if !canary.is_empty()
                    && canonical
                        .windows(canary.len())
                        .any(|window| window == canary.as_bytes())
                {
                    return Err(StorageError::PrivateCanaryFound((*canary).to_string()));
                }
            }
            records.push(record);
        }
        records.sort_by(|left, right| {
            logical_stage(left)
                .cmp(&logical_stage(right))
                .then_with(|| left.id.cmp(&right.id))
                .then_with(|| left.revision.cmp(&right.revision))
        });
        destination.receive(&records)
    }
}

/// An adapter's own remote transport.
pub trait RemoteSync {
    fn remotes(&self) -> Result<Vec<String>, StorageError>;
    fn push_remote(&self) -> Result<String, StorageError>;
    fn pull_remote(&self) -> Result<String, StorageError>;
}

// ---------- the switch ----------

/// The committed switch that names the record store for every record kind.
pub const STORE_CONFIG: &str = "whetstone/store.json";
/// The Virgil repository attachment a checkout carries.
pub const VIRGIL_ATTACHMENT: &str = ".virgil/repo.toml";

/// Where every record lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "store", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoreConfig {
    /// Beads: no network and no Virgil.
    #[default]
    Beads,
    /// The attached Virgil group's Records, through the `virgil` CLI only;
    /// the scope is the repository attachment (`.virgil/repo.toml`).
    Virgil,
}

impl StoreConfig {
    pub fn backend(self) -> &'static str {
        match self {
            Self::Beads => "beads",
            Self::Virgil => "virgil",
        }
    }

    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "beads" => Ok(Self::Beads),
            "virgil" => Ok(Self::Virgil),
            other => Err(format!(
                "{other:?} is not a record store; the stores are beads and virgil."
            )),
        }
    }
}

/// Which file named the store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchSource {
    /// No `store.json`: Beads.
    Default,
    Store,
}

/// The switch as read, with where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreSwitch {
    pub config: StoreConfig,
    pub source: SwitchSource,
}

/// Read the switch. No file means Beads; a file that does not parse fails
/// closed rather than silently falling back to another store.
pub fn read_switch(project_root: &Path) -> Result<StoreSwitch, String> {
    let text = match std::fs::read_to_string(project_root.join(STORE_CONFIG)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(StoreSwitch {
                config: StoreConfig::Beads,
                source: SwitchSource::Default,
            })
        }
        Err(error) => return Err(format!("{STORE_CONFIG} could not be read: {error}")),
    };
    let config = parse_switch(&text).map_err(|reason| {
        format!(
            "{STORE_CONFIG} is not a store switch ({reason}); write {{\"store\":\"beads\"}} or {{\"store\":\"virgil\"}}."
        )
    })?;
    Ok(StoreSwitch {
        config,
        source: SwitchSource::Store,
    })
}

fn parse_switch(text: &str) -> Result<StoreConfig, String> {
    let value =
        serde_json::from_str::<serde_json::Value>(text).map_err(|error| error.to_string())?;
    if let Some(name) = value.get("store").and_then(serde_json::Value::as_str) {
        StoreConfig::parse(name)?;
    }
    // Serde lets an internally tagged unit variant carry unknown fields.
    if let Some(extra) = value
        .as_object()
        .and_then(|object| object.keys().find(|key| *key != "store"))
    {
        return Err(format!(
            "unknown field `{extra}`; the switch holds only `store`"
        ));
    }
    serde_json::from_value::<StoreConfig>(value).map_err(|error| error.to_string())
}

/// The record store the switch names.
pub fn read_config(project_root: &Path) -> Result<StoreConfig, String> {
    read_switch(project_root).map(|switch| switch.config)
}

/// Write `store.json` (pretty JSON, one trailing newline).
pub fn write_config(project_root: &Path, config: StoreConfig) -> Result<(), String> {
    let path = project_root.join(STORE_CONFIG);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut text = serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?;
    text.push('\n');
    let staged = path.with_extension("json.tmp");
    std::fs::write(&staged, text).map_err(|error| error.to_string())?;
    std::fs::rename(&staged, &path).map_err(|error| error.to_string())
}

// ---------- the factory ----------

fn switch_of(layout: &ProjectLayout) -> Result<StoreConfig, StorageError> {
    read_config(layout.project_root()).map_err(StorageError::Config)
}

/// Open the private store the switch names, without creating anything.
pub fn open_private(layout: &ProjectLayout) -> Result<Box<dyn RecordStore>, StorageError> {
    match switch_of(layout)? {
        StoreConfig::Beads => Ok(Box::new(crate::beads::BeadsStore::open_existing(
            &layout.private_store(),
            StoreKind::Private,
        )?)),
        StoreConfig::Virgil => Ok(Box::new(crate::store::virgil::VirgilStore::private(
            layout.project_root(),
        ))),
    }
}

/// Open the private store the switch names, creating it when absent. Under
/// Virgil the private scope exists whenever the daemon answers.
pub fn initialize_private(layout: &ProjectLayout) -> Result<Box<dyn RecordStore>, StorageError> {
    match switch_of(layout)? {
        StoreConfig::Beads => Ok(Box::new(crate::beads::BeadsStore::initialize(
            &layout.private_store(),
            StoreKind::Private,
        )?)),
        StoreConfig::Virgil => Ok(Box::new(crate::store::virgil::VirgilStore::private(
            layout.project_root(),
        ))),
    }
}

/// Open the shared store the switch names, without creating anything:
/// `None` when the Beads arm has no shared database. Under Virgil the store
/// is the attachment's Records, which exist whenever Virgil answers.
pub fn open_shared(layout: &ProjectLayout) -> Result<Option<Box<dyn RecordStore>>, StorageError> {
    match switch_of(layout)? {
        StoreConfig::Beads => match layout.shared_store() {
            Some(dir) => Ok(Some(Box::new(crate::beads::BeadsStore::open_existing(
                &dir,
                StoreKind::Shareable,
            )?))),
            None => Ok(None),
        },
        StoreConfig::Virgil => Ok(Some(Box::new(crate::store::virgil::VirgilStore::shared(
            layout.project_root(),
        )))),
    }
}

/// Make the shared store ready for writes where Whetstone owns it: under
/// Beads, register Whetstone's bead types in the repository's database when
/// one exists; under Virgil, nothing (the daemon owns Records).
pub fn prepare_shared(layout: &ProjectLayout) -> Result<(), StorageError> {
    match switch_of(layout)? {
        StoreConfig::Beads => {
            if let Some(dir) = layout.shared_store() {
                crate::beads::BeadsStore::initialize(&dir, StoreKind::Shareable)?;
            }
            Ok(())
        }
        StoreConfig::Virgil => Ok(()),
    }
}

// ---------- shared helpers for adapters ----------

/// Canonical JSON with every non-ASCII character escaped, so no store's JSON
/// column ever sees characters (such as U+2028) that break its reads.
pub fn ascii_json(canonical: &[u8]) -> Result<String, StorageError> {
    let text = std::str::from_utf8(canonical)
        .map_err(|error| StorageError::Serialization(error.to_string()))?;
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_ascii() {
            out.push(character);
        } else {
            let mut units = [0_u16; 2];
            for unit in character.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    Ok(out)
}

/// The class of a record: agreement content (`record`), a decision on it
/// (`decision`), or evidence (`receipt`). Receipts never cross stores.
pub fn record_class(body: &RecordBody) -> &'static str {
    match body {
        RecordBody::Mission(_)
        | RecordBody::Principle(_)
        | RecordBody::Rule(_)
        | RecordBody::StoredRule(_)
        | RecordBody::Standard(_) => "record",
        RecordBody::Proposal(_) | RecordBody::LocalReview(_) | RecordBody::Retirement(_) => {
            "decision"
        }
        RecordBody::Retired(retired) => match retired.record_type.as_str() {
            "rule"
            | "guidance"
            | "core_value"
            | "implementation_philosophy"
            | "metric_definition"
            | "source_snapshot"
            | "policy_exception"
            | "lesson"
            | "feature"
            | "verification_map" => "record",
            "decision" | "mandate" | "activation" | "hand_answer" | "flag_decision" => "decision",
            _ => "receipt",
        },
    }
}

/// `id@revision#digest`.
pub fn reference_text(reference: &RecordRef) -> String {
    format!(
        "{}@{}#{}",
        reference.id.as_str(),
        reference.revision,
        reference.digest.as_str()
    )
}

/// The lifecycle view a revision starts with.
pub fn initial_lifecycle(record: &AgreementRecord) -> &'static str {
    if crate::agreement::is_agreement_body(&record.body)
        || matches!(record.body, RecordBody::Retirement(_))
    {
        "pending"
    } else {
        "operational"
    }
}

/// The lifecycle view the kernel's resolution gives a revision.
pub fn resolved_lifecycle(
    state: &crate::agreement::AgreementState,
    record: &AgreementRecord,
) -> &'static str {
    use crate::agreement::Lifecycle;
    match &record.body {
        body if crate::agreement::is_agreement_body(body)
            || matches!(body, RecordBody::Retirement(_)) =>
        {
            let lifecycle = state.lifecycle_of(record);
            if lifecycle == Lifecycle::Accepted
                && state.is_retired(&record.id)
                && state.in_force(&record.id).is_none()
            {
                "retired"
            } else if lifecycle == Lifecycle::Accepted && state.is_superseded(record) {
                "superseded"
            } else {
                lifecycle.label()
            }
        }
        _ => "operational",
    }
}

/// The batch an onboarding key belongs to.
pub fn batch_of(key: &str) -> Option<String> {
    // Onboarding writes keys `<request>:base-<revision>:<part>`.
    key.find(":base-")
        .map(|index| format!("batch:{}", &key[..index]))
}

/// The completion marker's key for a batch of appends.
pub fn batch_marker_key(requests: &[AppendRequest<'_>]) -> String {
    let first = &requests[0].record.idempotency_key;
    batch_of(first).unwrap_or_else(|| {
        let mut hasher = Sha256::new();
        for request in requests {
            hasher.update(request.record.idempotency_key.as_bytes());
            hasher.update([0]);
        }
        format!("batch:{:x}", hasher.finalize())
    })
}

/// Batches with members present but no completion marker among `completed`.
pub fn pending_batches(
    records: &[AgreementRecord],
    completed: &std::collections::BTreeSet<String>,
) -> Vec<IncompleteBatch> {
    let mut pending = std::collections::BTreeMap::<String, Vec<String>>::new();
    for record in records {
        if let Some(batch) = batch_of(&record.idempotency_key) {
            if !completed.contains(&batch) {
                pending
                    .entry(batch)
                    .or_default()
                    .push(record.idempotency_key.clone());
            }
        }
    }
    pending
        .into_iter()
        .map(|(marker_key, present)| IncompleteBatch {
            marker_key,
            present,
            missing: Vec::new(),
        })
        .collect()
}

/// Write order that keeps every link resolvable: agreement content first,
/// then proposals, then the reviews and decisions on them, then receipts.
pub fn logical_stage(record: &AgreementRecord) -> u8 {
    match &record.body {
        RecordBody::Proposal(_) => 1,
        RecordBody::LocalReview(_) => 2,
        body if record_class(body) == "receipt" => 3,
        _ => 0,
    }
}

pub fn map_append_domain_error(error: DomainError) -> StorageError {
    match error {
        DomainError::StaleRevision { expected, actual } => {
            StorageError::StaleRevision { expected, actual }
        }
        DomainError::IllegalRevision { expected, actual } => {
            StorageError::IllegalRevision { expected, actual }
        }
        DomainError::IdempotencyConflict(key) => StorageError::IdempotencyConflict(key),
        other => StorageError::Domain(other),
    }
}

/// Validate a record before any adapter writes it.
pub fn validate_for_append(record: &AgreementRecord) -> Result<RecordRef, StorageError> {
    record.validate().map_err(StorageError::Domain)?;
    if matches!(record.body, RecordBody::Retired(_)) {
        return Err(StorageError::Domain(DomainError::RetiredKind(
            record.body.type_name().to_string(),
        )));
    }
    record.reference().map_err(StorageError::Domain)
}

/// One append checked against everything the store holds (read under the
/// adapter's write lock): `Ok(Some(reference))` when the idempotency key
/// already recorded exactly this revision, `Ok(None)` when it may be written.
pub fn check_append(
    existing: &[AgreementRecord],
    record: &AgreementRecord,
    reference: &RecordRef,
    expected_revision: Option<u64>,
) -> Result<Option<RecordRef>, StorageError> {
    if let Some(found) = existing
        .iter()
        .find(|item| item.idempotency_key == record.idempotency_key)
    {
        return if found.reference().map_err(StorageError::Domain)? == *reference {
            Ok(Some(reference.clone()))
        } else {
            Err(StorageError::IdempotencyConflict(
                record.idempotency_key.clone(),
            ))
        };
    }
    let actual = existing
        .iter()
        .filter(|item| item.id == record.id)
        .map(|item| item.revision)
        .max();
    if actual != expected_revision {
        return Err(StorageError::StaleRevision {
            expected: expected_revision,
            actual,
        });
    }
    let required = actual.map_or(1, |revision| revision + 1);
    if record.revision != required {
        return Err(StorageError::IllegalRevision {
            expected: required,
            actual: record.revision,
        });
    }
    Ok(None)
}

/// Validate a whole batch against everything the store holds before any of
/// it is written: the store's history replays, and every request either
/// repeats exactly what its key recorded or appends cleanly.
pub fn check_batch(
    existing: &[AgreementRecord],
    requests: &[AppendRequest<'_>],
) -> Result<(), StorageError> {
    let mut keys = std::collections::BTreeSet::new();
    for request in requests {
        request.record.validate().map_err(StorageError::Domain)?;
        if !keys.insert(request.record.idempotency_key.clone()) {
            return Err(StorageError::IdempotencyConflict(
                request.record.idempotency_key.clone(),
            ));
        }
    }
    let mut history = AgreementHistory::default();
    let mut records = existing.to_vec();
    records.sort_by(|left, right| {
        logical_stage(left)
            .cmp(&logical_stage(right))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.revision.cmp(&right.revision))
    });
    for record in records {
        let expected = history.latest(&record.id).map(|current| current.revision);
        history
            .replay(record, expected)
            .map_err(StorageError::Domain)?;
    }
    for request in requests {
        let already = existing
            .iter()
            .find(|item| item.idempotency_key == request.record.idempotency_key);
        match already {
            Some(item) => {
                if item.reference().map_err(StorageError::Domain)?
                    != request.record.reference().map_err(StorageError::Domain)?
                {
                    return Err(StorageError::IdempotencyConflict(
                        request.record.idempotency_key.clone(),
                    ));
                }
            }
            None => {
                history
                    .append(request.record.clone(), request.expected_revision)
                    .map_err(map_append_domain_error)?;
            }
        }
    }
    Ok(())
}

/// The metadata every adapter stores beside a revision: the record as an
/// ASCII-escaped canonical JSON string with its id, revision, digest,
/// supersedes reference and idempotency key.
pub const METADATA_SCHEMA: u64 = 1;

pub fn record_metadata(
    record: &AgreementRecord,
    reference: &RecordRef,
) -> Result<serde_json::Value, StorageError> {
    let canonical = record.canonical_json().map_err(StorageError::Domain)?;
    Ok(serde_json::json!({
        "wh_schema": METADATA_SCHEMA,
        "wh_kind": record_class(&record.body),
        "wh_type": record.body.type_name(),
        "wh_id": record.id.as_str(),
        "wh_revision": record.revision,
        "wh_digest": reference.digest.as_str(),
        "wh_key": record.idempotency_key,
        "wh_supersedes": record.supersedes.as_ref().map(reference_text),
        "wh_record": ascii_json(&canonical)?,
    }))
}

/// Decode and verify a revision's metadata; `Err(reason)` when it does not
/// verify (the adapter reports it with where it lives).
pub fn decode_metadata(metadata: &serde_json::Value) -> Result<AgreementRecord, String> {
    use serde_json::Value;
    if metadata.get("wh_schema").and_then(Value::as_u64) != Some(METADATA_SCHEMA) {
        return Err("missing or unsupported Whetstone metadata schema".into());
    }
    let text = metadata
        .get("wh_record")
        .and_then(Value::as_str)
        .ok_or("missing wh_record")?;
    let record: AgreementRecord =
        serde_json::from_str(text).map_err(|error| format!("wh_record: {error}"))?;
    record
        .validate()
        .map_err(|error| format!("record: {error}"))?;
    let digest = record.digest().map_err(|error| error.to_string())?;
    if metadata.get("wh_digest").and_then(Value::as_str) != Some(digest.as_str()) {
        return Err("the record does not match its digest (edited or corrupted metadata)".into());
    }
    if metadata.get("wh_id").and_then(Value::as_str) != Some(record.id.as_str())
        || metadata.get("wh_revision").and_then(Value::as_u64) != Some(record.revision)
        || metadata.get("wh_key").and_then(Value::as_str) != Some(record.idempotency_key.as_str())
    {
        return Err("metadata fields disagree with the record".into());
    }
    Ok(record)
}

// ---------- the layout ----------

#[derive(Debug, Clone)]
pub struct ProjectLayout {
    project_root: PathBuf,
    state_root: PathBuf,
    project_id: String,
}

impl ProjectLayout {
    pub fn resolve(start: &Path) -> Result<Self, StorageError> {
        let start = start.canonicalize().map_err(StorageError::Io)?;
        let search_root = if start.is_file() {
            start.parent().ok_or(StorageError::ProjectRootNotFound)?
        } else {
            &start
        };
        let top = git_output(search_root, &["rev-parse", "--show-toplevel"])?;
        let project_root = PathBuf::from(top.trim())
            .canonicalize()
            .map_err(StorageError::Io)?;
        let common = git_output(search_root, &["rev-parse", "--git-common-dir"])?;
        let common = PathBuf::from(common.trim());
        let common = if common.is_absolute() {
            common
        } else {
            search_root.join(common)
        };
        let common = common.canonicalize().map_err(StorageError::Io)?;
        // The trailing "." is the whole-repository scope every earlier store
        // was keyed by; keeping it keeps existing private stores in place.
        let identity = format!("{}\0.", common.display());
        let project_id = format!("{:x}", Sha256::digest(identity.as_bytes()));
        let state_root = common.join("whetstone/v1").join(&project_id[..20]);
        Ok(Self {
            project_root,
            state_root,
            project_id,
        })
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    /// The private Beads database directory (holding `.beads`).
    pub fn private_store(&self) -> PathBuf {
        self.state_root.join(StoreKind::Private.as_str())
    }

    /// Whether the checkout carries a Virgil repository attachment
    /// (`.virgil/repo.toml`); answered from the file, never the daemon.
    pub fn virgil_attached(&self) -> bool {
        self.project_root.join(VIRGIL_ATTACHMENT).is_file()
    }

    /// Whether the private store the switch names exists: under Beads the
    /// private database, under Virgil the attachment whose private scope
    /// holds the drafts. An unreadable switch answers for either store, so
    /// opening it reports the switch.
    pub fn private_store_exists(&self) -> bool {
        let beads = || crate::beads::is_initialized(&self.private_store());
        match read_config(&self.project_root) {
            Ok(StoreConfig::Beads) => beads(),
            Ok(StoreConfig::Virgil) => self.virgil_attached(),
            Err(_) => beads() || self.virgil_attached(),
        }
    }

    /// The repository's shared Beads database directory, when one exists.
    pub fn shared_store(&self) -> Option<PathBuf> {
        crate::beads::shared_dir(&self.project_root)
    }

    pub fn projections_path(&self) -> PathBuf {
        self.state_root.join("projections")
    }
}

fn git_output(cwd: &Path, args: &[&str]) -> Result<String, StorageError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .map_err(StorageError::Io)?;
    checked_output("git", output)
}

fn checked_output(command: &'static str, output: Output) -> Result<String, StorageError> {
    if output.status.success() {
        String::from_utf8(output.stdout)
            .map_err(|error| StorageError::Serialization(error.to_string()))
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(StorageError::CommandFailed {
            command,
            status: output.status.code(),
            message: stderr
                .lines()
                .next()
                .unwrap_or("command failed")
                .chars()
                .take(240)
                .collect(),
        })
    }
}

#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    Domain(DomainError),
    Serialization(String),
    CommandFailed {
        command: &'static str,
        status: Option<i32>,
        message: String,
    },
    BeadsMissing,
    UnsupportedBeadsVersion {
        minimum: String,
        found: String,
    },
    /// A bead labelled as Whetstone's whose metadata does not verify.
    MalformedBead {
        bead: String,
        reason: String,
    },
    /// A stored revision (a file, a Virgil record) that does not verify,
    /// named by where it lives.
    MalformedRecord {
        locator: String,
        reason: String,
    },
    /// Two beads claim the same revision of a record with different content.
    ConflictingRevision {
        id: RecordId,
        revision: u64,
        beads: Vec<String>,
    },
    /// The private store must never be able to push anywhere.
    PrivateStoreHasRemote(PathBuf),
    ProjectRootNotFound,
    RepositoryNotInitialized(PathBuf),
    SymlinkPath(PathBuf),
    LockPoisoned,
    StaleRevision {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    IllegalRevision {
        expected: u64,
        actual: u64,
    },
    IdempotencyConflict(String),
    EmptyBatch,
    UnknownReference(RecordRef),
    DigestMismatch(RecordId),
    UnexpectedData(String),
    InvalidProjectionBoundary,
    PrivateCanaryFound(String),
    /// The store switch could not be read.
    Config(String),
    /// Virgil refused or could not answer, with Whetstone's refusal code
    /// (`unattached`, `store_unavailable`, `not_permitted`, ...).
    Virgil {
        code: String,
        message: String,
    },
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn no_switch_is_beads_and_store_json_names_every_record() {
        let temp = tempfile::tempdir().expect("temp");
        let switch = read_switch(temp.path()).expect("read");
        assert_eq!(switch.config, StoreConfig::Beads);
        assert_eq!(switch.source, SwitchSource::Default);
        for (text, config) in [
            ("{\"store\":\"beads\"}", StoreConfig::Beads),
            ("{\"store\":\"virgil\"}", StoreConfig::Virgil),
        ] {
            write(temp.path(), STORE_CONFIG, text);
            let switch = read_switch(temp.path()).expect("read");
            assert_eq!(switch.config, config);
            assert_eq!(switch.source, SwitchSource::Store);
        }
    }

    #[test]
    fn an_unknown_store_or_field_is_refused_by_name() {
        let temp = tempfile::tempdir().expect("temp");
        write(temp.path(), STORE_CONFIG, "{\"store\":\"linear\"}");
        let refused = read_switch(temp.path()).expect_err("unknown store");
        assert!(
            refused.contains("\"linear\" is not a record store"),
            "{refused}"
        );
        write(
            temp.path(),
            STORE_CONFIG,
            "{\"store\":\"virgil\",\"group\":\"01J8Z0000000000000000000AD\"}",
        );
        assert!(read_switch(temp.path()).is_err(), "store.json has no group");
        write(temp.path(), STORE_CONFIG, "not json");
        assert!(read_switch(temp.path()).is_err());
        assert!(matches!(
            open_private(&ProjectLayout {
                project_root: temp.path().to_path_buf(),
                state_root: temp.path().join("state"),
                project_id: "x".repeat(64),
            }),
            Err(StorageError::Config(_))
        ));
    }

    #[test]
    fn write_config_writes_pretty_json() {
        let temp = tempfile::tempdir().expect("temp");
        assert_eq!(write_config(temp.path(), StoreConfig::Beads), Ok(()));
        assert_eq!(
            std::fs::read_to_string(temp.path().join(STORE_CONFIG)).expect("read"),
            "{\n  \"store\": \"beads\"\n}\n"
        );
    }
}
