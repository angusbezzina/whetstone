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
        hand_raise: Vec::new(),
        privacy: Privacy::default(),
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
fn imported_notes_and_missing_evidence_cannot_grant_authority_or_pass() {
    let mut imported = record("principle.imported", "100", principle("Blue buttons"));
    imported.provenance.kind = ProvenanceKind::ImportedNote;
    imported.provenance.authority = ProvenanceAuthority::IndependentlyApproved;
    assert_eq!(
        imported.validate(),
        Err(DomainError::ImportedContentCannotGrantAuthority)
    );
    let receipt = record(
        "verification.missing",
        "100",
        RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Beads,
                stable_id: "T-044".into(),
                revision: Some("4".into()),
            },
            code_digest: digest('d'),
            policy_state: PolicyStateSnapshot {
                accepted: None,
                required: None,
                installed: None,
                experimental: None,
            },
            verification: VerificationAxis::Pass,
            authorization: AuthorizationAxis::Authorized,
            freshness: Freshness::Missing,
            checked_at: "2026-09-09T12:00:00Z".into(),
            related_records: vec![],
            evidence: vec![],
        }),
    );
    // Receipts are candidate-only; a pass also needs fresh evidence.
    assert!(receipt.validate().is_err());
    let mut candidate = receipt;
    candidate.provenance.authority = ProvenanceAuthority::CandidateOnly;
    assert_eq!(
        candidate.validate(),
        Err(DomainError::PassRequiresFreshEvidence)
    );
}

#[test]
fn a_rule_has_one_strength_one_enforcer_and_valid_examples() {
    let tokens = rule(Enforcer::DesignTokens {
        tokens: "src/tokens.css".into(),
        stylesheets: Vec::new(),
        sizes: true,
    });
    tokens.validate().expect("valid rule");
    assert_eq!(tokens.enforcer.family(), EnforcerFamily::Mechanical);
    assert!(tokens.enforcer.runs_staged());
    assert!(tokens.applies_to("src/app.css"));
    assert!(!tokens.applies_to("docs/a.css"));
    let json = serde_json::to_value(&tokens).expect("json");
    assert_eq!(json["enforcer"]["kind"], "design_tokens");
    let question = serde_json::from_value::<Rule>(serde_json::json!({
        "schema": RULE_SCHEMA_V2,
        "statement": "No unrequested tests",
        "rationale": "Maintenance cost",
        "strength": "should",
        "enforcer": {"kind": "question", "question": "Does this add a test nobody asked for?"},
        "source": {"kind": "starter"}
    }))
    .expect("question rule");
    assert!(question.in_shadow(), "new question rules start in shadow");
    assert!(matches!(
        question.enforcer,
        Enforcer::Question { ref model, threshold_bp: 5_000, promote_after: 50, .. } if model == DEFAULT_JEV_MODEL
    ));
    for broken in [
        {
            let mut rule = tokens.clone();
            rule.schema = "whetstone.rule.v1".into();
            rule
        },
        {
            let mut rule = tokens.clone();
            rule.paths = vec!["../outside".into()];
            rule
        },
        {
            let mut rule = tokens.clone();
            rule.privacy.redact_patterns = vec!["(".into()];
            rule
        },
        rule(Enforcer::PublicSurface {
            surfaces: vec!["everything".into()],
        }),
        rule(Enforcer::Review {
            reviewer: Reviewer::Person { name: " ".into() },
        }),
    ] {
        assert!(broken.validate().is_err(), "{broken:?} was accepted");
    }
}

#[test]
fn earlier_standards_and_guidance_read_as_rules() {
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
    assert_eq!(rule.strength, Strength::Advisory);
    assert!(matches!(rule.enforcer, Enforcer::Test { ref command } if command == "cargo test"));
    assert!(rule.rationale.contains("cargo test -q"));
    let body = RecordBody::Guidance(Guidance {
        statement: "Prefer small diffs".into(),
        rationale: "Review cost".into(),
        examples: Vec::new(),
    });
    let view = body.rule_view().expect("rule view");
    assert_eq!(view.enforcer.family(), EnforcerFamily::Review);
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
        retired, 5,
        "value, philosophy, metric, exception and mandate"
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

#[test]
fn judgment_receipts_carry_digests_and_honest_unavailability() {
    let mut judgment = JudgmentReceipt {
        rule: reference("rule.tests", 'b'),
        model: DEFAULT_JEV_MODEL.into(),
        question_digest: digest('c'),
        input_digest: digest('d'),
        unit: "tests/a.rs".into(),
        commit: None,
        outcome: JudgmentOutcome::Flag,
        probability_bp: Some(9_100),
        confidence_bp: Some(8_200),
        shadow: true,
        redaction: None,
        input_tokens: Some(300),
        request_id: None,
        detail: "Jev answered flag".into(),
        answered_at: "2026-09-28T10:00:00Z".into(),
    };
    let mut receipt = record(
        "verification.judgment_x",
        "100",
        RecordBody::Judgment(judgment.clone()),
    );
    receipt.provenance.authority = ProvenanceAuthority::CandidateOnly;
    receipt.validate().expect("valid judgment");
    judgment.outcome = JudgmentOutcome::Unavailable;
    let mut unavailable = record(
        "verification.judgment_y",
        "100",
        RecordBody::Judgment(judgment.clone()),
    );
    unavailable.provenance.authority = ProvenanceAuthority::CandidateOnly;
    assert!(
        unavailable.validate().is_err(),
        "an unavailable judgment cannot carry a probability"
    );
    judgment.probability_bp = None;
    judgment.confidence_bp = None;
    let mut honest = record(
        "verification.judgment_z",
        "100",
        RecordBody::Judgment(judgment),
    );
    honest.provenance.authority = ProvenanceAuthority::CandidateOnly;
    honest.validate().expect("honest unavailable judgment");
}

fn feature_body() -> Feature {
    Feature {
        name: "Rules".into(),
        summary: "Rules grouped by strength".into(),
        area: "Dashboard".into(),
        sweep_order: 2,
        sub_features: vec!["edit: inline draft edit".into()],
        user_path: "Open wh dash and choose Rules.".into(),
        drive_steps: vec!["open /".into(), "click #tab-rules".into()],
        proof: "Rules render grouped by strength.".into(),
        gotchas: vec![],
        entry_points: vec!["assets/dashboard/".into(), "src/**/dashboard*.rs".into()],
        serves: vec![id("mission.project")],
        constrained_by: vec![id("principle.prove-it-works")],
        proven_by: vec![id("rule.dashboard-journey")],
        index_summary: None,
        harness: None,
        preconditions: vec![],
        drive_recipe: vec![],
        mutations: vec![],
    }
}

#[test]
fn feature_records_validate_links_paths_and_bounds() {
    let valid = record("feature.rules", "100", RecordBody::Feature(feature_body()));
    valid.validate().expect("valid feature");
    assert_eq!(valid.body.type_name(), "feature");
    let json = serde_json::to_value(&valid).expect("serialize");
    assert_eq!(json["record_type"], "feature");
    for mutate in [
        |body: &mut Feature| body.proof = " ".into(),
        |body: &mut Feature| body.entry_points = vec!["../outside".into()],
        |body: &mut Feature| body.entry_points = vec!["/etc/passwd".into()],
        |body: &mut Feature| body.serves = vec![id("mission.project"), id("mission.project")],
        |body: &mut Feature| body.drive_steps = vec!["".into()],
    ] {
        let mut body = feature_body();
        mutate(&mut body);
        assert!(
            record("feature.bad", "100", RecordBody::Feature(body))
                .validate()
                .is_err(),
            "invalid feature accepted"
        );
    }
}

#[test]
fn feature_entry_points_match_prefixes_and_globs_only() {
    let feature = feature_body();
    assert!(feature.covers_path("assets/dashboard/app.js"));
    assert!(feature.covers_path("./assets/dashboard/index.html"));
    assert!(feature.covers_path("src/dashboard.rs"));
    assert!(feature.covers_path("src/nested/dashboard_service.rs"));
    assert!(!feature.covers_path("assets/dashboardx/app.js"));
    assert!(!feature.covers_path("src/service.rs"));
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
