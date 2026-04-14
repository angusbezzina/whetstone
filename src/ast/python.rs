//! Python tree-sitter query helpers.
//!
//! Queries capture the outer node (`@match`) so callers can pull
//! `line`/`column` from the whole construct and `name` via the
//! `child_by_field_name("name")` lookup in [`crate::ast`].

use tree_sitter::Tree;

use super::{run_query, AstLang, AstMatch};

const FUNCTION_QUERY: &str = r#"
(function_definition) @match
"#;

// `async def` is a `function_definition` carrying an anonymous `async` token.
const ASYNC_FUNCTION_QUERY: &str = r#"
(function_definition "async") @match
"#;

const CLASS_QUERY: &str = r#"
(class_definition) @match
"#;

const DECORATOR_QUERY: &str = r#"
(decorator) @match
"#;

const IMPORT_QUERY: &str = r#"
[
  (import_statement) @match
  (import_from_statement) @match
]
"#;

pub fn functions(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Python, tree, source, FUNCTION_QUERY, "function")
}

pub fn async_functions(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(
        AstLang::Python,
        tree,
        source,
        ASYNC_FUNCTION_QUERY,
        "async_function",
    )
}

pub fn classes(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Python, tree, source, CLASS_QUERY, "class")
}

pub fn decorators(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Python, tree, source, DECORATOR_QUERY, "decorator")
}

pub fn imports(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::Python, tree, source, IMPORT_QUERY, "import")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse, AstLang};

    const SAMPLE: &str = r#"
from typing import List
import os

@app.get("/users")
def list_users():
    return []

async def fetch():
    return {}

class User:
    pass
"#;

    fn tree() -> tree_sitter::Tree {
        parse(AstLang::Python, SAMPLE).expect("parse python")
    }

    #[test]
    fn functions_finds_sync_and_decorated() {
        let names: Vec<_> = functions(&tree(), SAMPLE)
            .into_iter()
            .filter_map(|m| m.name)
            .collect();
        assert!(names.contains(&"list_users".to_string()), "got: {names:?}");
        assert!(names.contains(&"fetch".to_string()), "got: {names:?}");
    }

    #[test]
    fn async_functions_finds_async_def() {
        let names: Vec<_> = async_functions(&tree(), SAMPLE)
            .into_iter()
            .filter_map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["fetch"]);
    }

    #[test]
    fn classes_finds_class_definitions() {
        let names: Vec<_> = classes(&tree(), SAMPLE)
            .into_iter()
            .filter_map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["User"]);
    }

    #[test]
    fn decorators_finds_decorator_expression() {
        let count = decorators(&tree(), SAMPLE).len();
        assert_eq!(count, 1, "expected one @app.get decorator");
    }

    #[test]
    fn imports_finds_both_import_forms() {
        let count = imports(&tree(), SAMPLE).len();
        assert_eq!(count, 2, "expected `from typing` and `import os`");
    }
}
