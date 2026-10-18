//! Sweep findings for a night sweep's controller: each carries the tool's own
//! fix when the rule's tool can rewrite files, as a diff that applies with
//! `git apply`, and the working tree is never written by the check.

mod support;

use support::{git, Repo};

fn cargo_available() -> bool {
    std::process::Command::new("cargo")
        .args(["fmt", "--version"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn a_formatter_finding_carries_a_diff_that_applies_and_a_plain_lint_finding_carries_none() {
    if !cargo_available() {
        eprintln!("cargo fmt is not installed; skipping");
        return;
    }
    let repo = Repo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    repo.write("src/main.rs", "fn main(){let x=1;\nprintln!(\"{}\",x);}\n");
    repo.write(
        "lint.sh",
        "#!/bin/sh\necho \"src/main.rs:1: W100 something is off\"\nexit 1\n",
    );
    repo.commit("base");
    repo.rule(
        "rule.formatted",
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"formatter","tool":"cargo fmt -- --check"}}"#,
    );
    repo.rule(
        "rule.plain-lint",
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"lint","tool":"sh lint.sh","code":"W100"}}"#,
    );
    let before = git(&repo.root, &["status", "--porcelain"]);

    let sweep = repo.wh_json(&["check", "--sweep", "--format", "findings"]);
    let findings = sweep["data"]["findings"].as_array().expect("findings");
    let formatted = findings
        .iter()
        .find(|finding| finding["responsibility"] == "rule.formatted")
        .unwrap_or_else(|| panic!("no formatter finding in {sweep}"));
    let fix = formatted["fix"]
        .as_str()
        .expect("the formatter finding carries a fix");
    assert!(
        fix.starts_with("diff --git a/src/main.rs b/src/main.rs"),
        "{fix}"
    );
    let plain = findings
        .iter()
        .find(|finding| finding["responsibility"] == "rule.plain-lint")
        .unwrap_or_else(|| panic!("no lint finding in {sweep}"));
    assert!(plain.get("fix").is_none(), "{plain}");

    // The check wrote nothing; the diff applies and the formatter then passes.
    assert_eq!(git(&repo.root, &["status", "--porcelain"]), before);
    let patch = repo.root.join("..").join("fix.patch");
    std::fs::write(&patch, fix).expect("patch");
    git(
        &repo.root,
        &["apply", "--check", patch.to_str().expect("path")],
    );
    git(&repo.root, &["apply", patch.to_str().expect("path")]);
    let (_, recheck) = {
        let output = repo.wh(&["check", "--rule", "rule.formatted"]);
        (output.status.code(), support::json(&output))
    };
    let gate = recheck["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == "rule.formatted")
        .expect("gate");
    assert_eq!(gate["state"], "pass", "{recheck}");

    // findings.json keeps every field it had, and `fix` only where present.
    let lines = sweep["data"]["findings_json"].as_str().expect("json");
    let parsed: serde_json::Value = serde_json::from_str(lines).expect("array");
    for finding in parsed.as_array().expect("array") {
        for field in [
            "fingerprint",
            "subjectDigest",
            "class",
            "priority",
            "title",
            "responsibility",
        ] {
            assert!(finding.get(field).is_some(), "{finding} lacks {field}");
        }
    }
}
