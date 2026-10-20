//! A rule's scope and reach in the file: `exclude` takes paths out of every
//! mode (generated code by default), `mode: change` runs a rule only against
//! a base and has a sweep count it, and `batch` groups a sweep's flags into
//! one finding per file or crate. Every repository is a throwaway.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;

#[path = "support/rulebook.rs"]
mod rulebook;

const DBG_QUERY: &str =
    r#"((macro_invocation macro: (identifier) @name) @match (#eq? @name "dbg"))"#;

fn git(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn wh(root: &Path, args: &[&str]) -> Output {
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

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(path, text).expect("write");
}

/// A repository with a dbg! in a crate's source and in its generated code.
fn repository() -> TempDir {
    let temp = TempDir::new().expect("temp");
    let root = temp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.email", "owner@example.com"]);
    git(root, &["config", "user.name", "Owner"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(
        root,
        &["remote", "add", "origin", "git@github.com:example/app.git"],
    );
    write(root, "crates/api/Cargo.toml", "[package]\nname = \"api\"\n");
    write(root, "crates/api/src/lib.rs", "fn a() { dbg!(1); }\n");
    write(
        root,
        "crates/api/src/generated/client.rs",
        "fn b() { dbg!(2); }\n",
    );
    temp
}

fn dbg_rule(strength: &str) -> Value {
    json!({
        "type": "rule",
        "strength": strength,
        "enforcer": {"kind": "ast", "language": "rust", "query": DBG_QUERY},
        "paths": ["crates/**"],
    })
}

/// Every place a check's gate for `rule` flagged.
fn flagged(response: &Value, rule: &str) -> Vec<String> {
    response["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .filter(|gate| gate["id"] == rule)
        .flat_map(|gate| gate["failures"].as_array().cloned().unwrap_or_default())
        .map(|failure| failure["location"].as_str().expect("location").to_string())
        .collect()
}

#[test]
fn excluded_paths_are_out_of_scope_in_every_mode_and_generated_code_by_default() {
    let temp = repository();
    let root = temp.path();
    let mut excluding = dbg_rule("must");
    excluding["exclude"] = json!(["**/generated/**"]);
    rulebook::add_rule(root, "rule.excluding", "No dbg!.", &excluding);
    rulebook::add_rule(root, "rule.defaulted", "No dbg!.", &dbg_rule("must"));
    let mut wanting = dbg_rule("should");
    wanting["exclude"] = json!([]);
    rulebook::add_rule(root, "rule.wanting", "No dbg!, generated too.", &wanting);
    git(root, &["add", "."]);

    // Staged: the generated file is in the index but out of scope.
    let staged = envelope(&wh(root, &["check", "--staged", "--json"]));
    for rule in ["rule.excluding", "rule.defaulted"] {
        assert_eq!(
            flagged(&staged, rule),
            ["crates/api/src/lib.rs:1"],
            "{rule}: {staged}"
        );
    }
    assert_eq!(
        flagged(&staged, "rule.wanting"),
        [
            "crates/api/src/generated/client.rs:1",
            "crates/api/src/lib.rs:1"
        ]
    );
    git(root, &["commit", "-qm", "start"]);
    let base = git(root, &["rev-parse", "HEAD"]);

    // The whole tree, and a base check over a change that touches both.
    let tree = envelope(&wh(root, &["check", "--json"]));
    assert_eq!(
        flagged(&tree, "rule.excluding"),
        ["crates/api/src/lib.rs:1"]
    );
    assert_eq!(
        flagged(&tree, "rule.defaulted"),
        ["crates/api/src/lib.rs:1"]
    );
    write(
        root,
        "crates/api/src/lib.rs",
        "fn a() { dbg!(1); dbg!(3); }\n",
    );
    write(
        root,
        "crates/api/src/generated/client.rs",
        "fn b() { dbg!(2); dbg!(4); }\n",
    );
    git(root, &["commit", "-qam", "change"]);
    let changed = envelope(&wh(root, &["check", "--ci", "--base", &base, "--json"]));
    assert_eq!(
        flagged(&changed, "rule.excluding"),
        ["crates/api/src/lib.rs:1", "crates/api/src/lib.rs:1"],
        "{changed}"
    );

    // A change that touches only generated code cannot break an excluding
    // rule.
    let before = git(root, &["rev-parse", "HEAD"]);
    write(
        root,
        "crates/api/src/generated/client.rs",
        "fn b() { dbg!(5); }\n",
    );
    git(root, &["commit", "-qam", "regenerate"]);
    let regenerated = envelope(&wh(root, &["check", "--ci", "--base", &before, "--json"]));
    for rule in ["rule.excluding", "rule.defaulted"] {
        assert!(flagged(&regenerated, rule).is_empty(), "{regenerated}");
    }
    assert_eq!(regenerated["state"], "success", "{regenerated}");

    // The sweep files only what is in scope.
    let sweep = envelope(&wh(root, &["check", "--sweep", "--format", "json"]));
    let titles = sweep["data"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .map(|finding| finding["title"].as_str().expect("title").to_string())
        .collect::<Vec<_>>();
    assert!(
        titles
            .iter()
            .any(|title| title.contains("crates/api/src/generated/client.rs")),
        "the rule that wants generated code files there: {titles:?}"
    );

    // wh explain names the exclusion and whether it is the default.
    let explained = envelope(&wh(root, &["explain", "rule.defaulted", "--json"]));
    assert_eq!(
        explained["data"]["rule"]["exclude"],
        json!(["**/generated/**"])
    );
    let text =
        String::from_utf8_lossy(&wh(root, &["explain", "rule.defaulted"]).stdout).to_string();
    assert!(
        text.contains("Excludes: **/generated/** (the default)"),
        "{text}"
    );
    let text = String::from_utf8_lossy(&wh(root, &["explain", "rule.wanting"]).stdout).to_string();
    assert!(text.contains("Excludes: nothing"), "{text}");
}

#[test]
fn a_change_mode_rule_runs_against_a_base_and_a_sweep_only_counts_it() {
    let temp = repository();
    let root = temp.path();
    let mut change = dbg_rule("should");
    change["mode"] = json!("change");
    rulebook::add_rule(root, "rule.change", "No new dbg!.", &change);
    rulebook::add_rule(root, "rule.tree", "No dbg! anywhere.", &dbg_rule("must"));
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    write(
        root,
        "crates/api/src/lib.rs",
        "fn a() { dbg!(1); }\nfn c() { dbg!(3); }\n",
    );
    git(root, &["commit", "-qam", "change"]);

    let against_base = envelope(&wh(root, &["check", "--ci", "--base", &base, "--json"]));
    assert_eq!(
        flagged(&against_base, "rule.change"),
        ["crates/api/src/lib.rs:1", "crates/api/src/lib.rs:2"],
        "{against_base}"
    );
    assert!(against_base["data"]["flags"]
        .as_array()
        .expect("flags")
        .contains(&json!("rule.change")));

    // A plain check, pre-commit and a check without a base leave it out.
    for args in [&["check", "--json"][..], &["check", "--staged", "--json"]] {
        let response = envelope(&wh(root, args));
        assert!(flagged(&response, "rule.change").is_empty(), "{response}");
        assert_eq!(
            response["data"]["selection"]["change_only"],
            json!(["rule.change"]),
            "{args:?}: {response}"
        );
    }

    // The sweep runs it to count its flags and files nothing for it.
    let swept = wh(root, &["check", "--sweep"]);
    assert_eq!(swept.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&swept.stdout), "[]\n");
    let sweep = envelope(&wh(root, &["check", "--sweep", "--format", "json"]));
    assert_eq!(
        sweep["data"]["counts"],
        json!([{"rule": "rule.change", "state": "fail", "count": 2}]),
        "{sweep}"
    );
    assert!(sweep["summary"]
        .as_str()
        .is_some_and(|summary| summary.contains("rule.change 2")));

    // wh explain says the mode.
    let explained = envelope(&wh(root, &["explain", "rule.change", "--json"]));
    assert_eq!(explained["data"]["rule"]["mode"], "change");
    assert_eq!(
        explained["data"]["rule"]["runs_at"],
        "pre-push and CI, on the change since the base"
    );
}

#[test]
fn a_batched_rule_files_one_finding_per_crate_with_its_sites() {
    let temp = repository();
    let root = temp.path();
    write(root, "crates/web/package.json", "{\"name\": \"web\"}\n");
    let calls = |count: usize| {
        (1..=count)
            .map(|index| format!("fn f{index}() {{ dbg!({index}); }}\n"))
            .collect::<String>()
    };
    write(root, "crates/api/src/lib.rs", &calls(4));
    write(root, "crates/api/src/more.rs", &calls(2));
    write(root, "crates/web/src/a.rs", &calls(4));
    let mut batched = dbg_rule("should");
    batched["batch"] = json!("by-crate");
    rulebook::add_rule(root, "rule.batched", "No dbg!.", &batched);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "start"]);

    let sweep = envelope(&wh(root, &["check", "--sweep", "--format", "json"]));
    let findings = sweep["data"]["findings"].as_array().expect("findings");
    assert_eq!(findings.len(), 2, "{sweep}");
    let site_count = |finding: &Value| finding["sites"].as_array().expect("sites").len();
    let api = findings
        .iter()
        .find(|finding| {
            finding["title"]
                .as_str()
                .is_some_and(|title| title.contains("crates/api"))
        })
        .expect("api batch");
    let web = findings
        .iter()
        .find(|finding| {
            finding["title"]
                .as_str()
                .is_some_and(|title| title.contains("crates/web"))
        })
        .expect("web batch");
    assert_eq!(site_count(api) + site_count(web), 10);
    assert_eq!(site_count(api), 6);
    assert_eq!(
        api["sites"][0],
        json!({"path": "crates/api/src/lib.rs", "line": 1, "text": "fn f1() { dbg!(1); }"})
    );
    assert_eq!(api["title"], "No dbg!. (6 site(s) in crates/api)");

    // The fingerprint is the rule and the batch; the digest moves when a
    // site is fixed, and the fingerprint holds.
    let before = api.clone();
    write(root, "crates/api/src/more.rs", &calls(1));
    git(root, &["commit", "-qam", "fix one"]);
    let after = envelope(&wh(root, &["check", "--sweep", "--format", "json"]));
    let api_after = after["data"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| finding["fingerprint"] == before["fingerprint"])
        .expect("same fingerprint")
        .clone();
    assert_ne!(api_after["subjectDigest"], before["subjectDigest"]);
    assert_eq!(site_count(&api_after), 5);

    // An edit elsewhere that only moves the lines keeps the digest.
    write(
        root,
        "crates/api/src/lib.rs",
        &format!("// moved\n{}", calls(4)),
    );
    git(root, &["commit", "-qam", "move"]);
    let moved = envelope(&wh(root, &["check", "--sweep", "--format", "json"]));
    let api_moved = moved["data"]["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .find(|finding| finding["fingerprint"] == before["fingerprint"])
        .expect("same fingerprint")
        .clone();
    assert_eq!(api_moved["subjectDigest"], api_after["subjectDigest"]);

    // wh explain resolves a batch's fingerprint without a findings file.
    let fingerprint = before["fingerprint"].as_str().expect("fingerprint");
    let explained = envelope(&wh(root, &["explain", fingerprint, "--json"]));
    assert_eq!(explained["state"], "success", "{explained}");
    assert_eq!(
        explained["data"]["resolved"]["subject"],
        "by-crate:crates/api"
    );
    assert_eq!(explained["data"]["rule"]["sweep"]["batch"], "by-crate");

    // The findings keep to the schema: `sites` is the only addition.
    let schema: Value = serde_json::from_str(include_str!("../references/findings-v1.schema.json"))
        .expect("schema");
    let allowed = schema["items"]["properties"]
        .as_object()
        .expect("properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let site_fields = schema["items"]["properties"]["sites"]["items"]["properties"]
        .as_object()
        .expect("site properties")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for finding in findings {
        for key in finding.as_object().expect("object").keys() {
            assert!(allowed.contains(key), "the schema does not allow {key}");
        }
        for site in finding["sites"].as_array().expect("sites") {
            for key in site.as_object().expect("site").keys() {
                assert!(site_fields.contains(key), "the schema does not allow {key}");
            }
        }
    }

    // by-file batches by the file.
    let book_path = root.join("whetstone/rulebook.json");
    let mut book: Value =
        serde_json::from_slice(&std::fs::read(&book_path).expect("read")).expect("json");
    book["rules"][0]["batch"] = json!("by-file");
    std::fs::write(&book_path, serde_json::to_vec_pretty(&book).expect("json")).expect("write");
    let by_file = envelope(&wh(root, &["check", "--sweep", "--format", "json"]));
    assert_eq!(
        by_file["data"]["findings"]
            .as_array()
            .expect("findings")
            .len(),
        3,
        "{by_file}"
    );
}
