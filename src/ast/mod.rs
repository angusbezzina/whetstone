#![allow(dead_code)]

//! Tree-sitter substrate for Whetstone's deterministic checks.
//!
//! The goal is a small, opinionated surface: parse a file into a [`Tree`],
//! then ask a handful of well-defined questions (imports, function defs,
//! classes, decorators) without having to hand-roll tree-sitter queries in
//! every caller. `wh check` uses this for its AST signals, and the eval
//! runner will use the same primitives to move off regex fallbacks.

use std::cell::RefCell;
use std::collections::HashMap;

use tree_sitter::{Language, Node, Parser, Query, QueryCursor, Tree};

pub mod python;
pub mod rust_lang;
pub mod typescript;

/// Languages that Whetstone knows how to parse with tree-sitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AstLang {
    Python,
    TypeScript,
    Rust,
}

impl AstLang {
    /// Parse the language name used in rule YAML (`python | typescript | rust`).
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "python" => Some(AstLang::Python),
            "typescript" | "ts" | "javascript" | "js" => Some(AstLang::TypeScript),
            "rust" => Some(AstLang::Rust),
            _ => None,
        }
    }

    /// Infer language from a source-file extension (`py | ts | tsx | rs`).
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext {
            "py" => Some(AstLang::Python),
            "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(AstLang::TypeScript),
            "rs" => Some(AstLang::Rust),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            AstLang::Python => "python",
            AstLang::TypeScript => "typescript",
            AstLang::Rust => "rust",
        }
    }

    fn ts_language(self) -> Language {
        match self {
            AstLang::Python => tree_sitter_python::language(),
            AstLang::TypeScript => tree_sitter_typescript::language_tsx(),
            AstLang::Rust => tree_sitter_rust::language(),
        }
    }
}

thread_local! {
    // Parser is `!Send`, so we cache one per thread. `wh check` is single-
    // threaded for now, but this keeps the door open if we parallelize.
    static PARSERS: RefCell<HashMap<AstLang, Parser>> = RefCell::new(HashMap::new());
}

/// Parse `source` as `lang`, returning the resulting tree. Returns `None` if
/// the grammar fails to install (only possible when the underlying crate is
/// ABI-incompatible, which we detect at test time).
pub fn parse(lang: AstLang, source: &str) -> Option<Tree> {
    PARSERS.with(|parsers| {
        let mut map = parsers.borrow_mut();
        let parser = map.entry(lang).or_insert_with(|| {
            let mut p = Parser::new();
            p.set_language(&lang.ts_language())
                .expect("tree-sitter grammar ABI mismatch — rebuild with matching tree-sitter crate");
            p
        });
        parser.parse(source, None)
    })
}

/// A query match suitable for deterministic reporting. `line` and `column`
/// are 1-based to match tool output conventions (ruff, cargo, etc.).
#[derive(Debug, Clone)]
pub struct AstMatch {
    pub kind: String,
    pub name: Option<String>,
    pub line: usize,
    pub column: usize,
    pub byte_range: (usize, usize),
    pub text: String,
}

/// Compile a query for `lang` and run it against `tree`, returning every
/// capture tagged `@match` paired with its source text.
///
/// Named captures other than `@match` are not returned; if you need them,
/// call tree-sitter directly. This helper keeps the common case — "find
/// every `x` in the tree" — cheap to call.
pub fn run_query(
    lang: AstLang,
    tree: &Tree,
    source: &str,
    query_src: &str,
    kind: &str,
) -> Vec<AstMatch> {
    let language = lang.ts_language();
    let query = match Query::new(&language, query_src) {
        Ok(q) => q,
        Err(e) => {
            eprintln!("Whetstone: invalid tree-sitter query for {}: {}", lang.as_str(), e);
            return Vec::new();
        }
    };
    let capture_index = query.capture_index_for_name("match");

    let mut cursor = QueryCursor::new();
    let mut matches = Vec::new();
    let bytes = source.as_bytes();

    for m in cursor.matches(&query, tree.root_node(), bytes) {
        for cap in m.captures {
            if let Some(ix) = capture_index {
                if cap.index != ix {
                    continue;
                }
            }
            matches.push(node_to_match(&cap.node, source, kind));
        }
    }

    matches
}

fn node_to_match(node: &Node, source: &str, kind: &str) -> AstMatch {
    let start = node.start_position();
    let range = node.byte_range();
    let text = source.get(range.clone()).unwrap_or("").to_string();
    AstMatch {
        kind: kind.to_string(),
        name: extract_name(node, source),
        line: start.row + 1,
        column: start.column + 1,
        byte_range: (range.start, range.end),
        text,
    }
}

fn extract_name(node: &Node, source: &str) -> Option<String> {
    node.child_by_field_name("name")
        .and_then(|n| source.get(n.byte_range()).map(|s| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_maps_language_aliases() {
        assert_eq!(AstLang::from_str("python"), Some(AstLang::Python));
        assert_eq!(AstLang::from_str("ts"), Some(AstLang::TypeScript));
        assert_eq!(AstLang::from_str("rust"), Some(AstLang::Rust));
        assert_eq!(AstLang::from_str("lolcode"), None);
    }

    #[test]
    fn from_extension_maps_common_suffixes() {
        assert_eq!(AstLang::from_extension("py"), Some(AstLang::Python));
        assert_eq!(AstLang::from_extension("tsx"), Some(AstLang::TypeScript));
        assert_eq!(AstLang::from_extension("rs"), Some(AstLang::Rust));
        assert_eq!(AstLang::from_extension("md"), None);
    }

    #[test]
    fn parse_python_produces_usable_tree() {
        let tree = parse(AstLang::Python, "def foo():\n    return 1\n").unwrap();
        assert_eq!(tree.root_node().kind(), "module");
    }

    #[test]
    fn parse_caches_parser_per_language() {
        // Multiple parses in the same thread share the same parser instance;
        // this check exercises the cache path rather than verifying identity
        // directly (Parser is not Clone or comparable).
        assert!(parse(AstLang::Python, "x = 1").is_some());
        assert!(parse(AstLang::Python, "y = 2").is_some());
        assert!(parse(AstLang::Rust, "fn main() {}").is_some());
    }
}
