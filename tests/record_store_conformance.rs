//! The record store conformance suite's positive control: an in-memory
//! adapter that keeps every invariant passes it, and the same adapter with
//! its append checks removed fails it. The real adapters run the suite in
//! tests/beads_store.rs and tests/virgil_store.rs.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use whetstone::agreement::AgreementState;
use whetstone::domain::{AgreementRecord, RecordRef};
use whetstone::storage::{
    batch_marker_key, check_append, check_batch, initial_lifecycle, pending_batches,
    reference_text, resolved_lifecycle, validate_for_append, AppendRequest, IncompleteBatch,
    RecordStore, StorageError, StoreBackend, StoreKind,
};

mod conformance;

/// A store in memory. With `broken`, appends skip every check.
#[derive(Debug)]
struct MemoryStore {
    name: String,
    kind: StoreKind,
    broken: bool,
    records: RefCell<Vec<AgreementRecord>>,
    markers: RefCell<BTreeSet<String>>,
    lifecycle: RefCell<BTreeMap<String, String>>,
    corrupt: RefCell<Option<RecordRef>>,
}

impl MemoryStore {
    fn new(name: &str, kind: StoreKind, broken: bool) -> Self {
        Self {
            name: name.into(),
            kind,
            broken,
            records: RefCell::default(),
            markers: RefCell::default(),
            lifecycle: RefCell::default(),
            corrupt: RefCell::default(),
        }
    }
}

impl RecordStore for MemoryStore {
    fn backend(&self) -> StoreBackend {
        // A test double; nothing reads the backend of a store under test.
        StoreBackend::Beads
    }

    fn kind(&self) -> StoreKind {
        self.kind
    }

    fn location(&self) -> String {
        format!("memory:{}", self.name)
    }

    fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        if let Some(reference) = self.corrupt.borrow().as_ref() {
            return Err(StorageError::MalformedRecord {
                locator: format!("memory:{}", reference_text(reference)),
                reason: "corrupted".into(),
            });
        }
        let mut records = self.records.borrow().clone();
        records.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.revision.cmp(&right.revision))
        });
        Ok(records)
    }

    fn locator(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        Ok(self
            .get(reference)?
            .map(|_| format!("memory:{}", reference_text(reference))))
    }

    fn incomplete_batches(&self) -> Result<Vec<IncompleteBatch>, StorageError> {
        Ok(pending_batches(
            &self.all_records()?,
            &self.markers.borrow(),
        ))
    }

    fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        let reference = validate_for_append(record)?;
        if !self.broken {
            if let Some(already) =
                check_append(&self.all_records()?, record, &reference, expected_revision)?
            {
                return Ok(already);
            }
        }
        self.records.borrow_mut().push(record.clone());
        Ok(reference)
    }

    fn append_batch(&self, requests: &[AppendRequest<'_>]) -> Result<Vec<RecordRef>, StorageError> {
        if requests.is_empty() {
            return Err(StorageError::EmptyBatch);
        }
        if !self.broken {
            check_batch(&self.all_records()?, requests)?;
        }
        let mut references = Vec::new();
        for request in requests {
            references.push(self.append(request.record, request.expected_revision)?);
        }
        self.markers.borrow_mut().insert(batch_marker_key(requests));
        Ok(references)
    }

    fn sync_lifecycle_labels(&self, state: &AgreementState) -> Result<usize, StorageError> {
        let mut changed = 0;
        for record in self.all_records()? {
            let key = reference_text(&record.reference().map_err(StorageError::Domain)?);
            let wanted = resolved_lifecycle(state, &record).to_string();
            let current = self
                .lifecycle
                .borrow()
                .get(&key)
                .cloned()
                .unwrap_or_else(|| initial_lifecycle(&record).to_string());
            if current != wanted {
                self.lifecycle.borrow_mut().insert(key, wanted);
                changed += 1;
            }
        }
        Ok(changed)
    }

    fn lifecycle_view(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        Ok(self.get(reference)?.map(|record| {
            self.lifecycle
                .borrow()
                .get(&reference_text(reference))
                .cloned()
                .unwrap_or_else(|| initial_lifecycle(&record).to_string())
        }))
    }

    fn receive(&self, records: &[AgreementRecord]) -> Result<Vec<RecordRef>, StorageError> {
        let mut copied = Vec::new();
        for record in records {
            let reference = record.reference().map_err(StorageError::Domain)?;
            if self.get(&reference)?.is_some() {
                continue;
            }
            self.records.borrow_mut().push(record.clone());
            copied.push(reference);
        }
        Ok(copied)
    }
}

/// A shared handle, so "reopen" sees the same records.
#[derive(Debug, Clone)]
struct Shared(std::rc::Rc<MemoryStore>);

impl RecordStore for Shared {
    fn backend(&self) -> StoreBackend {
        self.0.backend()
    }
    fn kind(&self) -> StoreKind {
        self.0.kind()
    }
    fn location(&self) -> String {
        self.0.location()
    }
    fn all_records(&self) -> Result<Vec<AgreementRecord>, StorageError> {
        self.0.all_records()
    }
    fn locator(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        self.0.locator(reference)
    }
    fn incomplete_batches(&self) -> Result<Vec<IncompleteBatch>, StorageError> {
        self.0.incomplete_batches()
    }
    fn append(
        &self,
        record: &AgreementRecord,
        expected_revision: Option<u64>,
    ) -> Result<RecordRef, StorageError> {
        self.0.append(record, expected_revision)
    }
    fn append_batch(&self, requests: &[AppendRequest<'_>]) -> Result<Vec<RecordRef>, StorageError> {
        self.0.append_batch(requests)
    }
    fn sync_lifecycle_labels(&self, state: &AgreementState) -> Result<usize, StorageError> {
        self.0.sync_lifecycle_labels(state)
    }
    fn lifecycle_view(&self, reference: &RecordRef) -> Result<Option<String>, StorageError> {
        self.0.lifecycle_view(reference)
    }
    fn receive(&self, records: &[AgreementRecord]) -> Result<Vec<RecordRef>, StorageError> {
        self.0.receive(records)
    }
}

struct MemoryHarness {
    store: Shared,
    shareable: MemoryStore,
}

impl MemoryHarness {
    fn boxed(broken: bool) -> Box<dyn conformance::Harness> {
        Box::new(Self {
            store: Shared(std::rc::Rc::new(MemoryStore::new(
                "private",
                StoreKind::Private,
                broken,
            ))),
            shareable: MemoryStore::new("shareable", StoreKind::Shareable, broken),
        })
    }
}

impl conformance::Harness for MemoryHarness {
    fn store(&self) -> &dyn RecordStore {
        &self.store
    }
    fn reopen(&self) -> Box<dyn RecordStore> {
        Box::new(self.store.clone())
    }
    fn copy_pair(&self) -> (&dyn RecordStore, &dyn RecordStore) {
        (&self.store, &self.shareable)
    }
    fn corrupt(&self, reference: &RecordRef) -> String {
        *self.store.0.corrupt.borrow_mut() = Some(reference.clone());
        format!("memory:{}", reference_text(reference))
    }
}

mod memory {
    use super::*;
    conformance_tests!(|| MemoryHarness::boxed(false));
}

#[test]
fn the_suite_runs_end_to_end_over_a_store_that_keeps_every_invariant() {
    conformance::run_conformance(&|| MemoryHarness::boxed(false));
}

#[test]
fn a_store_that_skips_its_append_checks_fails_the_suite() {
    type Check = fn(&dyn conformance::Harness);
    let checks: [(&str, Check); 3] = [
        ("idempotent_repeat", conformance::idempotent_repeat),
        ("latest_and_history", conformance::latest_and_history),
        (
            "interrupted_batch_resumes",
            conformance::interrupted_batch_resumes,
        ),
    ];
    for (name, check) in checks {
        let harness = MemoryHarness::boxed(true);
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(&*harness)));
        assert!(outcome.is_err(), "the broken store passed {name}");
    }
    // The positive control's control: the same adapter with its checks
    // passes every invariant.
    conformance::run_conformance(&|| MemoryHarness::boxed(false));
}
