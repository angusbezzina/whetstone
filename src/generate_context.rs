//! Generate agent context files from approved Whetstone rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml and renders Tera
//! templates for each requested format (CLAUDE.md, AGENTS.md, .cursorrules,
//! copilot-instructions.md, .windsurfrules, codex.md).

use anyhow::Result;
use serde_json::Value;
use std::path::Path;
use tera::Context;

use crate::config::WhetstoneConfig;
use crate::rules::{self, ApprovedRule};
use crate::templates::{build_tera, render};

/// Default formats if none specified in config.
const DEFAULT_FORMATS: &[&str] = &["agents.md"];

/// All supported format names and the template + filename they resolve to.
const FORMAT_SPECS: &[(&str, &str, &str)] = &[
    ("claude.md", "claude_md.tera", "CLAUDE.md"),
    ("agents.md", "agents_md.tera", "AGENTS.md"),
    (".cursorrules", "cursorrules.tera", ".cursorrules"),
    (
        "copilot-instructions.md",
        "copilot_md.tera",
        ".github/copilot-instructions.md",
    ),
    (".windsurfrules", "windsurfrules.tera", ".windsurfrules"),
    ("codex.md", "codex_md.tera", "codex.md"),
];

pub fn generate_context(
    project_dir: &Path,
    formats_filter: Option<&str>,
    lang_filter: Option<&str>,
    dry_run: bool,
    personal_output: bool,
    terse: bool,
) -> Result<Value> {
    let project_initialized = crate::layers::project_is_initialized(project_dir);
    let config = WhetstoneConfig::load(project_dir);
    let paths = crate::layers::LayerPaths::for_project(project_dir);

    let (approved, output_dir, warnings): (Vec<ApprovedRule>, _, Vec<String>) = if personal_output {
        let (rules, warns) = crate::layers::load_personal_only(project_dir, lang_filter);
        (rules, paths.personal_context(), warns)
    } else if project_initialized {
        let merged = crate::layers::resolve_merged(project_dir, lang_filter, true, false, false);
        let approved = merged.merged.into_iter().map(|lr| lr.rule).collect();
        (
            approved,
            paths.whetstone_dir.join("context"),
            merged.warnings,
        )
    } else {
        let (approved, warns) = rules::load_approved_rules(&paths.project_rules_dir, lang_filter);
        (approved, paths.whetstone_dir.join("context"), warns)
    };

    if approved.is_empty() {
        // Check whether any personal rules exist — if so, tell the user how
        // to include them rather than silently emitting nothing.
        let (personal_rules, _) = crate::layers::load_personal_only(project_dir, lang_filter);
        let hint = if !personal_rules.is_empty() && !personal_output {
            format!(
                "No project rules found, but {} personal rule(s) exist. Run `wh context --personal` to render them into `whetstone/.personal/context/` (gitignored).",
                personal_rules.len()
            )
        } else {
            "No approved rules found. Run 'wh init' to extract and approve rules.".to_string()
        };
        let next = if !personal_rules.is_empty() && !personal_output {
            "wh context --personal"
        } else {
            "wh init"
        };
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": [],
            "warnings": [hint],
            "next_command": next,
        }));
    }

    let formats: Vec<String> = if let Some(filter) = formats_filter {
        filter.split(',').map(|s| s.trim().to_lowercase()).collect()
    } else if !config.generate.formats.is_empty() {
        config.generate.formats.clone()
    } else {
        DEFAULT_FORMATS.iter().map(|s| s.to_string()).collect()
    };

    for f in &formats {
        if FORMAT_SPECS.iter().all(|(name, _, _)| *name != f.as_str()) {
            let valid: Vec<&str> = FORMAT_SPECS.iter().map(|(n, _, _)| *n).collect();
            return Ok(serde_json::json!({
                "status": "error",
                "error": format!("Unknown format: '{f}'. Valid: {}", valid.join(", ")),
            }));
        }
    }

    let tera = build_tera();
    let (use_rules, avoid_rules) = split_rules(&approved);
    let deps = dedup_sorted_deps(&approved);
    let timestamp = chrono::Utc::now().format("%Y-%m-%d").to_string();

    let mut ctx = Context::new();
    ctx.insert("use_rules", &use_rules);
    ctx.insert("avoid_rules", &avoid_rules);
    ctx.insert("deps", &deps);
    ctx.insert("timestamp", &timestamp);
    ctx.insert("terse", &terse);
    ctx.insert("sidecar_languages", &collect_languages(&approved));

    let mut generated = Vec::new();
    let mut skipped = Vec::new();

    for format in &formats {
        let (template, filename) = lookup_format(format).unwrap();
        let rendered = render(&tera, template, &ctx);
        let output_path = output_dir.join(filename);

        if dry_run {
            generated.push(serde_json::json!({
                "format": format,
                "path": output_path.display().to_string(),
                "lines": rendered.lines().count(),
                "dry_run": true,
            }));
            continue;
        }

        if let Some(parent) = output_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(&output_path, &rendered) {
            Ok(()) => {
                generated.push(serde_json::json!({
                    "format": format,
                    "path": output_path.display().to_string(),
                    "lines": rendered.lines().count(),
                }));
            }
            Err(e) => {
                skipped.push(format!("Failed to write {}: {e}", output_path.display()));
            }
        }
    }

    // Per-language AGENTS.<lang>.md sidecars when >1 language has rules.
    // Tools with per-language hooks can point at the narrower file.
    // Always emits the agents.md template; only runs when agents.md is in the
    // requested formats and at least two languages are present.
    if formats.iter().any(|f| f == "agents.md") {
        let languages = collect_languages(&approved);
        if languages.len() > 1 {
            for lang in &languages {
                let lang_rules: Vec<ApprovedRule> = approved
                    .iter()
                    .filter(|r| &r.language == lang)
                    .cloned()
                    .collect();
                if lang_rules.is_empty() {
                    continue;
                }
                let (lang_use, lang_avoid) = split_rules(&lang_rules);
                let lang_deps = dedup_sorted_deps(&lang_rules);

                let siblings: Vec<String> = languages
                    .iter()
                    .filter(|l| *l != lang)
                    .cloned()
                    .collect();

                let mut lang_ctx = Context::new();
                lang_ctx.insert("use_rules", &lang_use);
                lang_ctx.insert("avoid_rules", &lang_avoid);
                lang_ctx.insert("deps", &lang_deps);
                lang_ctx.insert("timestamp", &timestamp);
                lang_ctx.insert("terse", &terse);
                lang_ctx.insert("sidecar_language", lang);
                // Sibling sidecars so agents loading AGENTS.<lang>.md can discover the others.
                lang_ctx.insert("sidecar_siblings", &siblings);

                let rendered = render(&tera, "agents_md.tera", &lang_ctx);
                let filename = format!("AGENTS.{lang}.md");
                let output_path = output_dir.join(&filename);

                if dry_run {
                    generated.push(serde_json::json!({
                        "format": "agents.md",
                        "language": lang,
                        "path": output_path.display().to_string(),
                        "lines": rendered.lines().count(),
                        "dry_run": true,
                    }));
                    continue;
                }
                if let Some(parent) = output_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&output_path, &rendered) {
                    Ok(()) => {
                        generated.push(serde_json::json!({
                            "format": "agents.md",
                            "language": lang,
                            "path": output_path.display().to_string(),
                            "lines": rendered.lines().count(),
                        }));
                    }
                    Err(e) => {
                        skipped.push(format!(
                            "Failed to write {}: {e}",
                            output_path.display()
                        ));
                    }
                }
            }
        }
    }

    let next_command = if generated.is_empty() {
        "wh init"
    } else {
        "wh tests"
    };

    Ok(serde_json::json!({
        "status": "ok",
        "generated": generated,
        "skipped": skipped,
        "rules_count": approved.len(),
        "dependencies": deps,
        "formats": formats,
        "warnings": warnings,
        "next_command": next_command,
    }))
}

fn lookup_format(name: &str) -> Option<(&'static str, &'static str)> {
    FORMAT_SPECS
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, tpl, file)| (*tpl, *file))
}

#[derive(serde::Serialize)]
struct ContentRule {
    id: String,
    description: String,
    short_desc: String,
    source_url: String,
    severity: String,
    category: String,
    source_name: String,
    language: String,
    pass_examples: Vec<String>,
    fail_examples: Vec<String>,
}

/// Strip newlines, collapse whitespace, truncate to `max` chars on a word
/// boundary (falls back to hard truncation if no space fits).
/// Used by the terse template to emit one-line rule summaries.
fn short_description(desc: &str, max: usize) -> String {
    let flat: String = desc
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    // Budget: leave room for the ellipsis.
    let budget = max.saturating_sub(1);
    // Find the last whitespace at or before the budget; truncate there.
    let truncated: String = flat.chars().take(budget).collect();
    let cut_at = truncated
        .rfind(' ')
        .filter(|idx| *idx > budget / 2); // avoid chopping off half the sentence
    let base = match cut_at {
        Some(idx) => truncated[..idx].trim_end().to_string(),
        None => truncated,
    };
    format!("{base}…")
}

fn split_rules(approved: &[ApprovedRule]) -> (Vec<ContentRule>, Vec<ContentRule>) {
    let mut use_rules = Vec::new();
    let mut avoid_rules = Vec::new();

    for rule in approved {
        let pass_examples: Vec<String> = rule
            .golden_examples
            .iter()
            .filter(|e| e.verdict == "pass")
            .map(|e| e.code.clone())
            .collect();
        let fail_examples: Vec<String> = rule
            .golden_examples
            .iter()
            .filter(|e| e.verdict == "fail")
            .map(|e| e.code.clone())
            .collect();

        let cr = ContentRule {
            short_desc: short_description(&rule.description, 120),
            id: rule.id.clone(),
            description: rule.description.clone(),
            source_url: rule.source_url.clone(),
            severity: rule.severity.clone(),
            category: rule.category.clone(),
            source_name: rule.source_name.clone(),
            language: rule.language.clone(),
            pass_examples,
            fail_examples,
        };

        if matches!(rule.category.as_str(), "migration" | "breaking-change") {
            avoid_rules.push(cr);
        } else {
            use_rules.push(cr);
        }
    }

    (use_rules, avoid_rules)
}

fn collect_languages(approved: &[ApprovedRule]) -> Vec<String> {
    let mut langs: Vec<String> = approved.iter().map(|r| r.language.clone()).collect();
    langs.sort();
    langs.dedup();
    langs
}

fn dedup_sorted_deps(approved: &[ApprovedRule]) -> Vec<String> {
    let mut names: Vec<String> = approved.iter().map(|r| r.source_name.clone()).collect();
    names.sort();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::short_description;

    #[test]
    fn short_description_word_boundary() {
        let s = short_description(
            "Use an actively maintained alternative such as serde-yml.",
            40,
        );
        assert!(s.ends_with("…"), "got {s}");
        assert!(!s.contains(" se…"), "should not end mid-word: {s}");
    }

    #[test]
    fn short_description_preserves_short_input() {
        let s = short_description("Short description.", 120);
        assert_eq!(s, "Short description.");
    }

    #[test]
    fn short_description_handles_no_whitespace() {
        let s = short_description("abcdefghij", 5);
        assert!(s.ends_with("…"));
    }
}
