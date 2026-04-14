//! AI eval system: threshold gating, eval request generation, calibration.
//!
//! The binary does deterministic work (select code, apply thresholds, generate requests).
//! The agent does judgment (answer binary questions about ambiguous cases).
//! Communication is file-based: binary writes JSON requests, agent writes JSON verdicts.

use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use crate::rules::{self, ApprovedRule};

// ─── Types ───

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalRequest {
    pub id: String,
    pub rule_id: String,
    pub question: String,
    pub code_snippet: String,
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub golden_examples: Vec<GoldenEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoldenEntry {
    pub code: String,
    pub verdict: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalVerdict {
    pub id: String,
    pub verdict: String,
    pub reason: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize)]
pub struct ThresholdResult {
    pub rule_id: String,
    pub file_path: String,
    pub deterministic_hits: u32,
    pub total_deterministic: u32,
    pub outcome: String, // "auto_pass", "auto_fail", "ambiguous"
    pub matching_signals: Vec<String>,
}

// ─── Threshold Gating ───

#[allow(dead_code)]
/// Evaluate threshold gating for a rule against file content.
/// Returns auto_pass, auto_fail, or ambiguous based on how many
/// deterministic signals fire.
pub fn evaluate_thresholds(rule: &ApprovedRule, file_path: &str, content: &str) -> ThresholdResult {
    let deterministic_signals: Vec<&crate::rules::ApprovedSignal> = rule
        .signals
        .iter()
        .filter(|s| matches!(s.strategy.as_str(), "pattern" | "ast" | "lint_proxy"))
        .collect();

    let total = deterministic_signals.len() as u32;
    let mut hits = 0u32;
    let mut matching = Vec::new();

    for signal in &deterministic_signals {
        if let Some(ref pattern) = signal.match_pattern {
            if let Ok(re) = Regex::new(pattern) {
                if re.is_match(content) {
                    hits += 1;
                    matching.push(signal.id.clone());
                }
            }
        }
    }

    let pass_threshold = rule.deterministic_pass_threshold.unwrap_or(total);
    let fail_threshold = rule.deterministic_fail_threshold.unwrap_or(0);

    let outcome = if hits >= pass_threshold && pass_threshold > 0 {
        "auto_pass"
    } else if hits <= fail_threshold {
        "auto_fail"
    } else {
        "ambiguous"
    };

    ThresholdResult {
        rule_id: rule.id.clone(),
        file_path: file_path.to_string(),
        deterministic_hits: hits,
        total_deterministic: total,
        outcome: outcome.to_string(),
        matching_signals: matching,
    }
}

// ─── Eval Definition Generation ───

/// Generate AI eval definition YAML files for rules with ai signals or ai_eval config.
pub fn generate_eval_definitions(
    project_dir: &Path,
    lang_filter: Option<&str>,
    dry_run: bool,
) -> Result<Value> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let output_dir = project_dir.join("whetstone").join("evals").join("ai");

    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();
    let approved = if whetstone_config_exists {
        crate::layers::resolve_merged(project_dir, lang_filter, true, true, false)
            .merged
            .into_iter()
            .map(|lr| lr.rule)
            .collect()
    } else {
        let (approved, _) = rules::load_approved_rules(&rules_dir, lang_filter);
        approved
    };

    // Filter to rules with ai signals or ai_eval
    let eval_rules: Vec<&ApprovedRule> = approved
        .iter()
        .filter(|r| r.ai_eval.is_some() || r.signals.iter().any(|s| s.strategy == "ai"))
        .collect();

    if eval_rules.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "generated": [],
            "message": "No rules with AI signals found. All rules use deterministic signals only.",
            "next_command": "wh eval run --deterministic-only",
        }));
    }

    if !dry_run {
        std::fs::create_dir_all(&output_dir)?;
    }

    let mut generated = Vec::new();

    for rule in &eval_rules {
        let ai_eval = rule.ai_eval.as_ref();
        let question = ai_eval
            .map(|e| e.question.clone())
            .filter(|q| !q.is_empty())
            .unwrap_or_else(|| {
                format!(
                    "Does this code follow the rule: {}? Answer PASS or FAIL with a one-line reason.",
                    rule.description
                )
            });

        let context_lines = ai_eval.and_then(|e| e.context_lines).unwrap_or(10);

        // Build pre-filter from deterministic pattern signals
        let pre_filter: Option<Value> = rule
            .signals
            .iter()
            .find(|s| s.strategy == "pattern" && s.match_pattern.is_some())
            .map(|s| {
                serde_json::json!({
                    "strategy": "pattern",
                    "match": s.match_pattern.as_ref().unwrap(),
                })
            });

        let golden: Vec<Value> = rule
            .golden_examples
            .iter()
            .map(|e| {
                serde_json::json!({
                    "code": e.code,
                    "verdict": e.verdict,
                    "reason": e.reason,
                })
            })
            .collect();

        let definition = serde_json::json!({
            "rule_id": rule.id,
            "language": rule.language,
            "question": question,
            "context_lines": context_lines,
            "trigger": ai_eval.map(|e| e.trigger.as_str()).unwrap_or("ambiguous"),
            "pre_filter": pre_filter,
            "golden_examples": golden,
        });

        let filename = format!("{}.yaml", rule.id.replace('.', "_"));
        let path = output_dir.join(&filename);

        if !dry_run {
            let yaml = serde_yaml::to_string(&definition)?;
            std::fs::write(&path, yaml)?;
        }

        generated.push(serde_json::json!({
            "rule_id": rule.id,
            "path": path.display().to_string(),
            "golden_examples": golden.len(),
        }));
    }

    Ok(serde_json::json!({
        "status": "ok",
        "generated": generated,
        "eval_count": generated.len(),
        "next_command": "wh eval run",
    }))
}

// ─── Eval Runner ───

/// Run evaluations: apply threshold gating, generate AI requests for ambiguous cases.
pub fn run_evals(
    project_dir: &Path,
    lang_filter: Option<&str>,
    collect: bool,
    deterministic_only: bool,
) -> Result<Value> {
    if collect {
        return collect_verdicts(project_dir);
    }

    let rules_dir = project_dir.join("whetstone").join("rules");
    let state_dir = project_dir.join("whetstone").join(".state");

    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();
    let approved = if whetstone_config_exists {
        crate::layers::resolve_merged(project_dir, lang_filter, true, true, false)
            .merged
            .into_iter()
            .map(|lr| lr.rule)
            .collect()
    } else {
        let (approved, _) = rules::load_approved_rules(&rules_dir, lang_filter);
        approved
    };

    // Walk source files
    let src_dir = project_dir.join("src");
    let source_files = find_source_files(&src_dir);

    let mut files_clean = 0u32;
    let mut files_with_violations = 0u32;
    let mut violations = Vec::new();
    let mut eval_requests = Vec::new();

    for rule in &approved {
        // Compile all pattern regexes for this rule
        let patterns: Vec<(&crate::rules::ApprovedSignal, Regex)> = rule
            .signals
            .iter()
            .filter(|s| s.match_pattern.is_some())
            .filter_map(|s| {
                let re = Regex::new(s.match_pattern.as_ref()?).ok()?;
                Some((s, re))
            })
            .collect();

        if patterns.is_empty() {
            continue; // No checkable signals for this rule
        }

        for file_path in &source_files {
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let rel_path = file_path
                .strip_prefix(project_dir)
                .unwrap_or(file_path)
                .to_string_lossy()
                .to_string();

            let mut file_violations = Vec::new();

            // Check each pattern signal against the file
            for (signal, re) in &patterns {
                for (i, line) in content.lines().enumerate() {
                    if re.is_match(line) {
                        file_violations.push(serde_json::json!({
                            "rule_id": rule.id,
                            "signal_id": signal.id,
                            "file": rel_path,
                            "line": i + 1,
                            "code": line.trim(),
                            "severity": rule.severity,
                        }));
                    }
                }
            }

            if file_violations.is_empty() {
                files_clean += 1;
            } else {
                files_with_violations += 1;

                // If rule has ai_eval and we're not deterministic-only,
                // route violations through AI for judgment
                if !deterministic_only {
                    if let Some(ref ai_eval) = rule.ai_eval {
                        let ctx_lines = ai_eval.context_lines.unwrap_or(10) as usize;
                        for v in &file_violations {
                            let line_num = v["line"].as_u64().unwrap_or(1) as usize;
                            let start = line_num.saturating_sub(1 + ctx_lines / 2);
                            let end = (line_num + ctx_lines / 2).min(content.lines().count());
                            let snippet: String = content
                                .lines()
                                .skip(start)
                                .take(end - start)
                                .collect::<Vec<_>>()
                                .join("\n");

                            eval_requests.push(EvalRequest {
                                id: format!("{}:{}:{}", rule.id, rel_path, line_num),
                                rule_id: rule.id.clone(),
                                question: ai_eval.question.clone(),
                                code_snippet: snippet,
                                file_path: rel_path.clone(),
                                line_start: start + 1,
                                line_end: end,
                                golden_examples: rule
                                    .golden_examples
                                    .iter()
                                    .map(|e| GoldenEntry {
                                        code: e.code.clone(),
                                        verdict: e.verdict.clone(),
                                        reason: Some(e.reason.clone()),
                                    })
                                    .collect(),
                            });
                        }
                        continue; // Don't add to violations yet — AI will judge
                    }
                }

                violations.extend(file_violations);
            }
        }
    }

    // Write eval requests if any
    if !eval_requests.is_empty() {
        std::fs::create_dir_all(&state_dir)?;
        let batch = serde_json::json!({
            "version": 1,
            "generated_at": chrono::Utc::now().to_rfc3339(),
            "instructions": "For each request, answer the question about the code snippet. Respond with PASS or FAIL and a one-line reason. Write your verdicts to whetstone/.state/eval-verdicts.json.",
            "response_format": {
                "version": 1,
                "judged_at": "<ISO 8601>",
                "verdicts": [{"id": "<request id>", "verdict": "pass|fail", "reason": "<one line>"}]
            },
            "requests": eval_requests,
        });
        let requests_path = state_dir.join("eval-requests.json");
        std::fs::write(&requests_path, serde_json::to_string_pretty(&batch)?)?;
    }

    Ok(serde_json::json!({
        "status": "ok",
        "summary": {
            "rules_evaluated": approved.len(),
            "files_scanned": source_files.len(),
            "files_clean": files_clean,
            "files_with_violations": files_with_violations,
            "deterministic_violations": violations.len(),
            "pending_ai_review": eval_requests.len(),
        },
        "violations": violations,
        "pending_requests": if eval_requests.is_empty() { Value::Null } else {
            serde_json::json!({
                "count": eval_requests.len(),
                "path": "whetstone/.state/eval-requests.json",
                "instructions": "Agent: read eval-requests.json, judge each snippet, write verdicts to eval-verdicts.json. Then run 'wh eval run --collect'.",
            })
        },
        "next_command": if eval_requests.is_empty() { "wh status" } else { "Agent judges eval-requests.json, then: wh eval run --collect" },
    }))
}

/// Collect verdicts from the agent and produce the final report.
fn collect_verdicts(project_dir: &Path) -> Result<Value> {
    let state_dir = project_dir.join("whetstone").join(".state");
    let verdicts_path = state_dir.join("eval-verdicts.json");

    if !verdicts_path.exists() {
        return Ok(serde_json::json!({
            "status": "error",
            "error": "No eval-verdicts.json found. The agent must judge the requests first.",
            "next_command": "Agent reads whetstone/.state/eval-requests.json and writes eval-verdicts.json",
        }));
    }

    let verdicts_text = std::fs::read_to_string(&verdicts_path)?;
    let verdicts_data: Value = serde_json::from_str(&verdicts_text)?;

    let verdicts: Vec<EvalVerdict> = verdicts_data
        .get("verdicts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    let mut ai_violations = Vec::new();
    let mut ai_passes = 0u32;

    for verdict in &verdicts {
        if verdict.verdict.to_lowercase() == "fail" {
            ai_violations.push(serde_json::json!({
                "id": verdict.id,
                "verdict": "fail",
                "reason": verdict.reason,
                "outcome": "ai_fail",
            }));
        } else {
            ai_passes += 1;
        }
    }

    Ok(serde_json::json!({
        "status": "ok",
        "summary": {
            "verdicts_collected": verdicts.len(),
            "ai_passes": ai_passes,
            "ai_failures": ai_violations.len(),
        },
        "ai_violations": ai_violations,
        "next_command": "wh status",
    }))
}

// ─── Calibration ───

/// Calibrate AI eval prompts against golden examples.
pub fn calibrate(project_dir: &Path, lang_filter: Option<&str>, collect: bool) -> Result<Value> {
    let state_dir = project_dir.join("whetstone").join(".state");

    if collect {
        return collect_calibration(project_dir);
    }

    let rules_dir = project_dir.join("whetstone").join("rules");
    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();
    let approved = if whetstone_config_exists {
        crate::layers::resolve_merged(project_dir, lang_filter, true, true, false)
            .merged
            .into_iter()
            .map(|lr| lr.rule)
            .collect()
    } else {
        let (approved, _) = rules::load_approved_rules(&rules_dir, lang_filter);
        approved
    };

    // Filter to rules with ai_eval
    let eval_rules: Vec<&ApprovedRule> = approved.iter().filter(|r| r.ai_eval.is_some()).collect();

    if eval_rules.is_empty() {
        return Ok(serde_json::json!({
            "status": "ok",
            "message": "No rules with ai_eval config found. Nothing to calibrate.",
            "next_command": "wh status",
        }));
    }

    let mut requests = Vec::new();

    for rule in &eval_rules {
        let ai_eval = rule.ai_eval.as_ref().unwrap();
        for (i, example) in rule.golden_examples.iter().enumerate() {
            requests.push(EvalRequest {
                id: format!("calibrate:{}:example_{}", rule.id, i),
                rule_id: rule.id.clone(),
                question: ai_eval.question.clone(),
                code_snippet: example.code.clone(),
                file_path: format!("golden_example_{}", i),
                line_start: 0,
                line_end: 0,
                golden_examples: rule
                    .golden_examples
                    .iter()
                    .map(|e| GoldenEntry {
                        code: e.code.clone(),
                        verdict: e.verdict.clone(),
                        reason: Some(e.reason.clone()),
                    })
                    .collect(),
            });
        }
    }

    std::fs::create_dir_all(&state_dir)?;
    let batch = serde_json::json!({
        "version": 1,
        "type": "calibration",
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "instructions": "Judge each golden example as if it were real code. Do NOT look at the golden_examples for the answer — judge independently. Write verdicts to whetstone/.state/calibration-verdicts.json.",
        "requests": requests,
    });

    let path = state_dir.join("calibration-requests.json");
    std::fs::write(&path, serde_json::to_string_pretty(&batch)?)?;

    Ok(serde_json::json!({
        "status": "ok",
        "calibration_requests": requests.len(),
        "rules_tested": eval_rules.len(),
        "path": path.display().to_string(),
        "next_command": "Agent judges calibration-requests.json, then: wh eval calibrate --collect",
    }))
}

fn collect_calibration(project_dir: &Path) -> Result<Value> {
    let state_dir = project_dir.join("whetstone").join(".state");
    let verdicts_path = state_dir.join("calibration-verdicts.json");

    if !verdicts_path.exists() {
        return Ok(serde_json::json!({
            "status": "error",
            "error": "No calibration-verdicts.json found.",
            "next_command": "Agent judges calibration-requests.json first",
        }));
    }

    let verdicts_text = std::fs::read_to_string(&verdicts_path)?;
    let verdicts_data: Value = serde_json::from_str(&verdicts_text)?;
    let verdicts: Vec<EvalVerdict> = verdicts_data
        .get("verdicts")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| serde_json::from_value(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default();

    // Load golden examples to compare
    let rules_dir = project_dir.join("whetstone").join("rules");
    let whetstone_config_exists = project_dir
        .join("whetstone")
        .join("whetstone.yaml")
        .exists()
        || project_dir.join("whetstone.yaml").exists();
    let approved = if whetstone_config_exists {
        crate::layers::resolve_merged(project_dir, None, true, true, false)
            .merged
            .into_iter()
            .map(|lr| lr.rule)
            .collect()
    } else {
        let (approved, _) = rules::load_approved_rules(&rules_dir, None);
        approved
    };

    let mut golden_map: HashMap<String, String> = HashMap::new();
    for rule in &approved {
        if rule.ai_eval.is_some() {
            for (i, example) in rule.golden_examples.iter().enumerate() {
                let key = format!("calibrate:{}:example_{}", rule.id, i);
                golden_map.insert(key, example.verdict.to_lowercase());
            }
        }
    }

    let mut agreements = 0u32;
    let mut disagreements = Vec::new();
    let total = verdicts.len() as u32;

    for verdict in &verdicts {
        if let Some(expected) = golden_map.get(&verdict.id) {
            if verdict.verdict.to_lowercase() == *expected {
                agreements += 1;
            } else {
                disagreements.push(serde_json::json!({
                    "id": verdict.id,
                    "expected": expected,
                    "got": verdict.verdict,
                    "reason": verdict.reason,
                }));
            }
        }
    }

    let agreement_rate = if total > 0 {
        (agreements as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "status": "ok",
        "summary": {
            "total_examples": total,
            "agreements": agreements,
            "disagreements": disagreements.len(),
            "agreement_rate": format!("{:.1}%", agreement_rate),
        },
        "disagreements": disagreements,
        "calibration_passed": disagreements.is_empty(),
        "next_command": if disagreements.is_empty() { "wh eval run" } else { "Review and fix ai_eval prompts for disagreeing rules" },
    }))
}

// ─── Helpers ───

fn find_source_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    if !dir.exists() {
        return files;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if !matches!(
                    name.as_ref(),
                    "target" | ".git" | "whetstone" | "node_modules" | "__pycache__"
                ) {
                    files.extend(find_source_files(&path));
                }
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if matches!(ext, "rs" | "py" | "ts" | "tsx" | "js" | "jsx") {
                    files.push(path);
                }
            }
        }
    }
    files
}
