//! The pre-push gate through real `git push` to a bare remote, with the
//! hooks `wh init --action wire --hooks` installs. A push is refused unless
//! the pushed commit itself holds every must rule: it must be checked out
//! with no uncommitted tracked changes, and a broken mechanical rule blocks
//! it. A new branch is checked against the remote's
//! default branch.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/rulebook.rs"]
mod rulebook;

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

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// A repository cloned from a bare remote named `team`, with `wh` (the
/// binary under test) first on the PATH its hooks see.
struct Repo {
    temp: tempfile::TempDir,
    root: PathBuf,
    origin: PathBuf,
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
        let repo = Self {
            temp,
            root,
            origin,
            path,
        };
        repo.git_in(
            repo.temp.path(),
            &[
                "init",
                "-q",
                "--bare",
                "--initial-branch=main",
                repo.origin.to_str().expect("path"),
            ],
        );
        repo.git_in(
            repo.temp.path(),
            &[
                "clone",
                "-q",
                "-o",
                "team",
                repo.origin.to_str().expect("path"),
                repo.root.to_str().expect("path"),
            ],
        );
        repo
    }

    fn git_command(&self, cwd: &Path, args: &[&str]) -> Output {
        Command::new("git")
            .args([
                "-c",
                "user.name=Owner",
                "-c",
                "user.email=owner@example.invalid",
            ])
            .args(args)
            .current_dir(cwd)
            .env("PATH", &self.path)
            .output()
            .expect("git")
    }

    fn git_in(&self, cwd: &Path, args: &[&str]) -> String {
        let output = self.git_command(cwd, args);
        assert!(
            output.status.success(),
            "git {args:?}: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn git(&self, args: &[&str]) -> String {
        self.git_in(&self.root, args)
    }

    /// Commit everything with the hooks bypassed, so only the push gate
    /// decides.
    fn commit_unchecked(&self, message: &str) {
        self.git(&["add", "-A"]);
        self.git(&["-c", "core.hooksPath=/dev/null", "commit", "-qm", message]);
    }

    /// `git push`; returns whether it succeeded and everything it printed.
    fn push(&self, args: &[&str]) -> (bool, String) {
        let mut all = vec!["push"];
        all.extend_from_slice(args);
        let output = self.git_command(&self.root, &all);
        (
            output.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
        )
    }

    fn remote(&self, branch: &str) -> Option<String> {
        let refs = self.git(&["ls-remote", "team", &format!("refs/heads/{branch}")]);
        refs.split_whitespace().next().map(str::to_owned)
    }

    fn head(&self) -> String {
        self.git(&["rev-parse", "HEAD"]).trim().to_string()
    }

    fn write(&self, path: &str, text: &str) {
        let target = self.root.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("dir");
        }
        fs::write(target, text).expect("write");
    }

    /// Write a rule into the rulebook; the next commit carries it.
    fn rule(&self, id: &str, statement: &str, definition: serde_json::Value) {
        rulebook::add_rule(&self.root, id, statement, &definition);
    }

    fn wire_hooks(&self) {
        let wired = json(&run(
            &["init", "--json", "--action", "wire", "--hooks"],
            &self.root,
        ));
        assert_eq!(wired["state"], "success", "{wired}");
        for name in ["pre-commit", "pre-push"] {
            let hook = fs::read_to_string(self.root.join(".git/hooks").join(name)).expect("hook");
            assert_eq!(hook.lines().nth(1), Some("# whetstone-hook v1"));
        }
    }
}

fn no_print() -> serde_json::Value {
    serde_json::json!({
        "type": "rule",
        "strength": "must",
        "enforcer": {
            "kind": "ast",
            "language": "python",
            "query": "((call function: (identifier) @f) @match (#eq? @f \"print\"))"
        }
    })
}

#[test]
fn a_push_is_refused_unless_the_pushed_commit_itself_holds_the_must_rules() {
    let repo = Repo::new();
    repo.rule("rule.no-print", "No print calls.", no_print());
    repo.wire_hooks();
    repo.write("app.py", "import logging\n");
    repo.commit_unchecked("base");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(pushed, "a clean first push goes through: {log}");
    let base = repo.head();
    assert_eq!(repo.remote("main").as_deref(), Some(base.as_str()));

    // A committed violation is refused and names the finding.
    repo.write("bad.py", "print(\"x\")\n");
    repo.commit_unchecked("bad");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(!pushed, "{log}");
    assert!(
        log.contains("rule.no-print") && log.contains("bad.py:1"),
        "{log}"
    );
    assert!(log.contains("whetstone pre-push: blocked"), "{log}");
    assert_eq!(repo.remote("main").as_deref(), Some(base.as_str()));

    // Fixing only the working tree is not fixing the pushed commit.
    repo.write("bad.py", "import logging\n");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(!pushed, "a dirty working tree cannot vouch for HEAD: {log}");
    assert!(log.contains("uncommitted changes"), "{log}");
    assert_eq!(repo.remote("main").as_deref(), Some(base.as_str()));

    // Committing the fix clears it; an untracked violating file is not
    // part of the push and does not block it.
    repo.commit_unchecked("fix");
    repo.write("scratch.py", "print(\"not tracked\")\n");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(pushed, "{log}");
    assert_eq!(repo.remote("main"), Some(repo.head()));

    // The hook checks the commit it pushes, and only that one.
    let elsewhere = json(&run(
        &["check", "--json", "--pushed", &base, "--base", &base],
        &repo.root,
    ));
    assert_eq!(elsewhere["state"], "unknown", "{elsewhere}");
    assert!(elsewhere["summary"]
        .as_str()
        .expect("summary")
        .contains("is checked out"));
}

#[test]
fn a_new_branch_is_checked_against_the_remotes_default_branch() {
    let repo = Repo::new();
    repo.rule("rule.no-print", "No print calls.", no_print());
    repo.wire_hooks();
    // The default branch is `trunk`, and it already carries a violation that
    // predates the gate (pushed with the hook bypassed).
    repo.git(&["checkout", "-q", "-b", "trunk"]);
    repo.write("legacy.py", "print(\"legacy\")\n");
    repo.commit_unchecked("legacy");
    let (pushed, log) = repo.push(&["--no-verify", "team", "trunk"]);
    assert!(pushed, "{log}");
    repo.git_in(&repo.origin, &["symbolic-ref", "HEAD", "refs/heads/trunk"]);
    repo.git(&["remote", "set-head", "team", "--auto"]);
    assert_eq!(
        repo.git(&["symbolic-ref", "refs/remotes/team/HEAD"]).trim(),
        "refs/remotes/team/trunk"
    );
    let whole = json(&run(&["check", "--json"], &repo.root));
    assert_eq!(whole["state"], "violated", "the whole tree has it: {whole}");

    // A new branch adds clean work: only what it adds is checked.
    repo.git(&["checkout", "-q", "-b", "topic"]);
    repo.write("topic.py", "import logging\n");
    repo.commit_unchecked("topic");
    let (pushed, log) = repo.push(&["team", "topic"]);
    assert!(
        pushed,
        "the first push of a branch uses the remote's HEAD as the base: {log}"
    );
    assert_eq!(repo.remote("topic"), Some(repo.head()));

    // The same base catches what the branch itself adds.
    repo.git(&["checkout", "-q", "-b", "topic-bad", "trunk"]);
    repo.write("worse.py", "print(\"new\")\n");
    repo.commit_unchecked("worse");
    let (pushed, log) = repo.push(&["team", "topic-bad"]);
    assert!(!pushed, "{log}");
    assert!(log.contains("worse.py:1"), "{log}");
    assert!(
        !log.contains("legacy.py"),
        "only the branch's own change is checked: {log}"
    );
    assert_eq!(repo.remote("topic-bad"), None);
}
