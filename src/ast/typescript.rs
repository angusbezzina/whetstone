//! TypeScript tree-sitter query helpers. We always parse with the TSX
//! grammar so both `.ts` and `.tsx` work through a single code path.

use tree_sitter::Tree;

use super::{run_query, AstLang, AstMatch};

const FUNCTION_QUERY: &str = r#"
[
  (function_declaration) @match
  (method_definition) @match
  (arrow_function) @match
]
"#;

const CLASS_QUERY: &str = r#"
(class_declaration) @match
"#;

const IMPORT_QUERY: &str = r#"
[
  (import_statement) @match
  (call_expression function: (identifier) @_fn (#eq? @_fn "require")) @match
]
"#;

const DECORATOR_QUERY: &str = r#"
(decorator) @match
"#;

pub fn functions(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::TypeScript, tree, source, FUNCTION_QUERY, "function")
}

pub fn classes(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::TypeScript, tree, source, CLASS_QUERY, "class")
}

pub fn imports(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(AstLang::TypeScript, tree, source, IMPORT_QUERY, "import")
}

pub fn decorators(tree: &Tree, source: &str) -> Vec<AstMatch> {
    run_query(
        AstLang::TypeScript,
        tree,
        source,
        DECORATOR_QUERY,
        "decorator",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse, AstLang};

    const SAMPLE: &str = r#"
import { useState } from 'react';

class Counter {
  @log
  increment() {}
}

function hello() {}
const square = (n: number) => n * n;
"#;

    fn tree() -> tree_sitter::Tree {
        parse(AstLang::TypeScript, SAMPLE).expect("parse typescript")
    }

    #[test]
    fn imports_finds_import_statement() {
        let count = imports(&tree(), SAMPLE).len();
        assert_eq!(count, 1);
    }

    #[test]
    fn classes_finds_class_declaration() {
        let names: Vec<_> = classes(&tree(), SAMPLE)
            .into_iter()
            .filter_map(|m| m.name)
            .collect();
        assert_eq!(names, vec!["Counter"]);
    }

    #[test]
    fn functions_finds_declaration_method_and_arrow() {
        let ms = functions(&tree(), SAMPLE);
        // `hello`, `increment`, and the arrow function = 3
        assert_eq!(ms.len(), 3, "got: {ms:?}");
    }
}
