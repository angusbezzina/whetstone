//! The Beads adapter against throwaway `bd init` databases on the pinned
//! minimum bd version: exact round trips, tamper reporting, append
//! semantics, resumable batches, the bead type of every record kind, retired
//! kinds kept exactly but never written, and a private store that can never
//! inherit the team's remote.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use whetstone::agreement::AgreementState;
use whetstone::beads::{
    ascii_json, ensure_bd_version, RecordStore, METADATA_SCHEMA, MINIMUM_BD_VERSION, MIN_BD_VERSION,
};
use whetstone::domain::{
    AgreementRecord, Attestation, AttestationVerdict, AuthorizationAxis, Brief, ContentDigest,
    DomainError, Enforcer, EvidenceRef, ExternalRef, ExternalSystem, FlagDecision, FlagVerdict,
    Freshness, HandAnswer, HandRaise, HandTrigger, JudgmentOutcome, JudgmentReceipt, LocalReview,
    LocalReviewVerdict, Mission, PolicyStateSnapshot, PrincipalKind, PrincipalRef, Principle,
    PrincipleSource, Privacy, Proposal, ProposalState, Provenance, ProvenanceAuthority,
    ProvenanceKind, RecordBody, RecordId, RecordRef, RetiredRecord, Reviewer, Rule, RuleSource,
    Scope, Strength, VerificationAxis, VerificationReceipt, DEFAULT_JEV_MODEL,
    LOCAL_REVIEW_ASSURANCE, RETIRED_KINDS, RULE_SCHEMA_V2, SCHEMA_VERSION_V1,
};
use whetstone::storage::{AppendRequest, StorageError, StoreKind};

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn owner() -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local:owner".into(),
        display_name: Some("Owner".into()),
    }
}

fn record(id: &str, revision: u64, key: &str, body: RecordBody) -> AgreementRecord {
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
            recorded_at: "2026-09-10T12:00:00Z".into(),
            sources: vec![EvidenceRef {
                system: "fixture".into(),
                locator: "test".into(),
                digest: None,
            }],
            // Receipts are evidence; they never carry agreement authority.
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

fn rule(statement: &str) -> RecordBody {
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
        hand_raise: Vec::new(),
        privacy: Privacy::default(),
    })
}

fn mission(text: &str) -> RecordBody {
    RecordBody::Mission(Mission {
        statement: text.into(),
        desired_outcomes: Vec::new(),
    })
}

fn principle(text: &str) -> RecordBody {
    RecordBody::Principle(Principle {
        statement: text.into(),
        source: PrincipleSource::Custom,
        rationale: None,
    })
}

fn digest(seed: char) -> ContentDigest {
    ContentDigest::new(format!("sha256:{}", seed.to_string().repeat(64))).expect("digest")
}

fn bd(dir: &Path, args: &[&str]) -> Value {
    let output = Command::new("bd")
        .args(args)
        .current_dir(dir)
        .env("BEADS_DIR", dir.join(".beads"))
        .env("GIT_CEILING_DIRECTORIES", dir.parent().expect("parent"))
        .env("BD_NON_INTERACTIVE", "1")
        .env_remove("BEADS_DB")
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

fn show(dir: &Path, bead: &str) -> Value {
    let shown = bd(dir, &["show", bead, "--json"]);
    shown
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(shown)
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

/// Write a stored record the way an earlier Whetstone did: the exact
/// canonical bytes in the bead metadata, bypassing today's append checks.
fn plant_legacy_bead(dir: &Path, canonical: &str, digest: &str) -> String {
    let record: Value = serde_json::from_str(canonical).expect("legacy JSON");
    let kind = match record["record_type"].as_str().expect("type") {
        "decision" | "mandate" | "activation" => "decision",
        "observation_receipt" => "receipt",
        _ => "record",
    };
    let metadata = json!({
        "wh_schema": METADATA_SCHEMA,
        "wh_kind": kind,
        "wh_type": record["record_type"],
        "wh_id": record["id"],
        "wh_revision": record["revision"],
        "wh_digest": digest,
        "wh_key": record["idempotency_key"],
        "wh_supersedes": null,
        "wh_record": ascii_json(canonical.as_bytes()).expect("ascii"),
    });
    let file = dir.join("legacy-metadata.json");
    fs::write(&file, metadata.to_string()).expect("metadata file");
    let labels = format!(
        "whetstone,wh:{},wh:lifecycle:pending",
        record["record_type"].as_str().expect("type")
    );
    let created = bd(
        dir,
        &[
            "create",
            "--json",
            "--type",
            kind,
            "--title",
            "legacy record",
            "--metadata",
            &format!("@{}", file.display()),
            "--labels",
            &labels,
            "--dolt-auto-commit",
            "on",
        ],
    );
    let created = created
        .as_array()
        .and_then(|items| items.first())
        .cloned()
        .unwrap_or(created);
    created["id"].as_str().expect("bead id").to_string()
}

#[test]
fn the_pinned_bd_floor_is_one_point_one_point_two_and_is_met() {
    let version = ensure_bd_version().expect("bd is installed at the pinned minimum");
    assert!(!version.is_empty());
    assert_eq!(MINIMUM_BD_VERSION, (1, 1, 2));
    assert_eq!(MIN_BD_VERSION, "1.1.2");
    assert_eq!(
        format!(
            "{}.{}.{}",
            MINIMUM_BD_VERSION.0, MINIMUM_BD_VERSION.1, MINIMUM_BD_VERSION.2
        ),
        MIN_BD_VERSION,
        "the text pin and the tuple pin agree"
    );
}

#[test]
fn records_round_trip_byte_for_byte_and_tampering_names_the_bead() {
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");

    // Characters that break Beads' metadata column when stored raw.
    let tricky = record(
        "rule.tricky",
        1,
        "init:rule",
        rule("Line\u{2028}separator, \"quotes\", back\\slash, é, 😀 and </script>"),
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
    assert_eq!(fresh.all_records().expect("records"), vec![tricky.clone()]);

    // The raw bead holds ASCII only, and every bd read still works.
    let bead = fresh.bead_for(&reference).expect("bead").expect("bead id");
    let shown = show(&dir, &bead);
    let raw = shown["metadata"]["wh_record"].as_str().expect("wh_record");
    assert!(raw.is_ascii());
    assert!(raw.contains("\\u2028"));
    assert_eq!(shown["metadata"]["wh_digest"], reference.digest.as_str());
    assert_eq!(shown["metadata"]["wh_type"], "rule");
    assert_eq!(shown["issue_type"], "record");
    let labels = shown["labels"].as_array().expect("labels");
    for expected in ["whetstone", "wh:rule"] {
        assert!(
            labels.iter().any(|label| label == expected),
            "missing {expected}: {labels:?}"
        );
    }

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
fn appends_are_idempotent_revisioned_and_batches_resume() {
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    let v1 = record("rule.tests", 1, "v1", rule("Run the tests"));
    let v1_ref = store.append(&v1, None).expect("v1");
    // Same key, different content: conflict.
    assert!(matches!(
        store.append(&record("rule.tests", 1, "v1", rule("Other")), None),
        Err(StorageError::IdempotencyConflict(_))
    ));
    // Stale and illegal revisions fail closed.
    let mut v2 = record("rule.tests", 2, "v2", rule("Run the whole test suite"));
    v2.supersedes = Some(v1_ref.clone());
    assert!(matches!(
        store.append(&v2, None),
        Err(StorageError::StaleRevision {
            expected: None,
            actual: Some(1)
        })
    ));
    let mut v3 = record("rule.tests", 3, "v3", rule("Skip"));
    v3.supersedes = Some(v1_ref.clone());
    assert!(matches!(
        store.append(&v3, Some(1)),
        Err(StorageError::IllegalRevision {
            expected: 2,
            actual: 3
        })
    ));
    // A supersession that names another record is refused by validation.
    let mut crossed = record("rule.other", 2, "crossed", rule("Crossed"));
    crossed.supersedes = Some(v1_ref.clone());
    assert!(matches!(
        store.append(&crossed, None),
        Err(StorageError::Domain(DomainError::InvalidSupersession))
    ));
    let v2_ref = store.append(&v2, Some(1)).expect("v2");

    // The revision chain is kept: both revisions, newest last, v2 linked to v1.
    let history = store.history(&v1.id).expect("history");
    assert_eq!(
        history
            .iter()
            .map(|record| record.revision)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(store.latest(&v1.id).expect("latest").expect("present"), v2);
    let v1_bead = store.bead_for(&v1_ref).expect("bead").expect("v1 bead");
    let v2_bead = store.bead_for(&v2_ref).expect("bead").expect("v2 bead");
    let shown = show(&dir, &v2_bead);
    assert_eq!(
        shown["metadata"]["wh_supersedes"],
        format!("rule.tests@1#{}", v1_ref.digest.as_str())
    );
    assert!(
        shown.to_string().contains(&v1_bead),
        "the superseding bead depends on the one it replaces: {shown}"
    );

    // A batch interrupted after its first record resumes on replay and then
    // writes its completion marker.
    let a = record(
        "mission.project",
        1,
        "init-9:base-0:mission",
        mission("Make intent inspectable."),
    );
    let b = record(
        "principle.custom-evidence",
        1,
        "init-9:base-0:principle",
        principle("Evidence before assertion."),
    );
    store.append(&a, None).expect("first half");
    let partial = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    let pending = partial.incomplete_batches().expect("batches");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].marker_key, "batch:init-9");
    assert_eq!(
        pending[0].present,
        vec!["init-9:base-0:mission".to_string()]
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
    let references = partial.append_batch(&requests).expect("resumed batch");
    assert_eq!(
        references,
        vec![
            a.reference().expect("reference"),
            b.reference().expect("reference")
        ]
    );
    let settled = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    assert!(settled.incomplete_batches().expect("batches").is_empty());
    // The marker is not a record: two rule revisions, the mission and the principle.
    assert_eq!(settled.all_records().expect("records").len(), 4);
    // Replaying the finished batch writes nothing more.
    assert_eq!(
        settled.append_batch(&requests).expect("replayed batch"),
        references
    );
    assert_eq!(settled.all_records().expect("records").len(), 4);
    // A batch naming one key twice is refused before anything is written.
    let duplicate = record("principle.dup", 1, "dup-key", principle("Dup"));
    assert!(matches!(
        settled.append_batch(&[
            AppendRequest {
                record: &duplicate,
                expected_revision: None,
            },
            AppendRequest {
                record: &duplicate,
                expected_revision: None,
            },
        ]),
        Err(StorageError::IdempotencyConflict(_))
    ));
    assert!(matches!(
        settled.append_batch(&[]),
        Err(StorageError::EmptyBatch)
    ));
    assert_eq!(settled.all_records().expect("records").len(), 4);
}

#[test]
fn every_record_kind_lands_in_its_bead_type_with_its_digest() {
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    let rule_ref = store
        .append(
            &record("rule.tests", 1, "rule", rule("Run the tests")),
            None,
        )
        .expect("rule");
    let gate = record(
        "verification.gate_fixture",
        1,
        "gate:fixture",
        RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Custom,
                stable_id: "gate:rule.tests".into(),
                revision: None,
            },
            code_digest: digest('a'),
            policy_state: PolicyStateSnapshot {
                accepted: Some(rule_ref.clone()),
                required: None,
                installed: None,
                experimental: None,
            },
            verification: VerificationAxis::Fail,
            authorization: AuthorizationAxis::Unknown,
            freshness: Freshness::Fresh,
            checked_at: "2026-09-10T12:01:00Z".into(),
            related_records: vec![rule_ref.clone()],
            evidence: Vec::new(),
        }),
    );
    let gate_ref = store.append(&gate, None).expect("gate");
    let raise = record(
        "hand.fixture",
        1,
        "hand:fixture",
        RecordBody::HandRaise(HandRaise {
            issue: "whp-1".into(),
            trigger: HandTrigger::VagueSpec,
            rule: Some(rule_ref.id.clone()),
            question: "Which API shape?".into(),
            tried: "Read the spec.".into(),
            recommendation: "Keep the current shape.".into(),
            raised_at: "2026-09-10T12:02:00Z".into(),
        }),
    );
    let raise_ref = store.append(&raise, None).expect("raise");
    let kinds: Vec<(AgreementRecord, &str)> = vec![
        (
            record(
                "mission.project",
                1,
                "mission",
                mission("Ship honest code."),
            ),
            "record",
        ),
        (
            record(
                "principle.prove-it-works",
                1,
                "principle",
                RecordBody::Principle(Principle {
                    statement: "Verify against the real artifact.".into(),
                    source: PrincipleSource::Pstack {
                        id: "prove-it-works".into(),
                        version: "0.15.5".into(),
                    },
                    rationale: None,
                }),
            ),
            "record",
        ),
        (
            record(
                "judgment.fixture",
                1,
                "judgment:fixture",
                RecordBody::Judgment(JudgmentReceipt {
                    rule: rule_ref.clone(),
                    model: DEFAULT_JEV_MODEL.into(),
                    question_digest: digest('b'),
                    input_digest: digest('c'),
                    unit: "src/lib.rs:1-9".into(),
                    commit: Some(COMMIT.into()),
                    outcome: JudgmentOutcome::Unavailable,
                    probability_bp: None,
                    confidence_bp: None,
                    shadow: true,
                    redaction: None,
                    input_tokens: None,
                    request_id: None,
                    detail: "offline".into(),
                    answered_at: "2026-09-10T12:03:00Z".into(),
                }),
            ),
            "receipt",
        ),
        (
            record(
                "attestation.fixture",
                1,
                "attest:fixture",
                RecordBody::Attestation(Attestation {
                    rule: rule_ref.clone(),
                    commit: COMMIT.into(),
                    tree: None,
                    reviewer: Reviewer::Interrogate,
                    verdict: AttestationVerdict::Pass,
                    notes: "Reviewed the diff.".into(),
                    evidence: Vec::new(),
                    attested_at: "2026-09-10T12:04:00Z".into(),
                }),
            ),
            "receipt",
        ),
        (
            record(
                "brief.fixture",
                1,
                "brief:fixture",
                RecordBody::Brief(Brief {
                    area: "src/**".into(),
                    commit: None,
                    skills: vec!["how".into(), "why".into()],
                    reuse: vec!["the shared service".into()],
                    risks: Vec::new(),
                    notes: "Reuse the service.".into(),
                    recorded_at: "2026-09-10T12:05:00Z".into(),
                }),
            ),
            "receipt",
        ),
        (
            record(
                "decision.flag_fixture",
                1,
                "flag:fixture",
                RecordBody::FlagDecision(FlagDecision {
                    receipt: gate_ref.clone(),
                    rule: rule_ref.id.clone(),
                    verdict: FlagVerdict::Dismiss,
                    reason: "A false flag.".into(),
                    decided_by: "Owner".into(),
                    decided_at: "2026-09-10T12:06:00Z".into(),
                }),
            ),
            "decision",
        ),
        (
            record(
                "decision.hand_fixture",
                1,
                "answer:fixture",
                RecordBody::HandAnswer(HandAnswer {
                    raise: raise_ref.clone(),
                    issue: "whp-1".into(),
                    answer: "Keep it.".into(),
                    answered_by: "Owner".into(),
                    answered_at: "2026-09-10T12:07:00Z".into(),
                }),
            ),
            "decision",
        ),
    ];
    let mut expected = vec![
        (rule_ref.clone(), "rule", "record"),
        (gate_ref, "verification_receipt", "receipt"),
        (raise_ref, "hand_raise", "receipt"),
    ];
    for (record, bead_type) in &kinds {
        let reference = store.append(record, None).expect("append kind");
        expected.push((reference, record.body.type_name(), bead_type));
    }
    let fresh = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    for (reference, type_name, bead_type) in expected {
        let bead = fresh.bead_for(&reference).expect("bead").expect("bead id");
        let shown = show(&dir, &bead);
        assert_eq!(shown["issue_type"], bead_type, "{type_name}");
        assert_eq!(shown["metadata"]["wh_type"], type_name);
        assert_eq!(shown["metadata"]["wh_kind"], bead_type);
        assert_eq!(shown["metadata"]["wh_digest"], reference.digest.as_str());
        let stored = fresh.get(&reference).expect("get").expect("present");
        assert_eq!(stored.digest().expect("digest"), reference.digest);
    }
    // A receipt that claims agreement authority is refused before any write.
    let mut authoritative = gate.clone();
    authoritative.id = RecordId::new("verification.gate_forged").expect("id");
    authoritative.idempotency_key = "gate:forged".into();
    authoritative.provenance.authority = ProvenanceAuthority::OwnerAuthored;
    assert!(matches!(
        fresh.append(&authoritative, None),
        Err(StorageError::Domain(
            DomainError::OperationalRecordCannotGrantAuthority
        ))
    ));
    assert_eq!(fresh.all_records().expect("records").len(), 10);
}

#[test]
fn retired_kinds_are_read_back_byte_exact_but_never_appended() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/legacy/records.json")).expect("fixture");
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");

    // Retired kinds cannot be written, alone or in a batch.
    let retired_record = |record_type: &str| {
        record(
            "value.core",
            1,
            &format!("retired:{record_type}"),
            RecordBody::Retired(RetiredRecord {
                record_type: record_type.into(),
                record: json!({"name": "Core values", "description": "Honest changes"}),
            }),
        )
    };
    for record_type in RETIRED_KINDS {
        let retired = retired_record(record_type);
        match store.append(&retired, None) {
            Err(StorageError::Domain(DomainError::RetiredKind(kind))) => {
                assert_eq!(kind, *record_type)
            }
            other => panic!("{record_type} was not refused: {other:?}"),
        }
    }
    let retired = retired_record("core_value");
    assert!(matches!(
        store.append_batch(&[AppendRequest {
            record: &retired,
            expected_revision: None,
        }]),
        Err(StorageError::Domain(DomainError::RetiredKind(_)))
    ));
    assert!(store.all_records().expect("records").is_empty());

    // Records an earlier Whetstone wrote stay exactly as stored.
    let mut planted = Vec::new();
    for fixture in &fixtures {
        let canonical = fixture["canonical"].as_str().expect("canonical");
        let digest = fixture["digest"].as_str().expect("digest");
        planted.push((
            plant_legacy_bead(&dir, canonical, digest),
            canonical,
            digest,
        ));
    }
    let reopened = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    let records = reopened.all_records().expect("legacy records are readable");
    assert_eq!(records.len(), fixtures.len());
    let mut retired_seen = 0;
    for (bead, canonical, digest) in &planted {
        let record = records
            .iter()
            .find(|record| record.digest().expect("digest").as_str() == *digest)
            .expect("legacy record read back");
        assert_eq!(
            String::from_utf8(record.canonical_json().expect("canonical")).expect("utf8"),
            *canonical,
            "a legacy record reads back byte for byte"
        );
        let reference = record.reference().expect("reference");
        assert_eq!(
            reopened.bead_for(&reference).expect("bead").as_deref(),
            Some(bead.as_str())
        );
        if let RecordBody::Retired(body) = &record.body {
            retired_seen += 1;
            assert!(RETIRED_KINDS.contains(&body.record_type.as_str()));
            // Re-appending the exact stored revision is an idempotent replay
            // of history at most; a new revision of a retired kind never is.
            let mut next = record.clone();
            next.revision = 2;
            next.supersedes = Some(reference.clone());
            next.idempotency_key = format!("{}:next", record.idempotency_key);
            assert!(matches!(
                reopened.append(&next, Some(1)),
                Err(StorageError::Domain(DomainError::RetiredKind(_)))
            ));
        }
    }
    assert_eq!(
        retired_seen, 5,
        "value, philosophy, metric, exception and mandate are retired"
    );
    assert_eq!(
        reopened.all_records().expect("records").len(),
        fixtures.len(),
        "nothing retired was written"
    );
}

#[test]
fn a_store_holding_retired_history_still_accepts_new_records_and_batches() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("fixtures/legacy/records.json")).expect("fixture");
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    for fixture in &fixtures {
        plant_legacy_bead(
            &dir,
            fixture["canonical"].as_str().expect("canonical"),
            fixture["digest"].as_str().expect("digest"),
        );
    }
    let store = RecordStore::open_existing(&dir, StoreKind::Private).expect("reopen");
    let single = record(
        "rule.after-legacy",
        1,
        "after-legacy",
        rule("Still enforced"),
    );
    store
        .append(&single, None)
        .expect("a single append beside retired history");
    // Onboarding and every multi-record write go through append_batch: a
    // project upgraded from the old product must still be able to agree.
    let mission = record(
        "mission.project",
        1,
        "init-legacy:base-0:mission",
        mission("Make intent inspectable."),
    );
    let principle = record(
        "principle.custom-evidence",
        1,
        "init-legacy:base-0:principle",
        principle("Evidence before assertion."),
    );
    store
        .append_batch(&[
            AppendRequest {
                record: &mission,
                expected_revision: None,
            },
            AppendRequest {
                record: &principle,
                expected_revision: None,
            },
        ])
        .expect("a batch beside retired history");
    assert_eq!(
        store.all_records().expect("records").len(),
        fixtures.len() + 3
    );
}

#[test]
fn lifecycle_labels_follow_the_kernel() {
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    let candidate = record("rule.small-diffs", 1, "change-1", rule("Keep diffs small"));
    let candidate_ref = store.append(&candidate, None).expect("candidate");
    let proposal = record(
        "proposal.change-1",
        1,
        "change-1:proposal",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Rule proposed: Keep diffs small".into(),
            rationale: "Rationale: review cost".into(),
            proposed_records: vec![candidate_ref.clone()],
            binding: None,
        }),
    );
    let proposal_ref = store.append(&proposal, None).expect("proposal");
    let state = AgreementState::from_records(store.all_records().expect("records"));
    store.sync_lifecycle_labels(&state).expect("labels");
    let bead = store.bead_for(&candidate_ref).expect("bead").expect("id");
    let labels = |dir: &Path| show(dir, &bead)["labels"].to_string();
    assert!(
        labels(&dir).contains("wh:lifecycle:draft"),
        "{}",
        labels(&dir)
    );
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
    assert!(state.in_force(&candidate.id).is_some());
    store.sync_lifecycle_labels(&state).expect("labels");
    let now = labels(&dir);
    assert!(
        now.contains("wh:lifecycle:accepted") && !now.contains("wh:lifecycle:draft"),
        "{now}"
    );
    // Labels already in line are not rewritten.
    assert_eq!(store.sync_lifecycle_labels(&state).expect("labels"), 0);
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
            &record("principle.team", 1, "team-1", principle("Team principle")),
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

#[test]
fn record_refs_in_the_store_match_their_domain_digest() {
    // The adapter never invents a reference: the one it returns is exactly
    // the domain's id, revision and digest of the canonical bytes.
    let temp = tempfile::tempdir().expect("temp");
    let dir = temp.path().join("private");
    let store = RecordStore::initialize(&dir, StoreKind::Private).expect("store");
    let principle = record(
        "principle.custom-honest",
        1,
        "honest",
        principle("Say what was verified."),
    );
    let reference = store.append(&principle, None).expect("append");
    assert_eq!(
        reference,
        RecordRef {
            id: principle.id.clone(),
            revision: 1,
            digest: principle.digest().expect("digest"),
        }
    );
    assert_eq!(
        store
            .by_idempotency_key("honest")
            .expect("lookup")
            .expect("present"),
        principle
    );
}
