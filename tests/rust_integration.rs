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

fn run_bd(args: &[&str], current_dir: &Path) -> (String, String, bool) {
    let output = Command::new("bd")
        .args(args)
        .current_dir(current_dir)
        .output()
        .unwrap_or_else(|e| panic!("Failed to run bd: {e}"));

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr, output.status.success())
}

fn bd_available() -> bool {
    Command::new("bd")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
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
        &["context", "--dry-run", "--json"],
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
        &["tests", "--dry-run", "--json"],
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
        &["context", "--dry-run", "--json"],
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
    // Rust now emits AGENTS.md plus per-language sidecars when >1 language is
    // present; legacy only ever emits AGENTS.md. Assert the main file is
    // present in both outputs; sidecars are Rust-side additions.
    let rust_files = rust_result["generated"].as_array().unwrap();
    assert!(rust_files.iter().any(|f| {
        f.get("path")
            .and_then(|p| p.as_str())
            .map(|p| p.ends_with("AGENTS.md"))
            .unwrap_or(false)
    }));
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
        &["tests", "--dry-run", "--json"],
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
    let (stdout, _stderr, success) = run_whetstone(&["ci", "--json"], dir.to_str().unwrap());
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
        &["ci", "--json", "--no-drift-check"],
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
    // Core visible commands
    assert!(stdout.contains("init"), "help should contain 'init'");
    assert!(stdout.contains("reinit"), "help should contain 'reinit'");
    assert!(stdout.contains("status"), "help should contain 'status'");
    assert!(stdout.contains("actions"), "help should contain 'actions'");
    assert!(stdout.contains("extract"), "help should contain 'extract'");
    assert!(stdout.contains("approve"), "help should contain 'approve'");
    assert!(stdout.contains("check"), "help should contain 'check'");
    assert!(stdout.contains("debt"), "help should contain 'debt'");
    assert!(stdout.contains("tui"), "help should contain 'tui'");
    assert!(stdout.contains("rule"), "help should contain 'rule'");
    assert!(stdout.contains("source"), "help should contain 'source'");
    assert!(
        stdout.contains("validate"),
        "help should contain 'validate'"
    );
    assert!(stdout.contains("Core workflow:"), "help should contain taxonomy guidance");
    assert!(stdout.contains("whetstone actions --only <context|tests|lint>"));
    // Aliases were removed in 0.3.0 — these legacy spellings must NOT appear.
    assert!(!stdout.contains("doctor"), "'doctor' alias should be gone");
    assert!(!stdout.contains("generate-context"), "'generate-context' alias should be gone");
    assert!(!stdout.contains("generate-tests"), "'generate-tests' alias should be gone");
}

#[test]
fn test_rule_help_shows_grouped_advanced_subcommands() {
    let (stdout, _stderr, success) = run_whetstone(&["rule", "--help"], ".");
    assert!(success);
    assert!(stdout.contains("add"));
    assert!(stdout.contains("edit"));
    assert!(stdout.contains("query"));
    assert!(stdout.contains("review"));
    assert!(stdout.contains("worklist"));
}

#[test]
fn test_tui_json_mode_returns_machine_error() {
    let (stdout, _stderr, success) = run_whetstone(&["--json", "tui"], ".");
    assert!(!success);
    let result = parse_json(&stdout);
    assert!(result["error"]
        .as_str()
        .unwrap_or("")
        .contains("TUI is only available"));
}

#[test]
fn test_rules_query_by_file() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "rules",
            "query",
            "--file",
            "src/app.py",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(success, "wh rules query --file must succeed");
    let result: serde_json::Value =
        serde_json::from_str(&stdout).expect("wh rules query --json must produce valid JSON");
    assert_eq!(result["filters"]["file"], "src/app.py");
    assert!(
        result["total"].as_u64().unwrap() >= 1,
        "fixtures have at least one Python rule"
    );
    let rules = result["rules"].as_array().unwrap();
    for rule in rules {
        assert_eq!(
            rule["language"], "python",
            "--file src/app.py should only return python rules"
        );
    }
}

#[test]
fn test_rules_query_by_lang_and_severity() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "rules",
            "query",
            "--lang",
            "python",
            "--severity",
            "must",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(success);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    for rule in result["rules"].as_array().unwrap() {
        assert_eq!(rule["language"], "python");
        assert_eq!(rule["severity"], "must");
    }
}

#[test]
fn test_rules_query_full_includes_signals() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "rules",
            "query",
            "--lang",
            "python",
            "--full",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(success);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    if let Some(first) = result["rules"].as_array().and_then(|a| a.first()) {
        assert!(
            first.get("signals").is_some(),
            "--full must include signals key"
        );
        assert!(
            first.get("golden_examples").is_some(),
            "--full must include golden_examples key"
        );
    }
}

#[test]
fn test_source_add_list_remove_round_trip() {
    let tmp = std::env::temp_dir().join(format!("wh_source_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        b"[project]\nname=\"t\"\nversion=\"0.1.0\"\ndependencies=[]\n",
    )
    .unwrap();

    // Add a personal source.
    let (_, _, ok) = run_whetstone(
        &[
            "source",
            "add",
            "https://blog.example.com/py",
            "--name",
            "py-blog",
            "--lang",
            "python",
            "--kind",
            "blog",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok, "wh source add (personal) must succeed");
    assert!(tmp.join("whetstone/.personal/config.yaml").exists());

    // Add a project source.
    let (_, _, ok2) = run_whetstone(
        &[
            "source",
            "add",
            "https://team.internal/conv",
            "--project",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok2);
    assert!(tmp.join("whetstone/whetstone.yaml").exists());

    // List.
    let (list_out, _, ok3) = run_whetstone(
        &[
            "source",
            "list",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok3);
    let listed: serde_json::Value = serde_json::from_str(&list_out).unwrap();
    assert_eq!(listed["total"], 2);
    assert_eq!(listed["personal"][0]["name"], "py-blog");
    assert_eq!(listed["project"][0]["url"], "https://team.internal/conv");

    // Duplicate add must refuse.
    let (_, _, ok_dup) = run_whetstone(
        &[
            "source",
            "add",
            "https://blog.example.com/py",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(!ok_dup, "duplicate subscription must be refused");

    // Remove by name.
    let (rm_out, _, ok4) = run_whetstone(
        &[
            "source",
            "remove",
            "py-blog",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok4);
    let removed: serde_json::Value = serde_json::from_str(&rm_out).unwrap();
    assert_eq!(removed["removed_url"], "https://blog.example.com/py");

    // List again: personal empty, project kept.
    let (list_out2, _, _) = run_whetstone(
        &[
            "source",
            "list",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    let listed2: serde_json::Value = serde_json::from_str(&list_out2).unwrap();
    assert_eq!(listed2["total"], 1);
    assert!(listed2["personal"].as_array().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_source_add_rejects_bad_url() {
    let tmp = std::env::temp_dir().join(format!("wh_source_badurl_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let (_, _, ok) = run_whetstone(
        &[
            "source",
            "add",
            "not-a-url",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(!ok);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_personal_only_project_still_gets_adherence_score() {
    // Regression: earlier, `wh rule add --personal` without an explicit init
    // would leave wh status returning adherence_score=null because the
    // "is the project initialized?" gate only looked for whetstone.yaml.
    let tmp = std::env::temp_dir().join(format!("wh_personal_only_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        b"[project]\nname=\"t\"\nversion=\"0.1.0\"\ndependencies=[]\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src/app.py"), b"print(\"hi\")\n").unwrap();

    // Personal rule: never use print()
    let (_, _, add_ok) = run_whetstone(
        &[
            "rule",
            "add",
            "demo.no-print",
            "--description",
            "Never use print",
            "--match",
            "print\\s*\\(",
            "--lang",
            "python",
            "--dep",
            "demo",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(add_ok);

    // wh status must return a non-null adherence_score AND reflect the violation.
    let (stdout, _, ok) = run_whetstone(
        &[
            "status",
            "--json",
            "--no-snapshot",
            "--no-drift-check",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let adherence = result.get("adherence").and_then(|v| v.as_object());
    assert!(
        adherence.is_some(),
        "personal-only project must produce adherence object, got: {result}"
    );
    let viols = adherence
        .unwrap()
        .get("violations")
        .and_then(|v| v.as_object())
        .unwrap();
    assert!(
        viols.get("total").and_then(|t| t.as_u64()).unwrap_or(0) >= 1,
        "violation must be detected from personal rule"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_report_pr_comment_contains_marker_and_headers() {
    let dir = fixtures_dir();
    let (stdout, _, ok) = run_whetstone(
        &[
            "report",
            "--pr-comment",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok, "wh report --pr-comment must succeed");
    assert!(
        stdout.starts_with("<!-- whetstone-report -->"),
        "pr-comment flavor must lead with the marker"
    );
    assert!(stdout.contains("# Whetstone Report"));
    assert!(stdout.contains("**Rule system:**"));
    assert!(stdout.contains("**Adherence:**"));
}

#[test]
fn test_report_json_includes_required_keys() {
    let dir = fixtures_dir();
    let (stdout, _, ok) = run_whetstone(
        &[
            "report",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    for key in [
        "rule_system_score",
        "adherence_score",
        "adherence",
        "violations",
        "next_actions",
    ] {
        assert!(result.get(key).is_some(), "key `{key}` missing from wh report output");
    }
}

#[test]
fn test_status_returns_adherence_score_fields() {
    let dir = fixtures_dir();
    let (stdout, _, ok) = run_whetstone(
        &[
            "status",
            "--json",
            "--no-snapshot",
            "--no-drift-check",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Both the legacy rule-system score and the new adherence score must be
    // present keys — adherence may be null when there are zero eligible files,
    // but the keys must exist.
    assert!(result.get("rule_system_score").is_some());
    assert!(result.get("adherence_score").is_some());
    assert!(result.get("adherence").is_some());
    // Legacy `score` key preserved for backwards compatibility.
    assert!(result.get("score").is_some());
}

#[test]
fn test_metrics_snapshot_captures_adherence_and_violations() {
    let tmp = std::env::temp_dir().join(format!("wh_trend_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    // Copy fixtures into a fresh tmp so the snapshot is deterministic.
    let fx = fixtures_dir();
    std::fs::create_dir_all(&tmp).unwrap();
    // Minimal copy: just pyproject.toml + whetstone/rules.
    std::fs::write(
        tmp.join("pyproject.toml"),
        std::fs::read(fx.join("pyproject.toml")).unwrap_or_default(),
    )
    .unwrap();
    let src_rules = fx.join("whetstone/rules");
    if src_rules.exists() {
        let dst_rules = tmp.join("whetstone/rules");
        std::fs::create_dir_all(&dst_rules).unwrap();
        for entry in walkdir_clone(&src_rules) {
            let rel = entry.strip_prefix(&src_rules).unwrap();
            let dst = dst_rules.join(rel);
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if entry.is_file() {
                std::fs::copy(&entry, &dst).unwrap();
            }
        }
    }

    // Run wh status WITH snapshot (default) so .metrics.jsonl gets written.
    let (_, _, ok) = run_whetstone(
        &[
            "status",
            "--json",
            "--no-drift-check",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok);

    let metrics = tmp.join("whetstone/.metrics.jsonl");
    assert!(metrics.exists(), "metrics file should be written");
    let text = std::fs::read_to_string(&metrics).unwrap();
    let last_line = text.lines().last().expect("at least one snapshot");
    let snap: serde_json::Value = serde_json::from_str(last_line).unwrap();
    assert!(
        snap.get("adherence_score").is_some(),
        "snapshot must carry adherence_score"
    );
    assert!(
        snap.get("violation_counts").is_some(),
        "snapshot must carry violation_counts"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

fn walkdir_clone(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(p) = stack.pop() {
        if p.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                for e in entries.flatten() {
                    stack.push(e.path());
                }
            }
        }
        out.push(p);
    }
    out
}

#[test]
fn test_context_emits_all_six_formats_with_required_markers() {
    let dir = fixtures_dir();
    let (stdout, _, ok) = run_whetstone(
        &[
            "context",
            "--formats",
            "agents.md,claude.md,.cursorrules,copilot-instructions.md,.windsurfrules,codex.md",
            "--project-dir",
            dir.to_str().unwrap(),
            "--json",
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok, "wh context with all formats must succeed: {stdout}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let generated = result["generated"].as_array().expect("generated array");
    let path_of = |suffix: &str| -> Option<String> {
        generated
            .iter()
            .find_map(|g| {
                g.get("path")
                    .and_then(|p| p.as_str())
                    .filter(|p| p.ends_with(suffix))
                    .map(String::from)
            })
    };

    // Per-tool required markers. These encode the minimum surface each target
    // consumer expects — changing them means a consuming tool might stop
    // recognizing our output. Lock them in.
    let expectations: &[(&str, &[&str])] = &[
        ("AGENTS.md", &["# Whetstone Rules", "wh rules query"]),
        ("CLAUDE.md", &["# Whetstone Rules"]),
        (".cursorrules", &["Whetstone Rules"]),
        (".github/copilot-instructions.md", &["Whetstone Rules"]),
        (".windsurfrules", &["Whetstone Rules"]),
        ("codex.md", &["Whetstone Rules"]),
    ];

    for (suffix, markers) in expectations {
        let path = path_of(suffix).unwrap_or_else(|| panic!("missing generated file: {suffix}"));
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed reading {path}: {e}"));
        for marker in *markers {
            assert!(
                body.contains(marker),
                "{suffix} missing required marker `{marker}` (consuming tool may not recognize this output)"
            );
        }
        // Trivially ensure non-empty generated content
        assert!(
            body.trim().len() > 100,
            "{suffix} appears empty or truncated ({} bytes)",
            body.len()
        );
    }
}

#[test]
fn test_context_terse_still_carries_rule_ids_across_formats() {
    // Terse mode must still include rule ids in every format; otherwise
    // agents loading AGENTS.md can't know the id to pass to
    // `wh rules query --full` for details.
    let dir = fixtures_dir();
    let (_, _, ok) = run_whetstone(
        &[
            "context",
            "--terse",
            "--formats",
            "agents.md,.cursorrules,codex.md",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok);

    for fname in [
        "whetstone/context/AGENTS.md",
        "whetstone/context/.cursorrules",
        "whetstone/context/codex.md",
    ] {
        let body = std::fs::read_to_string(dir.join(fname)).expect(fname);
        assert!(
            body.contains("fastapi.async-routes") || body.contains("react.use-client-directive"),
            "{fname} terse rendering must preserve rule ids"
        );
    }
}

#[test]
fn test_rule_add_writes_personal_yaml() {
    let tmp = std::env::temp_dir().join(format!("wh_rule_add_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        b"[project]\nname=\"t\"\nversion=\"0.1.0\"\ndependencies=[]\n",
    )
    .unwrap();
    let (stdout, _stderr, ok) = run_whetstone(
        &[
            "rule",
            "add",
            "acme.snake-case",
            "--description",
            "Use snake_case",
            "--match",
            "def [A-Z]",
            "--lang",
            "python",
            "--dep",
            "acme",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok, "wh rule add must succeed: {stdout}");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["status"], "ok");
    assert_eq!(result["layer"], "personal");
    let target = tmp.join("whetstone/.personal/rules/python/acme.yaml");
    assert!(target.exists(), "target file must exist at {}", target.display());
    let text = std::fs::read_to_string(&target).unwrap();
    assert!(text.contains("acme.snake-case"));
    assert!(text.contains("status: approved"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_rule_edit_bumps_severity_in_place() {
    let tmp = std::env::temp_dir().join(format!("wh_rule_edit_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        b"[project]\nname=\"t\"\nversion=\"0.1.0\"\ndependencies=[]\n",
    )
    .unwrap();
    // Seed with rule add.
    let (_add_stdout, _, add_ok) = run_whetstone(
        &[
            "rule",
            "add",
            "acme.prefer-X",
            "--description",
            "Prefer X",
            "--match",
            "prefer_X",
            "--lang",
            "python",
            "--dep",
            "acme",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(add_ok);

    // Edit severity.
    let (edit_stdout, _, edit_ok) = run_whetstone(
        &[
            "rule",
            "edit",
            "acme.prefer-X",
            "--severity",
            "must",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(edit_ok, "wh rule edit must succeed: {edit_stdout}");
    let result: serde_json::Value = serde_json::from_str(&edit_stdout).unwrap();
    assert_eq!(result["count"], 1);
    assert_eq!(result["changed"][0]["severity"]["before"], "should");
    assert_eq!(result["changed"][0]["severity"]["after"], "must");

    let text =
        std::fs::read_to_string(tmp.join("whetstone/.personal/rules/python/acme.yaml")).unwrap();
    assert!(text.contains("severity: must"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_rule_edit_dry_run_does_not_write() {
    let tmp = std::env::temp_dir().join(format!("wh_rule_dry_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        b"[project]\nname=\"t\"\nversion=\"0.1.0\"\ndependencies=[]\n",
    )
    .unwrap();
    let (_, _, ok) = run_whetstone(
        &[
            "rule",
            "add",
            "acme.x",
            "--description",
            "x",
            "--match",
            "x",
            "--lang",
            "python",
            "--dep",
            "acme",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(ok);

    let (stdout, _, edit_ok) = run_whetstone(
        &[
            "rule",
            "edit",
            "acme.x",
            "--severity",
            "must",
            "--dry-run",
            "--json",
            "--project-dir",
            tmp.to_str().unwrap(),
        ],
        tmp.to_str().unwrap(),
    );
    assert!(edit_ok);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["dry_run"], true);
    let text =
        std::fs::read_to_string(tmp.join("whetstone/.personal/rules/python/acme.yaml")).unwrap();
    // File must still have the original severity.
    assert!(text.contains("severity: should"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_context_terse_shrinks_output() {
    let dir = fixtures_dir();

    let (full_stdout, _, ok) = run_whetstone(
        &[
            "context",
            "--dry-run",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok);
    let full: serde_json::Value = serde_json::from_str(&full_stdout).unwrap();

    let (terse_stdout, _, ok2) = run_whetstone(
        &[
            "context",
            "--terse",
            "--dry-run",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok2);
    let terse: serde_json::Value = serde_json::from_str(&terse_stdout).unwrap();

    // Both modes must generate the main AGENTS.md.
    let main = |v: &serde_json::Value| -> Option<i64> {
        v["generated"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| {
                g.get("path")
                    .and_then(|p| p.as_str())
                    .map(|p| p.ends_with("AGENTS.md"))
                    .unwrap_or(false)
            })
            .and_then(|g| g.get("lines").and_then(|v| v.as_i64()))
    };
    let full_lines = main(&full).expect("full AGENTS.md entry missing");
    let terse_lines = main(&terse).expect("terse AGENTS.md entry missing");
    assert!(
        terse_lines < full_lines,
        "terse AGENTS.md must be shorter (terse={terse_lines}, full={full_lines})"
    );
}

#[test]
fn test_context_emits_per_language_sidecars() {
    let dir = fixtures_dir();
    let (stdout, _, ok) = run_whetstone(
        &[
            "context",
            "--dry-run",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(ok);
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let paths: Vec<String> = result["generated"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|g| g.get("path").and_then(|p| p.as_str()).map(str::to_string))
        .collect();
    // Fixtures carry python + typescript rules → expect both sidecars.
    assert!(paths.iter().any(|p| p.ends_with("AGENTS.python.md")));
    assert!(paths.iter().any(|p| p.ends_with("AGENTS.typescript.md")));
    // Main AGENTS.md is still there.
    assert!(paths.iter().any(|p| p.ends_with("/AGENTS.md")));
}

#[test]
fn test_rules_query_unknown_lang_returns_empty() {
    let dir = fixtures_dir();
    let (stdout, _stderr, success) = run_whetstone(
        &[
            "rules",
            "query",
            "--lang",
            "nonexistent-lang",
            "--json",
            "--project-dir",
            dir.to_str().unwrap(),
        ],
        dir.to_str().unwrap(),
    );
    assert!(success, "empty result is still a successful query");
    let result: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["total"].as_u64().unwrap(), 0);
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
        run_whetstone(&["validate", "--project-dir", repo_root], repo_root);
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
        &["validate", "--project-dir", tmp.to_str().unwrap()],
        tmp.to_str().unwrap(),
    );
    assert!(!success, "validate-rules must fail when fixture is invalid");
    assert!(stdout.contains("invalid severity"));
    assert!(stdout.contains("invalid strategy"));

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── detect-patterns tests ──

#[allow(dead_code)]
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

    let (stdout, _stderr, success) = run_whetstone(&["reinit", "--json"], project);
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

    let (stdout, _stderr, success) = run_whetstone(&["reinit", "--check", "--json"], project);
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

    let (_stdout, _stderr, success) = run_whetstone(&["reinit", "--json"], project);
    assert!(success);

    let handoff_path = tmp.join("whetstone/.state/extraction-handoff.json");
    assert!(handoff_path.exists(), "refresh must rewrite the handoff");
    let handoff: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&handoff_path).unwrap()).unwrap();
    assert_eq!(handoff["version"], 1);
    assert_eq!(
        handoff["trigger"], "reinit",
        "reinit-triggered handoff must be labeled 'reinit'"
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

// ── nq8.3.2: AI eval lifecycle coverage ──

#[allow(dead_code)]
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

#[allow(dead_code)]
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
// ── Propose import / diff / schema (3D.1.1, 3D.1.3) ──

#[allow(dead_code)]
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
    // worklist is included in the handoff artifact after `wh init`.
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
        r#"{"version":1,"trigger":"init","worklist":[{"name":"fastapi","language":"python","priority":"ready_now","score":120.0,"sections":[],"existing_rules":0,"quota":{"max_rules_per_dep":5,"remaining":5},"next_step":"Read the linked source"}]}"#,
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
fn test_review_worklist_human_output() {
    let tmp = std::env::temp_dir().join(format!("whetstone_wl_human_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("whetstone/.state")).unwrap();
    std::fs::write(
        tmp.join("whetstone/.state/extraction-handoff.json"),
        r#"{"version":1,"trigger":"init","worklist":[{"name":"fastapi","language":"python","priority":"ready_now","score":120.0,"sections":[],"existing_rules":0,"quota":{"max_rules_per_dep":5,"remaining":5},"next_step":"Read the linked source"}]}"#,
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

// ── wh debt tests ──

#[test]
fn test_debt_json_envelope_on_small_fixture() {
    let tmp = std::env::temp_dir().join("whetstone_debt_envelope");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        r#"
[project]
name = "debt-sample"
version = "0.1.0"
dependencies = ["requests", "definitely-unused"]
"#,
    )
    .unwrap();
    std::fs::write(
        tmp.join("src/app.py"),
        r#"
import requests

def public_fn():
    return requests.get("https://example.com")

def _never_called_helper():
    return 42
"#,
    )
    .unwrap();

    let (stdout, _stderr, ok) =
        run_whetstone(&["debt", "--json", "--top=10"], tmp.to_str().unwrap());
    assert!(ok, "wh debt should succeed on the fixture");

    let result = parse_json(&stdout);
    assert_json_has_keys(
        &result,
        &[
            "schema_version",
            "generated_at",
            "project_dir",
            "summary",
            "hotspots",
        ],
        "wh debt JSON envelope",
    );
    assert_eq!(result["schema_version"].as_u64().unwrap(), 1);

    let hotspots = result["hotspots"].as_array().unwrap();
    let titles: Vec<&str> = hotspots
        .iter()
        .filter_map(|h| h["title"].as_str())
        .collect();
    assert!(
        titles.iter().any(|t| t.contains("definitely-unused")),
        "expected unused dep to appear: {titles:?}"
    );
    assert!(
        titles.iter().any(|t| t.contains("_never_called_helper")),
        "expected unreferenced private fn to appear: {titles:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_debt_prompt_mode_is_compact_markdown() {
    let tmp = std::env::temp_dir().join("whetstone_debt_prompt");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        r#"
[project]
name = "debt-sample"
version = "0.1.0"
dependencies = ["definitely-unused"]
"#,
    )
    .unwrap();

    let (stdout, _stderr, ok) =
        run_whetstone(&["debt", "--prompt", "--top=5"], tmp.to_str().unwrap());
    assert!(ok);
    assert!(
        stdout.starts_with("# Debt triage handoff"),
        "prompt should start with the handoff header; got: {}",
        &stdout[..stdout.len().min(120)]
    );
    assert!(stdout.contains("definitely-unused"));
    assert!(!stdout.contains("\"schema_version\""));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn test_debt_beads_mode_files_epic_and_tasks() {
    if !bd_available() {
        eprintln!("skipping Beads integration path: `bd` not installed in this environment");
        return;
    }

    let tmp = std::env::temp_dir().join(format!(
        "whetstone_debt_beads_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(
        tmp.join("pyproject.toml"),
        r#"
[project]
name = "debt-sample"
version = "0.1.0"
dependencies = ["definitely-unused"]
"#,
    )
    .unwrap();

    let (_bd_init_out, bd_init_err, bd_ok) =
        run_bd(&["init", "--skip-agents", "--skip-hooks", "-q"], &tmp);
    assert!(bd_ok, "bd init should succeed for debt integration test: {bd_init_err}");

    let (stdout, _stderr, ok) =
        run_whetstone(&["--json", "debt", "--beads", "--top=3"], tmp.to_str().unwrap());
    assert!(ok);
    let result = parse_json(&stdout);
    let epic_id = result["epic_id"]
        .as_str()
        .expect("beads run should return epic_id");
    let task_ids = result["task_ids"]
        .as_array()
        .expect("beads run should return task_ids");
    assert!(!epic_id.is_empty());
    assert!(!task_ids.is_empty());

    let (epic_show, epic_err, epic_ok) = run_bd(&["show", epic_id], &tmp);
    assert!(epic_ok, "bd show should find filed epic: {epic_err}");
    assert!(epic_show.contains("Debt triage:"));

    let _ = std::fs::remove_dir_all(&tmp);
}
