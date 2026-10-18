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

const UNWRAP_QUERY: &str = r#"((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method \"unwrap\"))"#;
const DBG_QUERY: &str =
    r#"((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))"#;

#[test]
fn a_change_s_flags_become_annotations_on_the_lines_it_touched() {
    let repo = Repo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    repo.write("src/main.rs", "fn main() {\n    old().unwrap();\n}\n");
    repo.commit("base");
    let base = git(&repo.root, &["rev-parse", "HEAD"]).trim().to_string();
    repo.rule(
        "rule.no-unwrap",
        &format!(r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}}}}"#),
    );
    repo.rule(
        "rule.no-dbg",
        &format!(r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{DBG_QUERY}","language":"rust"}}}}"#),
    );
    repo.write(
        "src/main.rs",
        "fn main() {\n    old().unwrap();\n    new().unwrap();\n    dbg!(1);\n}\n",
    );
    repo.commit("change");
    let output = support::run(
        &["check", "--base", &base, "--format", "annotations"],
        &repo.root,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    let mut commands = stdout
        .lines()
        .filter(|line| line.starts_with("::"))
        .collect::<Vec<_>>();
    commands.sort();
    assert_eq!(
        commands,
        [
            "::error file=src/main.rs,line=3,title=rule.no-unwrap%3A Statement of rule.no-unwrap.::new().unwrap() (wh explain rule.no-unwrap)",
            "::warning file=src/main.rs,line=4,title=rule.no-dbg%3A Statement of rule.no-dbg.::dbg!(1) (wh explain rule.no-dbg)",
        ],
        "the unchanged line 2 is not annotated:\n{stdout}"
    );
}

#[test]
fn the_ci_scaffold_uses_annotations_and_passes_actionlint() {
    let template = include_str!("../assets/ci/whetstone.yml");
    assert!(template.contains("--format annotations"));
    let has_actionlint = std::process::Command::new("actionlint")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !has_actionlint {
        eprintln!("actionlint is not installed; skipping the lint half");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let workflows = temp.path().join(".github/workflows");
    std::fs::create_dir_all(&workflows).expect("dir");
    let rendered = template
        .replace("__WHETSTONE_VERSION__", "0.13.0")
        .replace("__BD_VERSION__", "1.3.0");
    let path = workflows.join("whetstone.yml");
    std::fs::write(&path, rendered).expect("write");
    let output = std::process::Command::new("actionlint")
        .arg(&path)
        .current_dir(temp.path())
        .output()
        .expect("actionlint");
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
