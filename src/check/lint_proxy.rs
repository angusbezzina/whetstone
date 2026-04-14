//! Verify `lint_proxy` signals against the project's linter config.
//!
//! A `lint_proxy` signal declares that an existing linter rule covers the
//! check (ruff E501, biome `suspicious/noExplicitAny`, etc.). `wh tests`
//! produces overlay configs that turn those rules on; this module walks
//! the project's primary linter config and reports any mapped rule that
//! is NOT enabled so the user knows enforcement is missing.
//!
//! Scope of linter support:
//! - **Ruff**: `ruff.toml`, `.ruff.toml`, `pyproject.toml` under
//!   `[tool.ruff.lint]` or `[tool.ruff]`, checking the `select =` list.
//! - **Biome**: `biome.json` / `biome.jsonc`, checking
//!   `linter.rules.<category>.<rule>` and the boolean `linter.enabled`.
//! - Clippy is deliberately skipped for now — its rule enablement is
//!   spread across `Cargo.toml` `[lints.clippy]`, `clippy.toml`, and
//!   in-source `#![warn(..)]` attributes, which is a larger investigation.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::rules::ApprovedRule;

pub fn verify_lint_proxies(project_dir: &Path, rules: &[&ApprovedRule]) -> Vec<Value> {
    let ruff = load_ruff_selects(project_dir);
    let biome = load_biome_enabled(project_dir);
    let mut issues: Vec<Value> = Vec::new();

    for rule in rules {
        for sig in &rule.signals {
            if sig.strategy != "lint_proxy" {
                continue;
            }
            for (linter, code) in parse_lint_codes(&sig.description) {
                let verdict = match linter.as_str() {
                    "ruff" => verify_ruff(&ruff, &code),
                    "biome" => verify_biome(&biome, &code),
                    _ => Verdict::Unsupported,
                };
                match verdict {
                    Verdict::Verified => continue,
                    Verdict::Missing => issues.push(json!({
                        "rule_id": rule.id,
                        "signal_id": sig.id,
                        "linter": linter,
                        "code": code,
                        "issue": "linter rule is not enabled in project config",
                        "fix": "run `wh tests` to generate the overlay config, or enable manually",
                        "config_files_checked": ruff.config_paths.iter().chain(biome.paths.iter()).map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    })),
                    Verdict::NoConfig => issues.push(json!({
                        "rule_id": rule.id,
                        "signal_id": sig.id,
                        "linter": linter,
                        "code": code,
                        "issue": "no linter config found to verify against",
                        "fix": "add ruff.toml / biome.json, or run `wh tests` for overlays",
                    })),
                    Verdict::Unsupported => {
                        // Silently skip unsupported linters (e.g. clippy until
                        // we support it); treating them as issues would create
                        // noise.
                    }
                }
            }
        }
    }
    issues
}

enum Verdict {
    Verified,
    Missing,
    NoConfig,
    Unsupported,
}

// ── Extraction of (linter, code) tuples from a signal description ──

/// Mine `"ruff E501"` / `"biome suspicious/noExplicitAny"` pairs out of a
/// signal description. Duplicates the logic used by `generate_tests.rs` so
/// both sides agree on what a lint_proxy references.
fn parse_lint_codes(description: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let parts: Vec<&str> = description.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        let normalized = part.to_ascii_lowercase();
        if matches!(normalized.as_str(), "ruff" | "biome" | "clippy") && i + 1 < parts.len() {
            out.push((normalized, parts[i + 1].trim_matches(&[',', '.', ';'][..]).to_string()));
        }
    }
    out
}

// ── Ruff ──

struct RuffConfig {
    config_paths: Vec<PathBuf>,
    selects: Vec<String>,
}

impl RuffConfig {
    fn has_code(&self, code: &str) -> bool {
        if self.selects.iter().any(|s| s.eq_ignore_ascii_case("ALL")) {
            return true;
        }
        self.selects.iter().any(|s| code_matches_ruff_select(s, code))
    }
}

fn load_ruff_selects(project_dir: &Path) -> RuffConfig {
    let candidates = [
        project_dir.join("ruff.toml"),
        project_dir.join(".ruff.toml"),
        project_dir.join("pyproject.toml"),
    ];
    let mut selects: Vec<String> = Vec::new();
    let mut config_paths: Vec<PathBuf> = Vec::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        config_paths.push(path.clone());
        if let Ok(text) = fs::read_to_string(path) {
            let parsed: toml::Value = match toml::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let root = if path.file_name().map(|f| f == "pyproject.toml").unwrap_or(false) {
                parsed
                    .get("tool")
                    .and_then(|t| t.get("ruff"))
                    .cloned()
                    .unwrap_or_else(|| toml::Value::Table(Default::default()))
            } else {
                parsed
            };
            if let Some(arr) = root
                .get("lint")
                .and_then(|l| l.get("select"))
                .or_else(|| root.get("select"))
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        selects.push(s.to_string());
                    }
                }
            }
            if let Some(arr) = root
                .get("lint")
                .and_then(|l| l.get("extend-select"))
                .or_else(|| root.get("extend-select"))
                .and_then(|v| v.as_array())
            {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        selects.push(s.to_string());
                    }
                }
            }
        }
    }
    RuffConfig {
        config_paths,
        selects,
    }
}

/// Ruff `select` entries are either exact codes (`E501`) or prefixes that
/// match a family (`E`, `B`, `B006`). A rule's code matches if any select
/// entry is a prefix of it.
fn code_matches_ruff_select(select: &str, code: &str) -> bool {
    let s = select.trim();
    code.eq_ignore_ascii_case(s) || code.to_ascii_uppercase().starts_with(&s.to_ascii_uppercase())
}

fn verify_ruff(cfg: &RuffConfig, code: &str) -> Verdict {
    if cfg.config_paths.is_empty() {
        return Verdict::NoConfig;
    }
    if cfg.has_code(code) {
        Verdict::Verified
    } else {
        Verdict::Missing
    }
}

// ── Biome ──

struct BiomeConfig {
    paths: Vec<PathBuf>,
    enabled: std::collections::BTreeSet<String>,
}

fn load_biome_enabled(project_dir: &Path) -> BiomeConfig {
    let candidates = [
        project_dir.join("biome.json"),
        project_dir.join("biome.jsonc"),
    ];
    let mut paths = Vec::new();
    let mut enabled = std::collections::BTreeSet::new();
    for path in &candidates {
        if !path.exists() {
            continue;
        }
        paths.push(path.clone());
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        // Strip JSONC comments defensively — biome.jsonc is allowed.
        let cleaned = strip_jsonc_comments(&text);
        let parsed: serde_json::Value = match serde_json::from_str(&cleaned) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let linter = parsed.get("linter");
        if linter.and_then(|l| l.get("enabled")).and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        if let Some(rules) = linter.and_then(|l| l.get("rules")).and_then(|r| r.as_object()) {
            for (category, body) in rules {
                if let Some(obj) = body.as_object() {
                    for (name, severity) in obj {
                        let active = match severity {
                            serde_json::Value::String(s) => {
                                matches!(s.as_str(), "error" | "warn" | "info")
                            }
                            serde_json::Value::Object(o) => o
                                .get("level")
                                .and_then(|v| v.as_str())
                                .map(|s| matches!(s, "error" | "warn" | "info"))
                                .unwrap_or(false),
                            _ => false,
                        };
                        if active {
                            enabled.insert(format!("{category}/{name}"));
                        }
                    }
                }
            }
        }
    }
    BiomeConfig { paths, enabled }
}

fn verify_biome(cfg: &BiomeConfig, code: &str) -> Verdict {
    if cfg.paths.is_empty() {
        return Verdict::NoConfig;
    }
    if cfg.enabled.contains(code) {
        Verdict::Verified
    } else {
        Verdict::Missing
    }
}

/// Minimal JSONC → JSON comment stripper: removes `//` line comments and
/// `/* */` block comments while preserving strings. Good enough for biome
/// configs, not a general-purpose JSONC parser.
fn strip_jsonc_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            out.push(c as char);
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'/' => {
                    i += 2;
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                    continue;
                }
                b'*' => {
                    i += 2;
                    while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                        i += 1;
                    }
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                _ => {}
            }
        }
        out.push(c as char);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruff_prefix_select_matches_subcode() {
        assert!(code_matches_ruff_select("E", "E501"));
        assert!(code_matches_ruff_select("B006", "B006"));
        assert!(!code_matches_ruff_select("F", "E501"));
    }

    #[test]
    fn parse_lint_codes_recognizes_ruff_and_biome() {
        let got = parse_lint_codes("Covered by ruff B006 and biome suspicious/noExplicitAny.");
        assert!(got.contains(&("ruff".into(), "B006".into())));
        assert!(got.contains(&("biome".into(), "suspicious/noExplicitAny".into())));
    }

    #[test]
    fn strip_jsonc_removes_comments_outside_strings() {
        let src = r#"{
            // trailing line
            "k": "has // inner",
            /* block */
            "v": 1
        }"#;
        let cleaned = strip_jsonc_comments(src);
        assert!(!cleaned.contains("trailing line"));
        assert!(cleaned.contains("has // inner"));
    }
}
