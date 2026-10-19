//! `wh change` edits `whetstone/rulebook.json` and nothing else: a draft is
//! the edit in the working tree, accepting commits the file, withdrawing
//! restores it, retiring moves a rule to the file's retired list, and
//! `wh dash --trail` is the file's Git log. No `bd` is on the PATH.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{json, Value};

const RULEBOOK: &str = "whetstone/rulebook.json";

/// A throwaway repository and a PATH with only the tools Whetstone uses here.
struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
    path: PathBuf,
    home: PathBuf,
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

impl Repo {
    #[cfg(unix)]
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("project");
        let path = temp.path().join("bin");
        let home = temp.path().join("home");
        for dir in [&root, &path, &home] {
            fs::create_dir(dir).expect("dir");
        }
        for program in ["git", "date", "sh", "env"] {
            let found = which(program).unwrap_or_else(|| panic!("{program} is required"));
            std::os::unix::fs::symlink(found, path.join(program)).expect("link");
        }
        let repo = Self {
            _temp: temp,
            root,
            path,
            home,
        };
        repo.git(&["init", "--quiet"]);
        repo.git(&["config", "user.name", "Test Owner"]);
        repo.git(&["config", "user.email", "owner@example.invalid"]);
        repo.git(&["config", "commit.gpgsign", "false"]);
        repo
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.root)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .expect("git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).expect("utf-8")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_whetstone"))
            .args(args)
            .current_dir(&self.root)
            .env("PATH", &self.path)
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("BEADS_DIR")
            .output()
            .expect("run whetstone")
    }

    fn wh(&self, args: &[&str]) -> Value {
        let mut args = args.to_vec();
        args.insert(1, "--json");
        let output = self.run(&args);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn rulebook(&self) -> Value {
        serde_json::from_str(&self.text()).expect("rulebook JSON")
    }

    fn text(&self) -> String {
        fs::read_to_string(self.root.join(RULEBOOK)).expect("rulebook")
    }

    /// Every path Whetstone may have written outside `.git/objects`.
    fn written(&self) -> Vec<String> {
        let status = self.git(&["status", "--porcelain", "--untracked-files=all"]);
        let mut paths = status
            .lines()
            .map(|line| line[3..].to_string())
            .collect::<Vec<_>>();
        if self.root.join(".git/whetstone").exists() {
            paths.push(".git/whetstone".into());
        }
        paths
    }
}

fn rule_definition(strength: &str) -> String {
    json!({
        "type": "rule",
        "strength": strength,
        "enforcer": {
            "kind": "ast",
            "language": "rust",
            "query": "((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))"
        },
        "paths": ["src/**"]
    })
    .to_string()
}

fn draft_rule(repo: &Repo, strength: &str) -> Value {
    repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.no-dbg",
        "--content",
        "No dbg! is left in the source.",
        "--rationale",
        "Debug output is unfinished work.",
        "--definition",
        &rule_definition(strength),
    ])
}

#[cfg(unix)]
#[test]
fn change_accept_and_retire_round_trip_through_the_file() {
    let repo = Repo::new();
    let refused = draft_rule(&repo, "must");
    assert_eq!(refused["state"], "needs_input", "{refused}");
    assert!(
        refused["blocking_questions"][0]
            .as_str()
            .is_some_and(|question| question.contains("Set the mission first")),
        "{refused}"
    );

    let mission = repo.wh(&[
        "change",
        "--kind",
        "mission",
        "--content",
        "Ship code people can trust.",
    ]);
    assert_eq!(mission["state"], "success", "{mission}");
    assert_eq!(mission["data"]["revision"], "uncommitted");
    assert_eq!(
        repo.rulebook()["mission"]["statement"],
        "Ship code people can trust."
    );

    let preview = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.no-dbg",
        "--content",
        "No dbg! is left in the source.",
        "--rationale",
        "Debug output is unfinished work.",
        "--definition",
        &rule_definition("must"),
        "--dry-run",
    ]);
    assert_eq!(preview["state"], "needs_decision", "{preview}");
    assert_eq!(preview["data"]["preview_only"], true);
    assert_eq!(preview["data"]["diff"]["before"], Value::Null);
    assert_eq!(preview["data"]["diff"]["after"]["strength"], "must");
    assert!(
        repo.rulebook()["rules"]
            .as_array()
            .expect("rules")
            .is_empty(),
        "a preview writes nothing"
    );

    let drafted = draft_rule(&repo, "must");
    assert_eq!(drafted["state"], "success", "{drafted}");
    assert_eq!(drafted["data"]["written"], true);
    let first_digest = drafted["data"]["digest"]["after"]
        .as_str()
        .expect("digest")
        .to_string();
    assert!(first_digest.starts_with("sha256:"), "{drafted}");
    assert_eq!(repo.rulebook()["rules"][0]["id"], "rule.no-dbg");
    assert_eq!(
        repo.written(),
        [RULEBOOK],
        "only the rulebook file is written"
    );
    let replay = draft_rule(&repo, "must");
    assert_eq!(replay["data"]["written"], false, "{replay}");

    let accepted = repo.wh(&[
        "change",
        "--accept",
        "--rationale",
        "The owner agreed the mission and the first rule.",
    ]);
    assert_eq!(accepted["state"], "success", "{accepted}");
    let first_commit = accepted["data"]["commit"]
        .as_str()
        .expect("commit")
        .to_string();
    assert_eq!(repo.git(&["rev-parse", "HEAD"]).trim(), first_commit);
    assert_eq!(accepted["data"]["revision"], first_commit.as_str());
    assert_eq!(
        accepted["data"]["message"]["subject"],
        "Change the rulebook: set the mission, add rule.no-dbg"
    );
    assert_eq!(
        accepted["data"]["changes"],
        json!([
            "set the mission",
            format!("add rule.no-dbg ({first_digest})")
        ])
    );
    assert!(repo.written().is_empty(), "accepting commits the file");
    let nothing = repo.wh(&["change", "--accept"]);
    assert_eq!(nothing["state"], "needs_input", "{nothing}");

    let revised = draft_rule(&repo, "should");
    assert_eq!(revised["state"], "success", "{revised}");
    assert_eq!(revised["data"]["digest"]["before"], first_digest.as_str());
    let second_digest = revised["data"]["digest"]["after"]
        .as_str()
        .expect("digest")
        .to_string();
    assert_ne!(
        second_digest, first_digest,
        "the digest follows the content"
    );
    let accepted = repo.wh(&["change", "--accept"]);
    assert_eq!(
        accepted["data"]["message"]["subject"], "Change rule.no-dbg in the rulebook",
        "{accepted}"
    );
    assert_eq!(
        accepted["data"]["changes"],
        json!([format!(
            "change rule.no-dbg ({first_digest} -> {second_digest})"
        )])
    );

    let retired = repo.wh(&[
        "change",
        "--retire",
        "rule.no-dbg",
        "--rationale",
        "Clippy's dbg_macro lint covers it.",
    ]);
    assert_eq!(retired["state"], "success", "{retired}");
    assert_eq!(retired["data"]["digest"]["before"], second_digest.as_str());
    let book = repo.rulebook();
    assert!(book["rules"].as_array().expect("rules").is_empty());
    assert_eq!(
        book["retired"],
        json!([{"id": "rule.no-dbg", "reason": "Clippy's dbg_macro lint covers it."}])
    );
    let again = repo.wh(&["change", "--retire", "rule.no-dbg", "--rationale", "Twice."]);
    assert_eq!(again["state"], "needs_input", "{again}");
    let accepted = repo.wh(&["change", "--accept"]);
    assert_eq!(accepted["state"], "success", "{accepted}");
    let body = repo.git(&["log", "-1", "--format=%s%n%b"]);
    assert_eq!(
        body.trim(),
        "Retire rule.no-dbg from the rulebook\nRetire rule.no-dbg: Clippy's dbg_macro lint covers it."
    );

    let trail = repo.run(&["dash", "--trail"]);
    assert!(trail.status.success(), "{trail:?}");
    let trail = String::from_utf8(trail.stdout).expect("utf-8");
    let rows = trail.lines().collect::<Vec<_>>();
    assert_eq!(rows[0], "ts\tphase\tdecision\twhy\tevidence\tresult");
    assert_eq!(rows.len(), 4, "{trail}");
    let first = rows[1].split('\t').collect::<Vec<_>>();
    assert_eq!(first[1], "rulebook");
    assert_eq!(
        first[2],
        "Change the rulebook: set the mission, add rule.no-dbg"
    );
    assert_eq!(first[3], "The owner agreed the mission and the first rule.");
    assert_eq!(first[4], format!("git:{first_commit} {RULEBOOK}"));
    assert_eq!(first[5], "committed by Test Owner");
    assert!(rows[3].contains("\tRetire rule.no-dbg from the rulebook\t"));

    let dash = repo.wh(&["dash"]);
    let changelog = dash["data"]["changelog"].as_array().expect("changelog");
    assert_eq!(changelog.len(), 3, "{dash}");
    assert_eq!(
        changelog[0]["title"],
        "Retire rule.no-dbg from the rulebook"
    );
    assert_eq!(changelog[2]["commit"], first_commit.as_str());
    assert_eq!(changelog[2]["owner"], "Test Owner");
    assert!(repo.written().is_empty(), "{:?}", repo.written());
    assert!(
        !repo.root.join(".beads").exists(),
        "no record store appears"
    );
}

#[cfg(unix)]
#[test]
fn withdraw_restores_the_last_commit_and_a_retired_id_can_return() {
    let repo = Repo::new();
    repo.wh(&["change", "--kind", "mission", "--content", "Ship."]);
    let never = repo.wh(&["change", "--withdraw"]);
    assert_eq!(never["state"], "needs_input", "{never}");
    draft_rule(&repo, "must");
    repo.wh(&["change", "--accept"]);
    let committed = repo.text();

    repo.wh(&[
        "change",
        "--retire",
        "rule.no-dbg",
        "--rationale",
        "Trying it out.",
    ]);
    let preview = repo.wh(&["change", "--withdraw", "--dry-run"]);
    assert_eq!(preview["state"], "needs_decision", "{preview}");
    assert_eq!(
        preview["data"]["discards"],
        json!(["retire rule.no-dbg: Trying it out."])
    );
    let withdrawn = repo.wh(&["change", "--withdraw"]);
    assert_eq!(withdrawn["state"], "success", "{withdrawn}");
    assert_eq!(repo.text(), committed);
    assert!(repo.written().is_empty());

    repo.wh(&[
        "change",
        "--retire",
        "rule.no-dbg",
        "--rationale",
        "Retired for now.",
    ]);
    let back = draft_rule(&repo, "must");
    assert_eq!(back["state"], "success", "{back}");
    assert_eq!(back["data"]["reinstated"], true);
    assert_eq!(repo.rulebook().get("retired"), None);
    assert_eq!(repo.text(), committed, "the same rule makes the same file");
}

#[cfg(unix)]
#[test]
fn the_file_holds_only_valid_rules_and_an_agent_never_widens_a_sweep() {
    let repo = Repo::new();
    repo.wh(&["change", "--kind", "mission", "--content", "Ship."]);
    let missing = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.x",
        "--content",
        "Something.",
    ]);
    assert_eq!(missing["state"], "needs_input");
    assert_eq!(
        missing["blocking_questions"],
        json!(["Missing --rationale."])
    );
    let before = repo.text();
    let invalid = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.x",
        "--content",
        "Something.",
        "--rationale",
        "Because.",
        "--definition",
        &json!({
            "type": "rule",
            "strength": "should",
            "enforcer": {"kind": "test", "command": "cargo test"},
            "sweep_priority": 9
        })
        .to_string(),
    ]);
    assert_eq!(invalid["state"], "needs_input", "{invalid}");
    assert_eq!(repo.text(), before, "an invalid rule is never written");
    let widened = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.x",
        "--content",
        "Something.",
        "--rationale",
        "Because.",
        "--definition",
        &json!({
            "type": "rule",
            "strength": "should",
            "enforcer": {"kind": "test", "command": "cargo test"},
            "sweep": "draft_pr"
        })
        .to_string(),
    ]);
    assert_eq!(widened["state"], "needs_input", "{widened}");
    assert!(
        widened["blocking_questions"][0]
            .as_str()
            .is_some_and(|question| question.contains("a person sets the sweep class")),
        "{widened}"
    );
    let principle = repo.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.prove-it-works",
        "--content",
        "Prove it works.",
        "--definition",
        r#"{"type": "principle", "pstack": "prove-it-works"}"#,
    ]);
    assert_eq!(principle["state"], "success", "{principle}");
    assert_eq!(repo.rulebook()["principles"][0]["source"]["kind"], "pstack");
    let clash = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "principle.prove-it-works",
        "--content",
        "Something.",
        "--rationale",
        "Because.",
        "--definition",
        &rule_definition("should"),
    ]);
    assert_eq!(clash["state"], "needs_input", "{clash}");
    fs::write(
        repo.root.join(RULEBOOK),
        "{\"schema\": \"whetstone.rulebook.v1\"}",
    )
    .expect("break");
    let broken = repo.wh(&["change", "--kind", "mission", "--content", "Again."]);
    assert_eq!(broken["state"], "unknown", "{broken}");
    assert!(
        broken["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("missing field `mission`")),
        "{broken}"
    );
}
