//! Direct Dolt persistence behind a narrow, typed repository boundary.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::domain::{
    AgreementHistory, AgreementRecord, ContentDigest, DomainError, RecordBody, RecordId, RecordRef,
};

pub const SUPPORTED_DOLT_VERSION: &str = "2.2.3";
const STORAGE_SCHEMA_VERSION: u64 = 1;
const MIGRATION_V1: &str = r#"
CREATE TABLE whetstone_migrations (
  version BIGINT UNSIGNED PRIMARY KEY,
  checksum VARCHAR(71) NOT NULL,
  applied_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)
);
CREATE TABLE records (
  record_id VARCHAR(96) NOT NULL,
  revision BIGINT UNSIGNED NOT NULL,
  digest VARCHAR(71) NOT NULL,
  visibility VARCHAR(16) NOT NULL,
  idempotency_key VARCHAR(200) NOT NULL UNIQUE,
  record_json LONGTEXT NOT NULL,
  created_at TIMESTAMP(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
  PRIMARY KEY (record_id, revision),
  UNIQUE KEY record_digest (record_id, digest)
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    Private,
    Shareable,
}

impl StoreKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Shareable => "shareable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectLayout {
    project_root: PathBuf,
    state_root: PathBuf,
    project_id: String,
}

impl ProjectLayout {
    pub fn resolve(start: &Path, monorepo_scope: Option<&str>) -> Result<Self, StorageError> {
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
        let scope = monorepo_scope.unwrap_or(".");
        validate_scope_path(scope)?;
        let identity = format!("{}\0{scope}", common.display());
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

    pub fn store_path(&self, kind: StoreKind) -> PathBuf {
        self.state_root.join(kind.as_str())
    }

    pub fn projections_path(&self) -> PathBuf {
        self.state_root.join("projections")
    }
}

fn validate_scope_path(scope: &str) -> Result<(), StorageError> {
    let path = Path::new(scope);
    if scope.is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        Err(StorageError::InvalidProjectScope(scope.to_string()))
    } else {
        Ok(())
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

#[derive(Debug, Clone)]
pub struct DoltRepository {
    root: PathBuf,
    kind: StoreKind,
    write_guard: Arc<Mutex<()>>,
}

impl DoltRepository {
    pub fn initialize(root: &Path, kind: StoreKind) -> Result<Self, StorageError> {
        ensure_dolt_version()?;
        reject_symlink(root)?;
        fs::create_dir_all(root).map_err(StorageError::Io)?;
        let root = root.canonicalize().map_err(StorageError::Io)?;
        let repository = Self {
            root,
            kind,
            write_guard: Arc::new(Mutex::new(())),
        };
        if !repository.root.join(".dolt").is_dir() {
            repository.run(&[
                "init",
                "--name",
                "Whetstone",
                "--email",
                "local@whetstone.invalid",
                "--initial-branch",
                "main",
            ])?;
            repository.apply_initial_migration()?;
        }
        repository.verify_schema()?;
        Ok(repository)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn kind(&self) -> StoreKind {
        self.kind
    }

    pub fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        self.append_with_crash(record, expected_revision, CrashPoint::None)
    }

    pub fn append_with_crash(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
        crash: CrashPoint,
    ) -> Result<RecordRef, StorageError> {
        record.validate().map_err(StorageError::Domain)?;
        let reference = record.reference().map_err(StorageError::Domain)?;
        let _guard = self
            .write_guard
            .lock()
            .map_err(|_| StorageError::LockPoisoned)?;

        if let Some(existing) = self.find_idempotency(&record.idempotency_key)? {
            return if existing == reference {
                Ok(existing)
            } else {
                Err(StorageError::IdempotencyConflict(
                    record.idempotency_key.clone(),
                ))
            };
        }
        let actual = self.latest_revision(&record.id)?;
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
        if crash == CrashPoint::BeforeCommit {
            return Err(StorageError::InjectedCrash(crash));
        }

        let json = String::from_utf8(record.canonical_json().map_err(StorageError::Domain)?)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let query = format!(
            "SET @@dolt_transaction_commit=1; START TRANSACTION; \
             INSERT INTO records (record_id, revision, digest, visibility, idempotency_key, record_json) \
             VALUES ({}, {}, {}, {}, {}, {}); COMMIT;",
            sql_text(record.id.as_str()),
            record.revision,
            sql_text(reference.digest.as_str()),
            sql_text(self.kind.as_str()),
            sql_text(&record.idempotency_key),
            sql_text(&json),
        );
        self.sql(&query)?;
        if crash == CrashPoint::AfterCommitBeforeReceipt {
            return Err(StorageError::InjectedCrash(crash));
        }
        let persisted = self
            .get(&reference)?
            .ok_or_else(|| StorageError::AcceptedRecordMissing(reference.clone()))?;
        if persisted.reference().map_err(StorageError::Domain)? != reference {
            return Err(StorageError::DigestMismatch(record.id.clone()));
        }
        Ok(reference)
    }

    pub fn get(&self, reference: &RecordRef) -> Result<Option<AgreementRecord>, StorageError> {
        let query = format!(
            "SELECT digest, record_json FROM records WHERE record_id={} AND revision={}",
            sql_text(reference.id.as_str()),
            reference.revision
        );
        let rows = self.sql_rows(&query)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let digest = row_string(row, "digest")?;
        if digest != reference.digest.as_str() {
            return Err(StorageError::DigestMismatch(reference.id.clone()));
        }
        let record: AgreementRecord = serde_json::from_str(row_string(row, "record_json")?)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        record.validate().map_err(StorageError::Domain)?;
        if record.reference().map_err(StorageError::Domain)? != *reference {
            return Err(StorageError::DigestMismatch(reference.id.clone()));
        }
        Ok(Some(record))
    }

    /// Resolve a previously accepted request without creating a new revision.
    ///
    /// Command adapters use this before optimistic-revision checks so a retry
    /// after a lost response returns the original result instead of appearing
    /// stale. A reused key with different input is still rejected by the
    /// service (and, defensively, by `append`).
    pub fn by_idempotency_key(&self, key: &str) -> Result<Option<AgreementRecord>, StorageError> {
        match self.find_idempotency(key)? {
            Some(reference) => self.get(&reference),
            None => Ok(None),
        }
    }

    pub fn latest(&self, id: &RecordId) -> Result<Option<AgreementRecord>, StorageError> {
        let query = format!(
            "SELECT digest, record_json FROM records WHERE record_id={} ORDER BY revision DESC LIMIT 1",
            sql_text(id.as_str())
        );
        let rows = self.sql_rows(&query)?;
        let Some(row) = rows.first() else {
            return Ok(None);
        };
        let record: AgreementRecord = serde_json::from_str(row_string(row, "record_json")?)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let actual = record.digest().map_err(StorageError::Domain)?;
        if actual.as_str() != row_string(row, "digest")? {
            return Err(StorageError::DigestMismatch(id.clone()));
        }
        Ok(Some(record))
    }

    pub fn history(&self, id: &RecordId) -> Result<Vec<AgreementRecord>, StorageError> {
        let query = format!(
            "SELECT digest, record_json FROM records WHERE record_id={} ORDER BY revision",
            sql_text(id.as_str())
        );
        self.sql_rows(&query)?
            .iter()
            .map(|row| {
                let record: AgreementRecord = serde_json::from_str(row_string(row, "record_json")?)
                    .map_err(|error| StorageError::Serialization(error.to_string()))?;
                let actual = record.digest().map_err(StorageError::Domain)?;
                if actual.as_str() != row_string(row, "digest")? {
                    return Err(StorageError::DigestMismatch(id.clone()));
                }
                Ok(record)
            })
            .collect()
    }

    pub fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        self.sql_rows("SELECT digest, record_json FROM records ORDER BY record_id, revision")?
            .iter()
            .map(|row| {
                let record: AgreementRecord = serde_json::from_str(row_string(row, "record_json")?)
                    .map_err(|error| StorageError::Serialization(error.to_string()))?;
                if record.digest().map_err(StorageError::Domain)?.as_str()
                    != row_string(row, "digest")?
                {
                    return Err(StorageError::DigestMismatch(record.id.clone()));
                }
                Ok(record)
            })
            .collect()
    }

    pub fn project_selected(
        &self,
        destination: &DoltRepository,
        allowlist: &[RecordRef],
        private_canaries: &[&str],
    ) -> Result<ProjectionReceipt, StorageError> {
        if self.kind != StoreKind::Private || destination.kind != StoreKind::Shareable {
            return Err(StorageError::InvalidProjectionBoundary);
        }
        let source = self.root.canonicalize().map_err(StorageError::Io)?;
        let target = destination.root.canonicalize().map_err(StorageError::Io)?;
        if source == target || target.starts_with(&source) || source.starts_with(&target) {
            return Err(StorageError::InvalidProjectionBoundary);
        }
        let mut copied = Vec::new();
        for reference in allowlist {
            let record = self
                .get(reference)?
                .ok_or_else(|| StorageError::UnknownReference(reference.clone()))?;
            let canonical = record.canonical_json().map_err(StorageError::Domain)?;
            for canary in private_canaries {
                if !canary.is_empty() && contains_bytes(&canonical, canary.as_bytes()) {
                    return Err(StorageError::PrivateCanaryFound((*canary).to_string()));
                }
            }
            let expected = destination.latest_revision(&record.id)?;
            copied.push(destination.append(&record, expected)?);
        }
        for canary in private_canaries {
            if !canary.is_empty() && tree_contains_bytes(&destination.root, canary.as_bytes())? {
                return Err(StorageError::PrivateCanaryFound((*canary).to_string()));
            }
        }
        Ok(ProjectionReceipt {
            source_store_digest: repository_head(self)?,
            destination_store_digest: repository_head(destination)?,
            records: copied,
        })
    }

    pub fn render_projection(
        &self,
        projection_root: &Path,
        selected: &[RecordRef],
    ) -> Result<RenderedProjection, StorageError> {
        self.render_projection_with_crash(projection_root, selected, CrashPoint::None)
    }

    pub fn render_projection_with_crash(
        &self,
        projection_root: &Path,
        selected: &[RecordRef],
        crash: CrashPoint,
    ) -> Result<RenderedProjection, StorageError> {
        reject_symlink(projection_root)?;
        fs::create_dir_all(projection_root).map_err(StorageError::Io)?;
        let mut records = Vec::new();
        for reference in selected {
            records.push(
                self.get(reference)?
                    .ok_or_else(|| StorageError::UnknownReference(reference.clone()))?,
            );
        }
        let payload = ProjectionPayload {
            schema_version: 1,
            base_store_digest: repository_head(self)?,
            records,
        };
        let compact = serde_json::to_vec(&payload)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let digest = ContentDigest::new(format!("sha256:{:x}", Sha256::digest(&compact)))
            .map_err(StorageError::Domain)?;
        let generation = digest.as_str().trim_start_matches("sha256:");
        let generation_root = projection_root.join(generation);
        if !generation_root.exists() {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| StorageError::Serialization(error.to_string()))?
                .as_nanos();
            let staging = projection_root.join(format!(".{generation}-{nonce}.staging"));
            fs::create_dir(&staging).map_err(StorageError::Io)?;
            let json = serde_json::to_vec_pretty(&ProjectionDocument {
                payload_digest: digest.clone(),
                payload: payload.clone(),
            })
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
            if let Err(error) = atomic_write(&staging.join("agreement.json"), &json) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
            let markdown = render_markdown(&payload, &digest);
            if let Err(error) = atomic_write(&staging.join("agreement.md"), markdown.as_bytes()) {
                let _ = fs::remove_dir_all(&staging);
                return Err(error);
            }
            fs::rename(&staging, &generation_root).map_err(StorageError::Io)?;
            File::open(projection_root)
                .and_then(|directory| directory.sync_all())
                .map_err(StorageError::Io)?;
        } else if !generation_root.join("agreement.json").is_file()
            || !generation_root.join("agreement.md").is_file()
        {
            return Err(StorageError::ProjectionTampered(
                "incomplete projection generation".into(),
            ));
        }
        if crash == CrashPoint::AfterProjectionGeneration {
            return Err(StorageError::InjectedCrash(
                CrashPoint::AfterProjectionGeneration,
            ));
        }
        atomic_write(&projection_root.join("current"), generation.as_bytes())?;
        Ok(RenderedProjection {
            digest,
            generation_root,
        })
    }

    pub fn inspect_projection(
        projection_root: &Path,
    ) -> Result<ProjectionInspection, StorageError> {
        let generation =
            fs::read_to_string(projection_root.join("current")).map_err(StorageError::Io)?;
        validate_generation_name(&generation)?;
        let path = projection_root.join(generation).join("agreement.json");
        let bytes = fs::read(&path).map_err(StorageError::Io)?;
        let document: ProjectionDocument = serde_json::from_slice(&bytes)
            .map_err(|error| StorageError::ProjectionTampered(error.to_string()))?;
        let compact = serde_json::to_vec(&document.payload)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let actual = ContentDigest::new(format!("sha256:{:x}", Sha256::digest(compact)))
            .map_err(StorageError::Domain)?;
        if actual != document.payload_digest {
            return Ok(ProjectionInspection::TamperedDraft {
                expected: document.payload_digest,
                actual,
                path,
            });
        }
        Ok(ProjectionInspection::Verified {
            digest: actual,
            records: document.payload.records,
        })
    }

    pub fn backup_to(&self, destination: &Path) -> Result<BackupReceipt, StorageError> {
        reject_symlink(destination)?;
        if destination.exists() {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        let absolute = absolute_without_existing(destination)?;
        self.run(&["backup", "sync-url", &file_url(&absolute)])?;
        Ok(BackupReceipt {
            source_head: repository_head(self)?,
            destination: absolute,
            kind: self.kind,
        })
    }

    pub fn export_logical(
        &self,
        destination: &Path,
    ) -> Result<LogicalArchiveReceipt, StorageError> {
        reject_symlink(destination)?;
        if destination.exists() {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        let payload = LogicalArchivePayload {
            schema_version: 1,
            kind: self.kind,
            source_head: repository_head(self)?,
            records: self.all_records()?,
        };
        let canonical = serde_json::to_vec(&payload)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        if canonical.len() > 16 * 1024 * 1024 || payload.records.len() > 10_000 {
            return Err(StorageError::LogicalArchiveTooLarge);
        }
        let digest = ContentDigest::new(format!("sha256:{:x}", Sha256::digest(canonical)))
            .map_err(StorageError::Domain)?;
        let document = LogicalArchiveDocument {
            payload_digest: digest.clone(),
            payload,
        };
        let json = serde_json::to_vec_pretty(&document)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        atomic_write(destination, &json)?;
        Ok(LogicalArchiveReceipt {
            payload_digest: digest,
            records: document.payload.records.len(),
            destination: destination.to_path_buf(),
        })
    }

    pub fn import_logical(
        archive: &Path,
        destination: &Path,
        expected_kind: StoreKind,
    ) -> Result<(Self, LogicalArchiveReceipt), StorageError> {
        if destination.exists() {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        let bytes = fs::read(archive).map_err(StorageError::Io)?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err(StorageError::LogicalArchiveTooLarge);
        }
        let document: LogicalArchiveDocument = serde_json::from_slice(&bytes)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        if document.payload.schema_version != 1 {
            return Err(StorageError::UnsupportedLogicalArchiveSchema(
                document.payload.schema_version,
            ));
        }
        if document.payload.kind != expected_kind {
            return Err(StorageError::LogicalArchiveKindMismatch);
        }
        if document.payload.records.len() > 10_000 {
            return Err(StorageError::LogicalArchiveTooLarge);
        }
        let canonical = serde_json::to_vec(&document.payload)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        let actual = ContentDigest::new(format!("sha256:{:x}", Sha256::digest(canonical)))
            .map_err(StorageError::Domain)?;
        if actual != document.payload_digest {
            return Err(StorageError::LogicalArchiveDigestMismatch);
        }
        validate_logical_records(&document.payload.records)?;

        let repository = Self::initialize(destination, expected_kind)?;
        if !repository.all_records()?.is_empty() {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        if !document.payload.records.is_empty() {
            let mut query = String::from("SET @@dolt_transaction_commit=1; START TRANSACTION;");
            for record in &document.payload.records {
                let reference = record.reference().map_err(StorageError::Domain)?;
                let json =
                    String::from_utf8(record.canonical_json().map_err(StorageError::Domain)?)
                        .map_err(|error| StorageError::Serialization(error.to_string()))?;
                query.push_str(&format!(
                    "INSERT INTO records (record_id, revision, digest, visibility, idempotency_key, record_json) VALUES ({}, {}, {}, {}, {}, {});",
                    sql_text(record.id.as_str()),
                    record.revision,
                    sql_text(reference.digest.as_str()),
                    sql_text(expected_kind.as_str()),
                    sql_text(&record.idempotency_key),
                    sql_text(&json),
                ));
            }
            query.push_str("COMMIT;");
            repository.sql(&query)?;
        }
        let imported = repository.all_records()?;
        if imported != document.payload.records {
            return Err(StorageError::LogicalArchiveRoundTripMismatch);
        }
        let receipt = LogicalArchiveReceipt {
            payload_digest: actual,
            records: imported.len(),
            destination: destination.to_path_buf(),
        };
        Ok((repository, receipt))
    }

    pub fn restore_from(
        backup: &Path,
        destination: &Path,
        kind: StoreKind,
    ) -> Result<Self, StorageError> {
        if destination.exists() {
            return Err(StorageError::DestinationExists(destination.to_path_buf()));
        }
        let backup = backup.canonicalize().map_err(StorageError::Io)?;
        let parent = destination
            .parent()
            .ok_or(StorageError::ProjectRootNotFound)?;
        fs::create_dir_all(parent).map_err(StorageError::Io)?;
        let parent = parent.canonicalize().map_err(StorageError::Io)?;
        let name = destination
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or(StorageError::InvalidDestination)?;
        let output = Command::new("dolt")
            .current_dir(&parent)
            .env("DOLT_DISABLE_EVENT_FLUSH", "1")
            .args(["backup", "restore", &file_url(&backup), name])
            .output()
            .map_err(StorageError::Io)?;
        checked_output("dolt backup restore", output)?;
        let repository = Self::initialize(&parent.join(name), kind)?;
        repository.health()?;
        Ok(repository)
    }

    pub fn health(&self) -> Result<StorageHealth, StorageError> {
        self.verify_schema()?;
        self.run(&["fsck"])?;
        Ok(StorageHealth {
            schema_version: STORAGE_SCHEMA_VERSION,
            head: repository_head(self)?,
            repair: None,
        })
    }

    fn apply_initial_migration(&self) -> Result<(), StorageError> {
        let checksum = format!("sha256:{:x}", Sha256::digest(MIGRATION_V1.as_bytes()));
        let query = format!(
            "SET @@dolt_transaction_commit=1; START TRANSACTION; {MIGRATION_V1} \
             INSERT INTO whetstone_migrations (version, checksum) VALUES (1, {}); COMMIT;",
            sql_text(&checksum)
        );
        self.sql(&query)?;
        Ok(())
    }

    fn verify_schema(&self) -> Result<(), StorageError> {
        let rows = self.sql_rows(
            "SELECT version, checksum FROM whetstone_migrations ORDER BY version DESC LIMIT 1",
        )?;
        let row = rows.first().ok_or(StorageError::MissingMigration)?;
        let version = row_u64(row, "version")?;
        if version > STORAGE_SCHEMA_VERSION {
            return Err(StorageError::UnsupportedSchema(version));
        }
        if version < STORAGE_SCHEMA_VERSION {
            return Err(StorageError::MissingMigration);
        }
        let expected = format!("sha256:{:x}", Sha256::digest(MIGRATION_V1.as_bytes()));
        if row_string(row, "checksum")? != expected {
            return Err(StorageError::MigrationChecksumMismatch(version));
        }
        Ok(())
    }

    fn latest_revision(&self, id: &RecordId) -> Result<Option<u64>, StorageError> {
        let query = format!(
            "SELECT MAX(revision) AS revision FROM records WHERE record_id={}",
            sql_text(id.as_str())
        );
        let rows = self.sql_rows(&query)?;
        let value = rows
            .first()
            .and_then(|row| row.get("revision"))
            .unwrap_or(&Value::Null);
        if value.is_null() {
            Ok(None)
        } else {
            value_u64(value, "revision").map(Some)
        }
    }

    fn find_idempotency(&self, key: &str) -> Result<Option<RecordRef>, StorageError> {
        let query = format!(
            "SELECT record_id, revision, digest FROM records WHERE idempotency_key={}",
            sql_text(key)
        );
        let rows = self.sql_rows(&query)?;
        rows.first().map(row_reference).transpose()
    }

    fn sql(&self, query: &str) -> Result<String, StorageError> {
        self.run(&["sql", "-q", query])
    }

    fn sql_rows(&self, query: &str) -> Result<Vec<Value>, StorageError> {
        let output = self.run(&["sql", "-r", "json", "-q", query])?;
        let value: Value = serde_json::from_str(&output)
            .map_err(|error| StorageError::Serialization(error.to_string()))?;
        match value.get("rows") {
            Some(rows) => rows
                .as_array()
                .cloned()
                .ok_or_else(|| StorageError::UnexpectedData("rows".into())),
            None if value.as_object().is_some_and(serde_json::Map::is_empty) => Ok(Vec::new()),
            None => Err(StorageError::UnexpectedData("rows".into())),
        }
    }

    fn run(&self, args: &[&str]) -> Result<String, StorageError> {
        let output = Command::new("dolt")
            .current_dir(&self.root)
            .env("DOLT_DISABLE_EVENT_FLUSH", "1")
            .args(args)
            .output()
            .map_err(StorageError::Io)?;
        checked_output("dolt", output)
    }
}

fn ensure_dolt_version() -> Result<(), StorageError> {
    let output = Command::new("dolt")
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .arg("version")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                StorageError::DoltMissing
            } else {
                StorageError::Io(error)
            }
        })?;
    let version = checked_output("dolt version", output)?;
    let found = version.split_whitespace().nth(2).unwrap_or_default().trim();
    if found == SUPPORTED_DOLT_VERSION {
        Ok(())
    } else {
        Err(StorageError::UnsupportedDoltVersion {
            expected: SUPPORTED_DOLT_VERSION,
            found: found.to_string(),
        })
    }
}

fn repository_head(repository: &DoltRepository) -> Result<ContentDigest, StorageError> {
    let output = repository.run(&["log", "-n", "1", "--oneline"])?;
    let head = output.split_whitespace().next().unwrap_or_default();
    if head.is_empty() {
        return Err(StorageError::UnexpectedData("dolt head".into()));
    }
    ContentDigest::new(format!("sha256:{:x}", Sha256::digest(head.as_bytes())))
        .map_err(StorageError::Domain)
}

fn sql_text(value: &str) -> String {
    let hex = value
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("CONVERT(X'{hex}' USING utf8mb4)")
}

fn row_string<'a>(row: &'a Value, key: &str) -> Result<&'a str, StorageError> {
    row.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| StorageError::UnexpectedData(key.to_string()))
}

fn row_u64(row: &Value, key: &str) -> Result<u64, StorageError> {
    let value = row
        .get(key)
        .ok_or_else(|| StorageError::UnexpectedData(key.to_string()))?;
    value_u64(value, key)
}

fn value_u64(value: &Value, key: &str) -> Result<u64, StorageError> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        .ok_or_else(|| StorageError::UnexpectedData(key.to_string()))
}

fn row_reference(row: &Value) -> Result<RecordRef, StorageError> {
    Ok(RecordRef {
        id: RecordId::new(row_string(row, "record_id")?).map_err(StorageError::Domain)?,
        revision: row_u64(row, "revision")?,
        digest: ContentDigest::new(row_string(row, "digest")?).map_err(StorageError::Domain)?,
    })
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
            message: redact_command_error(&stderr),
        })
    }
}

fn redact_command_error(message: &str) -> String {
    let first = message.lines().next().unwrap_or("command failed");
    first.chars().take(240).collect()
}

fn reject_symlink(path: &Path) -> Result<(), StorageError> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() {
            return Err(StorageError::SymlinkPath(path.to_path_buf()));
        }
    }
    Ok(())
}

fn absolute_without_existing(path: &Path) -> Result<PathBuf, StorageError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(StorageError::Io)?
            .join(path))
    }
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let parent = path.parent().ok_or(StorageError::InvalidDestination)?;
    fs::create_dir_all(parent).map_err(StorageError::Io)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| StorageError::Serialization(error.to_string()))?
        .as_nanos();
    let temp = parent.join(format!(".whetstone-{nonce}.tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(StorageError::Io)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temp);
        return Err(StorageError::Io(error));
    }
    fs::rename(&temp, path).map_err(StorageError::Io)?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(StorageError::Io)
}

fn validate_generation_name(value: &str) -> Result<(), StorageError> {
    if value.len() == 64 && value.chars().all(|character| character.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(StorageError::ProjectionTampered(
            "invalid current generation".into(),
        ))
    }
}

fn render_markdown(payload: &ProjectionPayload, digest: &ContentDigest) -> String {
    let mut markdown = format!(
        "# Whetstone agreement\n\nProjection: `{}`\n\nBase store: `{}`\n\n",
        digest.as_str(),
        payload.base_store_digest.as_str()
    );
    for record in &payload.records {
        markdown.push_str(&format!(
            "## {} · revision {}\n\n- Scope: `{}`\n- Owner: `{:?}:{}`\n- Provenance: `{:?}` from `{}` source(s)\n- Record type: `{}`\n- Digest: `{}`\n\n",
            record.id.as_str(),
            record.revision,
            record.scope.project,
            record.owner.kind,
            record.owner.stable_id,
            record.provenance.kind,
            record.provenance.sources.len(),
            record_type_name(record),
            record.digest().map_or_else(
                |_| "invalid".to_string(),
                |value| value.as_str().to_string()
            )
        ));
    }
    markdown
}

fn record_type_name(record: &AgreementRecord) -> &'static str {
    use crate::domain::RecordBody;
    match &record.body {
        RecordBody::Mission(_) => "mission",
        RecordBody::CoreValue(_) => "core_value",
        RecordBody::ImplementationPhilosophy(_) => "implementation_philosophy",
        RecordBody::Standard(_) => "standard",
        RecordBody::Guidance(_) => "guidance",
        RecordBody::MetricDefinition(_) => "metric_definition",
        RecordBody::SourceSnapshot(_) => "source_snapshot",
        RecordBody::Proposal(_) => "proposal",
        RecordBody::Decision(_) => "decision",
        RecordBody::Mandate(_) => "mandate",
        RecordBody::Activation(_) => "activation",
        RecordBody::VerificationReceipt(_) => "verification_receipt",
        RecordBody::ObservationReceipt(_) => "observation_receipt",
        RecordBody::Retirement(_) => "retirement",
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn tree_contains_bytes(root: &Path, needle: &[u8]) -> Result<bool, StorageError> {
    if needle.is_empty() || needle.len() > 4096 {
        return Err(StorageError::InvalidCanaryLength);
    }
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| StorageError::Walk(error.to_string()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        if file_contains_bytes(entry.path(), needle)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn file_contains_bytes(path: &Path, needle: &[u8]) -> Result<bool, StorageError> {
    let mut file = File::open(path).map_err(StorageError::Io)?;
    let mut chunk = [0_u8; 64 * 1024];
    let mut overlap = Vec::new();
    loop {
        let read = file.read(&mut chunk).map_err(StorageError::Io)?;
        if read == 0 {
            return Ok(false);
        }
        let mut window = Vec::with_capacity(overlap.len() + read);
        window.extend_from_slice(&overlap);
        window.extend_from_slice(&chunk[..read]);
        if contains_bytes(&window, needle) {
            return Ok(true);
        }
        let keep = needle.len().saturating_sub(1).min(window.len());
        overlap.clear();
        overlap.extend_from_slice(&window[window.len() - keep..]);
    }
}

fn validate_logical_records(records: &[AgreementRecord]) -> Result<(), StorageError> {
    let mut ordered = records.to_vec();
    ordered.sort_by(|left, right| {
        logical_stage(left)
            .cmp(&logical_stage(right))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.revision.cmp(&right.revision))
    });
    let mut history = AgreementHistory::default();
    for record in ordered {
        let expected = history.latest(&record.id).map(|current| current.revision);
        history
            .append(record, expected)
            .map_err(StorageError::Domain)?;
    }
    Ok(())
}

fn logical_stage(record: &AgreementRecord) -> u8 {
    match &record.body {
        RecordBody::Decision(_) => 1,
        RecordBody::Activation(_) | RecordBody::ObservationReceipt(_) => 2,
        _ => 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPoint {
    None,
    BeforeCommit,
    AfterCommitBeforeReceipt,
    AfterProjectionGeneration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionReceipt {
    pub source_store_digest: ContentDigest,
    pub destination_store_digest: ContentDigest,
    pub records: Vec<RecordRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedProjection {
    pub digest: ContentDigest,
    pub generation_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionInspection {
    Verified {
        digest: ContentDigest,
        records: Vec<AgreementRecord>,
    },
    TamperedDraft {
        expected: ContentDigest,
        actual: ContentDigest,
        path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionPayload {
    schema_version: u16,
    base_store_digest: ContentDigest,
    records: Vec<AgreementRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectionDocument {
    payload_digest: ContentDigest,
    payload: ProjectionPayload,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalArchivePayload {
    schema_version: u16,
    kind: StoreKind,
    source_head: ContentDigest,
    records: Vec<AgreementRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogicalArchiveDocument {
    payload_digest: ContentDigest,
    payload: LogicalArchivePayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalArchiveReceipt {
    pub payload_digest: ContentDigest,
    pub records: usize,
    pub destination: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupReceipt {
    pub source_head: ContentDigest,
    pub destination: PathBuf,
    pub kind: StoreKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageHealth {
    pub schema_version: u64,
    pub head: ContentDigest,
    pub repair: Option<String>,
}

#[derive(Debug)]
/// An owned, short-lived concurrency helper for local storage tests and adapters.
///
/// This loopback server is not an authentication boundary and must not be
/// exposed by hosted or user-facing workflows without the separately reviewed
/// credential and transport adapter.
pub struct DoltServer {
    child: Child,
    port: u16,
    data_dir: PathBuf,
    database: String,
    ownership_file: PathBuf,
    ownership_token: String,
    stopped: bool,
}

impl DoltServer {
    pub fn start(data_dir: &Path, database: &str) -> Result<Self, StorageError> {
        ensure_dolt_version()?;
        let data_dir = data_dir.canonicalize().map_err(StorageError::Io)?;
        validate_scope_path(database)?;
        if !data_dir.join(database).join(".dolt").is_dir() {
            return Err(StorageError::InvalidDestination);
        }
        let port = TcpListener::bind(("127.0.0.1", 0))
            .map_err(StorageError::Io)?
            .local_addr()
            .map_err(StorageError::Io)?
            .port();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| StorageError::Serialization(error.to_string()))?
            .as_nanos();
        let ownership_token = format!(
            "{:x}",
            Sha256::digest(format!("{}:{port}:{nonce}", data_dir.display()).as_bytes())
        );
        let ownership_file = data_dir.join(format!(".whetstone-server-{port}.json"));
        let mut owner = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&ownership_file)
            .map_err(StorageError::Io)?;
        owner
            .write_all(ownership_token.as_bytes())
            .and_then(|()| owner.sync_all())
            .map_err(StorageError::Io)?;
        let child_result = Command::new("dolt")
            .env("DOLT_DISABLE_EVENT_FLUSH", "1")
            .args([
                "sql-server",
                "--data-dir",
                data_dir.to_str().ok_or(StorageError::InvalidDestination)?,
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--loglevel",
                "error",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let child = match child_result {
            Ok(child) => child,
            Err(error) => {
                let _ = fs::remove_file(&ownership_file);
                return Err(StorageError::Io(error));
            }
        };
        let mut server = Self {
            child,
            port,
            data_dir,
            database: database.to_string(),
            ownership_file,
            ownership_token,
            stopped: false,
        };
        server.wait_ready(Duration::from_secs(5))?;
        Ok(server)
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn is_ready(&mut self) -> Result<bool, StorageError> {
        if self.child.try_wait().map_err(StorageError::Io)?.is_some() {
            return Ok(false);
        }
        if TcpStream::connect(("127.0.0.1", self.port)).is_err() {
            return Ok(false);
        }
        let port = self.port.to_string();
        let output = Command::new("dolt")
            .env("DOLT_DISABLE_EVENT_FLUSH", "1")
            .args([
                "--host=127.0.0.1",
                &format!("--port={port}"),
                "--no-tls",
                &format!("--use-db={}", self.database),
                "sql",
                "-r",
                "json",
                "-q",
                "SELECT 1 AS ready",
            ])
            .output()
            .map_err(StorageError::Io)?;
        Ok(output.status.success())
    }

    pub fn shutdown(&mut self) -> Result<(), StorageError> {
        if self.stopped {
            return Ok(());
        }
        let token = fs::read_to_string(&self.ownership_file).map_err(StorageError::Io)?;
        if token != self.ownership_token {
            return Err(StorageError::ServerOwnershipMismatch);
        }
        let pid = self.child.id().to_string();
        let terminated = Command::new("kill")
            .args(["-TERM", &pid])
            .status()
            .map_err(StorageError::Io)?;
        if !terminated.success() {
            return Err(StorageError::ServerShutdownFailed);
        }
        let started = Instant::now();
        loop {
            if self.child.try_wait().map_err(StorageError::Io)?.is_some() {
                break;
            }
            if started.elapsed() >= Duration::from_secs(2) {
                self.child.kill().map_err(StorageError::Io)?;
                self.child.wait().map_err(StorageError::Io)?;
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        fs::remove_file(&self.ownership_file).map_err(StorageError::Io)?;
        self.stopped = true;
        Ok(())
    }

    fn wait_ready(&mut self, timeout: Duration) -> Result<(), StorageError> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.is_ready()? {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(50));
        }
        let _ = self.shutdown();
        Err(StorageError::ServerReadinessTimeout)
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }
}

impl Drop for DoltServer {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.shutdown();
        }
    }
}

#[derive(Debug)]
pub enum StorageError {
    Io(std::io::Error),
    Walk(String),
    Domain(DomainError),
    Serialization(String),
    CommandFailed {
        command: &'static str,
        status: Option<i32>,
        message: String,
    },
    DoltMissing,
    UnsupportedDoltVersion {
        expected: &'static str,
        found: String,
    },
    UnsupportedSchema(u64),
    MissingMigration,
    MigrationChecksumMismatch(u64),
    InvalidProjectScope(String),
    ProjectRootNotFound,
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
    UnknownReference(RecordRef),
    AcceptedRecordMissing(RecordRef),
    DigestMismatch(RecordId),
    UnexpectedData(String),
    InjectedCrash(CrashPoint),
    InvalidProjectionBoundary,
    PrivateCanaryFound(String),
    InvalidCanaryLength,
    ProjectionTampered(String),
    LogicalArchiveTooLarge,
    UnsupportedLogicalArchiveSchema(u16),
    LogicalArchiveKindMismatch,
    LogicalArchiveDigestMismatch,
    LogicalArchiveRoundTripMismatch,
    InvalidDestination,
    DestinationExists(PathBuf),
    ServerReadinessTimeout,
    ServerOwnershipMismatch,
    ServerShutdownFailed,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for StorageError {}
