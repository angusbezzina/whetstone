//! First-run onboarding wizard (whetstone-v5n / arx / gxh / dyg / a9e / j09 /
//! eg4 / 3qd / 420).
//!
//! A skin over deterministic oracles — this file holds ZERO business logic:
//! every step reads from an oracle (`onboard::setup_status`, `detect_deps`,
//! `corpus::catalog`, `check::run` preview) and every mutation dispatches to one
//! (`onboard::import_pack`, `add_deny`, `register_mcp`, `install_hooks_step`,
//! `generate_context_step`, `set_dismissed`). Progress is DERIVED, never stored.

use std::collections::BTreeSet;
use std::path::Path;

use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};
use serde_json::Value;

use crate::corpus::{self, CatalogEntry};
use crate::tui::{app::App, components::footer, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Home,
    Packs,
    Sources,
    Review,
    Conflicts,
    Payoff,
    Infer,
}

/// What the wizard asks the app to do after a key.
pub enum Outcome {
    Stay,
    /// Leave the wizard (skip or finish) → return to the dashboard.
    Exit,
}

/// One row in the Review gate — a pack rule (pre-import) or a project candidate.
pub struct ReviewRow {
    pub id: String,
    pub citation: String,
    pub severity: String,
    pub source_label: String,
    pub goldens: Vec<String>,
    /// True for a project candidate rule (approved on confirm); false for a
    /// pack rule (imported on confirm).
    pub is_candidate: bool,
}

fn format_golden(g: &crate::rules::GoldenExample) -> String {
    let code = g.code.lines().next().unwrap_or("").trim();
    let verdict = if g.verdict == "pass" { "ok" } else { "flag" };
    format!("[{verdict}] {code}")
}

pub struct OnboardState {
    pub step: Step,
    pub setup: Value,
    pub catalog: Vec<CatalogEntry>,
    pub matched: Vec<usize>, // indices into catalog matching detected deps
    pub deps_detected: usize,
    pub selected: BTreeSet<usize>, // catalog indices selected for import
    pub cursor: usize,
    pub denied_rules: BTreeSet<String>,
    pub preview: Option<Value>,
    pub payoff: Option<Value>,
    pub conflicts: Option<Value>,
    pub conflict_cursor: usize,
    /// Detected dependencies as (name, language) — the SOURCES step's rows.
    pub deps: Vec<(String, String)>,
    pub source_cursor: usize,
    pub message: String,
    pub wired: bool,
}

impl OnboardState {
    pub fn load(project_dir: &Path) -> Self {
        let setup = crate::onboard::setup_status(project_dir);
        let catalog = corpus::catalog();

        // Detected deps (name, language) + which catalog packs match them.
        let mut matched = Vec::new();
        let mut deps: Vec<(String, String)> = Vec::new();
        if let Ok(detected) = crate::detect::detect_deps(project_dir, false, &[], &[], false) {
            if let Some(dep_list) = detected.get("dependencies").and_then(|d| d.as_array()) {
                for dep in dep_list {
                    let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let lang = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    if !deps.iter().any(|(n, _)| n == name) {
                        deps.push((name.to_string(), lang.to_string()));
                    }
                    if let Some(i) = catalog.iter().position(|c| {
                        c.dep.eq_ignore_ascii_case(name) && c.language.eq_ignore_ascii_case(lang)
                    }) {
                        if !matched.contains(&i) {
                            matched.push(i);
                        }
                    }
                }
            }
        }
        let deps_detected = deps.len();

        OnboardState {
            step: Step::Home,
            setup,
            catalog,
            matched,
            deps_detected,
            selected: BTreeSet::new(),
            cursor: 0,
            denied_rules: BTreeSet::new(),
            preview: None,
            payoff: None,
            conflicts: None,
            conflict_cursor: 0,
            deps,
            source_cursor: 0,
            message: String::new(),
            wired: false,
        }
    }

    /// The docs/changelog URL Whetstone watches for a dependency, by registry.
    fn dep_docs_url(name: &str, language: &str) -> String {
        match language.to_lowercase().as_str() {
            "python" => format!("https://pypi.org/project/{name}/"),
            "typescript" | "javascript" => format!("https://www.npmjs.com/package/{name}"),
            "rust" => format!("https://docs.rs/{name}"),
            _ => format!("https://www.google.com/search?q={name}+changelog"),
        }
    }

    /// URLs currently subscribed in config (read-only — no cache writes).
    fn watched_urls(&self, project_dir: &Path) -> BTreeSet<String> {
        let opts = crate::config::SnapshotOptions {
            read_only: true,
            injected_packs: Vec::new(),
        };
        crate::config::WhetstoneConfig::load_with(project_dir, &opts)
            .sources
            .custom
            .iter()
            .map(|c| c.url.clone())
            .collect()
    }

    fn toggle_source(&mut self, project_dir: &Path) {
        let Some((name, lang)) = self.deps.get(self.source_cursor).cloned() else {
            return;
        };
        let url = Self::dep_docs_url(&name, &lang);
        let watched = self.watched_urls(project_dir);
        if watched.contains(&url) {
            let _ = crate::source_mgmt::remove(
                project_dir,
                crate::source_mgmt::RemoveOptions {
                    target: &url,
                    personal: false,
                },
            );
            self.message = format!("Stopped watching {name}.");
        } else {
            let _ = crate::source_mgmt::add(
                project_dir,
                crate::source_mgmt::AddOptions {
                    url: &url,
                    name: Some(&name),
                    language: if lang.is_empty() { None } else { Some(&lang) },
                    source_kind: Some("official_docs"),
                    personal: false,
                },
            );
            self.message = format!("Watching {name} for drift.");
        }
    }

    fn conflict_list(&self) -> Vec<Value> {
        self.conflicts
            .as_ref()
            .and_then(|c| c.get("conflicts"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    fn move_cursor(&mut self, delta: i64, len: usize) {
        if len == 0 {
            return;
        }
        let cur = self.cursor as i64 + delta;
        self.cursor = cur.rem_euclid(len as i64) as usize;
    }

    /// Every rule the Review screen lists: the rules of each selected pack
    /// (pre-import), plus any existing project CANDIDATE rules — e.g. taste an
    /// agent proposed via the Infer handoff (whetstone-eg4 / 420 return leg).
    fn review_rows(&self, project_dir: &Path) -> Vec<ReviewRow> {
        let mut rows = Vec::new();
        for &i in &self.selected {
            let entry = &self.catalog[i];
            if let Ok(pack) = crate::config_packs::resolve_inline_pack(entry.yaml, &entry.name) {
                for r in &pack.pack.rules {
                    if !r.approved {
                        continue;
                    }
                    rows.push(ReviewRow {
                        id: r.id.clone(),
                        citation: r.source_url.clone().unwrap_or_default(),
                        severity: r.severity.clone().unwrap_or_default(),
                        source_label: entry.name.clone(),
                        goldens: r.golden_examples.iter().map(format_golden).collect(),
                        is_candidate: false,
                    });
                }
            }
        }
        // Project candidate rules (status != approved) — awaiting a decision.
        let rules_dir = crate::layers::LayerPaths::for_project(project_dir).project_rules_dir;
        let (files, _) = crate::rules::load_rule_files(&rules_dir);
        for lrf in &files {
            for r in &lrf.rule_file.rules {
                if r.approved {
                    continue;
                }
                rows.push(ReviewRow {
                    id: r.id.clone(),
                    citation: r.source_url.clone().unwrap_or_default(),
                    severity: r.severity.clone().unwrap_or_default(),
                    source_label: "candidate".to_string(),
                    goldens: r.golden_examples.iter().map(format_golden).collect(),
                    is_candidate: true,
                });
            }
        }
        rows
    }

    fn review_rule_ids(&self, project_dir: &Path) -> Vec<String> {
        self.review_rows(project_dir).into_iter().map(|r| r.id).collect()
    }

    fn injected_from_selected(&self) -> Vec<crate::config_packs::ResolvedConfigPack> {
        self.selected
            .iter()
            .filter_map(|&i| {
                let e = &self.catalog[i];
                crate::config_packs::resolve_inline_pack(e.yaml, e.name.as_str()).ok()
            })
            .collect()
    }

    /// Run a read-only preview scan of the current selection.
    fn run_preview(&mut self, project_dir: &Path) {
        let injected = self.injected_from_selected();
        if injected.is_empty() {
            self.message = "Select at least one pack to preview.".to_string();
            return;
        }
        let scan_paths = [project_dir.to_path_buf()];
        match crate::check::run(crate::check::CheckOptions {
            project_dir,
            scan_paths: &scan_paths,
            lang_filter: None,
            rule_filter: None,
            injected_packs: &injected,
        }) {
            Ok(v) => {
                self.preview = Some(v);
                self.message.clear();
            }
            Err(e) => self.message = format!("preview failed: {e}"),
        }
    }

    /// THE GATE: import selected packs (writes extends via the oracle) + deny the
    /// opted-out rules, then compute the payoff scan. Nothing is enforceable
    /// before this confirm (whetstone-eg4).
    fn confirm_and_import(&mut self, project_dir: &Path) {
        // Snapshot rows before writing (kept/denied classification).
        let rows = self.review_rows(project_dir);
        // Import selected packs (extends).
        for &i in &self.selected {
            let e = &self.catalog[i];
            if let Err(err) = crate::onboard::import_pack(project_dir, e.dep, e.yaml) {
                self.message = format!("import failed: {err}");
                return;
            }
        }
        // Candidate rules: approve the kept ones (the Infer return leg); pack
        // rules opted out become deny entries.
        let mut pack_denies: Vec<String> = Vec::new();
        for row in &rows {
            let denied = self.denied_rules.contains(&row.id);
            if row.is_candidate {
                if !denied {
                    let _ = crate::approve::approve_by_id(project_dir, &row.id);
                }
            } else if denied {
                pack_denies.push(row.id.clone());
            }
        }
        let _ = crate::onboard::add_deny(project_dir, &pack_denies);
        self.step = Step::Payoff;
        self.compute_payoff(project_dir);
    }

    fn compute_payoff(&mut self, project_dir: &Path) {
        let scan_paths = [project_dir.to_path_buf()];
        self.payoff = crate::check::run(crate::check::CheckOptions {
            project_dir,
            scan_paths: &scan_paths,
            lang_filter: None,
            rule_filter: None,
            injected_packs: &[],
        })
        .ok();
        self.setup = crate::onboard::setup_status(project_dir);
    }

    pub fn on_key(&mut self, key: crossterm::event::KeyCode, project_dir: &Path) -> Outcome {
        use crossterm::event::KeyCode::*;
        match self.step {
            Step::Home => match key {
                Char('e') | Char('E') => {
                    // Express: accept matched packs → Review. Zero-match → resources.
                    if self.matched.is_empty() {
                        self.step = Step::Packs;
                        self.message =
                            "No starter pack matched your deps — pick a resource pack, or run the agent taste handoff ([i]).".to_string();
                    } else {
                        self.selected = self.matched.iter().copied().collect();
                        self.step = Step::Review;
                        self.cursor = 0;
                    }
                }
                Char('c') | Char('C') => {
                    self.step = Step::Packs;
                    self.cursor = 0;
                }
                Char('i') | Char('I') => self.step = Step::Infer,
                Char('s') | Char('S') => {
                    let _ = crate::onboard::set_dismissed(project_dir, true);
                    return Outcome::Exit;
                }
                _ => {}
            },
            Step::Packs => match key {
                Up | Char('k') => self.move_cursor(-1, self.catalog.len()),
                Down | Char('j') => self.move_cursor(1, self.catalog.len()),
                Char(' ') => {
                    if self.selected.contains(&self.cursor) {
                        self.selected.remove(&self.cursor);
                    } else {
                        self.selected.insert(self.cursor);
                    }
                    self.preview = None;
                }
                Char('p') | Char('P') => self.run_preview(project_dir),
                Char('i') | Char('I') => self.step = Step::Infer,
                Enter => {
                    if self.selected.is_empty() {
                        self.message = "Select at least one pack (Space) before continuing.".to_string();
                    } else {
                        self.step = Step::Sources;
                    }
                }
                Esc => return Outcome::Exit,
                _ => {}
            },
            Step::Sources => match key {
                Up | Char('k') => {
                    if !self.deps.is_empty() {
                        self.source_cursor =
                            (self.source_cursor + self.deps.len() - 1) % self.deps.len();
                    }
                }
                Down | Char('j') => {
                    if !self.deps.is_empty() {
                        self.source_cursor = (self.source_cursor + 1) % self.deps.len();
                    }
                }
                Char(' ') => self.toggle_source(project_dir),
                Char('a') | Char('A') => {
                    // Bulk: watch every not-yet-watched dependency.
                    let watched = self.watched_urls(project_dir);
                    for (name, lang) in self.deps.clone() {
                        let url = Self::dep_docs_url(&name, &lang);
                        if !watched.contains(&url) {
                            let _ = crate::source_mgmt::add(
                                project_dir,
                                crate::source_mgmt::AddOptions {
                                    url: &url,
                                    name: Some(&name),
                                    language: if lang.is_empty() { None } else { Some(&lang) },
                                    source_kind: Some("official_docs"),
                                    personal: false,
                                },
                            );
                        }
                    }
                    self.message = "Watching all dependencies for drift.".to_string();
                }
                Enter => {
                    // Compute conflicts for the proposed selection, then advance.
                    self.conflicts = Some(crate::conflicts::detect(
                        project_dir,
                        None,
                        &self.injected_from_selected(),
                        true,
                    ));
                    self.conflict_cursor = 0;
                    self.step = Step::Conflicts;
                }
                Esc => self.step = Step::Packs,
                _ => {}
            },
            Step::Conflicts => {
                let conflicts = self.conflict_list();
                match key {
                    Up | Char('k') => {
                        if !conflicts.is_empty() {
                            self.conflict_cursor =
                                (self.conflict_cursor + conflicts.len() - 1) % conflicts.len();
                        }
                    }
                    Down | Char('j') => {
                        if !conflicts.is_empty() {
                            self.conflict_cursor = (self.conflict_cursor + 1) % conflicts.len();
                        }
                    }
                    Char('d') | Char('D') => {
                        // Resolve a same-id conflict by denying the contested rule
                        // id (writes a deny entry — whetstone-j09).
                        if let Some(c) = conflicts.get(self.conflict_cursor) {
                            if c.get("kind").and_then(|v| v.as_str()) == Some("same-id") {
                                if let Some(id) = c.get("rule_id").and_then(|v| v.as_str()) {
                                    let _ = crate::onboard::add_deny(project_dir, &[id.to_string()]);
                                    self.message = format!("Denied {id} (writes a deny entry).");
                                }
                            } else {
                                self.message =
                                    "Formatter clashes are resolved by an override — deny a rule in Review.".to_string();
                            }
                        }
                    }
                    Enter => {
                        self.step = Step::Review;
                        self.cursor = 0;
                    }
                    Esc => self.step = Step::Sources,
                    _ => {}
                }
            }
            Step::Review => match key {
                Up | Char('k') => {
                    let len = self.review_rule_ids(project_dir).len();
                    self.move_cursor(-1, len);
                }
                Down | Char('j') => {
                    let len = self.review_rule_ids(project_dir).len();
                    self.move_cursor(1, len);
                }
                Char(' ') | Char('d') | Char('D') => {
                    let ids = self.review_rule_ids(project_dir);
                    if let Some(id) = ids.get(self.cursor) {
                        if self.denied_rules.contains(id) {
                            self.denied_rules.remove(id);
                        } else {
                            self.denied_rules.insert(id.clone());
                        }
                    }
                }
                Char('a') | Char('A') | Enter => self.confirm_and_import(project_dir),
                Esc => self.step = Step::Packs,
                _ => {}
            },
            Step::Payoff => match key {
                Char('g') | Char('G') => {
                    let _ = crate::onboard::generate_context_step(project_dir);
                    self.compute_payoff(project_dir);
                    self.message = "Generated agent context.".to_string();
                }
                Char('w') | Char('W') => {
                    let _ = crate::onboard::register_mcp(project_dir);
                    let _ = crate::onboard::install_hooks_step(project_dir);
                    self.wired = true;
                    self.compute_payoff(project_dir);
                    self.message = "Wired MCP + in-session hooks.".to_string();
                }
                Enter | Char('q') | Esc => return Outcome::Exit,
                _ => {}
            },
            Step::Infer => match key {
                Esc => self.step = Step::Home,
                Enter | Char('q') => return Outcome::Exit,
                _ => {}
            },
        }
        Outcome::Stay
    }
}

// ── rendering ──

pub fn render(frame: &mut Frame<'_>, area: Rect, app: &App) {
    let s = &app.onboard;
    let mut lines: Vec<Line<'static>> = Vec::new();
    match s.step {
        Step::Home => render_home(&mut lines, s),
        Step::Packs => render_packs(&mut lines, s),
        Step::Sources => render_sources(&mut lines, s, &app.project_dir),
        Step::Conflicts => render_conflicts(&mut lines, s),
        Step::Review => render_review(&mut lines, s, &app.project_dir),
        Step::Payoff => render_payoff(&mut lines, s),
        Step::Infer => render_infer(&mut lines),
    }
    if !s.message.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("• {}", s.message),
            Style::default().fg(theme::STATUS_WARN),
        )));
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::AMBER));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn heading(lines: &mut Vec<Line<'static>>, title: &str, sub: &str) {
    lines.push(Line::from(Span::styled(
        title.to_string(),
        Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD),
    )));
    if !sub.is_empty() {
        lines.push(Line::from(Span::styled(sub.to_string(), Style::default().fg(theme::MUTED))));
    }
    lines.push(Line::from(""));
}

fn render_home(lines: &mut Vec<Line<'static>>, s: &OnboardState) {
    let done = s.setup.get("done").and_then(|v| v.as_u64()).unwrap_or(0);
    let total = s.setup.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
    heading(
        lines,
        &format!("Whetstone setup — {done}/{total} complete"),
        "Sharpen the tools that write your code. This runs once.",
    );
    if let Some(items) = s.setup.get("items").and_then(|v| v.as_array()) {
        for it in items {
            let ok = it.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
            let key = it.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let mark = if ok { "✓" } else { "○" };
            let color = if ok { theme::STATUS_OK } else { theme::MUTED };
            lines.push(Line::from(Span::styled(
                format!("  {mark} {key}"),
                Style::default().fg(color),
            )));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("{} dependencies detected.", s.deps_detected),
        Style::default().fg(theme::MUTED),
    )));
    if s.setup
        .get("private_mode")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        lines.push(Line::from(Span::styled(
            "  private mode — artifacts hidden from git (`wh publish` shares them)",
            Style::default().fg(theme::MUTED),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        key("E"),
        Span::raw(" Express (accept matched starter packs)   "),
        key("C"),
        Span::raw(" Curated (browse + preview)"),
    ]));
    lines.push(Line::from(vec![
        key("I"),
        Span::raw(" Infer taste from my code (agent)   "),
        key("S"),
        Span::raw(" Skip (don't ask again)"),
    ]));
}

fn render_packs(lines: &mut Vec<Line<'static>>, s: &OnboardState) {
    heading(
        lines,
        "Choose rule packs",
        "Space selects · P previews against your code · Enter continues",
    );
    for (i, e) in s.catalog.iter().enumerate() {
        let sel = if s.selected.contains(&i) { "[x]" } else { "[ ]" };
        let cursor = if i == s.cursor { "›" } else { " " };
        let is_match = s.matched.contains(&i);
        let tag = if is_match {
            " (matches your deps)"
        } else if e.kind == "resource" {
            " (style guide)"
        } else {
            ""
        };
        let style = if i == s.cursor {
            theme::selection()
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!(
                "{cursor} {sel} {} · {} rules · {}{tag}",
                e.name, e.rule_count, e.language
            ),
            style,
        )));
    }
    if let Some(p) = &s.preview {
        lines.push(Line::from(""));
        let ch = p.pointer("/preview/candidate_hits").and_then(|v| v.as_i64()).unwrap_or(0);
        let cr = p.pointer("/preview/candidate_rules").and_then(|v| v.as_i64()).unwrap_or(0);
        lines.push(Line::from(Span::styled(
            format!("Preview: {cr} candidate rule(s) · {ch} hit(s) on your code right now"),
            Style::default().fg(theme::STATUS_OK),
        )));
    }
}

fn render_sources(lines: &mut Vec<Line<'static>>, s: &OnboardState, project_dir: &Path) {
    heading(
        lines,
        "Watch dependency docs for drift",
        "Space toggles a watch · A watches all · Enter continues (defaults are fine to skip)",
    );
    if s.deps.is_empty() {
        lines.push(Line::from(Span::styled(
            "No dependencies detected — nothing to watch. Enter to continue.",
            Style::default().fg(theme::MUTED),
        )));
        return;
    }
    let watched = s.watched_urls(project_dir);
    for (i, (name, lang)) in s.deps.iter().enumerate() {
        let url = OnboardState::dep_docs_url(name, lang);
        let on = watched.contains(&url);
        let mark = if on { "[x]" } else { "[ ]" };
        let cursor = if i == s.source_cursor { "›" } else { " " };
        let style = if i == s.source_cursor {
            theme::selection()
        } else if on {
            Style::default().fg(theme::STATUS_OK)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{cursor} {mark} {name} ({lang}) — {url}"),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Watched sources feed Whetstone's drift gate — it flags rules when the docs change.",
        Style::default().fg(theme::MUTED),
    )));
}

fn render_conflicts(lines: &mut Vec<Line<'static>>, s: &OnboardState) {
    heading(
        lines,
        "Conflicts",
        "↑/↓ move · D denies a contested rule · Enter accepts precedence + continues",
    );
    let count = s
        .conflicts
        .as_ref()
        .and_then(|c| c.get("conflicts_count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if count == 0 {
        lines.push(Line::from(Span::styled(
            "No conflicts — your selection layers cleanly.",
            Style::default().fg(theme::STATUS_OK),
        )));
        return;
    }
    for (i, c) in s.conflict_list().iter().enumerate() {
        let kind = c.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let id = c.get("rule_id").or_else(|| c.get("option")).and_then(|v| v.as_str()).unwrap_or("");
        let winner = c.get("winner").and_then(|v| v.as_str()).unwrap_or("");
        let cursor = if i == s.conflict_cursor { "›" } else { " " };
        let extra = if winner.is_empty() {
            String::new()
        } else {
            format!("  (winner: {winner})")
        };
        let style = if i == s.conflict_cursor {
            theme::selection()
        } else {
            Style::default().fg(theme::STATUS_WARN)
        };
        lines.push(Line::from(Span::styled(
            format!("{cursor} ⚠ {kind}: {id}{extra}"),
            style,
        )));
    }
}

fn render_review(lines: &mut Vec<Line<'static>>, s: &OnboardState, project_dir: &Path) {
    heading(
        lines,
        "Review — nothing is enforced until you confirm",
        "Space/D opts a rule out · A keeps all · Enter confirms (imports packs, approves candidates)",
    );
    let rows = s.review_rows(project_dir);
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "No rules selected. Go back (Esc) and pick a pack.",
            Style::default().fg(theme::MUTED),
        )));
        return;
    }
    for (i, row) in rows.iter().enumerate() {
        let denied = s.denied_rules.contains(&row.id);
        let cursor = if i == s.cursor { "›" } else { " " };
        let mark = if denied { "✗ deny" } else { "✓ keep" };
        let origin = if row.is_candidate { "candidate" } else { &row.source_label };
        let color = if denied { theme::STATUS_ERR } else { theme::severity_color(&row.severity) };
        let style = if i == s.cursor { theme::selection() } else { Style::default().fg(color) };
        lines.push(Line::from(Span::styled(
            format!("{cursor} {mark} [{}] {}  ({origin})", row.severity, row.id),
            style,
        )));
        // Detail on the cursor row: citation + golden examples (whetstone-eg4).
        if i == s.cursor {
            if !row.citation.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("      source: {}", row.citation),
                    Style::default().fg(theme::MUTED),
                )));
            }
            for g in row.goldens.iter().take(3) {
                lines.push(Line::from(Span::styled(
                    format!("      {g}"),
                    Style::default().fg(theme::MUTED),
                )));
            }
        }
    }
}

fn render_payoff(lines: &mut Vec<Line<'static>>, s: &OnboardState) {
    let hits = s
        .payoff
        .as_ref()
        .and_then(|p| p.get("violations_count"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    if hits == 0 {
        heading(
            lines,
            "You're set — and clean.",
            "Your new standards find nothing to fix right now.",
        );
        lines.push(Line::from(Span::styled(
            "That's the goal: they'll catch regressions as you (and your agents) write code.",
            Style::default().fg(theme::MUTED),
        )));
    } else {
        heading(
            lines,
            &format!("Your new standards flag {hits} thing(s) today"),
            "This is the value, live on your code:",
        );
        if let Some(vs) = s.payoff.as_ref().and_then(|p| p.get("violations")).and_then(|v| v.as_array()) {
            for v in vs.iter().take(6) {
                let id = v.get("rule_id").and_then(|x| x.as_str()).unwrap_or("");
                let file = v.get("file").and_then(|x| x.as_str()).unwrap_or("");
                let line = v.get("line").and_then(|x| x.as_i64()).unwrap_or(0);
                lines.push(Line::from(Span::styled(
                    format!("  • {id} — {file}:{line}"),
                    Style::default().fg(theme::SEVERITY_SHOULD),
                )));
            }
        }
    }
    lines.push(Line::from(""));
    let context_done = s
        .setup
        .pointer("/items/1/done")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    lines.push(Line::from(vec![
        key("G"),
        Span::raw(if context_done {
            " Regenerate agent context   "
        } else {
            " Generate agent context (AGENTS.md)   "
        }),
        key("W"),
        Span::raw(if s.wired {
            " Re-wire agent (Claude hooks + MCP)"
        } else {
            " Wire your agent (Claude hooks + MCP)"
        }),
    ]));
    lines.push(Line::from(Span::styled(
        "  Not on Claude Code? Context files above are agent-agnostic; point any agent at them.",
        Style::default().fg(theme::MUTED),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![key("Enter"), Span::raw(" Finish")]));
}

fn render_infer(lines: &mut Vec<Line<'static>>) {
    heading(
        lines,
        "Infer taste from your code (agent handoff)",
        "The wizard never guesses — it hands this judgment to your agent.",
    );
    lines.push(Line::from("Run this with your coding agent in this repo:"));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  \"Read this repo and propose Whetstone taste rules for patterns I already follow.",
        Style::default().fg(theme::STATUS_OK),
    )));
    lines.push(Line::from(Span::styled(
        "   Add each as a CANDIDATE rule (`wh rules add`) and verify it with `wh eval`.\"",
        Style::default().fg(theme::STATUS_OK),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Candidate rules appear in this wizard's Review step (marked 'candidate') for you to",
        Style::default().fg(theme::MUTED),
    )));
    lines.push(Line::from(Span::styled(
        "approve or deny — approve happens only when you confirm there.",
        Style::default().fg(theme::MUTED),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![key("Esc"), Span::raw(" Back   "), key("Enter"), Span::raw(" Finish")]));
}

fn key(k: &str) -> Span<'static> {
    Span::styled(
        format!("[{k}]"),
        Style::default().fg(theme::AMBER).add_modifier(Modifier::BOLD),
    )
}

#[allow(dead_code)]
pub fn hints() -> &'static [footer::Hint] {
    &[]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyCode;
    use std::fs;

    fn press(s: &mut OnboardState, dir: &Path, k: KeyCode) -> bool {
        matches!(s.on_key(k, dir), Outcome::Exit)
    }

    fn home_text(s: &OnboardState) -> String {
        let mut lines: Vec<Line<'static>> = Vec::new();
        render_home(&mut lines, s);
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|sp| sp.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The wizard DISPLAYS private mode; it never decides it (whetstone-55u).
    /// The flag comes from the setup_status oracle, so this asserts the skin
    /// reflects the oracle rather than any TUI-local state.
    #[test]
    fn home_shows_private_mode_from_the_oracle() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wizpriv_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("requirements.txt"), "fastapi==0.115\n").unwrap();

        let public = OnboardState::load(&tmp);
        assert!(
            !home_text(&public).contains("private mode"),
            "public project must not advertise private mode"
        );

        crate::onboard::set_private(&tmp, true).unwrap();
        let private = OnboardState::load(&tmp);
        assert!(
            private.setup["private_mode"].as_bool().unwrap_or(false),
            "setup_status must carry the marker"
        );
        let text = home_text(&private);
        assert!(text.contains("private mode"), "home should show the mode: {text}");
        assert!(text.contains("wh publish"), "home should point at the flip: {text}");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn express_review_gate_then_payoff() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wiz_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("src")).unwrap();
        fs::write(tmp.join("requirements.txt"), "fastapi==0.115\n").unwrap();
        fs::write(
            tmp.join("src/app.py"),
            "from fastapi import FastAPI\napp = FastAPI()\n@app.on_event(\"startup\")\nasync def s(): ...\n",
        ).unwrap();

        let mut s = OnboardState::load(&tmp);
        assert!(!s.matched.is_empty(), "fastapi should match a starter pack");

        // Express: accept matched → Review. NOTHING imported yet (the gate).
        press(&mut s, &tmp, KeyCode::Char('e'));
        assert_eq!(s.step, Step::Review);
        assert!(!tmp.join("whetstone/whetstone.yaml").exists(), "no import before confirm");
        assert!(!s.review_rows(&tmp).is_empty(), "review lists the pack's rules with citations");

        // Confirm (approve all) → imports via the oracle → Payoff with real hits.
        press(&mut s, &tmp, KeyCode::Char('a'));
        assert_eq!(s.step, Step::Payoff);
        let ws = fs::read_to_string(tmp.join("whetstone/whetstone.yaml")).unwrap();
        assert!(ws.contains("path:./whetstone/packs/fastapi.yaml"), "confirm wrote extends: {ws}");
        let hits = s.payoff.as_ref().and_then(|p| p.get("violations_count")).and_then(|v| v.as_i64()).unwrap_or(0);
        assert!(hits > 0, "payoff scan should find the on_event violation");

        // Wiring uses the shared oracles (byte-equivalent to init --claude pieces).
        press(&mut s, &tmp, KeyCode::Char('w'));
        assert!(tmp.join(".mcp.json").exists() && tmp.join(".claude/settings.json").exists());

        // Finish exits.
        assert!(press(&mut s, &tmp, KeyCode::Enter));

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn infer_return_leg_candidate_appears_and_approves() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wizcand_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("whetstone/rules/python")).unwrap();
        fs::write(tmp.join("requirements.txt"), "fastapi==0.115\n").unwrap();
        // A project candidate rule (status candidate, approved false) — as if an
        // agent proposed it via the Infer handoff.
        fs::write(
            tmp.join("whetstone/rules/python/taste.yaml"),
            "source: { name: taste }\nrules:\n  - id: taste.mine\n    severity: should\n    confidence: high\n    category: convention\n    description: candidate\n    source_url: https://x\n    approved: false\n    status: candidate\n    signals: [{ id: s, strategy: ast, weight: required, ast_query: '(pass_statement) @match' }]\n    golden_examples: [{ code: \"def f(): pass\", verdict: fail, reason: y }]\n",
        ).unwrap();
        let mut s = OnboardState::load(&tmp);
        press(&mut s, &tmp, KeyCode::Char('e')); // express → Review (fastapi matched)
        assert_eq!(s.step, Step::Review);
        // The candidate rule shows up in Review.
        assert!(s.review_rows(&tmp).iter().any(|r| r.id == "taste.mine" && r.is_candidate), "candidate not listed");
        press(&mut s, &tmp, KeyCode::Char('a')); // confirm keeps all → approves the candidate
        let after = fs::read_to_string(tmp.join("whetstone/rules/python/taste.yaml")).unwrap();
        assert!(after.contains("approved: true") || after.contains("status: approved"), "candidate should be approved: {after}");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn sources_toggle_persists_and_lists() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wizsrc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("requirements.txt"), "fastapi==0.115\n").unwrap();
        let mut s = OnboardState::load(&tmp);
        assert!(!s.deps.is_empty(), "fastapi should be detected");
        press(&mut s, &tmp, KeyCode::Char('c')); // Packs
        // jump straight to Sources by selecting a pack then Enter
        s.cursor = s.catalog.iter().position(|c| c.dep == "fastapi").unwrap();
        press(&mut s, &tmp, KeyCode::Char(' '));
        press(&mut s, &tmp, KeyCode::Enter); // Sources
        assert_eq!(s.step, Step::Sources);
        press(&mut s, &tmp, KeyCode::Char(' ')); // watch fastapi
        assert!(s.watched_urls(&tmp).iter().any(|u| u.contains("pypi.org/project/fastapi")), "watch should persist to config");
        // Toggling again removes it.
        press(&mut s, &tmp, KeyCode::Char(' '));
        assert!(!s.watched_urls(&tmp).iter().any(|u| u.contains("pypi.org/project/fastapi")), "second toggle unsubscribes");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn conflict_deny_writes_deny_entry() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wizcd_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        // Onboard fastapi first, so re-selecting it in the wizard collides (same-id).
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fastapi = manifest.join("packs/python/fastapi.yaml");
        crate::onboard::import_pack_from_file(&tmp, &fastapi).unwrap();
        let mut s = OnboardState::load(&tmp);
        let fa = s.catalog.iter().position(|c| c.dep == "fastapi").unwrap();
        press(&mut s, &tmp, KeyCode::Char('c')); // Packs
        s.cursor = fa;
        press(&mut s, &tmp, KeyCode::Char(' ')); // select fastapi (collides with configured)
        press(&mut s, &tmp, KeyCode::Enter); // Sources
        press(&mut s, &tmp, KeyCode::Enter); // Conflicts (same-id present)
        assert_eq!(s.step, Step::Conflicts);
        assert!(!s.conflict_list().is_empty(), "expected a same-id conflict");
        let cid = s.conflict_list()[0]["rule_id"].as_str().unwrap().to_string();
        press(&mut s, &tmp, KeyCode::Char('d')); // deny the contested rule
        let ws = fs::read_to_string(tmp.join("whetstone/whetstone.yaml")).unwrap();
        assert!(ws.contains(&cid) && ws.contains("deny"), "conflict deny should write a deny entry: {ws}");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn curated_path_sources_conflicts_deny() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wizcur_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        let mut s = OnboardState::load(&tmp);
        // Curated: Home C -> Packs. Select the airbnb resource pack, then walk
        // Packs -> Sources -> Conflicts -> Review.
        press(&mut s, &tmp, KeyCode::Char('c'));
        assert_eq!(s.step, Step::Packs);
        let airbnb = s.catalog.iter().position(|c| c.kind == "resource").expect("a resource pack exists");
        s.cursor = airbnb;
        press(&mut s, &tmp, KeyCode::Char(' ')); // select
        assert!(s.selected.contains(&airbnb));
        press(&mut s, &tmp, KeyCode::Enter); // -> Sources
        assert_eq!(s.step, Step::Sources);
        press(&mut s, &tmp, KeyCode::Enter); // -> Conflicts (computed)
        assert_eq!(s.step, Step::Conflicts);
        press(&mut s, &tmp, KeyCode::Enter); // -> Review
        assert_eq!(s.step, Step::Review);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn per_rule_deny_writes_deny_entry() {
        let tmp = std::env::temp_dir().join(format!(
            "wh_wizdeny_{}_{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        fs::write(tmp.join("requirements.txt"), "fastapi==0.115\n").unwrap();
        let mut s = OnboardState::load(&tmp);
        press(&mut s, &tmp, KeyCode::Char('e')); // → Review
        let first = s.review_rule_ids(&tmp)[0].clone();
        press(&mut s, &tmp, KeyCode::Char(' ')); // deny the cursor rule
        assert!(s.denied_rules.contains(&first));
        press(&mut s, &tmp, KeyCode::Char('a')); // confirm
        let ws = fs::read_to_string(tmp.join("whetstone/whetstone.yaml")).unwrap();
        assert!(ws.contains(&first) && ws.contains("deny"), "deny entry written: {ws}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
