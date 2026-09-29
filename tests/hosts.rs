//! Agent hosts receive the same canonical skill; Claude Code, Codex and
//! Cursor each get hooks that speak their own contract (session context that
//! refuses a stale skill, Stop-time repair feedback bounded to two returns
//! before a hand is raised). Delivery is only ever claimed from a
//! checkpoint. The Git gates are pre-commit and pre-push, chained to any
//! hook that was there; the required CI check enforces shared rules only.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn command(args: &[&str], cwd: &Path) -> Command {
    let mut command = Command::new(bin());
    command
        .args(args)
        .current_dir(cwd)
        .env_remove("BEADS_DIR")
        .env("WHETSTONE_JEV_OFFLINE", "1");
    command
}

fn run(args: &[&str], cwd: &Path) -> Output {
    command(args, cwd).output().expect("run whetstone")
}

fn run_with_stdin(args: &[&str], cwd: &Path, input: &str) -> Output {
    let mut child = command(args, cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write");
    child.wait_with_output().expect("wait")
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

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
}

fn bd(root: &Path, args: &[&str]) -> String {
    let output = Command::new("bd")
        .args(args)
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect("bd");
    assert!(
        output.status.success(),
        "bd {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn write_script(root: &Path, body: &str) {
    let path = root.join("check.sh");
    fs::write(&path, body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

/// Two-step change: inspect for the base revision and token, then record.
fn change(root: &Path, request: &str, args: &[&str]) -> serde_json::Value {
    let mut probe = vec!["change", "--json", "--request-id", request];
    probe.extend_from_slice(args);
    let inspected = json(&run(&probe, root));
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let mut record = probe.clone();
    record.extend_from_slice(&["--expected-revision", &revision, "--resume", &token]);
    json(&run(&record, root))
}

fn accept(root: &Path, drafted: &serde_json::Value) {
    assert_eq!(drafted["state"], "success", "{drafted}");
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal");
    let accepted = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            &format!("accept-{proposal}"),
            "--accept",
            proposal,
        ],
        root,
    ));
    assert_eq!(accepted["state"], "success", "{accepted}");
}

fn rule(root: &Path, id: &str, statement: &str, definition: &serde_json::Value) {
    let drafted = change(
        root,
        &format!("{id}-draft"),
        &[
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            statement,
            "--rationale",
            "It is the proof.",
            "--definition",
            &definition.to_string(),
        ],
    );
    accept(root, &drafted);
}

/// A mission and one must rule proven by `./check.sh`.
fn establish(root: &Path) {
    git(root, &["init", "-q"]);
    write_script(root, "#!/bin/sh\necho ok\nexit 0\n");
    let inspected = json(&run(&["init", "--json", "--request-id", "i1"], root));
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "agree",
            "--request-id",
            "i1",
            "--expected-revision",
            &revision,
            "--resume",
            &token,
            "--mission",
            "Two hosts, one truth.",
        ],
        root,
    ));
    assert_eq!(
        agreed["data"]["records"][0]["id"], "mission.project",
        "{agreed}"
    );
    rule(
        root,
        "rule.check-script",
        "The check script passes.",
        &serde_json::json!({"type": "rule", "strength": "must", "enforcer": {"kind": "test", "command": "./check.sh"}}),
    );
    let check = json(&run(&["check", "--json"], root));
    assert_eq!(check["state"], "success", "{check}");
}

fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    for entry in walkdir(root) {
        let relative = entry
            .strip_prefix(root)
            .expect("relative")
            .to_string_lossy()
            .to_string();
        found.push((relative, fs::read(&entry).expect("read")));
    }
    found.sort();
    found
}

fn walkdir(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("dir").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

fn verify_skill(host_skills: &Path) -> PathBuf {
    fs::read_dir(host_skills)
        .expect("skills")
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with("verify-"))
        .expect("verify skill")
        .path()
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).expect("read")).expect("json")
}

/// A directory holding `wh`, the binary under test, so the Git hooks run it
/// rather than whatever `wh` happens to be installed.
fn wh_on_path(dir: &Path) -> std::ffi::OsString {
    fs::create_dir_all(dir).expect("bin dir");
    #[cfg(unix)]
    std::os::unix::fs::symlink(bin(), dir.join("wh")).expect("symlink wh");
    let mut paths = vec![dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    std::env::join_paths(paths).expect("PATH")
}

fn commit(root: &Path, path: &std::ffi::OsStr, message: &str) -> Output {
    Command::new("git")
        .args([
            "-c",
            "user.name=Owner",
            "-c",
            "user.email=owner@example.invalid",
            "commit",
            "-qm",
            message,
        ])
        .current_dir(root)
        .env("PATH", path)
        .env_remove("BEADS_DIR")
        .env("WHETSTONE_JEV_OFFLINE", "1")
        .output()
        .expect("git commit")
}

#[test]
fn two_hosts_get_one_truth_and_feedback_reaches_the_working_agent() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    fs::create_dir_all(root.join(".claude")).expect("claude");
    fs::write(
        root.join(".claude/settings.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"make lint"}]}]}}"#,
    )
    .expect("settings");

    let wired = json(&run(
        &[
            "init", "--json", "--action", "wire", "--host", "claude", "--host", "cursor", "--hooks",
        ],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");
    let claude = verify_skill(&root.join(".claude/skills"));
    let cursor = root
        .join(".cursor/skills")
        .join(claude.file_name().expect("name"));
    assert_eq!(files(&claude), files(&cursor), "byte-identical projections");
    // A bare wire regenerates the hosts wired before; another tool's
    // directory is not a request to project the skill there.
    fs::create_dir_all(root.join(".agents/skills/some-other-skill")).expect("other tool");
    let rewired = json(&run(&["init", "--json", "--action", "wire"], root));
    assert_eq!(rewired["state"], "success", "{rewired}");
    assert!(
        !fs::read_dir(root.join(".agents/skills"))
            .expect("agents skills")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().starts_with("verify-")),
        "no verify skill was projected into .agents"
    );
    let settings = read_json(&root.join(".claude/settings.json"));
    let stop = settings["hooks"]["Stop"].to_string();
    assert!(stop.contains("make lint"), "the team's hook is kept");
    assert!(stop.contains("wh check --hook stop --host claude"));
    assert!(settings["hooks"]["SessionStart"]
        .to_string()
        .contains("wh dash --hook session-start --host claude"));
    let cursor_hooks = read_json(&root.join(".cursor/hooks.json"));
    assert!(cursor_hooks["hooks"]["stop"]
        .to_string()
        .contains("wh check --hook stop --host cursor"));

    let dash = json(&run(&["dash", "--json"], root));
    let projections = dash["data"]["current"]["skill"]["projections"].to_string();
    assert!(
        projections.contains("\"host\":\"claude\"") && projections.contains("\"host\":\"cursor\"")
    );
    assert!(!projections.contains("stale"), "{projections}");
    let acknowledged = dash["data"]["current"]["skill"]["acknowledged"].to_string();
    assert!(
        acknowledged.contains("not acknowledged"),
        "no checkpoint, no claim: {acknowledged}"
    );

    // Cursor checks in explicitly; now, and only now, delivery is recorded.
    let checkpoint = json(&run(
        &["check", "--json", "--changed", "--host", "cursor"],
        root,
    ));
    assert_eq!(
        checkpoint["data"]["host_checkpoint"]["acknowledged"], true,
        "{checkpoint}"
    );
    assert_eq!(checkpoint["data"]["host_checkpoint"]["recorded"], true);
    let dash = json(&run(&["dash", "--json"], root));
    let rows = dash["data"]["current"]["skill"]["acknowledged"]
        .as_array()
        .expect("acknowledged")
        .clone();
    let cursor_row = rows
        .iter()
        .find(|row| row["host"] == "cursor")
        .expect("cursor");
    assert_eq!(cursor_row["state"], "acknowledged");
    let claude_row = rows
        .iter()
        .find(|row| row["host"] == "claude")
        .expect("claude");
    assert_eq!(claude_row["state"], "not acknowledged");

    // Session context is canonical while current...
    let context = run(
        &["dash", "--hook", "session-start", "--host", "claude"],
        root,
    );
    let text = String::from_utf8_lossy(&context.stdout);
    assert!(text.contains("Two hosts, one truth."), "{text}");
    assert!(text.contains("is current"), "{text}");
    assert!(
        text.contains("The check script passes. [must · Test]"),
        "the rules in force are named: {text}"
    );
    // ...Cursor's own hook speaks JSON, and the Claude hook stays quiet when
    // Cursor runs it.
    let cursor_context = json(&run(
        &["dash", "--hook", "session-start", "--host", "cursor"],
        root,
    ));
    assert!(cursor_context["additional_context"]
        .as_str()
        .expect("context")
        .contains("Two hosts, one truth."));
    let relayed = run_with_stdin(
        &["dash", "--hook", "session-start", "--host", "claude"],
        root,
        r#"{"conversation_id":"c1","cursor_version":"1.7.0"}"#,
    );
    assert!(relayed.status.success());
    assert!(relayed.stdout.is_empty(), "{relayed:?}");

    // ...and refuses the skill once the agreement moves on.
    let principle = change(
        root,
        "p2",
        &[
            "--kind",
            "principle",
            "--record-id",
            "principle.speed",
            "--content",
            "Fast feedback",
            "--rationale",
            "speed",
        ],
    );
    accept(root, &principle);
    let stale = run(
        &["dash", "--hook", "session-start", "--host", "claude"],
        root,
    );
    let text = String::from_utf8_lossy(&stale.stdout);
    assert!(
        text.contains("is stale") && text.contains("Do not rely on it"),
        "{text}"
    );
    let checkpoint = json(&run(
        &["check", "--json", "--changed", "--host", "cursor"],
        root,
    ));
    assert_eq!(checkpoint["data"]["host_checkpoint"]["acknowledged"], false);
    assert_eq!(checkpoint["data"]["host_checkpoint"]["skill"], "stale");

    // Stop hook: a failing must rule returns to the same session twice; the
    // third time the same rule fails the agent may stop, and a hand is due.
    write_script(root, "#!/bin/sh\necho \"check.sh:1: nope\"\nexit 1\n");
    let first = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s1","stop_hook_active":false,"hook_event_name":"Stop"}"#,
    );
    assert_eq!(
        first.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let stderr = String::from_utf8_lossy(&first.stderr);
    assert!(
        stderr.contains("rule.check-script")
            && stderr.contains("check.sh:1")
            && stderr.contains("never weaken a rule"),
        "{stderr}"
    );
    let again = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s1","stop_hook_active":true}"#,
    );
    assert_eq!(again.status.code(), Some(2));
    let bounded = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s1","stop_hook_active":true}"#,
    );
    assert_eq!(bounded.status.code(), Some(0), "the loop is bounded");
    assert!(
        String::from_utf8_lossy(&bounded.stderr).contains("two repairs did not clear"),
        "{}",
        String::from_utf8_lossy(&bounded.stderr)
    );
    // Cursor runs Claude hooks too; the Claude hook leaves Cursor's session
    // to Cursor's own hook.
    let skipped = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"conversation_id":"c9","cursor_version":"1.7.0"}"#,
    );
    assert_eq!(skipped.status.code(), Some(0));
    assert!(skipped.stdout.is_empty() && skipped.stderr.is_empty());
    write_script(root, "#!/bin/sh\necho ok\nexit 0\n");
    let repaired = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s2","stop_hook_active":false}"#,
    );
    assert_eq!(
        repaired.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&repaired.stderr)
    );

    // A session remembers the commit it began at, so the Stop hook also
    // proves work committed during the session (wh check --changed --base).
    git(root, &["add", "check.sh"]);
    git(
        root,
        &[
            "-c",
            "user.name=Owner",
            "-c",
            "user.email=owner@example.invalid",
            "commit",
            "--no-verify",
            "-qm",
            "baseline",
        ],
    );
    let started = run_with_stdin(
        &["dash", "--hook", "session-start", "--host", "claude"],
        root,
        r#"{"session_id":"s3","hook_event_name":"SessionStart"}"#,
    );
    assert!(started.status.success());
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("head");
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    let remembered = walkdir(&root.join(".git/whetstone"))
        .into_iter()
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("session-base-"))
        })
        .expect("session base recorded");
    assert_eq!(fs::read_to_string(remembered).expect("base").trim(), head);

    // The required CI check enforces the team's shared rules only, and none
    // were shared here.
    let required = json(&run(&["check", "--json", "--ci"], root));
    assert_eq!(required["state"], "unknown", "{required}");
    assert!(required["summary"]
        .as_str()
        .expect("summary")
        .contains("shared rules"));
}

/// Codex and Cursor get the same feedback in their own formats, and the
/// same failures after two repairs raise a hand in the repository's Beads.
#[test]
fn each_host_hears_its_own_format_and_a_third_failure_raises_a_hand() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    bd(
        root,
        &[
            "init",
            "--non-interactive",
            "--skip-agents",
            "--skip-hooks",
            "-p",
            "hosts",
            "-q",
        ],
    );
    write_script(root, "#!/bin/sh\necho \"check.sh:1: nope\"\nexit 1\n");

    let codex = run_with_stdin(
        &["check", "--hook", "stop", "--host", "codex"],
        root,
        r#"{"session_id":"x1","hook_event_name":"Stop"}"#,
    );
    assert_eq!(codex.status.code(), Some(0), "Codex reads the decision");
    let decision = json(&codex);
    assert_eq!(decision["decision"], "block", "{decision}");
    assert!(decision["reason"]
        .as_str()
        .expect("reason")
        .contains("rule.check-script"));

    let cursor = run_with_stdin(
        &["check", "--hook", "stop", "--host", "cursor"],
        root,
        r#"{"conversation_id":"c1","hook_event_name":"stop"}"#,
    );
    assert_eq!(cursor.status.code(), Some(0));
    let followup = json(&cursor);
    assert!(
        followup["followup_message"]
            .as_str()
            .expect("followup")
            .contains("never weaken a rule"),
        "{followup}"
    );

    for attempt in 0..2 {
        let blocked = run_with_stdin(
            &["check", "--hook", "stop", "--host", "claude"],
            root,
            r#"{"session_id":"s1"}"#,
        );
        assert_eq!(blocked.status.code(), Some(2), "return {attempt}");
    }
    let raised = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s1"}"#,
    );
    assert_eq!(raised.status.code(), Some(0), "the agent may stop");
    let stderr = String::from_utf8_lossy(&raised.stderr).to_string();
    assert!(stderr.contains("A hand was raised"), "{stderr}");
    assert!(stderr.contains("labelled human"), "{stderr}");
    let issues: serde_json::Value = serde_json::from_str(&bd(
        root,
        &["list", "--json", "--label", "human", "--limit", "0"],
    ))
    .expect("issues");
    let issues = issues.as_array().expect("issues");
    assert_eq!(issues.len(), 1, "{issues:?}");
    let issue = issues[0]["id"].as_str().expect("issue id");
    assert!(stderr.contains(issue), "{stderr}");
    assert!(issues[0]["title"]
        .as_str()
        .expect("title")
        .contains("rule.check-script"));
    let dash = json(&run(&["dash", "--json"], root));
    let requests = dash["data"]["current"]["requests"]
        .as_array()
        .expect("requests");
    let request = requests
        .iter()
        .find(|request| request["issue"] == issue)
        .unwrap_or_else(|| panic!("the hand is on the dashboard: {requests:?}"));
    assert_eq!(request["trigger"], "second_failed_repair");
    assert_eq!(request["rule"], "rule.check-script");

    // A fourth stop does not raise a second hand.
    let fourth = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s1"}"#,
    );
    assert_eq!(fourth.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&fourth.stderr).contains("already raised"));
    let issues: serde_json::Value = serde_json::from_str(&bd(
        root,
        &["list", "--json", "--label", "human", "--limit", "0"],
    ))
    .expect("issues");
    assert_eq!(issues.as_array().map(Vec::len), Some(1));
}

#[test]
fn git_gates_chain_the_existing_hook_block_a_broken_must_rule_and_are_idempotent() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path().join("repo");
    fs::create_dir_all(&root).expect("repo");
    let root = root.as_path();
    establish(root);
    rule(
        root,
        "rule.no-print",
        "No print calls.",
        &serde_json::json!({
            "type": "rule",
            "strength": "must",
            "enforcer": {
                "kind": "ast",
                "language": "python",
                "query": "((call function: (identifier) @f) @match (#eq? @f \"print\"))"
            }
        }),
    );
    let hooks = root.join(".git/hooks");
    fs::create_dir_all(&hooks).expect("hooks");
    let earlier = "#!/bin/sh\necho \"the team's own pre-commit ran\"\nexit 0\n";
    fs::write(hooks.join("pre-commit"), earlier).expect("earlier hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(hooks.join("pre-commit"), fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }

    let wired = json(&run(
        &["init", "--json", "--action", "wire", "--hooks"],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");
    let planned = wired["data"]["git_hooks"].as_array().expect("git hooks");
    assert_eq!(planned.len(), 2, "{wired}");
    assert!(planned
        .iter()
        .any(|hook| hook["path"] == ".git/hooks/pre-commit"
            && hook["chains"] == "pre-commit.pre-whetstone"));
    assert!(planned
        .iter()
        .any(|hook| hook["path"] == ".git/hooks/pre-push" && hook["chains"].is_null()));
    for name in ["pre-commit", "pre-push"] {
        let text = fs::read_to_string(hooks.join(name)).expect("hook");
        assert_eq!(
            text.lines().nth(1),
            Some("# whetstone-hook v1"),
            "{name}: {text}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(hooks.join(name))
                .expect("meta")
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "{name} is executable");
        }
    }
    assert_eq!(
        fs::read_to_string(hooks.join("pre-commit.pre-whetstone")).expect("chained"),
        earlier,
        "the earlier hook is kept byte for byte"
    );
    let pre_commit = fs::read_to_string(hooks.join("pre-commit")).expect("hook");
    let pre_push = fs::read_to_string(hooks.join("pre-push")).expect("hook");

    // Idempotent: a second wire changes nothing and never chains itself.
    let again = json(&run(
        &["init", "--json", "--action", "wire", "--hooks"],
        root,
    ));
    assert_eq!(again["state"], "success", "{again}");
    assert_eq!(again["data"]["git_hooks"], serde_json::json!([]), "{again}");
    assert_eq!(
        fs::read_to_string(hooks.join("pre-commit")).expect("hook"),
        pre_commit
    );
    assert_eq!(
        fs::read_to_string(hooks.join("pre-push")).expect("hook"),
        pre_push
    );
    assert_eq!(
        fs::read_to_string(hooks.join("pre-commit.pre-whetstone")).expect("chained"),
        earlier
    );
    assert!(!hooks.join("pre-push.pre-whetstone").exists());
    let inspected = json(&run(&["init", "--json"], root));
    let steps = inspected["data"]["onboarding"]["steps"]
        .as_array()
        .unwrap_or_else(|| panic!("onboarding steps: {inspected}"));
    assert!(
        steps
            .iter()
            .any(|step| step["key"] == "gates" && step["done"] == true),
        "{steps:?}"
    );

    // The pre-commit gate blocks a commit that breaks the must rule, runs
    // the chained hook first, and passes once the change is repaired.
    let path = wh_on_path(&temp.path().join("bin"));
    git(root, &["add", "check.sh"]);
    fs::write(root.join("app.py"), "print(\"hi\")\n").expect("bad");
    git(root, &["add", "app.py"]);
    let blocked = commit(root, &path, "bad");
    let stderr = String::from_utf8_lossy(&blocked.stderr).to_string();
    let stdout = String::from_utf8_lossy(&blocked.stdout).to_string();
    assert!(!blocked.status.success(), "{stdout}\n{stderr}");
    assert!(
        format!("{stdout}{stderr}").contains("the team's own pre-commit ran"),
        "the chained hook runs: {stdout}{stderr}"
    );
    assert!(
        stderr.contains("rule.no-print") && stderr.contains("app.py:1"),
        "{stderr}"
    );
    assert!(stderr.contains("whetstone pre-commit: blocked"), "{stderr}");
    let head = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", "HEAD"])
        .current_dir(root)
        .output()
        .expect("head");
    assert!(!head.status.success(), "nothing was committed");

    fs::write(
        root.join("app.py"),
        "import logging\nlogging.info(\"hi\")\n",
    )
    .expect("good");
    git(root, &["add", "app.py"]);
    let passed = commit(root, &path, "good");
    assert!(
        passed.status.success(),
        "{}",
        String::from_utf8_lossy(&passed.stderr)
    );
    assert!(String::from_utf8_lossy(&passed.stderr).contains("whetstone pre-commit:"));
}

/// Without a Beads tracker the third failure says plainly that no hand was
/// raised, and leaves the hand unset: once `bd init` runs, the next failing
/// stop files it.
#[test]
fn a_hand_that_could_not_be_filed_is_never_claimed_and_is_filed_once_beads_exists() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    write_script(root, "#!/bin/sh\necho \"check.sh:1: nope\"\nexit 1\n");
    let stop = || {
        run_with_stdin(
            &["check", "--hook", "stop", "--host", "claude"],
            root,
            r#"{"session_id":"s1"}"#,
        )
    };
    for attempt in 0..2 {
        assert_eq!(stop().status.code(), Some(2), "return {attempt}");
    }
    let unfiled = stop();
    assert_eq!(unfiled.status.code(), Some(0), "the agent may stop");
    let stderr = String::from_utf8_lossy(&unfiled.stderr).to_string();
    assert!(stderr.contains("no hand could be raised"), "{stderr}");
    assert!(!stderr.contains("A hand was raised"), "{stderr}");
    let again = stop();
    let stderr = String::from_utf8_lossy(&again.stderr).to_string();
    assert!(
        !stderr.contains("already raised"),
        "an unfiled hand is never claimed: {stderr}"
    );
    assert!(stderr.contains("no hand could be raised"), "{stderr}");

    bd(
        root,
        &[
            "init",
            "--non-interactive",
            "--skip-agents",
            "--skip-hooks",
            "-p",
            "unfiled",
            "-q",
        ],
    );
    let filed = stop();
    assert_eq!(filed.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&filed.stderr).to_string();
    assert!(stderr.contains("A hand was raised"), "{stderr}");
    let issues: serde_json::Value = serde_json::from_str(&bd(
        root,
        &["list", "--json", "--label", "human", "--limit", "0"],
    ))
    .expect("issues");
    let issues = issues.as_array().expect("issues");
    assert_eq!(issues.len(), 1, "{issues:?}");
    assert!(
        stderr.contains(issues[0]["id"].as_str().expect("id")),
        "{stderr}"
    );
    let after = stop();
    assert!(String::from_utf8_lossy(&after.stderr).contains("already raised"));
}

#[test]
fn wiring_codex_and_cursor_merges_their_hooks_and_ci_scaffolds_the_required_check() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    fs::create_dir_all(root.join(".codex")).expect("codex");
    fs::write(
        root.join(".codex/hooks.json"),
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./mine.sh"}]}]}}"#,
    )
    .expect("codex hooks");
    fs::create_dir_all(root.join(".cursor")).expect("cursor");
    fs::write(
        root.join(".cursor/hooks.json"),
        r#"{"version":1,"hooks":{"stop":[{"command":"./lint.sh"}],"afterFileEdit":[{"command":"./fmt.sh"}]}}"#,
    )
    .expect("cursor hooks");

    let dry = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "wire",
            "--host",
            "codex",
            "--host",
            "cursor",
            "--hooks",
            "--ci",
            "--dry-run",
        ],
        root,
    ));
    assert_eq!(dry["state"], "needs_decision", "{dry}");
    assert!(!root.join(".github").exists(), "a dry run writes nothing");
    assert!(!root.join(".agents").exists());

    let wired = json(&run(
        &[
            "init", "--json", "--action", "wire", "--host", "codex", "--host", "cursor", "--hooks",
            "--ci",
        ],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");

    // Codex reads the shared .agents/skills; Cursor its own directory.
    let codex_skill = verify_skill(&root.join(".agents/skills"));
    let cursor_skill = verify_skill(&root.join(".cursor/skills"));
    assert_eq!(files(&codex_skill), files(&cursor_skill));
    let rule_md = fs::read_to_string(codex_skill.join("rules/check-script.md")).expect("rule file");
    assert!(
        rule_md.starts_with("---\nrecord: \"rule.check-script\"\n"),
        "{rule_md}"
    );
    assert!(rule_md.contains("strength: \"must\""), "{rule_md}");
    assert!(rule_md.contains("# The check script passes."), "{rule_md}");
    let verify = read_json(&codex_skill.join("whetstone.verify.json"));
    assert_eq!(verify["schema"], "whetstone.verify.v1");
    assert_eq!(verify["mission"], "Two hosts, one truth.");
    assert_eq!(verify["rules"][0]["id"], "rule.check-script");
    assert_eq!(
        verify["rules"][0]["rule"]["enforcer"]["command"],
        "./check.sh"
    );
    let skill_md = fs::read_to_string(codex_skill.join("SKILL.md")).expect("skill");
    assert!(
        skill_md.contains("rules/"),
        "SKILL.md points at the rule files"
    );

    let codex = read_json(&root.join(".codex/hooks.json"));
    let stop = codex["hooks"]["Stop"].as_array().expect("stop");
    assert_eq!(stop.len(), 2, "{codex}");
    assert_eq!(
        stop[0]["hooks"][0]["command"], "./mine.sh",
        "the user's hook stays first"
    );
    assert_eq!(
        stop[1]["hooks"][0]["command"],
        "wh check --hook stop --host codex"
    );
    assert_eq!(
        codex["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "wh dash --hook session-start --host codex"
    );
    let cursor = read_json(&root.join(".cursor/hooks.json"));
    assert_eq!(cursor["version"], 1);
    let stop = cursor["hooks"]["stop"].as_array().expect("stop");
    assert_eq!(stop.len(), 2, "{cursor}");
    assert_eq!(stop[0]["command"], "./lint.sh");
    assert_eq!(stop[1]["command"], "wh check --hook stop --host cursor");
    assert_eq!(stop[1]["loop_limit"], 3);
    assert_eq!(cursor["hooks"]["afterFileEdit"][0]["command"], "./fmt.sh");
    assert_eq!(
        cursor["hooks"]["sessionStart"][0]["command"],
        "wh dash --hook session-start --host cursor"
    );

    let workflow = fs::read_to_string(root.join(".github/workflows/whetstone.yml")).expect("ci");
    assert!(workflow.contains("name: whetstone/policy"), "{workflow}");
    assert!(workflow.contains("wh check --ci --base"), "{workflow}");
    assert!(workflow.contains("bd bootstrap"), "{workflow}");

    // Rewiring merges nothing twice and leaves the team-owned workflow alone.
    fs::write(
        root.join(".github/workflows/whetstone.yml"),
        format!("{workflow}# the team's edit\n"),
    )
    .expect("edit workflow");
    let codex_before = fs::read_to_string(root.join(".codex/hooks.json")).expect("codex");
    let cursor_before = fs::read_to_string(root.join(".cursor/hooks.json")).expect("cursor");
    let again = json(&run(
        &[
            "init", "--json", "--action", "wire", "--host", "codex", "--host", "cursor", "--hooks",
            "--ci",
        ],
        root,
    ));
    assert_eq!(again["state"], "success", "{again}");
    assert_eq!(
        fs::read_to_string(root.join(".codex/hooks.json")).expect("codex"),
        codex_before
    );
    assert_eq!(
        fs::read_to_string(root.join(".cursor/hooks.json")).expect("cursor"),
        cursor_before
    );
    assert!(
        fs::read_to_string(root.join(".github/workflows/whetstone.yml"))
            .expect("ci")
            .ends_with("# the team's edit\n")
    );
}
