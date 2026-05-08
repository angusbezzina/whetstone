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
    pub help: HelpState,
    pub dashboard_ui: DashboardUiState,
    pub dashboard: DashboardState,
    pub sources_ui: SourcesUiState,
    pub rules_ui: RulesUiState,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RulesLanguageFilter {
    #[default]
    All,
    Python,
    Rust,
    Typescript,
}

impl RulesLanguageFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Python,
            Self::Python => Self::Rust,
            Self::Rust => Self::Typescript,
            Self::Typescript => Self::All,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::All => Self::Typescript,
            Self::Python => Self::All,
            Self::Rust => Self::Python,
            Self::Typescript => Self::Rust,
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct SourcesFormState {
    pub active_field: usize,
    pub team_scope: bool,
    pub url: String,
    pub language_idx: usize,
    pub error: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct HelpState {
    pub scroll_y: u16,
    pub scroll_x: u16,
}

#[derive(Debug, Default, Clone)]
pub struct DashboardUiState {
    pub scroll: usize,
}

#[derive(Debug, Default, Clone)]
pub struct SourcesUiState {
    pub dataset: SourcesDataset,
    pub selected: usize,
    pub detail_selected: bool,
    pub detail_scroll_y: u16,
    pub form: SourcesFormState,
}

#[derive(Debug, Default, Clone)]
pub struct RulesUiState {
    pub language_filter: RulesLanguageFilter,
    pub selected: usize,
    pub form: RulesFormState,
}

#[derive(Debug, Clone)]
pub struct RulesFormState {
    pub active_field: usize,
    pub team_scope: bool,
    pub name: String,
    pub language_idx: usize,
    pub severity_idx: usize,
    pub rule_text: String,
    pub error: Option<String>,
}

impl Default for RulesFormState {
    fn default() -> Self {
        Self {
            active_field: 0,
            team_scope: false,
            name: String::new(),
            language_idx: 3,
            severity_idx: 1,
            rule_text: String::new(),
            error: None,
        }
    }
}

/// Cached data for the dashboard. Populated on start.
#[derive(Default)]
pub struct DashboardState {
    pub rule_system_score: Option<i64>,
    pub adherence_score: Option<i64>,
    pub adherence_detail: Value,
    pub sources_total: usize,
    pub sources_internal: usize,
    pub sources_external: usize,
    pub rules_total: usize,
    pub rules_personal: usize,
    pub rules_by_language: Vec<(String, usize)>,
    pub violation_counts: ViolationCounts,
    pub violations_by_language: LanguageCounts,
    pub result: crate::tui::screens::result::ResultView,
    /// Debt report. `None` = not yet computed (open the Debt screen to compute it).
    /// `Some(Err(..))` = the compute failed and the screen shows the reason.
    pub debt: DebtView,
    /// Per-screen view state for the second-slice screens (whetstone-69jb).
    /// Each starts at `NotComputed` and transitions via its `ensure_*_loaded`
    /// method. Screens own their own data shape — see `src/tui/screens/*.rs`.
    pub rules: crate::tui::screens::rules::RulesView,
    pub sources: crate::tui::screens::sources::SourcesView,
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
    pub selected: usize,
    pub detail_selected: bool,
    pub detail_scroll_y: u16,
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

#[derive(Default, Clone)]
pub struct LanguageCounts {
    pub ts: usize,
    pub rust: usize,
    pub python: usize,
    pub all: usize,
}

impl App {
    pub fn new(project_dir: impl Into<PathBuf>) -> Result<Self> {
        let project_dir = project_dir.into();
        let mut app = Self {
            project_dir: project_dir.clone(),
            screen: Screen::Dashboard,
            quit: false,
            input_mode: InputMode::Normal,
            help: HelpState::default(),
            dashboard_ui: DashboardUiState::default(),
            dashboard: DashboardState::default(),
            sources_ui: SourcesUiState::default(),
            rules_ui: RulesUiState::default(),
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
            Msg::GoToScreen(s) => {
                self.screen = s;
                self.ensure_current_screen_loaded();
            }
            Msg::Key(ev) => self.handle_key(ev),
        }
    }

    /// Trigger the lazy loader for whichever screen is currently active.
    /// Screens that don't have a loader (Dashboard, Help) are no-ops.
    pub fn ensure_current_screen_loaded(&mut self) {
        crate::tui::screens::ensure_loaded(self.screen, self);
    }

    /// Each ensure_*_loaded method transitions `NotComputed` → `Loading` →
    /// `Ready`/`Error` synchronously. Wire the actual compute into the
    /// screen's `load` function in `src/tui/screens/<name>.rs`; the method
    /// below just drives the state machine.
    pub fn ensure_rules_loaded(&mut self) {
        if self.dashboard.rules.is_not_computed() {
            self.dashboard.rules = crate::tui::screens::rules::RulesView::Loading;
            self.dashboard.rules = crate::tui::screens::rules::load(&self.project_dir);
        }
    }

    pub fn ensure_sources_loaded(&mut self) {
        if self.dashboard.sources.is_not_computed() {
            self.dashboard.sources = crate::tui::screens::sources::SourcesView::Loading;
            self.dashboard.sources = crate::tui::screens::sources::load(&self.project_dir);
        }
    }

    pub fn ensure_check_loaded(&mut self) {
        if self.dashboard.check.is_not_computed() {
            self.dashboard.check = crate::tui::screens::check::CheckView::Loading;
            self.dashboard.check = crate::tui::screens::check::load(&self.project_dir);
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
        self.dashboard.debt = load_debt_view(&self.project_dir);
    }

    fn handle_key(&mut self, ev: KeyEvent) {
        if self.input_mode != InputMode::Normal {
            self.handle_form_key(ev);
            return;
        }

        // Global keybinds — available on every screen.
        if ev.modifiers.contains(KeyModifiers::CONTROL) && matches!(ev.code, KeyCode::Char('c')) {
            self.quit = true;
            return;
        }

        match ev.code {
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char(c) if Screen::from_nav_key(c).is_some() => {
                if let Some(screen) = Screen::from_nav_key(c) {
                    self.update(Msg::GoToScreen(screen));
                }
            }
            KeyCode::Char('?') => self.screen = Screen::Help,
            KeyCode::Char('a') | KeyCode::Char('A') => match self.screen {
                Screen::Sources
                    if matches!(
                        self.sources_ui.dataset,
                        SourcesDataset::Personal | SourcesDataset::Team
                    ) =>
                {
                    self.open_sources_form()
                }
                Screen::Rules => self.open_rules_form(),
                _ => {}
            },
            KeyCode::Tab if self.screen == Screen::Sources => {
                self.sources_ui.detail_selected = !self.sources_ui.detail_selected;
            }
            KeyCode::BackTab if self.screen == Screen::Sources => {
                self.sources_ui.detail_selected = !self.sources_ui.detail_selected;
            }
            KeyCode::Tab if self.screen == Screen::Debt => {
                if let DebtView::Ready(data) = &mut self.dashboard.debt {
                    data.detail_selected = !data.detail_selected;
                }
            }
            KeyCode::Char('d') | KeyCode::Char('D') if self.screen == Screen::Sources => {
                self.sources_ui.dataset = SourcesDataset::Dependencies;
                self.sources_ui.selected = 0;
                self.sources_ui.detail_scroll_y = 0;
            }
            KeyCode::Char('p') | KeyCode::Char('P') if self.screen == Screen::Sources => {
                self.sources_ui.dataset = SourcesDataset::Personal;
                self.sources_ui.selected = 0;
                self.sources_ui.detail_scroll_y = 0;
            }
            KeyCode::Char('t') | KeyCode::Char('T') if self.screen == Screen::Sources => {
                self.sources_ui.dataset = SourcesDataset::Team;
                self.sources_ui.selected = 0;
                self.sources_ui.detail_scroll_y = 0;
            }
            KeyCode::Left | KeyCode::Char('h')
                if self.screen == Screen::Sources && !self.sources_ui.detail_selected =>
            {
                self.sources_ui.dataset = self.sources_ui.dataset.prev();
                self.sources_ui.selected = 0;
                self.sources_ui.detail_scroll_y = 0;
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.screen == Screen::Sources && !self.sources_ui.detail_selected =>
            {
                self.sources_ui.dataset = self.sources_ui.dataset.next();
                self.sources_ui.selected = 0;
                self.sources_ui.detail_scroll_y = 0;
            }
            KeyCode::Left | KeyCode::Char('h') if self.screen == Screen::Rules => {
                self.rules_ui.language_filter = self.rules_ui.language_filter.prev();
                self.rules_ui.selected = 0;
            }
            KeyCode::Right | KeyCode::Char('l') if self.screen == Screen::Rules => {
                self.rules_ui.language_filter = self.rules_ui.language_filter.next();
                self.rules_ui.selected = 0;
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
                Screen::Dashboard => {
                    self.dashboard_ui.scroll = self.dashboard_ui.scroll.saturating_sub(1)
                }
                Screen::Help => self.help.scroll_y = self.help.scroll_y.saturating_sub(1),
                Screen::Result => self.dashboard.result.scroll_up(1),
                Screen::Debt => self.dashboard.debt.select_prev(),
                Screen::Sources => match self.sources_ui.dataset {
                    SourcesDataset::Dependencies if self.sources_ui.detail_selected => {
                        self.sources_ui.detail_scroll_y =
                            self.sources_ui.detail_scroll_y.saturating_sub(1)
                    }
                    SourcesDataset::Dependencies => {
                        self.sources_ui.selected = self.sources_ui.selected.saturating_sub(1);
                        self.sources_ui.detail_scroll_y = 0;
                    }
                    SourcesDataset::Personal | SourcesDataset::Team => {
                        if self.sources_ui.detail_selected {
                            self.sources_ui.detail_scroll_y =
                                self.sources_ui.detail_scroll_y.saturating_sub(1);
                        } else {
                            self.sources_ui.selected = self.sources_ui.selected.saturating_sub(1);
                            self.sources_ui.detail_scroll_y = 0;
                        }
                    }
                },
                Screen::Rules => self.rules_ui.selected = self.rules_ui.selected.saturating_sub(1),
                Screen::Check => self.dashboard.check.select_prev(),
            }
        }
    }

    fn select_next_on_current_screen(&mut self, steps: usize) {
        for _ in 0..steps {
            match self.screen {
                Screen::Dashboard => {
                    self.dashboard_ui.scroll = self.dashboard_ui.scroll.saturating_add(1)
                }
                Screen::Help => self.help.scroll_y = self.help.scroll_y.saturating_add(1),
                Screen::Result => self.dashboard.result.scroll_down(1),
                Screen::Debt => self.dashboard.debt.select_next(),
                Screen::Sources => match self.sources_ui.dataset {
                    SourcesDataset::Dependencies if self.sources_ui.detail_selected => {
                        self.sources_ui.detail_scroll_y =
                            self.sources_ui.detail_scroll_y.saturating_add(1)
                    }
                    SourcesDataset::Dependencies => {
                        let max = self
                            .dashboard
                            .sources
                            .row_count_for(SourcesDataset::Dependencies)
                            .saturating_sub(1);
                        if self.sources_ui.selected < max {
                            self.sources_ui.selected += 1;
                        }
                        self.sources_ui.detail_scroll_y = 0;
                    }
                    SourcesDataset::Personal => {
                        if self.sources_ui.detail_selected {
                            self.sources_ui.detail_scroll_y =
                                self.sources_ui.detail_scroll_y.saturating_add(1);
                        } else {
                            let max = self
                                .dashboard
                                .sources
                                .row_count_for(SourcesDataset::Personal)
                                .saturating_sub(1);
                            if self.sources_ui.selected < max {
                                self.sources_ui.selected += 1;
                            }
                            self.sources_ui.detail_scroll_y = 0;
                        }
                    }
                    SourcesDataset::Team => {
                        if self.sources_ui.detail_selected {
                            self.sources_ui.detail_scroll_y =
                                self.sources_ui.detail_scroll_y.saturating_add(1);
                        } else {
                            let max = self
                                .dashboard
                                .sources
                                .row_count_for(SourcesDataset::Team)
                                .saturating_sub(1);
                            if self.sources_ui.selected < max {
                                self.sources_ui.selected += 1;
                            }
                            self.sources_ui.detail_scroll_y = 0;
                        }
                    }
                },
                Screen::Rules => {
                    let max = self
                        .dashboard
                        .rules
                        .row_count_for(self.rules_ui.language_filter)
                        .saturating_sub(1);
                    if self.rules_ui.selected < max {
                        self.rules_ui.selected += 1;
                    }
                }
                Screen::Check => self.dashboard.check.select_next(),
            }
        }
    }

    fn scroll_left_on_current_screen(&mut self, steps: u16) {
        match self.screen {
            Screen::Help => self.help.scroll_x = self.help.scroll_x.saturating_sub(steps),
            Screen::Result => self.dashboard.result.scroll_left(steps),
            Screen::Debt => self.dashboard.debt.scroll_left(steps),
            _ => {}
        }
    }

    fn scroll_right_on_current_screen(&mut self, steps: u16) {
        match self.screen {
            Screen::Help => self.help.scroll_x = self.help.scroll_x.saturating_add(steps),
            Screen::Result => self.dashboard.result.scroll_right(steps),
            Screen::Debt => self.dashboard.debt.scroll_right(steps),
            _ => {}
        }
    }

    fn open_sources_form(&mut self) {
        self.sources_ui.form = SourcesFormState {
            active_field: 0,
            team_scope: self.sources_ui.dataset == SourcesDataset::Team,
            url: String::new(),
            language_idx: 3,
            error: None,
        };
        self.sources_ui.detail_selected = true;
        self.input_mode = InputMode::SourcesAdd;
    }

    fn open_rules_form(&mut self) {
        self.rules_ui.form = RulesFormState {
            active_field: 1,
            ..RulesFormState::default()
        };
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
                self.sources_ui.form.error = None;
                self.sources_ui.detail_selected = false;
            }
            KeyCode::Tab => {
                self.sources_ui.form.active_field = (self.sources_ui.form.active_field + 1) % 2;
            }
            KeyCode::BackTab => {
                self.sources_ui.form.active_field =
                    self.sources_ui.form.active_field.saturating_sub(1);
            }
            KeyCode::Backspace if self.sources_ui.form.active_field == 0 => {
                self.current_sources_field_mut().pop();
            }
            KeyCode::Enter => self.submit_sources_form(),
            KeyCode::Left | KeyCode::Char('h') if self.sources_ui.form.active_field == 1 => {
                self.sources_ui.form.language_idx =
                    self.sources_ui.form.language_idx.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') if self.sources_ui.form.active_field == 1 => {
                self.sources_ui.form.language_idx = (self.sources_ui.form.language_idx + 1).min(3);
            }
            KeyCode::Char(c) if self.sources_ui.form.active_field == 0 => {
                self.current_sources_field_mut().push(c);
            }
            _ => {}
        }
    }

    fn handle_rules_form_key(&mut self, ev: KeyEvent) {
        match ev.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.rules_ui.form.error = None;
            }
            KeyCode::Tab => {
                self.rules_ui.form.active_field = (self.rules_ui.form.active_field + 1) % 5;
            }
            KeyCode::BackTab => {
                self.rules_ui.form.active_field = self.rules_ui.form.active_field.saturating_sub(1);
            }
            KeyCode::Backspace => {
                if matches!(self.rules_ui.form.active_field, 1 | 4) {
                    self.current_rules_field_mut().pop();
                }
            }
            KeyCode::Enter => self.submit_rules_form(),
            KeyCode::Char('s') | KeyCode::Char('S')
                if ev.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.submit_rules_form();
            }
            KeyCode::Left | KeyCode::Char('h') if self.rules_ui.form.active_field == 0 => {
                self.rules_ui.form.team_scope = !self.rules_ui.form.team_scope;
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_ui.form.active_field == 0 => {
                self.rules_ui.form.team_scope = !self.rules_ui.form.team_scope;
            }
            KeyCode::Left | KeyCode::Char('h') if self.rules_ui.form.active_field == 2 => {
                self.rules_ui.form.language_idx = self.rules_ui.form.language_idx.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_ui.form.active_field == 2 => {
                self.rules_ui.form.language_idx = (self.rules_ui.form.language_idx + 1).min(3);
            }
            KeyCode::Left | KeyCode::Char('h') if self.rules_ui.form.active_field == 3 => {
                self.rules_ui.form.severity_idx = self.rules_ui.form.severity_idx.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_ui.form.active_field == 3 => {
                self.rules_ui.form.severity_idx = (self.rules_ui.form.severity_idx + 1).min(2);
            }
            KeyCode::Char(c) if matches!(self.rules_ui.form.active_field, 1 | 4) => {
                self.current_rules_field_mut().push(c);
            }
            _ => {}
        }
    }

    fn current_sources_field_mut(&mut self) -> &mut String {
        &mut self.sources_ui.form.url
    }

    fn current_rules_field_mut(&mut self) -> &mut String {
        match self.rules_ui.form.active_field {
            1 => &mut self.rules_ui.form.name,
            4 => &mut self.rules_ui.form.rule_text,
            _ => &mut self.rules_ui.form.rule_text,
        }
    }

    fn submit_sources_form(&mut self) {
        match crate::source_mgmt::add(
            &self.project_dir,
            crate::source_mgmt::AddOptions {
                url: self.sources_ui.form.url.trim(),
                name: None,
                language: Some(source_form_language(self.sources_ui.form.language_idx)),
                source_kind: None,
                personal: !self.sources_ui.form.team_scope,
            },
        ) {
            Ok(_) => {
                self.dashboard.sources = crate::tui::screens::sources::SourcesView::NotComputed;
                self.ensure_sources_loaded();
                self.input_mode = InputMode::Normal;
                self.sources_ui.form = SourcesFormState::default();
                self.sources_ui.detail_selected = false;
            }
            Err(e) => self.sources_ui.form.error = Some(e.to_string()),
        }
    }

    fn submit_rules_form(&mut self) {
        let slug = slugify_rule_name(&self.rules_ui.form.name);
        let severity = rule_form_severity(self.rules_ui.form.severity_idx);
        if slug.is_empty() {
            self.rules_ui.form.error =
                Some("Rule name must contain at least one letter or number.".into());
            return;
        }
        if self.rules_ui.form.rule_text.trim().is_empty() {
            self.rules_ui.form.error = Some("Rule text must be non-empty.".into());
            return;
        }
        let languages: &[&str] = match self.rules_ui.form.language_idx {
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
            self.rules_ui.form.error = Some(format!(
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
                    description: self.rules_ui.form.rule_text.trim(),
                    match_regex: None,
                    severity,
                    confidence: "high",
                    category: "convention",
                    language,
                    source_url: None,
                    dep: Some("custom"),
                    personal: !self.rules_ui.form.team_scope,
                },
            ) {
                errors.push(e.to_string());
            }
        }

        if errors.is_empty() {
            self.dashboard.rules = crate::tui::screens::rules::RulesView::NotComputed;
            self.ensure_rules_loaded();
            self.input_mode = InputMode::Normal;
            self.rules_ui.form = RulesFormState::default();
            self.rules_ui.language_filter = RulesLanguageFilter::All;
            self.rules_ui.selected = 0;
        } else {
            self.rules_ui.form.error = Some(errors.join("\n"));
        }
    }
}

pub fn rule_form_severity(idx: usize) -> &'static str {
    match idx {
        0 => "must",
        1 => "should",
        _ => "may",
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
        d.adherence_detail = status.get("adherence").cloned().unwrap_or(Value::Null);

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
        d.sources_internal = handoff
            .get("worklist")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0);
    }
    if let Ok(custom) = crate::source_mgmt::list(project_dir) {
        d.sources_external = custom.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    }
    d.sources_total = d.sources_internal + d.sources_external;

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
            for v in arr {
                match v.get("language").and_then(|s| s.as_str()).unwrap_or("") {
                    "typescript" => d.violations_by_language.ts += 1,
                    "rust" => d.violations_by_language.rust += 1,
                    "python" => d.violations_by_language.python += 1,
                    _ => d.violations_by_language.all += 1,
                }
            }
        }
    }

    d.debt = DebtView::NotComputed;

    d
}

fn load_debt_view(project_dir: &Path) -> DebtView {
    let opts = crate::debt::DebtOptions {
        project_dir: project_dir.to_path_buf(),
        top: usize::MAX,
        min_confidence: crate::debt::types::Confidence::Medium,
        since_days: 90,
    };
    match crate::debt::run(&opts) {
        Ok(report) => build_debt_view(report),
        Err(e) => DebtView::Error(e.to_string()),
    }
}

fn build_debt_view(report: crate::debt::types::DebtReport) -> DebtView {
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
            impact_level: impact_level(normalize_impact_percent(h.score, max_score)).to_string(),
        })
        .collect();

    DebtView::Ready(Box::new(DebtSummaryView {
        debt_label: report.summary.debt_label.as_str().to_string(),
        finding_count: report.summary.finding_count,
        by_dead: report.summary.by_category.dead,
        by_dup: report.summary.by_category.dup,
        by_deps: report.summary.by_category.deps,
        selected: 0,
        detail_selected: false,
        detail_scroll_y: 0,
        scroll_x: 0,
        hotspots,
    }))
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
        Evidence::SymbolDef {
            name, symbol_kind, ..
        } => {
            format!("{symbol_kind}: {name}")
        }
        Evidence::DuplicateCluster { snippet, .. } => truncate_inline(snippet, 220),
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

fn source_form_language(idx: usize) -> &'static str {
    match idx {
        0 => "typescript",
        1 => "python",
        2 => "rust",
        _ => "any",
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn submit_rules_form_persists_selected_severity() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_submit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        let mut app = App::new(&tmp).unwrap();
        app.rules_ui.form = RulesFormState {
            name: "Critical Rule".into(),
            language_idx: 2,
            severity_idx: 0,
            rule_text: "Never do the bad thing.".into(),
            ..RulesFormState::default()
        };

        app.submit_rules_form();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (rules, _warnings) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "custom.critical-rule");
        assert_eq!(rules[0].severity, "must");

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rules_screen_left_right_cycles_language_filters() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_filter_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Rules;

        app.update(Msg::Key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)));
        assert_eq!(app.rules_ui.language_filter, RulesLanguageFilter::Python);

        app.update(Msg::Key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)));
        assert_eq!(app.rules_ui.language_filter, RulesLanguageFilter::All);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
