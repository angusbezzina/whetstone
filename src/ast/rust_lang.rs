//! Rust tree-sitter query helpers.
//!
//! Named `rust_lang` to avoid colliding with the standard `rust` prelude name.

use tree_sitter::Tree;

use super::{run_query, AstLang, AstMatch};

const FUNCTION_QUERY: &str = r#"
(function_item) @match
"#;

const TYPE_QUERY: &str = r#"
[
  (struct_item) @match
  (enum_item) @match
  (trait_item) @match
]
"#;

const IMPL_QUERY: &str = r#"
(impl_item) @match
"#;

const USE_QUERY: &str = r#"
(use_declaration) @match
"#;

const ATTRIBUTE_QUERY: &str = r#"
(attribute_item) @match
"#;

pub fn functions(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Rust, tree, source, FUNCTION_QUERY, "function")
}

pub fn types(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Rust, tree, source, TYPE_QUERY, "type")
}

pub fn impls(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Rust, tree, source, IMPL_QUERY, "impl")
}

pub fn uses(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Rust, tree, source, USE_QUERY, "use")
}

pub fn attributes(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Rust, tree, source, ATTRIBUTE_QUERY, "attribute")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse, AstLang};

    const SAMPLE: &str = r#"
use std::fs;

#[derive(Debug)]
pub struct User { name: String }

pub enum Role { Admin, Guest }

pub fn greet(u: &User) -> String { format!("hi {}", u.name) }

impl User {
    fn new(name: String) -> Self { Self { name } }
}
"#;

    fn tree() -> tree_sitter::Tree {
        parse(AstLang::Rust, SAMPLE).expect("parse rust")
    }

    #[test]
    fn functions_finds_free_and_method_fn() {
        let names: Vec<_> = functions(&tree(), SAMPLE)
            .into_iter()
            .filter_map(|m| m.name)
            .collect();
        assert!(names.contains(&"greet".to_string()), "got: {names:?}");
        assert!(names.contains(&"new".to_string()), "got: {names:?}");
    }

    #[test]
    fn types_finds_struct_enum_and_traits() {
        let names: Vec<_> = types(&tree(), SAMPLE)
            .into_iter()
            .filter_map(|m| m.name)
            .collect();
        assert!(names.contains(&"User".to_string()), "got: {names:?}");
        assert!(names.contains(&"Role".to_string()), "got: {names:?}");
    }

    #[test]
    fn uses_finds_use_declaration() {
        assert_eq!(uses(&tree(), SAMPLE).len(), 1);
    }

    #[test]
    fn impls_finds_impl_block() {
        assert_eq!(impls(&tree(), SAMPLE).len(), 1);
    }

    #[test]
    fn attributes_finds_derive() {
        let count = attributes(&tree(), SAMPLE).len();
        assert!(count >= 1, "expected at least one #[derive(...)] attribute");
    }
}
