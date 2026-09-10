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

fn scope() -> Scope {
    Scope {
        organization: Some("acme".into()),
        project: "checkout".into(),
        component: None,
        environment: None,
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
        scope: scope(),
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

fn binding() -> ProposalBinding {
    ProposalBinding {
        repository_id: 42,
        payload_digest: digest('b'),
        base_active_digest: digest('c'),
        authority_revision: 7,
        expires_at: "2026-09-16T12:00:00Z".into(),
    }
}

#[test]
fn canonical_serialization_and_digest_are_stable_and_sensitive() {
    let original = record(
        "mission.checkout",
        "100",
        RecordBody::Mission(Mission {
            statement: "Make checkout dependable".into(),
            desired_outcomes: vec!["Fewer failed purchases".into()],
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
    let valid = record(
        "value.reliability",
        "100",
        RecordBody::CoreValue(CoreValue {
            name: "Reliability".into(),
            description: "Prefer honest unknowns".into(),
        }),
    );
    let mut value = serde_json::to_value(valid).expect("value");
    value
        .as_object_mut()
        .expect("object")
        .insert("mystery".into(), serde_json::json!(true));
    assert!(serde_json::from_value::<AgreementRecord>(value).is_err());

    let mut missing = serde_json::to_value(record(
        "value.simple",
        "100",
        RecordBody::CoreValue(CoreValue {
            name: "Simple".into(),
            description: "Keep it small".into(),
        }),
    ))
    .expect("value");
    missing.as_object_mut().expect("object").remove("owner");
    assert!(serde_json::from_value::<AgreementRecord>(missing).is_err());

    let mut future = record(
        "mission.future",
        "100",
        RecordBody::Mission(Mission {
            statement: "Future".into(),
            desired_outcomes: vec![],
        }),
    );
    future.schema_version = 2;
    assert_eq!(future.validate(), Err(DomainError::UnsupportedSchema(2)));
}

#[test]
fn history_rejects_stale_writes_and_preserves_idempotency() {
    let first = record(
        "value.care",
        "100",
        RecordBody::CoreValue(CoreValue {
            name: "Care".into(),
            description: "Do not silently lose context".into(),
        }),
    );
    let mut history = AgreementHistory::default();
    let first_ref = history.append(first.clone(), None).expect("append");
    assert_eq!(history.append(first, None).expect("idempotent"), first_ref);

    let mut second = record(
        "value.care",
        "100",
        RecordBody::CoreValue(CoreValue {
            name: "Care".into(),
            description: "Preserve the complete original history".into(),
        }),
    );
    second.revision = 2;
    second.idempotency_key = "request-value-care-2".into();
    second.supersedes = Some(first_ref.clone());
    assert!(matches!(
        history.append(second.clone(), None),
        Err(DomainError::StaleRevision { .. })
    ));
    history.append(second, Some(1)).expect("revision 2");
    assert!(history.by_ref(&first_ref).is_some());
}

#[test]
fn proposal_lifecycle_is_forward_only_and_terminal_states_preserve_history() {
    let proposed_ref = reference("standard.lifecycle", '9');
    let draft = record(
        "proposal.lifecycle",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Lifecycle".into(),
            rationale: "Exercise transitions".into(),
            proposed_records: vec![proposed_ref],
            binding: None,
        }),
    );
    let draft_ref = draft.reference().expect("draft ref");
    let mut shared = draft.clone();
    shared.revision = 2;
    shared.supersedes = Some(draft_ref);
    shared.idempotency_key = "request-proposal-lifecycle-shared".into();
    let RecordBody::Proposal(shared_body) = &mut shared.body else {
        panic!("proposal")
    };
    shared_body.state = ProposalState::Shared;
    shared_body.binding = Some(binding());
    let shared_ref = shared.reference().expect("shared ref");

    let mut declined = shared.clone();
    declined.revision = 3;
    declined.supersedes = Some(shared_ref.clone());
    declined.idempotency_key = "request-proposal-lifecycle-declined".into();
    let RecordBody::Proposal(declined_body) = &mut declined.body else {
        panic!("proposal")
    };
    declined_body.state = ProposalState::Declined;

    let mut history = AgreementHistory::default();
    history.append(draft, None).expect("draft");
    history.append(shared.clone(), Some(1)).expect("shared");
    history.append(declined.clone(), Some(2)).expect("declined");
    assert!(history.by_ref(&shared_ref).is_some());

    let mut reopened = declined;
    reopened.revision = 4;
    reopened.supersedes = Some(shared_ref);
    reopened.idempotency_key = "request-proposal-lifecycle-reopened".into();
    let RecordBody::Proposal(reopened_body) = &mut reopened.body else {
        panic!("proposal")
    };
    reopened_body.state = ProposalState::Shared;
    assert_eq!(
        history.append(reopened, Some(3)),
        Err(DomainError::IllegalProposalTransition)
    );
}

#[test]
fn imported_notes_and_missing_evidence_cannot_grant_authority_or_pass() {
    let mut imported = record(
        "guidance.imported",
        "100",
        RecordBody::Guidance(Guidance {
            statement: "Always use blue buttons".into(),
            rationale: "Imported meeting note".into(),
            examples: vec![],
        }),
    );
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
    assert_eq!(
        receipt.validate(),
        Err(DomainError::PassRequiresFreshEvidence)
    );
}

#[test]
fn draft_self_approval_and_activation_without_acceptance_are_rejected() {
    let policy = record(
        "standard.latency",
        "100",
        RecordBody::Standard(Standard {
            statement: "Checkout p95 must stay below 220ms".into(),
            rationale: "Responsive checkout".into(),
            strength: StandardStrength::Must,
            enforcement: Enforcement::Test {
                command_ref: "check.checkout-latency".into(),
            },
            examples: vec![],
        }),
    );
    let policy_ref = policy.reference().expect("policy ref");
    let draft = record(
        "proposal.latency",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Tighten checkout latency".into(),
            rationale: "Local experiment succeeded".into(),
            proposed_records: vec![policy_ref.clone()],
            binding: None,
        }),
    );
    let draft_ref = draft.reference().expect("draft ref");
    let decision = record(
        "decision.latency",
        "100",
        RecordBody::Decision(Decision {
            proposal: draft_ref.clone(),
            proposal_binding: binding(),
            verdict: DecisionVerdict::Accept,
            reviewer: principal("100"),
            rationale: "Looks good".into(),
            decided_at: "2026-09-09T12:00:00Z".into(),
        }),
    );
    let mut history = AgreementHistory::default();
    history.append(policy, None).expect("policy");
    history.append(draft, None).expect("draft");
    assert_eq!(
        history.validate_decision(&decision),
        Err(DomainError::DraftCannotBeApproved)
    );

    let activation = record(
        "activation.latency",
        "200",
        RecordBody::Activation(Activation {
            proposal: draft_ref,
            acceptance_decision: decision.reference().expect("decision ref"),
            policy: policy_ref,
            binding: binding(),
            activation_sequence: 1,
            activated_at: "2026-09-09T12:01:00Z".into(),
        }),
    );
    assert_eq!(
        history.validate_activation(&activation),
        Err(DomainError::DraftCannotBeActivated)
    );
}

#[test]
fn independently_accepted_policy_activates_and_traces_without_collapsing_states() {
    let policy = record(
        "standard.latency",
        "100",
        RecordBody::Standard(Standard {
            statement: "Checkout p95 must stay below 220ms".into(),
            rationale: "Responsive checkout".into(),
            strength: StandardStrength::Must,
            enforcement: Enforcement::Test {
                command_ref: "check.checkout-latency".into(),
            },
            examples: vec![],
        }),
    );
    let policy_ref = policy.reference().expect("policy ref");
    let proposal = record(
        "proposal.latency",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Shared,
            title: "Tighten checkout latency".into(),
            rationale: "Local experiment succeeded".into(),
            proposed_records: vec![policy_ref.clone()],
            binding: Some(binding()),
        }),
    );
    let proposal_ref = proposal.reference().expect("proposal ref");
    let decision = record(
        "decision.latency",
        "200",
        RecordBody::Decision(Decision {
            proposal: proposal_ref.clone(),
            proposal_binding: binding(),
            verdict: DecisionVerdict::Accept,
            reviewer: principal("200"),
            rationale: "Evidence supports the narrower standard".into(),
            decided_at: "2026-09-09T12:00:00Z".into(),
        }),
    );
    let decision_ref = decision.reference().expect("decision ref");
    let activation = record(
        "activation.latency",
        "200",
        RecordBody::Activation(Activation {
            proposal: proposal_ref,
            acceptance_decision: decision_ref,
            policy: policy_ref,
            binding: binding(),
            activation_sequence: 1,
            activated_at: "2026-09-09T12:01:00Z".into(),
        }),
    );
    let mut history = AgreementHistory::default();
    for item in [policy, proposal, decision] {
        history.append(item, None).expect("append");
    }
    history
        .validate_activation(&activation)
        .expect("valid activation");
    let activation_ref = history.append(activation, None).expect("activation");
    assert_eq!(history.trace(&activation_ref).expect("trace").len(), 4);
}

#[test]
fn a_shared_proposal_still_cannot_be_self_approved() {
    let proposed_ref = reference("standard.boundary", 'e');
    let proposal = record(
        "proposal.boundary",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Shared,
            title: "Change a boundary".into(),
            rationale: "The public API is insufficient".into(),
            proposed_records: vec![proposed_ref],
            binding: Some(binding()),
        }),
    );
    let decision = record(
        "decision.boundary",
        "100",
        RecordBody::Decision(Decision {
            proposal: proposal.reference().expect("proposal ref"),
            proposal_binding: binding(),
            verdict: DecisionVerdict::Accept,
            reviewer: principal("100"),
            rationale: "Self approval must fail".into(),
            decided_at: "2026-09-09T12:00:00Z".into(),
        }),
    );
    let mut history = AgreementHistory::default();
    history.append(proposal, None).expect("proposal");
    assert_eq!(
        history.validate_decision(&decision),
        Err(DomainError::SelfApproval)
    );
}

#[test]
fn trace_joins_intent_constraint_change_verification_decision_and_outcome() {
    let mission = record(
        "mission.trace",
        "100",
        RecordBody::Mission(Mission {
            statement: "Dependable checkout".into(),
            desired_outcomes: vec!["No duplicate charges".into()],
        }),
    );
    let standard = record(
        "standard.trace",
        "100",
        RecordBody::Standard(Standard {
            statement: "Retries preserve idempotency keys".into(),
            rationale: "Prevent duplicate charges".into(),
            strength: StandardStrength::Must,
            enforcement: Enforcement::Test {
                command_ref: "check.retry-idempotency".into(),
            },
            examples: vec![],
        }),
    );
    let proposal = record(
        "proposal.trace",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Shared,
            title: "Preserve idempotency keys".into(),
            rationale: "Repair an observed duplicate charge".into(),
            proposed_records: vec![
                mission.reference().expect("mission ref"),
                standard.reference().expect("standard ref"),
            ],
            binding: Some(binding()),
        }),
    );
    let verification = record(
        "verification.trace",
        "100",
        RecordBody::VerificationReceipt(VerificationReceipt {
            subject: ExternalRef {
                system: ExternalSystem::Beads,
                stable_id: "T-046".into(),
                revision: Some("4".into()),
            },
            code_digest: digest('f'),
            policy_state: PolicyStateSnapshot {
                accepted: Some(standard.reference().expect("standard ref")),
                required: Some(standard.reference().expect("standard ref")),
                installed: Some(standard.reference().expect("standard ref")),
                experimental: None,
            },
            verification: VerificationAxis::Pass,
            authorization: AuthorizationAxis::Authorized,
            freshness: Freshness::Fresh,
            checked_at: "2026-09-09T12:02:00Z".into(),
            related_records: vec![proposal.reference().expect("proposal ref")],
            evidence: vec![EvidenceRef {
                system: "github-check".into(),
                locator: "check-46".into(),
                digest: Some(digest('1')),
            }],
        }),
    );
    let decision = record(
        "decision.trace",
        "200",
        RecordBody::Decision(Decision {
            proposal: proposal.reference().expect("proposal ref"),
            proposal_binding: binding(),
            verdict: DecisionVerdict::Accept,
            reviewer: principal("200"),
            rationale: "Exact change and evidence accepted".into(),
            decided_at: "2026-09-09T12:03:00Z".into(),
        }),
    );
    let metric = record(
        "metric.trace",
        "100",
        RecordBody::MetricDefinition(MetricDefinition {
            name: "Duplicate charges".into(),
            rationale: "Safeguard the mission".into(),
            source: EvidenceRef {
                system: "telemetry".into(),
                locator: "duplicate-charges".into(),
                digest: None,
            },
            cohort: "release cohort".into(),
            window: "24h".into(),
            direction: MetricDirection::Zero,
            threshold: "0".into(),
            freshness_seconds: 900,
            expected_release: None,
        }),
    );
    let outcome = record(
        "observation.trace",
        "100",
        RecordBody::ObservationReceipt(ObservationReceipt {
            metric: metric.reference().expect("metric ref"),
            release: ExternalRef {
                system: ExternalSystem::GithubRelease,
                stable_id: "REL-046.2".into(),
                revision: Some("abc4602".into()),
            },
            observed_at: "2026-09-10T12:05:00Z".into(),
            source_updated_at: "2026-09-10T12:03:00Z".into(),
            freshness: Freshness::Fresh,
            outcome: OutcomeAxis::Maintained,
            sample_size: 1400,
            summary: "No duplicate charges".into(),
            related_records: vec![
                decision.reference().expect("decision ref"),
                verification.reference().expect("verification ref"),
            ],
            evidence: vec![],
        }),
    );
    let mut history = AgreementHistory::default();
    for item in [mission, standard, proposal, verification, decision, metric] {
        history.append(item, None).expect("append trace record");
    }
    let outcome_ref = history.append(outcome, None).expect("append outcome");
    let traced_ids = history
        .trace(&outcome_ref)
        .expect("trace")
        .into_iter()
        .map(|reference| reference.id)
        .collect::<BTreeSet<_>>();
    for expected in [
        "mission.trace",
        "standard.trace",
        "proposal.trace",
        "verification.trace",
        "decision.trace",
        "observation.trace",
    ] {
        assert!(traced_ids.contains(&id(expected)), "missing {expected}");
    }
}

#[test]
fn observation_must_match_the_release_named_by_the_metric() {
    let expected = ExternalRef {
        system: ExternalSystem::GithubRelease,
        stable_id: "REL-046.1".into(),
        revision: Some("abc4601".into()),
    };
    let metric = record(
        "metric.duplicate-charge",
        "100",
        RecordBody::MetricDefinition(MetricDefinition {
            name: "Confirmed duplicate charges".into(),
            rationale: "Never charge twice".into(),
            source: EvidenceRef {
                system: "telemetry".into(),
                locator: "checkout/duplicate-charge".into(),
                digest: None,
            },
            cohort: "release checkout journeys".into(),
            window: "24h".into(),
            direction: MetricDirection::Zero,
            threshold: "0".into(),
            freshness_seconds: 900,
            expected_release: Some(expected),
        }),
    );
    let observation = record(
        "observation.duplicate-charge",
        "100",
        RecordBody::ObservationReceipt(ObservationReceipt {
            metric: metric.reference().expect("metric ref"),
            release: ExternalRef {
                system: ExternalSystem::GithubRelease,
                stable_id: "REL-046.2".into(),
                revision: Some("abc4602".into()),
            },
            observed_at: "2026-09-09T12:05:00Z".into(),
            source_updated_at: "2026-09-09T12:03:00Z".into(),
            freshness: Freshness::Fresh,
            outcome: OutcomeAxis::Harmed,
            sample_size: 1240,
            summary: "Three confirmed duplicate charges".into(),
            related_records: vec![],
            evidence: vec![],
        }),
    );
    let mut history = AgreementHistory::default();
    history.append(metric, None).expect("metric");
    assert_eq!(
        history.validate_observation(&observation),
        Err(DomainError::ObservationWrongRelease)
    );
}

#[test]
fn personal_constraints_may_narrow_but_not_contradict_team_scope() {
    let team = scope();
    let personal = Scope {
        component: Some("ui".into()),
        ..scope()
    };
    assert!(team.contains(&personal));
    assert!(!personal.contains(&team));
    let invalid = Scope {
        project: "../other".into(),
        ..scope()
    };
    assert!(matches!(
        invalid.validate(),
        Err(DomainError::InvalidScope(..))
    ));
}

#[test]
fn personal_and_team_policy_states_remain_distinct() {
    let state = PolicyStateSnapshot {
        accepted: Some(reference("policy.latency.v14", 'a')),
        required: Some(reference("policy.latency.v13", 'b')),
        installed: Some(reference("policy.latency.v13", 'b')),
        experimental: Some(reference("policy.latency.local", 'c')),
    };
    let value = serde_json::to_value(state).expect("value");
    assert_ne!(value["accepted"], value["required"]);
    assert_eq!(value["required"], value["installed"]);
    assert_ne!(value["experimental"], value["accepted"]);
}

fn local_principal() -> PrincipalRef {
    PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "local:owner".into(),
        display_name: Some("Owner".into()),
    }
}

fn feature_body() -> Feature {
    Feature {
        name: "Foundations".into(),
        summary: "Five-stage agreement flow".into(),
        area: "Dashboard".into(),
        sweep_order: 2,
        sub_features: vec!["edit: inline governed edit".into()],
        user_path: "Open wh dash and choose Foundations.".into(),
        drive_steps: vec!["open /".into(), "click #tab-foundations".into()],
        proof: "Five stages render in order.".into(),
        gotchas: vec![],
        entry_points: vec!["assets/dashboard/".into(), "src/**/dashboard*.rs".into()],
        serves: vec![id("mission.project")],
        constrained_by: vec![id("value.core")],
        proven_by: vec![id("standard.dashboard-journey")],
        index_summary: None,
        harness: None,
        preconditions: vec![],
        drive_recipe: vec![],
    }
}

#[test]
fn feature_records_validate_links_paths_and_bounds() {
    let valid = record(
        "feature.foundations",
        "100",
        RecordBody::Feature(feature_body()),
    );
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
fn drive_gates_require_a_feature_reference() {
    let gate = record(
        "standard.dashboard-journey",
        "100",
        RecordBody::Standard(Standard {
            statement: "The dashboard journey is proven by driving it".into(),
            rationale: "Browser behaviour is the product".into(),
            strength: StandardStrength::Must,
            enforcement: Enforcement::Drive {
                feature: id("feature.foundations"),
            },
            examples: vec![],
        }),
    );
    gate.validate().expect("drive gate");
    let json = serde_json::to_string(&gate).expect("serialize");
    assert!(json.contains(r#""enforcement":"drive""#));
    assert!(json.contains(r#""feature":"feature.foundations""#));
}

#[test]
fn solo_review_binds_an_owned_draft_and_never_permits_team_activation() {
    let mut history = AgreementHistory::default();
    let mut candidate = record(
        "value.evidence",
        "100",
        RecordBody::CoreValue(CoreValue {
            name: "Evidence".into(),
            description: "Evidence before assertion".into(),
        }),
    );
    candidate.owner = local_principal();
    candidate.provenance.recorded_by = local_principal();
    let candidate_ref = history.append(candidate, None).expect("candidate");
    let mut proposal = record(
        "proposal.local",
        "100",
        RecordBody::Proposal(Proposal {
            state: ProposalState::Draft,
            title: "Add evidence value".into(),
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
