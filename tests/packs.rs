//! Rule packs: export the accepted rules from one repository and import them
//! as drafts in another; a changed pack is refused; the starter packs and
//! principle packs arrive the same way.

mod support;

use support::Repo;

const UNWRAP_QUERY: &str = r#"((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method \"unwrap\"))"#;

#[test]
fn a_pack_round_trips_as_drafts_a_person_accepts_and_a_changed_pack_is_refused() {
    let a = Repo::new();
    a.write("src/main.rs", "fn main() {}\n");
    a.commit("base");
    let agreed = a.agree();
    assert_eq!(
        agreed["data"]["progress"]["agreement_complete"], true,
        "{agreed}"
    );
    a.rule(
        "rule.no-unwrap",
        &format!(r#"{{"type":"rule","strength":"must","enforcer":{{"kind":"ast","query":"{UNWRAP_QUERY}","language":"rust"}},"paths":["src/**"]}}"#),
    );
    // A draft never leaves: it was not accepted.
    let draft = a.propose(
        "draft-only",
        &[
            "--kind",
            "rule",
            "--record-id",
            "rule.unaccepted",
            "--content",
            "Not yet.",
            "--rationale",
            "Still deciding.",
            "--definition",
            r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"true"}}"#,
        ],
    );
    assert_eq!(draft["state"], "success", "{draft}");

    let pack_path = a.root.join("..").join("rules.pack.json");
    let pack_arg = pack_path.to_str().expect("path");
    let exported = a.wh_json(&["push", "--pack", pack_arg]);
    assert_eq!(exported["state"], "success", "{exported}");
    let rules = exported["data"]["rules"].as_array().expect("rules");
    assert!(
        rules.iter().any(|rule| rule == "rule.no-unwrap"),
        "{exported}"
    );
    assert!(
        !rules.iter().any(|rule| rule == "rule.unaccepted"),
        "{exported}"
    );
    let text = std::fs::read_to_string(&pack_path).expect("pack");
    let pack: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(pack["schema"], "whetstone.rule-pack.v1");
    let unwrap = pack["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .find(|rule| rule["id"] == "rule.no-unwrap")
        .expect("unwrap");
    assert!(
        unwrap["rule"].get("paths").is_none(),
        "paths leave only when flagged: {unwrap}"
    );
    assert!(!text.contains("decision"), "no decision log in a pack");

    let b = Repo::new();
    b.write(
        "src/lib.rs",
        "pub fn f(x: Option<u8>) -> u8 { x.unwrap() }\n",
    );
    b.commit("base");
    let imported = b.wh_json(&["pull", "--pack", pack_arg]);
    assert_eq!(imported["state"], "success", "{imported}");
    let drafts = imported["data"]["drafts"].as_array().expect("drafts");
    let unwrap = drafts
        .iter()
        .find(|draft| draft["id"] == "rule.no-unwrap")
        .unwrap_or_else(|| panic!("no unwrap draft in {imported}"));
    // Nothing is in force until a person accepts it.
    let dry = b.wh_json(&["check", "--dry-run"]);
    assert_eq!(
        dry["data"]["selection"]["rules"],
        serde_json::json!([]),
        "{dry}"
    );
    let accepted = b.accept(unwrap["proposal"]["id"].as_str().expect("proposal"));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let checked = b.wh_json(&["check", "--rule", "rule.no-unwrap"]);
    let gate = checked["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == "rule.no-unwrap")
        .expect("gate");
    assert_eq!(gate["state"], "fail", "{checked}");
    // The rule says where it came from.
    let explained = b.wh_json(&["explain", "rule.no-unwrap"]);
    assert_eq!(explained["data"]["rule"]["source"]["kind"], "pack");
    assert!(explained["data"]["rule"]["rationale"]
        .as_str()
        .expect("rationale")
        .contains("imported from the"));

    // Importing again changes nothing; a changed pack is refused.
    let again = b.wh_json(&["pull", "--pack", pack_arg]);
    assert_eq!(again["data"]["drafts"], serde_json::json!([]), "{again}");
    let tampered = text.replace("\"must\"", "\"should\"");
    let tampered_path = b.root.join("..").join("tampered.pack.json");
    std::fs::write(&tampered_path, tampered).expect("tampered");
    let refused = b.wh(&["pull", "--pack", tampered_path.to_str().expect("path")]);
    assert_eq!(refused.status.code(), Some(6));
    let refused = support::json(&refused);
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
    let repo = Repo::new();
    repo.write(
        "Cargo.toml",
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
    );
    repo.write("src/main.rs", "fn main() {}\n");
    repo.commit("base");
    let rust = repo.wh_json(&["pull", "--pack", "rust"]);
    assert_eq!(rust["state"], "success", "{rust}");
    assert_eq!(
        rust["data"]["drafts"].as_array().map(Vec::len),
        Some(4),
        "{rust}"
    );

    let pack = r#"{"schema":"whetstone.principle-pack.v1","name":"team","version":"1.0.0","source":"handbook","license":"internal","principles":[{"id":"ship-small","group":"delivery","text":"Ship small.","enforcer_hint":"none","rationale":null}]}"#;
    let path = repo.root.join("..").join("team.principles.json");
    std::fs::write(&path, pack).expect("pack");
    let installed = repo.wh_json(&["pull", "--pack", path.to_str().expect("path")]);
    assert_eq!(installed["state"], "success", "{installed}");
    assert!(repo.root.join("whetstone/principles/team.json").is_file());
    let inspect = repo.wh_json(&["init"]);
    assert!(inspect.to_string().contains("team/ship-small"), "{inspect}");
}
