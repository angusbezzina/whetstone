//! AST enforcer: a raw tree-sitter query over the files in scope; every
//! `@match` capture is a violation.

use tree_sitter::QueryCursor;

use crate::ast::{self, AstLang};
use crate::proof::ArtifactFailure;

use super::files::FileSet;

/// The outcome of one query over a file set.
#[derive(Debug, Default)]
pub struct AstRun {
    pub files_checked: usize,
    pub failures: Vec<ArtifactFailure>,
    /// Languages the query compiled for.
    pub languages: Vec<&'static str>,
}

fn language_of(path: &str) -> Option<AstLang> {
    path.rsplit_once('.')
        .and_then(|(_, extension)| AstLang::from_extension(extension))
}

/// Run `query` over every file in `files` accepted by `applies`. With a named
/// language the query compiles once; without one it runs for each language
/// it compiles for. A query that compiles for no language is a configuration
/// error, never an empty pass.
pub fn run_query(
    files: &FileSet,
    query: &str,
    language: Option<&str>,
    applies: impl Fn(&str) -> bool,
    limit: usize,
) -> Result<AstRun, String> {
    let languages = match language {
        Some(name) => vec![AstLang::from_str(name)
            .ok_or_else(|| format!("{name} is not a language the AST enforcer parses"))?],
        None => vec![AstLang::Rust, AstLang::TypeScript, AstLang::Python],
    };
    let mut compiled = Vec::new();
    let mut errors = Vec::new();
    for lang in languages {
        match ast::compile_query(lang, query) {
            Ok(query) => compiled.push((lang, query)),
            Err(error) => errors.push(error),
        }
    }
    if compiled.is_empty() {
        return Err(errors.join("; "));
    }
    let mut run = AstRun {
        languages: compiled.iter().map(|(lang, _)| lang.as_str()).collect(),
        ..AstRun::default()
    };
    for (path, bytes) in files.iter() {
        if !applies(path) {
            continue;
        }
        let Some(lang) = language_of(path) else {
            continue;
        };
        let Some((_, query)) = compiled.iter().find(|(candidate, _)| *candidate == lang) else {
            continue;
        };
        let Ok(source) = std::str::from_utf8(bytes) else {
            continue;
        };
        let Some(tree) = ast::parse(lang, source) else {
            continue;
        };
        run.files_checked += 1;
        let Some(match_index) = query.capture_index_for_name("match") else {
            continue;
        };
        let mut cursor = QueryCursor::new();
        for found in cursor.matches(query, tree.root_node(), source.as_bytes()) {
            for capture in found.captures {
                if capture.index != match_index || run.failures.len() >= limit {
                    continue;
                }
                let line = capture.node.start_position().row + 1;
                let text = source
                    .get(capture.node.byte_range())
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .chars()
                    .take(160)
                    .collect::<String>();
                run.failures.push(ArtifactFailure {
                    location: format!("{path}:{line}"),
                    message: text,
                });
            }
        }
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::gates::files::Source;

    #[test]
    fn a_query_reports_each_match_with_its_location() {
        let files = FileSet::from_contents(
            Source::Staged,
            BTreeMap::from([
                (
                    "src/lib.rs".to_string(),
                    b"fn a() { x.unwrap(); }\nfn b() { y.expect(\"why\"); }\n".to_vec(),
                ),
                ("README.md".to_string(), b"x.unwrap()".to_vec()),
            ]),
        );
        let run = run_query(
            &files,
            r#"(call_expression function: (field_expression field: (field_identifier) @m (#eq? @m "unwrap"))) @match"#,
            Some("rust"),
            |_| true,
            20,
        )
        .expect("run");
        assert_eq!(run.files_checked, 1);
        assert_eq!(run.failures.len(), 1);
        assert_eq!(run.failures[0].location, "src/lib.rs:1");
    }

    #[test]
    fn a_query_that_compiles_for_no_language_is_an_error() {
        let files = FileSet::default();
        assert!(run_query(&files, "(not_a_node) @match", None, |_| true, 20).is_err());
        assert!(run_query(&files, "(identifier) @nomatch", Some("rust"), |_| true, 20).is_err());
    }
}
