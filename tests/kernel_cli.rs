use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run lean Whetstone binary")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn write_rule_project(root: &Path, source: &str) {
    std::fs::create_dir_all(root.join("whetstone/rules/python")).expect("create rule fixture");
    std::fs::create_dir_all(root.join("src")).expect("create source fixture");
    std::fs::write(
        root.join("whetstone/rules/python/names.yaml"),
        r#"source:
  name: team
rules:
  - id: team.lowercase-functions
    severity: must
    confidence: high
    category: convention
    description: Function names must begin with a lowercase character.
    source_url: https://example.com/team/functions
    approved: true
    status: approved
    signals:
      - id: uppercase-function
        strategy: ast
        description: Finds uppercase function names.
        weight: required
        ast_query: '((function_definition name: (identifier) @match) (#match? @match "^[A-Z]"))'
    golden_examples:
      - code: "def read_config():\n    pass\n"
        verdict: pass
      - code: "def ReadConfig():\n    pass\n"
        verdict: fail
"#,
    )
    .expect("write rule fixture");
    std::fs::write(root.join("src/app.py"), source).expect("write source fixture");
}

#[test]
fn bare_json_is_truthful_read_only_orientation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run(&["--json"], root);
    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["status"], "foundation");
    assert_eq!(value["phase"], "r0-prune-first");
    assert_eq!(
        value["target_workflows"],
        serde_json::json!(["init", "dash", "change", "check", "pull", "push"])
    );
    assert_eq!(value["available"], serde_json::json!([]));
}

#[test]
fn public_help_exposes_no_legacy_or_premature_workflow() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run(&["--help"], root);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    for retired in [
        "publish", "status", "debt", "report", "rules", "sources", "pack", "mcp", "tui",
    ] {
        assert!(!help.contains(retired), "help leaked {retired}:\n{help}");
    }
    assert!(
        !help.contains("Commands:"),
        "hidden maintenance surface leaked"
    );
}

#[test]
fn retired_commands_cannot_execute() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for command in ["publish", "status", "debt", "report", "mcp", "actions"] {
        let output = run(&[command], root);
        assert_eq!(output.status.code(), Some(2), "{command} unexpectedly ran");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"),
            "unexpected error for {command}"
        );
    }
}

#[test]
fn validate_eval_and_self_scan_are_non_vacuous() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root_arg = root.to_string_lossy();

    let validate = run(&["validate", "--project-dir", &root_arg, "--json"], root);
    assert!(validate.status.success());
    let validate_json = json(&validate);
    assert_eq!(validate_json["ok"], true);
    let report = validate_json["report"]
        .as_str()
        .expect("validation report should be text");
    assert!(report.contains("Checking "), "{validate_json}");
    assert!(!report.contains("Checking 0 rule files"), "{validate_json}");

    let eval = run(&["eval", "--project-dir", &root_arg, "--json"], root);
    assert!(eval.status.success());
    let eval_json = json(&eval);
    assert_eq!(eval_json["ok"], true);
    assert!(
        eval_json["rules_evaluated"]
            .as_u64()
            .expect("rules_evaluated should be numeric")
            > 0
    );
    let checked: u64 = eval_json["scorecards"]
        .as_array()
        .expect("scorecards should be an array")
        .iter()
        .filter_map(|card| card["golden_checked"].as_u64())
        .sum();
    assert!(
        checked > 0,
        "eval performed no scanner-backed golden checks"
    );

    let scan = run(
        &[
            "scan",
            "src",
            "--project-dir",
            &root_arg,
            "--lang",
            "rust",
            "--json",
            "--no-fail",
        ],
        root,
    );
    assert!(scan.status.success());
    let scan_json = json(&scan);
    assert_eq!(scan_json["violations_count"], 0);
    assert!(
        scan_json["files_scanned"]
            .as_u64()
            .expect("files_scanned should be numeric")
            > 0
    );
    assert!(
        scan_json["rules_applied"]
            .as_u64()
            .expect("rules_applied should be numeric")
            > 0
    );
}

#[test]
fn scanner_finds_known_bad_and_accepts_known_good() {
    let temp = tempfile::tempdir().expect("create scanner fixture");
    write_rule_project(temp.path(), "def ReadConfig():\n    pass\n");
    let project = temp.path().to_string_lossy();
    let bad = run(
        &[
            "scan",
            "src",
            "--project-dir",
            &project,
            "--lang",
            "python",
            "--json",
            "--no-fail",
        ],
        temp.path(),
    );
    assert!(bad.status.success());
    let bad_json = json(&bad);
    assert_eq!(bad_json["violations_count"], 1);
    assert_eq!(bad_json["files_scanned"], 1);
    assert_eq!(bad_json["rules_applied"], 1);

    std::fs::write(
        temp.path().join("src/app.py"),
        "def read_config():\n    pass\n",
    )
    .expect("update source fixture");
    let good = run(
        &[
            "scan",
            "src",
            "--project-dir",
            &project,
            "--lang",
            "python",
            "--json",
            "--no-fail",
        ],
        temp.path(),
    );
    assert!(good.status.success());
    assert_eq!(json(&good)["violations_count"], 0);
}

#[test]
fn retained_gate_commands_do_not_mutate_user_data() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let protected: Vec<PathBuf> = [
        root.join("whetstone/.metrics.jsonl"),
        root.join("whetstone/.personal/config.yaml"),
        root.join("whetstone/.personal/rules/python/snake.yaml"),
        root.join("whetstone/.state/extraction-handoff.json"),
        root.join("whetstone/.state/inventory.json"),
        root.join("whetstone/.state/manifests.json"),
        root.join("whetstone/.state/refresh-log.json"),
        root.join("whetstone/.state/source-cache.json"),
        root.join("whetstone/context/AGENTS.md"),
        root.join("whetstone/evals/rust/test_anyhow.rs"),
        root.join("whetstone/evals/rust/test_clap.rs"),
        root.join("whetstone/evals/rust/test_reqwest.rs"),
        root.join("whetstone/evals/rust/test_serde_yaml.rs"),
        root.join("whetstone/evals/rust/test_whetstone:recommended/rust.rs"),
        root.join("whetstone/rules/rust/anyhow.yaml"),
        root.join("whetstone/whetstone.yaml"),
    ]
    .into_iter()
    .filter(|path| path.exists())
    .collect();
    assert!(
        protected.len() >= 3,
        "clean and local checkouts must expose tracked protected records"
    );
    let before: Vec<Vec<u8>> = protected
        .iter()
        .map(|path| std::fs::read(path).expect("read protected fixture"))
        .collect();
    let root_arg = root.to_string_lossy();

    assert!(run(&["validate", "--project-dir", &root_arg], root)
        .status
        .success());
    assert!(run(&["eval", "--project-dir", &root_arg], root)
        .status
        .success());
    assert!(run(
        &[
            "scan",
            "src",
            "--project-dir",
            &root_arg,
            "--lang",
            "rust",
            "--no-fail",
        ],
        root,
    )
    .status
    .success());

    for (path, expected) in protected.iter().zip(before) {
        assert_eq!(
            std::fs::read(path).expect("reread protected fixture"),
            expected,
            "mutated {}",
            path.display()
        );
    }
}
