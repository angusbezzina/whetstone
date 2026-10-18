//! Moving a repository's records from Beads into Virgil without losing or
//! changing one: Whetstone writes its accepted records and the decisions
//! that accepted them as exactly the file `virgil rulebook import-beads`
//! reads, and flips the switch only after every exported revision reads back
//! from Records with its digest.
//!
//! The export holds the shared Beads database's records (what `wh push`
//! shared) and the private store's accepted package (what `wh push` would
//! share next), never receipts, drafts, draft ancestry or lessons (lessons
//! move through `virgil lessons import-beads`). Like the lessons export it
//! lives outside the repository, private to the person, beside it under
//! `~/.whetstone/exports/<project>`: exports, manifests and receipts are
//! never committed. Nothing in Beads or Git history is rewritten.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::beads::BeadsStore;
use crate::domain::{AgreementRecord, RecordBody};
use crate::storage::{
    logical_stage, record_class, reference_text, ProjectLayout, RecordStore, StorageError,
    StoreKind,
};

use super::virgil::{wire, VirgilStore};

pub const RULEBOOK_FILE: &str = "rulebook.json";
pub const EXPORT_SCHEMA: &str = "whetstone.rulebook-export/v1";

/// One exported revision and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub record: AgreementRecord,
    /// `shared` (the team's Beads database) or `private` (accepted here).
    pub source: &'static str,
    /// The lifecycle view the kernel's resolution gives it across both
    /// stores (`accepted`, `superseded`, `retired`, `operational`, ...).
    pub lifecycle: &'static str,
}

/// Everything the flip moves, in the order it must be applied.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Export {
    pub entries: Vec<Entry>,
    /// Accepted records the push package holds back (draft ancestry or an
    /// unshared link), reported with why.
    pub blocked: Vec<Value>,
    /// Accepted private revisions whose id and revision the shared store
    /// holds with other content, as `id@revision`: the flip is refused until
    /// they are rebased with `wh change`, never dropped.
    pub collisions: Vec<String>,
}

impl Export {
    /// The file `virgil rulebook import-beads` reads.
    pub fn file(&self, repository: &str) -> Result<Value, StorageError> {
        let records = self
            .entries
            .iter()
            .map(|entry| {
                let mut value = wire(&entry.record, entry.lifecycle)?;
                value["evidence"] = json!({"source": entry.source, "store": "beads"});
                Ok(value)
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        Ok(json!({
            "schema": EXPORT_SCHEMA,
            "scope": {"repository": repository},
            "records": records,
        }))
    }

    /// `id@revision#digest` of every exported revision.
    pub fn references(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|entry| entry.record.reference().ok())
            .map(|reference| reference_text(&reference))
            .collect()
    }
}

/// Whether a record may travel into Records at all.
fn travels(record: &AgreementRecord) -> bool {
    record_class(&record.body) != "receipt"
        && !matches!(record.body, RecordBody::Retired(_) | RecordBody::Lesson(_))
}

/// Build the export from the Beads stores, without writing anything.
pub fn build(layout: &ProjectLayout) -> Result<Export, StorageError> {
    let shared = match layout.shared_store() {
        Some(dir) => BeadsStore::open_existing(&dir, StoreKind::Shareable)?.all_records()?,
        None => Vec::new(),
    };
    let private = if crate::beads::is_initialized(&layout.private_store()) {
        BeadsStore::open_existing(&layout.private_store(), StoreKind::Private)?.all_records()?
    } else {
        Vec::new()
    };
    let mut union = shared.clone();
    union.extend(private.iter().cloned());
    let resolved = AgreementState::from_records(union);
    let lifecycle =
        |record: &AgreementRecord| crate::storage::resolved_lifecycle(&resolved, record);
    let mut entries = shared
        .iter()
        .filter(|record| travels(record))
        .map(|record| Entry {
            record: record.clone(),
            source: "shared",
            lifecycle: lifecycle(record),
        })
        .collect::<Vec<_>>();
    let shared_refs = shared
        .iter()
        .filter_map(|record| record.reference().ok())
        .collect::<BTreeSet<_>>();
    let shared_ids = shared
        .iter()
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    let state = AgreementState::from_records(private);
    let (package, mut blocked) = crate::sync::accepted_package(&state, &shared_refs, &shared_ids);
    // Before the flip Beads can still share draft ancestry explicitly.
    for item in &mut blocked {
        if let (Some(record), Some(reason)) = (item["record"].as_str(), item["reason"].as_str()) {
            if reason.contains("wh change --retire") {
                item["reason"] = json!(format!(
                    "{reason}; or, before switching, share that ancestry with wh push --include-ancestry {record}"
                ));
            }
        }
    }
    let mut collisions = Vec::new();
    for reference in package {
        if let Some(record) = state.get(&reference) {
            if !travels(record) {
                continue;
            }
            if shared
                .iter()
                .any(|item| item.id == record.id && item.revision == record.revision)
            {
                collisions.push(format!("{}@{}", record.id.as_str(), record.revision));
                continue;
            }
            entries.push(Entry {
                record: record.clone(),
                source: "private",
                lifecycle: lifecycle(record),
            });
        }
    }
    entries.sort_by(|left, right| {
        logical_stage(&left.record)
            .cmp(&logical_stage(&right.record))
            .then_with(|| left.record.id.cmp(&right.record.id))
            .then_with(|| left.record.revision.cmp(&right.record.revision))
    });
    // A private revision identical to a shared one is already excluded by
    // the package (it compares full references), so nothing here repeats.
    Ok(Export {
        entries,
        blocked,
        collisions,
    })
}

/// Where the export is written.
pub fn path(layout: &ProjectLayout) -> Result<PathBuf, String> {
    crate::memory::export::directory(layout).map(|directory| directory.join(RULEBOOK_FILE))
}

/// Write the export (private, outside the repository); the same records
/// write the same bytes.
pub fn write(layout: &ProjectLayout, export: &Export, repository: &str) -> Result<PathBuf, String> {
    let file = path(layout)?;
    let directory = file
        .parent()
        .ok_or("the export path has no directory")?
        .to_path_buf();
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("{}: {error}", directory.display()))?;
    }
    let value = export.file(repository).map_err(|error| error.to_string())?;
    let mut text = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
    text.push('\n');
    std::fs::write(&file, text).map_err(|error| format!("{}: {error}", file.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("{}: {error}", file.display()))?;
    }
    Ok(file)
}

/// Whether the export on disk is exactly what `export` would write now.
pub fn written(layout: &ProjectLayout, export: &Export, repository: &str) -> bool {
    let Ok(file) = path(layout) else {
        return false;
    };
    let on_disk = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    match (on_disk, export.file(repository)) {
        (Some(on_disk), Ok(now)) => on_disk == now,
        _ => false,
    }
}

/// What reading the export back from Records found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Confirmation {
    /// Exported revisions Records does not hold, as `id@revision`.
    pub missing: Vec<String>,
    /// Exported revisions Records holds with another digest.
    pub mismatched: Vec<String>,
    pub confirmed: usize,
}

impl Confirmation {
    pub fn complete(&self) -> bool {
        self.missing.is_empty() && self.mismatched.is_empty()
    }
}

/// Read every exported revision back from Records and compare digests.
pub fn confirm(project_root: &Path, export: &Export) -> Result<Confirmation, StorageError> {
    let virgil = VirgilStore::shared(project_root);
    let mut confirmation = Confirmation::default();
    for entry in &export.entries {
        let reference = entry.record.reference().map_err(StorageError::Domain)?;
        let label = format!("{}@{}", reference.id.as_str(), reference.revision);
        match virgil.fetch(&reference.id, reference.revision) {
            Ok(Some(found)) if found.reference == reference => confirmation.confirmed += 1,
            Ok(Some(_)) => confirmation.mismatched.push(label),
            Ok(None) => confirmation.missing.push(label),
            // A revision whose stored body does not verify did not arrive
            // intact: it is a mismatch, named, never repaired.
            Err(StorageError::MalformedRecord { .. }) => confirmation.mismatched.push(label),
            Err(error) => return Err(error),
        }
    }
    Ok(confirmation)
}

/// After the flip's read-back: the private Beads store's revisions (drafts,
/// receipts, accepted records) copied exactly into the private scope of the
/// repository's rulebook, so nothing local is left behind. Returns how many
/// were copied.
pub fn carry_private(layout: &ProjectLayout) -> Result<usize, StorageError> {
    if !crate::beads::is_initialized(&layout.private_store()) {
        return Ok(0);
    }
    let mut records =
        BeadsStore::open_existing(&layout.private_store(), StoreKind::Private)?.all_records()?;
    records.retain(|record| !matches!(record.body, RecordBody::Retired(_)));
    records.sort_by(|left, right| {
        logical_stage(left)
            .cmp(&logical_stage(right))
            .then_with(|| left.id.cmp(&right.id))
            .then_with(|| left.revision.cmp(&right.revision))
    });
    Ok(VirgilStore::private(layout.project_root())
        .receive(&records)?
        .len())
}

/// Records left in the store the switch no longer names, with their ids and
/// kinds: under Virgil, revisions still only in Beads (the shared database's
/// shareable records against the repository scope, the private Beads store
/// against the private scope); under Beads, revisions still only in Virgil
/// (a flip back). A store that cannot be read here (no `bd`, no daemon) is
/// skipped: this is a report, never a gate.
pub fn left_behind(layout: &ProjectLayout) -> Vec<Value> {
    use crate::storage::StoreConfig;
    let Ok(config) = crate::storage::read_config(layout.project_root()) else {
        return Vec::new();
    };
    let refs = |records: &[AgreementRecord]| {
        records
            .iter()
            .filter_map(|record| record.reference().ok())
            .collect::<BTreeSet<_>>()
    };
    let report = |records: Vec<AgreementRecord>, current: &BTreeSet<_>, store: &str| {
        records
            .into_iter()
            .filter(|record| !matches!(record.body, RecordBody::Retired(_)))
            .filter(|record| {
                record
                    .reference()
                    .is_ok_and(|reference| !current.contains(&reference))
            })
            .map(|record| {
                json!({
                    "record": record.id.as_str(),
                    "revision": record.revision,
                    "kind": record.body.type_name(),
                    "store": store,
                })
            })
            .collect::<Vec<_>>()
    };
    let beads_private = || {
        crate::beads::is_initialized(&layout.private_store())
            .then(|| {
                BeadsStore::open_existing(&layout.private_store(), StoreKind::Private)
                    .and_then(|store| store.all_records())
                    .ok()
            })
            .flatten()
    };
    let beads_shared = || {
        layout
            .shared_store()
            .and_then(|dir| BeadsStore::open_existing(&dir, StoreKind::Shareable).ok())
            .and_then(|store| store.all_records().ok())
    };
    let root = layout.project_root();
    let mut left = Vec::new();
    match config {
        StoreConfig::Virgil => {
            if let Some(beads) = beads_shared() {
                if let Ok(virgil) = VirgilStore::shared(root).all_records() {
                    let beads = beads.into_iter().filter(travels).collect();
                    left.extend(report(beads, &refs(&virgil), "beads"));
                }
            }
            if let Some(beads) = beads_private() {
                if let Ok(virgil) = VirgilStore::private(root).all_records() {
                    left.extend(report(beads, &refs(&virgil), "beads"));
                }
            }
        }
        StoreConfig::Beads => {
            if !layout.virgil_attached() {
                return left;
            }
            let mut current = refs(&beads_private().unwrap_or_default());
            current.extend(refs(&beads_shared().unwrap_or_default()));
            for store in [VirgilStore::shared(root), VirgilStore::private(root)] {
                if let Ok(virgil) = store.all_records() {
                    left.extend(report(virgil, &current, "virgil"));
                }
            }
        }
    }
    left
}
