//! `App` is the root Elm-architecture model for the TUI.
//!
//! `update(&mut self, msg)` mutates state; `view(&self, frame)` renders.
//! Screen-specific state lives on sub-structs under `App`.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::tui::msg::{Msg, Screen};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub struct App {
    pub project_dir: PathBuf,
    pub screen: Screen,
    pub quit: bool,
    pub dashboard: DashboardState,
}

/// Cached data for the dashboard. Populated on start and on `Msg::Refresh`.
#[derive(Default)]
pub struct DashboardState {
    pub rule_system_score: Option<i64>,
    pub adherence_score: Option<i64>,
    pub adherence_detail: Value,
    pub rules_total: usize,
    pub rules_personal: usize,
    pub rules_by_language: Vec<(String, usize)>,
    pub drift_count: i64,
    pub drift_deps: Vec<String>,
    pub last_refresh: Option<String>,
    pub top_violations: Vec<TopViolation>,
    pub violation_counts: ViolationCounts,
    /// Debt report. `None` = not yet computed (press R or open Debt screen).
    /// `Some(Err(..))` = the compute failed and the screen shows the reason.
    pub debt: DebtView,
}

#[derive(Default, Clone)]
pub enum DebtView {
    #[default]
    NotComputed,
    Loading,
    Ready(Box<DebtSummaryView>),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct DebtSummaryView {
    pub debt_label: String,
    pub finding_count: u32,
    pub by_dead: u32,
    pub by_dup: u32,
    pub by_deps: u32,
    pub by_hotspots: u32,
    pub hotspots: Vec<DebtHotspotRow>,
}

#[derive(Debug, Clone)]
pub struct DebtHotspotRow {
    pub rank: u32,
    pub category: String,
    pub confidence: String,
    pub title: String,
    pub next_action: String,
    pub score: f64,
}

#[derive(Default, Clone)]
pub struct ViolationCounts {
    pub must: usize,
    pub should: usize,
    pub may: usize,
}

pub struct TopViolation {
    pub severity: String,
    pub rule_id: String,
    pub file: String,
    pub line: u64,
    pub snippet: String,
}

impl App {
    pub fn new(project_dir: impl Into<PathBuf>) -> Result<Self> {
        let project_dir = project_dir.into();
        let mut app = Self {
            project_dir: project_dir.clone(),
            screen: Screen::Dashboard,
            quit: false,
            dashboard: DashboardState::default(),
        };
        app.load_dashboard();
        Ok(app)
    }

    /// Best-effort load of the dashboard data. Errors are swallowed and
    /// surface as empty fields — the TUI must never panic on bad project state.
    pub fn load_dashboard(&mut self) {
        self.dashboard = collect_dashboard(&self.project_dir);
    }

    pub fn update(&mut self, msg: Msg) {
        match msg {
            Msg::Quit => self.quit = true,
            Msg::GoToScreen(s) => {
                self.screen = s;
                if s == Screen::Debt {
                    self.ensure_debt_loaded();
                }
            }
            Msg::Refresh => {
                self.load_dashboard();
                // Recompute debt on an explicit refresh.
                self.dashboard.debt = DebtView::NotComputed;
                if self.screen == Screen::Debt {
                    self.ensure_debt_loaded();
                }
            }
            Msg::Tick => {} // reserved for future spinner animation
            Msg::Key(ev) => self.handle_key(ev),
        }
    }

    /// Compute the debt report on-demand. Synchronous — running `wh debt`
    /// on a medium repo takes a couple of seconds, which is acceptable
    /// for a user-triggered screen open.
    pub fn ensure_debt_loaded(&mut self) {
        if !matches!(self.dashboard.debt, DebtView::NotComputed) {
            return;
        }
        self.dashboard.debt = DebtView::Loading;
        let opts = crate::debt::DebtOptions {
            project_dir: self.project_dir.clone(),
            top: 20,
            min_confidence: crate::debt::types::Confidence::Medium,
            since_days: 90,
        };
        self.dashboard.debt = match crate::debt::run(&opts) {
            Ok(report) => {
                let hotspots = report
                    .hotspots
                    .iter()
                    .map(|h| DebtHotspotRow {
                        rank: h.rank,
                        category: h.category.as_str().to_string(),
                        confidence: h.confidence.as_str().to_string(),
                        title: h.title.clone(),
                        next_action: h.next_action.clone(),
                        score: h.score,
                    })
                    .collect();
                DebtView::Ready(Box::new(DebtSummaryView {
                    debt_label: report.summary.debt_label.as_str().to_string(),
                    finding_count: report.summary.finding_count,
                    by_dead: report.summary.by_category.dead,
                    by_dup: report.summary.by_category.dup,
                    by_deps: report.summary.by_category.deps,
                    by_hotspots: report.summary.by_category.hotspots,
                    hotspots,
                }))
            }
            Err(e) => DebtView::Error(e.to_string()),
        };
    }

    fn handle_key(&mut self, ev: KeyEvent) {
        // Global keybinds — available on every screen.
        if ev.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(ev.code, KeyCode::Char('c'))
        {
            self.quit = true;
            return;
        }

        match ev.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('1') => self.screen = Screen::Dashboard,
            KeyCode::Char('2') => self.screen = Screen::Rules,
            KeyCode::Char('3') => self.screen = Screen::Sources,
            KeyCode::Char('4') => self.screen = Screen::Extract,
            KeyCode::Char('5') => self.screen = Screen::Check,
            KeyCode::Char('6') => self.screen = Screen::Report,
            KeyCode::Char('7') => self.screen = Screen::Drift,
            KeyCode::Char('8') => {
                self.screen = Screen::Debt;
                self.ensure_debt_loaded();
            }
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.load_dashboard();
                self.dashboard.debt = DebtView::NotComputed;
                if self.screen == Screen::Debt {
                    self.ensure_debt_loaded();
                }
            }
            _ => {}
        }
    }
}

/// Gather everything the dashboard needs in one pass. Reuses the existing
/// `status::compute_status` + `adherence::compute` + `handoff` paths so the
/// TUI stays consistent with `wh status` / `wh report` output.
fn collect_dashboard(project_dir: &Path) -> DashboardState {
    let mut d = DashboardState::default();

    // Status (rule_system_score, rule counts, drift, metrics).
    if let Ok(status) = crate::status::compute_status(project_dir, false, false) {
        d.rule_system_score = status
            .get("rule_system_score")
            .and_then(|v| v.as_i64())
            .or_else(|| status.get("score").and_then(|v| v.as_i64()));
        d.adherence_score = status.get("adherence_score").and_then(|v| v.as_i64());
        d.adherence_detail = status
            .get("adherence")
            .cloned()
            .unwrap_or(Value::Null);

        d.rules_total = status
            .get("dimensions")
            .and_then(|v| v.get("rules_count"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

        if let Some(drift) = status.get("drift") {
            d.drift_count = drift.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
            if let Some(changes) = drift.get("dependency_changes").and_then(|v| v.as_array()) {
                d.drift_deps = changes
                    .iter()
                    .filter_map(|c| c.get("name").and_then(|n| n.as_str()).map(String::from))
                    .take(10)
                    .collect();
            }
        }

        if let Some(counts) = d.adherence_detail.get("violations") {
            d.violation_counts.must =
                counts.get("must").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            d.violation_counts.should =
                counts.get("should").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            d.violation_counts.may =
                counts.get("may").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        }
    }

    // Personal rule count (separate from total — merged rules include personal).
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let (personal_rules, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
    d.rules_personal = personal_rules.len();

    // Rules-by-language breakdown from the merged set.
    let merged = crate::layers::resolve_merged(project_dir, None, true, true, false);
    let mut by_lang: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for lr in &merged.merged {
        *by_lang.entry(lr.rule.language.clone()).or_insert(0) += 1;
    }
    d.rules_by_language = by_lang.into_iter().collect();

    // Last-refresh timestamp from refresh-diff.json, if present.
    let refresh_diff_path = project_dir
        .join("whetstone")
        .join(".state")
        .join("refresh-diff.json");
    if let Ok(text) = std::fs::read_to_string(&refresh_diff_path) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            d.last_refresh = v
                .get("generated_at")
                .and_then(|s| s.as_str())
                .map(String::from);
        }
    }

    // Top violations — reuse `wh check` directly.
    let scan_root = if project_dir.join("src").is_dir() {
        project_dir.join("src")
    } else {
        project_dir.to_path_buf()
    };
    if let Ok(check) = crate::check::run(crate::check::CheckOptions {
        project_dir,
        scan_paths: std::slice::from_ref(&scan_root),
        lang_filter: None,
        rule_filter: None,
    }) {
        if let Some(arr) = check.get("violations").and_then(|v| v.as_array()) {
            let mut sorted = arr.clone();
            sorted.sort_by_key(|v| match v.get("severity").and_then(|s| s.as_str()) {
                Some("must") => 0,
                Some("should") => 1,
                _ => 2,
            });
            d.top_violations = sorted
                .iter()
                .take(5)
                .filter_map(|v| {
                    Some(TopViolation {
                        severity: v.get("severity").and_then(|s| s.as_str())?.to_string(),
                        rule_id: v.get("rule_id").and_then(|s| s.as_str())?.to_string(),
                        file: v.get("file").and_then(|s| s.as_str())?.to_string(),
                        line: v.get("line").and_then(|s| s.as_u64()).unwrap_or(0),
                        snippet: v
                            .get("match")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect();
        }
    }

    d
}
