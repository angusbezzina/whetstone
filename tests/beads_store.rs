//! The Beads adapter against throwaway `bd init` databases on the pinned
//! minimum bd version: exact round trips, tamper reporting, append
//! semantics, resumable batches, recovery, and a private store that can never
//! inherit the team's remote.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;
use whetstone::agreement::AgreementState;
use whetstone::beads::{ensure_bd_version, RecordStore, MINIMUM_BD_VERSION};
use whetstone::domain::{
    AgreementRecord, CoreValue, EvidenceRef, LocalReview, LocalReviewVerdict, PrincipalKind,
    PrincipalRef, Proposal, ProposalState, Provenance, ProvenanceAuthority, ProvenanceKind,
    RecordBody, RecordId, Scope, LOCAL_REVIEW_ASSURANCE, SCHEMA_VERSION_V1,
};
use whetstone::storage::{AppendRequest, StorageError, StoreKind};

fn owner() -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local:owner".into(),
        display_name: Some("Owner".into()),
    }
}

fn record(id: &str, revision: u64, key: &str, body: RecordBody) -> AgreementRecord {
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: RecordId::new(id).expect("id"),
        revision,
        scope: Scope {
            organization: None,
            project: "fixture".into(),
            component: None,
            environment: None,
        },
        owner: owner(),
        provenance: Provenance {
            kind: ProvenanceKind::HumanAuthored,
            recorded_by: owner(),
            recorded_at: "2026-09-10T12:00:00Z".into(),
            sources: vec![EvidenceRef {
                system: "fixture".into(),
                locator: "test".into(),
                digest: None,
            }],
            authority: ProvenanceAuthority::OwnerAuthored,
        },
        supersedes: None,
        idempotency_key: key.into(),
        body,
    }
}

fn value(text: &str) -> RecordBody {
    RecordBody::CoreValue(CoreValue {
        name: "Core values".into(),
        description: text.into(),
    })
}

fn bd(dir: &Path, args: &[&str]) -> Value {
    let output = Command::new("bd")
        .args(args)
        .current_dir(dir)
        .env("BEADS_DIR", dir.join(".beads"))
        .env("GIT_CEILING_DIRECTORIES", dir.parent().expect("parent"))
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect("bd");
    assert!(
        output.status.success(),
        "bd {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let start = text.find(['[', '{']).unwrap_or(0);
    serde_json::from_str(&text[start..]).unwrap_or(Value::Null)
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

#[test]
fn records_round_trip_byte_for_byte_and_tampering_names_the_bead() {
    let version = ensure_bd_version().expect("bd is installed at the pinned minimum");
    assert!(!version.is_empty());
    assert!(MINIMUM_BD_VERSION >= (1, 1, 2));
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");

    // Characters that break Beads' metadata column when stored raw.
    let tricky = record(
        "value.core",
        1,
        "init:values",
        value("Line\u{2028}separator, \"quotes\", back\\slash, é, 😀 and </script>"),
    );
    let reference = store.append(&tricky, None).expect("append");
    assert_eq!(store.append(&tricky, None).expect("replay"), reference);

    let fresh = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    let stored = fresh.get(&reference).expect("get").expect("present");
    assert_eq!(
        stored.canonical_json().expect("canonical"),
        tricky.canonical_json().expect("canonical"),
        "canonical bytes survive the round trip"
    );
    assert_eq!(stored.digest().expect("digest"), reference.digest);

    // The raw bead holds ASCII only, and every bd read still works.
    let bead = fresh.bead_for(&reference).expect("bead").expect("bead id");
    let shown = bd(&dir, &["show", &bead, "--json"]);
    let shown = shown
        .as_array()
        .and_then(|items| items.first())
        .unwrap_or(&shown);
    let raw = shown["metadata"]["wh_record"].as_str().expect("wh_record");
    assert!(raw.is_ascii());
    assert!(raw.contains("\\u2028"));
    assert_eq!(shown["issue_type"], "record");
    assert!(shown["labels"]
        .as_array()
        .expect("labels")
        .iter()
        .any(|label| label == "whetstone"));

    // A hand edit of the metadata is reported with the bead id, not repaired.
    let edited = raw.replace("separator", "SEPARATOR");
    bd(
        &dir,
        &[
            "update",
            &bead,
            "--set-metadata",
            &format!("wh_record={edited}"),
        ],
    );
    let tampered = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    match tampered.all_records() {
        Err(StorageError::MalformedBead {
            bead: named,
            reason,
        }) => {
            assert_eq!(named, bead);
            assert!(
                reason.contains("digest") || reason.contains("disagree"),
                "{reason}"
            );
        }
        other => panic!("tampering was not reported: {other:?}"),
    }
}

#[test]
fn appends_are_idempotent_revisioned_exclusive_and_batches_resume() {
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    let v1 = record("value.core", 1, "v1", value("First"));
    let v1_ref = store.append(&v1, None).expect("v1");
    // Same key, different content: conflict.
    assert!(matches!(
        store.append(&record("value.core", 1, "v1", value("Other")), None),
        Err(StorageError::IdempotencyConflict(_))
    ));
    // Stale and illegal revisions fail closed.
    let mut v2 = record("value.core", 2, "v2", value("Second"));
    v2.supersedes = Some(v1_ref.clone());
    assert!(matches!(
        store.append(&v2, None),
        Err(StorageError::StaleRevision { .. })
    ));
    let mut v3 = record("value.core", 3, "v3", value("Skip"));
    v3.supersedes = Some(v1_ref.clone());
    assert!(matches!(
        store.append(&v3, Some(1)),
        Err(StorageError::IllegalRevision { .. })
    ));
    store.append(&v2, Some(1)).expect("v2");
    // Exclusive: a replay is never ownership.
    let claim = record("value.claim", 1, "claim", value("Mine"));
    store.append_exclusive(&claim, None).expect("claim");
    assert!(matches!(
        store.append_exclusive(&claim, None),
        Err(StorageError::ExclusiveRecordExists(_))
    ));

    // A batch interrupted after its first record resumes on replay and then
    // writes its completion marker.
    let a = record("mission.project", 1, "init-9:base-0:mission", value("A"));
    let b = record("value.batch", 1, "init-9:base-0:values", value("B"));
    store.append(&a, None).expect("first half");
    let partial = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    let pending = partial.incomplete_batches().expect("batches");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].marker_key, "batch:init-9");
    let references = partial
        .append_batch(&[
            AppendRequest {
                record: &a,
                expected_revision: None,
            },
            AppendRequest {
                record: &b,
                expected_revision: None,
            },
        ])
        .expect("resumed batch");
    assert_eq!(references.len(), 2);
    let settled = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    assert!(settled.incomplete_batches().expect("batches").is_empty());
    assert_eq!(settled.all_records().expect("records").len(), 5);
}

#[test]
fn lifecycle_labels_follow_the_kernel_and_archives_restore_exactly() {
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    let mut candidate = record("value.speed", 1, "change-1", value("Fast feedback"));
    candidate.provenance.authority = ProvenanceAuthority::OwnerAuthored;
    let candidate_ref = store.append(&candidate, None).expect("candidate");
    let proposal = record(
        "proposal.change-1",
        1,
        "change-1:proposal",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Core value proposed: Fast feedback".into(),
            rationale: "Rationale: speed".into(),
            proposed_records: vec![candidate_ref.clone()],
            binding: None,
        }),
    );
    let proposal_ref = store.append(&proposal, None).expect("proposal");
    let state = AgreementState::from_records(store.all_records().expect("records"));
    store.sync_lifecycle_labels(&state).expect("labels");
    let bead = store.bead_for(&candidate_ref).expect("bead").expect("id");
    let labels = |dir: &Path| {
        let shown = bd(dir, &["show", &bead, "--json"]);
        let shown = shown
            .as_array()
            .and_then(|items| items.first())
            .cloned()
            .unwrap_or(shown);
        shown["labels"].clone()
    };
    assert!(labels(&dir).to_string().contains("wh:lifecycle:draft"));
    let review = record(
        "review.change-1",
        1,
        "review:change-1",
        RecordBody::LocalReview(LocalReview {
            proposal: proposal_ref,
            verdict: LocalReviewVerdict::Accept,
            reviewer: owner(),
            assurance: LOCAL_REVIEW_ASSURANCE.into(),
            rationale: "Explicit solo acceptance by the owner.".into(),
            reviewed_at: "2026-09-10T12:05:00Z".into(),
            team_activation_permitted: false,
        }),
    );
    store.append(&review, None).expect("review");
    let state = AgreementState::from_records(store.all_records().expect("records"));
    store.sync_lifecycle_labels(&state).expect("labels");
    let now = labels(&dir).to_string();
    assert!(
        now.contains("wh:lifecycle:accepted") && !now.contains("wh:lifecycle:draft"),
        "{now}"
    );

    let archive = temp.path().join("archive.json");
    let exported = store.export_logical(&archive).expect("export");
    assert_eq!(exported.records, 3);
    let (restored, receipt) =
        RecordStore::import_logical(&archive, &temp.path().join("restored"), StoreKind::Private)
            .expect("import");
    assert_eq!(receipt.payload_digest, exported.payload_digest);
    let mut before = store.all_records().expect("records");
    let mut after = restored.all_records().expect("records");
    before.sort_by(|l, r| l.id.cmp(&r.id));
    after.sort_by(|l, r| l.id.cmp(&r.id));
    assert_eq!(before, after);
    // A tampered archive is refused.
    let text = fs::read_to_string(&archive).expect("archive");
    fs::write(&archive, text.replace("Fast feedback", "Slow feedback")).expect("tamper");
    assert!(matches!(
        RecordStore::import_logical(&archive, &temp.path().join("again"), StoreKind::Private),
        Err(StorageError::LogicalArchiveDigestMismatch)
    ));
}

#[test]
fn the_private_store_never_inherits_the_team_remote_or_its_data() {
    // A repository whose origin carries the team's Beads data: a naive
    // `bd init` inside it clones that data and wires the remote.
    let temp = tempfile::tempdir().expect("temp");
    let origin = temp.path().join("origin.git");
    git(
        temp.path(),
        &["init", "-q", "--bare", origin.to_str().expect("path")],
    );
    let team = temp.path().join("team");
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().expect("path"),
            team.to_str().expect("path"),
        ],
    );
    git(
        &team,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    git(&team, &["push", "-q", "origin", "HEAD:main"]);
    let shared = RecordStore::initialize(&team, StoreKind::Shareable).expect("shared");
    let url = format!("git+file://{}", origin.display());
    let status = Command::new("bd")
        .args(["dolt", "remote", "add", "origin", &url])
        .current_dir(&team)
        .env("BEADS_DIR", team.join(".beads"))
        .status()
        .expect("remote");
    assert!(status.success());
    shared
        .append(
            &record("value.team", 1, "team-1", value("Team value")),
            None,
        )
        .expect("team record");
    shared.push_remote().expect("push team data");

    let me = temp.path().join("me");
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().expect("path"),
            me.to_str().expect("path"),
        ],
    );
    let private_dir = me.join(".git/whetstone/v1/fixture/private");
    let private = RecordStore::initialize(&private_dir, StoreKind::Private).expect("private");
    assert!(private.remotes().expect("remotes").is_empty(), "no remote");
    assert!(
        private.all_records().expect("records").is_empty(),
        "no team data was cloned into the private store"
    );
    assert!(matches!(
        private.push_remote(),
        Err(StorageError::PrivateStoreHasRemote(_))
    ));
}
