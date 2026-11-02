//! Rule packs: export the accepted rules from one repository and import them
//! as drafts in another's rulebook; a changed pack is refused; the starter
//! packs and principle packs arrive the same way.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use whetstone::domain::ActorKind;
use whetstone::service::{ChangeRequest, CommandService, ServiceRequest};

const UNWRAP_QUERY: &str = r#"((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method \"unwrap\"))"#;

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
    fn new(files: &[(&str, &str)]) -> Self {
        let temp = tempfile::tempdir().expect("temp");
        let root = temp.path().join("project");
        std::fs::create_dir(&root).expect("project");
        git(&root, &["init", "--quiet"]);
        git(&root, &["config", "user.name", "Test Owner"]);
        git(&root, &["config", "user.email", "owner@example.invalid"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        for (path, text) in files {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().expect("parent")).expect("dir");
            std::fs::write(full, text).expect("write");
        }
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "--quiet", "-m", "base"]);
        let repo = Self { _temp: temp, root };
        repo.wh(&["change", "--kind", "mission", "--content", "Ship."]);
        repo
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut all = args.to_vec();
        all.push("--json");
        Command::new(env!("CARGO_BIN_EXE_whetstone"))
            .args(all)
            .current_dir(&self.root)
            .output()
            .expect("whetstone")
    }

    fn wh(&self, args: &[&str]) -> Value {
        let output = self.run(args);
        serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "no envelope ({error}): {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })
    }

    /// `wh change` as a person: only a person's process can consent, and a
    /// test's stdin is not a terminal, so this goes through the service.
    fn person(&self, mut request: ChangeRequest) -> Value {
        request.project_dir = self.root.clone();
        request.actor = ActorKind::Person;
        let response = CommandService.execute(ServiceRequest::Change(request));
        serde_json::to_value(response).expect("envelope")
    }

    /// Import `pack` with a person's consent.
    fn consent(&self, pack: &str, only: &[&str], maps: &[&str]) -> Value {
        self.person(ChangeRequest {
            import_pack: Some(pack.into()),
            allow_commands: true,
            only: only.iter().map(|id| id.to_string()).collect(),
            map_paths: maps.iter().map(|map| map.to_string()).collect(),
            ..ChangeRequest::default()
        })
    }

    fn write(&self, path: &str, text: &str) {
        let full = self.root.join(path);
        std::fs::create_dir_all(full.parent().expect("parent")).expect("dir");
        std::fs::write(full, text).expect("write");
    }

    fn commit(&self, message: &str) {
        git(&self.root, &["add", "-A"]);
        git(&self.root, &["commit", "--quiet", "-m", message]);
    }

    fn pack_path(&self, name: &str) -> String {
        self.root
            .parent()
            .expect("parent")
            .join(name)
            .to_str()
            .expect("path")
            .to_string()
    }

    fn rulebook(&self) -> Value {
        serde_json::from_slice(
            &std::fs::read(self.root.join("whetstone/rulebook.json")).expect("rulebook"),
        )
        .expect("rulebook JSON")
    }

    fn rule(&self, id: &str, definition: &str) -> Value {
        self.wh(&[
            "change",
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            &format!("Statement of {id}."),
            "--rationale",
            "The owner wants this.",
            "--definition",
            definition,
        ])
    }
}

#[test]
fn a_pack_round_trips_as_drafts_a_person_accepts_and_a_changed_pack_is_refused() {
    let a = Repo::new(&[("src/main.rs", "fn main() {}\n")]);
    let unwrap = format!(
        r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["src/**"]}}"#
    );
    assert_eq!(a.rule("rule.no-unwrap", &unwrap)["state"], "success");
    assert_eq!(a.wh(&["change", "--accept"])["state"], "success");
    // A draft never leaves: it is not committed.
    assert_eq!(
        a.rule(
            "rule.unaccepted",
            &format!(
                r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}}}}"#
            )
        )["state"],
        "success"
    );

    let pack_path = a.root.parent().expect("parent").join("rules.pack.json");
    let pack_arg = pack_path.to_str().expect("path");
    let before = a.rulebook();
    let exported = a.wh(&["change", "--export-pack", pack_arg]);
    assert_eq!(exported["state"], "success", "{exported}");
    assert_eq!(
        exported["data"]["rules"],
        serde_json::json!(["rule.no-unwrap"])
    );
    assert_eq!(a.rulebook(), before, "exporting changed the rulebook");
    let text = std::fs::read_to_string(&pack_path).expect("pack");
    let pack: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(pack["schema"], "whetstone.rule-pack.v2");
    assert!(
        pack["rules"][0]["rule"].get("paths").is_none(),
        "paths leave only when asked: {pack}"
    );

    let b = Repo::new(&[(
        "src/lib.rs",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )]);
    let imported = b.wh(&["change", "--import-pack", pack_arg]);
    assert_eq!(imported["state"], "success", "{imported}");
    let drafts = imported["data"]["drafts"].as_array().expect("drafts");
    assert!(
        drafts
            .iter()
            .any(|draft| draft["id"] == "rule.no-unwrap" && draft["probe"]["matched"] == 1),
        "{imported}"
    );
    // It is a draft in the file until a person accepts it, and it says where
    // it came from.
    let rule = b.rulebook()["rules"][0].clone();
    assert_eq!(rule["id"], "rule.no-unwrap");
    assert_eq!(rule["source"]["kind"], "pack");
    assert!(rule["rationale"]
        .as_str()
        .expect("rationale")
        .contains("imported from the"));
    let explained = b.wh(&["explain", "rule.no-unwrap"]);
    assert_eq!(explained["data"]["rule"]["lifecycle"], "draft");
    assert_eq!(b.wh(&["change", "--accept"])["state"], "success");
    let checked = b.run(&["check", "--rule", "rule.no-unwrap"]);
    assert_eq!(checked.status.code(), Some(1));

    // Importing again changes nothing; a changed pack is refused.
    let again = b.wh(&["change", "--import-pack", pack_arg]);
    assert_eq!(again["data"]["drafts"], serde_json::json!([]), "{again}");
    let tampered = text.replace("\"must\"", "\"should\"");
    let tampered_path = b.root.parent().expect("parent").join("tampered.pack.json");
    std::fs::write(&tampered_path, tampered).expect("tampered");
    let refused = b.run(&[
        "change",
        "--import-pack",
        tampered_path.to_str().expect("path"),
    ]);
    assert_eq!(refused.status.code(), Some(6));
    let refused: Value = serde_json::from_slice(&refused.stdout).expect("envelope");
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("does not match"),
        "{refused}"
    );
}

#[test]
fn a_starter_pack_and_a_principle_pack_arrive_by_name_and_file() {
    let repo = Repo::new(&[
        (
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/main.rs", "fn main() {}\n"),
        (".gitignore", "target/\n"),
    ]);
    let rust = repo.wh(&["change", "--import-pack", "rust"]);
    assert_eq!(rust["state"], "success", "{rust}");
    let drafted = rust["data"]["drafts"].as_array().map(Vec::len).unwrap_or(0);
    let refused = rust["data"]["not_validatable"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0);
    assert_eq!(drafted + refused, 4, "{rust}");
    assert!(
        drafted >= 3,
        "the AST and formatter rules probe here: {rust}"
    );

    let pack = r#"{"schema":"whetstone.principle-pack.v1","name":"team","version":"1.0.0","source":"handbook","license":"internal","principles":[{"id":"ship-small","group":"delivery","text":"Ship small.","enforcer_hint":"none","rationale":null}]}"#;
    let path = repo
        .root
        .parent()
        .expect("parent")
        .join("team.principles.json");
    std::fs::write(&path, pack).expect("pack");
    let installed = repo.wh(&["change", "--import-pack", path.to_str().expect("path")]);
    assert_eq!(installed["state"], "success", "{installed}");
    assert!(repo.root.join("whetstone/principles/team.json").is_file());
    let inspect = repo.wh(&["init"]);
    assert!(inspect.to_string().contains("team/ship-small"), "{inspect}");
}

#[test]
fn a_rule_names_the_principle_it_serves_and_the_owners_principles_export_as_a_pack() {
    let a = Repo::new(&[("src/main.rs", "fn main() {}\n")]);
    // One principle in the owner's words and one picked from pstack.
    let own = a.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.ship-small",
        "--content",
        "Ship the smallest change that solves the problem.",
        "--rationale",
        "Small changes review fast.",
    ]);
    assert_eq!(own["state"], "success", "{own}");
    let picked = a.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.prove-it-works",
        "--content",
        "Prove it works.",
        "--definition",
        r#"{"type":"principle","principle":"prove-it-works"}"#,
    ]);
    assert_eq!(picked["state"], "success", "{picked}");

    // A rule that serves a principle the rulebook does not hold is refused.
    let failing =
        r#"{"type":"rule","strength":"should","enforcer":{"kind":"test","command":"false"}}"#;
    let refused = a.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.small-diffs",
        "--content",
        "A change stays small.",
        "--rationale",
        "Serves ship-small.",
        "--definition",
        failing,
        "--serves",
        "no-such-principle",
    ]);
    assert_eq!(refused["state"], "needs_input", "{refused}");
    assert!(
        refused["blocking_questions"][0]
            .as_str()
            .expect("question")
            .contains("not a principle in the rulebook"),
        "{refused}"
    );
    // Named by its bare id, it is stored as the record id.
    let served = a.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.small-diffs",
        "--content",
        "A change stays small.",
        "--rationale",
        "Serves ship-small.",
        "--definition",
        failing,
        "--serves",
        "ship-small",
    ]);
    assert_eq!(served["state"], "success", "{served}");
    let rule = a.rulebook()["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["id"] == "rule.small-diffs")
        .cloned()
        .expect("the rule");
    assert_eq!(
        rule["source"],
        serde_json::json!({"kind": "principle", "reference": "principle.ship-small"})
    );
    let explained = a.wh(&["explain", "rule.small-diffs"]);
    assert_eq!(
        explained["data"]["principle"]["id"], "principle.ship-small",
        "{explained}"
    );
    // The dashboard says what the rule serves.
    let dash = a.wh(&["dash"]);
    let shown = dash["data"]["current"]["groups"]
        .as_array()
        .expect("groups")
        .iter()
        .flat_map(|group| group["rules"].as_array().cloned().unwrap_or_default())
        .find(|shown| shown["id"] == "rule.small-diffs")
        .expect("the rule on the dashboard");
    assert_eq!(shown["serves"]["id"], "principle.ship-small", "{shown}");
    // A sweep finding carries the principle, so the why travels with it.
    let sweep = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(["check", "--sweep"])
        .current_dir(&a.root)
        .output()
        .expect("whetstone");
    let findings: Value = serde_json::from_slice(&sweep.stdout).unwrap_or_else(|error| {
        panic!(
            "no findings ({error}): {}{}",
            String::from_utf8_lossy(&sweep.stdout),
            String::from_utf8_lossy(&sweep.stderr)
        )
    });
    let finding = findings
        .as_array()
        .expect("array")
        .iter()
        .find(|finding| finding["responsibility"] == "rule.small-diffs")
        .expect("the finding");
    assert_eq!(finding["principle"], "principle.ship-small", "{finding}");

    // The owner's own principles leave as a principle pack; pstack's stays
    // with pstack.
    assert_eq!(a.wh(&["change", "--accept"])["state"], "success");
    let pack_path = a.root.parent().expect("parent").join("team.json");
    let pack_arg = pack_path.to_str().expect("path");
    let before = a.rulebook();
    let exported = a.wh(&["change", "--export-pack", pack_arg, "--principles"]);
    assert_eq!(exported["state"], "success", "{exported}");
    assert_eq!(a.rulebook(), before, "exporting changed the rulebook");
    let pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&pack_path).expect("pack")).expect("json");
    assert_eq!(pack["schema"], "whetstone.principle-pack.v1");
    assert_eq!(pack["name"], "project");
    let ids = pack["principles"]
        .as_array()
        .expect("principles")
        .iter()
        .map(|principle| principle["id"].as_str().expect("id").to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec!["ship-the-smallest-change-that-solves-the-problem"]
    );
    assert_eq!(pack["principles"][0]["enforcer_hint"], "none");
    assert_eq!(
        pack["principles"][0]["rationale"],
        "Small changes review fast."
    );
    assert_eq!(
        exported["data"]["left_out"][0]["principle"], "principle.prove-it-works",
        "{exported}"
    );

    // Another repository installs it and picks from it like any pack.
    let b = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let installed = b.wh(&["change", "--import-pack", pack_arg]);
    assert_eq!(installed["state"], "success", "{installed}");
    assert!(b.root.join("whetstone/principles/project.json").is_file());
    let adopted = b.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.project.ship-small",
        "--content",
        "Ship the smallest change that solves the problem.",
        "--definition",
        r#"{"type":"principle","principle":"project/ship-the-smallest-change-that-solves-the-problem"}"#,
    ]);
    assert_eq!(adopted["state"], "success", "{adopted}");
    let record = b.rulebook()["principles"][0].clone();
    assert_eq!(record["source"]["kind"], "pack");
    assert_eq!(record["source"]["pack"], "project");
}

/// A validator, formatter, test and lint rule that each write a marker, as a
/// pack from an unknown source would.
fn marker_pack(marker: &Path) -> String {
    let source = Repo::new(&[("src/main.rs", "fn main() {}\n")]);
    let touch = marker.to_str().expect("marker");
    for (id, enforcer) in [
        (
            "rule.m-validator",
            format!(r#"{{"kind":"validator","command":"touch {touch}.validator"}}"#),
        ),
        (
            "rule.m-test",
            format!(r#"{{"kind":"test","command":"touch {touch}.test"}}"#),
        ),
        (
            "rule.m-formatter",
            format!(r#"{{"kind":"formatter","tool":"touch {touch}.formatter"}}"#),
        ),
        (
            "rule.m-lint",
            format!(r#"{{"kind":"lint","tool":"touch {touch}.lint","code":"W100"}}"#),
        ),
    ] {
        let written = source.wh(&[
            "change",
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            "Runs.",
            "--rationale",
            "A test.",
            "--allow-unvalidated",
            "--definition",
            &format!(r#"{{"type":"rule","strength":"should","enforcer":{enforcer}}}"#),
        ]);
        assert_eq!(written["state"], "success", "{written}");
    }
    let ast = format!(
        r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}}}}"#
    );
    assert_eq!(source.rule("rule.m-ast", &ast)["state"], "success");
    assert_eq!(source.wh(&["change", "--accept"])["state"], "success");
    for suffix in ["validator", "test", "formatter", "lint"] {
        let _ = std::fs::remove_file(format!("{touch}.{suffix}"));
    }
    let path = marker.with_extension("pack.json");
    let exported = source.wh(&["change", "--export-pack", path.to_str().expect("path")]);
    assert_eq!(exported["state"], "success", "{exported}");
    std::fs::read_to_string(path).expect("pack")
}

fn no_markers(marker: &Path) -> bool {
    ["validator", "test", "formatter", "lint"]
        .iter()
        .all(|suffix| !Path::new(&format!("{}.{suffix}", marker.display())).exists())
}

#[test]
fn reviewing_a_pack_runs_nothing_and_a_commands_rule_waits_for_a_person() {
    let temp = tempfile::tempdir().expect("temp");
    let marker = temp.path().join("ran");
    let text = marker_pack(&marker);
    let target = Repo::new(&[(
        "src/lib.rs",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )]);
    let pack = target.pack_path("marker.pack.json");
    std::fs::write(&pack, text).expect("pack");
    let before = std::fs::read(target.root.join("whetstone/rulebook.json")).expect("rulebook");

    // The preview names every command and starts none.
    let review = target.wh(&["change", "--import-pack", &pack, "--preview"]);
    assert_eq!(review["state"], "needs_decision", "{review}");
    assert!(no_markers(&marker), "a preview ran a pack's command");
    assert_eq!(
        std::fs::read(target.root.join("whetstone/rulebook.json")).expect("rulebook"),
        before
    );
    let entries = review["data"]["review"]["entries"]
        .as_array()
        .expect("entries");
    let entry = |id: &str| {
        entries
            .iter()
            .find(|entry| entry["id"] == id)
            .expect("entry")
    };
    assert!(entry("rule.m-validator")["target"]
        .as_str()
        .expect("target")
        .starts_with("touch "));
    assert_eq!(entry("rule.m-validator")["probe"]["status"], "not_run");
    assert_eq!(entry("rule.m-validator")["status"], "needs_consent");
    assert_eq!(
        entry("rule.m-ast")["probe"]["matched"],
        1,
        "an AST rule still reports"
    );

    // Without consent: the query is drafted, the commands are listed, and
    // --allow-unvalidated is not consent.
    for extra in [None, Some("--allow-unvalidated")] {
        let mut args = vec!["change", "--import-pack", &pack];
        args.extend(extra);
        let applied = target.wh(&args);
        assert_eq!(applied["state"], "needs_decision", "{applied}");
        assert!(no_markers(&marker), "{applied}");
        assert_eq!(
            applied["data"]["needs_consent"].as_array().map(Vec::len),
            Some(4)
        );
        let rules = target.rulebook()["rules"].clone();
        assert_eq!(rules.as_array().map(Vec::len), Some(1), "{rules}");
        assert_eq!(rules[0]["id"], "rule.m-ast");
    }

    // An agent's consent is refused and changes nothing.
    let refused = target.wh(&["change", "--import-pack", &pack, "--allow-commands"]);
    assert_eq!(refused["state"], "needs_input", "{refused}");
    assert!(refused["summary"]
        .as_str()
        .expect("summary")
        .contains("Only a person"));
    assert!(no_markers(&marker));

    // A person's consent runs them and drafts them.
    let consented = target.consent(&pack, &[], &[]);
    assert_eq!(consented["state"], "success", "{consented}");
    assert_eq!(target.rulebook()["rules"].as_array().map(Vec::len), Some(5));
    assert!(!no_markers(&marker));
}

#[test]
fn a_built_in_pack_needs_no_consent_but_its_preview_is_inert_too() {
    let repo = Repo::new(&[
        (
            "Cargo.toml",
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/main.rs", "fn main() {}\n"),
        (".gitignore", "target/\n"),
    ]);
    let preview = repo.wh(&["change", "--import-pack", "rust", "--preview"]);
    assert!(
        preview["data"]["drafts"]
            .as_array()
            .expect("drafts")
            .iter()
            .any(|draft| {
                draft["id"] == "rule.rust-tests-pass" && draft["probe"]["status"] == "not_run"
            }),
        "{preview}"
    );
    assert_eq!(repo.rulebook()["rules"], json!([]));
    assert_eq!(preview["data"]["needs_consent"], json!([]));
    let applied = repo.wh(&["change", "--import-pack", "rust"]);
    assert_eq!(applied["state"], "success", "{applied}");
    assert_eq!(applied["data"]["needs_consent"], json!([]));
}

/// A repository with principles, an AST rule, a scoped script rule with a
/// support file, and a file a pack can read.
fn source_repository() -> Repo {
    let repo = Repo::new(&[
        (
            "apps/web/src/main.rs",
            "pub fn f(x: Option<u8>) -> u8 { x.unwrap_or(0) }\n",
        ),
        ("config/allow.txt", "ok\n"),
        ("scripts/lib/shared.sh", "true\n"),
        ("docs/guide.md", "words\n"),
    ]);
    repo.write(
        "scripts/check.sh",
        "#!/bin/sh\n# reads config/allow.txt and scripts/lib/shared.sh\ntest -f config/allow.txt || exit 1\n",
    );
    git(&repo.root, &["add", "-A"]);
    Command::new("chmod")
        .args(["+x", "scripts/check.sh"])
        .current_dir(&repo.root)
        .status()
        .expect("chmod");
    repo.commit("scripts");
    for (id, statement) in [
        ("principle.ship-small", "Ship small."),
        ("principle.left-behind", "Left behind."),
    ] {
        let written = repo.wh(&[
            "change",
            "--kind",
            "principle",
            "--record-id",
            id,
            "--content",
            statement,
            "--rationale",
            "Ours.",
        ]);
        assert_eq!(written["state"], "success", "{written}");
    }
    let picked = repo.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.prove-it-works",
        "--content",
        "Prove it works.",
        "--definition",
        r#"{"type":"principle","principle":"prove-it-works"}"#,
    ]);
    assert_eq!(picked["state"], "success", "{picked}");
    let ast = format!(
        r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["apps/web/src/**"]}}"#
    );
    let written = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.no-unwrap",
        "--content",
        "No unwrap.",
        "--rationale",
        "Panics.",
        "--definition",
        &ast,
        "--serves",
        "principle.ship-small",
    ]);
    assert_eq!(written["state"], "success", "{written}");
    let script = r#"{"type":"rule","strength":"must","enforcer":{"kind":"validator","command":"sh scripts/check.sh"},"paths":["apps/web/src/**"]}"#;
    let written = repo.wh(&[
        "change",
        "--kind",
        "rule",
        "--record-id",
        "rule.allow-list",
        "--content",
        "The list exists.",
        "--rationale",
        "It must.",
        "--definition",
        script,
        "--serves",
        "principle.prove-it-works",
    ]);
    assert_eq!(written["state"], "success", "{written}");
    assert_eq!(repo.wh(&["change", "--accept"])["state"], "success");
    repo
}

#[test]
fn a_chosen_rule_brings_its_principle_and_one_revision_makes_one_pack() {
    let a = source_repository();
    let one = a.pack_path("one.json");
    let two = a.pack_path("two.json");
    let exported = a.wh(&["change", "--export-pack", &one, "--only", "rule.no-unwrap"]);
    assert_eq!(exported["state"], "success", "{exported}");
    assert_eq!(exported["data"]["rules"], json!(["rule.no-unwrap"]));
    assert_eq!(
        exported["data"]["principles"],
        json!(["principle.ship-small"])
    );
    assert_eq!(
        exported["data"]["added_by_closure"],
        json!(["principle.ship-small"])
    );
    a.wh(&["change", "--export-pack", &two, "--only", "rule.no-unwrap"]);
    assert_eq!(
        std::fs::read(&one).expect("one"),
        std::fs::read(&two).expect("two")
    );

    // A chosen principle brings only itself; such a pack has no rule.
    let alone = a.wh(&[
        "change",
        "--export-pack",
        &one,
        "--only",
        "principle.left-behind",
    ]);
    assert_eq!(alone["data"]["rules"], json!([]), "{alone}");
    assert_eq!(
        alone["data"]["principles"],
        json!(["principle.left-behind"])
    );

    // What cannot be chosen is refused by name and nothing is written.
    let nowhere = a.pack_path("nowhere.json");
    for id in ["rule.unknown", "principle.unknown"] {
        let refused = a.wh(&["change", "--export-pack", &nowhere, "--only", id]);
        assert_eq!(refused["state"], "needs_input", "{refused}");
        assert!(refused["summary"].as_str().expect("summary").contains(id));
    }
    assert!(!Path::new(&nowhere).exists());
    let retired = a.wh(&[
        "change",
        "--retire",
        "principle.left-behind",
        "--rationale",
        "Gone.",
    ]);
    assert_eq!(retired["state"], "success", "{retired}");
    assert_eq!(a.wh(&["change", "--accept"])["state"], "success");
    let refused = a.wh(&[
        "change",
        "--export-pack",
        &nowhere,
        "--only",
        "principle.left-behind",
    ]);
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("is retired"),
        "{refused}"
    );

    // A pack with no principles imports cleanly, and names and versions are checked.
    let bare = a.wh(&[
        "change",
        "--export-pack",
        &one,
        "--only",
        "rule.no-unwrap",
        "--pack-name",
        "team",
        "--pack-version",
        "2.0",
    ]);
    let text: Value =
        serde_json::from_str(&std::fs::read_to_string(&one).expect("pack")).expect("json");
    assert_eq!(
        (text["name"].as_str(), text["version"].as_str()),
        (Some("team"), Some("2.0")),
        "{bare}"
    );
    let bad = a.wh(&["change", "--export-pack", &nowhere, "--pack-name", "../x"]);
    assert_eq!(bad["state"], "needs_input", "{bad}");
}

#[test]
fn a_mission_derived_rule_stays_and_says_why() {
    let a = source_repository();
    let mut book = a.rulebook();
    let rules = book["rules"].as_array_mut().expect("rules");
    let index = rules
        .iter()
        .position(|rule| rule["id"] == "rule.no-unwrap")
        .expect("rule");
    rules[index]["source"] = json!({"kind": "mission"});
    std::fs::write(
        a.root.join("whetstone/rulebook.json"),
        serde_json::to_string_pretty(&book).expect("json"),
    )
    .expect("write");
    assert_eq!(a.wh(&["change", "--accept"])["state"], "success");
    let path = a.pack_path("m.json");
    let refused = a.wh(&["change", "--export-pack", &path, "--only", "rule.no-unwrap"]);
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("mission"),
        "{refused}"
    );
    let plan = a.wh(&["change", "--export-pack", &path, "--preview"]);
    let rule = plan["data"]["plan"]["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["id"] == "rule.no-unwrap")
        .cloned()
        .expect("rule");
    assert_eq!(rule["portability"], "stays", "{plan}");
    assert_eq!(rule["can_leave"], false);
    assert_eq!(plan["data"]["left_out"][0]["rule"], "rule.no-unwrap");
}

#[test]
fn a_rule_keeps_the_principle_it_serves_and_a_principle_its_origin() {
    let a = source_repository();
    let path = a.pack_path("all.json");
    assert_eq!(
        a.wh(&["change", "--export-pack", &path, "--include-paths"])["state"],
        "success"
    );
    let b = Repo::new(&[("apps/web/src/main.rs", "pub fn f() {}\n")]);
    b.write("whetstone/x", "");
    std::fs::remove_file(b.root.join("whetstone/x")).expect("rm");
    let imported = b.consent(&path, &[], &[]);
    assert_eq!(imported["state"], "success", "{imported}");
    let book = b.rulebook();
    let principle = |id: &str| {
        book["principles"]
            .as_array()
            .expect("principles")
            .iter()
            .find(|principle| principle["id"] == id)
            .cloned()
            .expect("principle")
    };
    assert_eq!(
        principle("principle.prove-it-works")["source"]["kind"],
        "pstack",
        "{book}"
    );
    assert_eq!(
        principle("principle.ship-small")["source"]["kind"],
        "custom"
    );
    let rule = |id: &str| {
        book["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .find(|rule| rule["id"] == id)
            .cloned()
            .expect("rule")
    };
    assert_eq!(
        rule("rule.no-unwrap")["source"],
        json!({"kind": "principle", "reference": "principle.ship-small"})
    );
    let checked = b.wh(&["explain", "principle.ship-small"]);
    assert!(checked.to_string().contains("rule.no-unwrap"), "{checked}");
    // Only the principle nothing serves is a gap.
    let dash = b.wh(&["dash"]);
    let coverage = &dash["data"]["current"]["principle_coverage"];
    assert_eq!(coverage["unanchored"], json!([]), "{coverage}");
    assert_eq!(
        coverage["unserved"].as_array().map(Vec::len),
        Some(1),
        "{coverage}"
    );
    assert_eq!(coverage["unserved"][0]["id"], "principle.left-behind");
}

#[test]
fn a_collision_on_a_principle_is_reported_and_a_missing_one_cuts_the_link() {
    let a = source_repository();
    let path = a.pack_path("all.json");
    a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--include-paths",
        "--only",
        "rule.no-unwrap",
    ]);
    let b = Repo::new(&[("apps/web/src/main.rs", "pub fn f() {}\n")]);
    b.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.ship-small",
        "--content",
        "Something else.",
        "--rationale",
        "Ours.",
    ]);
    let imported = b.wh(&["change", "--import-pack", &path]);
    assert_eq!(
        imported["data"]["principle_conflicts"][0]["rule"], "rule.no-unwrap",
        "{imported}"
    );
    assert_eq!(
        imported["data"]["principle_conflicts"][0]["here"],
        "Something else."
    );
    assert_eq!(b.rulebook()["rules"][0]["source"]["kind"], "principle");

    // A pack that lost the principle: the rule arrives from the pack and says so.
    let mut pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pack")).expect("json");
    pack["principles"] = json!([]);
    let digest = {
        use sha2::{Digest, Sha256};
        let bytes =
            serde_json::to_vec(&json!({"principles": pack["principles"], "rules": pack["rules"]}))
                .expect("json");
        format!("sha256:{:x}", Sha256::digest(bytes))
    };
    pack["digest"] = json!(digest);
    let thin = a.pack_path("thin.json");
    std::fs::write(&thin, pack.to_string()).expect("pack");
    let c = Repo::new(&[("apps/web/src/main.rs", "pub fn f() {}\n")]);
    let imported = c.wh(&["change", "--import-pack", &thin]);
    assert_eq!(
        imported["data"]["principle_missing"][0]["rule"], "rule.no-unwrap",
        "{imported}"
    );
    assert_eq!(c.rulebook()["rules"][0]["source"]["kind"], "pack");
}

#[test]
fn a_rule_passing_through_repositories_keeps_one_provenance_clause() {
    let ast = format!(
        r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}}}}"#
    );
    let files = [("src/lib.rs", "pub fn f() {}\n")];
    let first = Repo::new(&files);
    first.rule("rule.travels", &ast);
    first.wh(&["change", "--accept"]);
    let mut last_pack = first.pack_path("first.json");
    first.wh(&[
        "change",
        "--export-pack",
        &last_pack,
        "--pack-name",
        "first",
    ]);
    let mut holders = vec![first];
    for name in ["second", "third"] {
        let next = Repo::new(&files);
        let imported = next.wh(&["change", "--import-pack", &last_pack]);
        assert_eq!(imported["state"], "success", "{imported}");
        next.wh(&["change", "--accept"]);
        last_pack = next.pack_path(&format!("{name}.json"));
        next.wh(&["change", "--export-pack", &last_pack, "--pack-name", name]);
        holders.push(next);
    }
    let end = Repo::new(&files);
    end.wh(&["change", "--import-pack", &last_pack]);
    let rationale = end.rulebook()["rules"][0]["rationale"]
        .as_str()
        .expect("rationale")
        .to_string();
    assert_eq!(
        rationale.matches("imported from the").count(),
        1,
        "{rationale}"
    );
    assert!(
        rationale.starts_with("The owner wants this."),
        "{rationale}"
    );
    assert!(rationale.contains("third@"), "{rationale}");
}

#[test]
fn a_rules_paths_travel_only_when_kept_and_can_be_mapped() {
    let a = source_repository();
    let path = a.pack_path("paths.json");
    let exported = a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--keep-paths",
        "rule.no-unwrap",
        "--only",
        "rule.no-unwrap",
        "--only",
        "rule.allow-list",
    ]);
    assert_eq!(exported["state"], "success", "{exported}");
    let pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pack")).expect("json");
    let packed = |id: &str| {
        pack["rules"]
            .as_array()
            .expect("rules")
            .iter()
            .find(|rule| rule["id"] == id)
            .cloned()
            .expect("rule")
    };
    assert_eq!(
        packed("rule.no-unwrap")["rule"]["paths"],
        json!(["apps/web/src/**"])
    );
    assert!(packed("rule.no-unwrap").get("scoped_at_source").is_none());
    assert!(packed("rule.allow-list")["rule"].get("paths").is_none());
    assert_eq!(packed("rule.allow-list")["scoped_at_source"], true);
    assert!(
        !pack
            .to_string()
            .contains("\"paths\":[\"apps/web/src/**\"],\"scoped"),
        "the source's layout stays home"
    );

    let b = Repo::new(&[(
        "src/lib.rs",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n",
    )]);
    // The kept path covers nothing here: reported with the mapping that would work.
    let review = b.wh(&["change", "--import-pack", &path, "--preview"]);
    let entries = review["data"]["review"]["entries"]
        .as_array()
        .expect("entries");
    let entry = |id: &str| {
        entries
            .iter()
            .find(|entry| entry["id"] == id)
            .cloned()
            .expect("entry")
    };
    assert_eq!(
        entry("rule.no-unwrap")["status"],
        "needs_mapping",
        "{review}"
    );
    assert_eq!(
        entry("rule.no-unwrap")["suggestion"]["flag"],
        "--map-path apps/web/src=src"
    );
    assert_eq!(entry("rule.allow-list")["widened"], true, "{review}");
    let unmapped = b.wh(&["change", "--import-pack", &path]);
    assert_eq!(
        unmapped["data"]["not_validatable"][0]["id"], "rule.no-unwrap",
        "{unmapped}"
    );
    assert_eq!(
        unmapped["data"]["not_validatable"][0]["suggestion"]["to"],
        "src"
    );
    // Mapped, it probes here and finds the flag.
    let mapped = b.wh(&[
        "change",
        "--import-pack",
        &path,
        "--map-path",
        "apps/web/src=src",
    ]);
    let draft = mapped["data"]["drafts"]
        .as_array()
        .expect("drafts")
        .iter()
        .find(|draft| draft["id"] == "rule.no-unwrap")
        .cloned()
        .expect("draft");
    assert_eq!(draft["probe"]["matched"], 1, "{mapped}");
    let rule = b.rulebook()["rules"][0].clone();
    assert_eq!(rule["paths"], json!(["src/**"]));
}

#[test]
fn a_chosen_part_of_a_pack_is_imported_and_the_review_shows_the_rest() {
    let a = source_repository();
    let path = a.pack_path("all.json");
    a.wh(&["change", "--export-pack", &path]);
    let b = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let refused = b.wh(&["change", "--import-pack", &path, "--only", "rule.nowhere"]);
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("rule.nowhere"),
        "{refused}"
    );
    let imported = b.wh(&["change", "--import-pack", &path, "--only", "rule.no-unwrap"]);
    assert_eq!(imported["state"], "success", "{imported}");
    let book = b.rulebook();
    assert_eq!(book["rules"].as_array().map(Vec::len), Some(1));
    assert_eq!(book["principles"].as_array().map(Vec::len), Some(1));
    assert_eq!(book["principles"][0]["id"], "principle.ship-small");
    let entries = imported["data"]["review"]["entries"]
        .as_array()
        .expect("entries");
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["selected"] == true)
            .count(),
        2,
        "{imported}"
    );
    assert_eq!(entries.len(), 5, "the review covers the whole pack");
    // Consent still applies to a chosen command rule.
    let waiting = b.wh(&[
        "change",
        "--import-pack",
        &path,
        "--only",
        "rule.allow-list",
    ]);
    assert_eq!(
        waiting["data"]["needs_consent"][0]["id"], "rule.allow-list",
        "{waiting}"
    );
    assert!(!b.root.join("scripts/check.sh").exists());
}

#[test]
fn a_pack_carries_files_from_the_commit_and_a_person_picks_the_support_files() {
    let a = source_repository();
    let committed = std::fs::read_to_string(a.root.join("scripts/check.sh")).expect("script");
    a.write("scripts/check.sh", "#!/bin/sh\necho dirty\n");
    let path = a.pack_path("files.json");
    let preview = a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
        "--preview",
    ]);
    assert_eq!(
        preview["data"]["files"],
        json!(["scripts/check.sh"]),
        "{preview}"
    );
    assert_eq!(
        preview["data"]["suggested_files"],
        json!(["config/allow.txt", "scripts/lib/shared.sh"])
    );
    assert_eq!(preview["data"]["dirty_files"], json!(["scripts/check.sh"]));
    assert!(!Path::new(&path).exists(), "a preview writes nothing");
    let plan = preview["data"]["plan"]["rules"][0].clone();
    assert_eq!(plan["portability"], "brings_files");
    assert_eq!(plan["tools"], json!(["sh"]));

    let exported = a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
        "--with-file",
        "config/allow.txt",
    ]);
    assert_eq!(exported["state"], "success", "{exported}");
    let pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pack")).expect("json");
    let files = pack["files"].as_array().expect("files");
    assert_eq!(files.len(), 2, "{pack}");
    let script = files
        .iter()
        .find(|file| file["path"] == "scripts/check.sh")
        .expect("script");
    assert_eq!(script["text"], committed, "carried from the commit");
    assert_eq!(script["executable"], true);
    let packed = &pack["rules"][0];
    assert_eq!(
        packed["files"],
        json!(["config/allow.txt", "scripts/check.sh"])
    );

    // A changed byte of a carried file fails verification.
    let tampered = std::fs::read_to_string(&path)
        .expect("pack")
        .replace("test -f", "curl x");
    let tampered_path = a.pack_path("tampered.json");
    std::fs::write(&tampered_path, tampered).expect("tampered");
    let b = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let refused = b.wh(&["change", "--import-pack", &tampered_path, "--preview"]);
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("does not match"),
        "{refused}"
    );
}

#[test]
fn a_file_that_cannot_travel_is_refused_by_name() {
    let a = source_repository();
    a.write("scripts/local.sh", "true\n");
    let written = a.wh(&[
        "change", "--kind", "rule", "--record-id", "rule.local", "--content", "Local.", "--rationale", "Local.",
        "--definition", r#"{"type":"rule","strength":"should","enforcer":{"kind":"validator","command":"sh scripts/local.sh"}}"#,
    ]);
    assert_eq!(written["state"], "success", "{written}");
    a.commit("rule without its script tracked is staged together");
    git(&a.root, &["rm", "--cached", "--quiet", "scripts/local.sh"]);
    git(&a.root, &["commit", "--quiet", "-m", "untrack"]);
    let path = a.pack_path("x.json");
    let refused = a.wh(&["change", "--export-pack", &path, "--only", "rule.local"]);
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("scripts/local.sh"),
        "{refused}"
    );
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("not committed"),
        "{refused}"
    );
    // A whole export leaves it out rather than stopping.
    let whole = a.wh(&["change", "--export-pack", &path]);
    assert_eq!(whole["state"], "success", "{whole}");
    assert_eq!(whole["data"]["left_out"][0]["rule"], "rule.local");

    // A binary file and an oversized pack.
    a.write("scripts/local.sh", "true\n");
    std::fs::write(a.root.join("config/blob.bin"), [0_u8, 1, 2]).expect("blob");
    a.write("config/huge.txt", &"x".repeat(1_200_000));
    a.commit("more files");
    let binary = a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
        "--with-file",
        "config/blob.bin",
    ]);
    assert!(
        binary["summary"]
            .as_str()
            .expect("summary")
            .contains("binary"),
        "{binary}"
    );
    let huge = a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
        "--with-file",
        "config/huge.txt",
    ]);
    let summary = huge["summary"].as_str().expect("summary");
    assert!(
        summary.contains("1 MiB") && summary.contains("config/huge.txt"),
        "{huge}"
    );
}

fn resealed(mut pack: Value) -> String {
    use sha2::{Digest, Sha256};
    for file in pack["files"].as_array_mut().into_iter().flatten() {
        let text = file["text"].as_str().expect("text").to_string();
        file["sha256"] = json!(format!("sha256:{:x}", Sha256::digest(text.as_bytes())));
    }
    let bytes = serde_json::to_vec(&json!({
        "principles": pack["principles"], "rules": pack["rules"], "files": pack["files"],
    }))
    .expect("json");
    pack["digest"] = json!(format!("sha256:{:x}", Sha256::digest(bytes)));
    pack.to_string()
}

#[test]
fn a_hostile_path_refuses_the_pack_and_nothing_is_written() {
    let a = source_repository();
    let path = a.pack_path("base.json");
    a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
    ]);
    let pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pack")).expect("json");
    let b = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    for hostile in [
        "/tmp/evil.sh",
        "../evil.sh",
        ".git/hooks/pre-commit",
        ".GIT/hooks/pre-commit",
        ".githooks/pre-push",
        ".github/workflows/x.yml",
        "whetstone/principles/x.json",
        "whetstone/rulebook.json",
    ] {
        let mut changed = pack.clone();
        changed["files"][0]["path"] = json!(hostile);
        changed["rules"][0]["files"] = json!([hostile]);
        let file = a.pack_path("hostile.json");
        std::fs::write(&file, resealed(changed)).expect("pack");
        let refused = b.consent(&file, &[], &[]);
        assert_eq!(refused["state"], "needs_input", "{hostile}: {refused}");
        assert!(
            refused["summary"]
                .as_str()
                .expect("summary")
                .contains("refused"),
            "{refused}"
        );
    }
    assert_eq!(b.rulebook()["rules"], json!([]));
    // A path through a symbolic link out of the repository.
    let outside = tempfile::tempdir().expect("outside");
    std::os::unix::fs::symlink(outside.path(), b.root.join("scripts")).expect("link");
    let refused = b.consent(&path, &[], &[]);
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("symbolic link"),
        "{refused}"
    );
    assert_eq!(std::fs::read_dir(outside.path()).expect("dir").count(), 0);
}

#[test]
fn an_import_never_overwrites_a_file_that_differs() {
    let a = source_repository();
    let path = a.pack_path("all.json");
    a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--with-file",
        "config/allow.txt",
    ]);
    let b = Repo::new(&[
        ("src/lib.rs", "pub fn f() {}\n"),
        ("scripts/check.sh", "mine\n"),
    ]);
    let imported = b.consent(&path, &[], &[]);
    assert_eq!(
        imported["data"]["file_conflicts"][0]["id"], "rule.allow-list",
        "{imported}"
    );
    assert_eq!(
        std::fs::read_to_string(b.root.join("scripts/check.sh")).expect("file"),
        "mine\n"
    );
    assert!(
        !b.root.join("config/allow.txt").exists(),
        "its other file stays out too"
    );
    let ids = b.rulebook()["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| rule["id"].as_str().expect("id").to_string())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["rule.no-unwrap"]);

    // An identical file counts as present, and the executable bit arrives.
    let c = Repo::new(&[
        ("src/lib.rs", "pub fn f() {}\n"),
        ("config/allow.txt", "ok\n"),
    ]);
    let imported = c.consent(&path, &[], &[]);
    assert_eq!(
        imported["data"]["files_written"],
        json!(["scripts/check.sh"]),
        "{imported}"
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(c.root.join("scripts/check.sh"))
        .expect("meta")
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0o111, "the executable bit is restored");
}

#[test]
fn a_failed_import_leaves_nothing_written() {
    let a = source_repository();
    let path = a.pack_path("one.json");
    a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
    ]);
    // Two rules cannot share an id with a principle held here, so the
    // rulebook refuses the draft after the file has been written.
    let b = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let mut book = b.rulebook();
    book["require_served"] = json!(false);
    b.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.other",
        "--content",
        "Other.",
        "--rationale",
        "Ours.",
    ]);
    let mut pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pack")).expect("json");
    pack["rules"][0]["id"] = json!("principle.prove-it-works");
    let file = a.pack_path("clash.json");
    std::fs::write(&file, resealed(pack)).expect("pack");
    let refused = b.consent(&file, &[], &[]);
    assert_ne!(refused["state"], "success", "{refused}");
    assert!(!b.root.join("scripts").exists(), "{refused}");
}

#[test]
fn accepting_waits_for_the_files_a_rule_runs_and_withdrawing_leaves_them() {
    let a = source_repository();
    let path = a.pack_path("all.json");
    a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.allow-list",
        "--with-file",
        "config/allow.txt",
    ]);
    let b = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    b.wh(&["change", "--accept"]);
    let imported = b.consent(&path, &[], &[]);
    assert_eq!(imported["state"], "success", "{imported}");
    let head = || {
        let output = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&b.root)
            .output()
            .expect("git");
        String::from_utf8_lossy(&output.stdout).to_string()
    };
    let before = head();
    let refused = b.wh(&["change", "--accept"]);
    assert_eq!(refused["state"], "needs_input", "{refused}");
    assert_eq!(head(), before, "no commit");
    assert_eq!(
        refused["data"]["uncommitted_files"],
        json!(["config/allow.txt", "scripts/check.sh"])
    );
    assert_eq!(
        refused["data"]["commands"][0],
        "git add config/allow.txt scripts/check.sh && git commit -m \"Add the files the rules run\""
    );

    // Withdrawing restores the rulebook, deletes nothing and says what stayed.
    let withdrawn = b.wh(&["change", "--withdraw"]);
    assert_eq!(
        withdrawn["data"]["left_behind"],
        json!(["config/allow.txt", "scripts/check.sh"]),
        "{withdrawn}"
    );
    assert!(b.root.join("scripts/check.sh").exists());

    let again = b.consent(&path, &[], &[]);
    assert_eq!(again["state"], "success", "{again}");
    git(&b.root, &["add", "config", "scripts"]);
    git(
        &b.root,
        &["commit", "--quiet", "-m", "Add the files the rules run"],
    );
    let accepted = b.wh(&["change", "--accept"]);
    assert_eq!(accepted["state"], "success", "{accepted}");
    let checked = b.run(&["check", "--rule", "rule.allow-list"]);
    assert_eq!(
        checked.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&checked.stdout)
    );

    // A rule accepted long ago with an untracked script blocks nothing else.
    let c = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    c.write("scripts/old.sh", "true\n");
    let old = c.wh(&[
        "change", "--kind", "rule", "--record-id", "rule.old", "--content", "Old.", "--rationale", "Old.",
        "--definition", r#"{"type":"rule","strength":"should","enforcer":{"kind":"validator","command":"sh scripts/old.sh"}}"#,
    ]);
    assert_eq!(old["state"], "success", "{old}");
    git(&c.root, &["add", "whetstone/rulebook.json"]);
    git(
        &c.root,
        &["commit", "--quiet", "-m", "the rule, without its script"],
    );
    c.wh(&[
        "change",
        "--kind",
        "principle",
        "--record-id",
        "principle.unrelated",
        "--content",
        "Unrelated.",
        "--rationale",
        "Ours.",
    ]);
    let unrelated = c.wh(&["change", "--accept"]);
    assert_eq!(unrelated["state"], "success", "{unrelated}");
}

#[test]
fn a_must_rule_that_fails_with_no_location_is_named_and_a_missing_file_refused() {
    let source = Repo::new(&[
        ("src/lib.rs", "pub fn f() {}\n"),
        ("scripts/real.sh", "exit 1\n"),
    ]);
    for (id, command) in [
        ("rule.fails-now", "sh scripts/real.sh"),
        ("rule.no-script", "node scripts/gone.mjs"),
    ] {
        let definition = format!(
            r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"validator","command":"{command}"}}}}"#
        );
        source.wh(&[
            "change",
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            "Runs.",
            "--rationale",
            "A test.",
            "--allow-unvalidated",
            "--definition",
            &definition,
        ]);
    }
    assert_eq!(source.wh(&["change", "--accept"])["state"], "success");
    let path = source.pack_path("f.json");
    assert_eq!(
        source.wh(&["change", "--export-pack", &path])["state"],
        "success"
    );
    let target = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let review = target.wh(&["change", "--import-pack", &path, "--preview"]);
    let entries = review["data"]["review"]["entries"]
        .as_array()
        .expect("entries");
    let gone = entries
        .iter()
        .find(|entry| entry["id"] == "rule.no-script")
        .expect("entry");
    assert_eq!(gone["status"], "not_validatable", "{review}");
    assert!(gone["reason"]
        .as_str()
        .expect("reason")
        .contains("scripts/gone.mjs"));
    let applied = target.consent(&path, &[], &[]);
    assert_eq!(
        applied["data"]["not_validatable"][0]["id"], "rule.no-script",
        "{applied}"
    );
    assert_eq!(
        applied["data"]["fails_today"][0]["id"], "rule.fails-now",
        "{applied}"
    );
    assert_eq!(applied["data"]["fails_today"][0]["strength"], "must");
}

#[test]
fn a_version_one_pack_still_imports() {
    let source = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let ast = format!(
        r#"{{"type":"rule","strength":"should","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}}}}"#
    );
    source.rule("rule.old-pack", &ast);
    source.wh(&["change", "--accept"]);
    let path = source.pack_path("v1.json");
    source.wh(&["change", "--export-pack", &path]);
    let text = std::fs::read_to_string(&path)
        .expect("pack")
        .replace("whetstone.rule-pack.v2", "whetstone.rule-pack.v1");
    std::fs::write(&path, text).expect("pack");
    let target = Repo::new(&[("src/lib.rs", "pub fn f() {}\n")]);
    let imported = target.wh(&["change", "--import-pack", &path]);
    assert_eq!(imported["state"], "success", "{imported}");
}

/// Every key `value` holds is one the schema object lists, and every key it
/// requires is there.
fn keys_match(schema: &Value, value: &Value, what: &str) {
    let object = value.as_object().expect("an object");
    let properties = schema["properties"].as_object().expect("properties");
    for key in object.keys() {
        assert!(
            properties.contains_key(key),
            "{what}: {key} is not in the schema"
        );
    }
    for key in schema["required"].as_array().expect("required") {
        assert!(
            object.contains_key(key.as_str().expect("key")),
            "{what}: {key} is missing"
        );
    }
}

#[test]
fn an_exported_pack_has_the_shape_the_schema_publishes() {
    let schema: Value =
        serde_json::from_str(include_str!("../references/rule-pack-v2.schema.json"))
            .expect("schema");
    let a = source_repository();
    let path = a.pack_path("s.json");
    a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--with-file",
        "config/allow.txt",
        "--keep-paths",
        "rule.no-unwrap",
    ]);
    let pack: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("pack")).expect("json");
    keys_match(&schema, &pack, "pack");
    let shapes = [
        ("principles", "principle"),
        ("rules", "packedRule"),
        ("files", "file"),
    ];
    for (list, definition) in shapes {
        let items = pack[list].as_array().expect("list");
        assert!(!items.is_empty(), "{list}");
        for item in items {
            keys_match(&schema["$defs"][definition], item, list);
        }
    }
}

/// The whole path once, with the CLI where a person would use it: a subset
/// with its principle, a kept path and a support file leaves one repository,
/// and in the next it reads inert, needs consent and a mapping, waits for its
/// script to be committed, passes, and fails the right rule on a violation.
#[test]
fn a_chosen_subset_travels_and_holds_in_another_repository() {
    let a = source_repository();
    let path = a.pack_path("subset.json");
    let exported = a.wh(&[
        "change",
        "--export-pack",
        &path,
        "--only",
        "rule.no-unwrap",
        "--only",
        "rule.allow-list",
        "--keep-paths",
        "rule.no-unwrap",
        "--with-file",
        "config/allow.txt",
    ]);
    assert_eq!(exported["state"], "success", "{exported}");
    assert_eq!(
        exported["data"]["added_by_closure"],
        json!(["principle.prove-it-works", "principle.ship-small"])
    );

    let b = Repo::new(&[(
        "src/lib.rs",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap_or(0) }\n",
    )]);
    assert_eq!(b.wh(&["change", "--accept"])["state"], "success");
    let review = b.wh(&["change", "--import-pack", &path, "--preview"]);
    assert_eq!(review["data"]["written"], false, "{review}");
    assert!(!b.root.join("scripts").exists(), "the review wrote a file");

    let imported = b.consent(&path, &[], &["apps/web/src=src"]);
    assert_eq!(imported["state"], "success", "{imported}");
    assert_eq!(
        imported["data"]["files_written"],
        json!(["config/allow.txt", "scripts/check.sh"])
    );
    assert_eq!(b.wh(&["change", "--accept"])["state"], "needs_input");
    git(&b.root, &["add", "config", "scripts"]);
    git(
        &b.root,
        &["commit", "--quiet", "-m", "Add the files the rules run"],
    );
    assert_eq!(b.wh(&["change", "--accept"])["state"], "success");

    let dash = b.wh(&["dash"]);
    let coverage = &dash["data"]["current"]["principle_coverage"];
    assert_eq!(
        (&coverage["unserved"], &coverage["unanchored"]),
        (&json!([]), &json!([])),
        "{coverage}"
    );
    let clean = b.run(&["check"]);
    assert_eq!(
        clean.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&clean.stdout)
    );

    b.write(
        "src/lib.rs",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n",
    );
    let seeded = b.run(&["check"]);
    assert_eq!(seeded.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&seeded.stdout).contains("rule.no-unwrap"));
}
