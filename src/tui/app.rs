//! `App` is the root Elm-architecture model for the TUI.
//!
//! `update(&mut self, msg)` mutates state; `view(&self, frame)` renders.
//! Screen-specific state lives on sub-structs under `App`.

use std::collections::BTreeMap;
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
    pub onboard: crate::tui::screens::onboard::OnboardState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    SourcesAdd,
    RulesAdd,
    RulesEdit,
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
    pub detail_selected: bool,
    pub detail_scroll_y: u16,
    pub detail_message: Option<String>,
    pub editing: Option<RulesEditState>,
    pub form: RulesFormState,
}

#[derive(Debug, Clone)]
pub struct RulesEditState {
    pub original_row: crate::tui::screens::rules::RuleRow,
}

#[derive(Debug, Clone)]
pub struct RulesFormState {
    pub active_field: usize,
    pub team_scope: bool,
    pub name: String,
    pub language_idx: usize,
    pub severity_idx: usize,
    pub mode_idx: usize,
    pub rule_text: String,
    pub detail_a: String,
    pub detail_b: String,
    pub detail_c: String,
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
            mode_idx: 1,
            rule_text: String::new(),
            detail_a: String::new(),
            detail_b: String::new(),
            detail_c: String::new(),
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
            onboard: crate::tui::screens::onboard::OnboardState::load(&project_dir),
        };
        app.load_dashboard();
        Ok(app)
    }

    /// If this project isn't set up yet and the user hasn't dismissed onboarding,
    /// open the wizard instead of the dashboard (whetstone-arx). Derived purely
    /// from `setup_status` — no stored "seen it" flag beyond `setup.dismissed`.
    pub fn start_onboarding_if_needed(&mut self) {
        let complete = self
            .onboard
            .setup
            .get("complete")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let dismissed = self
            .onboard
            .setup
            .get("dismissed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !complete && !dismissed {
            self.screen = Screen::Onboard;
        }
    }

    /// Best-effort load of the dashboard data. Errors are swallowed and
    /// surface as empty fields — the TUI must never panic on bad project state.
    pub fn load_dashboard(&mut self) {
        self.dashboard = collect_dashboard(&self.project_dir);
    }

    pub fn start_intro(&mut self) {
        self.screen = Screen::Intro;
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

        if self.screen == Screen::Intro {
            self.handle_intro_key(ev);
            return;
        }

        // The onboarding wizard owns all of its keys (whetstone-v5n). Ctrl-C
        // above still quits; everything else routes to its step machine.
        if self.screen == Screen::Onboard {
            use crate::tui::screens::onboard::Outcome;
            match self.onboard.on_key(ev.code, &self.project_dir) {
                Outcome::Stay => {}
                Outcome::Exit => {
                    self.onboard = crate::tui::screens::onboard::OnboardState::load(&self.project_dir);
                    self.load_dashboard();
                    self.screen = Screen::Dashboard;
                }
            }
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
            KeyCode::Char('e') | KeyCode::Char('E') if self.screen == Screen::Rules => {
                self.open_rules_edit_form()
            }
            KeyCode::Char('d') | KeyCode::Char('D') if self.screen == Screen::Rules => {
                self.delete_selected_rule()
            }
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
            KeyCode::Tab if self.screen == Screen::Rules => {
                self.rules_ui.detail_selected = !self.rules_ui.detail_selected;
            }
            KeyCode::BackTab if self.screen == Screen::Rules => {
                self.rules_ui.detail_selected = !self.rules_ui.detail_selected;
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
            KeyCode::Left | KeyCode::Char('h')
                if self.screen == Screen::Rules && !self.rules_ui.detail_selected =>
            {
                self.rules_ui.language_filter = self.rules_ui.language_filter.prev();
                self.rules_ui.selected = 0;
                self.rules_ui.detail_scroll_y = 0;
                self.rules_ui.detail_message = None;
            }
            KeyCode::Right | KeyCode::Char('l')
                if self.screen == Screen::Rules && !self.rules_ui.detail_selected =>
            {
                self.rules_ui.language_filter = self.rules_ui.language_filter.next();
                self.rules_ui.selected = 0;
                self.rules_ui.detail_scroll_y = 0;
                self.rules_ui.detail_message = None;
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

    fn handle_intro_key(&mut self, ev: KeyEvent) {
        match ev.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => self.quit = true,
            KeyCode::Char('?') => self.finish_intro(Screen::Help),
            KeyCode::Char(c) => {
                if let Some(screen) = Screen::from_nav_key(c) {
                    self.finish_intro(screen);
                } else {
                    self.finish_intro(Screen::Dashboard);
                }
            }
            _ => self.finish_intro(Screen::Dashboard),
        }
    }

    fn finish_intro(&mut self, next_screen: Screen) {
        self.screen = next_screen;
        // Leaving the splash for the dashboard on an un-onboarded project opens
        // the setup wizard instead (whetstone-arx). An explicit nav key (1-5)
        // overrides — the user asked for a specific screen.
        if next_screen == Screen::Dashboard {
            self.start_onboarding_if_needed();
        }
        self.ensure_current_screen_loaded();
    }

    /// Move selection one step backward on whichever list-oriented screen is
    /// active. No-op on screens without a selectable list.
    fn select_prev_on_current_screen(&mut self, steps: usize) {
        for _ in 0..steps {
            match self.screen {
                Screen::Dashboard => {
                    self.dashboard_ui.scroll = self.dashboard_ui.scroll.saturating_sub(1)
                }
                Screen::Intro | Screen::Onboard => {}
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
                Screen::Rules => {
                    if self.rules_ui.detail_selected {
                        self.rules_ui.detail_scroll_y =
                            self.rules_ui.detail_scroll_y.saturating_sub(1);
                    } else {
                        self.rules_ui.selected = self.rules_ui.selected.saturating_sub(1);
                        self.rules_ui.detail_scroll_y = 0;
                        self.rules_ui.detail_message = None;
                    }
                }
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
                Screen::Intro | Screen::Onboard => {}
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
                    if self.rules_ui.detail_selected {
                        self.rules_ui.detail_scroll_y =
                            self.rules_ui.detail_scroll_y.saturating_add(1);
                    } else {
                        let max = self
                            .dashboard
                            .rules
                            .row_count_for(self.rules_ui.language_filter)
                            .saturating_sub(1);
                        if self.rules_ui.selected < max {
                            self.rules_ui.selected += 1;
                        }
                        self.rules_ui.detail_scroll_y = 0;
                        self.rules_ui.detail_message = None;
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
        self.rules_ui.editing = None;
        self.rules_ui.detail_message = None;
        self.rules_ui.detail_selected = true;
        self.rules_ui.detail_scroll_y = 0;
        self.input_mode = InputMode::RulesAdd;
    }

    fn open_rules_edit_form(&mut self) {
        let Some(row) = self.current_selected_rules_row() else {
            return;
        };
        if !is_authored_rule_row(&row) {
            self.rules_ui.detail_message =
                Some("Only Personal and Team rules can be edited here.".into());
            return;
        }

        self.rules_ui.form = match rules_form_from_row(&row) {
            Ok(form) => form,
            Err(err) => {
                self.rules_ui.detail_message = Some(err.to_string());
                return;
            }
        };
        self.rules_ui.editing = Some(RulesEditState { original_row: row });
        self.rules_ui.detail_message = None;
        self.rules_ui.detail_selected = true;
        self.rules_ui.detail_scroll_y = 0;
        self.input_mode = InputMode::RulesEdit;
    }

    fn handle_form_key(&mut self, ev: KeyEvent) {
        match self.input_mode {
            InputMode::Normal => {}
            InputMode::SourcesAdd => self.handle_sources_form_key(ev),
            InputMode::RulesAdd | InputMode::RulesEdit => self.handle_rules_form_key(ev),
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
                self.sources_ui.form.language_idx = wrap_prev(self.sources_ui.form.language_idx, 4);
            }
            KeyCode::Right | KeyCode::Char('l') if self.sources_ui.form.active_field == 1 => {
                self.sources_ui.form.language_idx = wrap_next(self.sources_ui.form.language_idx, 4);
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
                self.rules_ui.editing = None;
            }
            KeyCode::Tab => {
                self.rules_ui.form.active_field = (self.rules_ui.form.active_field + 1)
                    % rule_form_field_count(self.rules_ui.form.mode_idx);
            }
            KeyCode::BackTab => {
                self.rules_ui.form.active_field = if self.rules_ui.form.active_field == 0 {
                    rule_form_field_count(self.rules_ui.form.mode_idx).saturating_sub(1)
                } else {
                    self.rules_ui.form.active_field.saturating_sub(1)
                };
            }
            KeyCode::Backspace => {
                if matches!(self.rules_ui.form.active_field, 1 | 5 | 6 | 7 | 8) {
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
                self.rules_ui.form.language_idx = wrap_prev(self.rules_ui.form.language_idx, 4);
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_ui.form.active_field == 2 => {
                self.rules_ui.form.language_idx = wrap_next(self.rules_ui.form.language_idx, 4);
            }
            KeyCode::Left | KeyCode::Char('h') if self.rules_ui.form.active_field == 3 => {
                self.rules_ui.form.severity_idx = wrap_prev(self.rules_ui.form.severity_idx, 3);
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_ui.form.active_field == 3 => {
                self.rules_ui.form.severity_idx = wrap_next(self.rules_ui.form.severity_idx, 3);
            }
            KeyCode::Left | KeyCode::Char('h') if self.rules_ui.form.active_field == 4 => {
                self.rules_ui.form.mode_idx = wrap_prev(self.rules_ui.form.mode_idx, 5);
                clamp_rule_form_active_field(&mut self.rules_ui.form);
            }
            KeyCode::Right | KeyCode::Char('l') if self.rules_ui.form.active_field == 4 => {
                self.rules_ui.form.mode_idx = wrap_next(self.rules_ui.form.mode_idx, 5);
                clamp_rule_form_active_field(&mut self.rules_ui.form);
            }
            KeyCode::Char(c) if matches!(self.rules_ui.form.active_field, 1 | 5 | 6 | 7 | 8) => {
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
            5 => &mut self.rules_ui.form.rule_text,
            6 => &mut self.rules_ui.form.detail_a,
            7 => &mut self.rules_ui.form.detail_b,
            8 => &mut self.rules_ui.form.detail_c,
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
        if self.rules_ui.editing.is_some() {
            self.submit_rules_edit_form();
            return;
        }

        let enforcement = match rules_form_enforcement(&self.rules_ui.form) {
            Ok(enforcement) => enforcement,
            Err(err) => {
                self.rules_ui.form.error = Some(err.to_string());
                return;
            }
        };

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
        let languages = rules_form_languages(self.rules_ui.form.language_idx);
        if matches!(
            enforcement,
            crate::rule_authoring::EnforcementMode::Lint { .. }
                | crate::rule_authoring::EnforcementMode::Formatter { .. }
                | crate::rule_authoring::EnforcementMode::Test { .. }
        ) && !is_single_concrete_language_selection(self.rules_ui.form.language_idx)
        {
            self.rules_ui.form.error = Some(
                "This enforcement mode requires a single language selection in the TUI.".into(),
            );
            return;
        }
        let planned_ids: Vec<String> = if languages == ["all"] {
            vec![format!("custom.{slug}")]
        } else {
            languages
                .iter()
                .map(|language| {
                    if languages.len() == 1 {
                        format!("custom.{slug}")
                    } else {
                        format!("custom.{slug}-{language}")
                    }
                })
                .collect()
        };
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
                    rule_id: rule_id.clone(),
                    description: self.rules_ui.form.rule_text.trim().to_string(),
                    severity: severity.to_string(),
                    confidence: "high".to_string(),
                    category: "convention".to_string(),
                    language: (*language).to_string(),
                    source_url: None,
                    dep: Some("custom".to_string()),
                    enforcement: enforcement.clone(),
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
            self.rules_ui.detail_selected = false;
            self.rules_ui.detail_scroll_y = 0;
            self.rules_ui.detail_message = None;
        } else {
            self.rules_ui.form.error = Some(errors.join("\n"));
        }
    }

    fn submit_rules_edit_form(&mut self) {
        let Some(editing) = self.rules_ui.editing.clone() else {
            self.input_mode = InputMode::Normal;
            return;
        };

        let enforcement = match rules_form_enforcement(&self.rules_ui.form) {
            Ok(enforcement) => enforcement,
            Err(err) => {
                self.rules_ui.form.error = Some(err.to_string());
                return;
            }
        };

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

        let languages = rules_form_languages(self.rules_ui.form.language_idx);
        if matches!(
            enforcement,
            crate::rule_authoring::EnforcementMode::Lint { .. }
                | crate::rule_authoring::EnforcementMode::Formatter { .. }
                | crate::rule_authoring::EnforcementMode::Test { .. }
        ) && !is_single_concrete_language_selection(self.rules_ui.form.language_idx)
        {
            self.rules_ui.form.error = Some(
                "This enforcement mode requires a single language selection in the TUI.".into(),
            );
            return;
        }
        let planned_ids: Vec<String> = if languages == ["all"] {
            vec![format!("custom.{slug}")]
        } else {
            languages
                .iter()
                .map(|language| {
                    if languages.len() == 1 {
                        format!("custom.{slug}")
                    } else {
                        format!("custom.{slug}-{language}")
                    }
                })
                .collect()
        };

        if let Some(existing) = first_existing_rule_id_except(
            &self.project_dir,
            &planned_ids,
            &editing.original_row.member_ids,
        ) {
            self.rules_ui.form.error = Some(format!(
                "Rule `{existing}` already exists. Choose a different name or remove the existing rule first."
            ));
            return;
        }

        let mut removed_ids = Vec::new();
        for rule_id in &editing.original_row.member_ids {
            match crate::rule_authoring::remove(
                &self.project_dir,
                crate::rule_authoring::RemoveOptions { rule_id },
            ) {
                Ok(_) => removed_ids.push(rule_id.clone()),
                Err(e) => {
                    let _ = restore_authored_rule(
                        &self.project_dir,
                        &editing.original_row,
                        &removed_ids,
                    );
                    self.rules_ui.form.error = Some(e.to_string());
                    return;
                }
            }
        }

        let mut created_ids = Vec::new();
        for (language, rule_id) in languages.iter().zip(planned_ids.iter()) {
            match crate::rule_authoring::add(
                &self.project_dir,
                crate::rule_authoring::AddOptions {
                    rule_id: rule_id.clone(),
                    description: self.rules_ui.form.rule_text.trim().to_string(),
                    severity: severity.to_string(),
                    confidence: "high".to_string(),
                    category: "convention".to_string(),
                    language: (*language).to_string(),
                    source_url: None,
                    dep: Some("custom".to_string()),
                    enforcement: enforcement.clone(),
                    personal: !self.rules_ui.form.team_scope,
                },
            ) {
                Ok(_) => created_ids.push(rule_id.clone()),
                Err(e) => {
                    for created_id in &created_ids {
                        let _ = crate::rule_authoring::remove(
                            &self.project_dir,
                            crate::rule_authoring::RemoveOptions {
                                rule_id: created_id,
                            },
                        );
                    }
                    let _ = restore_authored_rule(
                        &self.project_dir,
                        &editing.original_row,
                        &editing.original_row.member_ids,
                    );
                    self.rules_ui.form.error = Some(e.to_string());
                    return;
                }
            }
        }

        self.dashboard.rules = crate::tui::screens::rules::RulesView::NotComputed;
        self.ensure_rules_loaded();
        self.input_mode = InputMode::Normal;
        self.rules_ui.form = RulesFormState::default();
        self.rules_ui.editing = None;
        self.rules_ui.detail_message = None;
        self.rules_ui.language_filter = RulesLanguageFilter::All;
        self.rules_ui.selected = 0;
        self.rules_ui.detail_selected = false;
        self.rules_ui.detail_scroll_y = 0;
    }

    fn delete_selected_rule(&mut self) {
        let Some(row) = self.current_selected_rules_row() else {
            return;
        };
        if !is_authored_rule_row(&row) {
            self.rules_ui.detail_message =
                Some("Only Personal and Team rules can be deleted here.".into());
            return;
        }

        let mut errors = Vec::new();
        for rule_id in &row.member_ids {
            if let Err(e) = crate::rule_authoring::remove(
                &self.project_dir,
                crate::rule_authoring::RemoveOptions { rule_id },
            ) {
                errors.push(e.to_string());
            }
        }

        if errors.is_empty() {
            self.dashboard.rules = crate::tui::screens::rules::RulesView::NotComputed;
            self.ensure_rules_loaded();
            let max = self
                .dashboard
                .rules
                .row_count_for(self.rules_ui.language_filter)
                .saturating_sub(1);
            self.rules_ui.selected = self.rules_ui.selected.min(max);
            self.rules_ui.detail_scroll_y = 0;
            self.rules_ui.detail_message = None;
        } else {
            self.rules_ui.detail_message = Some(errors.join("\n"));
        }
    }

    fn current_selected_rules_row(&self) -> Option<crate::tui::screens::rules::RuleRow> {
        self.dashboard
            .rules
            .selected_row(self.rules_ui.language_filter, self.rules_ui.selected)
    }
}

pub fn rule_form_severity(idx: usize) -> &'static str {
    match idx {
        0 => "must",
        1 => "should",
        _ => "may",
    }
}

pub fn rule_form_mode_label(idx: usize) -> &'static str {
    match idx {
        1 => "Pattern",
        2 => "Linter",
        3 => "Formatter",
        4 => "Test",
        _ => "Advisory",
    }
}

pub fn rule_form_field_count(mode_idx: usize) -> usize {
    match mode_idx {
        0 => 6,
        1 => 7,
        2 => 8,
        3 | 4 => 9,
        _ => 6,
    }
}

fn clamp_rule_form_active_field(form: &mut RulesFormState) {
    let max = rule_form_field_count(form.mode_idx).saturating_sub(1);
    form.active_field = form.active_field.min(max);
}

fn wrap_prev(idx: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else if idx == 0 {
        len - 1
    } else {
        idx - 1
    }
}

fn wrap_next(idx: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        (idx + 1) % len
    }
}

fn rule_form_severity_idx(severity: &str) -> usize {
    match severity {
        "must" => 0,
        "should" => 1,
        _ => 2,
    }
}

fn rule_form_language_idx(languages: &[String]) -> usize {
    match languages {
        [language] => match language.as_str() {
            "typescript" => 0,
            "rust" => 1,
            "python" => 2,
            _ => 3,
        },
        _ => 3,
    }
}

fn rules_form_languages(idx: usize) -> &'static [&'static str] {
    match idx {
        0 => &["typescript"],
        1 => &["rust"],
        2 => &["python"],
        _ => &["all"],
    }
}

fn is_single_concrete_language_selection(idx: usize) -> bool {
    idx <= 2
}

fn rules_form_enforcement(form: &RulesFormState) -> Result<crate::rule_authoring::EnforcementMode> {
    match form.mode_idx {
        1 => {
            if form.detail_a.trim().is_empty() {
                Err(anyhow::anyhow!("Pattern mode requires a non-empty regex."))
            } else {
                Ok(crate::rule_authoring::EnforcementMode::Pattern {
                    regex: form.detail_a.trim().to_string(),
                    ast_scope: None,
                })
            }
        }
        2 => {
            if form.detail_a.trim().is_empty() || form.detail_b.trim().is_empty() {
                Err(anyhow::anyhow!("Linter mode requires a tool and code."))
            } else {
                Ok(crate::rule_authoring::EnforcementMode::Lint {
                    tool: form.detail_a.trim().to_string(),
                    code: form.detail_b.trim().to_string(),
                })
            }
        }
        3 => {
            if form.detail_a.trim().is_empty()
                || form.detail_b.trim().is_empty()
                || form.detail_c.trim().is_empty()
            {
                Err(anyhow::anyhow!(
                    "Formatter mode requires a tool, option key, and option value."
                ))
            } else {
                let mut options = BTreeMap::new();
                options.insert(
                    form.detail_b.trim().to_string(),
                    parse_rule_form_value(form.detail_c.trim()),
                );
                Ok(crate::rule_authoring::EnforcementMode::Formatter {
                    tool: form.detail_a.trim().to_string(),
                    options,
                })
            }
        }
        4 => {
            if form.detail_a.trim().is_empty() || form.detail_b.trim().is_empty() {
                Err(anyhow::anyhow!(
                    "Test mode requires a runner and test path."
                ))
            } else {
                Ok(crate::rule_authoring::EnforcementMode::Test {
                    runner: form.detail_a.trim().to_string(),
                    path: form.detail_b.trim().to_string(),
                    selector: (!form.detail_c.trim().is_empty())
                        .then(|| form.detail_c.trim().to_string()),
                })
            }
        }
        _ => Ok(crate::rule_authoring::EnforcementMode::Advisory),
    }
}

fn parse_rule_form_value(raw: &str) -> Value {
    if raw.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if raw.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(parsed) = raw.parse::<i64>() {
        return serde_json::json!(parsed);
    }
    if let Ok(parsed) = raw.parse::<f64>() {
        if parsed.is_finite() {
            return serde_json::json!(parsed);
        }
    }
    Value::String(raw.to_string())
}

fn rules_form_from_row(row: &crate::tui::screens::rules::RuleRow) -> Result<RulesFormState> {
    let enforcement = row_enforcement_for_form(row)?;
    Ok(RulesFormState {
        active_field: 1,
        team_scope: row.layer == "project",
        name: editable_rule_name(&row.id),
        language_idx: rule_form_language_idx(&row.languages),
        severity_idx: rule_form_severity_idx(&row.severity),
        mode_idx: enforcement.0,
        rule_text: row.description.clone(),
        detail_a: enforcement.1,
        detail_b: enforcement.2,
        detail_c: enforcement.3,
        error: None,
    })
}

fn row_enforcement_for_form(
    row: &crate::tui::screens::rules::RuleRow,
) -> Result<(usize, String, String, String)> {
    let surface_count = usize::from(!row.match_patterns.is_empty())
        + usize::from(!row.lint_bindings.is_empty())
        + usize::from(row.formatter.is_some())
        + usize::from(!row.tests.is_empty());
    if surface_count > 1 {
        return Err(anyhow::anyhow!(
            "This authored rule has multiple enforcement surfaces; edit it via CLI or YAML-backed workflow instead."
        ));
    }
    if row.match_patterns.len() > 1 {
        return Err(anyhow::anyhow!(
            "This authored rule has multiple match patterns; edit it via CLI instead."
        ));
    }
    if row.lint_bindings.len() > 1 {
        return Err(anyhow::anyhow!(
            "This authored rule has multiple lint bindings; edit it via CLI instead."
        ));
    }
    if row.tests.len() > 1 {
        return Err(anyhow::anyhow!(
            "This authored rule has multiple linked tests; edit it via CLI instead."
        ));
    }
    if let Some(pattern) = row.match_patterns.first() {
        return Ok((1, pattern.clone(), String::new(), String::new()));
    }
    if let Some(lint) = row.lint_bindings.first() {
        return Ok((2, lint.tool.clone(), lint.code.clone(), String::new()));
    }
    if let Some(formatter) = &row.formatter {
        if formatter.options.len() > 1 {
            return Err(anyhow::anyhow!(
                "This authored rule has multiple formatter options; edit it via CLI instead."
            ));
        }
        if let Some((key, value)) = formatter.options.iter().next() {
            return Ok((
                3,
                formatter.tool.clone(),
                key.clone(),
                formatter_value_to_string(value),
            ));
        }
        return Ok((3, formatter.tool.clone(), String::new(), String::new()));
    }
    if let Some(test) = row.tests.first() {
        return Ok((
            4,
            test.runner.clone(),
            test.path.clone(),
            test.selector.clone().unwrap_or_default(),
        ));
    }
    Ok((0, String::new(), String::new(), String::new()))
}

fn formatter_value_to_string(value: &Value) -> String {
    match value {
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        _ => value.to_string(),
    }
}

fn add_options_from_row(
    row: &crate::tui::screens::rules::RuleRow,
    rule_id: &str,
    language: &str,
) -> Result<crate::rule_authoring::AddOptions> {
    let (mode_idx, detail_a, detail_b, detail_c) = row_enforcement_for_form(row)?;
    let form = RulesFormState {
        active_field: 1,
        team_scope: row.layer == "project",
        name: editable_rule_name(&row.id),
        language_idx: rule_form_language_idx(&row.languages),
        severity_idx: rule_form_severity_idx(&row.severity),
        mode_idx,
        rule_text: row.description.clone(),
        detail_a,
        detail_b,
        detail_c,
        error: None,
    };
    Ok(crate::rule_authoring::AddOptions {
        rule_id: rule_id.to_string(),
        description: row.description.clone(),
        severity: row.severity.clone(),
        confidence: row.confidence.clone(),
        category: row.category.clone(),
        language: language.to_string(),
        source_url: None,
        dep: Some("custom".to_string()),
        enforcement: rules_form_enforcement(&form)?,
        personal: row.layer == "personal",
    })
}

fn editable_rule_name(rule_id: &str) -> String {
    rule_id
        .strip_prefix("custom.")
        .unwrap_or(rule_id)
        .to_string()
}

fn is_authored_rule_row(row: &crate::tui::screens::rules::RuleRow) -> bool {
    row.id.starts_with("custom.") || row.source_url.starts_with("personal://")
}

fn restore_authored_rule(
    project_dir: &Path,
    row: &crate::tui::screens::rules::RuleRow,
    rule_ids: &[String],
) -> Result<()> {
    for rule_id in rule_ids {
        let language = authored_rule_language(rule_id, &row.languages)?;
        let mut opts = add_options_from_row(row, rule_id, language)?;
        opts.personal = row.layer == "personal";
        crate::rule_authoring::add(project_dir, opts)?;
    }
    Ok(())
}

fn authored_rule_language<'a>(rule_id: &str, fallback_languages: &'a [String]) -> Result<&'a str> {
    if let Some(language) = rule_id.strip_suffix("-python") {
        let _ = language;
        return Ok("python");
    }
    if let Some(language) = rule_id.strip_suffix("-rust") {
        let _ = language;
        return Ok("rust");
    }
    if let Some(language) = rule_id.strip_suffix("-typescript") {
        let _ = language;
        return Ok("typescript");
    }
    if fallback_languages.len() == 3
        && fallback_languages
            .iter()
            .any(|language| language == "python")
        && fallback_languages.iter().any(|language| language == "rust")
        && fallback_languages
            .iter()
            .any(|language| language == "typescript")
    {
        return Ok("all");
    }

    fallback_languages
        .first()
        .map(|language| language.as_str())
        .ok_or_else(|| anyhow::anyhow!("unable to determine language for `{rule_id}`"))
}

fn first_existing_rule_id_except(
    project_dir: &Path,
    planned: &[String],
    ignored_ids: &[String],
) -> Option<String> {
    let ignored: std::collections::HashSet<&str> = ignored_ids.iter().map(String::as_str).collect();
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    for dir in [&paths.project_rules_dir, &paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = crate::rules::load_rule_files(dir);
        for file in files {
            for rule in file.rule_file.rules {
                if ignored.contains(rule.id.as_str()) {
                    continue;
                }
                if planned.iter().any(|id| id == &rule.id) {
                    return Some(rule.id);
                }
            }
        }
    }
    None
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
        injected_packs: &[],
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
        Evidence::ManifestEntry { snippet, .. } => snippet.clone(),
        Evidence::SymbolDef {
            name, symbol_kind, ..
        } => {
            format!("{symbol_kind}: {name}")
        }
        Evidence::DuplicateCluster { snippet, .. } => snippet.clone(),
        Evidence::OrphanedFile { path, .. } => path.clone(),
        Evidence::ChurnViolationIntersection {
            changes,
            violations,
            window_days,
            ..
        } => format!("{changes} changes and {violations} violations over {window_days}d"),
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
            detail_a: "bad_thing".into(),
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
    fn submit_all_language_rule_creates_one_persisted_rule() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_all_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        let mut app = App::new(&tmp).unwrap();
        app.rules_ui.form = RulesFormState {
            name: "Cross Language Rule".into(),
            language_idx: 3,
            severity_idx: 1,
            mode_idx: 1,
            rule_text: "Avoid this pattern everywhere.".into(),
            detail_a: "TODO".into(),
            ..RulesFormState::default()
        };

        app.submit_rules_form();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (files, _warnings) = crate::rules::load_rule_files(&paths.personal_rules_dir);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rule_file.rules.len(), 1);
        assert_eq!(files[0].rule_file.rules[0].id, "custom.cross-language-rule");
        assert_eq!(
            files[0].rule_file.rules[0].languages,
            vec!["python", "rust", "typescript"]
        );

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

    #[test]
    fn intro_any_key_opens_dashboard() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_intro_key_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        // Dismiss onboarding so intro → dashboard (an un-onboarded project would
        // route to the wizard — see intro_unonboarded_opens_wizard).
        crate::onboard::set_dismissed(&tmp, true).unwrap();
        let mut app = App::new(&tmp).unwrap();
        app.start_intro();

        app.update(Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));

        assert_eq!(app.screen, Screen::Dashboard);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn intro_unonboarded_opens_wizard() {
        // whetstone-arx: leaving the splash on a fresh (un-onboarded, not
        // dismissed) project opens the setup wizard, not the dashboard.
        let tmp = std::env::temp_dir().join(format!("wh_tui_onboard_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.start_intro();
        app.update(Msg::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert_eq!(app.screen, Screen::Onboard);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn intro_nav_key_opens_requested_screen() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_intro_nav_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.start_intro();

        app.update(Msg::Key(KeyEvent::new(
            KeyCode::Char('3'),
            KeyModifiers::NONE,
        )));

        assert_eq!(app.screen, Screen::Rules);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn delete_selected_custom_rule_removes_grouped_rules() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_delete_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        for language in ["python", "rust", "typescript"] {
            crate::rule_authoring::add(
                &tmp,
                crate::rule_authoring::AddOptions {
                    rule_id: format!("custom.clean-up-{language}"),
                    description: "Temporary rule".into(),
                    severity: "should".into(),
                    confidence: "high".into(),
                    category: "convention".into(),
                    language: language.into(),
                    source_url: None,
                    dep: Some("custom".into()),
                    enforcement: crate::rule_authoring::EnforcementMode::Pattern {
                        regex: "temporary_rule".into(),
                        ast_scope: None,
                    },
                    personal: true,
                },
            )
            .unwrap();
        }

        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Rules;
        app.dashboard.rules = crate::tui::screens::rules::load(&tmp);

        app.delete_selected_rule();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (rules, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        assert!(rules.is_empty());

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn editing_selected_custom_rule_rewrites_scope_language_and_text() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_edit_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let _ = std::fs::create_dir_all(&tmp);

        crate::rule_authoring::add(
            &tmp,
            crate::rule_authoring::AddOptions {
                rule_id: "custom.initial-rule".into(),
                description: "Initial rule text".into(),
                severity: "should".into(),
                confidence: "high".into(),
                category: "convention".into(),
                language: "python".into(),
                source_url: None,
                dep: Some("custom".into()),
                enforcement: crate::rule_authoring::EnforcementMode::Pattern {
                    regex: "initial_rule".into(),
                    ast_scope: None,
                },
                personal: true,
            },
        )
        .unwrap();

        let mut app = App::new(&tmp).unwrap();
        app.screen = Screen::Rules;
        app.dashboard.rules = crate::tui::screens::rules::load(&tmp);
        app.rules_ui.language_filter = RulesLanguageFilter::Python;

        app.open_rules_edit_form();
        app.rules_ui.form.team_scope = true;
        app.rules_ui.form.name = "updated-rule".into();
        app.rules_ui.form.language_idx = 1;
        app.rules_ui.form.severity_idx = 0;
        app.rules_ui.form.rule_text = "Updated rule text".into();
        app.rules_ui.form.mode_idx = 2;
        app.rules_ui.form.detail_a = "clippy".into();
        app.rules_ui.form.detail_b = "unwrap_used".into();
        app.rules_ui.form.detail_c = String::new();
        app.submit_rules_form();

        let paths = crate::layers::LayerPaths::for_project(&tmp);
        let (personal_rules, _) =
            crate::rules::load_approved_rules(&paths.personal_rules_dir, None);
        let (project_rules, _) = crate::rules::load_approved_rules(&paths.project_rules_dir, None);
        assert!(personal_rules.is_empty());
        assert_eq!(project_rules.len(), 1);
        assert_eq!(project_rules[0].id, "custom.updated-rule");
        assert_eq!(project_rules[0].language, "rust");
        assert_eq!(project_rules[0].severity, "must");
        assert_eq!(project_rules[0].description, "Updated rule text");
        assert_eq!(
            project_rules[0].signals[0]
                .lint
                .as_ref()
                .map(|lint| lint.tool.as_str()),
            Some("clippy")
        );
        assert_eq!(
            project_rules[0].signals[0]
                .lint
                .as_ref()
                .map(|lint| lint.code.as_str()),
            Some("unwrap_used")
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn debt_snippet_preserves_multiline_evidence() {
        let evidence = crate::debt::types::Evidence::ManifestEntry {
            snippet: "line one\nline two".into(),
            references: 2,
            locations: Vec::new(),
        };

        assert_eq!(debt_snippet(&evidence), "line one\nline two");
    }

    #[test]
    fn sources_form_language_wraps_both_directions() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_sources_wrap_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.input_mode = InputMode::SourcesAdd;
        app.sources_ui.form.active_field = 1;
        app.sources_ui.form.language_idx = 3;

        app.handle_sources_form_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.sources_ui.form.language_idx, 0);

        app.handle_sources_form_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
        assert_eq!(app.sources_ui.form.language_idx, 3);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn rules_form_option_selectors_wrap() {
        let tmp = std::env::temp_dir().join(format!("wh_tui_rules_wrap_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&tmp);
        let mut app = App::new(&tmp).unwrap();
        app.input_mode = InputMode::RulesAdd;

        app.rules_ui.form.active_field = 2;
        app.rules_ui.form.language_idx = 3;
        app.handle_rules_form_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.rules_ui.form.language_idx, 0);

        app.rules_ui.form.active_field = 3;
        app.rules_ui.form.severity_idx = 2;
        app.handle_rules_form_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.rules_ui.form.severity_idx, 0);

        app.rules_ui.form.active_field = 4;
        app.rules_ui.form.mode_idx = 4;
        app.handle_rules_form_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        assert_eq!(app.rules_ui.form.mode_idx, 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
