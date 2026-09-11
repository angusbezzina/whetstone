//! The kernel needs `bd`, not `dolt`: init, change, check and dash run with a
//! PATH that holds only `bd` and the basic tools, and no `dolt` anywhere.

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
        .env_remove("BEADS_DIR")
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
fn the_whole_local_flow_runs_without_a_dolt_binary() {
    let temp = tempfile::tempdir().expect("temp");
    let tools = temp.path().join("bin");
    fs::create_dir(&tools).expect("bin");
    for program in ["bd", "git", "date", "sh", "env"] {
        let found = which(program).unwrap_or_else(|| panic!("{program} is required"));
        std::os::unix::fs::symlink(found, tools.join(program)).expect("link");
    }
    assert!(!tools.join("dolt").exists());
    let root = temp.path().join("project");
    fs::create_dir(&root).expect("project");
    let git = Command::new(tools.join("git"))
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .expect("git");
    assert!(git.success());

    let inspected = json(&run(
        &["init", "--json", "--request-id", "i1"],
        &root,
        &tools,
    ));
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
            "No dolt needed.",
            "--desired-outcome",
            "Records in Beads.",
            "--values",
            "Evidence.",
            "--philosophy",
            "Small.",
            "--owner",
            "Owner",
            "--initial-safeguard",
            "Git answers.",
            "--safeguard-scope",
            "repository",
            "--revision-triggers",
            "a gate fails",
            "--gate-command",
            "git --version",
        ],
        &root,
        &tools,
    ));
    assert_eq!(
        agreed["data"]["records"].as_array().map(Vec::len),
        Some(5),
        "{agreed}"
    );

    let probe = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "c1",
            "--kind",
            "value",
            "--record-id",
            "value.speed",
            "--content",
            "Fast feedback",
            "--rationale",
            "speed",
        ],
        &root,
        &tools,
    ));
    let revision = probe["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let resume = probe["resume_token"].as_str().expect("token").to_string();
    let changed = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "c1",
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
            &resume,
        ],
        &root,
        &tools,
    ));
    assert_eq!(changed["state"], "success", "{changed}");

    let checked = json(&run(&["check", "--json"], &root, &tools));
    assert_eq!(checked["state"], "success", "{checked}");
    let dash = json(&run(&["dash", "--json"], &root, &tools));
    assert_eq!(dash["state"], "success", "{dash}");
    assert_eq!(dash["data"]["current"]["established"], true);
    assert_eq!(dash["data"]["current"]["header"]["drafts"], 1);
}
