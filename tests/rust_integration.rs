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
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
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
    assert!(!ts_deps.is_empty(), "Should detect TypeScript deps from package.json");
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
    let (stdout, _stderr, success) =
        run_whetstone(&["status", "--json", "--no-snapshot"], dir.to_str().unwrap());
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
    let (stdout, _stderr, _) =
        run_whetstone(&["status", "--json", "--no-snapshot"], dir.to_str().unwrap());
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
    let (stdout, _stderr, success) =
        run_whetstone(&["status", "--json", "--no-snapshot"], dir.to_str().unwrap());
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
    let (stdout, _stderr, success) =
        run_whetstone(&["ci-check", "--json"], dir.to_str().unwrap());
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
    assert!(result["breakdown"]["signals"]["deterministic"].as_i64().unwrap() >= 2);
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
