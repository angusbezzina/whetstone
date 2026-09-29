//! Public-surface enforcer: exported items, CLI flags and JSON schema fields,
//! compared with the last pushed revision. Any difference is the owner's call,
//! so it raises a hand instead of asking for a repair.

use std::collections::BTreeSet;
use std::path::Path;

use tree_sitter::Node;

use crate::ast::{self, AstLang};
use crate::proof::ArtifactFailure;

/// The surface kinds this enforcer compares.
pub const SURFACES: &[&str] = &["exports", "cli", "json"];

/// One file's public surface, as sorted item names.
pub fn surface_of(path: &str, text: &str, surfaces: &[String]) -> BTreeSet<String> {
    let wants = |kind: &str| surfaces.is_empty() || surfaces.iter().any(|surface| surface == kind);
    let mut items = BTreeSet::new();
    if path.ends_with(".schema.json") {
        if wants("json") {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
                collect_json_properties(&value, "", &mut items);
            }
        }
        return items;
    }
    let Some(lang) = path
        .rsplit_once('.')
        .and_then(|(_, extension)| AstLang::from_extension(extension))
    else {
        return items;
    };
    let Some(tree) = ast::parse(lang, text) else {
        return items;
    };
    let root = tree.root_node();
    if wants("exports") {
        match lang {
            AstLang::Rust => rust_exports(root, text, "", &mut items),
            AstLang::TypeScript => typescript_exports(root, text, &mut items),
            AstLang::Python => python_exports(root, text, &mut items),
        }
    }
    if wants("cli") {
        cli_flags(root, text, lang, &mut items);
    }
    items
}

fn node_text<'a>(node: Node<'_>, text: &'a str) -> &'a str {
    text.get(node.byte_range()).unwrap_or_default()
}

fn name_of(node: Node<'_>, text: &str) -> Option<String> {
    node.child_by_field_name("name")
        .map(|name| node_text(name, text).to_string())
}

fn signature(node: Node<'_>, text: &str) -> String {
    // The declaration up to its body: renaming a parameter or changing a type
    // changes the surface; editing a body does not.
    let body = node
        .child_by_field_name("body")
        .map_or(node.end_byte(), |body| body.start_byte());
    normalized(text.get(node.start_byte()..body).unwrap_or_default())
}

/// Whitespace only matters between two word characters: `f( a,b )` and
/// `f(a, b)` are the same signature, `pub fn` and `pubfn` are not.
fn normalized(text: &str) -> String {
    let word = |character: char| character.is_alphanumeric() || character == '_';
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space && out.chars().last().is_some_and(word) && word(character) {
            out.push(' ');
        }
        pending_space = false;
        out.push(character);
    }
    out
}

fn rust_exports(node: Node<'_>, text: &str, module: &str, items: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let public = child
            .children(&mut child.walk())
            .find(|part| part.kind() == "visibility_modifier")
            .is_some_and(|visibility| node_text(visibility, text) == "pub");
        match child.kind() {
            "mod_item" if public => {
                let name = name_of(child, text).unwrap_or_default();
                let path = if module.is_empty() {
                    name.clone()
                } else {
                    format!("{module}::{name}")
                };
                items.insert(format!("rust mod {path}"));
                if let Some(body) = child.child_by_field_name("body") {
                    rust_exports(body, text, &path, items);
                }
            }
            "function_item" | "struct_item" | "enum_item" | "trait_item" | "type_item"
            | "const_item" | "static_item" | "union_item"
                if public =>
            {
                items.insert(format!("rust {module} {}", signature(child, text)));
            }
            "use_declaration" if public => {
                items.insert(format!("rust {module} {}", signature(child, text)));
            }
            _ => {}
        }
    }
}

fn typescript_exports(node: Node<'_>, text: &str, items: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "export_statement" {
            items.insert(format!("ts {}", signature(child, text)));
        }
    }
}

fn python_exports(node: Node<'_>, text: &str, items: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let definition = match child.kind() {
            "decorated_definition" => child.child_by_field_name("definition"),
            "function_definition" | "class_definition" => Some(child),
            _ => None,
        };
        if let Some(definition) = definition {
            if name_of(definition, text).is_some_and(|name| !name.starts_with('_')) {
                items.insert(format!("py {}", signature(definition, text)));
            }
        }
    }
}

/// Long flags: `--name` string literals (argparse, commander, click) and clap
/// derive fields under an `#[arg(long ...)]` attribute.
fn cli_flags(root: Node<'_>, text: &str, lang: AstLang, items: &mut BTreeSet<String>) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        let kind = node.kind();
        if matches!(
            kind,
            "string_literal" | "string" | "string_content" | "string_fragment"
        ) {
            let literal = node_text(node, text).trim_matches(|c| c == '"' || c == '\'');
            if literal.starts_with("--")
                && literal.len() > 2
                && literal[2..]
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
            {
                items.insert(format!("cli {literal}"));
            }
        }
        if lang == AstLang::Rust && kind == "field_declaration" {
            let mut sibling = node.prev_sibling();
            while let Some(attribute) = sibling.filter(|node| node.kind() == "attribute_item") {
                let attribute_text = node_text(attribute, text);
                if attribute_text.contains("arg(") && attribute_text.contains("long") {
                    let explicit = attribute_text
                        .split("long = \"")
                        .nth(1)
                        .and_then(|rest| rest.split('"').next())
                        .map(str::to_owned);
                    if let Some(name) = explicit
                        .or_else(|| name_of(node, text).map(|field| field.replace('_', "-")))
                    {
                        items.insert(format!("cli --{name}"));
                    }
                }
                sibling = attribute.prev_sibling();
            }
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
}

fn collect_json_properties(value: &serde_json::Value, prefix: &str, items: &mut BTreeSet<String>) {
    let Some(object) = value.as_object() else {
        return;
    };
    if let Some(properties) = object
        .get("properties")
        .and_then(serde_json::Value::as_object)
    {
        for (name, child) in properties {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            items.insert(format!("json {path}"));
            collect_json_properties(child, &path, items);
        }
    }
    for key in ["items", "$defs", "definitions"] {
        match object.get(key) {
            Some(serde_json::Value::Object(map)) if key != "items" => {
                for (name, child) in map {
                    collect_json_properties(child, &format!("{prefix}#{name}"), items);
                }
            }
            Some(child) if key == "items" => collect_json_properties(child, prefix, items),
            _ => {}
        }
    }
}

/// Surface differences between `before` and `after` for one path.
pub fn diff(
    path: &str,
    before: &BTreeSet<String>,
    after: &BTreeSet<String>,
) -> Vec<ArtifactFailure> {
    let mut failures = Vec::new();
    for removed in before.difference(after) {
        failures.push(ArtifactFailure {
            location: path.to_string(),
            message: format!("removed or changed: {removed}"),
        });
    }
    for added in after.difference(before) {
        failures.push(ArtifactFailure {
            location: path.to_string(),
            message: format!("added or changed: {added}"),
        });
    }
    failures
}

/// Whether the enforcer reads this path at all.
pub fn is_surface_file(path: &str) -> bool {
    path.ends_with(".schema.json")
        || Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(AstLang::from_extension)
            .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_exports_change_with_signatures_not_bodies() {
        let before = surface_of(
            "src/lib.rs",
            "pub fn run(a: u8) -> u8 { a }\nfn private() {}\npub(crate) fn inner() {}\n",
            &[],
        );
        let body_only = surface_of("src/lib.rs", "pub fn run(a: u8) -> u8 { a + 1 }\n", &[]);
        let renamed = surface_of("src/lib.rs", "pub fn run(b: u8) -> u8 { b }\n", &[]);
        assert_eq!(before.len(), 1, "{before:?}");
        assert!(diff("src/lib.rs", &before, &body_only).is_empty());
        assert_eq!(diff("src/lib.rs", &before, &renamed).len(), 2);
        let reformatted = surface_of("src/lib.rs", "pub fn run( a : u8 )->u8 { a }\n", &[]);
        assert!(
            diff("src/lib.rs", &before, &reformatted).is_empty(),
            "formatting is not a surface change: {reformatted:?}"
        );
    }

    #[test]
    fn cli_flags_come_from_literals_and_clap_fields() {
        let rust = surface_of(
            "src/cli.rs",
            "struct Cli {\n    #[arg(long)]\n    dry_run: bool,\n    #[arg(long = \"host\")]\n    hosts: Vec<String>,\n}\n",
            &["cli".into()],
        );
        assert!(rust.contains("cli --dry-run"), "{rust:?}");
        assert!(rust.contains("cli --host"), "{rust:?}");
        let python = surface_of(
            "tool.py",
            "parser.add_argument('--verbose')\n",
            &["cli".into()],
        );
        assert!(python.contains("cli --verbose"), "{python:?}");
    }

    #[test]
    fn json_schema_properties_are_surface() {
        let items = surface_of(
            "references/x.schema.json",
            r#"{"properties": {"state": {"type": "string"}, "data": {"properties": {"gates": {}}}}}"#,
            &[],
        );
        assert!(items.contains("json state"));
        assert!(items.contains("json data.gates"));
    }

    #[test]
    fn typescript_and_python_exports() {
        let ts = surface_of(
            "a.ts",
            "export function f(x: number) {}\nfunction g() {}\n",
            &[],
        );
        assert_eq!(ts.len(), 1);
        let py = surface_of(
            "a.py",
            "def public(x):\n    pass\ndef _hidden():\n    pass\n",
            &[],
        );
        assert_eq!(py.len(), 1);
    }
}
