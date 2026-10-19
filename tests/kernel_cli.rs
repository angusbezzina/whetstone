use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/rulebook.rs"]
mod rulebook;

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

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git in isolated fixture");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 git output")
}

fn init_git(root: &Path) {
    git(root, &["init", "--quiet"]);
    git(root, &["config", "user.name", "Test Owner"]);
    git(root, &["config", "user.email", "owner@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
}

fn git_status(root: &Path) -> String {
    git(root, &["status", "--porcelain=v1"])
}

/// A throwaway Git repository, never this one.
struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create isolated repository");
        let root = temp.path().canonicalize().expect("canonical fixture root");
        init_git(&root);
        // A test command that passes, so the prove-it-works starter is one a
        // check can hold; kept out of Git so the tree stays clean.
        std::fs::write(root.join("package.json"), r#"{"scripts":{"test":"true"}}"#)
            .expect("write package.json");
        std::fs::write(root.join(".git/info/exclude"), "package.json\n")
            .expect("exclude package.json");
        Self { _temp: temp, root }
    }

    fn wh(&self, args: &[&str]) -> Output {
        let mut all = args.to_vec();
        all.push("--json");
        run(&all, &self.root)
    }

    fn wh_json(&self, args: &[&str]) -> serde_json::Value {
        json(&self.wh(args))
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create fixture directory");
        }
        std::fs::write(full, text).expect("write fixture file");
    }

    fn commit(&self, message: &str) {
        git(&self.root, &["add", "-A"]);
        git(
            &self.root,
            &["commit", "--quiet", "--no-verify", "-m", message],
        );
    }

    /// Draft the agreement through `wh init --action agree`.
    fn agree(&self, answers: &[&str]) -> serde_json::Value {
        let mut args = vec!["init", "--action", "agree"];
        args.extend_from_slice(answers);
        self.wh_json(&args)
    }

    fn rulebook(&self) -> Option<Vec<u8>> {
        std::fs::read(self.root.join("whetstone/rulebook.json")).ok()
    }

    /// Write a rule into the rulebook, where a check reads it.
    fn rule(&self, record_id: &str, definition: &str) {
        rulebook::add_rule(
            &self.root,
            record_id,
            &format!("Statement of {record_id}."),
            &serde_json::from_str(definition).expect("definition JSON"),
        );
    }

    fn check(&self, args: &[&str]) -> (Option<i32>, serde_json::Value) {
        let mut all = vec!["check"];
        all.extend_from_slice(args);
        let output = self.wh(&all);
        (output.status.code(), json(&output))
    }
}

fn gate<'a>(response: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    response["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == id)
        .unwrap_or_else(|| panic!("no gate {id} in {response}"))
}

fn drafted(response: &serde_json::Value) -> Vec<String> {
    response["data"]["drafted"]
        .as_array()
        .unwrap_or_else(|| panic!("nothing drafted: {response}"))
        .iter()
        .map(|id| id.as_str().expect("drafted id").to_string())
        .collect()
}

const AGREEMENT: [&str; 6] = [
    "--mission",
    "Keep project work aligned.",
    "--principle",
    "prove-it-works",
    "--starter",
    "prove-it-works",
];

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
        serde_json::json!(["init", "dash", "change", "check", "explain"])
    );
    assert_eq!(value["data"]["read_only"], true);
    assert_eq!(value["data"]["rulebook"], "whetstone/rulebook.json");
    let actions = value["permitted_actions"]
        .as_array()
        .expect("permitted actions")
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        actions,
        ["wh init", "wh dash", "wh change", "wh check", "wh explain"]
    );
}

#[test]
fn public_help_exposes_exactly_five_workflow_families() {
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
    assert_eq!(commands, ["init", "dash", "change", "check", "explain"]);
}

#[test]
fn retired_commands_cannot_execute() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for command in [
        "publish", "status", "debt", "report", "mcp", "actions", "validate", "eval", "scan",
        "pull", "push",
    ] {
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
fn dashboard_inspects_a_repository_without_a_rulebook() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    init_git(root);
    let output = run(&["dash", "--json", "--request-id", "probe-1"], root);
    assert_eq!(output.status.code(), Some(0));
    let value = json(&output);
    assert_eq!(value["state"], "success");
    assert_eq!(value["request_id"], "probe-1");
    assert_eq!(value["data"]["read_only"], true);
    assert_eq!(value["data"]["rulebook"]["state"], "missing");
}

#[test]
fn removed_flags_and_change_kinds_are_rejected_before_anything_runs() {
    let repo = Repo::new();
    for args in [
        vec!["init", "--action", "agree", "--desired-outcome", "x"],
        vec!["init", "--action", "agree", "--values", "x"],
        vec!["init", "--action", "agree", "--philosophy", "x"],
        vec!["init", "--action", "agree", "--owner", "x"],
        vec!["init", "--action", "agree", "--initial-safeguard", "x"],
        vec!["init", "--action", "agree", "--gate-command", "true"],
        vec!["init", "--action", "agree", "--expected-revision", "0"],
        vec!["init", "--action", "agree", "--resume", "x"],
        vec!["init", "--action", "setup", "--yes"],
        vec!["init", "--action", "setup", "--store", "beads"],
        vec!["init", "--action", "store"],
        vec!["init", "--action", "export"],
        vec!["change", "--kind", "value"],
        vec!["change", "--kind", "philosophy"],
        vec!["change", "--kind", "metric"],
        vec!["change", "--kind", "standard"],
        vec!["change", "--kind", "guidance"],
        vec!["change", "--kind", "exception"],
        vec!["change", "--activate", "proposal.x"],
        vec!["change", "--review-triggers", "x"],
        vec!["check", "--required"],
        vec!["check", "--repair-session", "x"],
        vec!["check", "--path", "src"],
        vec!["check", "--lang", "rust"],
        vec!["change", "--migrate"],
        vec!["change", "--idempotency-key", "x"],
        vec!["change", "--expected-revision", "1"],
        vec!["change", "--resume", "x"],
        vec!["change", "--new-owner", "x"],
        vec!["change", "--source", "x"],
        vec!["change", "--expected-effect", "x"],
        vec!["change", "--impact", "x"],
        vec!["change", "--example", "x"],
        vec!["change", "--conflict", "x"],
        vec!["change", "--accept", "proposal.x"],
        vec!["dash", "--expected-snapshot", "x"],
        vec!["dash", "--history-after", "x"],
    ] {
        let output = run(&args, &repo.root);
        assert_eq!(output.status.code(), Some(2), "{args:?} unexpectedly ran");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("unexpected argument") || stderr.contains("invalid value"),
            "{args:?}: {stderr}"
        );
    }
    assert!(!repo.root.join(".git/whetstone").exists());
    assert!(git_status(&repo.root).is_empty());
}

#[test]
fn init_inspects_read_only_then_drafts_the_agreement_once() {
    let repo = Repo::new();
    repo.write("Cargo.toml", "[package]\nname = \"x\"\n");
    repo.commit("base");
    let inspect = repo.wh(&["init", "--request-id", "onboard-1"]);
    assert_eq!(inspect.status.code(), Some(6));
    let handoff = json(&inspect);
    assert_eq!(handoff["state"], "needs_input");
    assert_eq!(handoff["data"]["inspection_writes"], serde_json::json!([]));
    assert_eq!(handoff["data"]["read_only"], true);
    assert_eq!(
        handoff["data"]["progress"]["missing_decisions"],
        serde_json::json!(["mission", "rules"])
    );
    assert_eq!(handoff["data"]["progress"]["agreement_complete"], false);
    let steps = handoff["data"]["onboarding"]["steps"]
        .as_array()
        .expect("onboarding steps")
        .iter()
        .map(|step| step["key"].as_str().expect("step key"))
        .collect::<Vec<_>>();
    assert_eq!(steps, ["mission", "principles", "rules", "gates"]);
    assert!(handoff["data"]["onboarding"]["starters"]
        .as_array()
        .expect("starters")
        .iter()
        .all(|starter| starter["accepted"] == false));
    assert!(!repo.root.join(".git/whetstone").exists());
    assert!(git_status(&repo.root).is_empty());

    let answers = [
        "--mission",
        "Keep project work aligned.",
        "--principle",
        "prove-it-works",
        "--custom-principle",
        "Leave the campsite cleaner.",
        "--starter",
        "all",
    ];
    let agreed = repo.agree(&answers);
    assert_eq!(agreed["state"], "success", "{agreed}");
    let mut ids = drafted(&agreed);
    ids.sort();
    assert_eq!(ids.len(), 4, "{agreed}");
    assert_eq!(ids[0], "mission");
    assert!(ids.contains(&"principle.prove-it-works".to_string()));
    assert!(ids.iter().any(|id| id.starts_with("principle.custom-")));
    assert!(ids.contains(&"rule.prove-it-works".to_string()));
    assert_eq!(agreed["data"]["progress"]["agreement_complete"], true);
    assert_eq!(agreed["data"]["progress"]["revision"], "uncommitted");
    assert_eq!(
        agreed["data"]["progress"]["missing_decisions"],
        serde_json::json!([])
    );
    // The draft is the rulebook file in the working tree, and nothing else.
    assert_eq!(git_status(&repo.root), "?? whetstone/\n");
    let rulebook = repo.rulebook().expect("rulebook");

    let established = repo.wh_json(&["init", "--request-id", "inspect-established"]);
    assert_eq!(established["state"], "needs_input");
    assert_eq!(established["data"]["progress"]["agreement_complete"], true);
    assert!(established["data"]["onboarding"]["starters"]
        .as_array()
        .expect("starters")
        .iter()
        .all(|starter| starter["accepted"] == true));
    assert_eq!(
        established["permitted_actions"][0], "wh change --accept",
        "{established}"
    );

    // Repeating the request drafts nothing a second time.
    let replay = repo.agree(&answers);
    assert_eq!(replay["state"], "needs_input", "{replay}");
    assert_eq!(repo.rulebook().expect("rulebook"), rulebook);

    let accepted = repo.wh_json(&["change", "--accept"]);
    assert_eq!(accepted["state"], "success", "{accepted}");
    let wired = repo.wh_json(&["init", "--action", "wire", "--hooks"]);
    assert_eq!(wired["state"], "success", "{wired}");
    let complete = repo.wh_json(&["init"]);
    assert_eq!(complete["state"], "success", "{complete}");
    assert_eq!(complete["data"]["progress"]["setup_complete"], true);
}

#[test]
fn init_needs_a_mission_and_names_only_catalogue_principles_and_starters() {
    let repo = Repo::new();
    std::fs::create_dir_all(repo.root.join("whetstone/packs/team"))
        .expect("create inert import fixture");
    std::fs::write(
        repo.root.join("whetstone/packs/team/install.sh"),
        "touch executed\nexit 99\n",
    )
    .expect("write inert imported content");
    let handoff = repo.wh_json(&["init", "--request-id", "explicit-input"]);
    assert_eq!(handoff["state"], "needs_input");
    assert!(!repo.root.join(".git/whetstone").exists());
    let agree = |extra: &[&str]| {
        let mut args = vec!["init", "--action", "agree"];
        args.extend_from_slice(extra);
        repo.wh(&args)
    };

    // No mission: nothing is recorded, and the question is asked.
    let no_mission = agree(&["--starter", "all"]);
    assert_eq!(no_mission.status.code(), Some(6));
    let no_mission = json(&no_mission);
    assert_eq!(no_mission["state"], "needs_input");
    assert!(no_mission["blocking_questions"][0]
        .as_str()
        .expect("question")
        .contains("mission"));

    for (extra, needle) in [
        (
            [
                "--mission",
                "Keep work aligned.",
                "--principle",
                "not-a-principle",
            ],
            "not-a-principle is not a principle in any loaded pack",
        ),
        (
            [
                "--mission",
                "Keep work aligned.",
                "--starter",
                "rule.made-up",
            ],
            "rule.made-up is not a starter rule",
        ),
    ] {
        let output = agree(&extra);
        assert_eq!(output.status.code(), Some(6));
        let value = json(&output);
        assert!(
            value["blocking_questions"][0]
                .as_str()
                .expect("question")
                .contains(needle),
            "{value}"
        );
    }
    let blank = agree(&["--mission", "   ", "--starter", "all"]);
    assert_eq!(blank.status.code(), Some(6));

    // A refused agreement writes nothing.
    assert!(
        repo.rulebook().is_none(),
        "a refused agreement wrote the rulebook"
    );
    assert!(!repo.root.join("executed").exists());
}

#[test]
fn init_completes_an_agreement_that_has_no_rule_yet() {
    let repo = Repo::new();
    repo.write("Cargo.toml", "[package]\nname = \"x\"\n");
    repo.commit("base");
    let mission_only = repo.agree(&["--mission", "Keep work aligned."]);
    assert_eq!(mission_only["state"], "success", "{mission_only}");
    assert_eq!(drafted(&mission_only), ["mission"]);
    assert_eq!(
        mission_only["data"]["progress"]["agreement_complete"],
        false
    );
    assert_eq!(
        mission_only["data"]["progress"]["missing_decisions"],
        serde_json::json!(["rules"])
    );

    let inspect = repo.wh_json(&["init", "--request-id", "add-rules"]);
    assert_eq!(
        inspect["data"]["progress"]["missing_decisions"],
        serde_json::json!(["rules"])
    );
    // A second mission through init is refused: missions change through wh change.
    let again = repo.wh(&["init", "--action", "agree", "--mission", "Another mission."]);
    assert_eq!(again.status.code(), Some(5));
    assert_eq!(json(&again)["state"], "needs_decision");

    // Resending the same mission drafts only what is new.
    let completed = repo.agree(&[
        "--mission",
        "Keep work aligned.",
        "--starter",
        "prove-it-works",
    ]);
    assert_eq!(drafted(&completed), ["rule.prove-it-works"]);
    assert_eq!(completed["data"]["progress"]["agreement_complete"], true);
}

#[test]
fn init_dry_run_shows_the_exact_entries_and_writes_nothing() {
    let repo = Repo::new();
    let mut args = vec!["init", "--action", "agree", "--dry-run"];
    args.extend_from_slice(&AGREEMENT);
    let output = repo.wh(&args);
    assert_eq!(output.status.code(), Some(5), "{}", json(&output));
    let value = json(&output);
    assert_eq!(value["workflow"], "init");
    assert_eq!(value["data"]["dry_run"], true);
    assert_eq!(value["data"]["entries"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        value["data"]["entries"][0]["statement"],
        "Keep project work aligned."
    );
    assert!(repo.rulebook().is_none());
}

#[test]
fn init_inspect_and_cancel_from_nested_directory_are_read_only() {
    let temp = tempfile::tempdir().expect("create nested onboarding fixture");
    init_git(temp.path());
    std::fs::create_dir_all(temp.path().join("apps/web/src/components"))
        .expect("create nested project");
    std::fs::create_dir_all(temp.path().join(".github/workflows")).expect("create CI fixture");
    std::fs::write(temp.path().join("ruff.toml"), "line-length = 100\n")
        .expect("write native config");
    std::fs::write(temp.path().join(".github/workflows/ci.yml"), "name: ci\n")
        .expect("write CI config");
    std::fs::write(temp.path().join("AGENTS.md"), "# Existing instructions\n")
        .expect("write agent context");
    let nested = temp.path().join("apps/web");
    let before = git_status(temp.path());
    let native_before = [
        std::fs::read(temp.path().join("ruff.toml")).expect("read ruff config"),
        std::fs::read(temp.path().join(".github/workflows/ci.yml")).expect("read CI config"),
        std::fs::read(temp.path().join("AGENTS.md")).expect("read agent context"),
    ];

    for (request_id, action) in [("nested-inspect", None), ("nested-cancel", Some("cancel"))] {
        let mut args = vec![
            "init",
            "--json",
            "--project-dir",
            ".",
            "--request-id",
            request_id,
        ];
        if let Some(action) = action {
            args.extend(["--action", action]);
        }
        let output = run(&args, &nested);
        if action.is_some() {
            assert!(output.status.success());
            assert_eq!(json(&output)["data"]["cancelled"], true);
        } else {
            assert_eq!(output.status.code(), Some(6));
        }
        assert_eq!(git_status(temp.path()), before);
        assert!(!temp.path().join(".git/whetstone").exists());
        assert!(!nested.join(".git").exists());
    }

    let native_after = [
        std::fs::read(temp.path().join("ruff.toml")).expect("read ruff config"),
        std::fs::read(temp.path().join(".github/workflows/ci.yml")).expect("read CI config"),
        std::fs::read(temp.path().join("AGENTS.md")).expect("read agent context"),
    ];
    assert_eq!(native_after, native_before);
}

#[test]
fn rule_definitions_hold_one_strength_and_one_enforcer() {
    let repo = Repo::new();
    repo.wh(&[
        "change",
        "--kind",
        "mission",
        "--content",
        "Keep project work aligned.",
    ]);
    let rulebook = std::fs::read(repo.root.join("whetstone/rulebook.json")).expect("rulebook");
    let preview = |request_id: &str, definition: &str| {
        repo.wh(&[
            "change",
            "--request-id",
            request_id,
            "--kind",
            "rule",
            "--record-id",
            "rule.candidate",
            "--content",
            "A candidate rule.",
            "--rationale",
            "Checking the definition.",
            "--definition",
            definition,
            "--preview",
            // The shape of the definition is under test, not the probe.
            "--allow-unvalidated",
        ])
    };
    let enforcers = [
        r#"{"kind":"ast","query":"(identifier) @match","language":"python"}"#,
        r#"{"kind":"lint","tool":"ruff check .","code":"F401"}"#,
        r#"{"kind":"formatter","tool":"ruff format --check ."}"#,
        r#"{"kind":"test","command":"cargo test"}"#,
        r#"{"kind":"validator","command":"python3 scripts/check.py"}"#,
    ];
    for (index, strength) in ["must", "should"].into_iter().enumerate() {
        for (offset, enforcer) in enforcers.iter().enumerate() {
            let definition =
                format!(r#"{{"type":"rule","strength":"{strength}","enforcer":{enforcer}}}"#);
            let output = preview(&format!("family-{index}-{offset}"), &definition);
            assert_eq!(
                output.status.code(),
                Some(5),
                "{strength} {enforcer}: {}",
                json(&output)
            );
            let value = json(&output);
            let rule = &value["data"]["diff"]["after"];
            assert_eq!(rule["strength"], strength);
            let expected: serde_json::Value = serde_json::from_str(enforcer).expect("enforcer");
            assert_eq!(rule["enforcer"]["kind"], expected["kind"]);
        }
    }

    // Two enforcers, an unknown kind or strength, or stray fields never parse.
    for definition in [
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"},"enforcers":[{"kind":"lint","tool":"ruff","code":"F401"}]}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"},"enforcer":{"kind":"lint","tool":"ruff","code":"F401"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"review","reviewer":{"by":"interrogate"}}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"brief","skills":["how"]}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"question","question":"Is this a test?"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"public_surface"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"drive","feature":"feature.search"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true","query":"(identifier)"}}"#,
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"vibes"}}"#,
        r#"{"type":"rule","strength":"may","enforcer":{"kind":"test","command":"true"}}"#,
        r#"{"type":"rule","strength":"advisory","enforcer":{"kind":"test","command":"true"}}"#,
        r#"{"type":"rule","strength":"must"}"#,
        r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"test","command_ref":"cargo test"}}"#,
    ] {
        let output = run(
            &[
                "change",
                "--json",
                "--kind",
                "rule",
                "--record-id",
                "rule.bad",
                "--content",
                "Bad.",
                "--rationale",
                "Bad.",
                "--definition",
                definition,
            ],
            &repo.root,
        );
        assert_eq!(output.status.code(), Some(2), "{definition} parsed");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("invalid change definition"),
            "{definition}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Well-formed but invalid enforcers are asked about, never recorded.
    for (index, enforcer) in [
        r#"{"kind":"test","command":"cargo test | tee out"}"#,
        r#"{"kind":"lint","tool":"ruff","code":""}"#,
    ]
    .into_iter()
    .enumerate()
    {
        let definition = format!(r#"{{"type":"rule","strength":"must","enforcer":{enforcer}}}"#);
        let output = preview(&format!("invalid-{index}"), &definition);
        assert_eq!(output.status.code(), Some(6), "{enforcer} was accepted");
        assert_eq!(json(&output)["state"], "needs_input");
    }
    assert_eq!(
        std::fs::read(repo.root.join("whetstone/rulebook.json")).expect("rulebook"),
        rulebook,
        "a rejected definition was written"
    );

    // A rule without a definition is asked for its strength and enforcer.
    let missing = repo.wh_json(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.vague",
        "--content",
        "Be good.",
        "--rationale",
        "The owner wants this.",
    ]);
    assert_eq!(missing["state"], "needs_input");
    assert!(missing["blocking_questions"][0]
        .as_str()
        .expect("question")
        .contains("strength"));
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
    let output = run(&["check", "--json", "--project-dir", &project], temp.path());
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
fn a_check_writes_no_record() {
    let repo = Repo::new();
    repo.write("src/app.py", "def read_config():\n    pass\n");
    repo.commit("base");
    repo.rule("rule.lowercase-functions", UPPERCASE_FUNCTIONS);

    let args = ["--request-id", "check-idempotent"];
    let before = (git_status(&repo.root), repo.rulebook());
    let (_, first) = repo.check(&args);
    let (_, second) = repo.check(&args);
    assert_eq!(first["state"], "success", "{first}");
    assert_eq!(second["state"], "success", "{second}");
    assert_eq!(
        first["data"]["report"]["state"],
        second["data"]["report"]["state"]
    );
    assert_eq!(
        (git_status(&repo.root), repo.rulebook()),
        before,
        "a check is its report and changes nothing"
    );
}

#[test]
fn command_response_schema_covers_all_states() {
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
}

const UPPERCASE_FUNCTIONS: &str = r#"{"type":"rule","strength":"must","enforcer":{"kind":"ast","language":"python","query":"((function_definition name: (identifier) @match) (#match? @match \"^[A-Z]\"))"}}"#;

#[test]
fn staged_check_reads_the_index_and_ignores_unstaged_edits() {
    let repo = Repo::new();
    repo.write("src/app.py", "def read_config():\n    pass\n");
    repo.commit("base");
    repo.rule("rule.lowercase-functions", UPPERCASE_FUNCTIONS);

    // Stage a good edit, then leave a bad edit unstaged on top of it.
    repo.write("src/app.py", "def read_config():\n    return 1\n");
    git(&repo.root, &["add", "src/app.py"]);
    repo.write("src/app.py", "def ReadConfig():\n    return 1\n");

    let (code, staged) = repo.check(&["--staged"]);
    assert_eq!(code, Some(0), "{staged}");
    assert_eq!(staged["state"], "success");
    assert_eq!(staged["data"]["selection"]["mode"], "staged");
    assert_eq!(
        staged["data"]["selection"]["changed_paths"],
        serde_json::json!(["src/app.py"])
    );
    assert_eq!(gate(&staged, "rule.lowercase-functions")["state"], "pass");

    // The working tree has the violation, and a full check says so.
    let (code, working) = repo.check(&["--rule", "rule.lowercase-functions"]);
    assert_eq!(code, Some(1), "{working}");
    assert_eq!(working["state"], "violated");
    let failing = gate(&working, "rule.lowercase-functions");
    assert_eq!(failing["state"], "fail");
    assert!(failing["failures"][0]["location"]
        .as_str()
        .expect("location")
        .starts_with("src/app.py"));

    // Once staged, pre-commit sees it too.
    git(&repo.root, &["add", "src/app.py"]);
    let (code, staged_bad) = repo.check(&["--staged"]);
    assert_eq!(code, Some(1), "{staged_bad}");
    assert_eq!(
        gate(&staged_bad, "rule.lowercase-functions")["state"],
        "fail"
    );
}

#[test]
fn a_should_rule_failure_is_a_flag_and_does_not_fail_the_check() {
    let repo = Repo::new();
    repo.write("README.md", "fixture\n");
    repo.commit("base");
    repo.rule(
        "rule.must-pass",
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"}}"#,
    );
    repo.rule(
        "rule.should-fail",
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"test","command":"false"}}"#,
    );
    let (code, checked) = repo.check(&[]);
    assert_eq!(code, Some(0), "{checked}");
    assert_eq!(checked["state"], "success");
    assert_eq!(
        checked["data"]["flags"],
        serde_json::json!(["rule.should-fail"])
    );
    assert_eq!(gate(&checked, "rule.must-pass")["state"], "pass");
    let should = gate(&checked, "rule.should-fail");
    assert_eq!(should["state"], "fail");
    assert_eq!(should["strength"], "should");
    assert!(checked["summary"]
        .as_str()
        .expect("summary")
        .contains("rule.should-fail"));

    // The same must rule failing does fail the check.
    repo.rule(
        "rule.must-fail",
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"false"}}"#,
    );
    let (code, failed) = repo.check(&[]);
    assert_eq!(code, Some(1), "{failed}");
    assert_eq!(failed["state"], "violated");
    assert_eq!(
        failed["data"]["flags"],
        serde_json::json!(["rule.should-fail"])
    );
}

#[test]
fn a_check_of_only_should_rules_lists_the_flag() {
    let repo = Repo::new();
    repo.rule(
        "rule.should-fail",
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"test","command":"false"}}"#,
    );
    let (code, checked) = repo.check(&["--rule", "rule.should-fail"]);
    assert_eq!(code, Some(0), "{checked}");
    assert_eq!(
        checked["data"]["flags"],
        serde_json::json!(["rule.should-fail"])
    );
}

const UNWRAP_QUERY: &str = r#"((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method \"unwrap\"))"#;

/// Draft `rule.probed` with this definition through `wh change`.
fn draft_probed(repo: &Repo, definition: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.probed",
        "--content",
        "No unwrap in the source.",
        "--rationale",
        "A panic is not an error message.",
        "--definition",
        definition,
    ];
    args.extend_from_slice(extra);
    repo.wh_json(&args)
}

#[test]
fn every_rule_is_probed_before_it_is_written_and_what_cannot_be_checked_here_is_refused() {
    let repo = Repo::new();
    repo.write(
        "src/main.rs",
        "fn main() {\n    let x: Option<u8> = None;\n    x.unwrap();\n}\n",
    );
    repo.commit("source");
    repo.wh(&["change", "--kind", "mission", "--content", "Ship."]);

    // A preview shows where the rule matches and writes nothing.
    let matching = format!(
        r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["src/**"]}}"#
    );
    let shown = draft_probed(&repo, &matching, &["--preview"]);
    assert_eq!(shown["state"], "needs_decision", "{shown}");
    let report = &shown["data"]["probes"][0]["probe"];
    assert_eq!(shown["data"]["probes"][0]["record_id"], "rule.probed");
    assert_eq!(report["validatable"], true, "{report}");
    assert_eq!(report["ran"], true);
    assert_eq!(report["files_in_scope"], 1);
    assert_eq!(report["matched"], 1);
    assert_eq!(report["samples"][0]["location"], "src/main.rs:3");
    let before = repo.rulebook().expect("rulebook");
    let outside = tempfile::tempdir().expect("outside");
    let outside_script = outside.path().join("pass.sh");
    std::fs::write(&outside_script, "#!/bin/sh\nexit 0\n").expect("outside script");

    // Each refusal names its reason and writes nothing.
    for (definition, reason) in [
        (
            r#"{"type":"rule","strength":"must","enforcer":{"kind":"ast","query":"(call_expression @match"}}"#.to_string(),
            "does not parse",
        ),
        (
            r#"{"type":"rule","strength":"should","enforcer":{"kind":"lint","tool":"ruff check .","code":"not_a_code"}}"#.to_string(),
            "is not a ruff code",
        ),
        (
            format!(
                r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}"}},"paths":["lib/**"]}}"#
            ),
            "no file in the repository matches",
        ),
        (
            r#"{"type":"rule","strength":"must","enforcer":{"kind":"validator","command":"sh whetstone/checks/missing.sh"}}"#.to_string(),
            "does not exist; write it first",
        ),
        (
            format!(
                r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"validator","command":"{}"}}}}"#,
                outside_script.display()
            ),
            "escapes the repository",
        ),
    ] {
        let refused = draft_probed(&repo, &definition, &[]);
        assert_eq!(refused["state"], "needs_input", "{refused}");
        assert_eq!(refused["data"]["written"], false, "{refused}");
        let probe = &refused["data"]["probes"][0]["probe"];
        assert_eq!(probe["validatable"], false, "{refused}");
        let why = probe["reason"].as_str().expect("reason");
        assert!(why.contains(reason), "{definition}: {why}");
        assert_eq!(
            repo.rulebook().expect("rulebook"),
            before,
            "a refused probe wrote the rulebook"
        );
    }

    // The matching rule is written, with its probe.
    let written = draft_probed(&repo, &matching, &[]);
    assert_eq!(written["state"], "success", "{written}");
    assert_eq!(written["data"]["probes"][0]["probe"]["matched"], 1);
    assert!(written["summary"]
        .as_str()
        .expect("summary")
        .contains("matched 1 place"));

    // A statement edit does not re-aim the rule, so nothing is probed again.
    let reworded = repo.wh_json(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.probed",
        "--content",
        "Never unwrap in the source.",
        "--rationale",
        "A panic is not an error message.",
        "--definition",
        &matching,
    ]);
    assert_eq!(reworded["state"], "success", "{reworded}");
    assert_eq!(reworded["data"]["probes"], serde_json::json!([]));

    // With --allow-unvalidated a rule that cannot be checked here is written.
    let empty = format!(
        r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}"}},"paths":["lib/**"]}}"#
    );
    let allowed = draft_probed(&repo, &empty, &["--allow-unvalidated"]);
    assert_eq!(allowed["state"], "success", "{allowed}");
    assert_eq!(allowed["data"]["probes"][0]["probe"]["validatable"], false);
}

const CONTRACT_SCRIPT: &str = r#"#!/bin/sh
echo "MODE=$WH_MODE RULE=$WH_RULE BASE=$WH_BASE"
found=0
while IFS= read -r path; do
  echo "PATH=$path"
  if grep -n FORBIDDEN "$path" | sed "s|^\([0-9]*\):.*|$path:\1 FORBIDDEN is not allowed|" | grep .; then
    found=1
  fi
  case "$path" in
    *.whole) echo "$path: the whole file is wrong"; found=1 ;;
  esac
done < "$WH_PATHS"
exit $found
"#;

#[test]
fn a_script_enforcer_reads_the_in_scope_files_and_reports_located_findings() {
    let repo = Repo::new();
    repo.write("whetstone/checks/no-forbidden.sh", CONTRACT_SCRIPT);
    repo.write("a.txt", "fine\n");
    repo.write("b.txt", "fine\n");
    repo.commit("base");
    repo.rule(
        "rule.script",
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"validator","command":"sh whetstone/checks/no-forbidden.sh"},"paths":["*.txt","*.whole"]}"#,
    );
    repo.commit("rulebook");

    // Staged mode: the script sees only the staged file, and its
    // `path:line message` line becomes a located finding.
    repo.write("a.txt", "fine\nFORBIDDEN\n");
    repo.write("b.txt", "FORBIDDEN but not staged\n");
    git(&repo.root, &["add", "a.txt"]);
    let (code, staged) = repo.check(&["--staged"]);
    assert_eq!(code, Some(1), "{staged}");
    let script = gate(&staged, "rule.script");
    assert_eq!(script["state"], "fail", "{script}");
    assert_eq!(script["failures"][0]["location"], "a.txt:2", "{script}");
    let log = gate_log(&staged, script);
    assert!(log.contains("MODE=staged RULE=rule.script BASE="), "{log}");
    assert!(log.contains("PATH=a.txt"), "{log}");
    assert!(
        !log.contains("PATH=b.txt"),
        "unstaged files are not in WH_PATHS: {log}"
    );
    assert!(
        !log.contains("PATH=whetstone/checks"),
        "files outside the rule's paths are not in WH_PATHS: {log}"
    );

    // A `path: message` line is a finding about the whole file.
    repo.write("a.txt", "fine\n");
    repo.write("c.whole", "anything\n");
    git(&repo.root, &["add", "a.txt", "c.whole"]);
    let (_, whole) = repo.check(&["--staged"]);
    let script = gate(&whole, "rule.script");
    assert_eq!(script["state"], "fail", "{script}");
    assert_eq!(script["failures"][0]["location"], "c.whole", "{script}");

    // After the fix the gate passes.
    git(&repo.root, &["rm", "--cached", "--quiet", "c.whole"]);
    std::fs::remove_file(repo.root.join("c.whole")).expect("remove");
    let (code, fixed) = repo.check(&["--staged"]);
    assert_eq!(code, Some(0), "{fixed}");
    assert_eq!(gate(&fixed, "rule.script")["state"], "pass");

    // A full check tells the script it sees the whole repository.
    let (_, all) = repo.check(&["--rule", "rule.script"]);
    let log = gate_log(&all, gate(&all, "rule.script"));
    assert!(log.contains("MODE=all"), "{log}");
    assert!(log.contains("PATH=b.txt"), "{log}");
}

/// The gate's own output log, read from the evidence root.
fn gate_log(response: &serde_json::Value, gate: &serde_json::Value) -> String {
    let root = PathBuf::from(response["data"]["evidence_root"].as_str().expect("root"));
    let locator = gate["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .filter_map(|item| item["locator"].as_str())
        .find(|locator| locator.ends_with(".log"))
        .expect("log evidence");
    std::fs::read_to_string(root.join(locator)).expect("gate log")
}

#[test]
fn wire_writes_the_agent_contract_and_the_script_scaffold_once() {
    let repo = Repo::new();
    repo.write("CLAUDE.md", "See AGENTS.md.\n");
    repo.write("AGENTS.md", "# Our agents\n\nTeam text stays.\n");
    repo.commit("base");

    let wired = repo.wh_json(&["init", "--action", "wire", "--hooks"]);
    assert_eq!(wired["state"], "success", "{wired}");
    assert_eq!(wired["data"]["agent_contract"]["status"], "written");
    let agents = std::fs::read_to_string(repo.root.join("AGENTS.md")).expect("AGENTS.md");
    assert!(
        agents.starts_with(
            "# Our agents\n\nTeam text stays.\n\n<!-- whetstone:agent-contract:start -->"
        ),
        "{agents}"
    );
    assert!(agents.contains("wh check --staged --json"));
    assert!(agents.contains("--preview"));
    assert!(agents.contains("whetstone/checks/README.md"));
    let block_lines = agents
        .lines()
        .skip_while(|line| !line.contains("agent-contract:start"))
        .take_while(|line| !line.contains("agent-contract:end"))
        .count();
    assert!(block_lines < 25, "the contract is {block_lines} lines");
    assert_eq!(
        std::fs::read_to_string(repo.root.join("CLAUDE.md")).expect("CLAUDE.md"),
        "See AGENTS.md.\n",
        "a CLAUDE.md that links to AGENTS.md is left alone"
    );
    assert!(repo.root.join("whetstone/checks/README.md").is_file());
    assert!(repo
        .root
        .join("whetstone/checks/no-conflict-markers.sh")
        .is_file());

    // A second wire changes nothing.
    let again = repo.wh_json(&["init", "--action", "wire", "--hooks"]);
    assert_eq!(
        again["data"]["agent_contract"]["status"], "unchanged",
        "{again}"
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("AGENTS.md")).expect("AGENTS.md"),
        agents
    );

    // Text inside the markers is Whetstone's; text outside is the team's.
    let edited =
        agents.replace("every agent follows them", "EDITED") + "\n## Added later\n\nAlso kept.\n";
    std::fs::write(repo.root.join("AGENTS.md"), &edited).expect("edit");
    std::fs::write(repo.root.join("whetstone/checks/README.md"), "ours now\n").expect("edit");
    let rewired = repo.wh_json(&["init", "--action", "wire", "--hooks"]);
    assert_eq!(rewired["data"]["agent_contract"]["status"], "updated");
    let fixed = std::fs::read_to_string(repo.root.join("AGENTS.md")).expect("AGENTS.md");
    assert!(!fixed.contains("EDITED"));
    assert!(fixed.contains("## Added later\n\nAlso kept.\n"));
    assert!(fixed.starts_with("# Our agents\n\nTeam text stays.\n"));
    assert_eq!(
        std::fs::read_to_string(repo.root.join("whetstone/checks/README.md")).expect("README"),
        "ours now\n",
        "the scaffold is the team's once written"
    );

    // Once the scaffold exists, init offers its example as a starter rule,
    // and the example's probe passes on this repository.
    let inspect = repo.wh_json(&["init"]);
    assert!(
        inspect.to_string().contains("rule.no-conflict-markers"),
        "{inspect}"
    );
    let agreed = repo.agree(&[
        "--mission",
        "Keep project work aligned.",
        "--starter",
        "rule.no-conflict-markers",
    ]);
    assert_eq!(agreed["state"], "success", "{agreed}");
    assert_eq!(agreed["data"]["probes"][0]["probe"]["validatable"], true);
}

#[test]
fn explain_resolves_a_sweep_finding_and_a_rule_id_to_the_rule_and_the_commits_that_changed_it() {
    let repo = Repo::new();
    repo.write(
        "src/main.rs",
        "fn main() {\n    let x: Option<u8> = None;\n    x.unwrap();\n}\n",
    );
    repo.commit("source");
    repo.wh(&["change", "--kind", "mission", "--content", "Ship."]);
    let definition = |priority: u8| {
        format!(
            r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["src/**"],"sweep_priority":{priority}}}"#
        )
    };
    let draft = |priority: u8| {
        repo.wh_json(&[
            "change",
            "--kind",
            "rule",
            "--record-id",
            "rule.no-unwrap",
            "--content",
            "Prefer handling an error to unwrap.",
            "--rationale",
            "A panic is not an error message.",
            "--definition",
            &definition(priority),
        ])
    };
    assert_eq!(draft(2)["state"], "success");
    // A draft has no decision yet.
    let drafted = repo.wh_json(&["explain", "rule.no-unwrap"]);
    assert_eq!(drafted["data"]["rule"]["lifecycle"], "draft", "{drafted}");
    assert_eq!(drafted["data"]["decisions"], serde_json::json!([]));
    let accepted = repo.wh_json(&[
        "change",
        "--accept",
        "--rationale",
        "We keep panics out of the parser.",
    ]);
    assert_eq!(accepted["state"], "success", "{accepted}");
    // An unrelated rulebook commit is not a decision on this rule.
    repo.wh(&["change", "--kind", "mission", "--content", "Ship it."]);
    repo.wh(&["change", "--accept"]);
    assert_eq!(draft(1)["state"], "success");
    repo.wh(&["change", "--accept", "--rationale", "It matters more now."]);

    let (_, sweep) = repo.check(&["--sweep", "--format", "json"]);
    let finding = sweep["data"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| finding["responsibility"] == "rule.no-unwrap")
        .unwrap_or_else(|| panic!("no finding in {sweep}"))
        .clone();
    let fingerprint = finding["fingerprint"].as_str().expect("fingerprint");

    // Recomputed from the repository: no findings file is needed.
    let explained = repo.wh_json(&["explain", fingerprint]);
    assert_eq!(explained["state"], "success", "{explained}");
    assert_eq!(explained["data"]["resolved"]["kind"], "fingerprint");
    assert_eq!(explained["data"]["resolved"]["subject"], "src/main.rs");
    assert_eq!(explained["data"]["resolved"]["resolved_from"], "recomputed");
    let rule = &explained["data"]["rule"];
    assert_eq!(rule["id"], "rule.no-unwrap");
    assert_eq!(rule["lifecycle"], "accepted");
    assert_eq!(rule["strength"], "should");
    assert_eq!(rule["enforcer"]["kind"], "ast");
    assert_eq!(rule["paths"], serde_json::json!(["src/**"]));
    assert_eq!(rule["sweep"]["class"], "discuss");
    assert_eq!(rule["sweep"]["priority"], 1);
    let decisions = explained["data"]["decisions"]
        .as_array()
        .expect("decisions");
    assert_eq!(decisions.len(), 2, "{explained}");
    assert_eq!(decisions[0]["change"], "added");
    assert_eq!(decisions[0]["by"], "Test Owner");
    assert_eq!(
        decisions[0]["rationale"],
        "We keep panics out of the parser."
    );
    assert_eq!(decisions[1]["change"], "changed");
    assert_eq!(decisions[1]["rationale"], "It matters more now.");
    assert_eq!(explained["data"]["created"], decisions[0]);
    assert_eq!(explained["data"]["last_changed"], decisions[1]);

    // From the sweep's findings file, and by a short prefix.
    let file = repo.root.join("sweep-findings.json");
    std::fs::write(
        &file,
        sweep["data"]["findings_json"].as_str().expect("json"),
    )
    .expect("write");
    let short = &fingerprint[..19];
    let from_file = repo.wh_json(&["explain", short, "--findings", file.to_str().expect("path")]);
    assert_eq!(from_file["state"], "success", "{from_file}");
    assert_eq!(from_file["data"]["rule"]["id"], "rule.no-unwrap");
    assert!(from_file["data"]["resolved"]["resolved_from"]
        .as_str()
        .expect("source")
        .ends_with("sweep-findings.json"));

    // The text a person reads at a terminal.
    let human = explained["data"]["text"].as_str().expect("text");
    assert!(
        human.contains("Rule rule.no-unwrap · should · accepted"),
        "{human}"
    );
    assert!(
        human.contains("added by Test Owner at ") && human.contains(": It matters more now."),
        "{human}"
    );

    // Something that is neither is asked about.
    let unknown = repo.wh_json(&["explain", "sha256:0000000000000000"]);
    assert_eq!(unknown["state"], "needs_input");
    assert!(unknown["summary"]
        .as_str()
        .expect("summary")
        .contains("No rule here produces the finding"));
}

const TEAM_PACK: &str = r#"{
  "schema": "whetstone.principle-pack.v1",
  "name": "team",
  "version": "2.1.0",
  "source": "our engineering handbook",
  "license": "internal",
  "principles": [
    {"id": "ship-small", "group": "delivery", "text": "Ship in changes a reviewer reads in ten minutes.", "enforcer_hint": "validator", "rationale": "Small changes are reviewed well."}
  ]
}"#;

#[test]
fn a_repository_principle_pack_is_offered_beside_pstack_and_written_with_its_version() {
    let repo = Repo::new();
    repo.write("whetstone/principles/team.json", TEAM_PACK);
    repo.commit("pack");
    let inspect = repo.wh_json(&["init"]);
    let onboarding = &inspect["data"]["onboarding"];
    let offered = onboarding["catalogue"]
        .as_array()
        .unwrap_or_else(|| panic!("no catalogue in {inspect}"));
    assert!(offered
        .iter()
        .any(|entry| entry["spec"] == "team/ship-small"
            && entry["pack_version"] == "2.1.0"
            && entry["enforcer_hint"] == "validator"));
    assert!(offered
        .iter()
        .any(|entry| entry["spec"] == "prove-it-works"));
    assert_eq!(onboarding["catalogue_version"], "pstack 0.15.5, team 2.1.0");

    let unknown = repo.agree(&[
        "--mission",
        "Keep work aligned.",
        "--principle",
        "team/nope",
    ]);
    assert_eq!(unknown["state"], "needs_input", "{unknown}");
    let question = unknown.to_string();
    assert!(
        question.contains("not a principle in any loaded pack"),
        "{question}"
    );

    let agreed = repo.agree(&[
        "--mission",
        "Keep work aligned.",
        "--principle",
        "team/ship-small",
        "--principle",
        "prove-it-works",
    ]);
    assert_eq!(agreed["state"], "success", "{agreed}");
    let rulebook: serde_json::Value =
        serde_json::from_slice(&repo.rulebook().expect("rulebook")).expect("rulebook JSON");
    let team = rulebook["principles"]
        .as_array()
        .expect("principles")
        .iter()
        .find(|principle| principle["id"] == "principle.team.ship-small")
        .unwrap_or_else(|| panic!("no team principle in {rulebook}"));
    assert_eq!(
        team["source"],
        serde_json::json!({"kind": "pack", "pack": "team", "id": "ship-small", "version": "2.1.0"})
    );
    assert_eq!(team["rationale"], "Small changes are reviewed well.");
    assert!(rulebook["principles"]
        .as_array()
        .expect("principles")
        .iter()
        .any(|principle| principle["id"] == "principle.prove-it-works"));

    // The same principle through wh change, by its pack and id.
    let changed = repo.wh_json(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.small",
        "--content",
        "Ship small.",
        "--definition",
        r#"{"type":"principle","principle":"team/ship-small"}"#,
    ]);
    assert_eq!(changed["state"], "success", "{changed}");
    assert_eq!(changed["data"]["diff"]["after"]["source"]["kind"], "pack");

    // A pack that repeats an id another pack holds is refused and reported.
    repo.write(
        "whetstone/principles/clash.json",
        &TEAM_PACK.replace("\"team\"", "\"clash\""),
    );
    let inspect = repo.wh_json(&["init"]);
    let refused = &inspect["data"]["onboarding"]["refused_packs"];
    assert!(
        refused[0]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("unique across packs")),
        "{inspect}"
    );
}

const CONTRIBUTING: &str = "# Contributing\n\nThanks for helping out.\n\n## Rules\n\n- Never call `.unwrap()` in `src`.\n- Never leave a `dbg!` in the code.\n- Every TODO must name its issue; `whetstone/checks/todo-issue.sh` checks it.\n- `whetstone/checks/missing.sh` must pass before a release.\n- Keep functions short enough to read on one screen.\n- Prefer early returns over nested conditionals.\n";

const TODO_SCRIPT: &str = r#"#!/bin/sh
found=0
while IFS= read -r path; do
  grep -n "TODO" "$path" | grep -v "TODO(#" | cut -d: -f1 | while read -r line; do echo "$path:$line TODO without an issue"; done | grep . && found=1
done < "$WH_PATHS"
exit $found
"#;

#[test]
fn prose_compiles_into_probed_drafts_and_candidates_that_need_an_enforcer() {
    let repo = Repo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    repo.write(
        "src/main.rs",
        "fn main() {\n    // TODO tidy\n    let x: Option<u8> = None;\n    x.unwrap();\n}\n",
    );
    repo.write("whetstone/checks/todo-issue.sh", TODO_SCRIPT);
    repo.write("CONTRIBUTING.md", CONTRIBUTING);
    repo.commit("base");
    repo.wh(&["change", "--kind", "mission", "--content", "Ship."]);
    let before = repo.rulebook().expect("rulebook");

    // A preview shows everything and writes nothing.
    let preview = repo.wh_json(&["change", "--from-file", "CONTRIBUTING.md", "--preview"]);
    assert_eq!(preview["state"], "needs_decision", "{preview}");
    assert_eq!(preview["data"]["candidates"], 6, "{preview}");
    assert_eq!(preview["data"]["written"], false);
    assert_eq!(preview["data"]["drafts"].as_array().map(Vec::len), Some(3));
    assert_eq!(repo.rulebook().expect("rulebook"), before);

    let compiled = repo.wh_json(&["change", "--kind", "rule", "--from-file", "CONTRIBUTING.md"]);
    assert_eq!(compiled["state"], "success", "{compiled}");
    let data = &compiled["data"];
    let drafts = data["drafts"].as_array().expect("drafts");
    let lines = |entries: &serde_json::Value| {
        entries
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| entry["line"].as_u64().expect("line"))
            .collect::<Vec<_>>()
    };
    assert_eq!(lines(&data["drafts"]), [7, 8, 9], "{compiled}");
    assert_eq!(lines(&data["not_validatable"]), [10], "{compiled}");
    assert_eq!(lines(&data["needs_enforcer"]), [11, 12], "{compiled}");
    let unwrap = &drafts[0];
    assert_eq!(unwrap["enforcer"]["kind"], "ast");
    assert_eq!(unwrap["strength"], "must");
    assert_eq!(unwrap["paths"], serde_json::json!(["src/**"]));
    assert_eq!(unwrap["probe"]["matched"], 1);
    assert_eq!(drafts[1]["probe"]["matched"], 0);
    assert_eq!(
        drafts[2]["enforcer"]["command"],
        "sh whetstone/checks/todo-issue.sh"
    );
    assert!(
        drafts[2]["probe"]["samples"]
            .as_array()
            .expect("samples")
            .iter()
            .any(|sample| sample["location"] == "src/main.rs:2"),
        "{compiled}"
    );
    assert!(data["not_validatable"][0]["probe"]["reason"]
        .as_str()
        .expect("reason")
        .contains("does not exist"));
    assert!(data["needs_enforcer"][0]["next"]
        .as_str()
        .expect("next")
        .contains("--preview"));

    // The drafts are in the rulebook file, with the prose as their source.
    let rulebook: serde_json::Value =
        serde_json::from_slice(&repo.rulebook().expect("rulebook")).expect("rulebook JSON");
    let id = unwrap["record_id"].as_str().expect("id");
    let rule = rulebook["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in {rulebook}"));
    assert_eq!(rule["source"]["kind"], "prose");
    assert_eq!(rule["source"]["reference"], "CONTRIBUTING.md:7");
    assert!(rule["rationale"]
        .as_str()
        .expect("rationale")
        .contains("Never call `.unwrap()`"));
    let (code, checked) = repo.check(&["--rule", id]);
    assert_eq!(code, Some(1), "{checked}");
    assert_eq!(
        gate(&checked, id)["failures"][0]["location"],
        "src/main.rs:4"
    );

    // The same prose again writes nothing new.
    let written = repo.rulebook();
    let again = repo.wh_json(&["change", "--from-file", "CONTRIBUTING.md"]);
    assert_eq!(again["state"], "needs_input", "{again}");
    assert_eq!(
        again["data"]["already"].as_array().map(Vec::len),
        Some(3),
        "{again}"
    );
    assert_eq!(repo.rulebook(), written, "a replay wrote new drafts");
}

#[test]
fn a_recurring_finding_becomes_a_must_draft_with_the_flagged_line_as_its_example() {
    let repo = Repo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    repo.write(
        "src/core/main.rs",
        "fn main() {\n    let x: Option<u8> = None;\n    x.unwrap();\n}\n",
    );
    repo.commit("base");
    repo.rule(
        "rule.unwrap-advice",
        &format!(
            r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}}}}"#
        ),
    );
    let (_, sweep) = repo.check(&["--sweep", "--format", "json"]);
    let fingerprint = sweep["data"]["findings"][0]["fingerprint"]
        .as_str()
        .unwrap_or_else(|| panic!("no finding in {sweep}"))
        .to_string();

    // The dashboard's form of the request names the rule and location.
    let located = repo.wh_json(&[
        "change",
        "--from-finding",
        "rule.unwrap-advice@src/core/main.rs:3",
        "--preview",
    ]);
    assert_eq!(located["state"], "needs_decision", "{located}");
    assert_eq!(located["data"]["drafts"][0]["probe"]["validatable"], true);

    let drafted = repo.wh_json(&["change", "--from-finding", &fingerprint]);
    assert_eq!(drafted["state"], "success", "{drafted}");
    let draft = &drafted["data"]["drafts"][0];
    assert_eq!(draft["strength"], "must");
    assert_eq!(draft["paths"], serde_json::json!(["src/core/**"]));
    assert_eq!(draft["probe"]["matched"], 1);
    let id = draft["record_id"].as_str().expect("id");
    let rulebook: serde_json::Value =
        serde_json::from_slice(&repo.rulebook().expect("rulebook")).expect("rulebook JSON");
    let rule = rulebook["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["id"] == id)
        .unwrap_or_else(|| panic!("{id} is not in {rulebook}"));
    assert_eq!(rule["source"]["kind"], "finding");
    assert_eq!(rule["examples"][0]["input"], "x.unwrap();");
    assert_eq!(rule["examples"][0]["path"], "src/core/main.rs");
}

#[cfg(unix)]
#[test]
fn the_runner_reads_a_findings_file_and_plans_work_by_class_without_changing_anything() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new();
    let tools = repo.root.join(".git/fake-bin");
    std::fs::create_dir_all(&tools).expect("bin");
    // A gh that knows no issues or pull requests and refuses to change any.
    std::fs::write(
        tools.join("gh"),
        "#!/bin/sh\ncase \"$2\" in list) echo '[]' ;; *) echo \"unexpected gh $*\" >&2; exit 1 ;; esac\n",
    )
    .expect("gh");
    std::fs::set_permissions(tools.join("gh"), std::fs::Permissions::from_mode(0o755))
        .expect("chmod");
    let fingerprint = format!("sha256:{}", "a".repeat(64));
    repo.write(
        "findings.json",
        &format!(
            r#"[{{"fingerprint":"{fingerprint}","subjectDigest":"sha256:b","class":"confirm_first","priority":1,"title":"No unwrap (src/lib.rs)","responsibility":"rule.no-unwrap"}},
{{"fingerprint":"sha256:c","subjectDigest":"sha256:d","class":"discuss","priority":2,"title":"Doubtful","responsibility":"rule.no-unwrap","unsure":"borderline"}}]"#
        ),
    );
    let path = format!(
        "{}:{}",
        tools.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let output = Command::new(bin())
        .args(["act", "--json", "--dry-run"])
        .current_dir(&repo.root)
        .env("PATH", path)
        .output()
        .expect("act");
    let report = json(&output);
    assert_eq!(report["workflow"], "act", "{report}");
    assert_eq!(report["state"], "needs_decision", "{report}");
    assert_eq!(
        report["data"]["planned"],
        serde_json::json!([{"fingerprint": fingerprint, "class": "confirm_first", "action": "create"}])
    );
    assert_eq!(report["data"]["parked"], serde_json::json!(["sha256:c"]));
}

#[test]
fn the_dashboard_shows_where_the_rulebook_is_thin_and_exports_a_static_page() {
    let repo = Repo::new();
    repo.write("src/main.rs", "fn main() {}\n");
    repo.write("docs/guide.md", "Read me.\n");
    repo.write("web/app.ts", "export const x = 1;\n");
    repo.commit("source");
    repo.wh(&[
        "change",
        "--kind",
        "mission",
        "--content",
        "Ship <safe> code.",
    ]);
    let rust = format!(
        r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["src/**"]}}"#
    );
    repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.no-unwrap",
        "--content",
        "No unwrap & no panics.",
        "--rationale",
        "A panic is not an error message.",
        "--definition",
        &rust,
    ]);
    let dead = format!(
        r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["lib/**"]}}"#
    );
    repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.lib-only",
        "--content",
        "No unwrap in lib.",
        "--rationale",
        "Libraries do not panic.",
        "--definition",
        &dead,
        "--allow-unvalidated",
    ]);
    let view = repo.wh_json(&["dash"]);
    let coverage = &view["data"]["current"]["coverage"];
    assert_eq!(coverage["covered"], 1, "{coverage}");
    let uncovered = coverage["uncovered"]
        .as_array()
        .expect("uncovered")
        .iter()
        .map(|group| {
            format!(
                "{} {}",
                group["directory"].as_str().unwrap_or_default(),
                group["language"].as_str().unwrap_or_default()
            )
        })
        .collect::<Vec<_>>();
    assert!(uncovered.contains(&"docs/ other".to_string()), "{coverage}");
    assert!(
        uncovered.contains(&"web/ typescript".to_string()),
        "{coverage}"
    );
    assert_eq!(
        coverage["dead_rules"][0]["rule"], "rule.lib-only",
        "{coverage}"
    );

    // The page shows the accepted rulebook only, escaped, with no script.
    let out = tempfile::tempdir().expect("out");
    let before = repo.wh_json(&["dash", "--export", out.path().to_str().expect("path")]);
    assert_eq!(before["data"]["counts"]["rules"], 0, "{before}");
    repo.wh(&["change", "--accept", "--rationale", "First rules."]);
    let exported = repo.wh_json(&["dash", "--export", out.path().to_str().expect("path")]);
    assert_eq!(exported["state"], "success", "{exported}");
    assert_eq!(exported["data"]["counts"]["rules"], 2);
    assert_eq!(exported["data"]["counts"]["changelog"], 1);
    let page = std::fs::read_to_string(out.path().join("index.html")).expect("page");
    assert!(page.contains("Ship &lt;safe&gt; code."));
    assert!(page.contains("No unwrap &amp; no panics."));
    assert!(page.contains("First rules."));
    assert!(!page.contains("<script"));
}
