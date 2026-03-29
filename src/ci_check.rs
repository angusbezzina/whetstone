use anyhow::Result;
use serde_json::Value;
use std::path::Path;
use std::time::Instant;

use crate::status;

pub fn ci_check(project_dir: &Path, check_drift: bool, changed_only: bool) -> Result<Value> {
    let start = Instant::now();
    let effective_drift = check_drift || changed_only;

    let status_result = status::compute_status(project_dir, effective_drift, changed_only)?;

    if status_result.get("status").and_then(|v| v.as_str()) == Some("not_initialized") {
        return Ok(serde_json::json!({
            "freshness_status": "not_initialized",
            "changed_sources_count": 0,
            "recommended_rules_count": 0,
            "requires_review": false,
            "score": 0,
            "label": "Not Initialized",
            "message": "Whetstone not initialized in this project.",
            "elapsed_seconds": (start.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        }));
    }

    let label = status_result
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let score = status_result
        .get("score")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let dims = status_result
        .get("dimensions")
        .cloned()
        .unwrap_or(Value::Null);
    let recommendations = status_result
        .get("recommendations")
        .cloned()
        .unwrap_or(serde_json::json!([]));
    let pending_updates = dims
        .get("pending_updates")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    let freshness_status = match label {
        "Healthy" => "healthy",
        "Needs Review" => "needs_review",
        "Stale" => "stale",
        "No Rules" => "no_rules",
        _ => "unknown",
    };

    let requires_review = matches!(freshness_status, "stale" | "needs_review");
    let elapsed = (start.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    let rec_count = recommendations.as_array().map(|a| a.len()).unwrap_or(0);

    Ok(serde_json::json!({
        "freshness_status": freshness_status,
        "changed_sources_count": pending_updates,
        "recommended_rules_count": rec_count,
        "requires_review": requires_review,
        "score": score,
        "label": label,
        "dimensions": dims,
        "recommendations": recommendations,
        "elapsed_seconds": elapsed,
        "next_command": status_result.get("next_command"),
    }))
}

pub fn format_pr_comment(result: &Value) -> String {
    let marker = "<!-- whetstone-ci-check -->";
    let status_emoji = match result
        .get("freshness_status")
        .and_then(|v| v.as_str())
        .unwrap_or("")
    {
        "healthy" => "OK",
        "needs_review" => "!!",
        "stale" => "XX",
        "no_rules" | "not_initialized" => "--",
        _ => "??",
    };

    let label = result
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown");
    let score = result.get("score").and_then(|v| v.as_i64()).unwrap_or(0);

    let mut lines = vec![
        marker.to_string(),
        "## Whetstone Status".to_string(),
        String::new(),
        format!("**[{}] {}** (score: {}/100)", status_emoji, label, score),
        String::new(),
    ];

    if let Some(dims) = result.get("dimensions").and_then(|v| v.as_object()) {
        lines.push("| Metric | Value |".to_string());
        lines.push("|--------|-------|".to_string());
        if let Some(freshness) = dims.get("freshness_days").and_then(|v| v.as_f64()) {
            lines.push(format!("| Freshness | {:.0} days |", freshness));
        }
        lines.push(format!(
            "| Rules | {} approved |",
            dims.get("rules_count")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        ));
        lines.push(format!(
            "| High confidence | {:.0}% |",
            dims.get("high_confidence_ratio")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        ));
        lines.push(format!(
            "| Deterministic coverage | {:.0}% |",
            dims.get("deterministic_coverage")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0)
        ));
        lines.push(format!(
            "| Pending updates | {} deps |",
            dims.get("pending_updates")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
        ));
        lines.push(String::new());
    }

    if let Some(next) = result.get("next_command").and_then(|v| v.as_str()) {
        lines.push(format!("**Next:** `{}`", next));
        lines.push(String::new());
    }

    lines.join("\n")
}
