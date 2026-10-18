//! A throwaway repository driven through the real binary, for the test files
//! that need one: never this repository's `.beads` or store.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

pub fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env_remove("BEADS_DIR")
        .env("WHETSTONE_JEV_OFFLINE", "1")
        .output()
        .expect("run whetstone")
}

pub fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

pub fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

pub struct Repo {
    _temp: tempfile::TempDir,
    pub root: PathBuf,
}

impl Repo {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().canonicalize().expect("root");
        git(&root, &["init", "--quiet"]);
        git(&root, &["config", "user.name", "Test Owner"]);
        git(&root, &["config", "user.email", "owner@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        Self { _temp: temp, root }
    }

    pub fn wh(&self, args: &[&str]) -> Output {
        let mut all = args.to_vec();
        all.push("--json");
        run(&all, &self.root)
    }

    pub fn wh_json(&self, args: &[&str]) -> serde_json::Value {
        json(&self.wh(args))
    }

    pub fn write(&self, path: &str, text: &str) {
        let full = self.root.join(path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("dir");
        }
        std::fs::write(full, text).expect("write");
    }

    pub fn commit(&self, message: &str) {
        git(&self.root, &["add", "-A"]);
        git(
            &self.root,
            &["commit", "--quiet", "--no-verify", "-m", message],
        );
    }

    /// Inspect, then record the draft with the returned revision and token.
    pub fn propose(&self, request_id: &str, args: &[&str]) -> serde_json::Value {
        let mut base = vec!["change", "--request-id", request_id];
        base.extend_from_slice(args);
        let probe = self.wh_json(&base);
        assert_eq!(probe["state"], "needs_input", "{probe}");
        let revision = probe["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let token = probe["resume_token"].as_str().expect("token").to_string();
        let mut args = base.clone();
        args.extend(["--expected-revision", &revision, "--resume", &token]);
        self.wh_json(&args)
    }

    pub fn accept(&self, proposal: &str) -> serde_json::Value {
        self.wh_json(&[
            "change",
            "--request-id",
            &format!("accept-{proposal}"),
            "--accept",
            proposal,
        ])
    }

    /// Record a rule draft and accept it, so it is in force.
    pub fn rule(&self, record_id: &str, definition: &str) {
        let content = format!("Statement of {record_id}.");
        let drafted = self.propose(
            &format!("draft-{record_id}"),
            &[
                "--kind",
                "rule",
                "--record-id",
                record_id,
                "--content",
                &content,
                "--rationale",
                "The owner wants this.",
                "--definition",
                definition,
            ],
        );
        assert_eq!(drafted["state"], "success", "{drafted}");
        let proposal = drafted["data"]["proposal"]["id"]
            .as_str()
            .expect("proposal")
            .to_string();
        let accepted = self.accept(&proposal);
        assert_eq!(accepted["state"], "success", "{accepted}");
    }

    /// Agree a mission and the public-API starter, so the agreement exists.
    pub fn agree(&self) -> serde_json::Value {
        let inspect = self.wh_json(&["init", "--request-id", "agree"]);
        let revision = inspect["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let token = inspect["resume_token"].as_str().expect("token").to_string();
        self.wh_json(&[
            "init",
            "--action",
            "agree",
            "--request-id",
            "agree",
            "--expected-revision",
            &revision,
            "--resume",
            &token,
            "--mission",
            "Keep project work aligned.",
            "--principle",
            "prove-it-works",
            "--starter",
            "ask-before-public-api",
        ])
    }
}
