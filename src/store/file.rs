//! The private file store: drafts and receipts as files under
//! `.git/whetstone`, so the Virgil arm never needs `bd`.
//!
//! One JSON file per record revision, `<root>/<id>/<revision>.json`, holding
//! the same metadata a bead carries (the record as ASCII-escaped canonical
//! JSON beside its id, revision, digest, supersedes reference and
//! idempotency key), verified on every read. `<root>/.index.json` holds what
//! changes: completed batch markers and the lifecycle view. Revision files
//! are immutable and written once (temporary file, then rename); every write
//! is serialized behind `<root>/.lock`. A file that does not verify is
//! reported with its path and never repaired.
//!
//! Record ids keep `[a-z0-9_-]` and any `.` after the first character as
//! they are; every other character becomes `%xx`, so ids that differ only in
//! case never share a directory and no id can name the index.
//!
//! The file store is always private: it has no remote, it can copy into a
//! shareable store, and nothing copies into it except migration.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::domain::{AgreementRecord, RecordId, RecordRef};
use crate::storage::{
    batch_marker_key, check_append, check_batch, decode_metadata, initial_lifecycle,
    pending_batches, record_metadata, reference_text, resolved_lifecycle, validate_for_append,
    AppendRequest, IncompleteBatch, RecordStore, StorageError, StoreBackend, StoreKind,
};

const INDEX: &str = ".index.json";
const LOCK: &str = ".lock";
const INDEX_SCHEMA: u64 = 1;

/// Whether a directory holds an initialized file store.
pub fn is_initialized(dir: &Path) -> bool {
    dir.join(INDEX).is_file()
}

/// The directory name for a record id (see the module documentation).
pub fn encode_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for (index, character) in id.chars().enumerate() {
        let plain = character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '_'
            || character == '-'
            || (character == '.' && index > 0);
        if plain {
            out.push(character);
        } else {
            let mut bytes = [0_u8; 4];
            for byte in character.encode_utf8(&mut bytes).bytes() {
                out.push_str(&format!("%{byte:02x}"));
            }
        }
    }
    out
}

fn decode_id(name: &str) -> Option<String> {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = name.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u64,
    /// Completed batch markers, by marker key, with their member keys.
    batches: BTreeMap<String, Vec<String>>,
    /// The lifecycle view, by `id@revision#digest`, where it moved from the
    /// revision's initial view.
    lifecycle: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct FileStore {
    dir: PathBuf,
    write_guard: Arc<Mutex<()>>,
}

impl FileStore {
    /// Open an initialized store without creating or repairing anything.
    pub fn open_existing(dir: &Path) -> Result<Self, StorageError> {
        reject_symlink(dir)?;
        if !is_initialized(dir) {
            return Err(StorageError::RepositoryNotInitialized(dir.to_path_buf()));
        }
        Self::new(dir)
    }

    /// Create the store when absent (idempotent).
    pub fn initialize(dir: &Path) -> Result<Self, StorageError> {
        reject_symlink(dir)?;
        create_private_dir(dir)?;
        let store = Self::new(dir)?;
        let _lock = store.lock()?;
        if !is_initialized(&store.dir) {
            store.write_index(&Index {
                schema: INDEX_SCHEMA,
                ..Index::default()
            })?;
        }
        Ok(store)
    }

    fn new(dir: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            dir: dir.canonicalize().map_err(StorageError::Io)?,
            write_guard: Arc::new(Mutex::new(())),
        })
    }

    pub fn root(&self) -> &Path {
        &self.dir
    }

    /// The file holding one revision.
    pub fn path_of(&self, id: &RecordId, revision: u64) -> PathBuf {
        self.dir
            .join(encode_id(id.as_str()))
            .join(format!("{revision}.json"))
    }

    fn lock(&self) -> Result<File, StorageError> {
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.dir.join(LOCK))
            .map_err(StorageError::Io)?;
        FileExt::lock_exclusive(&lock).map_err(StorageError::Io)?;
        Ok(lock)
    }

    fn index(&self) -> Result<Index, StorageError> {
        let path = self.dir.join(INDEX);
        let text = fs::read_to_string(&path).map_err(StorageError::Io)?;
        let index = serde_json::from_str::<Index>(&text).map_err(|error| {
            StorageError::MalformedRecord {
                locator: path.display().to_string(),
                reason: format!("the index does not parse: {error}"),
            }
        })?;
        if index.schema != INDEX_SCHEMA {
            return Err(StorageError::MalformedRecord {
                locator: path.display().to_string(),
                reason: format!("unsupported index schema {}", index.schema),
            });
        }
        Ok(index)
    }

    fn write_index(&self, index: &Index) -> Result<(), StorageError> {
        let bytes = serde_json::to_vec_pretty(index)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        write_atomic(&self.dir.join(INDEX), &bytes, false)
    }

    /// Every revision with the path it was read from, by id then revision.
    fn load(&self) -> Result<Vec<(AgreementRecord, PathBuf)>, StorageError> {
        let mut loaded = Vec::new();
        let mut directories = fs::read_dir(&self.dir)
            .map_err(StorageError::Io)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::Io)?;
        directories.sort_by_key(fs::DirEntry::file_name);
        for directory in directories {
            let name = directory.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let path = directory.path();
            let malformed = |path: &Path, reason: String| StorageError::MalformedRecord {
                locator: path.display().to_string(),
                reason,
            };
            if !directory.file_type().map_err(StorageError::Io)?.is_dir() {
                return Err(malformed(&path, "not a record directory".into()));
            }
            let id = decode_id(&name)
                .filter(|id| encode_id(id) == name)
                .ok_or_else(|| {
                    malformed(&path, "the directory does not name a record id".into())
                })?;
            let mut files = fs::read_dir(&path)
                .map_err(StorageError::Io)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(StorageError::Io)?;
            files.sort_by_key(fs::DirEntry::file_name);
            for file in files {
                let file_name = file.file_name().to_string_lossy().to_string();
                if file_name.starts_with('.') {
                    // An interrupted write's temporary file; never a record.
                    continue;
                }
                let file_path = file.path();
                let revision = file_name
                    .strip_suffix(".json")
                    .and_then(|stem| stem.parse::<u64>().ok())
                    .ok_or_else(|| malformed(&file_path, "not a revision file".into()))?;
                let text = fs::read_to_string(&file_path).map_err(StorageError::Io)?;
                let metadata = serde_json::from_str::<serde_json::Value>(&text)
                    .map_err(|error| malformed(&file_path, format!("not JSON: {error}")))?;
                let record =
                    decode_metadata(&metadata).map_err(|reason| malformed(&file_path, reason))?;
                if record.id.as_str() != id || record.revision != revision {
                    return Err(malformed(
                        &file_path,
                        "the file's place disagrees with the record it holds".into(),
                    ));
                }
                let supersedes = record.supersedes.as_ref().map(reference_text);
                if metadata
                    .get("wh_supersedes")
                    .and_then(serde_json::Value::as_str)
                    != supersedes.as_deref()
                {
                    return Err(malformed(
                        &file_path,
                        "metadata fields disagree with the record".into(),
                    ));
                }
                loaded.push((record, file_path));
            }
        }
        loaded.sort_by(|left, right| {
            left.0
                .id
                .cmp(&right.0.id)
                .then_with(|| left.0.revision.cmp(&right.0.revision))
        });
        Ok(loaded)
    }

    fn records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        Ok(self.load()?.into_iter().map(|(record, _)| record).collect())
    }

    /// Write one new revision file; it must not exist yet.
    fn write_revision(
        &self,
        record: &AgreementRecord,
        reference: &RecordRef,
    ) -> Result<(), StorageError> {
        let metadata = record_metadata(record, reference)?;
        let path = self.path_of(&record.id, record.revision);
        let parent = path
            .parent()
            .ok_or_else(|| StorageError::UnexpectedData("revision path has no parent".into()))?;
        create_private_dir(parent)?;
        let bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        write_atomic(&path, &bytes, true)
    }

    fn guarded<T>(
        &self,
        work: impl FnOnce() -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let _guard = self
            .write_guard
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;
        let _lock = self.lock()?;
        work()
    }
}

impl RecordStore for FileStore {
    fn backend(&self) -> StoreBackend {
        StoreBackend::Files
    }

    fn kind(&self) -> StoreKind {
        StoreKind::Private
    }

    fn location(&self) -> String {
        self.dir.display().to_string()
    }

    fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        self.records()
    }

    fn locator(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        Ok(self
            .load()?
            .into_iter()
            .find(|(record, _)| record.reference().is_ok_and(|found| &found == reference))
            .map(|(_, path)| path.display().to_string()))
    }

    fn incomplete_batches(&self) -> Result<Vec<IncompleteBatch>, StorageError> {
        let records = self.records()?;
        let completed = self.index()?.batches.into_keys().collect::<BTreeSet<_>>();
        Ok(pending_batches(&records, &completed))
    }

    fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        let reference = validate_for_append(record)?;
        self.guarded(|| {
            let existing = self.records()?;
            if let Some(already) = check_append(&existing, record, &reference, expected_revision)? {
                return Ok(already);
            }
            self.write_revision(record, &reference)?;
            Ok(reference)
        })
    }

    fn append_batch(&self, requests: &[AppendRequest<'_>]) -> Result<Vec<RecordRef>, StorageError> {
        if requests.is_empty() {
            return Err(StorageError::EmptyBatch);
        }
        for request in requests {
            validate_for_append(request.record)?;
        }
        let marker_key = batch_marker_key(requests);
        self.guarded(|| {
            let mut existing = self.records()?;
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
                    self.write_revision(request.record, &reference)?;
                    existing.push(request.record.clone());
                }
                references.push(reference);
            }
            let mut index = self.index()?;
            if !index.batches.contains_key(&marker_key) {
                index.batches.insert(
                    marker_key.clone(),
                    requests
                        .iter()
                        .map(|request| request.record.idempotency_key.clone())
                        .collect(),
                );
                self.write_index(&index)?;
            }
            Ok(references)
        })
    }

    fn sync_lifecycle_labels(
        &self,
        state: &crate::agreement::AgreementState,
    ) -> Result<usize, StorageError> {
        self.guarded(|| {
            let mut index = self.index()?;
            let mut changed = 0;
            for record in self.records()? {
                let reference = record.reference().map_err(StorageError::Domain)?;
                let key = reference_text(&reference);
                let current = index
                    .lifecycle
                    .get(&key)
                    .map_or_else(|| initial_lifecycle(&record), String::as_str);
                let wanted = resolved_lifecycle(state, &record);
                if current != wanted {
                    index.lifecycle.insert(key, wanted.to_string());
                    changed += 1;
                }
            }
            if changed > 0 {
                self.write_index(&index)?;
            }
            Ok(changed)
        })
    }

    fn lifecycle_view(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        let Some(record) = self.get(reference)? else {
            return Ok(None);
        };
        Ok(Some(
            self.index()?
                .lifecycle
                .get(&reference_text(reference))
                .cloned()
                .unwrap_or_else(|| initial_lifecycle(&record).to_string()),
        ))
    }

    fn receive(&self, records: &[AgreementRecord]) -> Result<Vec<RecordRef>, StorageError> {
        self.guarded(|| {
            let mut present = self.records()?;
            let mut copied = Vec::new();
            for record in records {
                record.validate().map_err(StorageError::Domain)?;
                let reference = record.reference().map_err(StorageError::Domain)?;
                if present
                    .iter()
                    .any(|item| item.reference().is_ok_and(|found| found == reference))
                {
                    continue;
                }
                if present
                    .iter()
                    .any(|item| item.id == record.id && item.revision == record.revision)
                {
                    return Err(StorageError::ConflictingRevision {
                        id: record.id.clone(),
                        revision: record.revision,
                        beads: Vec::new(),
                    });
                }
                self.write_revision(record, &reference)?;
                present.push(record.clone());
                copied.push(reference);
            }
            Ok(copied)
        })
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

/// Write through a temporary file in the same directory, then rename. With
/// `once`, an existing file is never replaced.
fn write_atomic(path: &Path, bytes: &[u8], once: bool) -> Result<(), StorageError> {
    let parent = path
        .parent()
        .ok_or_else(|| StorageError::UnexpectedData("path has no parent".into()))?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let staged = parent.join(format!(".{name}.{}-{sequence}.tmp", std::process::id()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&staged).map_err(StorageError::Io)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(StorageError::Io)?;
        if once {
            // A hard link fails when the target exists, so a revision file is
            // never overwritten even outside the lock. The store lives under
            // `.git`, on a local filesystem; one without hard links is not
            // supported.
            fs::hard_link(&staged, path).map_err(StorageError::Io)
        } else {
            fs::rename(&staged, path).map_err(StorageError::Io)
        }
    })();
    let _ = fs::remove_file(&staged);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_encode_reversibly_and_never_name_the_index() {
        for id in ["rule.tests-pass", "Rule.A", "rule:a", ".x.", "index.json"] {
            let encoded = encode_id(id);
            assert!(!encoded.starts_with('.'), "{encoded}");
            assert_eq!(decode_id(&encoded).as_deref(), Some(id));
        }
        assert_eq!(encode_id("rule.tests-pass"), "rule.tests-pass");
        assert_ne!(encode_id("Rule.A"), encode_id("rule.a"));
    }
}
