//! Exact-block duplicate detection.
//!
//! Strategy:
//! 1. Per file, strip comments and collapse runs of whitespace so that
//!    formatting differences don't mask duplicates.
//! 2. Tokenize into a stream of identifier / keyword / punctuation tokens.
//! 3. Slide a window of `WINDOW_TOKENS` tokens across each file, hash each
//!    window, and group matching hashes across all files.
//! 4. A cluster is a group with ≥ 2 occurrences across ≥ 2 distinct lines
//!    (same-file back-to-back windows would otherwise dominate).
//!
//! Normalization is intentionally lossy about identifier *values* but
//! preserves *shape*, so `fn foo(x: i32)` and `fn bar(y: i32)` still
//! collide. That matches the AI-code pattern we want to catch: models
//! copy-paste helpers with minor renames.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::debt::types::{Category, Confidence, Evidence, Finding, Location, SourceInventory};

const WINDOW_TOKENS: usize = 50;
const MIN_LINES: u32 = 8;

pub fn run(_project_dir: &Path, sources: &SourceInventory) -> Result<Vec<Finding>> {
    let mut per_file_tokens: Vec<(PathBuf, Vec<(u32, String)>)> = Vec::new();

    for path in sources.all() {
        if is_test_support_path(path) {
            continue;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let stripped = strip_comments(&text, &ext);
        let tokens = tokenize(&stripped);
        if tokens.len() >= WINDOW_TOKENS {
            per_file_tokens.push((path.clone(), tokens));
        }
    }

    // hash → list of (file, start_line, end_line)
    let mut buckets: HashMap<u64, Vec<Occurrence>> = HashMap::new();

    for (path, tokens) in &per_file_tokens {
        for (i, window) in tokens.windows(WINDOW_TOKENS).enumerate() {
            let hash = hash_window(window);
            let start_line = window.first().map(|(l, _)| *l).unwrap_or(0);
            let end_line = window.last().map(|(l, _)| *l).unwrap_or(0);
            buckets.entry(hash).or_default().push(Occurrence {
                file: path.clone(),
                start_line,
                end_line,
                window_idx: i,
            });
        }
    }

    // Turn clusters into findings. Overlapping sliding-window hashes
    // produce many near-duplicate clusters for the same actual block of
    // code; we collapse those below by claiming (file, start_line) pairs
    // greedily in strength order.
    let mut raw_clusters: Vec<Cluster> = Vec::new();

    for occs in buckets.values() {
        let distinct: Vec<&Occurrence> = distinct_positions(occs);
        if distinct.len() < 2 {
            continue;
        }
        let max_span = distinct
            .iter()
            .map(|o| o.end_line.saturating_sub(o.start_line) + 1)
            .max()
            .unwrap_or(0);
        if max_span < MIN_LINES {
            continue;
        }
        raw_clusters.push(Cluster {
            occurrences: distinct.into_iter().cloned().collect(),
            max_span,
        });
    }

    // Sort by span desc then occurrence count desc so the biggest duplicate
    // block claims its (file, line) pairs first.
    raw_clusters.sort_by(|a, b| {
        b.max_span
            .cmp(&a.max_span)
            .then_with(|| b.occurrences.len().cmp(&a.occurrences.len()))
    });

    let mut claimed: std::collections::HashSet<(PathBuf, u32)> = std::collections::HashSet::new();
    let mut findings = Vec::new();

    for cluster in raw_clusters {
        let mut fresh_occs: Vec<&Occurrence> = Vec::new();
        for o in &cluster.occurrences {
            // Consider a window "already covered" if any line inside its
            // span has been claimed by a larger earlier finding.
            let overlap = (o.start_line..=o.end_line)
                .any(|line| claimed.contains(&(o.file.clone(), line)));
            if !overlap {
                fresh_occs.push(o);
            }
        }
        if fresh_occs.len() < 2 {
            continue;
        }
        for o in &fresh_occs {
            for line in o.start_line..=o.end_line {
                claimed.insert((o.file.clone(), line));
            }
        }

        let distinct = fresh_occs;
        let max_span = cluster.max_span;

        let mut locations: Vec<Location> = distinct
            .iter()
            .map(|o| Location {
                file: o.file.display().to_string(),
                line: Some(o.start_line),
            })
            .collect();
        locations.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then_with(|| a.line.cmp(&b.line))
        });
        locations.dedup_by(|a, b| a.file == b.file && a.line == b.line);

        let files: Vec<String> = {
            let mut f: Vec<String> = locations.iter().map(|l| l.file.clone()).collect();
            f.sort();
            f.dedup();
            f
        };

        let snippet = snippet_for(distinct[0]);
        let occurrences = distinct.len() as u32;
        let evidence_strength = ((occurrences as f64) / 2.0).min(1.5);

        let title = if files.len() == 1 {
            format!(
                "Duplicate block ({occurrences} copies) in {}",
                files[0]
            )
        } else {
            format!(
                "Duplicate block ({occurrences} copies) across {} files",
                files.len()
            )
        };

        findings.push(Finding {
            category: Category::Dup,
            rule_id: "dup.exact_block".into(),
            title,
            confidence: Confidence::High,
            evidence_strength,
            files: files.clone(),
            evidence: Evidence::DuplicateCluster {
                snippet,
                normalized_lines: max_span,
                occurrences,
                locations,
            },
            next_action: "Extract a shared helper for the duplicated block and call it from each site.".into(),
        });
    }

    // Stable order for test snapshots.
    findings.sort_by(|a, b| b.evidence_strength.partial_cmp(&a.evidence_strength).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.title.cmp(&b.title)));

    Ok(findings)
}

#[derive(Debug, Clone)]
struct Occurrence {
    file: PathBuf,
    start_line: u32,
    end_line: u32,
    window_idx: usize,
}

#[derive(Debug, Clone)]
struct Cluster {
    occurrences: Vec<Occurrence>,
    max_span: u32,
}

fn distinct_positions(occs: &[Occurrence]) -> Vec<&Occurrence> {
    // Within the same file, prefer non-overlapping windows: consecutive
    // windows differ by a single token so they over-report.
    let mut grouped: HashMap<PathBuf, Vec<&Occurrence>> = HashMap::new();
    for o in occs {
        grouped.entry(o.file.clone()).or_default().push(o);
    }
    let mut out: Vec<&Occurrence> = Vec::new();
    for (_, mut list) in grouped {
        list.sort_by_key(|o| o.window_idx);
        let mut last_end: Option<usize> = None;
        for o in list {
            match last_end {
                Some(e) if o.window_idx < e + WINDOW_TOKENS => continue,
                _ => {
                    last_end = Some(o.window_idx);
                    out.push(o);
                }
            }
        }
    }
    out
}

fn hash_window(window: &[(u32, String)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (_, tok) in window {
        tok.hash(&mut h);
        0u8.hash(&mut h);
    }
    h.finish()
}

/// Tokenize `source` into (line_number, token) pairs. Identifiers are
/// normalized to `IDENT` so that near-duplicates with renamed variables
/// still collide. Numbers → `NUM`. String contents → `STR` (preserve
/// quote shape). Keywords are small so we keep them as-is.
fn tokenize(source: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0usize;
    let mut line: u32 = 1;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            b'"' | b'\'' | b'`' => {
                let quote = b;
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        if bytes[i + 1] == b'\n' {
                            line += 1;
                        }
                        i += 2;
                        continue;
                    }
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
                out.push((line, "STR".to_string()));
            }
            c if c.is_ascii_digit() => {
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'_') {
                    i += 1;
                }
                out.push((line, "NUM".to_string()));
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let tok = &source[start..i];
                if is_keyword(tok) {
                    out.push((line, tok.to_string()));
                } else {
                    out.push((line, "IDENT".to_string()));
                }
            }
            _ if !b.is_ascii() => {
                // Skip a full UTF-8 codepoint and emit it as a generic
                // unicode token so non-ASCII content doesn't break hashing.
                let width = utf8_width(b);
                i = (i + width).min(bytes.len());
                out.push((line, "UNI".to_string()));
            }
            _ => {
                // ASCII punctuation — safe to slice byte-wise.
                let start = i;
                i += 1;
                while i < bytes.len()
                    && bytes[i].is_ascii()
                    && !bytes[i].is_ascii_alphanumeric()
                    && bytes[i] != b'_'
                    && !matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n' | b'"' | b'\'' | b'`')
                {
                    if i - start >= 3 {
                        break;
                    }
                    i += 1;
                }
                out.push((line, source[start..i].to_string()));
            }
        }
    }
    out
}

fn is_test_support_path(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    s.contains("/tests/")
        || s.starts_with("tests/")
        || s.contains("/benches/")
        || s.contains("/examples/")
        || s.contains("/legacy/")
        || s.contains("/fixtures/")
}

fn utf8_width(first_byte: u8) -> usize {
    // `< 0xC0` covers both ASCII and stray continuation bytes — both should
    // advance a single byte (continuation bytes are invalid here but we
    // prefer forward progress over a stall).
    if first_byte < 0xC0 {
        1
    } else if first_byte < 0xE0 {
        2
    } else if first_byte < 0xF0 {
        3
    } else {
        4
    }
}

fn is_keyword(tok: &str) -> bool {
    matches!(
        tok,
        "fn" | "pub"
            | "use"
            | "mod"
            | "struct"
            | "enum"
            | "impl"
            | "trait"
            | "for"
            | "while"
            | "loop"
            | "if"
            | "else"
            | "match"
            | "return"
            | "let"
            | "mut"
            | "ref"
            | "as"
            | "in"
            | "self"
            | "Self"
            | "const"
            | "static"
            | "def"
            | "class"
            | "import"
            | "from"
            | "async"
            | "await"
            | "yield"
            | "try"
            | "except"
            | "raise"
            | "finally"
            | "with"
            | "lambda"
            | "and"
            | "or"
            | "not"
            | "is"
            | "None"
            | "True"
            | "False"
            | "function"
            | "var"
            | "interface"
            | "type"
            | "export"
            | "default"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "throw"
            | "catch"
            | "new"
    )
}

fn strip_comments(text: &str, ext: &str) -> String {
    match ext {
        "py" => strip_hash_and_triple_strings(text),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => strip_c_style(text),
        _ => text.to_string(),
    }
}

fn strip_hash_and_triple_strings(text: &str) -> String {
    // Only strip full-line `#` comments. Leave triple-string docstrings
    // in place — they contribute to the normalized hash, but tokenization
    // will collapse them to a single `STR` token anyway.
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            out.push('\n');
            continue;
        }
        // strip trailing # comment outside of strings (naive but ok here)
        let mut in_s = false;
        let mut quote = b'\0';
        let bytes = line.as_bytes();
        let mut cut = line.len();
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'"' | b'\'' => {
                    if !in_s {
                        in_s = true;
                        quote = b;
                    } else if b == quote {
                        in_s = false;
                    }
                }
                b'#' if !in_s => {
                    cut = i;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

fn strip_c_style(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut in_str: Option<u8> = None;

    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = in_str {
            out.push(b as char);
            if b == b'\\' && i + 1 < bytes.len() {
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            if b == q {
                in_str = None;
            }
            i += 1;
            continue;
        }

        // start-of-string detection
        if b == b'"' || b == b'\'' || b == b'`' {
            in_str = Some(b);
            out.push(b as char);
            i += 1;
            continue;
        }

        // line comment //
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // block comment /* */
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        out.push(b as char);
        i += 1;
    }

    out
}

fn snippet_for(occ: &Occurrence) -> String {
    let text = std::fs::read_to_string(&occ.file).unwrap_or_default();
    let start = occ.start_line.saturating_sub(1) as usize;
    let end = (occ.end_line as usize).min(text.lines().count());
    let picked: Vec<&str> = text.lines().skip(start).take(end - start).collect();
    let joined = picked.join("\n");
    if joined.len() > 400 {
        let mut s = joined.chars().take(400).collect::<String>();
        s.push('…');
        s
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn collect(dir: &Path) -> SourceInventory {
        crate::debt::source_walk::collect(dir)
    }

    #[test]
    fn finds_duplicate_across_files() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let block = r#"
fn compute(alpha: i32, beta: i32, gamma: i32) -> i32 {
    let sum_ab = alpha + beta;
    let sum_bc = beta + gamma;
    let product = sum_ab * sum_bc;
    let halved = product / 2;
    let doubled = halved * 2;
    let clamped = if doubled > 100 { 100 } else { doubled };
    let final_value = clamped + alpha;
    return final_value;
}
"#;
        fs::write(
            root.join("a.rs"),
            format!("// file a\npub mod a_mod {{ {block} }}"),
        )
        .unwrap();
        fs::write(
            root.join("b.rs"),
            format!("// file b\npub mod b_mod {{ {block} }}"),
        )
        .unwrap();

        let inv = collect(root);
        let findings = run(root, &inv).unwrap();
        assert!(
            !findings.is_empty(),
            "expected at least one duplicate cluster"
        );
        assert!(findings[0].title.to_lowercase().contains("duplicate"));
    }

    #[test]
    fn ignores_single_occurrence() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::write(
            root.join("a.rs"),
            r#"
fn only_me(x: i32) -> i32 {
    let a = x + 1;
    let b = a * 2;
    b - 3
}
"#,
        )
        .unwrap();
        let inv = collect(root);
        let findings = run(root, &inv).unwrap();
        assert!(findings.is_empty(), "single occurrence should not cluster");
    }

    #[test]
    fn tokenizer_normalizes_identifiers() {
        let a = tokenize("fn foo(x: i32) -> i32 { x + 1 }");
        let b = tokenize("fn bar(y: i32) -> i32 { y + 1 }");
        let a_toks: Vec<&str> = a.iter().map(|(_, t)| t.as_str()).collect();
        let b_toks: Vec<&str> = b.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(a_toks, b_toks, "identifier names must be normalized away");
    }
}
