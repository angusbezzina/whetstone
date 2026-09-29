//! The direction fixture catalogue (`fixtures/direction_cases.json`) run
//! against the domain: rule v2 validation (one strength, exactly one
//! enforcer, shadow questions, privacy, examples), receipts, earlier rules
//! read through their migration, and retired kinds that are read exactly but
//! never written. The published schemas are checked against the same types.

use std::collections::BTreeSet;

use serde_json::{json, Value};
use whetstone::domain::{
    AgreementHistory, AgreementRecord, DomainError, Enforcer, RecordBody, RETIRED_KINDS,
};

fn catalogue() -> Value {
    serde_json::from_str(include_str!("fixtures/direction_cases.json"))
        .expect("valid fixture catalogue")
}

fn record_schema() -> Value {
    serde_json::from_str(include_str!(
        "../references/agreement-record-v1.schema.json"
    ))
    .expect("valid record schema")
}

fn rule_schema() -> Value {
    serde_json::from_str(include_str!("../references/rule-v2.schema.json"))
        .expect("valid rule schema")
}

/// A complete record envelope around one fixture body.
fn envelope(case: &Value) -> Value {
    let id = format!(
        "fixture.{}",
        case["id"].as_str().expect("case id").to_ascii_lowercase()
    );
    let receipt = matches!(
        case["record_type"].as_str(),
        Some("judgment" | "attestation" | "brief" | "hand_raise" | "verification_receipt")
    );
    json!({
        "schema_version": 1,
        "id": id,
        "revision": 1,
        "scope": {"project": "fixture"},
        "owner": {"kind": "local_user", "stable_id": "local:owner"},
        "provenance": {
            "kind": "human_authored",
            "recorded_by": {"kind": "local_user", "stable_id": "local:owner"},
            "recorded_at": "2026-09-10T12:00:00Z",
            "sources": [{"system": "fixture", "locator": "direction_cases.json"}],
            "authority": if receipt { "candidate_only" } else { "owner_authored" },
        },
        "idempotency_key": format!("fixture:{id}"),
        "record_type": case["record_type"],
        "record": case["record"],
    })
}

fn cases() -> Vec<Value> {
    catalogue()["cases"]
        .as_array()
        .expect("cases array")
        .clone()
}

#[test]
fn the_catalogue_is_versioned_unique_and_covers_every_expectation() {
    let catalogue = catalogue();
    assert_eq!(catalogue["schema_version"], 2);
    let cases = cases();
    let ids = cases
        .iter()
        .map(|case| case["id"].as_str().expect("id"))
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), cases.len(), "case ids are unique");
    let expectations = catalogue["expectations"]
        .as_object()
        .expect("expectations")
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let used = cases
        .iter()
        .map(|case| case["expect"].as_str().expect("expect").to_string())
        .collect::<BTreeSet<_>>();
    assert_eq!(used, expectations, "every expectation is exercised");
    // Every enforcer family and every retired kind the catalogue names is real.
    for case in &cases {
        if let Some(family) = case["family"].as_str() {
            assert!(["mechanical", "question", "review"].contains(&family));
        }
        if case["expect"] == "retired" {
            assert!(
                RETIRED_KINDS.contains(&case["record_type"].as_str().expect("type")),
                "{}",
                case["id"]
            );
        }
    }
}

#[test]
fn every_case_meets_its_expectation() {
    for case in cases() {
        let id = case["id"].as_str().expect("id");
        let bytes = serde_json::to_vec(&envelope(&case)).expect("envelope bytes");
        let parsed = serde_json::from_slice::<AgreementRecord>(&bytes);
        match case["expect"].as_str().expect("expect") {
            "parse_error" => {
                assert!(parsed.is_err(), "{id} parsed but must not: {parsed:?}");
            }
            "invalid" => {
                let record = parsed.unwrap_or_else(|error| panic!("{id} did not parse: {error}"));
                assert!(
                    matches!(record.validate(), Err(DomainError::InvalidField(_))),
                    "{id} validated but must not: {:?}",
                    record.validate()
                );
                let mut history = AgreementHistory::default();
                assert!(history.append(record, None).is_err(), "{id} was appended");
            }
            "valid" => {
                let record = parsed.unwrap_or_else(|error| panic!("{id} did not parse: {error}"));
                record
                    .validate()
                    .unwrap_or_else(|error| panic!("{id} is invalid: {error}"));
                assert_eq!(record.body.type_name(), case["record_type"], "{id}");
                // Canonical bytes are a fixed point of parse and serialize.
                let canonical = record.canonical_json().expect("canonical");
                let again: AgreementRecord =
                    serde_json::from_slice(&canonical).expect("reparse canonical");
                assert_eq!(again, record, "{id}");
                assert_eq!(
                    again.canonical_json().expect("canonical"),
                    canonical,
                    "{id}"
                );
                let mut history = AgreementHistory::default();
                let reference = history
                    .append(record.clone(), None)
                    .unwrap_or_else(|error| panic!("{id} was not appended: {error}"));
                assert_eq!(reference.digest, record.digest().expect("digest"));
                assert_eq!(
                    history.append(record, None).expect("idempotent replay"),
                    reference,
                    "{id}"
                );
            }
            "retired" => {
                let record = parsed.unwrap_or_else(|error| panic!("{id} did not parse: {error}"));
                let RecordBody::Retired(retired) = &record.body else {
                    panic!("{id} was not read as retired: {:?}", record.body);
                };
                assert_eq!(retired.record_type, case["record_type"], "{id}");
                assert_eq!(retired.record, case["record"], "{id} keeps its body");
                record
                    .validate()
                    .expect("retired history validates as stored");
                assert_eq!(
                    record.canonical_json().expect("canonical"),
                    bytes,
                    "{id} re-serializes byte for byte"
                );
                let mut history = AgreementHistory::default();
                let record_type = retired.record_type.clone();
                match history.append(record, None) {
                    Err(DomainError::RetiredKind(kind)) => assert_eq!(kind, record_type),
                    other => panic!("{id} was not refused as retired: {other:?}"),
                }
            }
            other => panic!("{id} has unknown expectation {other}"),
        }
    }
}

#[test]
fn rule_cases_keep_their_family_shadow_privacy_and_defaults() {
    for case in cases() {
        if case["expect"] != "valid" {
            continue;
        }
        let id = case["id"].as_str().expect("id");
        let record: AgreementRecord =
            serde_json::from_value(envelope(&case)).expect("valid case parses");
        let Some(rule) = record.body.rule_view() else {
            assert!(
                case.get("family").is_none(),
                "{id} names a family but is no rule"
            );
            continue;
        };
        rule.validate().expect("the rule view is a valid v2 rule");
        if let Some(family) = case["family"].as_str() {
            assert_eq!(rule.enforcer.family().label(), family, "{id}");
        }
        assert_eq!(
            rule.in_shadow(),
            case["shadow"].as_bool().unwrap_or(false),
            "{id}: only a question in shadow is recorded without being enforced"
        );
        assert_eq!(
            rule.privacy.local_only,
            case["local_only"].as_bool().unwrap_or(false),
            "{id}"
        );
        if let Some(strength) = case["rule_view_strength"].as_str() {
            assert_eq!(rule.strength.label(), strength, "{id}");
        }
        if let Some(expected) = case.get("serializes_enforcer_as") {
            assert_eq!(
                &serde_json::to_value(&rule.enforcer).expect("enforcer JSON"),
                expected,
                "{id}: question defaults are written out"
            );
        }
        if case["record_type"] == "rule" {
            assert_eq!(
                serde_json::to_value(&rule.examples).expect("examples"),
                case["record"]
                    .get("examples")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                "{id}: labelled examples survive exactly"
            );
        }
    }
}

#[test]
fn each_enforcer_kind_parses_alone_and_matches_the_published_rule_schema() {
    let schema = rule_schema();
    let published = schema["$defs"]["enforcer"]["oneOf"]
        .as_array()
        .expect("enforcer oneOf")
        .iter()
        .map(|option| {
            assert_eq!(option["additionalProperties"], false, "{option}");
            option["properties"]["kind"]["const"]
                .as_str()
                .expect("kind const")
                .to_string()
        })
        .collect::<Vec<_>>();
    let minimal = [
        json!({"kind": "ast", "query": "(identifier) @id"}),
        json!({"kind": "lint", "tool": "clippy", "code": "clippy::unwrap_used"}),
        json!({"kind": "formatter", "tool": "rustfmt"}),
        json!({"kind": "test", "command": "cargo test"}),
        json!({"kind": "validator", "command": "wh validate"}),
        json!({"kind": "drive", "feature": "feature.dashboard"}),
        json!({"kind": "design_tokens", "tokens": "assets/tokens.css"}),
        json!({"kind": "public_surface"}),
        json!({"kind": "brief"}),
        json!({"kind": "question", "question": "Is this public?"}),
        json!({"kind": "review", "reviewer": {"by": "interrogate"}}),
    ];
    let mut kinds = Vec::new();
    for value in minimal {
        let enforcer: Enforcer = serde_json::from_value(value.clone())
            .unwrap_or_else(|error| panic!("{value} did not parse: {error}"));
        kinds.push(enforcer.kind().to_string());
        let option = schema["$defs"]["enforcer"]["oneOf"]
            .as_array()
            .expect("oneOf")
            .iter()
            .find(|option| option["properties"]["kind"]["const"] == enforcer.kind())
            .expect("published option");
        let serialized = serde_json::to_value(&enforcer).expect("serialize");
        let properties = option["properties"].as_object().expect("properties");
        for key in serialized.as_object().expect("object").keys() {
            assert!(
                properties.contains_key(key),
                "{} writes {key}, which the schema does not publish",
                enforcer.kind()
            );
        }
        for required in option["required"].as_array().expect("required") {
            assert!(
                serialized.get(required.as_str().expect("name")).is_some(),
                "{} omits required {required}",
                enforcer.kind()
            );
        }
        // An unknown key is a second enforcer in disguise: refused.
        let mut extra = value.clone();
        extra["command"] = json!("true");
        if enforcer.kind() != "test" && enforcer.kind() != "validator" {
            assert!(
                serde_json::from_value::<Enforcer>(extra).is_err(),
                "{} accepted a foreign key",
                enforcer.kind()
            );
        }
    }
    assert_eq!(
        kinds, published,
        "the schema publishes exactly the enforcers"
    );
}

#[test]
fn the_record_schema_lists_current_kinds_and_retires_the_removed_ones() {
    let schema = record_schema();
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["$defs"]["scope"]["additionalProperties"], false);
    let writable = schema["properties"]["record_type"]["enum"]
        .as_array()
        .expect("record type enum")
        .iter()
        .map(|kind| kind.as_str().expect("kind").to_string())
        .collect::<BTreeSet<_>>();
    let retired = schema["x-retired-kinds"]["kinds"]
        .as_array()
        .expect("retired kinds")
        .iter()
        .map(|kind| kind.as_str().expect("kind"))
        .collect::<Vec<_>>();
    assert_eq!(
        retired, RETIRED_KINDS,
        "the schema retires exactly what the domain retires"
    );
    for kind in RETIRED_KINDS {
        assert!(
            !writable.contains(*kind),
            "{kind} is still writable in the schema"
        );
    }
    for kind in [
        "mission",
        "principle",
        "rule",
        "judgment",
        "attestation",
        "brief",
        "flag_decision",
        "hand_raise",
        "hand_answer",
    ] {
        assert!(writable.contains(kind), "{kind} is not published");
        assert_eq!(
            schema["$defs"][kind]["additionalProperties"], false,
            "{kind} is strict"
        );
    }
    // Every valid catalogue case writes only published fields and all
    // required ones.
    for case in cases() {
        if case["expect"] != "valid" {
            continue;
        }
        let record: AgreementRecord =
            serde_json::from_value(envelope(&case)).expect("valid case parses");
        let serialized = serde_json::to_value(&record).expect("serialize");
        let kind = serialized["record_type"].as_str().expect("type");
        assert!(writable.contains(kind), "{kind}");
        let definition = &schema["$defs"][kind];
        let properties = definition["properties"].as_object().expect("properties");
        let body = serialized["record"].as_object().expect("record");
        for key in body.keys() {
            assert!(
                properties.contains_key(key),
                "{}: {kind} writes {key}, which the schema does not publish",
                case["id"]
            );
        }
        for required in definition["required"].as_array().expect("required") {
            assert!(
                body.contains_key(required.as_str().expect("name")),
                "{}: {kind} omits required {required}",
                case["id"]
            );
        }
    }
}
