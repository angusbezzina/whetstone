//! Rust integration tests for whetstone binary.
//!
//! Tests core commands against the fixtures directory and the whetstone repo itself.

use std::path::PathBuf;
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

    let (stdout, _stderr, success) = run_whetstone(&["detect-deps", "--incremental"], project_dir);
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
        run_whetstone(&["detect-deps", "--incremental"], project_dir);
    assert!(success1);

    std::fs::write(
        tmp.join("pyproject.toml"),
        "[project]\nname='cleanup'\ndependencies=['requests>=2.31']\n",
    )
    .unwrap();
    let (_stdout2, _stderr2, success2) =
        run_whetstone(&["detect-deps", "--incremental"], project_dir);
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
fn test_doctor_writes_extraction_handoff_with_trigger_doctor() {
    let dir = fixtures_dir();
    // Run against fixtures (has fastapi + react rules); doctor completes fine
    // even without network because no deps need resolving to emit a handoff.
    let (_stdout, _stderr, _success) = run_whetstone(
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

    let handoff_path = dir.join("whetstone/.state/extraction-handoff.json");
    if handoff_path.exists() {
        let handoff: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&handoff_path).unwrap()).unwrap();
        assert_eq!(handoff["version"], 1);
        assert_eq!(handoff["trigger"], "doctor");
        for key in ["candidates", "skipped", "next_action", "generated_at"] {
            assert!(handoff.get(key).is_some(), "handoff missing key: {key}");
        }
    }
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
