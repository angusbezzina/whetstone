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
        reason: Lowercase names comply with the rule.
      - code: "def ReadConfig():\n    pass\n"
        verdict: fail
        reason: Uppercase names violate the rule.
"#,
    )
    .expect("write rule fixture");
    std::fs::write(root.join("src/app.py"), source).expect("write source fixture");
}

fn init_git(root: &Path) {
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .expect("initialize isolated git fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn bare_json_is_truthful_read_only_orientation() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run(&["--json"], root);
    assert!(output.status.success());
    let value = json(&output);
    assert_eq!(value["schema"], "whetstone.command-response.v1");
    assert_eq!(value["state"], "success");
    assert_eq!(value["workflow"], "orientation");
    assert_eq!(
        value["data"]["workflows"],
        serde_json::json!(["init", "dash", "change", "check", "pull", "push"])
    );
    assert_eq!(value["data"]["read_only"], true);
    assert_eq!(
        value["data"]["lean_baseline_revision"],
        "2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48"
    );
}

#[test]
fn public_help_exposes_exactly_six_workflow_families() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = run(&["--help"], root);
    assert!(output.status.success());
    let help = String::from_utf8_lossy(&output.stdout);
    let commands = help
        .lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    assert_eq!(
        commands,
        ["init", "dash", "change", "check", "pull", "push"]
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

    let old_check_alias = run(&["check", "src"], root);
    assert_eq!(old_check_alias.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&old_check_alias.stderr).contains("unexpected argument"));
}

#[test]
fn sync_and_dashboard_are_honestly_unavailable() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for workflow in ["dash", "pull", "push"] {
        let output = run(&[workflow, "--json", "--request-id", "probe-1"], root);
        assert_eq!(output.status.code(), Some(4));
        let value = json(&output);
        assert_eq!(value["schema"], "whetstone.command-response.v1");
        assert_eq!(value["state"], "unavailable");
        assert_eq!(value["request_id"], "probe-1");
    }
}

#[test]
fn init_is_resumable_and_duplicate_requests_are_idempotent() {
    let temp = tempfile::tempdir().expect("create init fixture");
    init_git(temp.path());
    let project = temp.path().to_string_lossy();
    let inspect = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "onboard-1",
        ],
        temp.path(),
    );
    assert_eq!(inspect.status.code(), Some(6));
    let handoff = json(&inspect);
    assert_eq!(handoff["state"], "needs_input");
    let token = handoff["resume_token"].as_str().expect("resume token");
    let revision = handoff["expected_revision"].as_u64().expect("revision");

    let restarted = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "onboard-1",
        ],
        temp.path(),
    );
    assert_eq!(json(&restarted)["resume_token"], token);

    let revision_text = revision.to_string();
    let agreement_args = [
        "init",
        "--json",
        "--project-dir",
        &project,
        "--action",
        "agree",
        "--request-id",
        "onboard-1",
        "--expected-revision",
        &revision_text,
        "--resume",
        token,
        "--mission",
        "Keep project work aligned.",
        "--values",
        "Safety before speed.",
        "--philosophy",
        "Prefer small deterministic boundaries.",
    ];
    let accepted = run(&agreement_args, temp.path());
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let accepted_json = json(&accepted);
    assert_eq!(accepted_json["state"], "success");
    assert_eq!(
        accepted_json["data"]["records"].as_array().map(Vec::len),
        Some(3)
    );

    let replay = run(&agreement_args, temp.path());
    assert!(replay.status.success());
    assert_eq!(
        json(&replay)["data"]["records"],
        accepted_json["data"]["records"]
    );

    let conflict = run(
        &[
            "init",
            "--json",
            "--project-dir",
            &project,
            "--action",
            "agree",
            "--request-id",
            "onboard-1",
            "--expected-revision",
            &revision_text,
            "--resume",
            token,
            "--mission",
            "Different mission.",
            "--values",
            "Safety before speed.",
            "--philosophy",
            "Prefer small deterministic boundaries.",
        ],
        temp.path(),
    );
    assert_eq!(conflict.status.code(), Some(8));
    assert_eq!(json(&conflict)["state"], "conflict");
}

#[test]
fn stale_change_is_rejected_and_standard_needs_a_decision() {
    let temp = tempfile::tempdir().expect("create change fixture");
    init_git(temp.path());
    let project = temp.path().to_string_lossy();
    let probe = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--record-id",
            "guidance.local",
        ],
        temp.path(),
    );
    assert_eq!(probe.status.code(), Some(6));
    let handoff = json(&probe);
    let token = handoff["resume_token"].as_str().expect("resume token");

    let stale = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--kind",
            "guidance",
            "--record-id",
            "guidance.local",
            "--content",
            "Use explicit boundaries.",
            "--expected-revision",
            "1",
            "--resume",
            token,
        ],
        temp.path(),
    );
    assert_eq!(stale.status.code(), Some(7));
    assert_eq!(json(&stale)["state"], "stale");

    let accepted = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--kind",
            "guidance",
            "--record-id",
            "guidance.local",
            "--content",
            "Use explicit boundaries.",
            "--expected-revision",
            "0",
            "--resume",
            token,
        ],
        temp.path(),
    );
    assert!(accepted.status.success());
    let accepted_json = json(&accepted);
    let replay = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "change-stale",
            "--kind",
            "guidance",
            "--record-id",
            "guidance.local",
            "--content",
            "Use explicit boundaries.",
            "--expected-revision",
            "0",
            "--resume",
            token,
        ],
        temp.path(),
    );
    assert!(replay.status.success());
    let replay_json = json(&replay);
    assert_eq!(replay_json["data"]["idempotent_replay"], true);
    assert_eq!(
        replay_json["data"]["record"],
        accepted_json["data"]["record"]
    );

    let decision_probe = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "standard-1",
            "--record-id",
            "standard.boundary",
        ],
        temp.path(),
    );
    let decision_handoff = json(&decision_probe);
    let decision_token = decision_handoff["resume_token"]
        .as_str()
        .expect("decision token");
    let decision = run(
        &[
            "change",
            "--json",
            "--project-dir",
            &project,
            "--request-id",
            "standard-1",
            "--kind",
            "standard",
            "--record-id",
            "standard.boundary",
            "--content",
            "Do not import private modules.",
            "--expected-revision",
            "0",
            "--resume",
            decision_token,
        ],
        temp.path(),
    );
    assert_eq!(decision.status.code(), Some(5));
    assert_eq!(json(&decision)["state"], "needs_decision");
    assert_eq!(json(&decision)["data"]["recorded"], false);
}

#[test]
fn check_is_observational_and_fails_closed_without_rules() {
    let temp = tempfile::tempdir().expect("create observational fixture");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("src")).expect("create source");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source");
    let before = tree_digest(temp.path());
    let project = temp.path().to_string_lossy();
    let output = run(
        &[
            "check",
            "--json",
            "--project-dir",
            &project,
            "--path",
            "src",
        ],
        temp.path(),
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["state"], "unknown");
    assert_eq!(
        tree_digest(temp.path()),
        before,
        "check changed project or Git state"
    );
}

fn tree_digest(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            let path = entry
                .path()
                .strip_prefix(root)
                .expect("relative path")
                .to_path_buf();
            let bytes = std::fs::read(entry.path()).expect("read tree fixture");
            (path, bytes)
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

#[test]
fn validate_eval_and_self_scan_are_non_vacuous() {
    let temp = tempfile::tempdir().expect("create gate fixture");
    write_rule_project(temp.path(), "def ReadConfig():\n    pass\n");
    let root = temp.path();
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
            "python",
            "--json",
            "--no-fail",
        ],
        root,
    );
    assert!(scan.status.success());
    let scan_json = json(&scan);
    assert_eq!(scan_json["violations_count"], 1);
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

    let public_bad = run(
        &[
            "check",
            "--project-dir",
            &project,
            "--path",
            "src",
            "--lang",
            "python",
            "--json",
        ],
        temp.path(),
    );
    assert_eq!(public_bad.status.code(), Some(1));
    assert_eq!(json(&public_bad)["state"], "violated");

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

    let public_good = run(
        &[
            "check",
            "--project-dir",
            &project,
            "--path",
            "src",
            "--lang",
            "python",
            "--json",
        ],
        temp.path(),
    );
    assert!(public_good.status.success());
    let public_good_json = json(&public_good);
    assert_eq!(public_good_json["state"], "success");
    assert!(public_good_json["required_snapshot"]["policy_digest"]
        .as_str()
        .is_some_and(|digest| digest.starts_with("sha256:")));
}

#[test]
fn public_check_does_not_execute_untrusted_command_validators() {
    let temp = tempfile::tempdir().expect("create validator fixture");
    init_git(temp.path());
    let rules = temp.path().join("whetstone/rules/python");
    std::fs::create_dir_all(&rules).expect("create rules");
    std::fs::create_dir_all(temp.path().join("src")).expect("create source");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source");
    std::fs::write(
        rules.join("command.yaml"),
        r#"source:
  name: team
rules:
  - id: team.untrusted-command
    severity: must
    confidence: high
    category: convention
    description: An untrusted command must never run at the public boundary.
    source_url: https://example.com/command
    approved: true
    status: approved
    validators:
      - adapter: command
        rule: command-check
        config:
          command: "touch should-not-exist; printf '{\"violations\":[]}'"
          allow_shell: true
    golden_examples: []
"#,
    )
    .expect("write validator rule");
    let project = temp.path().to_string_lossy();
    let output = run(
        &[
            "check",
            "--json",
            "--project-dir",
            &project,
            "--path",
            "src",
            "--lang",
            "python",
        ],
        temp.path(),
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["state"], "unknown");
    assert!(!temp.path().join("should-not-exist").exists());
}

#[test]
fn command_response_schema_and_clients_cover_all_states() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let schema: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("references/command-response-v1.schema.json"))
            .expect("read response schema"),
    )
    .expect("parse response schema");
    assert_eq!(
        schema["properties"]["schema"]["const"],
        "whetstone.command-response.v1"
    );
    assert_eq!(schema["additionalProperties"], false);
    let states = schema["properties"]["state"]["enum"]
        .as_array()
        .expect("state enum")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    let expected = [
        "success",
        "violated",
        "unknown",
        "unavailable",
        "needs_decision",
        "needs_input",
        "stale",
        "conflict",
    ]
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(states, expected);
    for client in [
        "examples/whetstone-client.sh",
        "examples/whetstone_client.py",
    ] {
        let text = std::fs::read_to_string(root.join(client)).expect("read example client");
        for state in &expected {
            assert!(text.contains(state), "{client} does not handle {state}");
        }
    }
    assert!(Command::new("sh")
        .args(["-n", "examples/whetstone-client.sh"])
        .current_dir(root)
        .status()
        .expect("syntax-check shell client")
        .success());
}

fn scan_json(root: &Path, no_fail: bool) -> Output {
    let project = root.to_string_lossy();
    let mut args = vec![
        "scan",
        "src",
        "--project-dir",
        &project,
        "--lang",
        "python",
        "--json",
    ];
    if no_fail {
        args.push("--no-fail");
    }
    run(&args, root)
}

fn complete_rule(id: &str, status: &str, approved: bool, query: Option<&str>) -> String {
    let query = query
        .map(|query| format!("        ast_query: '{query}'\n"))
        .unwrap_or_default();
    format!(
        "source:\n  name: team\nrules:\n  - id: {id}\n    severity: must\n    confidence: high\n    category: convention\n    description: A deterministic test rule.\n    source_url: https://example.com/rule\n    approved: {approved}\n    status: {status}\n    signals:\n      - id: signal\n        strategy: ast\n        weight: required\n{query}    golden_examples:\n      - code: 'def good(): pass'\n        verdict: pass\n        reason: Expected pass.\n      - code: 'def Bad(): pass'\n        verdict: fail\n        reason: Expected failure.\n"
    )
}

#[test]
fn malformed_rule_inputs_are_configuration_failures() {
    let temp = tempfile::tempdir().expect("create malformed-rule fixture");
    let rules = temp.path().join("whetstone/rules/python");
    std::fs::create_dir_all(&rules).expect("create rule directory");
    std::fs::create_dir_all(temp.path().join("src")).expect("create source directory");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source fixture");
    std::fs::write(
        rules.join("valid.yaml"),
        complete_rule(
            "team.valid",
            "approved",
            true,
            Some("((function_definition name: (identifier) @match) (#match? @match \"^[A-Z]\"))"),
        ),
    )
    .expect("write valid rule");

    std::fs::write(rules.join("broken.yaml"), "source: [\nrules: nope")
        .expect("write malformed YAML");
    let malformed_yaml = scan_json(temp.path(), false);
    assert_eq!(malformed_yaml.status.code(), Some(1));
    let malformed_yaml_json = json(&malformed_yaml);
    assert_eq!(malformed_yaml_json["status"], "config_issues_found");
    assert!(
        malformed_yaml_json["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );

    std::fs::write(
        rules.join("broken.yaml"),
        complete_rule(
            "team.bad-query",
            "approved",
            true,
            Some("(function_definition"),
        ),
    )
    .expect("write malformed query");
    let malformed_query = scan_json(temp.path(), false);
    assert_eq!(malformed_query.status.code(), Some(1));
    assert!(
        json(&malformed_query)["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );
    let project = temp.path().to_string_lossy();
    let malformed_validate = run(
        &["validate", "--project-dir", &project, "--json"],
        temp.path(),
    );
    assert_eq!(malformed_validate.status.code(), Some(1));
    assert_eq!(json(&malformed_validate)["ok"], false);

    std::fs::write(
        rules.join("broken.yaml"),
        complete_rule("team.missing-query", "approved", true, None),
    )
    .expect("write missing query");
    let missing_query = scan_json(temp.path(), false);
    assert_eq!(missing_query.status.code(), Some(1));
    assert!(
        json(&missing_query)["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );
}

#[test]
fn only_consistent_approved_rules_are_loaded() {
    let temp = tempfile::tempdir().expect("create lifecycle fixture");
    let rules = temp.path().join("whetstone/rules/python");
    std::fs::create_dir_all(&rules).expect("create rule directory");
    std::fs::create_dir_all(temp.path().join("src")).expect("create source directory");
    std::fs::write(temp.path().join("src/app.py"), "def good():\n    pass\n")
        .expect("write source fixture");

    std::fs::write(
        rules.join("candidate.yaml"),
        complete_rule(
            "team.candidate",
            "candidate",
            false,
            Some("(function_definition) @match"),
        ),
    )
    .expect("write valid candidate");
    let candidate = scan_json(temp.path(), true);
    let candidate_json = json(&candidate);
    assert_eq!(candidate_json["rules_applied"], 0);
    assert_eq!(candidate_json["config_issues_count"], 0);

    std::fs::write(
        rules.join("candidate.yaml"),
        complete_rule(
            "team.inconsistent",
            "candidate",
            true,
            Some("(function_definition) @match"),
        ),
    )
    .expect("write inconsistent candidate");
    let inconsistent = scan_json(temp.path(), false);
    assert_eq!(inconsistent.status.code(), Some(1));
    assert!(
        json(&inconsistent)["config_issues_count"]
            .as_u64()
            .expect("config issue count")
            > 0
    );
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
    // Published crates deliberately exclude project/user state. Repository
    // checkouts exercise every protected record that is present.
    if protected.is_empty() {
        return;
    }
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
