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
        exclude: None,
        mode: RuleMode::Tree,
        batch: None,
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

#[test]
fn exclude_takes_paths_out_of_scope_and_defaults_to_generated_code() {
    let mut scoped = rule(Enforcer::Ast {
        query: "(unsafe_block) @match".into(),
        language: Some("rust".into()),
    });
    scoped.paths = vec!["crates/**".into()];
    assert!(scoped.applies_to("crates/api/src/lib.rs"));
    assert!(
        !scoped.applies_to("crates/api/src/generated/client.rs"),
        "no exclude means generated code is out of scope"
    );
    assert_eq!(scoped.exclusions(), DEFAULT_EXCLUDE);

    scoped.exclude = Some(Vec::new());
    assert!(
        scoped.applies_to("crates/api/src/generated/client.rs"),
        "an empty exclude wants generated code"
    );
    assert!(scoped.exclusions().is_empty());

    scoped.exclude = Some(vec!["crates/legacy/".into(), "**/*.pb.rs".into()]);
    assert!(!scoped.applies_to("crates/legacy/src/lib.rs"));
    assert!(!scoped.applies_to("crates/api/src/types.pb.rs"));
    assert!(scoped.applies_to("crates/api/src/generated/client.rs"));
    assert!(
        !scoped.applies_to("docs/a.rs"),
        "exclude never widens paths"
    );
    scoped.validate().expect("valid exclude");

    for everything in ["**", "./**", "*"] {
        let mut broken = scoped.clone();
        broken.exclude = Some(vec![everything.into()]);
        assert!(
            broken.validate().is_err(),
            "{everything} excludes everything"
        );
    }
    let mut outside = scoped.clone();
    outside.exclude = Some(vec!["../outside".into()]);
    assert!(outside.validate().is_err());
}

#[test]
fn an_unsure_entry_never_covers_its_excluded_paths() {
    let unsure = SweepUnsure {
        paths: vec!["tests/**".into()],
        exclude: vec!["tests/golden/**".into()],
        matching: Some("fixture".into()),
        reason: "fixtures may flag".into(),
    };
    assert!(unsure.covers(Some("tests/a.rs"), "x"));
    assert!(!unsure.covers(Some("tests/golden/a.rs"), "x"));
    assert!(
        !unsure.covers(Some("tests/golden/a.rs"), "a fixture"),
        "exclude wins over the pattern too"
    );
    assert!(unsure.covers(Some("src/a.rs"), "a fixture"));
    assert!(unsure.covers(None, "a fixture"));
}

#[test]
fn mode_and_batch_read_and_write_their_file_names() {
    let tree = rule(Enforcer::Ast {
        query: "(unsafe_block) @match".into(),
        language: Some("rust".into()),
    });
    let json = serde_json::to_value(&tree).expect("json");
    for field in ["exclude", "mode", "batch"] {
        assert!(
            json.get(field).is_none(),
            "{field} is left out at its default"
        );
    }
    let mut change = tree.clone();
    change.mode = RuleMode::Change;
    change.batch = Some(Batch::ByCrate);
    change.exclude = Some(Vec::new());
    let json = serde_json::to_value(&change).expect("json");
    assert_eq!(json["mode"], "change");
    assert_eq!(json["batch"], "by-crate");
    assert_eq!(json["exclude"], serde_json::json!([]));
    assert_eq!(serde_json::from_value::<Rule>(json).expect("rule"), change);
    assert_eq!(
        serde_json::to_value(Batch::ByFile).expect("json"),
        "by-file"
    );
}
