//! `wh debt` — AI-code debt triage.
//!
//! See `planning/debt.md` for the design. This module orchestrates a small
//! set of deterministic detectors (dead code, duplicates, dep hygiene,
//! churn × violations hotspots), ranks their findings, and emits a single
//! JSON envelope that backs both the human CLI and the downstream prompt
//! and Beads output modes.

use anyhow::Result;
use std::path::Path;

pub mod detectors;
pub mod beads;
pub mod output;
pub mod rank;
pub mod source_walk;
pub mod types;

use types::{DebtReport, Finding};

/// Options controlling a `wh debt` run.
#[derive(Debug, Clone)]
pub struct DebtOptions {
    pub project_dir: std::path::PathBuf,
    pub top: usize,
    pub min_confidence: types::Confidence,
    pub since_days: u32,
}

impl Default for DebtOptions {
    fn default() -> Self {
        Self {
            project_dir: std::path::PathBuf::from("."),
            top: 20,
            min_confidence: types::Confidence::Medium,
            since_days: 90,
        }
    }
}

/// Run all enabled detectors and return a fully ranked debt report.
pub fn run(opts: &DebtOptions) -> Result<DebtReport> {
    let project_dir: &Path = &opts.project_dir;

    let sources = source_walk::collect(project_dir);

    let mut findings: Vec<Finding> = Vec::new();
    findings.extend(detectors::dep_hygiene::run(project_dir, &sources)?);
    findings.extend(detectors::dead_code::run(project_dir, &sources)?);
    findings.extend(detectors::duplicates::run(project_dir, &sources)?);
    findings.extend(detectors::hotspots::run(project_dir, opts.since_days)?);

    let findings = findings
        .into_iter()
        .filter(|f| f.confidence >= opts.min_confidence)
        .collect::<Vec<_>>();

    let report = rank::build_report(project_dir, findings, opts.top);
    Ok(report)
}
