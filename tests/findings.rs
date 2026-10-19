//! Sweep findings: `wh check --sweep` turns every flag of a rule below must
//! into one finding a night sweep's controller works. The sweep reads the
//! rules from `whetstone/rulebook.json`, written straight into the file.
//! Every repository is a throwaway.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

#[path = "support/rulebook.rs"]
mod rulebook;

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

/// The binary with only the system tools on `PATH`.
fn run_with_system_tools(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(args)
        .current_dir(root)
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
        .output()
        .expect("whetstone")
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no JSON envelope ({error}): {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn repository() -> TempDir {
    let temp = TempDir::new().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::write(
        root.join("src/config.py"),
        "import json\n\ndef load(path):\n    return json.load(open(path))\n",
    )
    .expect("write");
    std::fs::write(root.join("README.md"), "notes\n").expect("write");
    std::fs::write(root.join(".gitignore"), "secrets/\n").expect("write");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);
    temp
}

const PRINT: &str = "((call function: (identifier) @f) @match (#eq? @f \"print\"))";

fn print_rule(strength: &str) -> Value {
    json!({
        "type": "rule",
        "strength": strength,
        "enforcer": {"kind": "ast", "language": "python", "query": PRINT},
    })
}

/// One finding as findings.json carries it, byte for byte.
fn finding_line(
    rule: &str,
    subject: &str,
    flagged: &str,
    class: &str,
    priority: u8,
    title: &str,
    unsure: Option<&str>,
) -> (String, String) {
    let fingerprint = sha256(format!("{rule}\0github.com/example/app\0{subject}").as_bytes());
    let unsure = unsure.map_or_else(String::new, |reason| format!(",\"unsure\":\"{reason}\""));
    let line = format!(
        "{{\"fingerprint\":\"{fingerprint}\",\"subjectDigest\":\"{}\",\"class\":\"{class}\",\"priority\":{priority},\"title\":\"{title}\",\"responsibility\":\"{rule}\"{unsure}}}",
        sha256(flagged.as_bytes())
    );
    (fingerprint, line)
}

/// findings.json for these findings: a JSON array, one finding per line,
/// sorted by fingerprint.
fn findings_json(mut findings: Vec<(String, String)>) -> String {
    findings.sort();
    let lines = findings
        .into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>();
    format!("[\n{}\n]\n", lines.join(",\n"))
}

#[test]
fn a_sweep_reads_the_rulebook_file_and_its_findings_are_byte_stable_and_below_must() {
    let temp = repository();
    let root = temp.path();
    git(
        root,
        &["remote", "add", "origin", "git@github.com:example/app.git"],
    );
    std::fs::write(root.join("src/app.py"), "print('one')\nvalue = 1\n").expect("write");
    let mut should = print_rule("should");
    should["sweep"] = json!("draft_pr");
    should["sweep_priority"] = json!(0);
    rulebook::add_rule(
        root,
        "rule.print-should",
        "Prefer logging to print.",
        &should,
    );
    let mut doubtful = print_rule("should");
    doubtful["sweep_unsure"] = json!([{"matching": "print\\('one'\\)", "reason": "a one-off print in a demo is borderline"}]);
    rulebook::add_rule(root, "rule.print-doubtful", "Print is a smell.", &doubtful);
    rulebook::add_rule(
        root,
        "rule.print-must",
        "No print at all.",
        &print_rule("must"),
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "rules"]);

    // Findings for the rules below must only (a sweep prints them by
    // default), byte-identical across runs and to the findings contract:
    // fingerprint sha256(rule \0 origin \0 subject), the flagged line's
    // digest, class, priority, title, responsibility and unsure.
    let first = run_with_system_tools(root, &["check", "--sweep"]);
    assert_eq!(
        first.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let second = run_with_system_tools(root, &["check", "--sweep", "--format", "findings"]);
    assert_eq!(
        first.stdout, second.stdout,
        "the same state gives the same findings"
    );
    let should_one = finding_line(
        "rule.print-should",
        "src/app.py",
        "print('one')",
        "draft_pr",
        0,
        "Prefer logging to print. (src/app.py)",
        None,
    );
    assert_eq!(
        String::from_utf8_lossy(&first.stdout),
        findings_json(vec![
            should_one.clone(),
            finding_line(
                "rule.print-doubtful",
                "src/app.py",
                "print('one')",
                "discuss",
                2,
                "Print is a smell. (src/app.py)",
                Some("a one-off print in a demo is borderline"),
            ),
        ]),
        "a must rule never emits"
    );
    // One JSON document (Messo's `messo-ctl observe` parses the whole
    // findings.json).
    serde_json::from_slice::<Vec<Value>>(&first.stdout).expect("findings are one JSON array");

    // The fingerprint holds while the flagged content changes; the digest
    // moves, and the doubt covers only the flags its pattern matches. A
    // repeat in the same subject is numbered in source order.
    std::fs::write(
        root.join("src/app.py"),
        "print('two')\nvalue = 1\nprint('two')\n",
    )
    .expect("write");
    let changed = run_with_system_tools(root, &["check", "--sweep"]);
    let should_two = finding_line(
        "rule.print-should",
        "src/app.py",
        "print('two')",
        "draft_pr",
        0,
        "Prefer logging to print. (src/app.py)",
        None,
    );
    assert_eq!(should_two.0, should_one.0);
    assert_ne!(should_two.1, should_one.1);
    let doubtful_two = |subject: &str| {
        finding_line(
            "rule.print-doubtful",
            subject,
            "print('two')",
            "discuss",
            2,
            "Print is a smell. (src/app.py)",
            None,
        )
    };
    assert_eq!(
        String::from_utf8_lossy(&changed.stdout),
        findings_json(vec![
            should_two,
            finding_line(
                "rule.print-should",
                "src/app.py#2",
                "print('two')",
                "draft_pr",
                0,
                "Prefer logging to print. (src/app.py)",
                None,
            ),
            doubtful_two("src/app.py"),
            doubtful_two("src/app.py#2"),
        ])
    );

    // A should flag is never required; the must rule's is. Each result
    // names its rule by id, digest and the rulebook's revision.
    let checked = envelope(&run_with_system_tools(root, &["check", "--json"]));
    let head = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .expect("git");
    let head = String::from_utf8_lossy(&head.stdout).trim().to_string();
    assert_eq!(
        checked["data"]["rulebook"],
        json!({"path": "whetstone/rulebook.json", "revision": head})
    );
    let result = |id: &str| {
        checked["data"]["report"]["results"]
            .as_array()
            .and_then(|results| {
                results
                    .iter()
                    .find(|result| result["requirement_id"] == format!("gate:{id}"))
            })
            .cloned()
            .unwrap_or_else(|| panic!("{id} checked: {}", checked["data"]["report"]))
    };
    assert_eq!(result("rule.print-should")["required"], json!(false));
    assert_eq!(result("rule.print-must")["required"], json!(true));
    let governing = &result("rule.print-must")["governing_rule"];
    assert_eq!(governing["id"], "rule.print-must");
    assert_eq!(governing["revision"], json!(head));
    assert!(
        governing["digest"]
            .as_str()
            .is_some_and(|digest| digest.starts_with("sha256:")),
        "{governing}"
    );

    // `--format json` keeps the envelope, with the same findings in it.
    let enveloped = envelope(&run_with_system_tools(
        root,
        &["check", "--sweep", "--format", "json"],
    ));
    assert_eq!(enveloped["schema"], "whetstone.command-response.v1");
    assert_eq!(
        enveloped["data"]["findings_json"]
            .as_str()
            .map(str::as_bytes),
        Some(changed.stdout.as_slice()),
        "{enveloped}"
    );
}

#[test]
fn a_check_without_a_rulebook_or_with_a_malformed_one_never_passes() {
    let temp = repository();
    let root = temp.path();
    for args in [
        &["check", "--json"][..],
        &["check", "--ci", "--json"],
        &["check", "--staged", "--json"],
        &["check", "--sweep", "--format", "json"],
    ] {
        let response = envelope(&run_with_system_tools(root, args));
        assert_eq!(response["state"], "unknown", "{args:?}: {response}");
        assert!(
            response["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("whetstone/rulebook.json does not exist")),
            "{response}"
        );
        assert!(
            response["permitted_actions"][0]
                .as_str()
                .is_some_and(|action| action.contains("wh change")
                    && action.contains("wh init --action export")),
            "{response}"
        );
    }
    let swept = run_with_system_tools(root, &["check", "--sweep"]);
    assert_ne!(swept.status.code(), Some(0));
    assert!(swept.stdout.is_empty(), "no findings.json without rules");
    assert!(String::from_utf8_lossy(&swept.stderr).contains("does not exist"));

    // A local check reads the file as it is; CI only a committed one.
    rulebook::add_rule(
        root,
        "rule.print-must",
        "No print at all.",
        &print_rule("must"),
    );
    let local = envelope(&run_with_system_tools(root, &["check", "--json"]));
    assert_eq!(local["state"], "success", "{local}");
    let ci = envelope(&run_with_system_tools(root, &["check", "--ci", "--json"]));
    assert_eq!(ci["state"], "unknown", "{ci}");
    assert!(
        ci["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("uncommitted changes")),
        "{ci}"
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "rules"]);
    let ci = envelope(&run_with_system_tools(root, &["check", "--ci", "--json"]));
    assert_eq!(ci["state"], "success", "{ci}");
    let unknown = envelope(&run_with_system_tools(
        root,
        &["check", "--json", "--rule", "rule.print-elsewhere"],
    ));
    assert_eq!(unknown["state"], "unknown", "{unknown}");
    assert!(unknown["summary"]
        .as_str()
        .is_some_and(|summary| summary.contains("rule.print-elsewhere")));

    // A malformed rulebook names the field.
    for (name, extra, field) in [
        ("priority", json!({"sweep_priority": 5}), "rules[1]"),
        ("unknown field", json!({"colour": "red"}), "colour"),
    ] {
        let mut bad = print_rule("should");
        for (key, value) in extra.as_object().expect("object") {
            bad[key] = value.clone();
        }
        git(root, &["checkout", "-q", "--", "whetstone/rulebook.json"]);
        rulebook::add_rule(root, "rule.print-bad", "Bad.", &bad);
        let response = envelope(&run_with_system_tools(root, &["check", "--json"]));
        assert_eq!(response["state"], "unknown", "{name}: {response}");
        assert!(
            response["summary"].as_str().is_some_and(|summary| summary
                .contains("whetstone/rulebook.json is not a rulebook")
                && summary.contains(field)),
            "{name}: {response}"
        );
    }
}

#[test]
fn a_sweep_covers_a_large_repository_with_in_repository_symlinks() {
    let temp = repository();
    let root = temp.path();
    let bulk = root.join("bulk");
    std::fs::create_dir(&bulk).expect("bulk");
    for index in 0..4_200 {
        std::fs::write(bulk.join(format!("{index}.txt")), "x\n").expect("write");
    }
    std::fs::write(root.join("src/app.py"), "print('one')\n").expect("write");
    std::os::unix::fs::symlink("src", root.join("alias")).expect("directory link");
    std::os::unix::fs::symlink("README.md", root.join("NOTES.md")).expect("file link");
    rulebook::add_rule(
        root,
        "rule.print-should",
        "Prefer logging to print.",
        &print_rule("should"),
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "bulk"]);

    let swept = run_with_system_tools(root, &["check", "--sweep"]);
    assert_eq!(
        swept.status.code(),
        Some(0),
        "{}\n{}",
        String::from_utf8_lossy(&swept.stdout),
        String::from_utf8_lossy(&swept.stderr)
    );
    let findings: Vec<Value> = serde_json::from_slice(&swept.stdout).expect("findings");
    assert!(
        findings
            .iter()
            .any(|finding| finding["responsibility"] == "rule.print-should"),
        "{findings:?}"
    );
}

fn cargo_fmt_available() -> bool {
    Command::new("cargo")
        .args(["fmt", "--version"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn a_formatter_finding_carries_a_diff_that_applies_and_a_plain_lint_finding_carries_none() {
    if !cargo_fmt_available() {
        eprintln!("cargo fmt is not installed; skipping");
        return;
    }
    let temp = repository();
    let root = temp.path();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest");
    std::fs::write(
        root.join("src/main.rs"),
        "fn main(){let x=1;\nprintln!(\"{}\",x);}\n",
    )
    .expect("source");
    std::fs::write(
        root.join("lint.sh"),
        "#!/bin/sh\necho \"src/main.rs:1: W100 something is off\"\nexit 1\n",
    )
    .expect("lint");
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "rust"]);
    rulebook::add_rule(
        root,
        "rule.formatted",
        "Rust code is formatted.",
        &json!({"type":"rule","strength":"should","enforcer":{"kind":"formatter","tool":"cargo fmt -- --check"}}),
    );
    rulebook::add_rule(
        root,
        "rule.plain-lint",
        "W100 never fires.",
        &json!({"type":"rule","strength":"should","enforcer":{"kind":"lint","tool":"sh lint.sh","code":"W100"}}),
    );
    let status = || {
        Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(root)
            .output()
            .expect("status")
            .stdout
    };
    let before = status();
    let wh = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_whetstone"))
            .args(args)
            .current_dir(root)
            .env("GIT_CEILING_DIRECTORIES", root.parent().expect("parent"))
            .output()
            .expect("whetstone")
    };

    let sweep = envelope(&wh(&["check", "--sweep", "--format", "json"]));
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
    assert_eq!(status(), before);
    let patch = root.parent().expect("parent").join("fix.patch");
    std::fs::write(&patch, fix).expect("patch");
    git(root, &["apply", "--check", patch.to_str().expect("path")]);
    git(root, &["apply", patch.to_str().expect("path")]);
    let recheck = envelope(&wh(&["check", "--json", "--rule", "rule.formatted"]));
    let gate = recheck["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == "rule.formatted")
        .expect("gate");
    assert_eq!(gate["state"], "pass", "{recheck}");

    // findings.json keeps its required fields, and `fix` only where present.
    let schema: Value = serde_json::from_str(include_str!("../references/findings-v1.schema.json"))
        .expect("schema");
    let allowed = schema["items"]["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let parsed: Value =
        serde_json::from_str(sweep["data"]["findings_json"].as_str().expect("json"))
            .expect("array");
    for finding in parsed.as_array().expect("array") {
        for field in schema["items"]["required"].as_array().expect("required") {
            let field = field.as_str().expect("field");
            assert!(finding.get(field).is_some(), "{finding} lacks {field}");
        }
        for key in finding.as_object().expect("object").keys() {
            assert!(allowed.contains(key), "the schema does not allow {key}");
        }
    }
}

const UNWRAP_QUERY: &str = r#"((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method "unwrap"))"#;
const DBG_QUERY: &str =
    r#"((macro_invocation macro: (identifier) @name) @match (#eq? @name "dbg"))"#;

#[test]
fn a_change_s_flags_become_annotations_on_the_lines_it_touched() {
    let temp = repository();
    let root = temp.path();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    old().unwrap();\n}\n",
    )
    .expect("source");
    rulebook::add_rule(
        root,
        "rule.no-unwrap",
        "No unwrap.",
        &json!({"type":"rule","strength":"must","enforcer":{"kind":"ast","query":UNWRAP_QUERY,"language":"rust"}}),
    );
    rulebook::add_rule(
        root,
        "rule.no-dbg",
        "No dbg!, ever.",
        &json!({"type":"rule","strength":"should","enforcer":{"kind":"ast","query":DBG_QUERY,"language":"rust"}}),
    );
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    let base = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(root)
            .output()
            .expect("rev-parse")
            .stdout,
    )
    .expect("utf-8")
    .trim()
    .to_string();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() {\n    old().unwrap();\n    new().unwrap();\n    dbg!(1);\n}\n",
    )
    .expect("change");
    git(root, &["commit", "-qam", "change"]);
    let output =
        run_with_system_tools(root, &["check", "--base", &base, "--format", "annotations"]);
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
            "::error file=src/main.rs,line=3,title=rule.no-unwrap%3A No unwrap.::new().unwrap() (wh explain rule.no-unwrap)",
            "::warning file=src/main.rs,line=4,title=rule.no-dbg%3A No dbg!%2C ever.::dbg!(1) (wh explain rule.no-dbg)",
        ],
        "the unchanged line 2 is not annotated:\n{stdout}"
    );

    // Annotations are a change's; findings and the envelope are a sweep's.
    let mixed = run_with_system_tools(root, &["check", "--sweep", "--format", "annotations"]);
    assert_eq!(mixed.status.code(), Some(2));
    let bare = run_with_system_tools(root, &["check", "--format", "findings"]);
    assert_eq!(bare.status.code(), Some(2));
}

#[test]
fn the_ci_scaffold_uses_annotations_and_passes_actionlint() {
    let template = include_str!("../assets/ci/whetstone.yml");
    assert!(template.contains("--format annotations"));
    let has_actionlint = Command::new("actionlint")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    if !has_actionlint {
        eprintln!("actionlint is not installed; skipping the lint half");
        return;
    }
    let temp = TempDir::new().expect("temp");
    let workflows = temp.path().join(".github/workflows");
    std::fs::create_dir_all(&workflows).expect("dir");
    let path = workflows.join("whetstone.yml");
    std::fs::write(&path, template.replace("__WHETSTONE_VERSION__", "0.13.0")).expect("write");
    let output = Command::new("actionlint")
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
