//! The pre-push gate through real `git push` to a bare remote, with the
//! hooks `wh init --action wire --hooks` installs. A push is refused unless
//! the pushed commit itself holds every must rule: it must be checked out
//! with no uncommitted tracked changes, a broken mechanical rule or a failed
//! re-proof blocks it, and a must review needs an attestation at the pushed
//! commit. A new branch is checked against the remote's default branch. A
//! repository whose only rule is a review can still commit.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
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

fn node_available() -> bool {
    let found = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        found || std::env::var_os("CI").is_none(),
        "node is required in CI"
    );
    found
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
            &["init", "-q", "--bare", repo.origin.to_str().expect("path")],
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
            .env_remove("BEADS_DIR")
            .env("WHETSTONE_JEV_OFFLINE", "1")
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

    fn change(&self, request: &str, args: &[&str]) -> serde_json::Value {
        let mut probe = vec!["change", "--json", "--request-id", request];
        probe.extend_from_slice(args);
        let inspected = json(&run(&probe, &self.root));
        let revision = inspected["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let token = inspected["resume_token"]
            .as_str()
            .expect("token")
            .to_string();
        let mut record = probe.clone();
        record.extend_from_slice(&["--expected-revision", &revision, "--resume", &token]);
        json(&run(&record, &self.root))
    }

    fn accept(&self, drafted: &serde_json::Value) {
        assert_eq!(drafted["state"], "success", "{drafted}");
        let proposal = drafted["data"]["proposal"]["id"]
            .as_str()
            .expect("proposal");
        let accepted = json(&run(
            &[
                "change",
                "--json",
                "--request-id",
                &format!("accept-{proposal}"),
                "--accept",
                proposal,
            ],
            &self.root,
        ));
        assert_eq!(accepted["state"], "success", "{accepted}");
    }

    fn rule(&self, id: &str, statement: &str, definition: serde_json::Value) {
        self.accept(&self.change(
            &format!("{id}-draft"),
            &[
                "--kind",
                "rule",
                "--record-id",
                id,
                "--content",
                statement,
                "--rationale",
                "The team relies on it.",
                "--definition",
                &definition.to_string(),
            ],
        ));
    }

    fn agree(&self) {
        let inspected = json(&run(&["init", "--json", "--request-id", "i1"], &self.root));
        let revision = inspected["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
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
                &revision,
                "--resume",
                &token,
                "--mission",
                "Nothing broken reaches the team.",
            ],
            &self.root,
        ));
        assert_eq!(
            agreed["data"]["records"][0]["id"], "mission.project",
            "{agreed}"
        );
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
    repo.agree();
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
fn a_push_whose_change_breaks_a_drive_proof_is_refused_with_the_reprove_command() {
    if !node_available() {
        eprintln!("SKIP: node is required for the driver");
        return;
    }
    let repo = Repo::new();
    repo.agree();
    repo.write("check.sh", "#!/bin/sh\necho \"all good\"\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            repo.root.join("check.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("chmod");
    }
    repo.write(
        "whetstone/verify/drive.mjs",
        &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/verify/drive.mjs"))
            .expect("driver"),
    );
    repo.write("whetstone/verify/driver.json", r#"{"surface":"cli"}"#);
    let feature = serde_json::json!({
        "type": "feature",
        "summary": "Runs the check",
        "area": "CLI",
        "sweep_order": 1,
        "user_path": "Run ./check.sh.",
        "drive_steps": ["run ./check.sh", "expect-output all good"],
        "proof": "It prints all good.",
        "entry_points": ["check.sh"],
        "serves": ["mission.project"],
        "proven_by": ["rule.check-journey"],
    });
    repo.accept(&repo.change(
        "feature",
        &[
            "--kind",
            "feature",
            "--record-id",
            "feature.check",
            "--content",
            "Check",
            "--rationale",
            "the core path",
            "--definition",
            &feature.to_string(),
        ],
    ));
    repo.rule(
        "rule.check-journey",
        "The check journey is proven by driving it.",
        serde_json::json!({"type": "rule", "strength": "must", "enforcer": {"kind": "drive", "feature": "feature.check"}}),
    );
    repo.wire_hooks();
    repo.commit_unchecked("mapped");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(pushed, "{log}");

    // A change to the entry point that keeps the behaviour goes through, and
    // the push re-proves the feature at the pushed commit.
    repo.write("check.sh", "#!/bin/sh\n# quieter\necho \"all good\"\n");
    repo.commit_unchecked("tweak");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(pushed, "{log}");
    let base = repo.head();
    assert_eq!(repo.remote("main").as_deref(), Some(base.as_str()));
    let dash = json(&run(&["dash", "--json"], &repo.root));
    let reproved = dash["data"]["changelog"]
        .as_array()
        .expect("changelog")
        .iter()
        .flat_map(|entry| entry["records"].as_array().cloned().unwrap_or_default())
        .filter_map(|item| item.get("record").cloned())
        .filter(|record| {
            record["record_type"] == "verification_receipt"
                && record["record"]["subject"]["stable_id"] == "gate:rule.check-journey"
                && record["record"]["verification"] == "pass"
        })
        .any(|record| {
            record["record"]["evidence"]
                .as_array()
                .is_some_and(|evidence| {
                    evidence.iter().any(|item| {
                        item["system"] == "git_head" && item["locator"] == base.as_str()
                    })
                })
        });
    assert!(reproved, "a passing receipt at the pushed commit: {dash}");

    // The entry point changes and the behaviour with it: the push re-proves
    // the feature, and the proof no longer holds.
    repo.write("check.sh", "#!/bin/sh\necho \"all bad\"\n");
    repo.commit_unchecked("regress");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(!pushed, "{log}");
    assert!(log.contains("rule.check-journey"), "{log}");
    assert!(
        log.contains("wh check --rule rule.check-journey")
            || log.contains("wh check --feature feature.check"),
        "the refusal names the re-prove command: {log}"
    );
    assert_eq!(repo.remote("main").as_deref(), Some(base.as_str()));
}

#[test]
fn a_must_review_blocks_the_push_until_it_is_attested_at_the_pushed_commit() {
    let repo = Repo::new();
    repo.agree();
    repo.rule(
        "rule.reviewed",
        "A reviewer checked the change.",
        serde_json::json!({"type": "rule", "strength": "must", "enforcer": {"kind": "review", "reviewer": {"by": "interrogate"}}}),
    );
    repo.wire_hooks();

    // With only a review rule there is nothing fast to run: the commit goes
    // through and says why.
    repo.write("a.txt", "a\n");
    repo.git(&["add", "-A"]);
    let committed = repo.git_command(&repo.root, &["commit", "-qm", "one"]);
    let said = String::from_utf8_lossy(&committed.stderr).to_string();
    assert!(committed.status.success(), "{said}");
    assert!(said.contains("No fast mechanical rule"), "{said}");

    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(!pushed, "an unreviewed change is not a pass: {log}");
    assert!(
        log.contains("wh check --attest rule.reviewed"),
        "the refusal says how to record the review: {log}"
    );
    assert_eq!(repo.remote("main"), None);

    let attest = |verdict: &str, notes: &str| {
        json(&run(
            &[
                "check",
                "--json",
                "--attest",
                "rule.reviewed",
                "--verdict",
                verdict,
                "--reviewer",
                "interrogate",
                "--notes",
                notes,
            ],
            &repo.root,
        ))
    };
    // An untracked scratch file while reviewing is not part of the push and
    // must not make the attestation miss it.
    repo.write("scratch-notes.txt", "thinking\n");
    let attested = attest("pass", "x");
    assert_eq!(attested["state"], "success", "{attested}");

    // A later verdict at the same commit replaces the earlier one.
    let withdrawn = attest("fail", "actually not");
    assert!(
        withdrawn["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("was recorded"),
        "{withdrawn}"
    );
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(!pushed, "a failing review blocks the push: {log}");
    let again = attest("pass", "fixed my mind");
    assert_eq!(again["state"], "success", "{again}");
    let repeated = attest("pass", "fixed my mind");
    assert!(
        repeated["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("already recorded"),
        "an exact repeat says so: {repeated}"
    );
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(pushed, "{log}");
    assert_eq!(repo.remote("main"), Some(repo.head()));

    // One more commit: the earlier attestation was for another commit.
    let reviewed = repo.head();
    repo.write("b.txt", "b\n");
    repo.commit_unchecked("two");
    let (pushed, log) = repo.push(&["team", "HEAD:main"]);
    assert!(!pushed, "an attestation covers its own commit only: {log}");
    assert!(log.contains("No review attestation"), "{log}");
    assert_eq!(repo.remote("main").as_deref(), Some(reviewed.as_str()));
}

#[test]
fn a_new_branch_is_checked_against_the_remotes_default_branch() {
    let repo = Repo::new();
    repo.agree();
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
