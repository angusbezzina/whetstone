//! Two agent hosts receive the same canonical skill; Claude Code gets hooks
//! (session context that refuses a stale skill, Stop-time repair feedback
//! bounded to three returns), Cursor uses explicit checkpoints; delivery is
//! only ever claimed from a checkpoint. Required checks fail closed without
//! team-active, verifiable policy.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run whetstone")
}

fn run_with_stdin(args: &[&str], cwd: &Path, input: &str) -> Output {
    let mut child = Command::new(bin())
        .args(args)
        .current_dir(cwd)
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

fn write_script(root: &Path, body: &str) {
    let path = root.join("check.sh");
    fs::write(&path, body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

fn establish(root: &Path) {
    git(root, &["init", "-q"]);
    write_script(root, "#!/bin/sh\necho ok\nexit 0\n");
    let inspected = json(&run(&["init", "--json", "--request-id", "i1"], root));
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
            "0",
            "--resume",
            &token,
            "--mission",
            "Two hosts, one truth.",
            "--desired-outcome",
            "Agents never act on stale guidance.",
            "--values",
            "Evidence.",
            "--philosophy",
            "Small.",
            "--owner",
            "Owner",
            "--initial-safeguard",
            "The check script passes.",
            "--safeguard-scope",
            "repository",
            "--revision-triggers",
            "a gate fails",
            "--gate-command",
            "./check.sh",
        ],
        root,
    ));
    assert_eq!(
        agreed["data"]["records"].as_array().map(Vec::len),
        Some(5),
        "{agreed}"
    );
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
    let claude = fs::read_dir(root.join(".claude/skills"))
        .expect("claude skills")
        .flatten()
        .find(|entry| entry.file_name().to_string_lossy().starts_with("verify-"))
        .expect("claude verify skill")
        .path();
    let cursor = root
        .join(".cursor/skills")
        .join(claude.file_name().expect("name"));
    assert_eq!(files(&claude), files(&cursor), "byte-identical projections");
    let settings: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(root.join(".claude/settings.json")).expect("settings"),
    )
    .expect("json");
    let stop = settings["hooks"]["Stop"].to_string();
    assert!(stop.contains("make lint"), "the team's hook is kept");
    assert!(stop.contains("wh check --hook stop --host claude"));
    assert!(settings["hooks"]["SessionStart"]
        .to_string()
        .contains("wh dash --hook session-start --host claude"));

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

    // ...and refuses the skill once the agreement moves on.
    let probe = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "v2",
            "--kind",
            "value",
            "--record-id",
            "value.speed",
            "--content",
            "Fast feedback",
            "--rationale",
            "speed",
        ],
        root,
    ));
    let revision = probe["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = probe["resume_token"].as_str().expect("token").to_string();
    let drafted = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "v2",
            "--kind",
            "value",
            "--record-id",
            "value.speed",
            "--content",
            "Fast feedback",
            "--rationale",
            "speed",
            "--expected-revision",
            &revision,
            "--resume",
            &token,
        ],
        root,
    ));
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    let accepted = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "a2",
            "--accept",
            &proposal,
        ],
        root,
    ));
    assert_eq!(accepted["state"], "success");
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

    // Stop hook: a failing gate returns to the same session, three times at
    // most, then hands back to the owner.
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
        stderr.contains("standard.initial-gate") && stderr.contains("never weaken a gate"),
        "{stderr}"
    );
    for _ in 0..2 {
        let again = run_with_stdin(
            &["check", "--hook", "stop", "--host", "claude"],
            root,
            r#"{"session_id":"s1","stop_hook_active":true}"#,
        );
        assert_eq!(again.status.code(), Some(2));
    }
    let bounded = run_with_stdin(
        &["check", "--hook", "stop", "--host", "claude"],
        root,
        r#"{"session_id":"s1","stop_hook_active":true}"#,
    );
    assert_eq!(bounded.status.code(), Some(0), "the loop is bounded");
    assert!(String::from_utf8_lossy(&bounded.stderr).contains("hand the brief"));
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
    git(root, &["add", "-A"]);
    git(
        root,
        &[
            "-c",
            "user.name=Owner",
            "-c",
            "user.email=owner@example.invalid",
            "commit",
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

    // Required checks: team-active policy only, and none exists here.
    let required = json(&run(&["check", "--json", "--required"], root));
    assert_eq!(required["state"], "unknown", "{required}");
    assert!(required["summary"]
        .as_str()
        .expect("summary")
        .contains("shared Beads database"));
}
