//! A test or validator command that exits 69 reached no verdict: a tool,
//! permission or service it needs is unavailable. Every gate reports that as
//! unknown, never as a pass and never as a violation: the pre-commit and
//! pre-push hooks, the CI check and its annotations, the sweep and the probe
//! that runs before a rule is written. Every repository is a throwaway.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

#[path = "support/rulebook.rs"]
mod rulebook;

/// Exits with the number in `exit-status` (0 when it is missing), after
/// printing a line shaped like a finding and a line giving the reason. The
/// finding line is never read when the script reaches no verdict.
const SCRIPT: &str = "#!/bin/sh\n\
echo 'a.txt:1 not a finding'\n\
echo 'the tool is unavailable'\n\
status=$(cat exit-status 2>/dev/null || echo 0)\n\
exit \"$status\"\n";

const REASON: &str = "the tool is unavailable";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn script_rule(strength: &str) -> Value {
    json!({
        "type": "rule",
        "strength": strength,
        "enforcer": {"kind": "validator", "command": "sh whetstone/checks/needs-a-tool.sh"}
    })
}

/// A repository cloned from a bare remote named `team`, with `wh` (the binary
/// under test) first on the PATH its hooks see, and the script in place.
/// `exit-status` is kept out of Git, so changing it never dirties the tree.
struct Repo {
    temp: tempfile::TempDir,
    root: PathBuf,
    path: OsString,
}

impl Repo {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let origin = temp.path().join("origin.git");
        let root = temp.path().join("repo");
        let bin_dir = temp.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        #[cfg(unix)]
        std::os::unix::fs::symlink(bin(), bin_dir.join("wh")).expect("symlink wh");
        let mut paths = vec![bin_dir];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        let path = std::env::join_paths(paths).expect("PATH");
        let repo = Self { temp, root, path };
        let origin = origin.to_str().expect("path");
        repo.git_in(
            repo.temp.path(),
            &["init", "-q", "--bare", "--initial-branch=main", origin],
        );
        let root = repo.root.to_str().expect("path");
        repo.git_in(
            repo.temp.path(),
            &["clone", "-q", "-o", "team", origin, root],
        );
        repo.write("whetstone/checks/needs-a-tool.sh", SCRIPT);
        repo.write("a.txt", "fine\n");
        repo.write(".git/info/exclude", "exit-status\n");
        repo
    }

    fn git_command(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .args([
                "-c",
                "user.name=Owner",
                "-c",
                "user.email=owner@example.invalid",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .current_dir(cwd)
            .env("PATH", &self.path)
            .output()
            .expect("git")
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let output = self.git_command(cwd, args);
        assert!(output.status.success(), "git {args:?}: {}", text(&output));
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.root, args)
    }

    fn wh(&self, args: &[&str]) -> Output {
        Command::new(bin())
            .args(args)
            .current_dir(&self.root)
            .output()
            .expect("run whetstone")
    }

    fn write(&self, path: &str, contents: &str) {
        let target = self.root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("dir");
        }
        fs::write(target, contents).expect("write");
    }

    fn status(&self, code: u8) {
        self.write("exit-status", &format!("{code}\n"));
    }

    fn rule(&self, id: &str, strength: &str) {
        rulebook::add_rule(
            &self.root,
            id,
            &format!("Statement of {id}."),
            &script_rule(strength),
        );
    }

    /// Commit with the hooks bypassed, so only what the test runs decides.
    fn commit_unchecked(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["-c", "core.hooksPath=/dev/null", "commit", "-qm", message]);
    }

    /// `git commit` through the installed hooks.
    fn commit(&self, message: &str) -> (bool, String) {
        self.git(&["add", "-A"]);
        let output = self.git_command(&self.root, &["commit", "-qm", message]);
        (output.status.success(), text(&output))
    }

    fn push(&self) -> (bool, String) {
        let output = self.git_command(&self.root, &["push", "team", "HEAD:main"]);
        (output.status.success(), text(&output))
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn wire_hooks(&self) {
        let wired = envelope(&self.wh(&["init", "--json", "--action", "wire", "--hooks"]));
        assert_eq!(wired["state"], "success", "{wired}");
    }
}

fn gate<'a>(response: &'a Value, id: &str) -> &'a Value {
    response["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == id)
        .unwrap_or_else(|| panic!("no gate {id} in {response}"))
}

#[test]
fn the_hooks_stop_on_a_must_rule_that_reached_no_verdict_and_say_why() {
    let repo = Repo::new();
    repo.rule("rule.needs-a-tool", "must");
    repo.wire_hooks();
    repo.status(0);
    let (committed, log) = repo.commit("base");
    assert!(committed, "{log}");
    let (pushed, log) = repo.push();
    assert!(pushed, "{log}");

    // Pre-commit: the commit is refused, naming the rule and the reason.
    repo.status(69);
    repo.write("a.txt", "changed\n");
    let (committed, log) = repo.commit("change");
    assert!(!committed, "an unknown must rule is not a pass: {log}");
    assert!(
        log.contains("rule.needs-a-tool") && log.contains(REASON),
        "{log}"
    );
    assert!(!log.contains("not a finding"), "no finding is read: {log}");

    // Pre-push: the push is refused the same way.
    repo.commit_unchecked("change");
    let before = repo.git(&["ls-remote", "team", "refs/heads/main"]);
    let (pushed, log) = repo.push();
    assert!(!pushed, "{log}");
    assert!(
        log.contains("rule.needs-a-tool") && log.contains(REASON),
        "{log}"
    );
    assert_eq!(repo.git(&["ls-remote", "team", "refs/heads/main"]), before);

    // Once the tool is there the same commit goes through.
    repo.status(0);
    let (pushed, log) = repo.push();
    assert!(pushed, "{log}");
    assert!(repo
        .git(&["ls-remote", "team", "refs/heads/main"])
        .starts_with(&repo.head()));
}

#[test]
fn the_hooks_report_a_should_rule_that_reached_no_verdict_and_go_on() {
    let repo = Repo::new();
    repo.rule("rule.soft", "should");
    repo.wire_hooks();
    repo.status(69);
    let (committed, log) = repo.commit("base");
    assert!(committed, "a should rule never blocks: {log}");
    assert!(
        log.contains("0 rule(s) hold")
            && log.contains("reached no verdict, which is not a pass: rule.soft"),
        "{log}"
    );
    let (pushed, log) = repo.push();
    assert!(pushed, "a should rule never blocks: {log}");
    assert!(
        log.contains("0 rule(s) hold")
            && log.contains("reached no verdict, which is not a pass: rule.soft"),
        "{log}"
    );
}

#[test]
fn ci_fails_closed_and_annotates_the_reason() {
    let repo = Repo::new();
    repo.rule("rule.needs-a-tool", "must");
    repo.rule("rule.soft", "should");
    repo.status(0);
    repo.commit_unchecked("base");
    let base = repo.head();
    repo.write("a.txt", "changed\n");
    repo.commit_unchecked("change");
    repo.status(69);

    let ci = repo.wh(&["check", "--ci", "--base", &base, "--json"]);
    assert_eq!(ci.status.code(), Some(3), "{}", text(&ci));
    let response = envelope(&ci);
    assert_eq!(response["state"], "unknown", "{response}");
    for id in ["rule.needs-a-tool", "rule.soft"] {
        let script = gate(&response, id);
        assert_eq!(script["state"], "unknown", "{script}");
        assert_eq!(script["failures"], json!([]), "{script}");
    }

    let annotated = repo.wh(&["check", "--ci", "--base", &base, "--format", "annotations"]);
    assert_eq!(annotated.status.code(), Some(3), "{}", text(&annotated));
    let commands = String::from_utf8_lossy(&annotated.stdout);
    let error = commands
        .lines()
        .find(|line| line.starts_with("::error ") && line.contains("rule.needs-a-tool"))
        .unwrap_or_else(|| panic!("no ::error for the must rule:\n{commands}"));
    assert!(error.contains(REASON), "{error}");
    let warning = commands
        .lines()
        .find(|line| line.starts_with("::warning ") && line.contains("rule.soft"))
        .unwrap_or_else(|| panic!("no ::warning for the should rule:\n{commands}"));
    assert!(warning.contains(REASON), "{warning}");
    assert!(!commands.contains("file=a.txt"), "{commands}");
}

#[test]
fn a_sweep_files_no_finding_for_it_and_keeps_the_others() {
    let repo = Repo::new();
    repo.rule("rule.soft", "should");
    rulebook::add_rule(
        &repo.root,
        "rule.no-fine",
        "No file says fine.",
        &json!({
            "type": "rule",
            "strength": "should",
            "enforcer": {"kind": "validator", "command": "sh whetstone/checks/no-fine.sh"}
        }),
    );
    repo.write(
        "whetstone/checks/no-fine.sh",
        "#!/bin/sh\ngrep -n fine a.txt | sed 's/^\\([0-9]*\\):.*/a.txt:\\1 says fine/' | grep . && exit 1\nexit 0\n",
    );
    let mut change = script_rule("should");
    change["mode"] = json!("change");
    rulebook::add_rule(&repo.root, "rule.counted", "Counted only.", &change);
    repo.commit_unchecked("base");
    repo.status(69);

    // Only should rules ran, so the state is the one a should-only check
    // always has; the rule that reached no verdict is named, never passed.
    let sweep = repo.wh(&["check", "--sweep", "--format", "json"]);
    let response = envelope(&sweep);
    assert_eq!(response["state"], "success", "{response}");
    assert_eq!(
        gate(&response, "rule.soft")["state"],
        "unknown",
        "{response}"
    );
    assert_eq!(
        response["data"]["no_verdict"],
        json!(["rule.counted", "rule.soft"]),
        "{response}"
    );
    assert!(
        response["summary"].as_str().is_some_and(|summary| summary
            .contains("2 should rule(s) reached no verdict, which is not a pass: rule.counted, rule.soft.")),
        "{response}"
    );
    let findings = response["data"]["findings"].as_array().expect("findings");
    assert!(
        findings
            .iter()
            .all(|finding| finding["responsibility"] != "rule.soft"),
        "{response}"
    );
    assert!(
        findings
            .iter()
            .any(|finding| finding["responsibility"] == "rule.no-fine"),
        "the other rules' findings are kept: {response}"
    );
    assert_eq!(
        response["data"]["counts"],
        json!([{"rule": "rule.counted", "state": "unknown", "count": 0}]),
        "{response}"
    );
    assert!(
        response["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("rule.counted unknown")),
        "{response}"
    );

    // The findings file a host reads holds the other rule's finding only.
    let printed = repo.wh(&["check", "--sweep"]);
    let file: Value = serde_json::from_slice(&printed.stdout).expect("findings.json");
    let responsibilities = file
        .as_array()
        .expect("array")
        .iter()
        .map(|finding| finding["responsibility"].clone())
        .collect::<Vec<_>>();
    assert_eq!(responsibilities, [json!("rule.no-fine")], "{file}");
}

#[test]
fn the_probe_refuses_a_rule_that_reached_no_verdict_and_says_why() {
    let repo = Repo::new();
    rulebook::add_rule(&repo.root, "rule.other", "Other.", &script_rule("should"));
    repo.status(0);
    repo.commit_unchecked("base");
    repo.status(69);
    let before = fs::read(repo.root.join("whetstone/rulebook.json")).expect("rulebook");

    let definition = script_rule("must").to_string();
    let drafted = envelope(&repo.wh(&[
        "change",
        "--json",
        "--kind",
        "rule",
        "--record-id",
        "rule.needs-a-tool",
        "--content",
        "A tool holds.",
        "--rationale",
        "Proves the probe.",
        "--definition",
        &definition,
    ]));
    assert_ne!(drafted["state"], "success", "{drafted}");
    assert!(drafted.to_string().contains(REASON), "{drafted}");
    assert_eq!(
        fs::read(repo.root.join("whetstone/rulebook.json")).expect("rulebook"),
        before,
        "nothing is written: {drafted}"
    );
}
