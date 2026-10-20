//! Principle coverage: the chain from a principle to the rules that serve it
//! is reported by `wh check` and `wh dash`, answered by `wh explain`, and
//! held by `wh change` when a served principle is retired or strict mode is
//! on at accept time. It never enters the sweep's findings.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

struct Repo {
    _temp: tempfile::TempDir,
    root: PathBuf,
}

fn git(root: &Path, args: &[&str]) {
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
}

impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("project");
        std::fs::create_dir_all(root.join("src")).expect("project");
        git(&root, &["init", "--quiet"]);
        git(&root, &["config", "user.name", "Test Owner"]);
        git(&root, &["config", "user.email", "owner@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("write");
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "--quiet", "-m", "base"]);
        let repo = Self { _temp: temp, root };
        repo.ok(&["change", "--kind", "mission", "--content", "Ship."]);
        repo
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_whetstone"))
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("whetstone")
    }

    fn wh(&self, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.push("--json");
        let output = self.run(&all);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "no envelope ({error}): {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    fn ok(&self, args: &[&str]) -> Value {
        let response = self.wh(args);
        assert_eq!(response["state"], "success", "{args:?}: {response}");
        response
    }

    fn principle(&self, id: &str, content: &str) {
        self.ok(&[
            "change",
            "--kind",
            "principle",
            "--record-id",
            id,
            "--content",
            content,
        ]);
    }

    /// A rule whose validator command passes (`true`) or flags (`false`),
    /// serving `serves` when given.
    fn rule(&self, id: &str, strength: &str, command: &str, serves: Option<&str>) {
        let definition = format!(
            r#"{{"type":"rule","strength":"{strength}","enforcer":{{"kind":"validator","command":"{command}"}}}}"#
        );
        let mut args = vec![
            "change",
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            "The rule holds.",
            "--rationale",
            "It matters.",
            "--definition",
            definition.as_str(),
            "--allow-unvalidated",
        ];
        if let Some(serves) = serves {
            args.extend(["--serves", serves]);
        }
        self.ok(&args);
    }
}

fn ids(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("an array: {value}"))
        .iter()
        .map(|id| id.as_str().expect("id"))
        .collect()
}

/// Two principles, one served and one not; one rule serving a principle and
/// one serving none.
fn rulebook_with_gaps() -> Repo {
    let repo = Repo::new();
    repo.principle("principle.ship-small", "Ship the smallest change.");
    repo.principle("principle.prove-it-works", "Prove it works.");
    repo.rule("rule.passes", "must", "true", Some("ship-small"));
    repo.rule("rule.loose", "should", "false", None);
    repo
}

#[test]
fn check_and_dash_report_the_gaps_without_changing_the_verdict_or_the_findings() {
    let repo = rulebook_with_gaps();

    let check = repo.wh(&["check"]);
    assert_eq!(check["state"], "success", "{check}");
    assert_eq!(
        ids(&check["data"]["coverage"]["unserved"]),
        ["principle.prove-it-works"]
    );
    assert_eq!(
        ids(&check["data"]["coverage"]["unanchored"]),
        ["rule.loose"]
    );
    assert!(
        check["summary"]
            .as_str()
            .expect("summary")
            .contains("1 principle has no rule; 1 rule serves no principle"),
        "{check}"
    );

    // The sweep's findings keep their shape: one finding for the should
    // rule's flag, and nothing about coverage.
    let sweep = repo.run(&["check", "--sweep"]);
    let text = String::from_utf8_lossy(&sweep.stdout);
    assert!(text.contains("rule.loose"), "{text}");
    assert!(
        !text.contains("unserved") && !text.contains("unanchored"),
        "{text}"
    );

    // Pre-commit stays fast and says nothing about the chain.
    let staged = repo.wh(&["check", "--staged"]);
    assert!(staged["data"].get("coverage").is_none(), "{staged}");

    let dash = repo.ok(&["dash"]);
    let current = &dash["data"]["current"];
    let unserved = current["principle_coverage"]["unserved"]
        .as_array()
        .expect("unserved");
    assert_eq!(unserved.len(), 1, "{current}");
    assert_eq!(unserved[0]["id"], "principle.prove-it-works");
    let served = current["principles"]
        .as_array()
        .expect("principles")
        .iter()
        .find(|principle| principle["id"] == "principle.ship-small")
        .expect("ship-small");
    assert_eq!(ids(&served["served_by"]), ["rule.passes"]);
    assert!(
        current["health"]["items"]
            .as_array()
            .expect("items")
            .iter()
            .any(|item| item["kind"] == "principles"),
        "{current}"
    );
}

#[test]
fn explain_answers_for_a_principle_and_still_for_a_rule() {
    let repo = rulebook_with_gaps();

    let served = repo.ok(&["explain", "principle.ship-small"]);
    assert_eq!(served["data"]["unserved"], false, "{served}");
    assert_eq!(served["data"]["rules"][0]["id"], "rule.passes");

    let unserved = repo.ok(&["explain", "prove-it-works"]);
    assert_eq!(unserved["data"]["unserved"], true, "{unserved}");
    assert!(
        unserved["data"]["text"]
            .as_str()
            .expect("text")
            .contains("--serves principle.prove-it-works"),
        "{unserved}"
    );

    let rule = repo.ok(&["explain", "rule.passes"]);
    assert_eq!(rule["data"]["rule"]["id"], "rule.passes", "{rule}");
    assert_eq!(rule["data"]["principle"]["id"], "principle.ship-small");
}

#[test]
fn a_served_principle_is_not_retired_and_strict_mode_holds_the_chain_at_accept() {
    let repo = rulebook_with_gaps();

    let refused = repo.wh(&[
        "change",
        "--retire",
        "principle.ship-small",
        "--rationale",
        "Folded into the mission.",
    ]);
    assert_eq!(refused["state"], "needs_input", "{refused}");
    assert_eq!(ids(&refused["data"]["served_by"]), ["rule.passes"]);

    // Once its rule serves another principle, it retires.
    repo.rule("rule.passes", "must", "true", Some("prove-it-works"));
    repo.ok(&[
        "change",
        "--retire",
        "principle.ship-small",
        "--rationale",
        "Folded into the mission.",
    ]);

    // Strict mode: accept refuses while a rule serves no principle.
    repo.ok(&["change", "--require-served", "true"]);
    let blocked = repo.wh(&["change", "--accept", "--rationale", "First rulebook."]);
    assert_eq!(blocked["state"], "needs_input", "{blocked}");
    assert_eq!(
        ids(&blocked["data"]["coverage"]["unanchored"]),
        ["rule.loose"]
    );
    assert!(
        blocked["blocking_questions"][0]
            .as_str()
            .expect("question")
            .contains("rule.loose serves no principle"),
        "{blocked}"
    );

    repo.rule("rule.loose", "should", "false", Some("prove-it-works"));
    let accepted = repo.ok(&["change", "--accept", "--rationale", "First rulebook."]);
    assert!(accepted["data"]["commit"].is_string(), "{accepted}");
    let text = std::fs::read_to_string(repo.root.join("whetstone/rulebook.json")).expect("file");
    assert!(text.contains("\"require_served\": true"), "{text}");
    let check = repo.ok(&["check"]);
    assert_eq!(check["data"]["coverage"]["complete"], true, "{check}");
}

#[test]
fn a_draft_retired_before_it_was_committed_is_named_in_the_commit() {
    let repo = Repo::new();
    repo.ok(&["change", "--accept", "--rationale", "The mission."]);
    repo.rule("rule.passes", "must", "true", None);
    repo.ok(&[
        "change",
        "--retire",
        "rule.passes",
        "--rationale",
        "Not needed.",
    ]);
    let accepted = repo.ok(&["change", "--accept"]);
    assert_eq!(
        accepted["data"]["message"]["subject"], "Retire rule.passes from the rulebook",
        "{accepted}"
    );
}
