use std::sync::{Arc, Barrier};
use std::{fs, process::Command};

use tempfile::TempDir;
use whetstone::domain::{
    AgreementRecord, ContentDigest, CoreValue, EvidenceRef, PrincipalKind, PrincipalRef,
    Provenance, ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, RecordRef, Scope,
    SCHEMA_VERSION_V1,
};
use whetstone::storage::{AppendRequest, CrashPoint, DoltRepository, StorageError, StoreKind};

fn integration_enabled() -> bool {
    std::env::var("WH_DOLT_INTEGRATION").as_deref() == Ok("1")
}

fn repository() -> Option<(TempDir, DoltRepository)> {
    if !integration_enabled() {
        eprintln!("skipped: set WH_DOLT_INTEGRATION=1 for pinned Dolt integration");
        return None;
    }
    let temp = TempDir::new().expect("temp");
    let repository = DoltRepository::initialize(&temp.path().join("private"), StoreKind::Private)
        .expect("private repository");
    Some((temp, repository))
}

#[test]
fn initialization_resumes_an_empty_dolt_repository_after_bootstrap_interruption() {
    if !integration_enabled() {
        eprintln!("skipped: set WH_DOLT_INTEGRATION=1 for pinned Dolt integration");
        return;
    }
    let temp = TempDir::new().expect("temp");
    let root = temp.path().join("private");
    fs::create_dir_all(&root).expect("create interrupted store root");
    let interrupted = Command::new("dolt")
        .current_dir(&root)
        .env("DOLT_DISABLE_EVENT_FLUSH", "1")
        .args([
            "init",
            "--name",
            "Whetstone",
            "--email",
            "local@whetstone.invalid",
            "--initial-branch",
            "main",
        ])
        .output()
        .expect("model interruption immediately after dolt init");
    assert!(
        interrupted.status.success(),
        "{}",
        String::from_utf8_lossy(&interrupted.stderr)
    );

    let repository = DoltRepository::initialize(&root, StoreKind::Private)
        .expect("resume the owned empty repository migration");
    assert!(repository.all_records().expect("records").is_empty());
    assert_eq!(repository.health().expect("health").schema_version, 1);
}

fn digest(seed: char) -> ContentDigest {
    ContentDigest::new(format!("sha256:{}", seed.to_string().repeat(64))).expect("digest")
}

fn record(
    id: &str,
    revision: u64,
    idempotency_key: &str,
    supersedes: Option<RecordRef>,
) -> AgreementRecord {
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "atomic-test-owner".into(),
        display_name: None,
    };
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id).expect("record ID"),
        revision,
        scope: Scope {
            organization: None,
            project: "atomic-test".into(),
            component: None,
            environment: None,
        },
        owner: principal.clone(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: principal,
            recorded_at: "2026-09-09T12:00:00Z".into(),
            sources: vec![EvidenceRef {
                system: "test".into(),
                locator: idempotency_key.into(),
                digest: Some(digest('a')),
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes,
        idempotency_key: idempotency_key.into(),
        body: RecordBody::CoreValue(CoreValue {
            name: id.into(),
            description: format!("revision {revision}"),
        }),
    }
}

#[test]
fn atomic_batch_appends_a_revision_and_companion_record_together() {
    let Some((_temp, repository)) = repository() else {
        return;
    };
    let initial = record("repair.session.fixture", 1, "session-1", None);
    let initial_ref = repository.append(&initial, None).expect("initial revision");
    let session = record("repair.session.fixture", 2, "session-2", Some(initial_ref));
    let handoff = record("repair.handoff.fixture.2", 1, "handoff-2", None);
    let requests = [
        AppendRequest {
            record: &session,
            expected_revision: Some(1),
        },
        AppendRequest {
            record: &handoff,
            expected_revision: None,
        },
    ];

    let references = repository.append_batch(&requests).expect("atomic append");

    assert_eq!(
        references,
        vec![
            session.reference().expect("session reference"),
            handoff.reference().expect("handoff reference")
        ]
    );
    assert_eq!(
        repository
            .latest(&session.id)
            .expect("session read")
            .expect("session exists"),
        session
    );
    assert_eq!(
        repository
            .latest(&handoff.id)
            .expect("handoff read")
            .expect("handoff exists"),
        handoff
    );
}

#[test]
fn exclusive_append_has_one_winner_across_repository_instances() {
    let Some((temp, repository)) = repository() else {
        return;
    };
    let root = repository.root().to_path_buf();
    drop(repository);
    let claim = record("repair.operation.exclusive", 1, "exclusive-claim", None);
    let barrier = Arc::new(Barrier::new(2));
    let handles = [(), ()].map(|()| {
        let root = root.clone();
        let claim = claim.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            let repository = DoltRepository::open_existing(&root, StoreKind::Private)
                .expect("open competing repository");
            barrier.wait();
            repository.append_exclusive(&claim, None)
        })
    });
    let results = handles.map(|handle| handle.join().expect("exclusive append thread"));
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(StorageError::ExclusiveRecordExists(_))))
            .count(),
        1,
        "the loser must observe the durable winner: {results:?}"
    );
    let repository = DoltRepository::open_existing(&root, StoreKind::Private)
        .expect("reopen exclusive repository");
    assert_eq!(repository.all_records().expect("records").len(), 1);
    drop(temp);
}

#[test]
fn atomic_batch_rejects_stale_input_without_any_append() {
    let Some((_temp, repository)) = repository() else {
        return;
    };
    let first = record("repair.session.stale", 1, "stale-session", None);
    let second = record("repair.handoff.stale", 1, "stale-handoff", None);
    let result = repository.append_batch(&[
        AppendRequest {
            record: &first,
            expected_revision: None,
        },
        AppendRequest {
            record: &second,
            expected_revision: Some(7),
        },
    ]);

    assert!(matches!(result, Err(StorageError::StaleRevision { .. })));
    assert!(repository.all_records().expect("records").is_empty());
}

#[test]
fn atomic_batch_exact_replay_is_idempotent_and_partial_replay_fails_closed() {
    let Some((_temp, repository)) = repository() else {
        return;
    };
    let first = record("repair.session.replay", 1, "replay-session", None);
    let second = record("repair.handoff.replay", 1, "replay-handoff", None);
    let requests = [
        AppendRequest {
            record: &first,
            expected_revision: None,
        },
        AppendRequest {
            record: &second,
            expected_revision: None,
        },
    ];

    let lost_receipt =
        repository.append_batch_with_crash(&requests, CrashPoint::AfterCommitBeforeReceipt);
    assert!(matches!(
        lost_receipt,
        Err(StorageError::InjectedCrash(
            CrashPoint::AfterCommitBeforeReceipt
        ))
    ));
    let replay = repository
        .append_batch(&requests)
        .expect("exact replay after a lost receipt");
    assert_eq!(
        replay,
        vec![
            first.reference().expect("first reference"),
            second.reference().expect("second reference")
        ]
    );
    assert_eq!(repository.all_records().expect("records").len(), 2);

    let third = record("repair.handoff.replay.2", 1, "replay-handoff-2", None);
    let mixed = repository.append_batch(&[
        AppendRequest {
            record: &first,
            expected_revision: None,
        },
        AppendRequest {
            record: &third,
            expected_revision: None,
        },
    ]);
    assert!(matches!(
        mixed,
        Err(StorageError::PartialBatchReplay {
            persisted: 1,
            total: 2
        })
    ));
    assert!(repository.latest(&third.id).expect("third read").is_none());
}

#[test]
fn atomic_batch_invalid_or_injected_second_step_never_leaves_the_first_record() {
    let Some((_temp, repository)) = repository() else {
        return;
    };
    let first = record("repair.session.rollback", 1, "rollback-session", None);
    let mut invalid = record("repair.handoff.invalid", 1, "rollback-invalid", None);
    invalid.idempotency_key.clear();
    let invalid_result = repository.append_batch(&[
        AppendRequest {
            record: &first,
            expected_revision: None,
        },
        AppendRequest {
            record: &invalid,
            expected_revision: None,
        },
    ]);
    assert!(matches!(invalid_result, Err(StorageError::Domain(_))));
    assert!(repository
        .all_records()
        .expect("invalid records")
        .is_empty());

    let second = record("repair.handoff.rollback", 1, "rollback-handoff", None);
    let injected = repository.append_batch_with_crash(
        &[
            AppendRequest {
                record: &first,
                expected_revision: None,
            },
            AppendRequest {
                record: &second,
                expected_revision: None,
            },
        ],
        CrashPoint::DuringBatchAfterFirstInsert,
    );
    assert!(matches!(injected, Err(StorageError::CommandFailed { .. })));
    assert!(
        repository
            .all_records()
            .expect("rollback records")
            .is_empty(),
        "the valid first insert must roll back when a later statement fails"
    );
}
