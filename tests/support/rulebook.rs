//! A rule written straight into `whetstone/rulebook.json`, the file
//! `wh check` reads: no record store and no `bd`.

use std::path::Path;

use serde_json::{json, Value};

/// Add a rule to the project's rulebook, creating the file when there is
/// none. The definition is what `wh change --definition` takes (its `type`
/// is dropped); a rationale and the owner as source are filled in.
pub fn add_rule(root: &Path, id: &str, statement: &str, definition: &Value) {
    let path = root.join("whetstone/rulebook.json");
    let mut rulebook = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Value>(&bytes).expect("rulebook JSON"),
        Err(_) => json!({
            "schema": "whetstone.rulebook.v1",
            "mission": {"statement": "Nothing broken reaches the team."},
            "principles": [],
            "rules": [],
        }),
    };
    let mut rule = definition.clone();
    let fields = rule.as_object_mut().expect("a definition is an object");
    fields.remove("type");
    fields.insert("id".into(), json!(id));
    fields.insert("statement".into(), json!(statement));
    fields
        .entry("rationale")
        .or_insert_with(|| json!("The team relies on it."));
    fields.insert("source".into(), json!({"kind": "owner"}));
    rulebook["rules"].as_array_mut().expect("rules").push(rule);
    std::fs::create_dir_all(root.join("whetstone")).expect("whetstone dir");
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&rulebook).expect("encode rulebook"),
    )
    .expect("write rulebook");
}
