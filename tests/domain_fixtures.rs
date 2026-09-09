use std::collections::BTreeSet;

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
