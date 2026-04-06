//! Generate agent context files from approved Whetstone rules.
//!
//! Reads approved rules from whetstone/rules/**/*.yaml and generates agent context
//! files in multiple formats: CLAUDE.md, AGENTS.md, .cursorrules, copilot-instructions.md,
//! .windsurfrules, codex.md.

use anyhow::Result;
use serde_json::Value;
use std::path::Path;

use crate::config::WhetstoneConfig;
use crate::rules::{self, ApprovedRule};

/// Default formats if none specified in config.
const DEFAULT_FORMATS: &[&str] = &["agents.md"];

/// All supported format names.
const ALL_FORMATS: &[&str] = &[
    "claude.md",
    "agents.md",
    ".cursorrules",
    "copilot-instructions.md",
    ".windsurfrules",
    "codex.md",
];

pub fn generate_context(
    project_dir: &Path,
    formats_filter: Option<&str>,
    lang_filter: Option<&str>,
    dry_run: bool,
) -> Result<Value> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let output_dir = project_dir.join("whetstone").join("context");

    let (approved, warnings) = rules::load_approved_rules(&rules_dir, lang_filter);

    if approved.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": [],
            "warnings": ["No approved rules found. Run 'wh doctor' to extract and approve rules."],
            "next_command": "wh doctor",
        }));
    }

    // Determine which formats to generate
    let config = WhetstoneConfig::load(project_dir);
    let formats: Vec<String> = if let Some(filter) = formats_filter {
        filter.split(',').map(|s| s.trim().to_lowercase()).collect()
    } else if !config.generate.formats.is_empty() {
        config.generate.formats.clone()
    } else {
        DEFAULT_FORMATS.iter().map(|s| s.to_string()).collect()
    };

    // Validate format names
    for f in &formats {
        if !ALL_FORMATS.contains(&f.as_str()) {
            return Ok(serde_json::json!({
                "status": "error",
                "error": format!("Unknown format: '{f}'. Valid: {}", ALL_FORMATS.join(", ")),
            }));
        }
    }

    // Group rules by category for content generation
    let content = generate_rules_content(&approved);

    let mut generated = Vec::new();
    let mut skipped = Vec::new();

    for format in &formats {
        let (filename, file_content) = generate_format(format, &content, &approved);

        let output_path = output_dir.join(&filename);

        if dry_run {
            generated.push(serde_json::json!({
                "format": format,
                "path": output_path.display().to_string(),
                "lines": file_content.lines().count(),
                "dry_run": true,
            }));
        } else {
            if let Some(parent) = output_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::write(&output_path, &file_content) {
                Ok(()) => {
                    generated.push(serde_json::json!({
                        "format": format,
                        "path": output_path.display().to_string(),
                        "lines": file_content.lines().count(),
                    }));
                }
                Err(e) => {
                    skipped.push(format!("Failed to write {}: {e}", output_path.display()));
                }
            }
        }
    }

    // Collect dependency names
    let dep_names: Vec<&str> = {
        let mut names: Vec<&str> = approved.iter().map(|r| r.source_name.as_str()).collect();
        names.sort();
        names.dedup();
        names
    };

    let next_command = if generated.is_empty() {
        "wh doctor"
    } else {
        "wh tests"
    };

    Ok(serde_json::json!({
        "status": "ok",
        "generated": generated,
        "skipped": skipped,
        "rules_count": approved.len(),
        "dependencies": dep_names,
        "formats": formats,
        "warnings": warnings,
        "next_command": next_command,
    }))
}

struct RulesContent {
    use_rules: Vec<ContentRule>,
    avoid_rules: Vec<ContentRule>,
}

#[allow(dead_code)]
struct ContentRule {
    id: String,
    description: String,
    source_url: String,
    severity: String,
    category: String,
    source_name: String,
    pass_examples: Vec<String>,
    fail_examples: Vec<String>,
}

fn generate_rules_content(approved: &[ApprovedRule]) -> RulesContent {
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
            id: rule.id.clone(),
            description: rule.description.clone(),
            source_url: rule.source_url.clone(),
            severity: rule.severity.clone(),
            category: rule.category.clone(),
            source_name: rule.source_name.clone(),
            pass_examples,
            fail_examples,
        };

        // "migration" and "breaking-change" are avoid-rules; others are use-rules
        if matches!(rule.category.as_str(), "migration" | "breaking-change") {
            avoid_rules.push(cr);
        } else {
            use_rules.push(cr);
        }
    }

    RulesContent {
        use_rules,
        avoid_rules,
    }
}

fn generate_format(
    format: &str,
    content: &RulesContent,
    approved: &[ApprovedRule],
) -> (String, String) {
    let header = format_header(format);
    let body = format_rules_body(content);
    let dep_names: Vec<&str> = {
        let mut names: Vec<&str> = approved.iter().map(|r| r.source_name.as_str()).collect();
        names.sort();
        names.dedup();
        names
    };
    let footer = format_footer(format, &dep_names);

    let filename = match format {
        "claude.md" => "CLAUDE.md",
        "agents.md" => "AGENTS.md",
        ".cursorrules" => ".cursorrules",
        "copilot-instructions.md" => ".github/copilot-instructions.md",
        ".windsurfrules" => ".windsurfrules",
        "codex.md" => "codex.md",
        _ => "rules.md",
    };

    let full_content = format!("{header}\n\n{body}\n\n{footer}\n");
    (filename.to_string(), full_content)
}

fn format_header(format: &str) -> String {
    let preamble = match format {
        "claude.md" => "# Whetstone Rules\n\nThe following rules are derived from dependency documentation and should be followed.",
        "agents.md" => "# Whetstone Rules\n\nThese rules are extracted from dependency documentation by Whetstone. Follow them when writing code.",
        ".cursorrules" => "# Whetstone Rules\n\nFollow these dependency-specific rules when writing code:",
        "copilot-instructions.md" => "# Whetstone Rules\n\nThese rules are derived from dependency documentation:",
        ".windsurfrules" => "# Whetstone Rules\n\nFollow these dependency-specific rules:",
        "codex.md" => "# Whetstone Rules\n\nThese rules come from dependency documentation and must be followed:",
        _ => "# Whetstone Rules",
    };
    preamble.to_string()
}

fn format_rules_body(content: &RulesContent) -> String {
    let mut lines = Vec::new();

    if !content.use_rules.is_empty() {
        lines.push("## Do".to_string());
        lines.push(String::new());
        for rule in &content.use_rules {
            lines.push(format!("### {} ({})", rule.id, rule.severity));
            lines.push(String::new());
            lines.push(rule.description.clone());
            lines.push(String::new());

            if !rule.pass_examples.is_empty() {
                lines.push("**Good:**".to_string());
                for ex in &rule.pass_examples {
                    lines.push("```".to_string());
                    lines.push(ex.clone());
                    lines.push("```".to_string());
                }
                lines.push(String::new());
            }

            if !rule.fail_examples.is_empty() {
                lines.push("**Bad:**".to_string());
                for ex in &rule.fail_examples {
                    lines.push("```".to_string());
                    lines.push(ex.clone());
                    lines.push("```".to_string());
                }
                lines.push(String::new());
            }

            lines.push(format!("Source: {}", rule.source_url));
            lines.push(String::new());
        }
    }

    if !content.avoid_rules.is_empty() {
        lines.push("## Don't".to_string());
        lines.push(String::new());
        for rule in &content.avoid_rules {
            lines.push(format!("### {} ({})", rule.id, rule.severity));
            lines.push(String::new());
            lines.push(rule.description.clone());
            lines.push(String::new());

            if !rule.fail_examples.is_empty() {
                lines.push("**Avoid:**".to_string());
                for ex in &rule.fail_examples {
                    lines.push("```".to_string());
                    lines.push(ex.clone());
                    lines.push("```".to_string());
                }
                lines.push(String::new());
            }

            if !rule.pass_examples.is_empty() {
                lines.push("**Instead:**".to_string());
                for ex in &rule.pass_examples {
                    lines.push("```".to_string());
                    lines.push(ex.clone());
                    lines.push("```".to_string());
                }
                lines.push(String::new());
            }

            lines.push(format!("Source: {}", rule.source_url));
            lines.push(String::new());
        }
    }

    lines.join("\n")
}

fn format_footer(format: &str, dep_names: &[&str]) -> String {
    let deps_str = dep_names.join(", ");
    let timestamp = chrono::Utc::now().format("%Y-%m-%d").to_string();

    match format {
        "claude.md" | "agents.md" => {
            format!(
                "---\n\n*Generated by [Whetstone](https://github.com/angusbezzina/whetstone) on {timestamp} from: {deps_str}*"
            )
        }
        _ => {
            format!("<!-- Generated by Whetstone on {timestamp} from: {deps_str} -->")
        }
    }
}
