//! Rust integration tests for whetstone binary.
//!
//! Tests core commands against the fixtures directory and the whetstone repo itself.

use std::path::PathBuf;
use std::process::Command;

fn whetstone_bin() -> PathBuf {
    // Try to find the debug binary
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // remove test binary name
    path.pop(); // remove deps
    path.push("whetstone");
    if !path.exists() {
        // Fallback to cargo build
        let status = Command::new("cargo")
            .args(["build", "--quiet"])
            .status()
            .expect("Failed to build whetstone");
        assert!(status.success(), "cargo build failed");
    }
    path
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn run_whetstone(args: &[&str], project_dir: &str) -> (String, String, bool) {
    let bin = whetstone_bin();
    let output = Command::new(&bin)
        .args(args)
        .current_dir(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run {}: {}", bin.display(), e));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

fn parse_json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("Invalid JSON: {e}\nInput: {s}"))
}

fn assert_json_has_keys(actual: &serde_json::Value, expected_keys: &[&str], context: &str) {
    let obj = actual
        .as_object()
        .unwrap_or_else(|| panic!("{context}: expected JSON object"));
    for key in expected_keys {
        assert!(
            obj.contains_key(*key),
            "{context}: missing key '{key}'"
        );
    }
}

// ── detect-deps tests ──

#[test]
fn test_detect_deps_fixtures() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["detect-deps"], dir.to_str().unwrap());
    assert!(success, "detect-deps should succeed");

    let result = parse_json(&stdout);
    assert_eq!(result["counts"]["runtime"]["_all"], 5); // fastapi, pydantic, requests + next, react
    assert_eq!(result["counts"]["dev"]["_all"], 2); // pytest + typescript

    let deps = result["dependencies"].as_array().unwrap();
    let names: Vec<&str> = deps.iter().filter_map(|d| d["name"].as_str()).collect();
    assert!(names.contains(&"fastapi"), "Should detect fastapi");
    assert!(names.contains(&"requests"), "Should detect requests");
}

#[test]
fn test_detect_deps_typescript_fixture() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["detect-deps"], dir.to_str().unwrap());
    assert!(success);

    let result = parse_json(&stdout);
    // The fixture has a package.json too
    let deps = result["dependencies"].as_array().unwrap();
    let ts_deps: Vec<&str> = deps
        .iter()
        .filter(|d| d["language"].as_str() == Some("typescript"))
        .filter_map(|d| d["name"].as_str())
        .collect();
    assert!(
        !ts_deps.is_empty(),
        "Should detect TypeScript deps from package.json"
    );
}

#[test]
fn test_detect_deps_whetstone_repo() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let (stdout, _stderr, success) = run_whetstone(&["detect-deps"], dir);
    assert!(success, "detect-deps should succeed on whetstone repo");

    let result = parse_json(&stdout);
    let deps = result["dependencies"].as_array().unwrap();
    let rust_deps: Vec<&str> = deps
        .iter()
        .filter(|d| d["language"].as_str() == Some("rust"))
        .filter_map(|d| d["name"].as_str())
        .collect();
    assert!(
        rust_deps.contains(&"serde"),
        "Should detect serde in Cargo.toml"
    );
    assert!(
        rust_deps.contains(&"clap"),
        "Should detect clap in Cargo.toml"
    );
}

// ── status tests ──

#[test]
fn test_status_fixtures_with_rules() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot"],
        dir.to_str().unwrap(),
    );
    assert!(success, "status should succeed");

    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");

    // Should detect the fastapi rule
    let dims = &result["dimensions"];
    assert!(
        dims["rules_count"].as_i64().unwrap() >= 1,
        "Should detect at least 1 approved rule from fixtures"
    );

    let deps: Vec<&str> = result["dependencies_covered"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(deps.contains(&"fastapi"), "Should list fastapi as covered");
}

#[test]
fn test_status_score_dimensions() {
    let dir = fixtures_dir();
    let (stdout, _stderr, _) = run_whetstone(
        &["status", "--json", "--no-snapshot"],
        dir.to_str().unwrap(),
    );
    let result = parse_json(&stdout);

    let dims = &result["dimensions"];
    assert!(dims["rules_count"].as_i64().unwrap() >= 0);
    assert!(dims["high_confidence_ratio"].as_f64().is_some());
    assert!(dims["deterministic_coverage"].as_f64().is_some());

    let breakdown = &result["breakdown"];
    assert!(breakdown["confidence"]["high"].as_i64().is_some());
    assert!(breakdown["signals"]["deterministic"].as_i64().is_some());
}

#[test]
fn test_status_not_initialized() {
    let dir = std::env::temp_dir().join("whetstone_test_empty");
    let _ = std::fs::create_dir_all(&dir);
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    assert_eq!(result["status"], "not_initialized");
    let _ = std::fs::remove_dir_all(&dir);
}

// ── generate-context tests ──

#[test]
fn test_generate_context_dry_run() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["generate-context", "--dry-run", "--json"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert!(
        result["rules_count"].as_i64().unwrap() >= 1,
        "Should have approved rules"
    );
    let generated = result["generated"].as_array().unwrap();
    assert!(!generated.is_empty(), "Should generate at least one format");
    assert!(
        generated[0]["dry_run"].as_bool().unwrap(),
        "Should be dry run"
    );
}

// ── generate-tests tests ──

#[test]
fn test_generate_tests_dry_run() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["generate-tests", "--dry-run", "--json"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert!(
        result["rules_count"].as_i64().unwrap() >= 1,
        "Should have approved rules"
    );
}

// ── ci-check tests ──

#[test]
fn test_ci_check_json() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["ci-check", "--json"], dir.to_str().unwrap());
    assert!(success);

    let result = parse_json(&stdout);
    assert!(result.get("score").is_some());
    assert!(result.get("label").is_some());
    assert!(result.get("freshness_status").is_some());
}

// ── CLI tests ──

#[test]
fn test_help_output() {
    let (stdout, _stderr, success) = run_whetstone(&["--help"], ".");
    assert!(success);
    assert!(stdout.contains("detect-deps"));
    assert!(stdout.contains("resolve-sources"));
    assert!(stdout.contains("doctor"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("generate-context"));
    assert!(stdout.contains("generate-tests"));
    assert!(stdout.contains("ci-check"));
}

#[test]
fn test_version_output() {
    let (stdout, _stderr, success) = run_whetstone(&["--version"], ".");
    assert!(success);
    assert!(stdout.contains("whetstone"));
}

// ── Rule YAML parsing tests ──

#[test]
fn test_rule_yaml_parsing() {
    let dir = fixtures_dir();

    // Use the rules module directly via the binary's status command
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    // Fixtures have 2 approved rules: fastapi.async-routes + react.use-client-directive
    assert_eq!(result["dimensions"]["rules_count"], 2);
    assert_eq!(result["breakdown"]["confidence"]["high"], 2);
    // fastapi is "must", react is "should"
    assert_eq!(result["breakdown"]["severity"]["must"], 1);
    assert_eq!(result["breakdown"]["severity"]["should"], 1);
    // Both have deterministic signals (ast and pattern)
    assert!(
        result["breakdown"]["signals"]["deterministic"]
            .as_i64()
            .unwrap()
            >= 2
    );
}

// ── Edge case rule YAML tests ──

#[test]
fn test_multi_rule_file() {
    // The react.yaml fixture has 2 rules (1 approved, 1 not)
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    // 2 approved rules total (fastapi.async-routes + react.use-client-directive)
    assert_eq!(
        result["dimensions"]["rules_count"], 2,
        "Should count 2 approved rules"
    );
    let deps: Vec<&str> = result["dependencies_covered"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(deps.contains(&"fastapi"));
    assert!(deps.contains(&"react"));
}

#[test]
fn test_malformed_rule_produces_warnings() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    let warnings = result["warnings"].as_array().unwrap();
    // The malformed.yaml should produce validation warnings
    let malformed_warnings: Vec<&str> = warnings
        .iter()
        .filter_map(|w| w.as_str())
        .filter(|w| w.contains("malformed") || w.contains("missing"))
        .collect();
    assert!(
        !malformed_warnings.is_empty(),
        "Should produce warnings for malformed rules: {:?}",
        warnings
    );
}

#[test]
fn test_unapproved_rules_not_counted() {
    // react.yaml has react.no-index-keys which is not approved
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    // Only 2 approved rules, the unapproved one should not be in the count
    let total = result["dimensions"]["rules_count"].as_i64().unwrap();
    assert_eq!(total, 2, "Should only count approved rules");
}

// ── JSON contract tests ──

#[test]
fn test_detect_deps_contract() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["detect-deps"], dir.to_str().unwrap());
    assert!(success);

    let result = parse_json(&stdout);
    // Required fields
    assert!(result.get("dependencies").is_some());
    assert!(result.get("counts").is_some());

    // Every dep has required fields
    for dep in result["dependencies"].as_array().unwrap() {
        assert!(dep.get("name").is_some(), "dep missing name");
        assert!(dep.get("language").is_some(), "dep missing language");
        assert!(dep.get("version").is_some(), "dep missing version");
        assert!(dep.get("dev").is_some(), "dep missing dev flag");
    }
}

#[test]
fn test_status_contract() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(success);

    let result = parse_json(&stdout);
    // Required fields
    assert!(result.get("status").is_some());
    assert!(result.get("label").is_some());
    assert!(result.get("score").is_some());
    assert!(result.get("dimensions").is_some());
    assert!(result.get("breakdown").is_some());
    assert!(result.get("recommendations").is_some());
    assert!(result.get("next_command").is_some());
}

// ── crd.3.1: Self-host regression scenario ──

#[test]
fn test_self_host_regression_detect_deps() {
    let dir = env!("CARGO_MANIFEST_DIR");
    let (stdout, _stderr, success) = run_whetstone(&["detect-deps"], dir);
    assert!(success, "detect-deps should succeed on whetstone repo");

    let result = parse_json(&stdout);

    // Contract: required top-level fields
    assert_json_has_keys(
        &result,
        &["languages", "counts", "dependencies", "manifests"],
        "detect-deps self-host",
    );

    // Rust must be detected from the root Cargo.toml
    let languages = result["languages"].as_array().unwrap();
    let lang_strs: Vec<&str> = languages.iter().filter_map(|v| v.as_str()).collect();
    assert!(lang_strs.contains(&"rust"), "Should detect rust language");

    // Validate known Cargo.toml deps
    let deps = result["dependencies"].as_array().unwrap();
    let rust_runtime: Vec<&str> = deps
        .iter()
        .filter(|d| d["language"].as_str() == Some("rust"))
        .filter(|d| !d["dev"].as_bool().unwrap_or(false))
        .filter_map(|d| d["name"].as_str())
        .collect();

    let required_deps = [
        "anyhow", "chrono", "clap", "serde", "serde_json", "serde_yaml", "reqwest", "toml",
        "walkdir", "sha2", "rayon", "regex",
    ];
    for dep in &required_deps {
        assert!(
            rust_runtime.contains(dep),
            "Missing required Rust dep: {dep}"
        );
    }

    // Every dep must have all contract fields
    for dep in deps {
        assert!(
            dep.get("name").and_then(|v| v.as_str()).is_some(),
            "dep missing name"
        );
        assert!(
            dep.get("language").and_then(|v| v.as_str()).is_some(),
            "dep missing language"
        );
        assert!(dep.get("version").is_some(), "dep missing version");
        assert!(dep.get("dev").is_some(), "dep missing dev flag");
        assert!(
            dep.get("sources").and_then(|v| v.as_array()).is_some(),
            "dep missing sources array"
        );
    }

    // Manifests must include Cargo.toml
    let manifests = result["manifests"].as_array().unwrap();
    let manifest_strs: Vec<&str> = manifests.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        manifest_strs.contains(&"Cargo.toml"),
        "Should detect Cargo.toml"
    );
}

// ── crd.3.2: Splinter regression (first-run, resume, warm re-run) ──

#[test]
fn test_splinter_first_run_resume_warm() {
    let tmp = std::env::temp_dir().join(format!(
        "whetstone_splinter_test_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let project_dir = tmp.to_str().unwrap();

    // Phase 1: First run (cold)
    std::fs::write(
        tmp.join("pyproject.toml"),
        r#"
[project]
name = "splinter-test"
version = "0.1.0"
dependencies = ["requests>=2.31.0", "click>=8.0"]
"#,
    )
    .unwrap();

    let (stdout, _stderr, success) =
        run_whetstone(&["detect-deps", "--incremental"], project_dir);
    assert!(success, "First run detect-deps should succeed");
    let first_run = parse_json(&stdout);

    assert_eq!(first_run["counts"]["runtime"]["_all"], 2);
    assert_eq!(
        first_run["manifests_changed"], true,
        "First run should detect new manifests"
    );
    let inv_diff = &first_run["inventory_diff"];
    assert_eq!(
        inv_diff["added"].as_array().unwrap().len(),
        2,
        "Should add 2 deps to inventory"
    );

    // Verify state files were created
    assert!(tmp.join("whetstone/.state/inventory.json").exists());
    assert!(tmp.join("whetstone/.state/manifests.json").exists());

    // Phase 2: Resume (no changes)
    let (stdout2, _stderr2, success2) =
        run_whetstone(&["detect-deps", "--incremental"], project_dir);
    assert!(success2, "Resume detect-deps should succeed");
    let resume_run = parse_json(&stdout2);

    assert_eq!(resume_run["counts"]["runtime"]["_all"], 2);
    assert_eq!(
        resume_run["manifests_changed"], false,
        "Resume should see no manifest changes"
    );
    let inv_diff2 = &resume_run["inventory_diff"];
    assert!(
        inv_diff2["added"].as_array().unwrap().is_empty(),
        "No new deps on resume"
    );
    assert_eq!(
        inv_diff2["unchanged"].as_array().unwrap().len(),
        2,
        "Both deps should be unchanged"
    );

    // Phase 3: Modify manifest (add flask, remove click, change requests version)
    std::fs::write(
        tmp.join("pyproject.toml"),
        r#"
[project]
name = "splinter-test"
version = "0.2.0"
dependencies = ["requests>=2.32.0", "flask>=3.0"]
"#,
    )
    .unwrap();

    let (stdout3, _stderr3, success3) =
        run_whetstone(&["detect-deps", "--incremental"], project_dir);
    assert!(success3, "Warm re-run detect-deps should succeed");
    let warm_run = parse_json(&stdout3);

    assert_eq!(warm_run["counts"]["runtime"]["_all"], 2);
    assert_eq!(
        warm_run["manifests_changed"], true,
        "Should detect manifest change"
    );
    let inv_diff3 = &warm_run["inventory_diff"];

    // flask is new
    let added: Vec<&str> = inv_diff3["added"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(added.contains(&"python:flask"), "Should add flask");

    // requests version changed
    let changed: Vec<&str> = inv_diff3["changed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        changed.contains(&"python:requests"),
        "Should detect requests version change"
    );

    // click was removed from manifest
    let removed: Vec<&str> = inv_diff3["removed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        removed.contains(&"python:click"),
        "Should detect click removal"
    );

    // crd.1.2: verify click was actually removed from inventory (not just reported)
    let inv_content =
        std::fs::read_to_string(tmp.join("whetstone/.state/inventory.json")).unwrap();
    let inv: serde_json::Value = serde_json::from_str(&inv_content).unwrap();
    assert!(
        inv.get("dependencies")
            .and_then(|d| d.get("python:click"))
            .is_none(),
        "click should be removed from inventory"
    );

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── crd.2.1: Parity snapshot tests ──

#[test]
fn test_detect_deps_parity_snapshot() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["detect-deps"], dir.to_str().unwrap());
    assert!(success);
    let result = parse_json(&stdout);

    // Structural checks matching Python contract
    assert_json_has_keys(
        &result,
        &["languages", "counts", "dependencies", "manifests"],
        "detect-deps parity",
    );

    // Key value parity — same deps as Python baseline
    let dep_names: Vec<&str> = result["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|d| d["name"].as_str())
        .collect();

    let expected_runtime = ["fastapi", "next", "pydantic", "react", "requests"];
    for name in &expected_runtime {
        assert!(dep_names.contains(name), "Missing runtime dep: {name}");
    }
    let expected_dev = ["pytest", "typescript"];
    for name in &expected_dev {
        assert!(dep_names.contains(name), "Missing dev dep: {name}");
    }

    // Count parity
    assert_eq!(result["counts"]["runtime"]["_all"], 5);
    assert_eq!(result["counts"]["dev"]["_all"], 2);
    assert_eq!(result["counts"]["runtime"]["python"], 3);
    assert_eq!(result["counts"]["runtime"]["typescript"], 2);
}

#[test]
fn test_status_parity_snapshot() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &["status", "--json", "--no-snapshot", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(success);
    let result = parse_json(&stdout);

    // Structural checks matching Python contract
    assert_json_has_keys(
        &result,
        &[
            "status",
            "label",
            "score",
            "dimensions",
            "breakdown",
            "pipeline_state",
            "recommendations",
            "metrics",
            "next_command",
        ],
        "status parity",
    );

    // Fixture has 2 approved rules (fastapi + react)
    assert_eq!(result["dimensions"]["rules_count"], 2);

    let deps_covered: Vec<&str> = result["dependencies_covered"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(deps_covered.contains(&"fastapi"));
    assert!(deps_covered.contains(&"react"));

    // Metrics contract
    assert_json_has_keys(
        &result["metrics"],
        &[
            "rules_approved",
            "rules_proposed",
            "approval_rate",
            "must_rules",
            "dependencies_covered",
            "dependencies_total",
        ],
        "status metrics parity",
    );
}
