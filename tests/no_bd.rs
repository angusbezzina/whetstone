//! Whetstone needs neither `bd` nor `dolt`: init, change, dash and check run
//! with a PATH that holds only `git` and the basic tools.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

fn run(args: &[&str], cwd: &Path, path: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .env("GIT_CEILING_DIRECTORIES", cwd.parent().expect("parent"))
        .output()
        .expect("run whetstone")
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

#[cfg(unix)]
#[test]
fn the_whole_local_flow_runs_with_only_git_on_the_path() {
    let temp = tempfile::tempdir().expect("temp");
    let tools = temp.path().join("bin");
    fs::create_dir(&tools).expect("bin");
    for program in ["git", "date", "sh", "env", "true"] {
        let found = which(program).unwrap_or_else(|| panic!("{program} is required"));
        std::os::unix::fs::symlink(found, tools.join(program)).expect("link");
    }
    let root = temp.path().join("project");
    fs::create_dir(&root).expect("project");
    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "Test Owner"],
        &["config", "user.email", "owner@example.invalid"],
        &["config", "commit.gpgsign", "false"],
    ] {
        let git = Command::new(tools.join("git"))
            .args(args)
            .current_dir(&root)
            .status()
            .expect("git");
        assert!(git.success());
    }
    // A test command, so the prove-it-works starter is offered, and an npm
    // that passes it, so the starter's probe runs.
    fs::write(root.join("package.json"), r#"{"scripts":{"test":"true"}}"#).expect("package.json");
    std::os::unix::fs::symlink(which("true").expect("true"), tools.join("npm")).expect("npm");

    let inspected = json(&run(&["init", "--json"], &root, &tools));
    assert_eq!(
        inspected["data"]["progress"]["missing_decisions"],
        serde_json::json!(["mission", "rules"]),
        "{inspected}"
    );
    let agreed = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "agree",
            "--mission",
            "No bd needed.",
            "--principle",
            "prove-it-works",
            "--starter",
            "prove-it-works",
        ],
        &root,
        &tools,
    ));
    assert_eq!(agreed["state"], "success", "{agreed}");
    assert_eq!(agreed["data"]["progress"]["agreement_complete"], true);

    let changed = json(&run(
        &[
            "change",
            "--json",
            "--kind",
            "rule",
            "--record-id",
            "rule.git-answers",
            "--content",
            "Git answers.",
            "--rationale",
            "Git answers.",
            "--definition",
            r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"git --version"}}"#,
        ],
        &root,
        &tools,
    ));
    assert_eq!(changed["state"], "success", "{changed}");
    let accepted = json(&run(&["change", "--json", "--accept"], &root, &tools));
    assert_eq!(accepted["state"], "success", "{accepted}");

    let dash = json(&run(&["dash", "--json"], &root, &tools));
    assert_eq!(dash["state"], "success", "{dash}");
    assert_eq!(dash["data"]["current"]["established"], true);
    let checked = json(&run(
        &["check", "--json", "--rule", "rule.git-answers"],
        &root,
        &tools,
    ));
    assert_eq!(checked["state"], "success", "{checked}");
    assert!(!root.join(".beads").exists());
}
