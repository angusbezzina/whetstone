//! `wh check` — scan source files and report rule violations.
//!
//! Signal handling:
//! - `ast` with `ast_query:` → tree-sitter S-expression query evaluated
//!   against the parsed file; every `@match` capture is a violation.
//! - `ast` with `match:` only → regex fallback; documented as weaker than
//!   a real AST check so extractors can upgrade incrementally.
//! - `pattern` with `match:` → regex scan. When `ast_scope:` is set, the
//!   regex is restricted to the source span of AST nodes whose kind
//!   matches (e.g. `function_definition`); this removes comment/no-op
//!   false positives for scope-sensitive rules.
//! - `lint_proxy` → verified against the project's linter config (ruff,
//!   biome). Missing rules surface as `config_issues` so the user gets
//!   actionable guidance rather than a silent pass.
//! - `ai` → skipped with a note; evaluated only via `wh eval run`.

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tree_sitter::{Query, QueryCursor, Tree};

use crate::ast::{self, AstLang};
use crate::detect::walk::SKIP_DIRS;
use crate::layers;
use crate::rules::ApprovedRule;

mod lint_proxy;

pub struct CheckOptions<'a> {
    pub project_dir: &'a Path,
    pub scan_paths: &'a [PathBuf],
    pub lang_filter: Option<&'a str>,
    pub rule_filter: Option<&'a [String]>,
}

pub fn run(opts: CheckOptions<'_>) -> Result<Value> {
    let project_dir = opts.project_dir;
    let whetstone_cfg = project_dir.join("whetstone").join("whetstone.yaml");
    let have_cfg = whetstone_cfg.exists() || project_dir.join("whetstone.yaml").exists();

    let rules: Vec<ApprovedRule> = if have_cfg {
        let merged = layers::resolve_merged(project_dir, opts.lang_filter, true, true, false);
        merged.merged.into_iter().map(|lr| lr.rule).collect()
    } else {
        let paths = layers::LayerPaths::for_project(project_dir);
        let (r, _) = crate::rules::load_approved_rules(&paths.project_rules_dir, opts.lang_filter);
        r
    };

    let rule_filter: Option<BTreeSet<&str>> = opts
        .rule_filter
        .map(|ids| ids.iter().map(String::as_str).collect());

    let rules: Vec<&ApprovedRule> = rules
        .iter()
        .filter(|r| rule_filter.as_ref().map(|set| set.contains(r.id.as_str())).unwrap_or(true))
        .collect();

    if rules.is_empty() {
        return Ok(json!({
            "status": "ok",
            "violations_count": 0,
            "files_scanned": 0,
            "rules_applied": 0,
            "violations": [],
            "skipped": [],
            "config_issues": [],
            "warnings": ["No approved rules match the supplied filters."],
        }));
    }

    let compiled = compile_rules(&rules);
    let mut violations: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut files_scanned: usize = 0;

    for crule in &compiled {
        for note in &crule.notes {
            skipped.push(json!({"rule_id": crule.rule.id, "reason": note}));
        }
    }

    let config_issues = lint_proxy::verify_lint_proxies(project_dir, &rules);

    let files = discover_source_files(opts.scan_paths);
    for (path, lang) in &files {
        if let Some(filter) = opts.lang_filter {
            if lang.as_str() != filter {
                continue;
            }
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("Failed to read {}: {e}", path.display()));
                continue;
            }
        };
        files_scanned += 1;

        let applicable: Vec<&CompiledRule> = compiled
            .iter()
            .filter(|cr| rule_applies_to_language(&cr.rule.language, lang))
            .collect();
        if applicable.is_empty() {
            continue;
        }

        let tree = if applicable
            .iter()
            .any(|cr| cr.signals.iter().any(|s| s.needs_tree()))
        {
            match ast::parse(*lang, &text) {
                Some(t) => Some(t),
                None => {
                    warnings.push(format!("tree-sitter parse failed for {}", path.display()));
                    None
                }
            }
        } else {
            None
        };

        for crule in applicable {
            for sig in &crule.signals {
                let hits = apply_signal(sig, *lang, &text, tree.as_ref());
                for hit in hits {
                    violations.push(json!({
                        "rule_id": crule.rule.id,
                        "severity": crule.rule.severity,
                        "category": crule.rule.category,
                        "description": first_line(&crule.rule.description),
                        "source_url": crule.rule.source_url,
                        "language": lang.as_str(),
                        "signal_id": sig.signal_id,
                        "signal_strategy": sig.strategy,
                        "signal_description": sig.description,
                        "signal_check_type": sig.check_kind(),
                        "file": path.display().to_string(),
                        "line": hit.line,
                        "column": hit.column,
                        "match": hit.text,
                    }));
                }
            }
        }
    }

    let violations_count = violations.len() as i64;
    let config_issue_count = config_issues.len() as i64;
    let status = if violations_count == 0 && config_issue_count == 0 {
        "ok"
    } else if violations_count > 0 {
        "violations_found"
    } else {
        "config_issues_found"
    };
    Ok(json!({
        "status": status,
        "violations_count": violations_count,
        "config_issues_count": config_issue_count,
        "files_scanned": files_scanned,
        "rules_applied": rules.len(),
        "violations": violations,
        "config_issues": config_issues,
        "skipped": skipped,
        "warnings": warnings,
    }))
}

// ── Human formatter ──

pub fn format_human_output(result: &Value) -> String {
    let mut out = String::new();
    let count = result
        .get("violations_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let config_count = result
        .get("config_issues_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let files = result
        .get("files_scanned")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let rules = result
        .get("rules_applied")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    if count == 0 && config_count == 0 {
        out.push_str(&format!(
            "wh check: no violations ({rules} rule(s), {files} file(s))\n"
        ));
        return out;
    }

    out.push_str(&format!(
        "wh check: {count} violation(s), {config_count} config issue(s) across {files} file(s) against {rules} rule(s)\n"
    ));
    if let Some(arr) = result.get("violations").and_then(|v| v.as_array()) {
        for v in arr {
            let file = v.get("file").and_then(|v| v.as_str()).unwrap_or("?");
            let line = v.get("line").and_then(|v| v.as_i64()).unwrap_or(0);
            let sev = v.get("severity").and_then(|v| v.as_str()).unwrap_or("?");
            let id = v.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?");
            let desc = v.get("description").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("  {file}:{line}: [{sev}] {id} — {desc}\n"));
        }
    }
    if let Some(arr) = result.get("config_issues").and_then(|v| v.as_array()) {
        for v in arr {
            let id = v.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?");
            let linter = v.get("linter").and_then(|v| v.as_str()).unwrap_or("?");
            let code = v.get("code").and_then(|v| v.as_str()).unwrap_or("?");
            let issue = v.get("issue").and_then(|v| v.as_str()).unwrap_or("?");
            out.push_str(&format!("  config[{linter}:{code}] {id}: {issue}\n"));
        }
    }
    out
}

// ── Internal types ──

struct CompiledRule<'a> {
    rule: &'a ApprovedRule,
    signals: Vec<CompiledSignal>,
    notes: Vec<String>,
}

struct CompiledSignal {
    signal_id: String,
    description: String,
    strategy: String,
    regex: Option<Regex>,
    ast_query: Option<String>,
    ast_scope: Option<String>,
}

impl CompiledSignal {
    fn needs_tree(&self) -> bool {
        self.ast_query.is_some() || self.ast_scope.is_some()
    }
    fn check_kind(&self) -> &'static str {
        if self.ast_query.is_some() {
            "ast_query"
        } else if self.ast_scope.is_some() {
            "ast_scoped_regex"
        } else {
            "regex"
        }
    }
}

#[derive(Debug)]
struct SignalHit {
    line: usize,
    column: usize,
    text: String,
}

fn compile_rules<'a>(rules: &[&'a ApprovedRule]) -> Vec<CompiledRule<'a>> {
    let mut out = Vec::new();
    for rule in rules {
        let mut signals = Vec::new();
        let mut notes = Vec::new();
        for sig in &rule.signals {
            match sig.strategy.as_str() {
                "pattern" | "ast" => {
                    let regex = match sig.match_pattern.as_deref() {
                        Some(pat) => match Regex::new(pat) {
                            Ok(re) => Some(re),
                            Err(e) => {
                                notes.push(format!(
                                    "signal {}: invalid regex `{}` — {e}",
                                    sig.id, pat
                                ));
                                None
                            }
                        },
                        None => None,
                    };
                    if sig.strategy == "ast" && sig.ast_query.is_none() && regex.is_none() {
                        notes.push(format!(
                            "signal {}: ast signal has neither `ast_query:` nor `match:`; cannot enforce",
                            sig.id
                        ));
                        continue;
                    }
                    if sig.strategy == "pattern" && regex.is_none() {
                        notes.push(format!(
                            "signal {}: pattern signal has no `match:` regex; cannot enforce",
                            sig.id
                        ));
                        continue;
                    }
                    signals.push(CompiledSignal {
                        signal_id: sig.id.clone(),
                        description: sig.description.clone(),
                        strategy: sig.strategy.clone(),
                        regex,
                        ast_query: sig.ast_query.clone(),
                        ast_scope: sig.ast_scope.clone(),
                    });
                }
                "lint_proxy" => {
                    notes.push(format!(
                        "signal {}: lint_proxy checked via linter config; see config_issues",
                        sig.id
                    ));
                }
                "ai" => {
                    notes.push(format!(
                        "signal {}: ai signal evaluated via `wh eval run` only",
                        sig.id
                    ));
                }
                other => notes.push(format!("signal {}: unknown strategy {other}", sig.id)),
            }
        }
        out.push(CompiledRule {
            rule,
            signals,
            notes,
        });
    }
    out
}

fn apply_signal(
    sig: &CompiledSignal,
    lang: AstLang,
    text: &str,
    tree: Option<&Tree>,
) -> Vec<SignalHit> {
    if let Some(query_src) = &sig.ast_query {
        if let Some(tree) = tree {
            return run_ast_query(query_src, lang, tree, text);
        }
        return Vec::new();
    }
    let re = match &sig.regex {
        Some(r) => r,
        None => return Vec::new(),
    };
    if let Some(scope_kind) = &sig.ast_scope {
        if let Some(tree) = tree {
            return scan_with_ast_scope(re, scope_kind, tree, text);
        }
        return Vec::new();
    }
    scan_lines(re, text)
}

fn scan_lines(re: &Regex, text: &str) -> Vec<SignalHit> {
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if let Some(m) = re.find(line) {
            hits.push(SignalHit {
                line: i + 1,
                column: m.start() + 1,
                text: m.as_str().to_string(),
            });
        }
    }
    hits
}

/// Run a raw tree-sitter query and turn every `@match` capture into a hit.
fn run_ast_query(query_src: &str, lang: AstLang, tree: &Tree, source: &str) -> Vec<SignalHit> {
    let language = match lang {
        AstLang::Python => tree_sitter_python::language(),
        AstLang::TypeScript => tree_sitter_typescript::language_tsx(),
        AstLang::Rust => tree_sitter_rust::language(),
    };
    let query = match Query::new(&language, query_src) {
        Ok(q) => q,
        Err(e) => {
            eprintln!(
                "Whetstone: skipping malformed ast_query for {}: {e}",
                lang.as_str()
            );
            return Vec::new();
        }
    };
    let match_index = query.capture_index_for_name("match");
    let mut cursor = QueryCursor::new();
    let bytes = source.as_bytes();
    let mut hits = Vec::new();
    for m in cursor.matches(&query, tree.root_node(), bytes) {
        for cap in m.captures {
            if let Some(ix) = match_index {
                if cap.index != ix {
                    continue;
                }
            }
            let start = cap.node.start_position();
            let range = cap.node.byte_range();
            let text = source.get(range.clone()).unwrap_or("").to_string();
            hits.push(SignalHit {
                line: start.row + 1,
                column: start.column + 1,
                text,
            });
        }
    }
    hits
}

/// For `ast_scope:` pattern signals, walk the tree and apply the regex only
/// to the source span of nodes whose kind matches `scope_kind`.
fn scan_with_ast_scope(re: &Regex, scope_kind: &str, tree: &Tree, source: &str) -> Vec<SignalHit> {
    let mut hits = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    walk_nodes(&mut cursor, scope_kind, source, re, &mut hits);
    hits
}

fn walk_nodes(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    scope_kind: &str,
    source: &str,
    re: &Regex,
    hits: &mut Vec<SignalHit>,
) {
    let node = cursor.node();
    if node.kind() == scope_kind {
        let start_byte = node.start_byte();
        let start_line = node.start_position().row;
        let span = source.get(node.byte_range()).unwrap_or("");
        if let Some(m) = re.find(span) {
            let offset = m.start();
            let (line_offset, col) = line_col_within(span, offset);
            let absolute_line = start_line + line_offset;
            let absolute_col = if line_offset == 0 {
                node.start_position().column + col
            } else {
                col
            };
            hits.push(SignalHit {
                line: absolute_line + 1,
                column: absolute_col + 1,
                text: m.as_str().to_string(),
            });
        }
        let _ = start_byte;
    }
    if cursor.goto_first_child() {
        loop {
            walk_nodes(cursor, scope_kind, source, re, hits);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Given a substring `span` and a byte offset inside it, return the 0-based
/// (line_offset_within_span, column) pair. Used to turn an intra-span regex
/// hit into absolute source coordinates.
fn line_col_within(span: &str, offset: usize) -> (usize, usize) {
    let prefix = &span[..offset.min(span.len())];
    let line_offset = prefix.bytes().filter(|b| *b == b'\n').count();
    let col = match prefix.rfind('\n') {
        Some(nl) => prefix.len() - nl - 1,
        None => prefix.len(),
    };
    (line_offset, col)
}

// ── File discovery ──

fn discover_source_files(roots: &[PathBuf]) -> Vec<(PathBuf, AstLang)> {
    let mut out = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                !SKIP_DIRS.iter().any(|d| *d == name)
            })
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(ext) = entry.path().extension().and_then(|e| e.to_str()) {
                if let Some(lang) = AstLang::from_extension(ext) {
                    out.push((entry.into_path(), lang));
                }
            }
        }
    }
    out
}

fn rule_applies_to_language(rule_lang: &str, file_lang: &AstLang) -> bool {
    let normalized = rule_lang.to_ascii_lowercase();
    match file_lang {
        AstLang::Python => matches!(normalized.as_str(), "python" | "py"),
        AstLang::TypeScript => matches!(
            normalized.as_str(),
            "typescript" | "javascript" | "ts" | "tsx" | "js" | "jsx"
        ),
        AstLang::Rust => matches!(normalized.as_str(), "rust" | "rs"),
    }
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rule_with(id: &str, lang: &str, strategy: &str) -> ApprovedRule {
        ApprovedRule {
            id: id.into(),
            severity: "must".into(),
            confidence: "high".into(),
            category: "default".into(),
            description: "test rule".into(),
            source_url: "https://example".into(),
            source_name: "demo".into(),
            language: lang.into(),
            signals: vec![crate::rules::ApprovedSignal {
                id: "s1".into(),
                strategy: strategy.into(),
                description: "signal".into(),
                weight: "required".into(),
                match_pattern: None,
                ast_query: None,
                ast_scope: None,
            }],
            golden_examples: Vec::new(),
            risk: None,
            linter_gap: None,
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
            ai_eval: None,
        }
    }

    #[test]
    fn pattern_signal_fires_on_match() {
        let mut rule = rule_with("demo.unwrap", "rust", "pattern");
        rule.signals[0].match_pattern = Some(r"\.unwrap\(\)".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let hits = apply_signal(
            &compiled[0].signals[0],
            AstLang::Rust,
            "let v = x.unwrap();\n",
            None,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].text, ".unwrap()");
    }

    #[test]
    fn invalid_regex_skipped_with_note() {
        let mut rule = rule_with("demo.bad", "python", "pattern");
        rule.signals[0].match_pattern = Some("[invalid".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        assert!(compiled[0].signals.is_empty());
        assert!(compiled[0].notes.iter().any(|n| n.contains("invalid regex")));
    }

    #[test]
    fn ast_query_reports_function_definitions_only() {
        // Python rule whose ast_query matches every def foo() in the file.
        let mut rule = rule_with("demo.nofun", "python", "ast");
        rule.signals[0].ast_query = Some("(function_definition) @match".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let source = "def a():\n    pass\n\ndef b():\n    pass\n\nx = 1\n";
        let tree = ast::parse(AstLang::Python, source).unwrap();
        let hits = apply_signal(
            &compiled[0].signals[0],
            AstLang::Python,
            source,
            Some(&tree),
        );
        assert_eq!(hits.len(), 2, "got: {hits:?}");
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 4);
    }

    #[test]
    fn ast_scope_restricts_regex_to_nodes_of_kind() {
        // Pattern rule that should only flag TODO inside function bodies,
        // not inside module-level comments.
        let mut rule = rule_with("demo.todo", "python", "pattern");
        rule.signals[0].match_pattern = Some("TODO".into());
        rule.signals[0].ast_scope = Some("function_definition".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let source = "# module-level TODO should be ignored\n\ndef foo():\n    # TODO inside body should fire\n    pass\n";
        let tree = ast::parse(AstLang::Python, source).unwrap();
        let hits = apply_signal(
            &compiled[0].signals[0],
            AstLang::Python,
            source,
            Some(&tree),
        );
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!(hits[0].line, 4);
    }

    #[test]
    fn discover_filters_by_extension_and_skip_dirs() {
        let tmp = tempdir();
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        write_file(&tmp.join("src/a.py"), "x = 1\n");
        write_file(&tmp.join("src/b.rs"), "fn main() {}\n");
        write_file(&tmp.join("node_modules/c.ts"), "const x = 1;\n");
        let files = discover_source_files(std::slice::from_ref(&tmp));
        let names: Vec<_> = files
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.py".to_string()));
        assert!(names.contains(&"b.rs".to_string()));
        assert!(!names.contains(&"c.ts".to_string()));
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "wh-check-test-{}",
            std::process::id() as u128 * 1_000_000 + rand_part()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn rand_part() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn write_file(path: &Path, body: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }
}
