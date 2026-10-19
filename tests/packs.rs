//! Rule packs: export the accepted rules from one repository and import them
//! as drafts in another's rulebook; a changed pack is refused; the starter
//! packs and principle packs arrive the same way.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

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
    assert_eq!(pack["schema"], "whetstone.rule-pack.v1");
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
