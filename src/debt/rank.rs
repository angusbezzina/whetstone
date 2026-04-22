//! Score, rank, and label findings. See `planning/debt.md` §"Ranking...".

use std::path::Path;

use super::types::{BycatCounts, Category, DebtLabel, DebtReport, Finding, Hotspot, Summary};

const HOTSPOT_SCORE_THRESHOLD: f64 = 1.5;

pub fn score(f: &Finding) -> f64 {
    f.category.base_weight() * f.evidence_strength * f.confidence.factor()
}

pub fn label(findings: &[Finding]) -> DebtLabel {
    let high_conf = findings
        .iter()
        .filter(|f| f.confidence == super::types::Confidence::High)
        .count();
    let high_score_hotspots = findings
        .iter()
        .filter(|f| f.category == Category::Hotspots && score(f) >= HOTSPOT_SCORE_THRESHOLD)
        .count();

    if high_conf > 60 || high_score_hotspots > 5 {
        DebtLabel::High
    } else if high_conf > 20 || high_score_hotspots > 2 {
        DebtLabel::Elevated
    } else if high_conf >= 5 || high_score_hotspots >= 1 {
        DebtLabel::Moderate
    } else {
        DebtLabel::Low
    }
}

pub fn counts(findings: &[Finding]) -> BycatCounts {
    let mut c = BycatCounts::default();
    for f in findings {
        match f.category {
            Category::Dead => c.dead += 1,
            Category::Dup => c.dup += 1,
            Category::Deps => c.deps += 1,
            Category::Hotspots => c.hotspots += 1,
        }
    }
    c
}

pub fn rank(findings: Vec<Finding>) -> Vec<Hotspot> {
    let mut scored: Vec<(f64, Finding)> = findings.into_iter().map(|f| (score(&f), f)).collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.category.as_str().cmp(b.1.category.as_str()))
            .then_with(|| a.1.rule_id.cmp(&b.1.rule_id))
            .then_with(|| a.1.title.cmp(&b.1.title))
    });

    scored
        .into_iter()
        .enumerate()
        .map(|(i, (score, f))| Hotspot {
            id: format!("h{}", i + 1),
            category: f.category,
            rule_id: f.rule_id,
            title: f.title,
            confidence: f.confidence,
            rank: (i + 1) as u32,
            score: round2(score),
            files: f.files,
            evidence: f.evidence,
            next_action: f.next_action,
        })
        .collect()
}

fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

pub fn build_report(project_dir: &Path, findings: Vec<Finding>, top: usize) -> DebtReport {
    let by_category = counts(&findings);
    let total = findings.len() as u32;
    let debt_label = label(&findings);
    let ranked = rank(findings);
    let hotspots: Vec<Hotspot> = ranked.into_iter().take(top.max(1)).collect();

    let summary = Summary {
        debt_label,
        hotspot_count: hotspots.len() as u32,
        finding_count: total,
        by_category,
    };

    DebtReport {
        schema_version: 1,
        generated_at: now_iso(),
        project_dir: project_dir.display().to_string(),
        summary,
        hotspots,
        notes: Vec::new(),
    }
}

fn now_iso() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}
