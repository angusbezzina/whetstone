use super::*;

fn digest(seed: char) -> ContentDigest {
    ContentDigest::new(format!("sha256:{}", seed.to_string().repeat(64))).expect("test digest")
}

fn id(value: &str) -> RecordId {
    RecordId::new(value).expect("test id")
}

fn principal(value: &str) -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::GithubUser,
        stable_id: value.to_string(),
        display_name: None,
    }
}

fn local_principal() -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local:owner".into(),
        display_name: Some("Owner".into()),
    }
}

fn provenance(actor: &str) -> Provenance {
    Provenance {
        kind: ProvenanceKind::HumanAuthored,
        recorded_by: principal(actor),
        recorded_at: "2026-09-09T12:00:00Z".into(),
        sources: vec![EvidenceRef {
            system: "conversation".into(),
            locator: "decision-1".into(),
            digest: Some(digest('a')),
        }],
        authority: ProvenanceAuthority::OwnerAuthored,
    }
}

fn record(record_id: &str, owner: &str, body: RecordBody) -> AgreementRecord {
    AgreementRecord {
        schema_version: SCHEMA_VERSION_V1,
        id: id(record_id),
        revision: 1,
        scope: Scope::project("checkout"),
        owner: principal(owner),
        provenance: provenance(owner),
        supersedes: None,
        idempotency_key: format!("request-{record_id}"),
        body,
    }
}

fn reference(record_id: &str, seed: char) -> RecordRef {
    RecordRef {
        id: id(record_id),
        revision: 1,
        digest: digest(seed),
    }
}

fn principle(text: &str) -> RecordBody {
    RecordBody::Principle(Principle {
        statement: text.into(),
        source: PrincipleSource::Custom,
        rationale: None,
    })
}

fn rule(enforcer: Enforcer) -> Rule {
    Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: "No raw colours".into(),
        rationale: "The design system owns colour".into(),
        strength: Strength::Must,
        enforcer,
        examples: vec![RuleExample {
            input: ".a { color: #fff; }".into(),
            expected: ExampleVerdict::Flag,
            reason: "A literal colour".into(),
            path: Some("src/app.css".into()),
        }],
        source: RuleSource::owner(),
        paths: vec!["src/**".into()],
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
    }
}

#[test]
fn canonical_serialization_and_digest_are_stable_and_sensitive() {
    let original = record(
        "mission.checkout",
        "100",
        RecordBody::Mission(Mission {
            statement: "Make checkout dependable".into(),
            desired_outcomes: Vec::new(),
        }),
    );
    let round_trip: AgreementRecord =
        serde_json::from_slice(&original.canonical_json().expect("serialize")).expect("parse");
    assert_eq!(original, round_trip);
    assert_eq!(
        original.digest().expect("digest"),
        round_trip.digest().expect("digest")
    );
    let mut changed = original.clone();
    let RecordBody::Mission(mission) = &mut changed.body else {
        panic!("mission")
    };
    mission.statement.push('!');
    assert_ne!(
        original.digest().expect("digest"),
        changed.digest().expect("digest")
    );
}

#[test]
fn strict_schema_rejects_missing_unknown_and_future_versions() {
    let valid = record("principle.simple", "100", principle("Keep it small"));
    let mut value = serde_json::to_value(&valid).expect("value");
    value
        .as_object_mut()
        .expect("object")
        .insert("mystery".into(), serde_json::json!(true));
    assert!(serde_json::from_value::<AgreementRecord>(value).is_err());
    let mut missing = serde_json::to_value(&valid).expect("value");
    missing.as_object_mut().expect("object").remove("owner");
    assert!(serde_json::from_value::<AgreementRecord>(missing).is_err());
    let mut unknown_field = serde_json::to_value(&valid).expect("value");
    unknown_field["record"]["mystery"] = serde_json::json!(1);
    assert!(serde_json::from_value::<AgreementRecord>(unknown_field).is_err());
    let mut future = valid;
    future.schema_version = 2;
    assert_eq!(future.validate(), Err(DomainError::UnsupportedSchema(2)));
}

#[test]
fn history_rejects_stale_writes_and_preserves_idempotency() {
    let first = record("principle.care", "100", principle("Do not lose context"));
    let mut history = AgreementHistory::default();
    let first_ref = history.append(first.clone(), None).expect("append");
    assert_eq!(history.append(first, None).expect("idempotent"), first_ref);
    let mut second = record(
        "principle.care",
        "100",
        principle("Preserve the complete history"),
    );
    second.revision = 2;
    second.idempotency_key = "request-principle-care-2".into();
    second.supersedes = Some(first_ref.clone());
    assert!(matches!(
        history.append(second.clone(), None),
        Err(DomainError::StaleRevision { .. })
    ));
    history.append(second, Some(1)).expect("revision 2");
    assert!(history.by_ref(&first_ref).is_some());
}

#[test]
fn proposals_move_forward_only() {
    let draft = record(
        "proposal.lifecycle",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Lifecycle".into(),
            rationale: "Exercise transitions".into(),
            proposed_records: vec![reference("rule.lifecycle", '9')],
            binding: None,
        }),
    );
    let mut history = AgreementHistory::default();
    let draft_ref = history.append(draft.clone(), None).expect("draft");
    let mut again = draft;
    again.revision = 2;
    again.supersedes = Some(draft_ref);
    again.idempotency_key = "request-proposal-again".into();
    assert_eq!(
        history.append(again, Some(1)),
        Err(DomainError::IllegalProposalTransition)
    );
}

#[test]
fn imported_notes_cannot_grant_authority() {
    let mut imported = record("principle.imported", "100", principle("Blue buttons"));
    imported.provenance.kind = ProvenanceKind::ImportedNote;
    imported.provenance.authority = ProvenanceAuthority::IndependentlyApproved;
    assert_eq!(
        imported.validate(),
        Err(DomainError::ImportedContentCannotGrantAuthority)
    );
}

#[test]
fn a_rule_has_one_strength_one_enforcer_and_valid_examples() {
    let ast = rule(Enforcer::Ast {
        query: "(unsafe_block) @match".into(),
        language: Some("rust".into()),
    });
    ast.validate().expect("valid rule");
    assert!(ast.enforcer.runs_staged());
    assert!(ast.applies_to("src/app.rs"));
    assert!(!ast.applies_to("docs/a.rs"));
    let json = serde_json::to_value(&ast).expect("json");
    assert_eq!(json["enforcer"]["kind"], "ast");
    assert!(
        serde_json::from_value::<Rule>(serde_json::json!({
            "schema": RULE_SCHEMA_V2,
            "statement": "No unrequested tests",
            "rationale": "Maintenance cost",
            "strength": "should",
            "enforcer": {"kind": "question", "question": "Does this add a test nobody asked for?"},
            "source": {"kind": "starter"}
        }))
        .is_err(),
        "a rule is held by a mechanical check"
    );
    for broken in [
        {
            let mut rule = ast.clone();
            rule.schema = "whetstone.rule.v1".into();
            rule
        },
        {
            let mut rule = ast.clone();
            rule.paths = vec!["../outside".into()];
            rule
        },
        rule(Enforcer::Lint {
            tool: "ruff".into(),
            code: " ".into(),
        }),
    ] {
        assert!(broken.validate().is_err(), "{broken:?} was accepted");
    }
}

#[test]
fn earlier_standards_read_as_rules() {
    let standard = Standard {
        statement: "Tests pass".into(),
        rationale: "Owner-confirmed first gate".into(),
        strength: StandardStrength::May,
        enforcement: Enforcement::Test {
            command_ref: "cargo test".into(),
        },
        examples: vec!["cargo test -q".into()],
    };
    let rule = Rule::from_standard(&standard);
    rule.validate().expect("valid");
    assert_eq!(rule.strength, Strength::Should, "an earlier may is should");
    assert!(matches!(rule.enforcer, Enforcer::Test { ref command } if command == "cargo test"));
    assert!(rule.rationale.contains("cargo test -q"));
}

#[test]
fn retired_kinds_keep_their_bytes_and_digests_and_cannot_be_created() {
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../tests/fixtures/legacy/records.json"))
            .expect("fixture");
    let mut retired = 0;
    for fixture in fixtures {
        let canonical = fixture["canonical"].as_str().expect("canonical");
        let record: AgreementRecord = serde_json::from_str(canonical).expect("parse legacy");
        record.validate().expect("legacy record validates");
        assert_eq!(
            String::from_utf8(record.canonical_json().expect("json")).expect("utf8"),
            canonical,
            "a legacy record must re-serialize byte for byte"
        );
        assert_eq!(
            record.digest().expect("digest").as_str(),
            fixture["digest"].as_str().expect("digest"),
        );
        if let RecordBody::Retired(body) = &record.body {
            retired += 1;
            assert!(RETIRED_KINDS.contains(&body.record_type.as_str()));
            let mut history = AgreementHistory::default();
            assert!(matches!(
                history.append(record.clone(), None),
                Err(DomainError::RetiredKind(_))
            ));
        }
    }
    assert_eq!(
        retired, 6,
        "value, philosophy, metric, exception, mandate and guidance"
    );
}

#[test]
fn retired_values_and_philosophy_offer_their_text_for_principles() {
    let value = RetiredRecord {
        record_type: "core_value".into(),
        record: serde_json::json!({"name": "Core values", "description": "Honest, small changes"}),
    };
    assert_eq!(
        value.principle_text(),
        Some(("Honest, small changes".into(), Some("Core values".into())))
    );
    let metric = RetiredRecord {
        record_type: "metric_definition".into(),
        record: serde_json::json!({"name": "Latency"}),
    };
    assert_eq!(metric.principle_text(), None);
}

#[test]
fn principles_name_their_pstack_source_or_are_the_owners_own() {
    let pstack = record(
        "principle.prove-it-works",
        "100",
        RecordBody::Principle(Principle {
            statement: "Verify against the real artifact".into(),
            source: PrincipleSource::Pstack {
                id: "prove-it-works".into(),
                version: "0.15.5".into(),
            },
            rationale: None,
        }),
    );
    pstack.validate().expect("pstack principle");
    let bad = record(
        "principle.bad",
        "100",
        RecordBody::Principle(Principle {
            statement: "x".into(),
            source: PrincipleSource::Pstack {
                id: "Not A Slug".into(),
                version: "0.15.5".into(),
            },
            rationale: None,
        }),
    );
    assert!(bad.validate().is_err());
}

/// Stored bodies of kinds and enforcers that are gone: each reads as a
/// retired record that re-serializes byte for byte.
const RETIRED_BODIES: &[(&str, &str)] = &[
    (
        "feature",
        r#"{"name":"Rules","summary":"Rules by strength","area":"Dashboard","user_path":"Open Rules.","proof":"They render."}"#,
    ),
    (
        "verification_map",
        r#"{"title":"Feature map","intro":"How the app is driven."}"#,
    ),
    (
        "verification_receipt",
        r#"{"subject":{"system":"custom","stable_id":"gate:rule.tests"},"verification":"pass","checked_at":"2026-09-09T12:00:00Z"}"#,
    ),
    (
        "rule",
        r#"{"schema":"whetstone.rule.v2","statement":"Search works.","rationale":"Users search.","strength":"must","enforcer":{"kind":"drive","feature":"feature.search"},"source":{"kind":"owner"}}"#,
    ),
    (
        "standard",
        r#"{"statement":"Search works.","rationale":"Users search.","strength":"must","enforcement":{"enforcement":"drive","feature":"feature.search"}}"#,
    ),
];

#[test]
fn removed_kinds_and_enforcers_read_as_retired_byte_for_byte() {
    for (kind, body) in RETIRED_BODIES {
        let mut stored = serde_json::to_value(record(
            "record.retired",
            "100",
            RecordBody::Mission(Mission {
                statement: "x".into(),
                desired_outcomes: Vec::new(),
            }),
        ))
        .expect("serialize");
        stored["record_type"] = (*kind).into();
        stored["record"] = serde_json::from_str(body).expect("body");
        let canonical = serde_json::to_string(&stored).expect("canonical");
        let record: AgreementRecord = serde_json::from_str(&canonical).expect("parse");
        assert!(
            matches!(&record.body, RecordBody::Retired(retired) if retired.record_type == *kind),
            "{kind} reads as retired"
        );
        assert_eq!(
            String::from_utf8(record.canonical_json().expect("json")).expect("utf8"),
            canonical,
            "{kind} re-serializes byte for byte"
        );
        assert!(matches!(
            AgreementHistory::default().append(record, None),
            Err(DomainError::RetiredKind(_))
        ));
    }
}

#[test]
fn paths_match_prefixes_and_globs_only() {
    assert!(entry_point_matches(
        "assets/dashboard/",
        "assets/dashboard/app.js"
    ));
    assert!(entry_point_matches(
        "src/**/dashboard*.rs",
        "src/dashboard.rs"
    ));
    assert!(entry_point_matches(
        "src/**/dashboard*.rs",
        "src/nested/dashboard_service.rs"
    ));
    assert!(!entry_point_matches(
        "assets/dashboard/",
        "assets/dashboardx/app.js"
    ));
    assert!(!entry_point_matches(
        "src/**/dashboard*.rs",
        "src/service.rs"
    ));
    assert!(entry_point_matches("tests/*.rs", "tests/a.rs"));
    assert!(!entry_point_matches("tests/*.rs", "tests/nested/a.rs"));
    assert!(entry_point_matches("**/README.md", "docs/deep/README.md"));
}

#[test]
fn solo_review_binds_an_owned_draft_and_never_permits_team_activation() {
    let mut history = AgreementHistory::default();
    let mut candidate = record("principle.evidence", "100", principle("Evidence first"));
    candidate.owner = local_principal();
    candidate.provenance.recorded_by = local_principal();
    let candidate_ref = history.append(candidate, None).expect("candidate");
    let mut proposal = record(
        "proposal.local",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Add evidence principle".into(),
            rationale: "Owner wants it".into(),
            proposed_records: vec![candidate_ref],
            binding: None,
        }),
    );
    proposal.owner = local_principal();
    proposal.provenance.recorded_by = local_principal();
    let proposal_ref = history.append(proposal, None).expect("proposal");
    let review = |reviewer: PrincipalRef, permitted: bool| {
        let mut review = record(
            "review.local",
            "100",
            RecordBody::LocalReview(LocalReview {
                proposal: proposal_ref.clone(),
                verdict: LocalReviewVerdict::Accept,
                reviewer: reviewer.clone(),
                assurance: LOCAL_REVIEW_ASSURANCE.into(),
                rationale: "Explicit solo confirmation".into(),
                reviewed_at: "2026-09-10T10:00:00Z".into(),
                team_activation_permitted: permitted,
                cites: Vec::new(),
            }),
        );
        review.owner = reviewer.clone();
        review.provenance.recorded_by = reviewer;
        review
    };
    let valid = review(local_principal(), false);
    valid.validate().expect("valid review");
    history
        .validate_local_review(&valid)
        .expect("owner may self-review a draft");
    assert_eq!(
        review(local_principal(), true).validate(),
        Err(DomainError::InvalidLocalReview)
    );
    assert_eq!(
        history.validate_local_review(&review(principal("200"), false)),
        Err(DomainError::PrincipalMismatch)
    );
}
