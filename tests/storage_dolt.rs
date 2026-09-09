use std::fs;
use std::process::Command;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::TempDir;
use whetstone::domain::{
    AgreementRecord, ContentDigest, CoreValue, EvidenceRef, PrincipalKind, PrincipalRef,
    Provenance, ProvenanceAuthority, ProvenanceKind, RecordBody, RecordId, Scope,
    SCHEMA_VERSION_V1,
};
use whetstone::storage::{
    CrashPoint, DoltRepository, DoltServer, ProjectLayout, ProjectionInspection, StorageError,
    StoreKind, SUPPORTED_DOLT_VERSION,
};

fn integration_enabled() -> bool {
    std::env::var("WH_DOLT_INTEGRATION").as_deref() == Ok("1")
}

fn command(cwd: &std::path::Path, program: &str, args: &[&str]) {
    let output = Command::new(program)
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("run fixture command");
    assert!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn digest(seed: char) -> ContentDigest {
    ContentDigest::new(format!("sha256:{}", seed.to_string().repeat(64))).expect("digest")
}

fn record(name: &str, request: &str, description: &str) -> AgreementRecord {
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local-owner".into(),
        display_name: None,
    };
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(name).expect("record id"),
        revision: 1,
        scope: Scope {
            organization: None,
            project: "fixture".into(),
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
                locator: request.into(),
                digest: Some(digest('a')),
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: request.into(),
        body: RecordBody::CoreValue(CoreValue {
            name: name.into(),
            description: description.into(),
        }),
    }
}

#[test]
fn nested_directories_and_worktrees_share_one_project_identity() {
    let temp = TempDir::new().expect("temp");
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).expect("repo");
    command(&repo, "git", &["init", "-q"]);
    command(&repo, "git", &["config", "user.name", "Whetstone Test"]);
    command(
        &repo,
        "git",
        &["config", "user.email", "test@whetstone.invalid"],
    );
    fs::write(repo.join("README.md"), "fixture").expect("readme");
    command(&repo, "git", &["add", "README.md"]);
    command(&repo, "git", &["commit", "-qm", "fixture"]);
    let nested = repo.join("a/b/c");
    fs::create_dir_all(&nested).expect("nested");
    let primary = ProjectLayout::resolve(&nested, None).expect("primary layout");

    let worktree = temp.path().join("worktree");
    command(
        &repo,
        "git",
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "fixture-worktree",
            worktree.to_str().expect("worktree path"),
        ],
    );
    let secondary = ProjectLayout::resolve(&worktree, None).expect("worktree layout");
    assert_eq!(primary.project_id(), secondary.project_id());
    assert_eq!(primary.state_root(), secondary.state_root());
    assert_ne!(primary.project_root(), secondary.project_root());
    assert!(ProjectLayout::resolve(&nested, Some("../escape")).is_err());
    assert_eq!(
        fs::read_to_string(repo.join("README.md")).expect("readme"),
        "fixture"
    );
    let status = Command::new("git")
        .current_dir(&repo)
        .args(["status", "--porcelain"])
        .output()
        .expect("git status");
    assert!(status.status.success());
    assert!(
        status.stdout.is_empty(),
        "layout resolution must not modify the repo"
    );
}

#[test]
fn real_dolt_storage_projection_recovery_concurrency_and_lifecycle() {
    if !integration_enabled() {
        eprintln!("skipped: set WH_DOLT_INTEGRATION=1 for pinned Dolt integration");
        return;
    }
    let version = Command::new("dolt")
        .arg("version")
        .output()
        .expect("dolt installed");
    assert!(String::from_utf8_lossy(&version.stdout).contains(SUPPORTED_DOLT_VERSION));

    let temp = TempDir::new().expect("temp");
    let data = temp.path().join("data");
    fs::create_dir(&data).expect("data");
    let private = DoltRepository::initialize(&data.join("private"), StoreKind::Private)
        .expect("private store");
    let share = DoltRepository::initialize(&data.join("shareable"), StoreKind::Shareable)
        .expect("share store");
    assert_ne!(private.root(), share.root());

    let first = record("value.first", "request-first", "Keep evidence honest");
    let before_commit = private.append_with_crash(&first, None, CrashPoint::BeforeCommit);
    assert!(
        matches!(
            before_commit,
            Err(StorageError::InjectedCrash(CrashPoint::BeforeCommit))
        ),
        "unexpected pre-commit result: {before_commit:?}"
    );
    assert!(private.latest(&first.id).expect("read").is_none());
    assert!(matches!(
        private.append_with_crash(&first, None, CrashPoint::AfterCommitBeforeReceipt),
        Err(StorageError::InjectedCrash(
            CrashPoint::AfterCommitBeforeReceipt
        ))
    ));
    let first_ref = private.append(&first, None).expect("idempotent recovery");
    assert_eq!(private.history(&first.id).expect("history").len(), 1);
    assert!(matches!(
        private.append(&record("value.stale", "request-stale", "stale"), Some(9)),
        Err(StorageError::StaleRevision { .. })
    ));

    let server_started = Instant::now();
    let mut server = DoltServer::start(&data, "private").expect("server starts");
    assert!(server.is_ready().expect("server ready"));
    let server_start_elapsed = server_started.elapsed();
    assert!(server_start_elapsed < Duration::from_secs(3));

    let writers = 12;
    let barrier = Arc::new(Barrier::new(writers));
    let mut handles = Vec::new();
    for index in 0..writers {
        let root = private.root().to_path_buf();
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let session =
                DoltRepository::initialize(&root, StoreKind::Private).expect("writer session");
            let item = record(
                &format!("value.concurrent-{index}"),
                &format!("request-concurrent-{index}"),
                "parallel",
            );
            barrier.wait();
            session.append(&item, None)
        }));
    }
    for handle in handles {
        handle
            .join()
            .expect("writer thread")
            .expect("writer append");
    }
    assert_eq!(
        private.all_records().expect("all records").len(),
        writers + 1
    );
    let port = server.port();
    let shutdown_started = Instant::now();
    server.shutdown().expect("owned shutdown");
    let shutdown_elapsed = shutdown_started.elapsed();
    assert!(shutdown_elapsed < Duration::from_secs(2));
    eprintln!(
        "dolt_server_start_ms={} shutdown_ms={}",
        server_start_elapsed.as_millis(),
        shutdown_elapsed.as_millis()
    );
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());

    let private_only = record(
        "value.private",
        "request-private",
        "WH_PRIVATE_CANARY must remain private",
    );
    let private_only_ref = private.append(&private_only, None).expect("private canary");
    assert!(matches!(
        private.project_selected(&share, &[private_only_ref], &["WH_PRIVATE_CANARY"]),
        Err(StorageError::PrivateCanaryFound(_))
    ));
    let projection = private
        .project_selected(
            &share,
            std::slice::from_ref(&first_ref),
            &["WH_PRIVATE_CANARY"],
        )
        .expect("safe projection");
    assert_eq!(projection.records, vec![first_ref.clone()]);
    assert_eq!(share.all_records().expect("shared records").len(), 1);

    let projections = temp.path().join("projections");
    let rendered = private
        .render_projection(&projections, std::slice::from_ref(&first_ref))
        .expect("render");
    assert!(rendered.generation_root.join("agreement.md").is_file());
    assert!(matches!(
        DoltRepository::inspect_projection(&projections).expect("inspect"),
        ProjectionInspection::Verified { .. }
    ));
    let current_before = fs::read_to_string(projections.join("current")).expect("current");
    let second = record("value.second", "request-second", "Second value");
    let second_ref = private.append(&second, None).expect("second");
    assert!(matches!(
        private.render_projection_with_crash(
            &projections,
            &[first_ref.clone(), second_ref],
            CrashPoint::AfterProjectionGeneration,
        ),
        Err(StorageError::InjectedCrash(
            CrashPoint::AfterProjectionGeneration
        ))
    ));
    assert_eq!(
        fs::read_to_string(projections.join("current")).expect("current after crash"),
        current_before
    );

    let json_path = rendered.generation_root.join("agreement.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&json_path).expect("projection json")).expect("json");
    json["payload"]["records"][0]["record"]["description"] = serde_json::json!("tampered");
    fs::write(
        &json_path,
        serde_json::to_vec_pretty(&json).expect("serialize tamper"),
    )
    .expect("tamper fixture");
    assert!(matches!(
        DoltRepository::inspect_projection(&projections).expect("inspect tamper"),
        ProjectionInspection::TamperedDraft { .. }
    ));

    let logical_archive = temp.path().join("private-logical.json");
    let logical_receipt = private
        .export_logical(&logical_archive)
        .expect("logical export");
    let (logical_restore, import_receipt) = DoltRepository::import_logical(
        &logical_archive,
        &temp.path().join("logical-restore"),
        StoreKind::Private,
    )
    .expect("logical import");
    assert_eq!(
        logical_receipt.payload_digest,
        import_receipt.payload_digest
    );
    assert_eq!(
        logical_restore.all_records().expect("logical records"),
        private.all_records().expect("source logical records")
    );
    assert!(matches!(
        DoltRepository::import_logical(
            &logical_archive,
            &temp.path().join("wrong-kind"),
            StoreKind::Shareable,
        ),
        Err(StorageError::LogicalArchiveKindMismatch)
    ));
    let mut tampered_archive: serde_json::Value =
        serde_json::from_slice(&fs::read(&logical_archive).expect("logical archive bytes"))
            .expect("logical archive json");
    tampered_archive["payload"]["records"][0]["record"]["description"] =
        serde_json::json!("tampered logical content");
    let tampered_path = temp.path().join("tampered-logical.json");
    fs::write(
        &tampered_path,
        serde_json::to_vec_pretty(&tampered_archive).expect("tampered archive bytes"),
    )
    .expect("write tampered archive");
    let tampered_destination = temp.path().join("tampered-restore");
    assert!(matches!(
        DoltRepository::import_logical(&tampered_path, &tampered_destination, StoreKind::Private,),
        Err(StorageError::LogicalArchiveDigestMismatch)
    ));
    assert!(!tampered_destination.exists());

    let backup = temp.path().join("backup");
    let before_backup = private.all_records().expect("before backup");
    let receipt = private.backup_to(&backup).expect("backup");
    assert_eq!(receipt.kind, StoreKind::Private);
    let restored =
        DoltRepository::restore_from(&backup, &temp.path().join("restored"), StoreKind::Private)
            .expect("restore");
    assert_eq!(
        restored.all_records().expect("restored records"),
        before_backup
    );
    assert_eq!(
        restored.history(&first.id).expect("restored history"),
        private.history(&first.id).expect("source history")
    );
    restored.health().expect("restored health");

    let records_before_migration_error = private.all_records().expect("pre-error records");
    command(
        private.root(),
        "dolt",
        &[
            "sql",
            "-q",
            "SET @@dolt_transaction_commit=1; START TRANSACTION; UPDATE whetstone_migrations SET checksum='sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff' WHERE version=1; COMMIT;",
        ],
    );
    assert!(matches!(
        DoltRepository::initialize(private.root(), StoreKind::Private),
        Err(StorageError::MigrationChecksumMismatch(1))
    ));
    assert_eq!(
        private
            .all_records()
            .expect("records survive migration error"),
        records_before_migration_error
    );
}
