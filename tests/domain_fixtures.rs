use std::collections::BTreeSet;

use serde_json::json;
use whetstone::domain::{
    ContentDigest, EvidenceRef, ExternalRef, ExternalSystem, PrincipalKind, PrincipalRef,
    RecordBody, RecordId, RecordRef, RepairAttemptRecord, RepairAuthorityReservationRecord,
    RepairCheckPhaseRecord, RepairHandoffRecord, RepairOperationClaimRecord, RepairSessionRecord,
    RepairSessionStateRecord, RepairSnapshotRecord, RepairWorkspaceFileRecord,
};

#[test]
fn direction_fixture_catalog_covers_every_demo_and_required_edge() {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/direction_cases.json"))
            .expect("valid fixture catalog");
    assert_eq!(catalog["schema_version"], 1);
    let cases = catalog["cases"].as_array().expect("cases array");
    let ids = cases
        .iter()
        .filter_map(|case| case["id"].as_str())
        .collect::<BTreeSet<_>>();
    let required = [
        "T-042", "T-043", "T-044", "T-045", "T-046", "T-047", "P-048", "C-049", "X-050", "X-051",
        "X-052", "X-053",
    ];
    assert_eq!(ids, required.into_iter().collect());
    assert_eq!(cases.len(), required.len());
}

#[test]
fn published_v1_schema_is_strict_and_version_pinned() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../references/agreement-record-v1.schema.json"
    ))
    .expect("valid JSON schema");
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["$defs"]["scope"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["proposal"]["additionalProperties"], false);
    assert_eq!(schema["$defs"]["decision"]["additionalProperties"], false);
}

#[test]
fn published_schema_covers_strict_serialized_repair_records() {
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../references/agreement-record-v1.schema.json"
    ))
    .expect("valid JSON schema");
    let reference = record_ref("repair.session.fixture", 1, 'a');
    let snapshot = snapshot();
    let principal = PrincipalRef {
        kind: PrincipalKind::LocalUser,
        stable_id: "owner-42".into(),
        display_name: None,
    };
    let task = ExternalRef {
        system: ExternalSystem::Beads,
        stable_id: "whetstone-k5r.11".into(),
        revision: Some("1".into()),
    };
    let bodies = [
        RecordBody::RepairSession(Box::new(RepairSessionRecord {
            session_id: "fixture".into(),
            lean_baseline_revision: "2c3f0a3".into(),
            authority_principal: principal,
            task: task.clone(),
            objective: "Repair the authorized source.".into(),
            non_goals: vec!["Do not change policy.".into()],
            applicable_guidance: Vec::new(),
            authority_revision: 1,
            authority_expires_at: "2026-09-09T13:00:00Z".into(),
            authority_expires_at_unix: 200,
            allowed_paths: vec!["src".into()],
            excluded_paths: Vec::new(),
            check_paths: vec!["src".into()],
            check_language: Some("python".into()),
            check_rules: vec!["team.rule".into()],
            final_check_paths: vec!["src".into()],
            final_check_language: Some("python".into()),
            final_check_rules: vec!["team.rule".into()],
            final_baseline_snapshot: snapshot.clone(),
            reviewed_snapshot: snapshot.clone(),
            reviewed_phase: RepairCheckPhaseRecord::Fast,
            workspace_files: vec![RepairWorkspaceFileRecord {
                path: "src/app.py".into(),
                digest: digest('a'),
                protected: false,
            }],
            started_at_unix: 100,
            last_checkpoint_at_unix: 101,
            max_attempts: 3,
            max_repeated_finding: 2,
            max_elapsed_seconds: 600,
            max_resource_units: 3,
            attempts: vec![RepairAttemptRecord {
                number: 1,
                candidate: snapshot.clone(),
                finding_ids: vec!["finding:one".into()],
                changed_paths: vec!["src/app.py".into()],
                elapsed_seconds: 1,
                resource_units: 1,
                observed_at_unix: 101,
            }],
            attempts_reserved: 2,
            total_elapsed_seconds: 1,
            total_resource_units: 1,
            final_verification_elapsed_seconds: 0,
            final_verification_resource_units: 0,
            current_finding_ids: vec!["finding:one".into()],
            state: RepairSessionStateRecord::Ready,
            updated_at: "2026-09-09T12:00:01Z".into(),
            last_check_response: json!({"schema": "whetstone.command-response.v1"}),
            last_checkpoint_kind: "post_edit_hook".into(),
            last_check_receipt: Some(record_ref("verification.fixture", 1, 'b')),
        })),
        RecordBody::RepairHandoff(Box::new(RepairHandoffRecord {
            session: reference.clone(),
            expected_session_revision: 1,
            reviewed_snapshot: snapshot.clone(),
            stable_finding_ids: vec!["finding:one".into()],
            question: "Which accountable change should proceed?".into(),
            recommendation: "Keep policy unchanged.".into(),
            alternatives: vec!["Authorize a reviewed scope change.".into()],
            impact: "The repair is paused.".into(),
            evidence: vec![EvidenceRef {
                system: "whetstone_check".into(),
                locator: "verification.fixture@1".into(),
                digest: Some(digest('b')),
            }],
            permitted_next_step: "Record an owner decision.".into(),
        })),
        RecordBody::RepairAuthorityReservation(Box::new(RepairAuthorityReservationRecord {
            project: "fixture".into(),
            task: task.clone(),
            authority_revision: 1,
            requested_session_id: "fixture".into(),
            begin_request_id: "fixture-begin".into(),
            started_at_unix: 100,
            bootstrap_resource_units: 2,
            session: Some(reference.clone()),
        })),
        RecordBody::RepairOperationClaim(Box::new(RepairOperationClaimRecord {
            session: reference,
            request_id: "fixture-check".into(),
            expected_session_revision: 1,
            phase: RepairCheckPhaseRecord::Fast,
            checkpoint_kind: "post_edit_hook".into(),
            started_at_unix: 101,
            resource_units: 1,
        })),
    ];

    for body in bodies {
        let serialized = serde_json::to_value(body).expect("serialize repair record body");
        assert!(schema_accepts_record_body_shape(&schema, &serialized));
        let record_type = serialized["record_type"].as_str().expect("record type");
        let definition = &schema["$defs"][record_type];
        let first_required = definition["required"][0].as_str().expect("required field");
        let mut missing = serialized.clone();
        missing["record"]
            .as_object_mut()
            .expect("record object")
            .remove(first_required);
        assert!(!schema_accepts_record_body_shape(&schema, &missing));
        let mut unknown = serialized;
        unknown["record"]
            .as_object_mut()
            .expect("record object")
            .insert("unexpected".into(), json!(true));
        assert!(!schema_accepts_record_body_shape(&schema, &unknown));
    }
}

fn digest(seed: char) -> ContentDigest {
    ContentDigest::new(format!("sha256:{}", seed.to_string().repeat(64))).expect("digest")
}

fn record_ref(id: &str, revision: u64, seed: char) -> RecordRef {
    RecordRef {
        id: RecordId::new(id).expect("record ID"),
        revision,
        digest: digest(seed),
    }
}

fn snapshot() -> RepairSnapshotRecord {
    RepairSnapshotRecord {
        code_tree: digest('a'),
        policy: digest('b'),
        checker_bundle: digest('c'),
        scope: digest('d'),
        environment: digest('e'),
        trust: digest('f'),
    }
}

fn schema_accepts_record_body_shape(schema: &serde_json::Value, value: &serde_json::Value) -> bool {
    let Some(record_type) = value.get("record_type").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let enum_values = schema["properties"]["record_type"]["enum"]
        .as_array()
        .expect("record type enum");
    if !enum_values.iter().any(|value| value == record_type) {
        return false;
    }
    let definition = &schema["$defs"][record_type];
    if definition["additionalProperties"] != false {
        return false;
    }
    let Some(record) = value.get("record").and_then(serde_json::Value::as_object) else {
        return false;
    };
    let properties = definition["properties"]
        .as_object()
        .expect("definition properties");
    let required = definition["required"]
        .as_array()
        .expect("definition required fields");
    required
        .iter()
        .filter_map(serde_json::Value::as_str)
        .all(|field| record.contains_key(field))
        && record.keys().all(|field| properties.contains_key(field))
}
