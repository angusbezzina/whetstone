//! The storage invariants every `RecordStore` adapter keeps, run against any
//! adapter through a [`Harness`]: append then get, latest and history across
//! revisions, idempotent repeats, an interrupted batch that resumes, the
//! lifecycle view following the kernel's resolution, copies from a private
//! into a shareable store (never the reverse), and a corrupted body reported
//! with where it lives, never repaired.
//!
//! Each adapter's test binary includes this module and calls
//! [`conformance_tests!`] (one test per invariant) with its harness;
//! `tests/record_store_conformance.rs` proves the suite catches a broken
//! adapter.

#![allow(dead_code)]

use whetstone::agreement::AgreementState;
use whetstone::domain::{
    AgreementRecord, AuthorizationAxis, ContentDigest, Enforcer, EvidenceRef, ExternalRef,
    ExternalSystem, Freshness, PolicyStateSnapshot, PrincipalKind, PrincipalRef, Provenance,
    ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, RecordRef, Rule, RuleSource, Scope,
    Strength, VerificationAxis, VerificationReceipt, RULE_SCHEMA_V2, SCHEMA_VERSION_V1,
};
use whetstone::storage::{AppendRequest, RecordStore, StorageError, StoreKind};

/// What a test needs from one adapter: a store, a second handle on it, a
/// private-to-shareable pair for copies, and a way to corrupt one stored
/// body (returning the locator the read error must name).
pub trait Harness {
    fn store(&self) -> &dyn RecordStore;
    fn reopen(&self) -> Box<dyn RecordStore>;
    /// `(private source, shareable destination)`, one of them this adapter.
    fn copy_pair(&self) -> (&dyn RecordStore, &dyn RecordStore);
    fn corrupt(&self, reference: &RecordRef) -> String;
}

fn owner() -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local:owner".into(),
        display_name: Some("Owner".into()),
    }
}

pub fn record(id: &str, revision: u64, key: &str, body: RecordBody) -> AgreementRecord {
    let receipt = body.is_receipt();
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id).expect("id"),
        revision,
        scope: Scope::project("fixture"),
        owner: owner(),
        provenance: Provenance {
            kind: if receipt {
                ProvenanceKind::DeterministicCheck
            } else {
                ProvenanceKind::HumanAuthored
            },
            recorded_by: owner(),
            recorded_at: "2026-10-17T12:00:00Z".into(),
            sources: vec![EvidenceRef {
                system: "fixture".into(),
                locator: "conformance".into(),
                digest: None,
            }],
            authority: if receipt {
                ProvenanceAuthority::CandidateOnly
            } else {
                ProvenanceAuthority::OwnerAuthored
            },
        },
        supersedes: None,
        idempotency_key: key.into(),
        body,
    }
}

pub fn rule(statement: &str) -> RecordBody {
    RecordBody::Rule(Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: statement.into(),
        rationale: "Evidence before assertion.".into(),
        strength: Strength::Must,
        enforcer: Enforcer::Test {
            command: "cargo test".into(),
        },
        examples: Vec::new(),
        source: RuleSource::owner(),
        paths: Vec::new(),
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
    })
}

pub fn receipt(subject: &RecordRef) -> RecordBody {
    RecordBody::VerificationReceipt(VerificationReceipt {
        subject: ExternalRef {
            system: ExternalSystem::Custom,
            stable_id: format!("gate:{}", subject.id.as_str()),
            revision: None,
        },
        code_digest: ContentDigest::new(format!("sha256:{}", "a".repeat(64))).expect("digest"),
        policy_state: PolicyStateSnapshot {
            accepted: Some(subject.clone()),
            required: None,
            installed: None,
            experimental: None,
        },
        verification: VerificationAxis::Fail,
        authorization: AuthorizationAxis::Unknown,
        freshness: Freshness::Fresh,
        checked_at: "2026-10-17T12:01:00Z".into(),
        related_records: vec![subject.clone()],
        evidence: Vec::new(),
    })
}

/// The next revision of `previous`, superseding it.
pub fn next(previous: &AgreementRecord, key: &str, statement: &str) -> AgreementRecord {
    let mut record = record(
        previous.id.as_str(),
        previous.revision + 1,
        key,
        rule(statement),
    );
    record.supersedes = Some(previous.reference().expect("reference"));
    record
}

fn reference(record: &AgreementRecord) -> RecordRef {
    record.reference().expect("reference")
}

pub fn append_then_get(harness: &dyn Harness) {
    let store = harness.store();
    let first = record(
        "rule.conformance-get",
        1,
        "conformance:get",
        rule("Get it back."),
    );
    let written = store.append(&first, None).expect("append");
    assert_eq!(written, reference(&first));
    assert_eq!(store.get(&written).expect("get").as_ref(), Some(&first));
    let reopened = harness.reopen();
    assert_eq!(
        reopened.get(&written).expect("reopen get").as_ref(),
        Some(&first)
    );
    assert!(reopened
        .all_records()
        .expect("all")
        .iter()
        .any(|record| record == &first));
    assert!(reopened.locator(&written).expect("locator").is_some());
}

pub fn latest_and_history(harness: &dyn Harness) {
    let store = harness.store();
    let first = record(
        "rule.conformance-latest",
        1,
        "conformance:latest:1",
        rule("One."),
    );
    store.append(&first, None).expect("first");
    let second = next(&first, "conformance:latest:2", "Two.");
    store.append(&second, Some(1)).expect("second");
    let id = first.id.clone();
    assert_eq!(store.latest(&id).expect("latest").as_ref(), Some(&second));
    assert_eq!(
        store.history(&id).expect("history"),
        vec![first.clone(), second.clone()]
    );
    let stale = next(&second, "conformance:latest:3", "Three.");
    assert!(matches!(
        store.append(&stale, Some(1)),
        Err(StorageError::StaleRevision {
            expected: Some(1),
            actual: Some(2)
        })
    ));
    let mut skipped = next(&second, "conformance:latest:5", "Five.");
    skipped.revision = 5;
    assert!(matches!(
        store.append(&skipped, Some(2)),
        Err(StorageError::IllegalRevision {
            expected: 3,
            actual: 5
        })
    ));
    assert_eq!(
        store.history(&id).expect("history").len(),
        2,
        "refusals write nothing"
    );
}

pub fn idempotent_repeat(harness: &dyn Harness) {
    let store = harness.store();
    let first = record("rule.conformance-key", 1, "conformance:key", rule("Once."));
    let written = store.append(&first, None).expect("append");
    assert_eq!(store.append(&first, None).expect("repeat"), written);
    assert_eq!(
        store
            .by_idempotency_key("conformance:key")
            .expect("by key")
            .as_ref(),
        Some(&first)
    );
    assert_eq!(store.history(&first.id).expect("history").len(), 1);
    let other = record(
        "rule.conformance-key-2",
        1,
        "conformance:key",
        rule("Other."),
    );
    assert!(matches!(
        store.append(&other, None),
        Err(StorageError::IdempotencyConflict(key)) if key == "conformance:key"
    ));
}

pub fn interrupted_batch_resumes(harness: &dyn Harness) {
    let store = harness.store();
    let a = record(
        "rule.conformance-batch-a",
        1,
        "conformance-batch:base-0:a",
        rule("A."),
    );
    let b = record(
        "rule.conformance-batch-b",
        1,
        "conformance-batch:base-0:b",
        rule("B."),
    );
    // The interruption: the first member landed, the marker never did.
    store.append(&a, None).expect("first member");
    let pending = store.incomplete_batches().expect("incomplete");
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert_eq!(pending[0].marker_key, "batch:conformance-batch");
    assert_eq!(
        pending[0].present,
        vec!["conformance-batch:base-0:a".to_string()]
    );
    let requests = [
        AppendRequest {
            record: &a,
            expected_revision: None,
        },
        AppendRequest {
            record: &b,
            expected_revision: None,
        },
    ];
    let written = store.append_batch(&requests).expect("resume");
    assert_eq!(written, vec![reference(&a), reference(&b)]);
    assert!(store.incomplete_batches().expect("incomplete").is_empty());
    assert_eq!(store.append_batch(&requests).expect("repeat"), written);
    assert_eq!(store.history(&a.id).expect("a").len(), 1);
    assert_eq!(store.history(&b.id).expect("b").len(), 1);
    assert!(matches!(
        store.append_batch(&[]),
        Err(StorageError::EmptyBatch)
    ));
}

pub fn lifecycle_follows_the_kernel(harness: &dyn Harness) {
    let store = harness.store();
    let first = record(
        "rule.conformance-life",
        1,
        "conformance:life:1",
        rule("Live."),
    );
    store.append(&first, None).expect("first");
    let check = record(
        "receipt.conformance-life",
        1,
        "conformance:life:receipt",
        receipt(&reference(&first)),
    );
    store.append(&check, None).expect("receipt");
    assert_eq!(
        store
            .lifecycle_view(&reference(&first))
            .expect("view")
            .as_deref(),
        Some("pending")
    );
    assert_eq!(
        store
            .lifecycle_view(&reference(&check))
            .expect("view")
            .as_deref(),
        Some("operational")
    );
    let second = next(&first, "conformance:life:2", "Live longer.");
    store.append(&second, Some(1)).expect("second");
    let state = AgreementState::from_records(store.all_records().expect("all"));
    store.sync_lifecycle_labels(&state).expect("sync");
    for record in [&first, &second, &check] {
        assert_eq!(
            store
                .lifecycle_view(&reference(record))
                .expect("view")
                .as_deref(),
            Some(whetstone::storage::resolved_lifecycle(&state, record)),
            "{}@{}",
            record.id.as_str(),
            record.revision
        );
    }
    assert_eq!(store.sync_lifecycle_labels(&state).expect("again"), 0);
}

pub fn copies_cross_from_private_to_shareable(harness: &dyn Harness) {
    let (private, shareable) = harness.copy_pair();
    assert_eq!(private.kind(), StoreKind::Private);
    assert_eq!(shareable.kind(), StoreKind::Shareable);
    let first = record(
        "rule.conformance-copy",
        1,
        "conformance:copy",
        rule("Share me."),
    );
    private.append(&first, None).expect("first");
    let check = record(
        "receipt.conformance-copy",
        1,
        "conformance:copy:receipt",
        receipt(&reference(&first)),
    );
    private.append(&check, None).expect("receipt");
    let copied = private
        .copy_to(shareable, &[reference(&first)], &[])
        .expect("copy");
    assert_eq!(copied, vec![reference(&first)]);
    assert_eq!(
        shareable.get(&reference(&first)).expect("get").as_ref(),
        Some(&first)
    );
    assert!(private
        .copy_to(shareable, &[reference(&first)], &[])
        .expect("repeat")
        .is_empty());
    assert!(matches!(
        private.copy_to(shareable, &[reference(&check)], &[]),
        Err(StorageError::InvalidProjectionBoundary)
    ));
    assert!(matches!(
        shareable.copy_to(private, &[reference(&first)], &[]),
        Err(StorageError::InvalidProjectionBoundary)
    ));
    let secret = record(
        "rule.conformance-secret",
        1,
        "conformance:secret",
        rule("Never mention CANARY-42."),
    );
    private.append(&secret, None).expect("secret");
    assert!(matches!(
        private.copy_to(shareable, &[reference(&secret)], &["CANARY-42"]),
        Err(StorageError::PrivateCanaryFound(_))
    ));
    assert!(shareable.get(&reference(&secret)).expect("get").is_none());
}

pub fn corruption_is_reported_never_repaired(harness: &dyn Harness) {
    let store = harness.store();
    let first = record(
        "rule.conformance-tamper",
        1,
        "conformance:tamper",
        rule("Intact."),
    );
    let written = store.append(&first, None).expect("append");
    let locator = harness.corrupt(&written);
    for _ in 0..2 {
        let error = harness.reopen().all_records().expect_err("corrupt read");
        assert!(
            matches!(
                error,
                StorageError::MalformedBead { .. } | StorageError::MalformedRecord { .. }
            ),
            "{error:?}"
        );
        assert!(
            error.to_string().contains(&locator),
            "{error} names {locator}"
        );
    }
}

/// Every invariant in turn, against harnesses `factory` builds.
pub fn run_conformance(factory: &dyn Fn() -> Box<dyn Harness>) {
    append_then_get(&*factory());
    latest_and_history(&*factory());
    idempotent_repeat(&*factory());
    interrupted_batch_resumes(&*factory());
    lifecycle_follows_the_kernel(&*factory());
    copies_cross_from_private_to_shareable(&*factory());
    corruption_is_reported_never_repaired(&*factory());
}

/// One `#[test]` per invariant, each with a fresh harness from `$factory`.
#[macro_export]
macro_rules! conformance_tests {
    ($factory:expr) => {
        #[test]
        fn conformance_append_then_get() {
            conformance::append_then_get(&*$factory());
        }
        #[test]
        fn conformance_latest_and_history() {
            conformance::latest_and_history(&*$factory());
        }
        #[test]
        fn conformance_idempotent_repeat() {
            conformance::idempotent_repeat(&*$factory());
        }
        #[test]
        fn conformance_interrupted_batch_resumes() {
            conformance::interrupted_batch_resumes(&*$factory());
        }
        #[test]
        fn conformance_lifecycle_follows_the_kernel() {
            conformance::lifecycle_follows_the_kernel(&*$factory());
        }
        #[test]
        fn conformance_copies_cross_from_private_to_shareable() {
            conformance::copies_cross_from_private_to_shareable(&*$factory());
        }
        #[test]
        fn conformance_corruption_is_reported_never_repaired() {
            conformance::corruption_is_reported_never_repaired(&*$factory());
        }
    };
}
