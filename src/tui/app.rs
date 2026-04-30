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
    pub input_mode: InputMode,
    pub help_scroll_y: u16,
    pub help_scroll_x: u16,
    pub dashboard_scroll: usize,
    pub dashboard: DashboardState,
    pub sources_dataset: SourcesDataset,
    pub sources_selected: usize,
    pub sources_form: SourcesFormState,
    pub rules_form: RulesFormState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    SourcesAdd,
    RulesAdd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourcesDataset {
    #[default]
    Dependencies,
    Personal,
    Team,
}

impl SourcesDataset {
    pub fn next(self) -> Self {
        match self {
            Self::Dependencies => Self::Personal,
            Self::Personal => Self::Team,
            Self::Team => Self::Dependencies,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Dependencies => Self::Team,
            Self::Personal => Self::Dependencies,
            Self::Team => Self::Personal,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SourcesFormState {
    pub active_field: usize,
    pub team_scope: bool,
    pub url: String,
    pub name: String,
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct RulesFormState {
    pub active_field: usize,
    pub team_scope: bool,
    pub name: String,
    pub language_idx: usize,
    pub rule_text: String,
    pub error: Option<String>,
}

/// Cached data for the dashboard. Populated on start.
#[derive(Default)]
pub struct DashboardState {
    pub rule_system_score: Option<i64>,
    pub adherence_score: Option<i64>,
    pub adherence_detail: Value,
    pub sources_total: usize,
    pub rules_total: usize,
    pub rules_personal: usize,
    pub rules_by_language: Vec<(String, usize)>,
    pub violation_counts: ViolationCounts,
    pub result: crate::tui::screens::result::ResultView,
    /// Debt report. `None` = not yet computed (open the Debt screen to compute it).
    /// `Some(Err(..))` = the compute failed and the screen shows the reason.
    pub debt: DebtView,
    /// Per-screen view state for the second-slice screens (whetstone-69jb).
    /// Each starts at `NotComputed` and transitions via its `ensure_*_loaded`
    /// method. Screens own their own data shape — see `src/tui/screens/*.rs`.
    pub rules: crate::tui::screens::rules::RulesView,
    pub sources: crate::tui::screens::sources::SourcesView,
    pub extract: crate::tui::screens::extract::ExtractView,
    pub check: crate::tui::screens::check::CheckView,
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
    pub selected: usize,
    pub scroll_x: u16,
    pub hotspots: Vec<DebtHotspotRow>,
}

#[derive(Debug, Clone)]
pub struct DebtHotspotRow {
    pub category: String,
    pub confidence: String,
    pub rule_id: String,
    pub title: String,
    pub compact_title: String,
    pub primary_file: String,
    pub files: Vec<String>,
    pub snippet: String,
    pub impact_level: String,
}

#[derive(Default, Clone)]
pub struct ViolationCounts {
    pub must: usize,
    pub should: usize,
    pub may: usize,
}

impl App {
    pub fn new(project_dir: impl Into<PathBuf>) -> Result<Self> {
        let project_dir = project_dir.into();
        let mut app = Self {
            project_dir: project_dir.clone(),
            screen: Screen::Dashboard,
            quit: false,
            input_mode: InputMode::Normal,
            help_scroll_y: 0,
            help_scroll_x: 0,
            dashboard_scroll: 0,
            dashboard: DashboardState::default(),
            sources_dataset: SourcesDataset::default(),
            sources_selected: 0,
            sources_form: SourcesFormState::default(),
            rules_form: RulesFormState::default(),
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
                self.ensure_current_screen_loaded();
            }
            Msg::Tick => {} // reserved for future spinner animation
            Msg::Key(ev) => self.handle_key(ev),
        }
    }

    /// Trigger the lazy loader for whichever screen is currently active.
    /// Screens that don't have a loader (Dashboard, Help) are no-ops.
    pub fn ensure_current_screen_loaded(&mut self) {
        match self.screen {
            Screen::Result => {}
            Screen::Debt => self.ensure_debt_loaded(),
            Screen::Sources => self.ensure_sources_loaded(),
            Screen::Rules => self.ensure_rules_loaded(),
            Screen::Check => self.ensure_check_loaded(),
            Screen::Dashboard | Screen::Help => {}
        }
    }

    /// Each ensure_*_loaded method transitions `NotComputed` → `Loading` →
    /// `Ready`/`Error` synchronously. Wire the actual compute into the
    /// screen's `load` function in `src/tui/screens/<name>.rs`; the method
    /// below just drives the state machine.
    pub fn ensure_rules_loaded(&mut self) {
        if !matches!(
            self.dashboard.rules,
            crate::tui::screens::rules::RulesView::NotComputed
        ) {
            return;
        }
        self.dashboard.rules = crate::tui::screens::rules::RulesView::Loading;
        self.dashboard.rules = crate::tui::screens::rules::load(&self.project_dir);
    }

    pub fn ensure_sources_loaded(&mut self) {
        self.ensure_extract_loaded();
        if !matches!(
            self.dashboard.sources,
            crate::tui::screens::sources::SourcesView::NotComputed
        ) {
            return;
        }
        self.dashboard.sources = crate::tui::screens::sources::SourcesView::Loading;
        self.dashboard.sources = crate::tui::screens::sources::load(&self.project_dir);
    }

    pub fn ensure_extract_loaded(&mut self) {
        if !matches!(
            self.dashboard.extract,
            crate::tui::screens::extract::ExtractView::NotComputed
        ) {
            return;
        }
        self.dashboard.extract = crate::tui::screens::extract::ExtractView::Loading;
        self.dashboard.extract = crate::tui::screens::extract::load(&self.project_dir);
    }

    pub fn ensure_check_loaded(&mut self) {
        if !matches!(
            self.dashboard.check,
            crate::tui::screens::check::CheckView::NotComputed
        ) {
            return;
        }
        self.dashboard.check = crate::tui::screens::check::CheckView::Loading;
        self.dashboard.check = crate::tui::screens::check::load(&self.project_dir);
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
            top: usize::MAX,
            min_confidence: crate::debt::types::Confidence::Medium,
            since_days: 90,
        };
        self.dashboard.debt = match crate::debt::run(&opts) {
            Ok(report) => {
                let max_score = report
                    .hotspots
                    .iter()
                    .map(|h| h.score)
                    .fold(0.0_f64, f64::max);
                let hotspots = report
                    .hotspots
                    .iter()
                    .map(|h| DebtHotspotRow {
                        category: h.category.as_str().to_string(),
                        confidence: h.confidence.as_str().to_string(),
                        rule_id: h.rule_id.clone(),
                        title: h.title.clone(),
                        compact_title: compact_hotspot_title(&h.title),
                        primary_file: h.files.first().cloned().unwrap_or_else(|| "—".to_string()),
                        files: h.files.clone(),
                        snippet: debt_snippet(&h.evidence),
                        impact_level: impact_level(normalize_impact_percent(h.score, max_score))
                            .to_string(),
                    })
                    .collect();
                DebtView::Ready(Box::new(DebtSummaryView {
                    debt_label: report.summary.debt_label.as_str().to_string(),
                    finding_count: report.summary.finding_count,
                    by_dead: report.summary.by_category.dead,
                    by_dup: report.summary.by_category.dup,
                    by_deps: report.summary.by_category.deps,
                    by_hotspots: report.summary.by_category.hotspots,
                    selected: 0,
                    scroll_x: 0,
                    hotspots,
                }))
            }
            Err(e) => DebtView::Error(e.to_string()),
        };
    }

    fn handle_key(&mut self, ev: KeyEvent) {
        if self.input_mode != InputMode::Normal {
            self.handle_form_key(ev);
            return;
        }

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
            KeyCode::Char('2') => {
                self.screen = Screen::Sources;
                self.ensure_sources_loaded();
            }
            KeyCode::Char('3') => {
                self.screen = Screen::Rules;
                self.ensure_rules_loaded();
            }
            KeyCode::Char('4') => {
                self.screen = Screen::Check;
                self.ensure_check_loaded();
            }
            KeyCode::Char('5') => {
                self.screen = Screen::Debt;
                self.ensure_debt_loaded();
            }
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('a') | KeyCode::Char('A') => match self.screen {
                Screen::Sources => self.open_sources_form(),
                Screen::Rules => self.open_rules_form(),
                _ => {}
            },
            KeyCode::Tab if self.screen == Screen::Sources => {
                self.sources_dataset = self.sources_dataset.next();
                self.sources_selected = 0;
            }
            KeyCode::BackTab if self.screen == Screen::Sources => {
                self.sources_dataset = self.sources_dataset.prev();
                self.sources_selected = 0;
            }
            KeyCode::Char('d') | KeyCode::Char('D') if self.screen == Screen::Sources => {
                self.sources_dataset = SourcesDataset::Dependencies;
                self.sources_selected = 0;
            }
            KeyCode::Char('p') | KeyCode::Char('P') if self.screen == Screen::Sources => {
                self.sources_dataset = SourcesDataset::Personal;
                self.sources_selected = 0;
            }
            KeyCode::Char('t') | KeyCode::Char('T') if self.screen == Screen::Sources => {
                self.sources_dataset = SourcesDataset::Team;
                self.sources_selected = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => self.select_prev_on_current_screen(1),
            KeyCode::Down | KeyCode::Char('j') => self.select_next_on_current_screen(1),
            KeyCode::PageUp => self.select_prev_on_current_screen(10),
            KeyCode::PageDown => self.select_next_on_current_screen(10),
            KeyCode::Left | KeyCode::Char('h') => self.scroll_left_on_current_screen(4),
            KeyCode::Right | KeyCode::Char('l') => self.scroll_right_on_current_screen(4),
            _ => {}
        }
    }

    /// Move selection one step backward on whichever list-oriented screen is
    /// active. No-op on screens without a selectable list.
    fn select_prev_on_current_screen(&mut self, steps: usize) {
        for _ in 0..steps {
            match self.screen {
                Screen::Dashboard => self.dashboard_scroll = self.dashboard_scroll.saturating_sub(1),
                Screen::Help => self.help_scroll_y = self.help_scroll_y.saturating_sub(1),
                Screen::Result => self.dashboard.result.scroll_up(1),
                Screen::Debt => self.dashboard.debt.select_prev(),
                Screen::Sources => match self.sources_dataset {
                    SourcesDataset::Dependencies => self.dashboard.extract.select_prev(),
                    SourcesDataset::Personal | SourcesDataset::Team => {
                        self.sources_selected = self.sources_selected.saturating_sub(1)
                    }
                },
                Screen::Rules => self.dashboard.rules.select_prev(),
                Screen::Check => self.dashboard.check.select_prev(),
            }
        }
    }

    fn select_next_on_current_screen(&mut self, steps: usize) {
        for _ in 0..steps {
            match self.screen {
                Screen::Dashboard => {}
                Screen::Help => self.help_scroll_y = self.help_scroll_y.saturating_add(1),
                Screen::Result => self.dashboard.result.scroll_down(1),
                Screen::Debt => self.dashboard.debt.select_next(),
                Screen::Sources => match self.sources_dataset {
                    SourcesDataset::Dependencies => self.dashboard.extract.select_next(),
                    SourcesDataset::Personal => {
                        let max = self
                            .dashboard
                            .sources
                            .row_count_for(SourcesDataset::Personal)
                            .saturating_sub(1);
                        if self.sources_selected < max {
                            self.sources_selected += 1;
                        }
                    }
                    SourcesDataset::Team => {
                        let max = self
                            .dashboard
                            .sources
                            .row_count_for(SourcesDataset::Team)
                            .saturating_sub(1);
                        if self.sources_selected < max {
                            self.sources_selected += 1;
                        }
                    }
                },
                Screen::Rules => self.dashboard.rules.select_next(),
                Screen::Check => self.dashboard.check.select_next(),
            }
        }
    }

    fn scroll_left_on_current_screen(&mut self, steps: u16) {
        match self.screen {
            Screen::Help => self.help_scroll_x = self.help_scroll_x.saturating_sub(steps),
            Screen::Result => self.dashboard.result.scroll_left(steps),
            Screen::Debt => self.dashboard.debt.scroll_left(steps),
            _ => {}
        }
    }

    fn scroll_right_on_current_screen(&mut self, steps: u16) {
        match self.screen {
            Screen::Help => self.help_scroll_x = self.help_scroll_x.saturating_add(steps),
            Screen::Result => self.dashboard.result.scroll_right(steps),
            Screen::Debt => self.dashboard.debt.scroll_right(steps),
            _ => {}
        }
    }

    fn open_sources_form(&mut self) {
        self.sources_form = SourcesFormState::default();
        self.input_mode = InputMode::SourcesAdd;
    }

    fn open_rules_form(&mut self) {
        self.rules_form = RulesFormState::default();
        self.input_mode = InputMode::RulesAdd;
    }

    fn handle_form_key(&mut self, ev: KeyEvent) {
        match self.input_mode {
            InputMode::Normal => {}
            InputMode::SourcesAdd => self.handle_sources_form_key(ev),
            InputMode::RulesAdd => self.handle_rules_form_key(ev),
        }
    }

    fn handle_sources_form_key(&mut self, ev: KeyEvent) {
        match ev.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.sources_form.error = None;
            }
            KeyCode::Tab => {
                self.sources_form.active_field = (self.sources_form.active_field + 1) % 2;
            }
            KeyCode::BackTab => {
                self.sources_form.active_field = self.sources_form.active_field.saturating_sub(1);
            }
            KeyCode::Backspace => {
                self.current_sources_field_mut().pop();
            }
            KeyCode::Enter => self.submit_sources_form(),
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.sources_form.team_scope = !self.sources_form.team_scope;
            }
            KeyCode::Char(c) => {
                self.current_sources_field_mut().push(c);
            }
            _ => {}
        }
    }

    fn handle_rules_form_key(&mut self, ev: KeyEvent) {
        match ev.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.rules_form.error = None;
            }
            KeyCode::Tab => {
                self.rules_form.active_field = (self.rules_form.active_field + 1) % 3;
            }
            KeyCode::BackTab => {
                self.rules_form.active_field = self.rules_form.active_field.saturating_sub(1);
            }
            KeyCode::Backspace => {
                if self.rules_form.active_field != 1 {
                    self.current_rules_field_mut().pop();
                }
            }
            KeyCode::Enter => {
                if self.rules_form.active_field == 2 {
                    self.current_rules_field_mut().push('\n');
                } else {
                    self.rules_form.active_field = (self.rules_form.active_field + 1) % 3;
                }
            }
            KeyCode::Char('s') | KeyCode::Char('S') if ev.modifiers.contains(KeyModifiers::CONTROL) => {
                self.submit_rules_form();
            }
            KeyCode::Char('t') | KeyCode::Char('T') => {
                self.rules_form.team_scope = !self.rules_form.team_scope;
            }
            KeyCode::Left | KeyCode::Char('h') if self.rules_form.active_field == 1 => {
                self.rules_form.language_idx = self.rules_form.language_idx.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_form.active_field == 1 => {
                self.rules_form.language_idx = (self.rules_form.language_idx + 1).min(3);
            }
            KeyCode::Char('p') | KeyCode::Char('P') if self.rules_form.active_field == 1 => {
                self.rules_form.language_idx = 2;
            }
            KeyCode::Char('r') | KeyCode::Char('R') if self.rules_form.active_field == 1 => {
                self.rules_form.language_idx = 1;
            }
            KeyCode::Char('s') | KeyCode::Char('S') if self.rules_form.active_field == 1 => {
                self.rules_form.language_idx = 0;
            }
            KeyCode::Char('a') | KeyCode::Char('A') if self.rules_form.active_field == 1 => {
                self.rules_form.language_idx = 3;
            }
            KeyCode::Char(c) if self.rules_form.active_field != 1 => {
                self.current_rules_field_mut().push(c);
            }
            _ => {}
        }
    }

    fn current_sources_field_mut(&mut self) -> &mut String {
        match self.sources_form.active_field {
            0 => &mut self.sources_form.url,
            _ => &mut self.sources_form.name,
        }
    }

    fn current_rules_field_mut(&mut self) -> &mut String {
        match self.rules_form.active_field {
            0 => &mut self.rules_form.name,
            1 => &mut self.rules_form.name,
            _ => &mut self.rules_form.rule_text,
        }
    }

    fn submit_sources_form(&mut self) {
        let name = if self.sources_form.name.trim().is_empty() {
            None
        } else {
            Some(self.sources_form.name.trim())
        };

        match crate::source_mgmt::add(
            &self.project_dir,
            crate::source_mgmt::AddOptions {
                url: self.sources_form.url.trim(),
                name,
                language: None,
                source_kind: None,
                personal: !self.sources_form.team_scope,
            },
        ) {
            Ok(_) => {
                self.dashboard.sources = crate::tui::screens::sources::SourcesView::NotComputed;
                self.ensure_sources_loaded();
                self.input_mode = InputMode::Normal;
                self.sources_form = SourcesFormState::default();
            }
            Err(e) => self.sources_form.error = Some(e.to_string()),
        }
    }

    fn submit_rules_form(&mut self) {
        let slug = slugify_rule_name(&self.rules_form.name);
        if slug.is_empty() {
            self.rules_form.error = Some("Rule name must contain at least one letter or number.".into());
            return;
        }
        let languages: &[&str] = match self.rules_form.language_idx {
            0 => &["typescript"],
            1 => &["rust"],
            2 => &["python"],
            _ => &["python", "rust", "typescript"],
        };
        let planned_ids: Vec<String> = languages
            .iter()
            .map(|language| {
                if languages.len() == 1 {
                    format!("custom.{slug}")
                } else {
                    format!("custom.{slug}-{language}")
                }
            })
            .collect();
        if let Some(existing) = first_existing_rule_id(&self.project_dir, &planned_ids) {
            self.rules_form.error = Some(format!(
                "Rule `{existing}` already exists. Choose a different name or remove the existing rule first."
            ));
            return;
        }
        let mut errors = Vec::new();

        for (language, rule_id) in languages.iter().zip(planned_ids.iter()) {
            if let Err(e) = crate::rule_authoring::add(
                &self.project_dir,
                crate::rule_authoring::AddOptions {
                    rule_id,
                    description: self.rules_form.rule_text.trim(),
                    match_regex: None,
                    severity: "should",
                    confidence: "high",
                    category: "convention",
                    language,
                    source_url: None,
                    dep: Some("custom"),
                    personal: !self.rules_form.team_scope,
                },
            ) {
                errors.push(e.to_string());
            }
        }

        if errors.is_empty() {
            self.dashboard.rules = crate::tui::screens::rules::RulesView::NotComputed;
            self.ensure_rules_loaded();
            self.input_mode = InputMode::Normal;
            self.rules_form = RulesFormState::default();
        } else {
            self.rules_form.error = Some(errors.join("\n"));
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

    if let Ok(handoff) = crate::worklist::load(project_dir) {
        d.sources_total = handoff
            .get("worklist")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);
    }
    if let Ok(custom) = crate::source_mgmt::list(project_dir) {
        d.sources_total += custom.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    }

    d
}

fn normalize_impact_percent(score: f64, max_score: f64) -> u8 {
    if max_score <= 0.0 {
        0
    } else {
        ((score / max_score) * 100.0).round().clamp(0.0, 100.0) as u8
    }
}

fn impact_level(percent: u8) -> &'static str {
    match percent {
        67..=100 => "High",
        34..=66 => "Moderate",
        _ => "Low",
    }
}

fn compact_hotspot_title(title: &str) -> String {
    let mut out = title
        .split(" in ")
        .next()
        .unwrap_or(title)
        .split(" across ")
        .next()
        .unwrap_or(title)
        .split(" (")
        .next()
        .unwrap_or(title)
        .trim()
        .to_string();
    if !out.is_empty() {
        let mut chars = out.chars();
        if let Some(first) = chars.next() {
            out = format!("{}{}", first.to_uppercase(), chars.as_str());
        }
    }
    out
}

fn debt_snippet(evidence: &crate::debt::types::Evidence) -> String {
    use crate::debt::types::Evidence;

    match evidence {
        Evidence::ManifestEntry { snippet, .. } => truncate_inline(snippet, 220),
        Evidence::SymbolDef { name, symbol_kind, .. } => {
            format!("{symbol_kind}: {name}")
        }
        Evidence::DuplicateCluster {
            snippet,
            ..
        } => truncate_inline(snippet, 220),
        Evidence::OrphanedFile { path, .. } => path.clone(),
        Evidence::ChurnViolationIntersection {
            changes,
            violations,
            window_days,
            ..
        } => format!("{changes} changes and {violations} violations over {window_days}d"),
    }
}

fn truncate_inline(text: &str, max: usize) -> String {
    let compact = text.replace('\n', " ");
    let mut chars = compact.chars();
    let taken: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        format!("{taken}…")
    } else {
        taken
    }
}

fn slugify_rule_name(input: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in input.chars() {
        let lc = ch.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            out.push(lc);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn first_existing_rule_id(project_dir: &Path, planned: &[String]) -> Option<String> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = crate::rules::load_rule_files(dir);
        for file in files {
            for rule in file.rule_file.rules {
                if planned.iter().any(|id| id == &rule.id) {
                    return Some(rule.id);
                }
            }
        }
    }
    None
}
