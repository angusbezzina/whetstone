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
        .env("WHETSTONE_JEV_OFFLINE", "1")
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
            "--principle",
            "prove-it-works",
            "--starter",
            "ask-before-public-api",
        ],
        &root,
        &tools,
    ));
    assert_eq!(
        agreed["data"]["records"].as_array().map(Vec::len),
        Some(3),
        "{agreed}"
    );
    assert_eq!(agreed["data"]["progress"]["agreement_complete"], true);

    let change = |request_id: &str, id: &str, content: &str, definition: &str| {
        let base = [
            "change",
            "--json",
            "--request-id",
            request_id,
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            content,
            "--rationale",
            "Git answers.",
            "--definition",
            definition,
        ];
        let probe = json(&run(&base, &root, &tools));
        let revision = probe["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let resume = probe["resume_token"].as_str().expect("token").to_string();
        let mut args = base.to_vec();
        args.extend(["--expected-revision", &revision, "--resume", &resume]);
        json(&run(&args, &root, &tools))
    };
    let definition =
        r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"git --version"}}"#;
    let changed = change("c1", "rule.git-answers", "Git answers.", definition);
    assert_eq!(changed["state"], "success", "{changed}");
    let proposal = changed["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    let accepted = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "a1",
            "--accept",
            &proposal,
        ],
        &root,
        &tools,
    ));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let pending = change(
        "c2",
        "rule.git-still-answers",
        "Git still answers.",
        definition,
    );
    assert_eq!(pending["state"], "success", "{pending}");

    let checked = json(&run(
        &["check", "--json", "--rule", "rule.git-answers"],
        &root,
        &tools,
    ));
    assert_eq!(checked["state"], "success", "{checked}");
    let dash = json(&run(&["dash", "--json"], &root, &tools));
    assert_eq!(dash["state"], "success", "{dash}");
    assert_eq!(dash["data"]["current"]["established"], true);
    assert_eq!(dash["data"]["current"]["header"]["drafts"], 1);
}
