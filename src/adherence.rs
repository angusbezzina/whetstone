//! Adherence score — how well the code matches the approved rules.
//!
//! Formula (per `planning/measurements/adherence-score-design.md`):
//!
//!   clean_ratio       = 100 * files_clean / files_eligible
//!   severity_penalty  = 100 * weighted_viols / (approved_rules * files_eligible)
//!   severity_component = 100 - penalty  (clamped ≥ 0)
//!   score             = round(0.6 * clean_ratio + 0.4 * severity_component)
//!
//! Severity weights: must=1.0, should=0.5, may=0.2.
//!
//! `wh status` calls `compute(..)` and embeds the result as `adherence_score`
//! alongside the existing `rule_system_score` (renamed from the legacy `score`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::{json, Value};

use crate::check;

/// Final adherence result to render into `wh status` JSON.
pub struct Adherence {
    /// Final hybrid score 0–100.
    pub score: i64,
    /// % of eligible files that had zero violations.
    pub clean_ratio: i64,
    /// 100 − severity-weighted penalty (0–100).
    pub severity_component: i64,
    pub files_eligible: usize,
    pub files_clean: usize,
    pub violation_counts: ViolationCounts,
}

pub struct ViolationCounts {
    pub must: usize,
    pub should: usize,
    pub may: usize,
    pub total: usize,
}

/// Run `wh check` against the project tree and compute the hybrid adherence
/// score. Returns `None` when there are no approved rules or no eligible
/// files — `wh status` renders that as `adherence_score: null` to distinguish
/// "unmeasured" from "perfect."
///
/// `rule_count_hint` is the caller's rule count (from `wh status`'s project-
/// layer scan); this function cross-checks against the merged personal +
/// project count via `wh check`'s `rules_applied` so personal-only projects
/// still score instead of silently returning None.
pub fn compute(project_dir: &Path, rule_count_hint: usize) -> Result<Option<Adherence>> {
    let scan_root = candidate_scan_root(project_dir);
    let paths = vec![scan_root];

    let result = check::run(check::CheckOptions {
        project_dir,
        scan_paths: &paths,
        lang_filter: None,
        rule_filter: None,
    })?;

    let rules_applied = result
        .get("rules_applied")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let approved_rule_count = rules_applied.max(rule_count_hint);
    if approved_rule_count == 0 {
        return Ok(None);
    }

    let files_eligible = result
        .get("files_scanned")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;

    if files_eligible == 0 {
        return Ok(None);
    }

    let violations = result
        .get("violations")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut files_with_violations: HashSet<String> = HashSet::new();
    let mut counts = ViolationCounts {
        must: 0,
        should: 0,
        may: 0,
        total: 0,
    };
    let mut weighted_viols = 0.0_f64;

    for v in &violations {
        let sev = v
            .get("severity")
            .and_then(|s| s.as_str())
            .unwrap_or("may");
        let weight = match sev {
            "must" => 1.0,
            "should" => 0.5,
            _ => 0.2,
        };
        weighted_viols += weight;
        counts.total += 1;
        match sev {
            "must" => counts.must += 1,
            "should" => counts.should += 1,
            _ => counts.may += 1,
        }
        if let Some(file) = v.get("file").and_then(|s| s.as_str()) {
            files_with_violations.insert(file.to_string());
        }
    }

    let files_clean = files_eligible.saturating_sub(files_with_violations.len());

    let clean_ratio = 100.0 * (files_clean as f64) / (files_eligible as f64);
    let denom = (approved_rule_count as f64) * (files_eligible as f64);
    let penalty = (100.0 * weighted_viols / denom.max(1.0)).min(100.0);
    let severity_component = (100.0 - penalty).max(0.0);

    let score = (0.6 * clean_ratio + 0.4 * severity_component).round() as i64;

    Ok(Some(Adherence {
        score,
        clean_ratio: clean_ratio.round() as i64,
        severity_component: severity_component.round() as i64,
        files_eligible,
        files_clean,
        violation_counts: counts,
    }))
}

/// Serialize an Adherence result into the JSON shape that `wh status` emits.
pub fn to_json(a: &Adherence) -> Value {
    json!({
        "score": a.score,
        "clean_ratio": a.clean_ratio,
        "severity_component": a.severity_component,
        "files_eligible": a.files_eligible,
        "files_clean": a.files_clean,
        "violations": {
            "must": a.violation_counts.must,
            "should": a.violation_counts.should,
            "may": a.violation_counts.may,
            "total": a.violation_counts.total,
        },
    })
}

/// Pick the best directory to scan. Prefer `src/` when it exists; otherwise
/// fall back to the whole project directory. Matches the implicit convention
/// agents use when calling `wh check src/`.
fn candidate_scan_root(project_dir: &Path) -> PathBuf {
    let src = project_dir.join("src");
    if src.is_dir() {
        src
    } else {
        project_dir.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    // Formula unit tests — avoid touching disk.
    use super::{Adherence, ViolationCounts};

    fn make(
        files_eligible: usize,
        files_clean: usize,
        must: usize,
        should: usize,
        may: usize,
        approved: usize,
    ) -> Adherence {
        let weighted_viols = must as f64 + 0.5 * should as f64 + 0.2 * may as f64;
        let clean_ratio = 100.0 * files_clean as f64 / files_eligible as f64;
        let denom = (approved as f64) * (files_eligible as f64);
        let penalty = (100.0 * weighted_viols / denom.max(1.0)).min(100.0);
        let severity_component = (100.0 - penalty).max(0.0);
        let score = (0.6 * clean_ratio + 0.4 * severity_component).round() as i64;
        Adherence {
            score,
            clean_ratio: clean_ratio.round() as i64,
            severity_component: severity_component.round() as i64,
            files_eligible,
            files_clean,
            violation_counts: ViolationCounts {
                must,
                should,
                may,
                total: must + should + may,
            },
        }
    }

    #[test]
    fn clean_project_scores_perfect() {
        let a = make(10, 10, 0, 0, 0, 1);
        assert_eq!(a.score, 100);
    }

    #[test]
    fn must_violations_hurt_more_than_may() {
        let must_hit = make(10, 9, 1, 0, 0, 1);
        let may_hit = make(10, 9, 0, 0, 1, 1);
        assert!(must_hit.score < may_hit.score);
    }

    #[test]
    fn small_project_single_must_does_not_tank_score() {
        // 10-file project, 1 must violation → should stay in the 90s.
        let a = make(10, 9, 1, 0, 0, 1);
        assert!(a.score > 80 && a.score <= 95, "got {}", a.score);
    }

    #[test]
    fn large_project_single_must_is_barely_noticeable() {
        // 100-file project, 1 must violation → essentially perfect.
        let a = make(100, 99, 1, 0, 0, 1);
        assert!(a.score > 98, "got {}", a.score);
    }

    #[test]
    fn many_may_violations_still_penalize() {
        // 10-file project, 10 may violations (1/file) → noticeable drop,
        // but not as bad as 10 must violations.
        let may_case = make(10, 0, 0, 0, 10, 1);
        let must_case = make(10, 0, 10, 0, 0, 1);
        assert!(may_case.score > must_case.score);
        assert!(may_case.score < 50, "may violations must penalize: {}", may_case.score);
    }
}
