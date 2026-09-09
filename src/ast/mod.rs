//! Tree-sitter substrate for Whetstone's deterministic checks.
//!
//! The goal is a small, opinionated surface: parse a file into a [`Tree`],
//! then ask a handful of well-defined questions (imports, function defs,
//! classes, decorators) without having to hand-roll tree-sitter queries in
//! every caller. `wh check` uses this for its AST signals, and the eval
//! scanner and golden evaluator share these exact parsing primitives.

use std::cell::RefCell;
use std::collections::HashMap;

use tree_sitter::{Language, Parser, Query, Tree};

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
        match crate::types::canonical_language(s)? {
            "python" => Some(AstLang::Python),
            "typescript" | "javascript" => Some(AstLang::TypeScript),
            "rust" => Some(AstLang::Rust),
            _ => None,
        }
    }

    /// Infer language from a source-file extension (`py | ts | tsx | rs`).
    pub fn from_extension(ext: &str) -> Option<Self> {
        match crate::types::source_language_for_extension(ext)? {
            "python" => Some(AstLang::Python),
            "typescript" | "javascript" => Some(AstLang::TypeScript),
            "rust" => Some(AstLang::Rust),
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

/// Compile a rule query for a supported language and require the capture that
/// the scanner reports as a violation. Configuration errors must never degrade
/// into an empty (apparently successful) scan.
pub fn compile_query(lang: AstLang, source: &str) -> Result<Query, String> {
    let query = Query::new(&lang.ts_language(), source)
        .map_err(|error| format!("invalid {} tree-sitter query: {error}", lang.as_str()))?;
    if query.capture_index_for_name("match").is_none() {
        return Err(format!(
            "{} tree-sitter query must define an @match capture",
            lang.as_str()
        ));
    }
    Ok(query)
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
            p.set_language(&lang.ts_language()).expect(
                "tree-sitter grammar ABI mismatch — rebuild with matching tree-sitter crate",
            );
            p
        });
        parser.parse(source, None)
    })
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
        assert_eq!(AstLang::from_extension("js"), Some(AstLang::TypeScript));
        assert_eq!(AstLang::from_extension("rs"), Some(AstLang::Rust));
        assert_eq!(AstLang::from_extension("md"), None);
    }

    #[test]
    fn parse_python_produces_usable_tree() {
        let tree = parse(AstLang::Python, "def foo():\n    return 1\n")
            .expect("valid Python fixture should parse");
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

    #[test]
    fn query_compilation_requires_valid_syntax_and_match_capture() {
        assert!(compile_query(AstLang::Python, "(function_definition) @match").is_ok());
        assert!(compile_query(AstLang::Python, "(function_definition").is_err());
        assert!(compile_query(AstLang::Python, "(function_definition) @node").is_err());
    }
}
