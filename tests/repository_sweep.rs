//! A check of the whole repository reads every file in it, however many there
//! are: a broken file sorted after tens of thousands of clean ones is still
//! found. The repository is a throwaway; no `bd` runs.

use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};
use tempfile::TempDir;

#[path = "support/rulebook.rs"]
mod rulebook;

const FILES: usize = 25_000;

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_check_of_the_repository_reads_every_file() {
    let temp = TempDir::new().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    for index in 0..FILES {
        let directory = root.join(format!("src/m{:03}", index / 1000));
        std::fs::create_dir_all(&directory).expect("directory");
        let body = if index == FILES - 1 {
            "fn last() { dbg!(1); }\n"
        } else {
            "fn clean() {}\n"
        };
        std::fs::write(directory.join(format!("f{index:05}.rs")), body).expect("write");
    }
    rulebook::add_rule(
        root,
        "rule.no-dbg",
        "No dbg! macro.",
        &json!({
            "strength": "must",
            "enforcer": {
                "kind": "ast",
                "query": "((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))",
                "language": "rust",
            },
            "paths": ["src/**"],
        }),
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);

    let output = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(["check", "--json"])
        .current_dir(root)
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .output()
        .expect("whetstone");
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no JSON envelope ({error}): {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    });
    assert_eq!(envelope["state"], "violated", "{envelope:#}");
    let text = envelope.to_string();
    assert!(
        text.contains(&format!("in {FILES} file(s)")),
        "every file is checked: {envelope:#}"
    );
    assert!(
        text.contains(&format!("src/m024/f{:05}.rs", FILES - 1)),
        "the last file's flag is reported: {envelope:#}"
    );
}
