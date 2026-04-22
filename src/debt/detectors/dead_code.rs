//! Dead-code detection:
//!
//! - **Unreferenced private symbols** — a `fn`/`struct`/`enum`/`trait`
//!   (Rust), `def`/`class` (Python), or `function`/`const`/`class`
//!   (TS) defined privately with zero references outside its definition
//!   site anywhere in the scanned source tree.
//! - **Orphaned module** (Rust-only v1) — a `.rs` file whose containing
//!   module path is never imported by a `mod` or `use` statement in any
//!   other file.
//!
//! Private = not part of the public API (`pub` absence in Rust, leading
//! `_` in Python, non-`export` in TS). Public API is intentionally
//! out-of-scope — downstream consumers may depend on it and we can't
//! see them.

use anyhow::Result;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::ast::{self, AstLang};
use crate::debt::types::{Category, Confidence, Evidence, Finding, Location, SourceInventory};

pub fn run(_project_dir: &Path, sources: &SourceInventory) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();

    findings.extend(rust_unreferenced_private_symbols(sources));
    findings.extend(python_unreferenced_private_symbols(sources));
    findings.extend(ts_unreferenced_private_symbols(sources));

    findings.extend(rust_orphaned_modules(sources));

    Ok(findings)
}

// -----------------------------------------------------------------------------
// Rust
// -----------------------------------------------------------------------------

struct Definition {
    name: String,
    kind: String, // "function" | "type"
    file: PathBuf,
    line: u32,
    public: bool,
}

fn rust_unreferenced_private_symbols(sources: &SourceInventory) -> Vec<Finding> {
    let mut defs: Vec<Definition> = Vec::new();
    let mut corpus: HashMap<PathBuf, String> = HashMap::new();

    let test_mod_re =
        Regex::new(r"(?ms)#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+[A-Za-z_][\w]*\s*\{").unwrap();
    let test_attr_re = Regex::new(
        r"#\[(?:test|cfg\s*\(\s*test\s*\)|tokio::test|async_std::test|rstest)\b",
    )
    .unwrap();

    for path in &sources.rust {
        // Helpers in top-level `tests/` are fixtures for integration test
        // binaries; their callers don't live in the crate graph we scan.
        if is_test_support_path(path) {
            corpus.insert(
                path.clone(),
                std::fs::read_to_string(path).unwrap_or_default(),
            );
            continue;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let tree = match ast::parse(AstLang::Rust, &text) {
            Some(t) => t,
            None => continue,
        };
        let fns = crate::ast::rust_lang::functions(&tree, &text);
        let types = crate::ast::rust_lang::types(&tree, &text);

        // Byte ranges covering `#[cfg(test)] mod <name> { ... }` blocks so
        // we can ignore test-only helpers living in them.
        let test_mod_ranges = locate_brace_blocks(&text, &test_mod_re);

        for m in fns {
            if let Some(name) = m.name {
                if name.starts_with("test_") {
                    continue;
                }
                let (def_start, _) = m.byte_range;
                if inside_any(&test_mod_ranges, def_start) {
                    continue;
                }
                if has_test_attribute(&text, def_start, &test_attr_re) {
                    continue;
                }
                let public = m.text.trim_start().starts_with("pub ")
                    || m.text.contains("pub(crate)")
                    || m.text.contains("pub(super)");
                defs.push(Definition {
                    name,
                    kind: "function".into(),
                    file: path.clone(),
                    line: m.line as u32,
                    public,
                });
            }
        }
        for m in types {
            if let Some(name) = m.name {
                let (def_start, _) = m.byte_range;
                if inside_any(&test_mod_ranges, def_start) {
                    continue;
                }
                let public = m.text.trim_start().starts_with("pub ")
                    || m.text.contains("pub(crate)");
                defs.push(Definition {
                    name,
                    kind: "type".into(),
                    file: path.clone(),
                    line: m.line as u32,
                    public,
                });
            }
        }

        corpus.insert(path.clone(), text);
    }

    emit_private_symbol_findings(defs, &corpus, "rust")
}

fn locate_brace_blocks(text: &str, re: &Regex) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut ranges = Vec::new();
    for m in re.find_iter(text) {
        // m matches up through the opening `{`; find its matching `}`.
        let start = m.start();
        let mut depth: i32 = 0;
        let mut i = m.end().saturating_sub(1);
        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        ranges.push((start, i + 1));
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    ranges
}

fn inside_any(ranges: &[(usize, usize)], pos: usize) -> bool {
    ranges.iter().any(|&(a, b)| pos >= a && pos < b)
}

fn is_test_support_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    let norm = s.replace('\\', "/");
    norm.contains("/tests/")
        || norm.starts_with("tests/")
        || norm.contains("/benches/")
        || norm.starts_with("benches/")
        || norm.contains("/examples/")
}

fn has_test_attribute(text: &str, def_start: usize, re: &Regex) -> bool {
    // Look at the ~200 bytes preceding the function definition for an outer
    // attribute. Rust outer attributes always come immediately before the
    // item they annotate.
    let window_start = def_start.saturating_sub(200);
    let slice = match text.get(window_start..def_start) {
        Some(s) => s,
        None => return false,
    };
    re.is_match(slice)
}

// -----------------------------------------------------------------------------
// Python
// -----------------------------------------------------------------------------

fn python_unreferenced_private_symbols(sources: &SourceInventory) -> Vec<Finding> {
    let mut defs: Vec<Definition> = Vec::new();
    let mut corpus: HashMap<PathBuf, String> = HashMap::new();

    for path in &sources.python {
        if is_test_support_path(path) {
            corpus.insert(
                path.clone(),
                std::fs::read_to_string(path).unwrap_or_default(),
            );
            continue;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let tree = match ast::parse(AstLang::Python, &text) {
            Some(t) => t,
            None => continue,
        };
        let fns = crate::ast::python::functions(&tree, &text);
        let classes = crate::ast::python::classes(&tree, &text);

        for m in fns {
            if let Some(name) = m.name {
                // Private = leading underscore. Dunders (__init__ etc.) are
                // framework-called and treated as public.
                let dunder = name.starts_with("__") && name.ends_with("__");
                let public = dunder || !name.starts_with('_');
                defs.push(Definition {
                    name,
                    kind: "function".into(),
                    file: path.clone(),
                    line: m.line as u32,
                    public,
                });
            }
        }
        for m in classes {
            if let Some(name) = m.name {
                let public = !name.starts_with('_');
                defs.push(Definition {
                    name,
                    kind: "type".into(),
                    file: path.clone(),
                    line: m.line as u32,
                    public,
                });
            }
        }

        corpus.insert(path.clone(), text);
    }

    emit_private_symbol_findings(defs, &corpus, "python")
}

// -----------------------------------------------------------------------------
// TypeScript / JavaScript
// -----------------------------------------------------------------------------

fn ts_unreferenced_private_symbols(sources: &SourceInventory) -> Vec<Finding> {
    // For TS, use a regex scan for top-level `function`, `const`,
    // `class` declarations, and decide public-ness by whether the line
    // starts with `export`.
    let fn_re = Regex::new(
        r"(?m)^(?P<prefix>export\s+(?:default\s+)?|)(?:async\s+)?function\s+(?P<name>[A-Za-z_$][\w$]*)",
    )
    .unwrap();
    let const_re = Regex::new(
        r"(?m)^(?P<prefix>export\s+(?:default\s+)?|)(?:const|let|var)\s+(?P<name>[A-Za-z_$][\w$]*)",
    )
    .unwrap();
    let class_re = Regex::new(
        r"(?m)^(?P<prefix>export\s+(?:default\s+)?|)(?:abstract\s+)?class\s+(?P<name>[A-Za-z_$][\w$]*)",
    )
    .unwrap();

    let mut defs: Vec<Definition> = Vec::new();
    let mut corpus: HashMap<PathBuf, String> = HashMap::new();

    for path in &sources.typescript {
        if is_test_support_path(path) {
            corpus.insert(
                path.clone(),
                std::fs::read_to_string(path).unwrap_or_default(),
            );
            continue;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for (re, kind) in [
            (&fn_re, "function"),
            (&const_re, "type"),
            (&class_re, "type"),
        ] {
            for cap in re.captures_iter(&text) {
                let name = match cap.name("name") {
                    Some(n) => n.as_str().to_string(),
                    None => continue,
                };
                let prefix = cap.name("prefix").map(|m| m.as_str()).unwrap_or("");
                let public = !prefix.is_empty();
                let line = (text[..cap.get(0).unwrap().start()]
                    .chars()
                    .filter(|c| *c == '\n')
                    .count()
                    + 1) as u32;
                defs.push(Definition {
                    name,
                    kind: kind.into(),
                    file: path.clone(),
                    line,
                    public,
                });
            }
        }
        corpus.insert(path.clone(), text);
    }

    emit_private_symbol_findings(defs, &corpus, "typescript")
}

// -----------------------------------------------------------------------------
// Shared finding emitter.
// -----------------------------------------------------------------------------

fn emit_private_symbol_findings(
    defs: Vec<Definition>,
    corpus: &HashMap<PathBuf, String>,
    lang: &str,
) -> Vec<Finding> {
    let mut out = Vec::new();
    // Count references for every symbol by scanning the full corpus for
    // a word-boundary match of the name. Same-line self-definition is
    // filtered out via the definition site counter.
    for d in &defs {
        if d.public {
            continue;
        }
        // Ignore privates that conventionally exist for tests only.
        if d.name.starts_with("test_") && lang == "python" {
            continue;
        }
        // Ignore common known-noise names.
        if matches!(d.name.as_str(), "_" | "__") {
            continue;
        }

        let re = Regex::new(&format!(r"\b{}\b", regex::escape(&d.name))).unwrap();
        let mut total = 0u32;
        for text in corpus.values() {
            total += re.find_iter(text).count() as u32;
        }
        // Subtract one occurrence for the definition itself.
        let refs = total.saturating_sub(1);

        if refs == 0 {
            let rel = d.file.display().to_string();
            out.push(Finding {
                category: Category::Dead,
                rule_id: "dead.unreferenced_private_symbols".into(),
                title: format!("Unreferenced private {} ({}): {}", d.kind, lang, d.name),
                confidence: Confidence::High,
                evidence_strength: 1.0,
                files: vec![rel.clone()],
                evidence: Evidence::SymbolDef {
                    name: d.name.clone(),
                    symbol_kind: d.kind.clone(),
                    references: 0,
                    locations: vec![Location {
                        file: rel.clone(),
                        line: Some(d.line),
                    }],
                },
                next_action: format!(
                    "Delete {} `{}` in {} if truly unused, or wire up the caller that was meant to use it.",
                    d.kind, d.name, rel
                ),
            });
        }
    }
    out
}

// -----------------------------------------------------------------------------
// Rust orphaned modules.
// -----------------------------------------------------------------------------

fn rust_orphaned_modules(sources: &SourceInventory) -> Vec<Finding> {
    if sources.rust.is_empty() {
        return Vec::new();
    }

    // A module file is "wired up" if *any* rust file contains a
    // `mod <name>;` or `mod <name> { ... }` declaration, or a
    // `use <crate_root>::<name>` that targets it.
    let mut declared_mods: HashSet<String> = HashSet::new();
    let mod_re = Regex::new(r"(?m)^\s*(?:pub(?:\([^)]+\))?\s+)?mod\s+([A-Za-z_][\w]*)\b").unwrap();

    let mut texts: HashMap<PathBuf, String> = HashMap::new();
    for path in &sources.rust {
        if let Ok(text) = std::fs::read_to_string(path) {
            for cap in mod_re.captures_iter(&text) {
                if let Some(m) = cap.get(1) {
                    declared_mods.insert(m.as_str().to_string());
                }
            }
            texts.insert(path.clone(), text);
        }
    }

    let mut out = Vec::new();
    for path in &sources.rust {
        let file_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if file_name.is_empty() || file_name == "main" || file_name == "lib" || file_name == "mod" {
            continue;
        }
        // A `bin/*.rs` and `examples/*.rs` are entry points: skip.
        let display = path.display().to_string();
        if display.contains("/bin/") || display.contains("/examples/") || display.contains("/tests/") {
            continue;
        }
        // Only flag if no `mod <file_name>` appears anywhere.
        if !declared_mods.contains(&file_name) {
            out.push(Finding {
                category: Category::Dead,
                rule_id: "dead.orphaned_module".into(),
                title: format!("Orphaned rust module: {display}"),
                confidence: Confidence::Medium,
                evidence_strength: 0.8,
                files: vec![display.clone()],
                evidence: Evidence::OrphanedFile {
                    path: display.clone(),
                    locations: vec![Location {
                        file: display.clone(),
                        line: Some(1),
                    }],
                },
                next_action: format!(
                    "Add `mod {file_name};` to the parent mod.rs / lib.rs, or delete {display}."
                ),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn collect(root: &Path) -> SourceInventory {
        crate::debt::source_walk::collect(root)
    }

    #[test]
    fn flags_unreferenced_private_rust_fn() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("a.rs"),
            r#"
pub fn public_one() -> i32 { used_one() + 1 }

fn used_one() -> i32 { 42 }

fn never_called() -> i32 { 99 }
"#,
        )
        .unwrap();

        let inv = collect(root);
        let findings = run(root, &inv).unwrap();
        let titles: Vec<&str> = findings.iter().map(|f| f.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.contains("never_called")),
            "expected never_called to be flagged: {titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t.contains("used_one")),
            "used_one is called and must not be flagged: {titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t.contains("public_one")),
            "public fns are skipped: {titles:?}"
        );
    }

    #[test]
    fn flags_orphaned_rust_module() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "mod wired;\npub fn entry() {}",
        )
        .unwrap();
        fs::write(root.join("src/wired.rs"), "pub fn x() {}").unwrap();
        fs::write(root.join("src/orphan.rs"), "pub fn y() {}").unwrap();

        let inv = collect(root);
        let findings = run(root, &inv).unwrap();
        let orphans: Vec<&Finding> = findings
            .iter()
            .filter(|f| f.rule_id == "dead.orphaned_module")
            .collect();
        let titles: Vec<&str> = orphans.iter().map(|f| f.title.as_str()).collect();
        assert!(
            titles.iter().any(|t| t.contains("orphan.rs")),
            "expected orphan.rs to be flagged: {titles:?}"
        );
        assert!(
            !titles.iter().any(|t| t.contains("wired.rs")),
            "wired.rs is declared via mod wired;: {titles:?}"
        );
    }
}
