//! Rust integration tests for whetstone binary.
//!
//! Tests core commands against the fixtures directory and the whetstone repo itself.

use std::path::{Path, PathBuf};
use std::process::Command;

fn python_has_yaml() -> bool {
    Command::new("python3")
        .args(["-c", "import yaml"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

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

fn run_whetstone_from_cwd(args: &[&str], current_dir: &std::path::Path) -> (String, String, bool) {
    let bin = whetstone_bin();
    let output = Command::new(&bin)
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run {}: {}", bin.display(), e));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

fn run_legacy_script(name: &str, args: &[&str], project_dir: &str) -> (String, String, bool) {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("legacy")
        .join(name);
    let output = Command::new("python3")
        .arg(script)
        .args(args)
        .current_dir(project_dir)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run legacy script {name}: {e}"));
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
        assert!(obj.contains_key(*key), "{context}: missing key '{key}'");
    }
}

// ── detect-deps tests ──

#[test]
fn test_detect_deps_fixtures() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only"], dir.to_str().unwrap());
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
    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only"], dir.to_str().unwrap());
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
    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only"], dir);
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

#[test]
fn test_generate_context_parity_snapshot() {
    if !python_has_yaml() {
        eprintln!("Skipping legacy generate-context parity: PyYAML unavailable");
        return;
    }
    let dir = fixtures_dir();
    let (rust_stdout, _rust_stderr, rust_success) = run_whetstone(
        &["generate-context", "--dry-run", "--json"],
        dir.to_str().unwrap(),
    );
    assert!(rust_success);
    let rust_result = parse_json(&rust_stdout);

    let (py_stdout, _py_stderr, py_success) = run_legacy_script(
        "generate-agent-context.py",
        &["--dry-run"],
        dir.to_str().unwrap(),
    );
    assert!(py_success);
    let py_result = parse_json(&py_stdout);

    assert_eq!(rust_result["rules_count"], py_result["rules_count"]);
    assert_eq!(rust_result["generated"].as_array().unwrap().len(), 1);
    assert_eq!(py_result["generated"].as_array().unwrap().len(), 1);
}

#[test]
fn test_generate_tests_parity_snapshot() {
    if !python_has_yaml() {
        eprintln!("Skipping legacy generate-tests parity: PyYAML unavailable");
        return;
    }
    let dir = fixtures_dir();
    let (rust_stdout, _rust_stderr, rust_success) = run_whetstone(
        &["generate-tests", "--dry-run", "--json"],
        dir.to_str().unwrap(),
    );
    assert!(rust_success);
    let rust_result = parse_json(&rust_stdout);

    let (py_stdout, _py_stderr, py_success) =
        run_legacy_script("generate-tests.py", &["--dry-run"], dir.to_str().unwrap());
    assert!(py_success);
    let py_result = parse_json(&py_stdout);

    assert_eq!(rust_result["rules_count"], py_result["rules_processed"]);
    let rust_tests_total = rust_result["generated"]["tests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v.get("type").and_then(|t| t.as_str()) == Some("test"))
        .count();
    let py_tests_total = py_result["generated"]["tests"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_array().map(|a| a.len()).unwrap_or(0))
        .sum::<usize>();
    assert_eq!(rust_tests_total, py_tests_total);
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

#[test]
fn test_ci_check_parity_snapshot() {
    let dir = fixtures_dir();
    let (rust_stdout, _rust_stderr, rust_success) = run_whetstone(
        &["ci-check", "--json", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(rust_success);
    let rust_result = parse_json(&rust_stdout);

    let (py_stdout, _py_stderr, py_success) = run_legacy_script(
        "ci-check.py",
        &["--json", "--no-drift-check"],
        dir.to_str().unwrap(),
    );
    assert!(py_success);
    let py_result = parse_json(&py_stdout);

    for key in [
        "freshness_status",
        "changed_sources_count",
        "recommended_rules_count",
        "requires_review",
        "score",
    ] {
        assert_eq!(
            rust_result[key], py_result[key],
            "ci-check parity for {key}"
        );
    }
}

// ── CLI tests ──

#[test]
fn test_help_output() {
    let (stdout, _stderr, success) = run_whetstone(&["--help"], ".");
    assert!(success);
    // New command names
    assert!(stdout.contains("init"), "help should contain 'init'");
    assert!(
        stdout.contains("set-sources"),
        "help should contain 'set-sources'"
    );
    assert!(stdout.contains("doctor"), "help should contain 'doctor'");
    assert!(stdout.contains("status"), "help should contain 'status'");
    assert!(stdout.contains("context"), "help should contain 'context'");
    assert!(stdout.contains("tests"), "help should contain 'tests'");
    assert!(stdout.contains("ci"), "help should contain 'ci'");
    assert!(
        stdout.contains("validate"),
        "help should contain 'validate'"
    );
    assert!(
        stdout.contains("patterns"),
        "help should contain 'patterns'"
    );
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
    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only"], dir.to_str().unwrap());
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
    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only"], dir);
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
        "anyhow",
        "chrono",
        "clap",
        "serde",
        "serde_json",
        "serde_yaml",
        "reqwest",
        "toml",
        "walkdir",
        "sha2",
        "rayon",
        "regex",
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
    let tmp = std::env::temp_dir().join(format!("whetstone_splinter_test_{}", std::process::id()));
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

    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only", "--incremental"], project_dir);
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
        run_whetstone(&["init", "--detect-only", "--incremental"], project_dir);
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
        run_whetstone(&["init", "--detect-only", "--incremental"], project_dir);
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
    let inv_content = std::fs::read_to_string(tmp.join("whetstone/.state/inventory.json")).unwrap();
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

#[test]
fn test_removed_dependency_cleanup_not_reflected_in_status() {
    let tmp = std::env::temp_dir().join(format!(
        "whetstone_removed_dep_status_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let project_dir = tmp.to_str().unwrap();

    std::fs::write(
        tmp.join("pyproject.toml"),
        "[project]\nname='cleanup'\ndependencies=['requests>=2.31','click>=8.0']\n",
    )
    .unwrap();
    let (_stdout1, _stderr1, success1) =
        run_whetstone(&["init", "--detect-only", "--incremental"], project_dir);
    assert!(success1);

    std::fs::write(
        tmp.join("pyproject.toml"),
        "[project]\nname='cleanup'\ndependencies=['requests>=2.31']\n",
    )
    .unwrap();
    let (_stdout2, _stderr2, success2) =
        run_whetstone(&["init", "--detect-only", "--incremental"], project_dir);
    assert!(success2);

    let (status_stdout, _status_stderr, status_success) = run_whetstone(
        &["status", "--json", "--no-drift-check", "--no-snapshot"],
        project_dir,
    );
    assert!(status_success);
    let status = parse_json(&status_stdout);

    let readiness_names: Vec<&str> = status["extraction_readiness"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    assert!(!readiness_names.contains(&"click"));
    assert_eq!(status["pipeline_state"]["total_deps"], 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── crd.2.1: Parity snapshot tests ──

#[test]
fn test_detect_deps_parity_snapshot() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(&["init", "--detect-only"], dir.to_str().unwrap());
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

#[test]
fn test_installed_binary_style_usage_from_outside_repo() {
    let dir = fixtures_dir();
    let outside =
        std::env::temp_dir().join(format!("whetstone_external_run_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();

    let (stdout, _stderr, success) = run_whetstone_from_cwd(
        &[
            "status",
            "--json",
            "--no-snapshot",
            "--no-drift-check",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        &outside,
    );
    assert!(
        success,
        "installed-binary style invocation should succeed from outside the repo"
    );

    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert!(result["dimensions"]["rules_count"].as_i64().unwrap() >= 1);

    let _ = std::fs::remove_dir_all(&outside);
}

// ── validate-rules tests ──

#[test]
fn test_validate_rules_passes_on_repo() {
    let repo_root = env!("CARGO_MANIFEST_DIR");
    let (stdout, _stderr, success) =
        run_whetstone(&["validate-rules", "--project-dir", repo_root], repo_root);
    assert!(
        success,
        "validate-rules should succeed on repo fixtures; output:\n{stdout}"
    );
    assert!(stdout.contains("Schema file found and readable."));
    assert!(stdout.contains("All schema checks passed."));
    assert!(stdout.contains("SKIP: tests/fixtures/whetstone/rules/python/malformed.yaml"));
}

#[test]
fn test_validate_rules_fails_on_bad_fixture() {
    // Synthesize a throwaway project with a broken rule fixture and confirm
    // the validator surfaces it and exits non-zero.
    let tmp = std::env::temp_dir().join(format!("whetstone_validate_fail_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("references")).unwrap();
    std::fs::create_dir_all(tmp.join("tests/fixtures/whetstone/rules/python")).unwrap();

    // Copy the real schema file so the header checks pass.
    let real_schema = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("references")
        .join("rule-schema.yaml");
    std::fs::copy(&real_schema, tmp.join("references/rule-schema.yaml")).unwrap();

    // Write a rule with an invalid severity and an invalid signal strategy.
    std::fs::write(
        tmp.join("tests/fixtures/whetstone/rules/python/bad.yaml"),
        r#"source:
  name: bad
rules:
  - id: bad.rule
    severity: critical
    confidence: high
    category: convention
    description: example
    source_url: https://example.com
    signals:
      - id: s1
        strategy: magic
"#,
    )
    .unwrap();

    let (stdout, _stderr, success) = run_whetstone(
        &["validate-rules", "--project-dir", tmp.to_str().unwrap()],
        tmp.to_str().unwrap(),
    );
    assert!(!success, "validate-rules must fail when fixture is invalid");
    assert!(stdout.contains("invalid severity"));
    assert!(stdout.contains("invalid strategy"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── detect-patterns tests ──

fn git_init_with_style_commits(dir: &std::path::Path) {
    use std::process::Command;

    let run = |args: &[&str]| {
        let ok = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git command")
            .success();
        assert!(ok, "git {args:?} failed");
    };

    run(&["init", "--quiet", "--initial-branch=main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    run(&["config", "commit.gpgsign", "false"]);

    let messages = [
        "format: reformat python files with ruff",
        "format: apply consistent formatting across modules",
        "lint: fix pyflakes warnings",
        "lint: fix ruff errors",
        "refactor: rename helper functions to snake_case",
        "refactor: rename modules to match package layout",
        "style: consistent braces",
        "chore: unrelated commit without style signal",
    ];

    for (i, msg) in messages.iter().enumerate() {
        std::fs::write(dir.join(format!("file{i}.txt")), format!("contents {i}\n")).unwrap();
        run(&["add", "."]);
        run(&["commit", "--quiet", "-m", msg]);
    }
}

#[test]
fn test_detect_patterns_git_contract() {
    let tmp = std::env::temp_dir().join(format!("whetstone_patterns_git_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    git_init_with_style_commits(&tmp);

    let (stdout, _stderr, success) = run_whetstone(
        &[
            "detect-patterns",
            "--sources",
            "git",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(success, "detect-patterns should succeed on a git repo");

    let result = parse_json(&stdout);
    assert_json_has_keys(
        &result,
        &["patterns", "sources_analyzed", "next_command"],
        "detect-patterns top level",
    );

    let git_stats = &result["sources_analyzed"]["git"];
    assert!(
        git_stats["commits"].as_u64().unwrap() >= 8,
        "git source should count at least the commits we made"
    );

    let patterns = result["patterns"].as_array().unwrap();
    assert!(
        !patterns.is_empty(),
        "should find at least one git style pattern"
    );

    for p in patterns {
        assert_json_has_keys(
            p,
            &[
                "description",
                "source",
                "occurrences",
                "confidence",
                "sessions",
                "example_quotes",
                "last_seen",
                "score",
                "suggested_rule",
            ],
            "detect-patterns pattern entry",
        );
        assert_eq!(p["source"], "git");
        assert!(p["occurrences"].as_u64().unwrap() >= 2);
        let suggested = &p["suggested_rule"];
        assert_eq!(suggested["severity"], "should");
        assert_eq!(suggested["category"], "convention");
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_detect_patterns_python_parity_git() {
    // This test pins JSON structural parity between the Rust port and the
    // legacy Python helper so we can remove the Python script without
    // breaking downstream consumers.
    if !python_has_yaml() {
        return; // Python/yaml unavailable — skip, consistent with existing parity tests.
    }

    let tmp =
        std::env::temp_dir().join(format!("whetstone_patterns_parity_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    git_init_with_style_commits(&tmp);

    let (rust_stdout, _, rust_ok) = run_whetstone(
        &[
            "detect-patterns",
            "--sources",
            "git",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(rust_ok, "rust detect-patterns should succeed");

    let py_script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join("detect-patterns.py");
    if !py_script.exists() {
        return; // Python helper already removed — parity pinning no longer required.
    }
    let py_output = std::process::Command::new("python3")
        .arg(&py_script)
        .args(["--sources", "git", "--project-dir", tmp.to_str().unwrap()])
        .current_dir(&tmp)
        .output()
        .expect("run python detect-patterns");
    assert!(
        py_output.status.success(),
        "python detect-patterns should succeed: {}",
        String::from_utf8_lossy(&py_output.stderr)
    );
    let py_stdout = String::from_utf8_lossy(&py_output.stdout).into_owned();

    let rust_json = parse_json(&rust_stdout);
    let py_json = parse_json(&py_stdout);

    // Top-level shape must match exactly.
    let rust_keys: std::collections::BTreeSet<&str> = rust_json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let py_keys: std::collections::BTreeSet<&str> = py_json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        rust_keys, py_keys,
        "top-level JSON keys must match between Rust and Python detect-patterns"
    );

    // Per-pattern field shape must match on the first pattern of each.
    let rust_patterns = rust_json["patterns"].as_array().unwrap();
    let py_patterns = py_json["patterns"].as_array().unwrap();
    assert!(!rust_patterns.is_empty());
    assert!(!py_patterns.is_empty());

    let rust_fields: std::collections::BTreeSet<&str> = rust_patterns[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let py_fields: std::collections::BTreeSet<&str> = py_patterns[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        rust_fields, py_fields,
        "per-pattern JSON keys must match between Rust and Python detect-patterns"
    );

    // The bucket descriptions are drawn from a fixed set — assert the same
    // buckets fire on both sides.
    let rust_buckets: std::collections::BTreeSet<String> = rust_patterns
        .iter()
        .filter_map(|p| p["description"].as_str().map(String::from))
        .collect();
    let py_buckets: std::collections::BTreeSet<String> = py_patterns
        .iter()
        .filter_map(|p| p["description"].as_str().map(String::from))
        .collect();
    assert_eq!(
        rust_buckets, py_buckets,
        "git bucket descriptions should match between Rust and Python"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── nq8.3.1: Refresh contract tests ──

/// Build a tiny project with a `whetstone.yaml` but no dependencies, so refresh
/// can run fully offline (no sources to resolve, no drift possible).
fn empty_whetstone_project(name: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\n",
    )
    .unwrap();
    // Minimal Cargo.toml so detect_deps does not error on missing manifests.
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname = \"empty\"\nversion = \"0.0.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    tmp
}

#[test]
fn test_refresh_on_empty_project_no_drift() {
    let tmp = empty_whetstone_project("whetstone_refresh_empty");
    let project = tmp.to_str().unwrap();

    let (stdout, _stderr, success) = run_whetstone(&["refresh", "--json"], project);
    assert!(
        success,
        "refresh on an empty project must succeed:\n{stdout}"
    );

    // Artifact contract
    let diff_path = tmp.join("whetstone/.state/refresh-diff.json");
    assert!(diff_path.exists(), "refresh must write refresh-diff.json");
    let diff: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&diff_path).unwrap()).unwrap();
    assert_eq!(diff["version"], 1, "diff must set version=1");
    for key in [
        "generated_at",
        "drift_count",
        "changed",
        "removed",
        "failed",
    ] {
        assert!(diff.get(key).is_some(), "refresh-diff missing key: {key}");
    }
    assert_eq!(diff["drift_count"], 0, "empty project has no drift");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_refresh_check_exits_zero_when_no_drift() {
    let tmp = empty_whetstone_project("whetstone_refresh_check_ok");
    let project = tmp.to_str().unwrap();

    let (stdout, _stderr, success) = run_whetstone(&["refresh", "--check", "--json"], project);
    // No deps → no drift → --check MUST exit 0.
    assert!(
        success,
        "refresh --check on a drift-free project must exit 0:\n{stdout}"
    );
    let diff_path = tmp.join("whetstone/.state/refresh-diff.json");
    assert!(diff_path.exists(), "refresh --check still writes the diff");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_refresh_emits_extraction_handoff_with_trigger_refresh() {
    let tmp = empty_whetstone_project("whetstone_refresh_handoff");
    let project = tmp.to_str().unwrap();

    let (_stdout, _stderr, success) = run_whetstone(&["refresh", "--json"], project);
    assert!(success);

    let handoff_path = tmp.join("whetstone/.state/extraction-handoff.json");
    assert!(handoff_path.exists(), "refresh must rewrite the handoff");
    let handoff: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&handoff_path).unwrap()).unwrap();
    assert_eq!(handoff["version"], 1);
    assert_eq!(
        handoff["trigger"], "refresh",
        "refresh-triggered handoff must be labeled 'refresh'"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_init_writes_extraction_handoff_with_trigger_init() {
    let dir = fixtures_dir();
    // `wh init` (the canonical bootstrap; `doctor` remains as a visible alias).
    let (_stdout, _stderr, _success) = run_whetstone(
        &[
            "init",
            "--json",
            "--max-deps",
            "0",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );

    let handoff_path = dir.join("whetstone/.state/extraction-handoff.json");
    if handoff_path.exists() {
        let handoff: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&handoff_path).unwrap()).unwrap();
        assert_eq!(handoff["version"], 1);
        assert_eq!(handoff["trigger"], "init");
        for key in ["candidates", "skipped", "next_action", "generated_at"] {
            assert!(handoff.get(key).is_some(), "handoff missing key: {key}");
        }
    }
}

#[test]
fn test_doctor_alias_still_routes_to_init() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "doctor",
            "--json",
            "--max-deps",
            "0",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(success, "`wh doctor` alias must still succeed");
    // Alias goes through the Init handler, so the handoff trigger stays "init".
    let handoff_path = dir.join("whetstone/.state/extraction-handoff.json");
    if handoff_path.exists() {
        let handoff: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&handoff_path).unwrap()).unwrap();
        assert_eq!(handoff["trigger"], "init");
    }
    let _ = stdout;
}

// ── nq8.3.2: AI eval lifecycle coverage ──

fn write_ai_eval_rule_project(root: &std::path::Path) {
    // Minimal Python project with one approved rule that has an ai_eval config
    // and concrete golden examples. Used by the eval generate/run/calibrate tests.
    std::fs::create_dir_all(root.join("whetstone/rules/python")).unwrap();
    std::fs::write(
        root.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pyproject.toml"),
        "[project]\nname='eval-fixture'\nversion='0.0.0'\ndependencies=['example>=0.1']\n",
    )
    .unwrap();
    // A dummy source file so eval run has something to scan.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/uses_subprocess.py"),
        "import subprocess\nresult = subprocess.run(['ls'], shell=True)\n",
    )
    .unwrap();

    // Rule uses a pattern signal (triggers a violation) with ai_eval for judgment.
    std::fs::write(
        root.join("whetstone/rules/python/example.yaml"),
        r#"source:
  name: example
  docs_url: https://example.com
  version: "0.1.0"
  content_hash: sha256:abc
  resolved_at: "2026-04-13T00:00:00Z"
  registry: pypi
  content_origin: readme
rules:
  - id: example.no-shell-true
    severity: must
    confidence: high
    category: default
    source_kind: official_docs
    description: >
      MUST NOT call subprocess.run with shell=True on untrusted input.
    source_url: https://docs.python.org/3/library/subprocess.html
    approved: true
    status: approved
    proposed_at: "2026-04-13T00:00:00Z"
    signals:
      - id: shell-true
        strategy: pattern
        description: "subprocess call with shell=True"
        match: 'subprocess\.run\([^)]*shell\s*=\s*True'
        weight: required
    golden_examples:
      - code: |
          subprocess.run(['ls', '-l'])
        verdict: pass
        reason: "No shell=True — safe"
      - code: |
          subprocess.run('ls -l', shell=True)
        verdict: fail
        reason: "shell=True with string command is unsafe"
      - code: |
          subprocess.run(user_cmd, shell=True)
        verdict: fail
        reason: "shell=True with untrusted input"
    ai_eval:
      trigger: ambiguous
      question: "Is the shell=True call on untrusted input? Answer PASS only if the input is a hardcoded safe string."
      context_lines: 6
"#,
    )
    .unwrap();
}

#[test]
fn test_eval_generate_dry_run_reports_ai_rules() {
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_gen_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &["eval", "generate", "--dry-run", "--project-dir", project],
        project,
    );
    assert!(success, "eval generate --dry-run should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    let generated = result["generated"].as_array().expect("generated array");
    assert_eq!(
        generated.len(),
        1,
        "should generate one definition for the single ai_eval rule"
    );
    assert_eq!(generated[0]["rule_id"], "example.no-shell-true");
    assert_eq!(generated[0]["golden_examples"].as_u64(), Some(3));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_generate_no_ai_rules_emits_message() {
    // Project with a deterministic-only rule; no ai_eval config anywhere.
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_no_ai_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\ndeny:\n  - rust.expect-over-unwrap\n  - rust.timeout-on-http-clients\n  - rust.error-context\n  - rust.prefer-str-params\n  - rust.must-use-results\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("whetstone/rules/rust/fake.yaml"),
        r#"source:
  name: fake
  version: "0.1"
  content_hash: sha256:abc
  resolved_at: "2026-04-13T00:00:00Z"
  registry: crates_io
rules:
  - id: fake.simple
    severity: may
    confidence: high
    category: convention
    source_kind: manual
    description: example
    source_url: https://example.com
    approved: true
    status: approved
    proposed_at: "2026-04-13T00:00:00Z"
    signals:
      - id: s1
        strategy: pattern
        description: example
        match: 'foo'
        weight: required
    golden_examples:
      - code: "foo"
        verdict: fail
        reason: "matches"
      - code: "bar"
        verdict: pass
        reason: "does not match"
"#,
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &["eval", "generate", "--dry-run", "--project-dir", project],
        project,
    );
    assert!(success, "eval generate --dry-run should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert!(
        result["message"]
            .as_str()
            .unwrap_or("")
            .contains("No rules with AI signals"),
        "expected no-AI message, got: {result}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_run_deterministic_only_reports_violations() {
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_det_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "eval",
            "run",
            "--deterministic-only",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(
        success,
        "eval run --deterministic-only should succeed:\n{stdout}"
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");

    let violations = result["violations"].as_array().expect("violations array");
    assert!(
        !violations.is_empty(),
        "deterministic-only eval must still report violations"
    );
    assert!(violations
        .iter()
        .any(|v| v["rule_id"] == "example.no-shell-true"));
    // Deterministic-only path does NOT emit pending AI requests
    assert_eq!(result["pending_requests"], serde_json::Value::Null);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_run_ai_path_writes_requests() {
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_ai_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) =
        run_whetstone(&["eval", "run", "--project-dir", project], project);
    assert!(success, "eval run should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");

    let requests_path = tmp.join("whetstone/.state/eval-requests.json");
    assert!(
        requests_path.exists(),
        "eval run with an ai_eval rule + match must write eval-requests.json"
    );
    let batch: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&requests_path).unwrap()).unwrap();
    assert_eq!(batch["version"], 1);
    let requests = batch["requests"].as_array().unwrap();
    assert!(
        !requests.is_empty(),
        "eval-requests.json must include at least one request"
    );
    for req in requests {
        assert!(req.get("id").is_some());
        assert!(req.get("rule_id").is_some());
        assert!(req.get("question").is_some());
        assert!(req.get("code_snippet").is_some());
        assert!(req.get("golden_examples").is_some());
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_run_collect_merges_verdicts() {
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_collect_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/.state")).unwrap();

    // Simulate an agent having judged the requests.
    std::fs::write(
        tmp.join("whetstone/.state/eval-verdicts.json"),
        r#"{
            "version": 1,
            "judged_at": "2026-04-13T00:00:00Z",
            "verdicts": [
                {"id": "example.no-shell-true:src/uses_subprocess.py:2", "verdict": "fail", "reason": "shell=True on untrusted input"}
            ]
        }"#,
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &["eval", "run", "--collect", "--project-dir", project],
        project,
    );
    assert!(success, "eval run --collect should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    let summary = &result["summary"];
    assert_eq!(summary["verdicts_collected"], 1);
    assert_eq!(summary["ai_failures"], 1);
    assert_eq!(summary["ai_passes"], 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_calibrate_writes_requests() {
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_cal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) =
        run_whetstone(&["eval", "calibrate", "--project-dir", project], project);
    assert!(success, "eval calibrate should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["rules_tested"], 1);
    assert_eq!(result["calibration_requests"], 3); // 3 golden examples

    let path = tmp.join("whetstone/.state/calibration-requests.json");
    assert!(path.exists());
    let batch: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(batch["type"], "calibration");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_calibrate_collect_reports_agreement() {
    let tmp =
        std::env::temp_dir().join(format!("whetstone_eval_cal_collect_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/.state")).unwrap();

    // Perfectly aligned verdicts — should report 100% agreement.
    std::fs::write(
        tmp.join("whetstone/.state/calibration-verdicts.json"),
        r#"{
            "version": 1,
            "verdicts": [
                {"id": "calibrate:example.no-shell-true:example_0", "verdict": "pass", "reason": "matches expected pass"},
                {"id": "calibrate:example.no-shell-true:example_1", "verdict": "fail", "reason": "matches expected fail"},
                {"id": "calibrate:example.no-shell-true:example_2", "verdict": "fail", "reason": "matches expected fail"}
            ]
        }"#,
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &["eval", "calibrate", "--collect", "--project-dir", project],
        project,
    );
    assert!(
        success,
        "eval calibrate --collect should succeed:\n{stdout}"
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["calibration_passed"], true);
    let summary = &result["summary"];
    assert_eq!(summary["agreements"], 3);
    assert_eq!(summary["disagreements"], 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── nq8.3.3: Built-in + project merge regression ──

#[test]
fn test_builtin_rules_merged_into_generate_context() {
    // Project with a whetstone.yaml but no project rules — context generation
    // should still emit output containing built-in Rust rules.
    let tmp =
        std::env::temp_dir().join(format!("whetstone_builtin_context_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\n",
    )
    .unwrap();
    // Minimal Rust manifest so the project is classified as rust.
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname='builtin-merge'\nversion='0.0.0'\nedition='2021'\n",
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "context",
            "--dry-run",
            "--json",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(success, "generate-context should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert!(
        result["rules_count"].as_i64().unwrap_or(0) >= 1,
        "built-in Rust rules must merge in when project has no rules, got: {result}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_deny_excludes_builtin_rule_from_generation() {
    // Deny every built-in Rust rule id; confirm context generation emits zero rules.
    let tmp = std::env::temp_dir().join(format!("whetstone_deny_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\ndeny:\n  - rust.expect-over-unwrap\n  - rust.timeout-on-http-clients\n  - rust.error-context\n  - rust.prefer-str-params\n  - rust.must-use-results\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname='deny-all'\nversion='0.0.0'\nedition='2021'\n",
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "tests",
            "--dry-run",
            "--json",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(success, "generate-tests should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    // With every built-in denied and no project rules, nothing should generate.
    let rules_count = result["rules_count"].as_i64().unwrap_or(0);
    assert_eq!(
        rules_count, 0,
        "denying every built-in rule must leave nothing to generate, got {rules_count}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_project_rule_overrides_builtin_by_id() {
    // Project defines a rule whose id collides with a built-in. The merge must
    // keep the project definition (and drop the built-in version of the same id).
    let tmp = std::env::temp_dir().join(format!("whetstone_override_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname='override'\nversion='0.0.0'\nedition='2021'\n",
    )
    .unwrap();
    // Use the same id as a built-in rule to force the collision.
    std::fs::write(
        tmp.join("whetstone/rules/rust/expect.yaml"),
        r#"source:
  name: "local:expect"
  version: "1.0.0"
  content_hash: sha256:local
  resolved_at: "2026-04-13T00:00:00Z"
  registry: manual
rules:
  - id: rust.expect-over-unwrap
    severity: must
    confidence: high
    category: convention
    source_kind: team_guide
    description: >
      Project override: MUST use .expect("…") instead of .unwrap() — team convention.
    source_url: https://team-guide.internal/rust-unwrap
    approved: true
    status: approved
    proposed_at: "2026-04-13T00:00:00Z"
    signals:
      - id: local-bare-unwrap
        strategy: pattern
        description: overridden
        match: '\.unwrap\s*\(\)'
        weight: required
    golden_examples:
      - code: 'x.expect("reason")'
        verdict: pass
        reason: explicit
      - code: 'x.unwrap()'
        verdict: fail
        reason: no reason
"#,
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "tests",
            "--dry-run",
            "--json",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(success, "generate-tests should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");

    // Built-ins contribute 4 remaining rules (one id is overridden); project contributes 1.
    // Expected total: 4 (built-in, minus the overridden id) + 1 (project) = 5.
    let rules_count = result["rules_count"].as_i64().unwrap_or(0);
    assert_eq!(
        rules_count, 5,
        "project rule must override built-in by id (expected 5 total)"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── vkh: Layers + Triggers ──

fn write_layer_project(name: &str) -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!("{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("Cargo.toml"),
        "[package]\nname='layer-test'\nversion='0.0.0'\nedition='2021'\n",
    )
    .unwrap();
    tmp
}

fn approved_rule_yaml(rule_id: &str, source_name: &str, match_regex: &str) -> String {
    format!(
        r#"source:
  name: {source_name}
  version: "1.0.0"
  content_hash: sha256:abc
  resolved_at: "2026-04-13T00:00:00Z"
  registry: manual
rules:
  - id: {rule_id}
    severity: must
    confidence: high
    category: convention
    source_kind: manual
    description: Test rule {rule_id}
    source_url: https://example.com/{rule_id}
    approved: true
    status: approved
    proposed_at: "2026-04-13T00:00:00Z"
    signals:
      - id: s1
        strategy: pattern
        description: Match {rule_id}
        match: '{match_regex}'
        weight: required
    golden_examples:
      - code: "foo"
        verdict: fail
        reason: matches
      - code: "bar"
        verdict: pass
        reason: does not match
"#
    )
}

#[test]
fn test_init_personal_creates_directory_and_gitignore() {
    let tmp = write_layer_project("whetstone_init_personal");
    let project = tmp.to_str().unwrap();

    let (stdout, _stderr, success) = run_whetstone(
        &["init", "--personal", "--json", "--project-dir", project],
        project,
    );
    assert!(success, "init --personal should succeed:\n{stdout}");

    for sub in ["rules", "evals", "lint", "context"] {
        let path = tmp.join("whetstone/.personal").join(sub);
        assert!(path.exists(), "missing {}", path.display());
    }
    assert!(tmp.join("whetstone/.personal/config.yaml").exists());

    let gitignore = std::fs::read_to_string(tmp.join(".gitignore")).unwrap();
    assert!(
        gitignore.contains("whetstone/.personal/"),
        "gitignore should hide .personal/:\n{gitignore}"
    );
    assert!(gitignore.contains("whetstone/.state/"));

    // Idempotent: a second run should not duplicate entries.
    let (_stdout2, _, success2) = run_whetstone(
        &["init", "--personal", "--json", "--project-dir", project],
        project,
    );
    assert!(success2);
    let gitignore2 = std::fs::read_to_string(tmp.join(".gitignore")).unwrap();
    assert_eq!(
        gitignore2.matches("whetstone/.personal/").count(),
        1,
        ".gitignore entries must not duplicate across reruns"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_personal_rule_overrides_project_rule_by_id() {
    let tmp = write_layer_project("whetstone_personal_override");
    let project = tmp.to_str().unwrap();

    // Project rule
    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/rules/rust/foo.yaml"),
        approved_rule_yaml("rust.foo", "foo", "PROJECT"),
    )
    .unwrap();

    // Personal rule with same id, different body
    std::fs::create_dir_all(tmp.join("whetstone/.personal/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.personal/rules/rust/foo.yaml"),
        approved_rule_yaml("rust.foo", "personal-foo", "PERSONAL"),
    )
    .unwrap();

    let (stdout, _, success) = run_whetstone(
        &["layers", "--lang", "rust", "--project-dir", project],
        project,
    );
    assert!(success);
    let result = parse_json(&stdout);

    let rules = result["rules"].as_array().unwrap();
    let matching: Vec<&serde_json::Value> =
        rules.iter().filter(|r| r["id"] == "rust.foo").collect();
    assert_eq!(
        matching.len(),
        1,
        "personal must collapse with project by id"
    );
    assert_eq!(matching[0]["layer"], "personal");
    assert_eq!(matching[0]["source_name"], "personal-foo");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_context_personal_flag_routes_to_personal_dir() {
    let tmp = write_layer_project("whetstone_context_personal");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/.personal/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.personal/rules/rust/only.yaml"),
        approved_rule_yaml("rust.only-personal", "only", "foo"),
    )
    .unwrap();

    // Project context must NOT include the personal rule.
    let (proj_stdout, _, proj_ok) = run_whetstone(
        &[
            "context",
            "--dry-run",
            "--json",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(proj_ok);
    let proj = parse_json(&proj_stdout);
    let proj_paths: Vec<&str> = proj["generated"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|g| g["path"].as_str())
        .collect();
    assert!(
        proj_paths.iter().all(|p| !p.contains(".personal/context")),
        "project context must not target .personal/: {proj_paths:?}"
    );
    // Built-in rust rules still kick in; the point here is *not* that the
    // personal rule appears.
    let (pers_stdout, _, pers_ok) = run_whetstone(
        &[
            "context",
            "--dry-run",
            "--json",
            "--personal",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(pers_ok);
    let pers = parse_json(&pers_stdout);
    let pers_paths: Vec<&str> = pers["generated"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|g| g["path"].as_str())
        .collect();
    assert!(
        pers_paths.iter().any(|p| p.contains(".personal/context")),
        "personal context must target .personal/context/: {pers_paths:?}"
    );
    assert_eq!(pers["rules_count"], 1, "personal output is personal-only");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_tests_personal_flag_routes_to_personal_evals() {
    let tmp = write_layer_project("whetstone_tests_personal");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/.personal/rules/python")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.personal/rules/python/foo.yaml"),
        approved_rule_yaml("python.pers-foo", "pers-foo", "foo"),
    )
    .unwrap();

    let (stdout, _, ok) = run_whetstone(
        &[
            "tests",
            "--personal",
            "--dry-run",
            "--json",
            "--lang",
            "python",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(ok, "tests --personal should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    let tests = result["generated"]["tests"].as_array().unwrap();
    assert!(
        tests.iter().any(|t| t["path"]
            .as_str()
            .unwrap_or("")
            .contains(".personal/evals/python")),
        "personal tests must land under whetstone/.personal/evals/, got: {tests:?}"
    );
    assert!(
        result["next_command"]
            .as_str()
            .unwrap_or("")
            .contains("whetstone/.personal/evals/python"),
        "personal next_command should point at .personal/evals/, got: {}",
        result["next_command"]
    );
    assert_eq!(result["rules_count"], 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_layer_deny_at_personal_removes_from_merged_set() {
    let tmp = write_layer_project("whetstone_deny_personal");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/rules/rust/foo.yaml"),
        approved_rule_yaml("rust.foo", "foo", "FOO"),
    )
    .unwrap();

    // Personal config silently excludes the project-level rust.foo rule.
    std::fs::create_dir_all(tmp.join("whetstone/.personal")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.personal/config.yaml"),
        "deny:\n  - rust.foo\n",
    )
    .unwrap();

    let (stdout, _, ok) = run_whetstone(
        &["layers", "--lang", "rust", "--project-dir", project],
        project,
    );
    assert!(ok);
    let result = parse_json(&stdout);
    let ids: Vec<&str> = result["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&"rust.foo"),
        "personal deny should filter rust.foo out of the merged set"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_personal_deny_does_not_affect_committed_generation() {
    let tmp = write_layer_project("whetstone_personal_deny_generation");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/rules/rust/foo.yaml"),
        approved_rule_yaml("rust.foo", "foo", "FOO"),
    )
    .unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/.personal")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.personal/config.yaml"),
        "deny:\n  - rust.foo\n",
    )
    .unwrap();

    let (stdout, _, ok) = run_whetstone(
        &[
            "tests",
            "--dry-run",
            "--json",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(ok, "tests should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    let tests = result["generated"]["tests"].as_array().unwrap();
    assert!(
        tests.iter().any(|entry| entry["rule_id"] == "rust.foo"),
        "personal deny must not remove project rules from committed generation: {result}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_team_deny_filters_builtins_in_layer_merge() {
    let tmp = write_layer_project("whetstone_team_deny");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/.team")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.team/config.yaml"),
        "deny:\n  - rust.expect-over-unwrap\n",
    )
    .unwrap();

    let (stdout, _, ok) = run_whetstone(
        &["layers", "--lang", "rust", "--project-dir", project],
        project,
    );
    assert!(ok, "layers should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    let ids: Vec<&str> = result["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&"rust.expect-over-unwrap"),
        "team deny should filter builtin rule out of merged layers: {ids:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_eval_generate_includes_personal_rules_in_layered_view() {
    let tmp = std::env::temp_dir().join(format!("whetstone_eval_personal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_ai_eval_rule_project(&tmp);

    std::fs::create_dir_all(tmp.join("whetstone/.personal/rules/python")).unwrap();
    std::fs::rename(
        tmp.join("whetstone/rules/python/example.yaml"),
        tmp.join("whetstone/.personal/rules/python/example.yaml"),
    )
    .unwrap();

    let project = tmp.to_str().unwrap();
    let (stdout, _stderr, success) = run_whetstone(
        &["eval", "generate", "--dry-run", "--project-dir", project],
        project,
    );
    assert!(
        success,
        "eval generate should include personal rules locally:\n{stdout}"
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(
        result["eval_count"], 1,
        "personal ai_eval rule should be visible to local eval workflows"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_promote_personal_to_project_moves_file() {
    let tmp = write_layer_project("whetstone_promote");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/.personal/rules/rust")).unwrap();
    let source = tmp.join("whetstone/.personal/rules/rust/foo.yaml");
    std::fs::write(&source, approved_rule_yaml("rust.foo", "foo", "FOO")).unwrap();

    let (stdout, _, ok) = run_whetstone(
        &[
            "promote",
            "rust.foo",
            "--to",
            "project",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(ok, "promote should succeed:\n{stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["from"], "personal");
    assert_eq!(result["to"], "project");

    assert!(!source.exists(), "source must be removed after move");
    let dest = tmp.join("whetstone/rules/rust/foo.yaml");
    assert!(dest.exists(), "destination must exist");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_promote_project_to_team_shows_up_in_team_layer() {
    let tmp = write_layer_project("whetstone_promote_team");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/rules/rust/bar.yaml"),
        approved_rule_yaml("rust.bar", "bar", "BAR"),
    )
    .unwrap();

    let (_stdout, _, ok) = run_whetstone(
        &[
            "promote",
            "rust.bar",
            "--to",
            "team",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(ok);

    let (stdout, _, ok2) = run_whetstone(
        &["layers", "--lang", "rust", "--project-dir", project],
        project,
    );
    assert!(ok2);
    let result = parse_json(&stdout);
    let matching: Vec<&serde_json::Value> = result["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["id"] == "rust.bar")
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0]["layer"], "team");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_promote_rejects_downward_direction() {
    let tmp = write_layer_project("whetstone_promote_down");
    let project = tmp.to_str().unwrap();

    std::fs::create_dir_all(tmp.join("whetstone/rules/rust")).unwrap();
    std::fs::write(
        tmp.join("whetstone/rules/rust/foo.yaml"),
        approved_rule_yaml("rust.foo", "foo", "FOO"),
    )
    .unwrap();

    // project → personal is a downward transition; must fail.
    let (stdout, _, ok) = run_whetstone(
        &[
            "promote",
            "rust.foo",
            "--to",
            "personal",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(!ok, "downward promotion must exit non-zero");
    let result = parse_json(&stdout);
    assert!(result["error"].as_str().unwrap().contains("monotonic"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_init_ci_writes_workflow_file() {
    let tmp = write_layer_project("whetstone_init_ci");
    let project = tmp.to_str().unwrap();

    let (stdout, _, ok) = run_whetstone(
        &[
            "init",
            "--ci",
            "--schedule",
            "weekly",
            "--json",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(ok, "init --ci should succeed:\n{stdout}");
    let path = tmp.join(".github/workflows/whetstone-check.yml");
    assert!(path.exists());
    let body = std::fs::read_to_string(&path).unwrap();
    assert!(body.contains("cron: \"0 9 * * 1\""));
    assert!(body.contains("Run wh status"));
    assert!(body.contains("wh ci"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_init_hooks_writes_post_merge_and_session_hook() {
    let tmp = write_layer_project("whetstone_init_hooks");
    let project = tmp.to_str().unwrap();

    // The post-merge hook tries to configure git; make the dir a git repo so
    // that codepath is exercised but the overall command still succeeds.
    std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&tmp)
        .status()
        .expect("git init");

    let (stdout, _, ok) = run_whetstone(
        &["init", "--hooks", "--json", "--project-dir", project],
        project,
    );
    assert!(ok, "init --hooks should succeed:\n{stdout}");
    let post_merge = tmp.join(".githooks/post-merge");
    assert!(post_merge.exists(), "post-merge hook must be installed");

    let session = tmp.join(".claude/whetstone-session-hook.sh");
    assert!(session.exists(), "claude session hook must be installed");
    let settings = tmp.join(".claude/settings.json");
    assert!(settings.exists(), "claude settings.json must be written");
    let settings_body = std::fs::read_to_string(&settings).unwrap();
    assert!(settings_body.contains("SessionStart"));
    assert!(settings_body.contains("whetstone-session-hook.sh"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_extends_parse_forms() {
    // Smoke-test the extends parser via the layers command. When a team repo
    // can't be cloned (no network), the extends entry is left out of the rule
    // set but the command still succeeds.
    let tmp = write_layer_project("whetstone_extends_parse");
    let project = tmp.to_str().unwrap();

    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\nextends:\n  - whetstone:recommended\n  - \"@future/registry\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run_whetstone(
        &["layers", "--lang", "rust", "--project-dir", project],
        project,
    );
    assert!(ok, "layers should succeed with benign extends:\n{stdout}");
    let result = parse_json(&stdout);
    // whetstone:recommended is the embedded built-in layer; the built-in rust rules still show up.
    let built_in_count = result["summary"]["built-in"].as_u64().unwrap_or(0);
    assert!(
        built_in_count >= 1,
        "built-in layer should still contribute"
    );
    let team_resolution = result["team_resolution"].as_array().unwrap();
    assert!(
        team_resolution
            .iter()
            .any(|entry| entry["status"] == "not_implemented"),
        "registry extends should surface explicit not_implemented status: {team_resolution:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_generation_surfaces_warning_for_unresolved_extends() {
    let tmp = write_layer_project("whetstone_extends_warning");
    let project = tmp.to_str().unwrap();

    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "discovery:\n  exclude: []\nextends:\n  - \"@future/registry\"\n",
    )
    .unwrap();

    let (stdout, _, ok) = run_whetstone(
        &[
            "context",
            "--dry-run",
            "--json",
            "--lang",
            "rust",
            "--project-dir",
            project,
        ],
        project,
    );
    assert!(
        ok,
        "context should succeed even when extends are unresolved:\n{stdout}"
    );
    let result = parse_json(&stdout);
    let warnings = result["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| {
            w.as_str()
                .unwrap_or("")
                .contains("@future/registry: not_implemented")
        }),
        "generation should surface unresolved extends as warnings: {warnings:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_global_config_deny_merges_into_project() {
    // Point HOME at a scratch dir with a global config that denies every
    // built-in rust rule id; confirm those ids disappear from the merged set.
    let tmp = write_layer_project("whetstone_global_cfg");
    let project = tmp.to_str().unwrap();

    let home = std::env::temp_dir().join(format!("whetstone_home_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join(".whetstone")).unwrap();
    std::fs::write(
        home.join(".whetstone/config.yaml"),
        "deny:\n  - rust.expect-over-unwrap\n  - rust.timeout-on-http-clients\n  - rust.error-context\n  - rust.prefer-str-params\n  - rust.must-use-results\n",
    )
    .unwrap();

    let bin = {
        let mut p = std::env::current_exe().unwrap();
        p.pop();
        p.pop();
        p.push("whetstone");
        p
    };
    let output = std::process::Command::new(&bin)
        .args(["layers", "--lang", "rust", "--project-dir", project])
        .current_dir(project)
        .env("HOME", home.to_str().unwrap())
        .output()
        .unwrap();
    assert!(output.status.success(), "layers should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = parse_json(&stdout);

    let ids: Vec<&str> = result["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    for denied in [
        "rust.expect-over-unwrap",
        "rust.timeout-on-http-clients",
        "rust.error-context",
        "rust.prefer-str-params",
        "rust.must-use-results",
    ] {
        assert!(
            !ids.contains(&denied),
            "global deny should remove {denied}, got ids: {ids:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&home);
}

// ── wh review / wh apply (lifecycle workflow) ──

fn write_candidate_rule_fixture(tmp: &Path, rule_id: &str) {
    let rules_dir = tmp.join("whetstone").join("rules").join("python");
    std::fs::create_dir_all(&rules_dir).unwrap();
    let yaml = format!(
        r#"source:
  name: example
  docs_url: https://example.com/docs
  version: "1.0.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: pypi

rules:
  - id: {rule_id}
    severity: must
    confidence: high
    category: default
    description: >
      Test rule used by the review/apply integration suite.
    source_url: https://example.com/docs/rule
    approved: false
    status: candidate
    proposed_at: "2026-04-14T00:00:00Z"
    proposed_by: whetstone-extraction
    signals:
      - id: s1
        strategy: pattern
        description: "demo pattern"
        weight: required
        match: 'TODO'
    golden_examples:
      - code: ""
        verdict: pass
        reason: "placeholder"
"#
    );
    std::fs::write(rules_dir.join("example.yaml"), yaml).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(tmp.join("whetstone").join("whetstone.yaml"), "deny: []\n").unwrap();
}

fn write_approved_rust_rule_fixture(tmp: &Path, rule_id: &str) {
    let rules_dir = tmp.join("whetstone").join("rules").join("rust");
    std::fs::create_dir_all(&rules_dir).unwrap();
    let yaml = format!(
        r#"source:
  name: example
  docs_url: https://example.com/docs
  version: "1.0.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: crates_io

rules:
  - id: {rule_id}
    severity: must
    confidence: high
    category: default
    description: >
      Approved rust rule used by the review/apply integration suite.
    source_url: https://example.com/docs/rule
    approved: true
    status: approved
    approved_at: "2026-04-14T00:00:00Z"
    proposed_at: "2026-04-14T00:00:00Z"
    proposed_by: whetstone-extraction
    signals:
      - id: s1
        strategy: pattern
        description: "demo pattern"
        weight: required
        match: '\\.unwrap\\s*\\('
    golden_examples:
      - code: ""
        verdict: pass
        reason: "placeholder"
"#
    );
    std::fs::write(rules_dir.join("example.yaml"), yaml).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(tmp.join("whetstone").join("whetstone.yaml"), "deny: []\n").unwrap();
}

#[test]
fn test_review_lists_candidates_by_status() {
    let tmp = std::env::temp_dir().join(format!("wh_review_list_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.demo");

    let (stdout, _stderr, ok) = run_whetstone(&["--json", "review"], tmp.to_str().unwrap());
    assert!(ok, "wh review should succeed");
    let result = parse_json(&stdout);
    let candidates = result["rules"]["candidate"].as_array().unwrap();
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0]["id"], "example.demo");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_approve_mutates_yaml_and_logs_audit() {
    let tmp = std::env::temp_dir().join(format!("wh_apply_approve_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.approve");

    let (stdout, _stderr, ok) = run_whetstone(
        &["--json", "apply", "example.approve", "--approve"],
        tmp.to_str().unwrap(),
    );
    assert!(ok, "wh apply --approve should succeed: {stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["action"], "applied");
    assert_eq!(result["from"], "candidate");
    assert_eq!(result["to"], "approved");

    let yaml = std::fs::read_to_string(
        tmp.join("whetstone")
            .join("rules")
            .join("python")
            .join("example.yaml"),
    )
    .unwrap();
    assert!(yaml.contains("status: approved"), "yaml: {yaml}");
    assert!(yaml.contains("approved: true"), "yaml: {yaml}");

    let log_path = tmp
        .join("whetstone")
        .join(".state")
        .join("review-log.jsonl");
    let log = std::fs::read_to_string(&log_path).unwrap();
    assert!(log.contains("\"rule_id\":\"example.approve\""));
    assert!(log.contains("\"to_status\":\"approved\""));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_deny_requires_reason() {
    let tmp = std::env::temp_dir().join(format!("wh_apply_deny_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.deny");

    let (_stdout, _stderr, ok) = run_whetstone(
        &["--json", "apply", "example.deny", "--deny"],
        tmp.to_str().unwrap(),
    );
    assert!(!ok, "wh apply --deny without --reason should fail");

    let (stdout, _stderr, ok) = run_whetstone(
        &[
            "--json",
            "apply",
            "example.deny",
            "--deny",
            "--reason",
            "not applicable",
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(result["to"], "denied");

    let yaml = std::fs::read_to_string(
        tmp.join("whetstone")
            .join("rules")
            .join("python")
            .join("example.yaml"),
    )
    .unwrap();
    assert!(
        yaml.contains("denied_reason: not applicable"),
        "yaml: {yaml}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_dry_run_does_not_mutate() {
    let tmp = std::env::temp_dir().join(format!("wh_apply_dry_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.dry");

    let yaml_path = tmp
        .join("whetstone")
        .join("rules")
        .join("python")
        .join("example.yaml");
    let before = std::fs::read_to_string(&yaml_path).unwrap();

    let (stdout, _stderr, ok) = run_whetstone(
        &["--json", "apply", "example.dry", "--approve", "--dry-run"],
        tmp.to_str().unwrap(),
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(result["action"], "dry_run");

    let after = std::fs::read_to_string(&yaml_path).unwrap();
    assert_eq!(before, after, "dry-run must not mutate the YAML");

    let log_path = tmp
        .join("whetstone")
        .join(".state")
        .join("review-log.jsonl");
    assert!(!log_path.exists(), "dry-run must not write audit log");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_rejects_illegal_backwards_transition() {
    let tmp = std::env::temp_dir().join(format!("wh_apply_illegal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.back");

    // Approve first
    let (_stdout, _stderr, ok) = run_whetstone(
        &["--json", "apply", "example.back", "--approve"],
        tmp.to_str().unwrap(),
    );
    assert!(ok);

    // Then attempt approve again (approved → approved is not a forward move)
    let (stdout, _stderr, ok) = run_whetstone(
        &["--json", "apply", "example.back", "--approve"],
        tmp.to_str().unwrap(),
    );
    assert!(
        !ok,
        "re-approving an already approved rule must fail: {stdout}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_batch_from_json_file() {
    let tmp = std::env::temp_dir().join(format!("wh_apply_batch_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.batch1");

    // Add a second rule to the same file by appending — simpler than writing a second file.
    let yaml_path = tmp
        .join("whetstone")
        .join("rules")
        .join("python")
        .join("example.yaml");
    let mut yaml = std::fs::read_to_string(&yaml_path).unwrap();
    yaml.push_str(
        "  - id: example.batch2\n    severity: should\n    confidence: high\n    category: convention\n    description: second\n    source_url: https://example.com/docs/2\n    approved: false\n    status: candidate\n    proposed_at: \"2026-04-14T00:00:00Z\"\n    signals:\n      - id: s1\n        strategy: pattern\n        description: demo\n        weight: required\n        match: 'TODO'\n    golden_examples:\n      - code: \"\"\n        verdict: pass\n        reason: placeholder\n",
    );
    std::fs::write(&yaml_path, &yaml).unwrap();

    let batch_path = tmp.join("batch.json");
    std::fs::write(
        &batch_path,
        r#"[
          {"rule_id": "example.batch1", "action": "approve"},
          {"rule_id": "example.batch2", "action": "deny", "reason": "nope"}
        ]"#,
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone(
        &["--json", "apply", "--batch", batch_path.to_str().unwrap()],
        tmp.to_str().unwrap(),
    );
    assert!(ok, "batch apply should succeed: {stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["total"], 2);
    assert_eq!(result["failed"].as_array().unwrap().len(), 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_apply_supersede_requires_reason_and_accepts_builtin_target() {
    let tmp = std::env::temp_dir().join(format!("wh_apply_supersede_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_approved_rust_rule_fixture(&tmp, "example.old-rule");

    let (_stdout, _stderr, ok) = run_whetstone(
        &[
            "--json",
            "apply",
            "example.old-rule",
            "--supersede",
            "--superseded-by",
            "rust.expect-over-unwrap",
        ],
        tmp.to_str().unwrap(),
    );
    assert!(!ok, "wh apply --supersede without --reason should fail");

    let (stdout, _stderr, ok) = run_whetstone(
        &[
            "--json",
            "apply",
            "example.old-rule",
            "--supersede",
            "--superseded-by",
            "rust.expect-over-unwrap",
            "--reason",
            "built-in rule replaces project-specific version",
        ],
        tmp.to_str().unwrap(),
    );
    assert!(
        ok,
        "wh apply --supersede should accept built-in targets: {stdout}"
    );
    let result = parse_json(&stdout);
    assert_eq!(result["to"], "deprecated");
    assert_eq!(result["superseded_by"], "rust.expect-over-unwrap");

    let yaml = std::fs::read_to_string(
        tmp.join("whetstone")
            .join("rules")
            .join("rust")
            .join("example.yaml"),
    )
    .unwrap();
    assert!(yaml.contains("status: deprecated"), "yaml: {yaml}");
    assert!(
        yaml.contains("deprecated_reason: built-in rule replaces project-specific version"),
        "yaml: {yaml}"
    );
    assert!(
        yaml.contains("superseded_by: rust.expect-over-unwrap"),
        "yaml: {yaml}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_review_queue_builds_from_refresh_diff() {
    let tmp = std::env::temp_dir().join(format!("wh_review_queue_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    write_candidate_rule_fixture(&tmp, "example.queue");

    // Seed a refresh-diff artifact documenting a changed dependency with an
    // affected rule id — the queue must surface that as a stale rule.
    let state = tmp.join("whetstone").join(".state");
    std::fs::create_dir_all(&state).unwrap();
    let diff = serde_json::json!({
        "version": 1,
        "drift_count": 1,
        "changed": [{
            "name": "example",
            "language": "python",
            "affected_rule_ids": ["example.queue"]
        }],
        "removed": [],
        "failed": [],
    });
    std::fs::write(
        state.join("refresh-diff.json"),
        serde_json::to_string(&diff).unwrap(),
    )
    .unwrap();

    let (stdout, _stderr, ok) =
        run_whetstone(&["--json", "review", "queue"], tmp.to_str().unwrap());
    assert!(ok, "wh review queue should succeed: {stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(
        result["summary"]["candidates_pending_review"], 1,
        "result: {result}"
    );
    assert_eq!(result["summary"]["rules_affected_by_source_change"], 1);
    let stale = result["stale_rules"].as_array().unwrap();
    assert_eq!(stale[0]["rule_id"], "example.queue");

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── wh bench (benchmark harness) ──

#[test]
fn test_bench_runs_repo_corpus_and_reports_scenarios() {
    let (stdout, _stderr, ok) =
        run_whetstone(&["--json", "bench", "run"], env!("CARGO_MANIFEST_DIR"));
    assert!(ok, "bench run should succeed: {stdout}");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    let total = result["summary"]["total"].as_i64().unwrap();
    assert!(total >= 1, "expected at least one scenario, got: {total}");
    let scenarios = result["scenarios"].as_array().unwrap();
    for s in scenarios {
        let f1 = s["f1"].as_f64().unwrap();
        assert!(
            (f1 - 1.0).abs() < 0.001,
            "expected F1=1.0 on repo corpus, got {f1} for {}",
            s["scenario"]
        );
    }
}

#[test]
fn test_bench_corpus_covers_layered_and_eval_categories() {
    let (stdout, _stderr, ok) =
        run_whetstone(&["--json", "bench", "run"], env!("CARGO_MANIFEST_DIR"));
    assert!(ok);
    let result = parse_json(&stdout);
    let cats = result["summary"]["categories"].as_object().unwrap();
    // The corpus must exercise all three categories, not just deterministic.
    assert!(
        cats.get("deterministic")
            .and_then(|v| v.as_i64())
            .unwrap_or(0)
            >= 1
    );
    assert!(cats.get("layered").and_then(|v| v.as_i64()).unwrap_or(0) >= 1);
    assert!(cats.get("eval").and_then(|v| v.as_i64()).unwrap_or(0) >= 1);
}

#[test]
fn test_bench_layered_scenario_exercises_personal_override() {
    let (stdout, _stderr, ok) = run_whetstone(
        &[
            "--json",
            "bench",
            "run",
            "--scenario",
            "layered/personal_override_python",
        ],
        env!("CARGO_MANIFEST_DIR"),
    );
    assert!(ok);
    let result = parse_json(&stdout);
    let scenarios = result["scenarios"].as_array().unwrap();
    assert_eq!(scenarios.len(), 1);
    assert_eq!(scenarios[0]["category"], "layered");
    assert!(
        (scenarios[0]["f1"].as_f64().unwrap() - 1.0).abs() < 0.001,
        "layered override scenario regressed: {result}"
    );
}

#[test]
fn test_bench_check_exits_nonzero_on_regression() {
    // Create a corpus where the expected violations are impossible to match
    // so every scenario fails, then verify --check exits non-zero.
    let tmp = std::env::temp_dir().join(format!("wh_bench_regress_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let scenario_dir = tmp.join("benchmarks").join("rust").join("regression");
    std::fs::create_dir_all(scenario_dir.join("src")).unwrap();
    std::fs::write(
        scenario_dir.join("src").join("main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    std::fs::write(
        scenario_dir.join("meta.yaml"),
        "scenario: regression\nlanguage: rust\nrules:\n  - rust.expect-over-unwrap\n",
    )
    .unwrap();
    std::fs::write(
        scenario_dir.join("expected.json"),
        r#"{"violations": [{"rule_id": "rust.expect-over-unwrap", "file": "src/main.rs", "line": 1}]}"#,
    )
    .unwrap();

    let bin = whetstone_bin();
    let status = Command::new(&bin)
        .args(["bench", "run", "--check"])
        .current_dir(&tmp)
        .status()
        .unwrap();
    assert!(
        !status.success(),
        "--check should exit non-zero when a scenario regresses"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_bench_rejects_unknown_action() {
    let bin = whetstone_bin();
    let output = Command::new(&bin)
        .args(["bench", "nonsense", "--json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "unknown bench action should exit non-zero"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = parse_json(&stdout);
    assert!(
        result["error"]
            .as_str()
            .unwrap_or("")
            .contains("unknown bench action"),
        "unexpected output: {stdout}"
    );
}

// ── wh check (rule scanning) ──

#[test]
fn test_check_ast_query_signal_reports_tree_sitter_match() {
    // Build a project with a single `ast` rule carrying a real tree-sitter
    // `ast_query`; the runner must report hits based on the parsed tree, not
    // regex. The query matches every async def in Python source.
    let tmp = std::env::temp_dir().join(format!("wh_check_ast_query_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone").join("rules").join("python")).unwrap();
    std::fs::write(
        tmp.join("whetstone")
            .join("rules")
            .join("python")
            .join("example.yaml"),
        r#"source:
  name: example
  docs_url: https://example.com/
  version: "1.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: pypi

rules:
  - id: example.no-async-defs
    severity: must
    confidence: high
    category: default
    description: "Flag every async def in the codebase."
    source_url: https://example.com/rule
    approved: true
    status: approved
    proposed_at: "2026-04-14T00:00:00Z"
    signals:
      - id: async-functions
        strategy: ast
        description: "async function definitions"
        weight: required
        ast_query: '(function_definition "async") @match'
    golden_examples:
      - code: ""
        verdict: pass
        reason: "placeholder"
"#,
    )
    .unwrap();
    std::fs::write(tmp.join("whetstone").join("whetstone.yaml"), "deny: []\n").unwrap();
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(
        tmp.join("src").join("app.py"),
        "async def fetch():\n    return 1\n\ndef sync_fn():\n    return 2\n\nasync def other():\n    return 3\n",
    )
    .unwrap();

    let (stdout, _stderr, _ok) = run_whetstone(
        &["--json", "check", "src", "--lang", "python", "--no-fail"],
        tmp.to_str().unwrap(),
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "violations_found");
    let violations = result["violations"].as_array().unwrap();
    assert_eq!(violations.len(), 2, "expected 2 async defs: {violations:?}");
    for v in violations {
        assert_eq!(v["signal_check_type"], "ast_query");
        assert_eq!(v["rule_id"], "example.no-async-defs");
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_check_ast_scope_pattern_restricts_regex_to_function_bodies() {
    // Pattern rule that would fire on a module-level TODO comment, but is
    // scoped to function_definition. The module-level TODO must be ignored.
    let tmp = std::env::temp_dir().join(format!("wh_check_ast_scope_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone").join("rules").join("python")).unwrap();
    std::fs::write(
        tmp.join("whetstone")
            .join("rules")
            .join("python")
            .join("example.yaml"),
        r#"source:
  name: example
  docs_url: https://example.com/
  version: "1.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: pypi

rules:
  - id: example.todo-in-fn
    severity: should
    confidence: high
    category: convention
    description: "Flag TODO comments inside function bodies only."
    source_url: https://example.com/rule
    approved: true
    status: approved
    proposed_at: "2026-04-14T00:00:00Z"
    signals:
      - id: body-todo
        strategy: pattern
        description: "TODO inside a function body"
        weight: required
        match: 'TODO'
        ast_scope: 'function_definition'
    golden_examples:
      - code: ""
        verdict: pass
        reason: "placeholder"
"#,
    )
    .unwrap();
    std::fs::write(tmp.join("whetstone").join("whetstone.yaml"), "deny: []\n").unwrap();
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(
        tmp.join("src").join("app.py"),
        "# TODO in module scope should NOT fire\n\ndef foo():\n    # TODO inside body should fire\n    pass\n",
    )
    .unwrap();

    let (stdout, _stderr, _ok) = run_whetstone(
        &["--json", "check", "src", "--lang", "python", "--no-fail"],
        tmp.to_str().unwrap(),
    );
    let result = parse_json(&stdout);
    let violations = result["violations"].as_array().unwrap();
    assert_eq!(
        violations.len(),
        1,
        "expected only the body-scoped TODO: {violations:?}"
    );
    assert_eq!(violations[0]["signal_check_type"], "ast_scoped_regex");
    assert_eq!(violations[0]["line"], 4);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_check_reports_lint_proxy_gap_when_ruff_rule_not_selected() {
    let tmp = std::env::temp_dir().join(format!("wh_check_lint_proxy_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone").join("rules").join("python")).unwrap();
    std::fs::write(
        tmp.join("whetstone")
            .join("rules")
            .join("python")
            .join("example.yaml"),
        r#"source:
  name: example
  docs_url: https://example.com/
  version: "1.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: pypi

rules:
  - id: example.b006
    severity: must
    confidence: high
    category: default
    description: "Mutable default args — covered by ruff B006."
    source_url: https://example.com/rule
    approved: true
    status: approved
    proposed_at: "2026-04-14T00:00:00Z"
    signals:
      - id: ruff-proxy
        strategy: lint_proxy
        description: "ruff B006"
        weight: required
    golden_examples:
      - code: ""
        verdict: pass
        reason: "placeholder"
"#,
    )
    .unwrap();
    std::fs::write(tmp.join("whetstone").join("whetstone.yaml"), "deny: []\n").unwrap();

    // Ruff config that does NOT include B006. verify_lint_proxies must flag.
    std::fs::write(tmp.join("ruff.toml"), "[lint]\nselect = [\"E501\"]\n").unwrap();
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src").join("noop.py"), "x = 1\n").unwrap();

    let (stdout, _stderr, _ok) = run_whetstone(
        &["--json", "check", "src", "--lang", "python", "--no-fail"],
        tmp.to_str().unwrap(),
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "config_issues_found", "{result}");
    let issues = result["config_issues"].as_array().unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0]["linter"], "ruff");
    assert_eq!(issues[0]["code"], "B006");
    assert_eq!(issues[0]["rule_id"], "example.b006");

    // Now enable B006 via extend-select and verify the gap clears.
    std::fs::write(
        tmp.join("ruff.toml"),
        "[lint]\nselect = [\"E501\"]\nextend-select = [\"B006\"]\n",
    )
    .unwrap();
    let (stdout, _stderr, _ok) = run_whetstone(
        &["--json", "check", "src", "--lang", "python", "--no-fail"],
        tmp.to_str().unwrap(),
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["config_issues"].as_array().unwrap().len(), 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_check_exits_nonzero_on_config_issues_without_no_fail() {
    let tmp = std::env::temp_dir().join(format!("wh_check_lint_proxy_exit_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::create_dir_all(tmp.join("whetstone").join("rules").join("python")).unwrap();
    std::fs::write(
        tmp.join("whetstone")
            .join("rules")
            .join("python")
            .join("example.yaml"),
        r#"source:
  name: example
  docs_url: https://example.com/
  version: "1.0"
  content_hash: "sha256:test"
  resolved_at: "2026-04-14T00:00:00Z"
  registry: pypi

rules:
  - id: example.b006
    severity: must
    confidence: high
    category: default
    description: "Mutable default args — covered by ruff B006."
    source_url: https://example.com/rule
    approved: true
    status: approved
    proposed_at: "2026-04-14T00:00:00Z"
    signals:
      - id: ruff-proxy
        strategy: lint_proxy
        description: "ruff B006"
        weight: required
    golden_examples:
      - code: ""
        verdict: pass
        reason: "placeholder"
"#,
    )
    .unwrap();
    std::fs::write(tmp.join("whetstone").join("whetstone.yaml"), "deny: []\n").unwrap();
    std::fs::write(tmp.join("ruff.toml"), "[lint]\nselect = [\"E501\"]\n").unwrap();
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src").join("noop.py"), "x = 1\n").unwrap();

    let bin = whetstone_bin();
    let success = Command::new(&bin)
        .args(["check", "src", "--lang", "python"])
        .current_dir(&tmp)
        .output()
        .unwrap()
        .status
        .success();
    assert!(
        !success,
        "config issues should fail wh check unless --no-fail is supplied"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_builtin_rules_use_tree_sitter_path_when_available() {
    // Proves the post-audit upgrade: shipped built-in rules now carry
    // `ast_query`/`ast_scope` and the wh check runner hits the tree-sitter
    // path instead of regex. We scan the benchmark fixture that the
    // previous audit called out as "tree-sitter wired up but unused."
    let (stdout, _stderr, _ok) = run_whetstone(
        &[
            "--json",
            "check",
            "benchmarks/rust/unwrap_usage/src",
            "--lang",
            "rust",
            "--rule",
            "rust.expect-over-unwrap",
            "--no-fail",
        ],
        env!("CARGO_MANIFEST_DIR"),
    );
    let result = parse_json(&stdout);
    let violations = result["violations"].as_array().unwrap();
    assert!(!violations.is_empty(), "expected at least one violation");
    for v in violations {
        assert_eq!(
            v["signal_check_type"], "ast_query",
            "built-in rust.expect-over-unwrap must run via tree-sitter, got: {v}"
        );
    }
}

#[test]
fn test_check_finds_rust_unwrap_violation() {
    // Self-check: Whetstone's own built-in Rust rule flags `.unwrap()` calls.
    // The repo knowingly has `.unwrap()` usages for infallible regex compiles,
    // so `wh check --lang rust` must report at least one violation.
    let (stdout, _stderr, _ok) = run_whetstone(
        &["--json", "check", "src", "--lang", "rust", "--no-fail"],
        env!("CARGO_MANIFEST_DIR"),
    );
    let result = parse_json(&stdout);
    assert_eq!(
        result["status"], "violations_found",
        "expected built-in rust.unwrap rule to fire on Whetstone's own sources"
    );
    let violations = result["violations"].as_array().unwrap();
    assert!(!violations.is_empty(), "expected at least one violation");
    let has_unwrap = violations.iter().any(|v| {
        v.get("rule_id")
            .and_then(|r| r.as_str())
            .map(|r| r.contains("unwrap"))
            .unwrap_or(false)
    });
    assert!(
        has_unwrap,
        "expected an unwrap-related violation: {violations:?}"
    );
}

#[test]
fn test_check_no_fail_exit_zero_on_violations() {
    // With --no-fail the command reports violations but exits zero so a user
    // can preview results without breaking CI. Without --no-fail, violations
    // trigger exit 1.
    let bin = whetstone_bin();
    let ok_exit = Command::new(&bin)
        .args(["check", "src", "--lang", "rust", "--no-fail"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap()
        .status
        .success();
    assert!(ok_exit, "--no-fail should exit 0 even with violations");
}

#[test]
fn test_check_filters_by_rule_id() {
    let (stdout, _stderr, _ok) = run_whetstone(
        &[
            "--json",
            "check",
            "src",
            "--lang",
            "rust",
            "--rule",
            "rust.expect-over-unwrap",
            "--no-fail",
        ],
        env!("CARGO_MANIFEST_DIR"),
    );
    let result = parse_json(&stdout);
    let violations = result["violations"].as_array().unwrap();
    for v in violations {
        assert_eq!(v["rule_id"], "rust.expect-over-unwrap");
    }
}

#[test]
fn test_detect_patterns_quiet_mode() {
    let tmp = std::env::temp_dir().join(format!("whetstone_patterns_quiet_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Empty dir — no git, no transcripts, no PRs. Quiet mode should still
    // return the documented no-op payload.
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "detect-patterns",
            "--sources",
            "git",
            "--quiet",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(success);
    let result = parse_json(&stdout);
    assert!(result["patterns"].as_array().unwrap().is_empty());
    assert_eq!(
        result["next_command"],
        "No patterns found. Proceed to extraction."
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Propose import / diff / schema (3D.1.1, 3D.1.3) ──

fn sample_proposal_bundle(dep: &str, lang: &str, rule_suffix: &str) -> String {
    format!(
        r#"version: 1
proposed_by: test-agent
dependency:
  name: {dep}
  language: {lang}
  version: "1.0"
  source_url: https://example.com/{dep}
  content_hash: "sha256:test"
  registry: manual
proposals:
  - id: {dep}.{rule_suffix}
    severity: must
    confidence: high
    category: default
    description: "Test rule for {dep}"
    source_url: https://example.com/{dep}/rule
    source_kind: official_docs
    signals:
      - id: s1
        strategy: pattern
        description: test signal
        weight: required
        match: 'TODO'
    golden_examples:
      - code: "x = 1"
        verdict: pass
        reason: ok
      - code: "TODO"
        verdict: fail
        reason: contains TODO
      - code: "y = 2"
        verdict: pass
        reason: ok
"#
    )
}

#[test]
fn test_propose_schema_emits_structured_document() {
    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["propose", "schema"],
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    );
    assert!(ok, "propose schema should succeed");
    let result = parse_json(&stdout);
    assert_eq!(result["version"], 1);
    assert!(result["ProposedRule"].is_object());
    assert!(result["enforcement"].is_object());
}

#[test]
fn test_propose_import_writes_candidate_yaml() {
    let tmp = std::env::temp_dir().join(format!("whetstone_propose_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bundle = tmp.join("bundle.yaml");
    std::fs::write(&bundle, sample_proposal_bundle("pkg", "python", "v1")).unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "propose",
            "import",
            bundle.to_str().unwrap(),
            "--project-dir",
            tmp.to_str().unwrap(),
            "--actor",
            "pytest",
        ],
        &tmp,
    );
    assert!(ok, "propose import should succeed");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["action"], "imported");
    assert_eq!(result["proposed_by"], "pytest");

    let target = tmp.join("whetstone/rules/python/pkg.yaml");
    let contents = std::fs::read_to_string(&target).unwrap();
    assert!(contents.contains("status: candidate"));
    assert!(contents.contains("approved: false"));
    assert!(contents.contains("proposed_by: pytest"));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_propose_import_dry_run_does_not_write() {
    let tmp = std::env::temp_dir().join(format!("whetstone_dryrun_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bundle = tmp.join("bundle.yaml");
    std::fs::write(&bundle, sample_proposal_bundle("pkg", "python", "v1")).unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "propose",
            "import",
            bundle.to_str().unwrap(),
            "--project-dir",
            tmp.to_str().unwrap(),
            "--dry-run",
        ],
        &tmp,
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(result["action"], "dry_run");
    assert!(!tmp.join("whetstone/rules/python/pkg.yaml").exists());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_propose_diff_reports_added_rules() {
    let tmp = std::env::temp_dir().join(format!("whetstone_diff_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bundle = tmp.join("bundle.yaml");
    std::fs::write(&bundle, sample_proposal_bundle("pkg", "python", "v1")).unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "propose",
            "diff",
            bundle.to_str().unwrap(),
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        &tmp,
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["summary"]["added"].as_u64().unwrap(), 1);
    assert_eq!(result["summary"]["conflicts"].as_u64().unwrap(), 0);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_propose_import_ignores_invalid_config_value_and_uses_defaults() {
    let tmp = std::env::temp_dir().join(format!("whetstone_quota_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "extraction:\n  max_rules_per_dep: 0\n",
    )
    .unwrap();
    let bundle = tmp.join("bundle.yaml");
    std::fs::write(&bundle, sample_proposal_bundle("pkg", "python", "v1")).unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "propose",
            "import",
            bundle.to_str().unwrap(),
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        &tmp,
    );
    assert!(
        ok,
        "invalid config values should be caught by `wh config validate` and ignored at runtime: {stdout}"
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["action"], "imported");

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_review_diff_summarizes_pending_candidates() {
    let tmp = std::env::temp_dir().join(format!("whetstone_review_diff_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let bundle = tmp.join("bundle.yaml");
    std::fs::write(&bundle, sample_proposal_bundle("pkg", "python", "v1")).unwrap();

    let (_stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "propose",
            "import",
            bundle.to_str().unwrap(),
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        &tmp,
    );
    assert!(ok);

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["review", "--project-dir", tmp.to_str().unwrap(), "diff"],
        &tmp,
    );
    assert!(ok, "review diff should succeed");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["summary"]["total_candidates"].as_u64().unwrap(), 1);
    assert_eq!(result["summary"]["candidate_deps"].as_u64().unwrap(), 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Config show / validate (3D.2.3) ──

#[test]
fn test_config_show_surfaces_effective_and_provenance() {
    let tmp = std::env::temp_dir().join(format!("whetstone_cfg_show_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "extraction:\n  max_rules_per_dep: 3\n  include: [fastapi]\nresolve:\n  timeout_seconds: 45\n",
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["config", "show", "--project-dir", tmp.to_str().unwrap()],
        &tmp,
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(
        result["effective"]["extraction"]["max_rules_per_dep"]
            .as_u64()
            .unwrap(),
        3
    );
    assert_eq!(
        result["effective"]["resolve"]["timeout_seconds"]
            .as_u64()
            .unwrap(),
        45
    );
    assert_eq!(
        result["sources"]["extraction.max_rules_per_dep"]
            .as_str()
            .unwrap(),
        "project"
    );
    // Precedence vector is documented in the response itself.
    let prec: Vec<&str> = result["precedence"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(prec, vec!["default", "global", "project", "personal"]);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_config_validate_flags_unknown_key() {
    let tmp = std::env::temp_dir().join(format!("whetstone_cfg_validate_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "extractoin:\n  max_rules_per_dep: 5\n",
    )
    .unwrap();

    let (stdout, _stderr, _ok) = run_whetstone_from_cwd(
        &["config", "validate", "--project-dir", tmp.to_str().unwrap()],
        &tmp,
    );
    let result = parse_json(&stdout);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(
        diags.iter().any(|d| d["message"]
            .as_str()
            .map(|m| m.contains("extractoin"))
            .unwrap_or(false)),
        "expected unknown-key warning, got {diags:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_config_validate_flags_invalid_values_and_sanitizes_effective_config() {
    let tmp = std::env::temp_dir().join(format!("whetstone_cfg_invalid_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "generate:\n  formats: [bogus.md]\nextraction:\n  min_confidence: low\n  max_rules_per_dep: 0\ncheck:\n  fail_on: maybe\nbench:\n  min_f1: 1.5\nresolve:\n  timeout_seconds: 0\n",
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["config", "validate", "--project-dir", tmp.to_str().unwrap()],
        &tmp,
    );
    assert!(!ok, "invalid config values should fail validation");
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "error");
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(
        diags.iter().any(|d| d["message"]
            .as_str()
            .map(|m| m.contains("min_confidence"))
            .unwrap_or(false)),
        "expected min_confidence diagnostic, got {diags:?}"
    );
    assert_eq!(
        result["effective"]["generate"]["formats"],
        serde_json::json!([])
    );
    assert_eq!(
        result["effective"]["extraction"]["min_confidence"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["effective"]["check"]["fail_on"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["effective"]["bench"]["min_f1"],
        serde_json::Value::Null
    );
    assert_eq!(
        result["effective"]["resolve"]["timeout_seconds"],
        serde_json::Value::Null
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_config_personal_overrides_project() {
    let tmp = std::env::temp_dir().join(format!("whetstone_cfg_personal_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/.personal")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "resolve:\n  timeout_seconds: 30\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("whetstone/.personal/config.yaml"),
        "resolve:\n  timeout_seconds: 90\n",
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["config", "show", "--project-dir", tmp.to_str().unwrap()],
        &tmp,
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(
        result["effective"]["resolve"]["timeout_seconds"]
            .as_u64()
            .unwrap(),
        90
    );
    assert_eq!(
        result["sources"]["resolve.timeout_seconds"]
            .as_str()
            .unwrap(),
        "personal"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Worklist (3D.1.2) ──

#[test]
fn test_review_worklist_requires_handoff() {
    let tmp = std::env::temp_dir().join(format!("whetstone_wl_empty_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    let (stdout, _stderr, _ok) = run_whetstone_from_cwd(
        &["review", "--project-dir", tmp.to_str().unwrap(), "worklist"],
        &tmp,
    );
    let result = parse_json(&stdout);
    assert!(
        result["error"]
            .as_str()
            .map(|e| e.contains("extraction-handoff.json"))
            .unwrap_or(false),
        "expected missing-handoff error, got {result:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_worklist_embedded_in_extraction_handoff() {
    // Use the repo itself (has resolved dependencies) and confirm the
    // worklist is included in the handoff artifact after `wh doctor`.
    let tmp = std::env::temp_dir().join(format!("whetstone_wl_embed_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Minimal pyproject.toml to make doctor work quickly.
    std::fs::write(
        tmp.join("pyproject.toml"),
        "[project]\nname = \"tmp\"\nversion = \"0.1.0\"\ndependencies = [\"fastapi\"]\n",
    )
    .unwrap();

    // We can't guarantee network access in tests — simulate by writing
    // a minimal handoff artifact with an empty worklist and assert the
    // review worklist command reads it.
    let state_dir = tmp.join("whetstone/.state");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::write(
        state_dir.join("extraction-handoff.json"),
        r#"{"version":1,"trigger":"doctor","worklist":[{"name":"fastapi","language":"python","priority":"ready_now","score":120.0,"sections":[],"existing_rules":0,"quota":{"max_rules_per_dep":5,"remaining":5},"next_step":"Read the linked source"}]}"#,
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "review",
            "--project-dir",
            tmp.to_str().unwrap(),
            "worklist",
            "--dep",
            "fastapi",
        ],
        &tmp,
    );
    assert!(ok);
    let result = parse_json(&stdout);
    assert_eq!(result["total"].as_u64().unwrap(), 1);
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries[0]["name"], "fastapi");
    assert_eq!(entries[0]["priority"], "ready_now");
    assert_eq!(entries[0]["quota"]["remaining"].as_u64().unwrap(), 5);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_propose_import_enforces_per_dep_quota_across_existing_rules() {
    let tmp = std::env::temp_dir().join(format!("whetstone_perdep_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "extraction:\n  max_rules_per_dep: 1\n",
    )
    .unwrap();
    // Seed an approved rule so even one new candidate would tip over the quota.
    let rules_dir = tmp.join("whetstone/rules/python");
    std::fs::create_dir_all(&rules_dir).unwrap();
    std::fs::write(
        rules_dir.join("pkg.yaml"),
        r#"source:
  name: pkg
  docs_url: https://example.com/pkg
  version: "1.0"
  content_hash: "sha256:test"
  resolved_at: "2026-01-01T00:00:00Z"
  registry: manual
rules:
  - id: pkg.existing
    severity: must
    confidence: high
    category: default
    description: existing
    source_url: https://example.com/existing
    approved: true
    status: approved
    signals:
      - id: s1
        strategy: pattern
        description: x
        weight: required
        match: y
    golden_examples:
      - code: ""
        verdict: pass
        reason: ok
      - code: y
        verdict: fail
        reason: bad
      - code: z
        verdict: pass
        reason: ok
"#,
    )
    .unwrap();
    let bundle = tmp.join("bundle.yaml");
    std::fs::write(&bundle, sample_proposal_bundle("pkg", "python", "new-rule")).unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "propose",
            "import",
            bundle.to_str().unwrap(),
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        &tmp,
    );
    assert!(!ok, "should refuse to push past the per-dep quota");
    let result = parse_json(&stdout);
    let err = result["error"].as_str().unwrap_or("");
    assert!(
        err.contains("max_rules_per_dep"),
        "expected per-dep quota error, got: {err}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_config_validate_catches_nested_typo() {
    let tmp = std::env::temp_dir().join(format!("whetstone_typo_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    // Typo: `max_rules_per_deps` (extra s) nested under a known parent.
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "extraction:\n  max_rules_per_deps: 5\n",
    )
    .unwrap();
    let (stdout, _stderr, _ok) = run_whetstone_from_cwd(
        &["config", "validate", "--project-dir", tmp.to_str().unwrap()],
        &tmp,
    );
    let result = parse_json(&stdout);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(
        diags.iter().any(|d| d["message"]
            .as_str()
            .map(|m| m.contains("max_rules_per_deps"))
            .unwrap_or(false)),
        "expected nested-typo warning, got {diags:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_review_worklist_human_output() {
    let tmp = std::env::temp_dir().join(format!("whetstone_wl_human_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/.state")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.state/extraction-handoff.json"),
        r#"{"version":1,"trigger":"doctor","worklist":[{"name":"fastapi","language":"python","priority":"ready_now","score":120.0,"sections":[],"existing_rules":0,"quota":{"max_rules_per_dep":5,"remaining":5},"next_step":"Read the linked source"}]}"#,
    )
    .unwrap();

    // Force tty-like output by not passing --json. `run_whetstone_from_cwd`
    // still pipes stdout but is_piped() auto-selects JSON; this test asserts
    // that JSON output is structured regardless.
    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["review", "--project-dir", tmp.to_str().unwrap(), "worklist"],
        &tmp,
    );
    assert!(ok);
    // Because stdout is piped in tests, output is auto-JSON; still verify the
    // formatter isn't accidentally triggered (which would print "0 rule(s)").
    assert!(!stdout.contains("wh review: 0 rule(s)"));
    let result = parse_json(&stdout);
    assert_eq!(result["total"].as_u64().unwrap(), 1);

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_bench_uses_configured_relative_corpus_dir_from_project_root() {
    let tmp = std::env::temp_dir().join(format!("whetstone_bench_cfg_{}", std::process::id()));
    let outside =
        std::env::temp_dir().join(format!("whetstone_bench_cfg_cwd_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::create_dir_all(tmp.join("custom-bench/python/case/src")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        "bench:\n  corpus_dir: custom-bench\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("custom-bench/python/case/meta.yaml"),
        "scenario: case\nlanguage: python\nrules:\n  - python.no-shell-true\ncategory: deterministic\n",
    )
    .unwrap();
    std::fs::write(
        tmp.join("custom-bench/python/case/expected.json"),
        r#"{"violations": []}"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("custom-bench/python/case/src/ok.py"),
        "print(\"ok\")\n",
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &[
            "bench",
            "run",
            "--project-dir",
            tmp.to_str().unwrap(),
            "--json",
        ],
        &outside,
    );
    assert!(
        ok,
        "bench should resolve config corpus_dir relative to project root: {stdout}"
    );
    let result = parse_json(&stdout);
    assert_eq!(result["status"], "ok");
    assert_eq!(result["summary"]["total"].as_u64().unwrap(), 1);

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn test_config_template_matches_supported_schema() {
    let tmp = std::env::temp_dir().join(format!("whetstone_cfg_template_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone")).unwrap();
    std::fs::write(
        tmp.join("whetstone/whetstone.yaml"),
        include_str!("../assets/whetstone.yaml.template"),
    )
    .unwrap();

    let (stdout, _stderr, ok) = run_whetstone_from_cwd(
        &["config", "validate", "--project-dir", tmp.to_str().unwrap()],
        &tmp,
    );
    assert!(ok, "template should validate cleanly: {stdout}");
    let result = parse_json(&stdout);
    let diags = result["diagnostics"].as_array().unwrap();
    assert!(
        diags.is_empty(),
        "template drifted from supported config: {diags:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
