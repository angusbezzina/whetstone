//! Churn × violation hotspot aggregation.
//!
//! A file is a hotspot if it both *changes often* (git churn within a
//! bounded window) and *carries outstanding rule/lint violations*. Either
//! signal alone is not interesting: high churn on a stable helper is fine,
//! and a single buggy file that never changes can be fixed in one patch.
//! The product of the two surfaces files where bugs keep returning.

use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

use crate::check::{self, CheckOptions};
use crate::debt::types::{Category, Confidence, Evidence, Finding, Location};

pub fn run(project_dir: &Path, since_days: u32) -> Result<Vec<Finding>> {
    // Skip silently if the project isn't a git repo — churn is undefined
    // there. This keeps `wh debt` usable outside of version-controlled trees.
    if !project_dir.join(".git").exists() {
        return Ok(Vec::new());
    }

    let churn = git_churn(project_dir, since_days).unwrap_or_default();
    if churn.is_empty() {
        return Ok(Vec::new());
    }

    let violations = collect_violations(project_dir);

    let mut out = Vec::new();
    for (file, changes) in &churn {
        let v = *violations.get(file).unwrap_or(&0);
        if v == 0 || *changes == 0 {
            continue;
        }
        let product = (*changes as f64) * (v as f64);
        if product < 4.0 {
            // Below the floor = noise.
            continue;
        }
        let strength = (product / 20.0).min(2.0);
        let confidence = if product >= 20.0 {
            Confidence::High
        } else {
            Confidence::Medium
        };

        out.push(Finding {
            category: Category::Hotspots,
            rule_id: "hotspots.churn_x_violations".into(),
            title: format!("Hotspot: {file} ({changes} changes × {v} violations)"),
            confidence,
            evidence_strength: strength,
            files: vec![file.clone()],
            evidence: Evidence::ChurnViolationIntersection {
                changes: *changes,
                violations: v,
                window_days: since_days,
                locations: vec![Location {
                    file: file.clone(),
                    line: None,
                }],
            },
            next_action: format!(
                "Stabilize {file}: address the {v} outstanding violations or factor the repeatedly-edited region into a smaller, better-tested unit."
            ),
        });
    }

    out.sort_by(|a, b| {
        b.evidence_strength
            .partial_cmp(&a.evidence_strength)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(out)
}

fn git_churn(project_dir: &Path, since_days: u32) -> Option<HashMap<String, u32>> {
    let output = Command::new("git")
        .current_dir(project_dir)
        .args([
            "log",
            &format!("--since={since_days}.days.ago"),
            "--name-only",
            "--pretty=format:",
            "--no-renames",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut counts: HashMap<String, u32> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Only count source-ish files; skip binaries/docs noise.
        if !is_source_like(line) {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    Some(counts)
}

fn is_source_like(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    for ext in [
        ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".go", ".java", ".rb",
    ] {
        if lower.ends_with(ext) {
            return true;
        }
    }
    false
}

fn collect_violations(project_dir: &Path) -> HashMap<String, u32> {
    // Run `wh check` on the project. If no rules are configured, the
    // violation map is empty — churn-only signals don't count as debt here.
    let scan_paths = vec![project_dir.to_path_buf()];
    let opts = CheckOptions {
        project_dir,
        scan_paths: &scan_paths,
        lang_filter: None,
        rule_filter: None,
    };
    let result = match check::run(opts) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let mut map: HashMap<String, u32> = HashMap::new();
    if let Some(violations) = result.get("violations").and_then(|v| v.as_array()) {
        for v in violations {
            let file = v
                .get("file")
                .and_then(|f| f.as_str())
                .unwrap_or("")
                .to_string();
            if file.is_empty() {
                continue;
            }
            // Normalize to repo-relative; `git log --name-only` already emits
            // repo-relative paths, so match that.
            let rel = file
                .strip_prefix(&format!("{}/", project_dir.display()))
                .unwrap_or(&file)
                .to_string();
            *map.entry(rel).or_insert(0) += 1;
        }
    }
    map
}
