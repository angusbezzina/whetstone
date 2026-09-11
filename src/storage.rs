//! Project layout, store kinds and storage errors. Records live in Beads
//! (see [`crate::beads`]): a private database under `.git/whetstone` that
//! never has a remote, and the repository's shared Beads database.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::{AgreementRecord, DomainError, RecordId, RecordRef};

pub use crate::beads::RecordStore;

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

    /// The private Beads database directory (holding `.beads`).
    pub fn private_store(&self) -> PathBuf {
        self.state_root.join(StoreKind::Private.as_str())
    }

    pub fn private_store_exists(&self) -> bool {
        crate::beads::is_initialized(&self.private_store())
    }

    /// The repository's shared Beads database directory, when one exists.
    pub fn shared_store(&self) -> Option<PathBuf> {
        crate::beads::shared_dir(&self.project_root)
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
    /// Two beads claim the same revision of a record with different content.
    ConflictingRevision {
        id: RecordId,
        revision: u64,
        beads: Vec<String>,
    },
    /// The private store must never be able to push anywhere.
    PrivateStoreHasRemote(PathBuf),
    InvalidProjectScope(String),
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
    ExclusiveRecordExists(RecordRef),
    EmptyBatch,
    UnknownReference(RecordRef),
    DigestMismatch(RecordId),
    UnexpectedData(String),
    InvalidProjectionBoundary,
    PrivateCanaryFound(String),
    LogicalArchiveTooLarge,
    LogicalArchiveKindMismatch,
    LogicalArchiveDigestMismatch,
    LogicalArchiveRoundTripMismatch,
    InvalidDestination,
    DestinationExists(PathBuf),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for StorageError {}
