use super::*;

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
fn principles_name_their_pstack_source_or_are_the_owners_own() {
    let pstack = |id: &str| Principle {
        statement: "Verify against the real artifact".into(),
        source: PrincipleSource::Pstack {
            id: id.into(),
            version: "0.15.5".into(),
        },
        rationale: None,
    };
    pstack("prove-it-works")
        .validate()
        .expect("pstack principle");
    assert!(pstack("Not A Slug").validate().is_err());
    let custom = Principle {
        statement: "Leave the campsite cleaner.".into(),
        source: PrincipleSource::Custom,
        rationale: None,
    };
    custom.validate().expect("custom principle");
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
