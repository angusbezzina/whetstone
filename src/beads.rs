//! Beads-backed record store: Whetstone's typed layer over `bd --json`.
//!
//! Every Whetstone record revision is one bead. The typed body travels in the
//! bead's metadata as an ASCII-escaped canonical JSON string (`wh_record`)
//! next to its id, revision, digest, supersedes reference and idempotency
//! key; Beads re-serializes metadata objects with sorted keys, so the record
//! is stored as a string and its digest is re-verified on every read. A bead
//! whose Whetstone metadata does not verify is reported with its bead id and
//! never repaired.
//!
//! Agreement records are `record` beads, proposals, reviews, decisions and
//! retirements are `decision` beads, and receipts and repair sessions are
//! `receipt` beads. Lifecycle is a label (`wh:lifecycle:<state>`) kept in
//! step with the kernel's resolution; the kernel never trusts the label.
//!
//! Embedded Beads has no SQL and no multi-statement transaction, so each read
//! or write is one `bd` process call, writes are serialized behind a file
//! lock, and a multi-record write is a sequence of idempotent appends closed
//! by a completion marker. An interrupted batch resumes when the same request
//! is repeated.
//!
//! The private store lives under `.git/whetstone` and must never have a
//! remote: it is initialized and used with Git discovery fenced off
//! (`GIT_CEILING_DIRECTORIES`), because `bd init` inside a repository whose
//! origin carries Beads data would otherwise clone the team's database and
//! wire the team remote into it.

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::domain::{
    AgreementHistory, AgreementRecord, ContentDigest, RecordBody, RecordId, RecordRef,
};
use crate::storage::{AppendRequest, StorageError, StoreKind};

/// The oldest `bd` whose JSON shapes Whetstone is tested against.
pub const MINIMUM_BD_VERSION: (u64, u64, u64) = (1, 1, 2);
pub const WHETSTONE_LABEL: &str = "whetstone";
pub const LIFECYCLE_LABEL_PREFIX: &str = "wh:lifecycle:";
pub const METADATA_SCHEMA: u64 = 1;
pub const PRIVATE_PREFIX: &str = "whp";
const CUSTOM_TYPES: [&str; 2] = ["record", "receipt"];

/// Whether a directory holds an initialized Beads database.
pub fn is_initialized(dir: &Path) -> bool {
    dir.join(".beads").join("metadata.json").is_file()
}

/// The repository's shared Beads database directory (the one holding
/// `.beads`), as `bd where` resolves it from the project root.
pub fn shared_dir(project_root: &Path) -> Option<PathBuf> {
    if !project_root.join(".beads").is_dir() {
        return None;
    }
    let output = Command::new("bd")
        .args(["where", "--json"])
        .current_dir(project_root)
        .env_remove("BEADS_DIR")
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = serde_json::from_slice::<Value>(&output.stdout).ok()?;
    let beads = PathBuf::from(value.get("path")?.as_str()?);
    let dir = beads.parent()?.to_path_buf();
    is_initialized(&dir).then_some(dir)
}

/// `bd` must exist and be at least [`MINIMUM_BD_VERSION`]; probed once.
pub fn ensure_bd_version() -> Result<String, StorageError> {
    static VERIFIED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    if let Some(version) = VERIFIED.get() {
        return Ok(version.clone());
    }
    let output = Command::new("bd")
        .arg("version")
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StorageError::BeadsMissing
            } else {
                StorageError::Io(error)
            }
        })?;
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let found = text
        .split_whitespace()
        .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .unwrap_or_default()
        .trim_start_matches('v')
        .to_string();
    let parts = found
        .split('.')
        .map(|part| {
            part.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse::<u64>()
                .unwrap_or(0)
        })
        .collect::<Vec<_>>();
    let version = (
        parts.first().copied().unwrap_or(0),
        parts.get(1).copied().unwrap_or(0),
        parts.get(2).copied().unwrap_or(0),
    );
    if !output.status.success() || version < MINIMUM_BD_VERSION {
        return Err(StorageError::UnsupportedBeadsVersion {
            minimum: format!(
                "{}.{}.{}",
                MINIMUM_BD_VERSION.0, MINIMUM_BD_VERSION.1, MINIMUM_BD_VERSION.2
            ),
            found,
        });
    }
    let _ = VERIFIED.set(found.clone());
    Ok(found)
}

/// Canonical JSON with every non-ASCII character escaped, so Beads' JSON
/// column never sees characters (such as U+2028) that break its reads.
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

fn bead_type(body: &RecordBody) -> &'static str {
    match body {
        RecordBody::Mission(_)
        | RecordBody::CoreValue(_)
        | RecordBody::ImplementationPhilosophy(_)
        | RecordBody::Standard(_)
        | RecordBody::Guidance(_)
        | RecordBody::MetricDefinition(_)
        | RecordBody::Feature(_)
        | RecordBody::VerificationMap(_)
        | RecordBody::PolicyException(_)
        | RecordBody::SourceSnapshot(_) => "record",
        RecordBody::Proposal(_)
        | RecordBody::Decision(_)
        | RecordBody::LocalReview(_)
        | RecordBody::Activation(_)
        | RecordBody::Mandate(_)
        | RecordBody::Retirement(_) => "decision",
        RecordBody::VerificationReceipt(_)
        | RecordBody::ObservationReceipt(_)
        | RecordBody::RepairSession(_)
        | RecordBody::RepairHandoff(_)
        | RecordBody::RepairAuthorityReservation(_)
        | RecordBody::RepairOperationClaim(_) => "receipt",
    }
}

/// Titles are plain text for people browsing `bd list`; never the record.
fn bead_title(record: &AgreementRecord) -> String {
    let title = crate::projection::record_title(record);
    let clean = title
        .chars()
        .filter(|character| character.is_ascii_graphic() || *character == ' ')
        .take(90)
        .collect::<String>();
    format!(
        "{} r{}: {}",
        record.id.as_str(),
        record.revision,
        clean.trim()
    )
}

fn reference_text(reference: &RecordRef) -> String {
    format!(
        "{}@{}#{}",
        reference.id.as_str(),
        reference.revision,
        reference.digest.as_str()
    )
}

#[derive(Debug, Clone)]
struct StoredBead {
    bead: String,
    record: AgreementRecord,
    reference: RecordRef,
    labels: Vec<String>,
}

/// A batch whose completion marker is missing: some of its records exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncompleteBatch {
    pub marker_key: String,
    pub present: Vec<String>,
    pub missing: Vec<String>,
}

/// A listing and the storage fingerprint it was read at.
#[derive(Debug, Clone)]
struct Cached {
    beads: Vec<StoredBead>,
    markers: Vec<Value>,
    fingerprint: String,
}

/// The listing cache every handle on one store directory shares.
type SharedCache = Arc<Mutex<Option<Cached>>>;

#[derive(Debug, Clone)]
pub struct RecordStore {
    dir: PathBuf,
    kind: StoreKind,
    write_guard: Arc<Mutex<()>>,
    cache: SharedCache,
    /// The private store proved remote-free in this process (before writes).
    remote_free: Arc<std::sync::atomic::AtomicBool>,
}

impl RecordStore {
    /// Open an initialized store without creating or repairing anything.
    pub fn open_existing(dir: &Path, kind: StoreKind) -> Result<Self, StorageError> {
        reject_symlink(dir)?;
        if !is_initialized(dir) {
            return Err(StorageError::RepositoryNotInitialized(dir.to_path_buf()));
        }
        ensure_bd_version()?;
        Self::new(dir, kind)
    }

    /// Create the store when absent (idempotent), register Whetstone's custom
    /// bead types, and for the private store prove it has no remote.
    pub fn initialize(dir: &Path, kind: StoreKind) -> Result<Self, StorageError> {
        ensure_bd_version()?;
        reject_symlink(dir)?;
        create_private_dir(dir)?;
        let _lock = lock_file(&dir.join(".whetstone-init.lock"))?;
        let store = Self::new(dir, kind)?;
        if !is_initialized(dir) {
            match kind {
                // bd init walks up the directory tree and refuses (or reuses)
                // any ancestor `.beads`, which is exactly the repository's
                // shared database. Initialize outside the repository, with Git
                // discovery fenced, and move the database into place.
                StoreKind::Private => store.staged_init(PRIVATE_PREFIX)?,
                StoreKind::Shareable => {
                    store.bd(&[
                        "init",
                        "--non-interactive",
                        "--skip-agents",
                        "--skip-hooks",
                        "-p",
                        "whs",
                        "-q",
                    ])?;
                }
            }
            if !is_initialized(dir) {
                return Err(StorageError::UnexpectedData(
                    "bd init did not create a database".into(),
                ));
            }
        }
        store.ensure_custom_types()?;
        if kind == StoreKind::Private {
            store.ensure_no_remote()?;
        }
        Ok(store)
    }

    fn staged_init(&self, prefix: &str) -> Result<(), StorageError> {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| StorageError::Serialization(error.to_string()))?
            .as_nanos();
        // Unique per call even when threads start in the same instant.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let stage = std::env::temp_dir().join(format!(
            "whetstone-private-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        create_private_dir(&stage)?;
        let result = (|| {
            let output = Command::new("bd")
                .args([
                    "init",
                    "--non-interactive",
                    "--skip-agents",
                    "--skip-hooks",
                    "-p",
                    prefix,
                    "-q",
                ])
                .current_dir(&stage)
                .env("BEADS_DIR", stage.join(".beads"))
                .env("BD_NON_INTERACTIVE", "1")
                .env("GIT_CEILING_DIRECTORIES", stage.parent().unwrap_or(&stage))
                .env_remove("BEADS_DB")
                .output()
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        StorageError::BeadsMissing
                    } else {
                        StorageError::Io(error)
                    }
                })?;
            if !output.status.success() || !is_initialized(&stage) {
                return Err(StorageError::CommandFailed {
                    command: "bd",
                    status: output.status.code(),
                    message: String::from_utf8_lossy(&output.stderr)
                        .lines()
                        .find(|line| !line.trim().is_empty() && line.trim() != "Error:")
                        .unwrap_or("bd init failed")
                        .chars()
                        .take(240)
                        .collect(),
                });
            }
            move_dir(&stage.join(".beads"), &self.dir.join(".beads"))
        })();
        let _ = fs::remove_dir_all(&stage);
        result
    }

    fn new(dir: &Path, kind: StoreKind) -> Result<Self, StorageError> {
        let dir = dir.canonicalize().map_err(StorageError::Io)?;
        Ok(Self {
            cache: shared_cache(&dir)?,
            dir,
            kind,
            write_guard: Arc::new(Mutex::new(())),
            remote_free: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub fn root(&self) -> &Path {
        &self.dir
    }

    pub fn kind(&self) -> StoreKind {
        self.kind
    }

    /// One `bd` call against exactly this database.
    fn bd(&self, args: &[&str]) -> Result<String, StorageError> {
        let mut command = Command::new("bd");
        command
            .args(args)
            .current_dir(&self.dir)
            .env("BEADS_DIR", self.dir.join(".beads"))
            .env("BD_NON_INTERACTIVE", "1")
            .env_remove("BEADS_DB");
        if self.kind == StoreKind::Private {
            // Fence Git discovery so bd never sees the enclosing repository
            // or its origin (see the module documentation).
            if let Some(parent) = self.dir.parent() {
                command.env("GIT_CEILING_DIRECTORIES", parent);
            }
        }
        let output = command.output().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StorageError::BeadsMissing
            } else {
                StorageError::Io(error)
            }
        })?;
        if output.status.success() {
            String::from_utf8(output.stdout)
                .map_err(|error| StorageError::Serialization(error.to_string()))
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let message = stderr
                .lines()
                .chain(stdout.lines())
                .find(|line| line.to_ascii_lowercase().contains("error"))
                .or_else(|| stderr.lines().next())
                .unwrap_or("bd failed")
                .chars()
                .take(240)
                .collect();
            Err(StorageError::CommandFailed {
                command: "bd",
                status: output.status.code(),
                message,
            })
        }
    }

    fn bd_json(&self, args: &[&str]) -> Result<Value, StorageError> {
        let output = self.bd(args)?;
        let start = output
            .find(['[', '{'])
            .ok_or_else(|| StorageError::UnexpectedData("bd returned no JSON".into()))?;
        serde_json::from_str(&output[start..])
            .map_err(|error| StorageError::Serialization(format!("bd JSON: {error}")))
    }

    fn ensure_custom_types(&self) -> Result<(), StorageError> {
        let current = self
            .bd(&["config", "get", "types.custom"])
            .unwrap_or_default();
        let mut types = current
            .lines()
            .next()
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.contains(' '))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let before = types.len();
        for name in CUSTOM_TYPES {
            if !types.iter().any(|existing| existing == name) {
                types.push(name.to_string());
            }
        }
        if types.len() != before {
            self.bd(&["config", "set", "types.custom", &types.join(",")])?;
        }
        Ok(())
    }

    /// The private store's defining property: nothing to push to. Checked
    /// once per process before the first write (bd may auto-push commits to
    /// a configured remote), and again by every push.
    pub fn ensure_no_remote(&self) -> Result<(), StorageError> {
        use std::sync::atomic::Ordering;
        if self.remote_free.load(Ordering::Acquire) {
            return Ok(());
        }
        let remotes = self.bd_json(&["dolt", "remote", "list", "--json"])?;
        if remotes.as_array().is_some_and(Vec::is_empty) {
            self.remote_free.store(true, Ordering::Release);
            Ok(())
        } else {
            Err(StorageError::PrivateStoreHasRemote(self.dir.clone()))
        }
    }

    fn before_write(&self) -> Result<(), StorageError> {
        if self.kind == StoreKind::Private {
            self.ensure_no_remote()?;
        }
        Ok(())
    }

    pub fn remotes(&self) -> Result<Vec<String>, StorageError> {
        let remotes = self.bd_json(&["dolt", "remote", "list", "--json"])?;
        Ok(remotes
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("url")
                            .or_else(|| item.get("URL"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                            .or_else(|| item.as_str().map(str::to_owned))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    // ---------- reads ----------

    /// A cheap fingerprint of the embedded Dolt files: any write by any
    /// process changes it, so a cached listing is never served stale.
    ///
    /// Every commit appends to Dolt's chunk journal (its size grows) and a
    /// garbage collection rewrites the manifest, so file sizes plus the
    /// manifest bytes identify a database state. Modification times are
    /// deliberately left out: every bd invocation, reads included, touches
    /// them, which would make every cached listing look stale.
    fn fingerprint(&self) -> String {
        fn walk(dir: &Path, prefix: &str, depth: usize, parts: &mut Vec<String>) {
            let Ok(entries) = fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "LOCK" {
                    continue;
                }
                let path = entry.path();
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.is_dir() {
                    if depth < 2 {
                        walk(&path, &format!("{prefix}{name}/"), depth + 1, parts);
                    }
                } else if name == "manifest" {
                    let content = fs::read(&path).unwrap_or_default();
                    parts.push(format!("{prefix}{name}:{:x}", Sha256::digest(&content)));
                } else {
                    parts.push(format!("{prefix}{name}:{}", metadata.len()));
                }
            }
        }
        let mut parts = Vec::new();
        let root = self.dir.join(".beads").join("embeddeddolt");
        if let Ok(databases) = fs::read_dir(&root) {
            for database in databases.flatten() {
                let name = database.file_name().to_string_lossy().to_string();
                walk(
                    &database.path().join(".dolt").join("noms"),
                    &format!("{name}/"),
                    0,
                    &mut parts,
                );
            }
        }
        parts.sort();
        format!("{:x}", Sha256::digest(parts.join("\n").as_bytes()))
    }

    fn load(&self) -> Result<(Vec<StoredBead>, Vec<Value>), StorageError> {
        let listed = self.bd_json(&[
            "list",
            "--json",
            "--all",
            "--limit",
            "0",
            "--label",
            WHETSTONE_LABEL,
        ])?;
        let items = listed.as_array().ok_or_else(|| {
            StorageError::UnexpectedData("bd list did not return an array".into())
        })?;
        let mut stored = Vec::new();
        let mut markers = Vec::new();
        for item in items {
            let bead = item
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let metadata = item.get("metadata").cloned().unwrap_or(Value::Null);
            if metadata.get("wh_kind").and_then(Value::as_str) == Some("batch") {
                markers.push(metadata);
                continue;
            }
            let record = decode_bead(&bead, &metadata)?;
            let reference = record
                .reference()
                .map_err(|error| StorageError::MalformedBead {
                    bead: bead.clone(),
                    reason: error.to_string(),
                })?;
            let labels = item
                .get("labels")
                .and_then(Value::as_array)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            stored.push(StoredBead {
                bead,
                record,
                reference,
                labels,
            });
        }
        stored.sort_by(|left, right| {
            left.record
                .id
                .cmp(&right.record.id)
                .then_with(|| left.record.revision.cmp(&right.record.revision))
                .then_with(|| left.bead.cmp(&right.bead))
        });
        // One revision of one record is one bead. A second bead with the same
        // content (a re-push) is harmless; different content is a conflict.
        let mut deduplicated: Vec<StoredBead> = Vec::with_capacity(stored.len());
        for item in stored {
            if let Some(previous) = deduplicated.last() {
                if previous.record.id == item.record.id
                    && previous.record.revision == item.record.revision
                {
                    if previous.reference == item.reference {
                        continue;
                    }
                    return Err(StorageError::ConflictingRevision {
                        id: item.record.id.clone(),
                        revision: item.record.revision,
                        beads: vec![previous.bead.clone(), item.bead.clone()],
                    });
                }
            }
            deduplicated.push(item);
        }
        Ok((deduplicated, markers))
    }

    fn snapshot(&self) -> Result<Vec<StoredBead>, StorageError> {
        let fingerprint = self.fingerprint();
        {
            let cache = self.cache.lock().map_err(|_| StorageError::LockPoisoned)?;
            if let Some(cached) = cache.as_ref() {
                if cached.fingerprint == fingerprint {
                    return Ok(cached.beads.clone());
                }
            }
        }
        self.refresh()
    }

    fn refresh(&self) -> Result<Vec<StoredBead>, StorageError> {
        let fingerprint = self.fingerprint();
        let (beads, markers) = self.load()?;
        *self.cache.lock().map_err(|_| StorageError::LockPoisoned)? = Some(Cached {
            beads: beads.clone(),
            markers,
            fingerprint,
        });
        Ok(beads)
    }

    /// Record our own write in the cache and adopt the post-write fingerprint
    /// (the caller holds the write lock, so no other writer intervened).
    fn remember(&self, item: StoredBead) -> Result<(), StorageError> {
        let fingerprint = self.fingerprint();
        let mut cache = self.cache.lock().map_err(|_| StorageError::LockPoisoned)?;
        if let Some(cached) = cache.as_mut() {
            cached.beads.push(item);
            cached.beads.sort_by(|left, right| {
                left.record
                    .id
                    .cmp(&right.record.id)
                    .then_with(|| left.record.revision.cmp(&right.record.revision))
            });
            cached.fingerprint = fingerprint;
        }
        Ok(())
    }

    pub fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        Ok(self
            .snapshot()?
            .into_iter()
            .map(|item| item.record)
            .collect())
    }

    pub fn get(&self, reference: &RecordRef) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .snapshot()?
            .into_iter()
            .find(|item| &item.reference == reference)
            .map(|item| item.record))
    }

    pub fn by_idempotency_key(&self, key: &str) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .snapshot()?
            .into_iter()
            .find(|item| item.record.idempotency_key == key)
            .map(|item| item.record))
    }

    pub fn latest(&self, id: &RecordId) -> Result<Option<AgreementRecord>, StorageError> {
        Ok(self
            .snapshot()?
            .into_iter()
            .filter(|item| &item.record.id == id)
            .max_by_key(|item| item.record.revision)
            .map(|item| item.record))
    }

    pub fn history(&self, id: &RecordId) -> Result<Vec<AgreementRecord>, StorageError> {
        Ok(self
            .snapshot()?
            .into_iter()
            .filter(|item| &item.record.id == id)
            .map(|item| item.record)
            .collect())
    }

    /// The Beads id holding a record revision, for people and `bd history`.
    pub fn bead_for(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        Ok(self
            .snapshot()?
            .into_iter()
            .find(|item| &item.reference == reference)
            .map(|item| item.bead))
    }

    /// Batches that started but never wrote their completion marker.
    pub fn incomplete_batches(&self) -> Result<Vec<IncompleteBatch>, StorageError> {
        let beads = self.snapshot()?;
        let markers = self
            .cache
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .as_ref()
            .map(|cached| cached.markers.clone())
            .unwrap_or_default();
        let completed = markers
            .iter()
            .filter_map(|marker| marker.get("wh_key").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<std::collections::BTreeSet<_>>();
        let mut pending = std::collections::BTreeMap::<String, Vec<String>>::new();
        for item in &beads {
            if let Some(batch) = batch_of(&item.record.idempotency_key) {
                if !completed.contains(&batch) {
                    pending
                        .entry(batch)
                        .or_default()
                        .push(item.record.idempotency_key.clone());
                }
            }
        }
        Ok(pending
            .into_iter()
            .map(|(marker_key, present)| IncompleteBatch {
                marker_key,
                present,
                missing: Vec::new(),
            })
            .collect())
    }

    // ---------- writes ----------

    fn write_lock(&self) -> Result<File, StorageError> {
        lock_file(&self.dir.join(".whetstone-write.lock"))
    }

    pub fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        self.append_checked(record, expected_revision, false, None)
    }

    /// Insert a record exactly once; an exact replay is an error, so a replay
    /// can never be mistaken for exclusive ownership of work.
    pub fn append_exclusive(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        self.append_checked(record, expected_revision, true, None)
    }

    /// Insert an exclusive record only while `guard` is still exactly the
    /// latest revision the caller observed (checked under the write lock).
    pub fn append_exclusive_guarded(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
        guard: &RecordRef,
    ) -> Result<RecordRef, StorageError> {
        self.append_checked(record, expected_revision, true, Some(guard))
    }

    fn append_checked(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
        exclusive: bool,
        guard: Option<&RecordRef>,
    ) -> Result<RecordRef, StorageError> {
        record.validate().map_err(StorageError::Domain)?;
        self.before_write()?;
        let reference = record.reference().map_err(StorageError::Domain)?;
        let _guard = self
            .write_guard
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let _lock = self.write_lock()?;
        // Under the write lock no other writer can move the fingerprint, so a
        // snapshot that still matches it is exactly what bd would list.
        let beads = self.snapshot()?;
        self.insert_checked(
            &beads,
            record,
            reference,
            expected_revision,
            exclusive,
            guard,
        )
    }

    /// Validate one append against `beads` (fresh, under the write lock) and
    /// create it.
    fn insert_checked(
        &self,
        beads: &[StoredBead],
        record: &AgreementRecord,
        reference: RecordRef,
        expected_revision: Option<u64>,
        exclusive: bool,
        guard: Option<&RecordRef>,
    ) -> Result<RecordRef, StorageError> {
        if let Some(existing) = beads
            .iter()
            .find(|item| item.record.idempotency_key == record.idempotency_key)
        {
            if exclusive {
                return Err(StorageError::ExclusiveRecordExists(
                    existing.reference.clone(),
                ));
            }
            return if existing.reference == reference {
                Ok(reference)
            } else {
                Err(StorageError::IdempotencyConflict(
                    record.idempotency_key.clone(),
                ))
            };
        }
        let latest = beads
            .iter()
            .filter(|item| item.record.id == record.id)
            .max_by_key(|item| item.record.revision);
        if exclusive {
            if let Some(existing) = latest {
                return Err(StorageError::ExclusiveRecordExists(
                    existing.reference.clone(),
                ));
            }
        }
        if let Some(guard) = guard {
            let current = beads
                .iter()
                .filter(|item| item.record.id == guard.id)
                .max_by_key(|item| item.record.revision);
            if current.map(|item| &item.reference) != Some(guard) {
                return Err(StorageError::StaleRevision {
                    expected: Some(guard.revision),
                    actual: current.map(|item| item.record.revision),
                });
            }
        }
        let actual = latest.map(|item| item.record.revision);
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
        let supersedes_bead = record.supersedes.as_ref().and_then(|previous| {
            beads
                .iter()
                .find(|item| &item.reference == previous)
                .map(|item| item.bead.clone())
        });
        let stored = self.create(record, &reference, supersedes_bead.as_deref(), None)?;
        self.remember(stored)?;
        Ok(reference)
    }

    /// One `bd create`, verified from the JSON it returns.
    fn create(
        &self,
        record: &AgreementRecord,
        reference: &RecordRef,
        supersedes_bead: Option<&str>,
        status: Option<&str>,
    ) -> Result<StoredBead, StorageError> {
        let canonical = record.canonical_json().map_err(StorageError::Domain)?;
        let metadata = json!({
            "wh_schema": METADATA_SCHEMA,
            "wh_kind": bead_type(&record.body),
            "wh_type": record.body.type_name(),
            "wh_id": record.id.as_str(),
            "wh_revision": record.revision,
            "wh_digest": reference.digest.as_str(),
            "wh_key": record.idempotency_key,
            "wh_supersedes": record.supersedes.as_ref().map(reference_text),
            "wh_record": ascii_json(&canonical)?,
        });
        let file = self.dir.join(format!(
            ".whetstone-metadata-{}.json",
            &format!("{:x}", Sha256::digest(reference_text(reference).as_bytes()))[..16]
        ));
        write_private(&file, metadata.to_string().as_bytes())?;
        let lifecycle = if crate::agreement::is_agreement_body(&record.body)
            || matches!(record.body, RecordBody::Retirement(_))
        {
            "pending"
        } else {
            "operational"
        };
        let labels = format!(
            "{WHETSTONE_LABEL},wh:{},{LIFECYCLE_LABEL_PREFIX}{lifecycle}",
            record.body.type_name()
        );
        let title = bead_title(record);
        let metadata_arg = format!("@{}", file.display());
        let deps = supersedes_bead.map(|bead| format!("supersedes:{bead}"));
        let mut args = vec![
            "create",
            "--json",
            "--type",
            bead_type(&record.body),
            "--title",
            &title,
            "--metadata",
            &metadata_arg,
            "--labels",
            &labels,
            "--dolt-auto-commit",
            "on",
        ];
        if let Some(deps) = deps.as_deref() {
            args.extend(["--deps", deps]);
        }
        let created = self.bd_json(&args);
        let _ = fs::remove_file(&file);
        let created = created?;
        let created = created
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(&created);
        let bead = created
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| StorageError::UnexpectedData("bd create returned no id".into()))?
            .to_string();
        let stored = decode_bead(&bead, created.get("metadata").unwrap_or(&Value::Null))?;
        if stored.reference().map_err(StorageError::Domain)? != *reference {
            return Err(StorageError::DigestMismatch(record.id.clone()));
        }
        if let Some(status) = status {
            self.bd(&[
                "update",
                &bead,
                "--status",
                status,
                "--dolt-auto-commit",
                "on",
            ])?;
        }
        Ok(StoredBead {
            bead,
            record: stored,
            reference: reference.clone(),
            labels: labels.split(',').map(str::to_owned).collect(),
        })
    }

    /// Append several records as one logical operation: sequential,
    /// idempotent appends closed by a completion marker. Repeating the same
    /// requests after an interruption writes only what is missing.
    pub fn append_batch(
        &self,
        requests: &[AppendRequest<'_>],
    ) -> Result<Vec<RecordRef>, StorageError> {
        if requests.is_empty() {
            return Err(StorageError::EmptyBatch);
        }
        let mut keys = std::collections::BTreeSet::new();
        for request in requests {
            request.record.validate().map_err(StorageError::Domain)?;
            if !keys.insert(request.record.idempotency_key.clone()) {
                return Err(StorageError::IdempotencyConflict(
                    request.record.idempotency_key.clone(),
                ));
            }
        }
        let marker_key = batch_marker_key(requests);
        self.before_write()?;
        let _guard = self
            .write_guard
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let _lock = self.write_lock()?;
        // Validate the complete result before writing anything.
        let mut beads = self.refresh()?;
        {
            let mut history = AgreementHistory::default();
            let mut records = beads
                .iter()
                .map(|item| item.record.clone())
                .collect::<Vec<_>>();
            records.sort_by(|left, right| {
                logical_stage(left)
                    .cmp(&logical_stage(right))
                    .then_with(|| left.id.cmp(&right.id))
                    .then_with(|| left.revision.cmp(&right.revision))
            });
            for record in records {
                let expected = history.latest(&record.id).map(|current| current.revision);
                history
                    .append(record, expected)
                    .map_err(StorageError::Domain)?;
            }
            for request in requests {
                let already = beads
                    .iter()
                    .find(|item| item.record.idempotency_key == request.record.idempotency_key);
                match already {
                    Some(item) => {
                        if item.reference
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
        }
        let mut references = Vec::new();
        for request in requests {
            let reference = request.record.reference().map_err(StorageError::Domain)?;
            if !beads
                .iter()
                .any(|item| item.record.idempotency_key == request.record.idempotency_key)
            {
                self.insert_checked(
                    &beads,
                    request.record,
                    reference.clone(),
                    request.expected_revision,
                    false,
                    None,
                )?;
                beads = self.snapshot()?;
            }
            references.push(reference);
        }
        self.write_marker(&marker_key, requests)?;
        Ok(references)
    }

    fn write_marker(
        &self,
        marker_key: &str,
        requests: &[AppendRequest<'_>],
    ) -> Result<(), StorageError> {
        let exists =
            self.cache
                .lock()
                .map_err(|_| StorageError::LockPoisoned)?
                .as_ref()
                .is_some_and(|cached| {
                    cached.markers.iter().any(|marker| {
                        marker.get("wh_key").and_then(Value::as_str) == Some(marker_key)
                    })
                });
        if exists {
            return Ok(());
        }
        let metadata = json!({
            "wh_schema": METADATA_SCHEMA,
            "wh_kind": "batch",
            "wh_key": marker_key,
            "wh_members": requests.iter().map(|request| request.record.idempotency_key.clone()).collect::<Vec<_>>(),
        });
        let file = self.dir.join(".whetstone-marker.json");
        write_private(&file, metadata.to_string().as_bytes())?;
        let metadata_arg = format!("@{}", file.display());
        let title = format!("whetstone batch complete ({} records)", requests.len());
        let labels = format!("{WHETSTONE_LABEL},wh:batch,{LIFECYCLE_LABEL_PREFIX}operational");
        let result = self.bd(&[
            "create",
            "--silent",
            "--type",
            "receipt",
            "--title",
            &title,
            "--metadata",
            &metadata_arg,
            "--labels",
            &labels,
            "--dolt-auto-commit",
            "on",
        ]);
        let _ = fs::remove_file(&file);
        result?;
        let fingerprint = self.fingerprint();
        if let Some(cached) = self
            .cache
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?
            .as_mut()
        {
            cached.markers.push(metadata);
            cached.fingerprint = fingerprint;
        }
        Ok(())
    }

    /// Bring `wh:lifecycle:*` labels in line with the kernel's resolution.
    /// Labels are a view for people browsing Beads; nothing reads them back.
    pub fn sync_lifecycle_labels(
        &self,
        state: &crate::agreement::AgreementState,
    ) -> Result<usize, StorageError> {
        let beads = self.snapshot()?;
        let mut changed = 0;
        for item in beads {
            let wanted = match &item.record.body {
                body if crate::agreement::is_agreement_body(body)
                    || matches!(body, RecordBody::Retirement(_)) =>
                {
                    let lifecycle = state.lifecycle_of(&item.record);
                    if lifecycle == crate::agreement::Lifecycle::Accepted
                        && state.is_retired(&item.record.id)
                        && state.in_force(&item.record.id).is_none()
                    {
                        "retired"
                    } else if lifecycle == crate::agreement::Lifecycle::Accepted
                        && state.is_superseded(&item.record)
                    {
                        "superseded"
                    } else {
                        lifecycle.label()
                    }
                }
                _ => "operational",
            };
            let label = format!("{LIFECYCLE_LABEL_PREFIX}{wanted}");
            if item.labels.iter().any(|existing| existing == &label) {
                continue;
            }
            let mut args = vec!["update".to_string(), item.bead.clone()];
            for old in item
                .labels
                .iter()
                .filter(|existing| existing.starts_with(LIFECYCLE_LABEL_PREFIX))
            {
                args.push("--remove-label".into());
                args.push(old.clone());
            }
            args.push("--add-label".into());
            args.push(label);
            args.push("--dolt-auto-commit".into());
            args.push("on".into());
            let args = args.iter().map(String::as_str).collect::<Vec<_>>();
            self.bd(&args)?;
            changed += 1;
        }
        if changed > 0 {
            self.refresh()?;
        }
        Ok(changed)
    }

    /// Copy exact record revisions into another store (the push path). Each
    /// copy keeps its canonical bytes; records already present are skipped.
    /// Operational records never cross, and no canary may appear in any
    /// copied record.
    pub fn copy_to(
        &self,
        destination: &RecordStore,
        selected: &[RecordRef],
        private_canaries: &[&str],
    ) -> Result<Vec<RecordRef>, StorageError> {
        if self.dir == destination.dir {
            return Err(StorageError::InvalidProjectionBoundary);
        }
        let mut records = Vec::new();
        for reference in selected {
            let record = self
                .get(reference)?
                .ok_or_else(|| StorageError::UnknownReference(reference.clone()))?;
            if bead_type(&record.body) == "receipt" {
                return Err(StorageError::InvalidProjectionBoundary);
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
        let mut copied = Vec::new();
        let _guard = destination
            .write_guard
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let _lock = destination.write_lock()?;
        let mut present = destination.refresh()?;
        for record in records {
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
            let supersedes_bead = record.supersedes.as_ref().and_then(|previous| {
                present
                    .iter()
                    .find(|item| &item.reference == previous)
                    .map(|item| item.bead.clone())
            });
            let status = (bead_type(&record.body) != "receipt").then_some("pinned");
            let stored =
                destination.create(&record, &reference, supersedes_bead.as_deref(), status)?;
            present.push(stored.clone());
            destination.remember(stored)?;
            copied.push(reference);
        }
        Ok(copied)
    }

    /// `bd dolt push` for the shared store; the private store refuses.
    pub fn push_remote(&self) -> Result<String, StorageError> {
        if self.kind == StoreKind::Private {
            return Err(StorageError::PrivateStoreHasRemote(self.dir.clone()));
        }
        self.bd(&["dolt", "push"])
    }

    /// `bd dolt pull` for the shared store; the private store refuses.
    pub fn pull_remote(&self) -> Result<String, StorageError> {
        if self.kind == StoreKind::Private {
            return Err(StorageError::PrivateStoreHasRemote(self.dir.clone()));
        }
        let output = self.bd(&["dolt", "pull"])?;
        self.refresh()?;
        Ok(output)
    }

    /// Write every record as a digest-bound logical archive.
    pub fn export_logical(&self, destination: &Path) -> Result<LogicalArchive, StorageError> {
        reject_symlink(destination)?;
        if destination.exists() {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        let records = self.all_records()?;
        let payload = json!({"schema_version": 1, "kind": self.kind, "records": records});
        let canonical = serde_json::to_vec(&payload)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        if canonical.len() > 16 * 1024 * 1024 || records.len() > 10_000 {
            return Err(StorageError::LogicalArchiveTooLarge);
        }
        let digest = ContentDigest::new(format!("sha256:{:x}", Sha256::digest(&canonical)))
            .map_err(StorageError::Domain)?;
        let document = json!({"payload_digest": digest, "payload": payload});
        write_private(
            destination,
            &serde_json::to_vec_pretty(&document)
                .map_err(|error| StorageError::Serialization(error.to_string()))?,
        )?;
        Ok(LogicalArchive {
            payload_digest: digest,
            records: records.len(),
            destination: destination.to_path_buf(),
        })
    }

    /// Restore a logical archive into a new store, verifying every digest
    /// and the complete history before and after.
    pub fn import_logical(
        archive: &Path,
        destination: &Path,
        kind: StoreKind,
    ) -> Result<(Self, LogicalArchive), StorageError> {
        if is_initialized(destination) {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        let bytes = fs::read(archive).map_err(StorageError::Io)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(StorageError::LogicalArchiveTooLarge);
        }
        let document: Value = serde_json::from_slice(&bytes)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let payload = document
            .get("payload")
            .ok_or(StorageError::LogicalArchiveDigestMismatch)?;
        let canonical = serde_json::to_vec(payload)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let actual = ContentDigest::new(format!("sha256:{:x}", Sha256::digest(&canonical)))
            .map_err(StorageError::Domain)?;
        if document.get("payload_digest").and_then(Value::as_str) != Some(actual.as_str()) {
            return Err(StorageError::LogicalArchiveDigestMismatch);
        }
        if payload.get("kind") != Some(&json!(kind)) {
            return Err(StorageError::LogicalArchiveKindMismatch);
        }
        let mut records: Vec<AgreementRecord> =
            serde_json::from_value(payload.get("records").cloned().unwrap_or(Value::Null))
                .map_err(|error| StorageError::Serialization(error.to_string()))?;
        records.sort_by(|left, right| {
            logical_stage(left)
                .cmp(&logical_stage(right))
                .then_with(|| left.id.cmp(&right.id))
                .then_with(|| left.revision.cmp(&right.revision))
        });
        let store = Self::initialize(destination, kind)?;
        for record in &records {
            let expected = store.latest(&record.id)?.map(|current| current.revision);
            store.append(record, expected)?;
        }
        let mut restored = store
            .refresh()?
            .into_iter()
            .map(|item| item.record)
            .collect::<Vec<_>>();
        let mut original = records.clone();
        let order = |left: &AgreementRecord, right: &AgreementRecord| {
            left.id
                .cmp(&right.id)
                .then(left.revision.cmp(&right.revision))
        };
        restored.sort_by(order);
        original.sort_by(order);
        if restored != original {
            return Err(StorageError::LogicalArchiveRoundTripMismatch);
        }
        Ok((
            store,
            LogicalArchive {
                payload_digest: actual,
                records: records.len(),
                destination: destination.to_path_buf(),
            },
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalArchive {
    pub payload_digest: ContentDigest,
    pub records: usize,
    pub destination: PathBuf,
}

/// Decode and verify one bead's Whetstone metadata.
fn decode_bead(bead: &str, metadata: &Value) -> Result<AgreementRecord, StorageError> {
    let malformed = |reason: &str| StorageError::MalformedBead {
        bead: bead.to_string(),
        reason: reason.to_string(),
    };
    if metadata.get("wh_schema").and_then(Value::as_u64) != Some(METADATA_SCHEMA) {
        return Err(malformed(
            "missing or unsupported Whetstone metadata schema",
        ));
    }
    let text = metadata
        .get("wh_record")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("missing wh_record"))?;
    let record: AgreementRecord =
        serde_json::from_str(text).map_err(|error| malformed(&format!("wh_record: {error}")))?;
    record
        .validate()
        .map_err(|error| malformed(&format!("record: {error}")))?;
    let digest = record
        .digest()
        .map_err(|error| malformed(&error.to_string()))?;
    if metadata.get("wh_digest").and_then(Value::as_str) != Some(digest.as_str()) {
        return Err(malformed(
            "the record does not match its digest (edited or corrupted metadata)",
        ));
    }
    if metadata.get("wh_id").and_then(Value::as_str) != Some(record.id.as_str())
        || metadata.get("wh_revision").and_then(Value::as_u64) != Some(record.revision)
        || metadata.get("wh_key").and_then(Value::as_str) != Some(record.idempotency_key.as_str())
    {
        return Err(malformed("metadata fields disagree with the record"));
    }
    Ok(record)
}

fn batch_of(key: &str) -> Option<String> {
    // Onboarding writes keys `<request>:base-<revision>:<part>`.
    key.find(":base-")
        .map(|index| format!("batch:{}", &key[..index]))
}

fn batch_marker_key(requests: &[AppendRequest<'_>]) -> String {
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

fn logical_stage(record: &AgreementRecord) -> u8 {
    match &record.body {
        RecordBody::Proposal(_) => 1,
        RecordBody::Decision(_) | RecordBody::LocalReview(_) => 2,
        RecordBody::Activation(_)
        | RecordBody::ObservationReceipt(_)
        | RecordBody::RepairSession(_)
        | RecordBody::RepairHandoff(_)
        | RecordBody::RepairAuthorityReservation(_)
        | RecordBody::RepairOperationClaim(_) => 3,
        _ => 0,
    }
}

fn map_append_domain_error(error: crate::domain::DomainError) -> StorageError {
    use crate::domain::DomainError;
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

fn reject_symlink(path: &Path) -> Result<(), StorageError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(StorageError::SymlinkPath(path.to_path_buf()));
        }
    }
    Ok(())
}

fn create_private_dir(dir: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(dir).map_err(StorageError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Rename, or copy then remove when the rename crosses filesystems.
fn move_dir(from: &Path, to: &Path) -> Result<(), StorageError> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    copy_dir(from, to)?;
    fs::remove_dir_all(from).map_err(StorageError::Io)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(to).map_err(StorageError::Io)?;
    for entry in fs::read_dir(from).map_err(StorageError::Io)? {
        let entry = entry.map_err(StorageError::Io)?;
        let kind = entry.file_type().map_err(StorageError::Io)?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target).map_err(StorageError::Io)?;
        }
    }
    Ok(())
}

/// One listing cache per store directory for the whole process, so every
/// handle on the same database reuses a listing whose fingerprint still
/// matches instead of asking bd again (each `bd list` costs ~0.3s).
fn shared_cache(dir: &Path) -> Result<SharedCache, StorageError> {
    static CACHES: std::sync::OnceLock<Mutex<std::collections::HashMap<PathBuf, SharedCache>>> =
        std::sync::OnceLock::new();
    let mut caches = CACHES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| StorageError::LockPoisoned)?;
    Ok(caches
        .entry(dir.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(None)))
        .clone())
}

fn lock_file(path: &Path) -> Result<File, StorageError> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(StorageError::Io)?;
    FileExt::lock_exclusive(&lock).map_err(StorageError::Io)?;
    Ok(lock)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).map_err(StorageError::Io)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(StorageError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_json_escapes_everything_beads_would_choke_on() {
        let raw = "{\"t\":\"a\u{2028}b é 😀 \\\"q\\\"\"}";
        let escaped = ascii_json(raw.as_bytes()).expect("escape");
        assert!(escaped.is_ascii());
        assert!(escaped.contains("\\u2028"));
        assert!(escaped.contains("\\ud83d\\ude00"));
        let back: Value = serde_json::from_str(&escaped).expect("json");
        let original: Value = serde_json::from_str(raw).expect("json");
        assert_eq!(back, original);
    }

    #[test]
    fn onboarding_keys_share_one_batch_marker() {
        assert_eq!(
            batch_of("init-1:base-0:mission").as_deref(),
            Some("batch:init-1")
        );
        assert_eq!(batch_of("change-1"), None);
    }
}
