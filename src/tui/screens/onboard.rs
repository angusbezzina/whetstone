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
    pub message: String,
    pub wired: bool,
}

impl OnboardState {
    pub fn load(project_dir: &Path) -> Self {
        let setup = crate::onboard::setup_status(project_dir);
        let catalog = corpus::catalog();

        // Which catalog packs match detected dependencies.
        let mut matched = Vec::new();
        if let Ok(detected) = crate::detect::detect_deps(project_dir, false, &[], &[], false) {
            if let Some(deps) = detected.get("dependencies").and_then(|d| d.as_array()) {
                for dep in deps {
                    let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    let lang = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
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
        let deps_detected = setup
            .get("dependencies_detected")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;

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
            message: String::new(),
            wired: false,
        }
    }

    fn list_len(&self) -> usize {
        match self.step {
            Step::Packs => self.catalog.len(),
            Step::Review => self.review_rule_ids().len(),
            _ => 0,
        }
    }

    fn move_cursor(&mut self, delta: i64) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let cur = self.cursor as i64 + delta;
        self.cursor = cur.rem_euclid(len as i64) as usize;
    }

    /// The (rule_id, citation, severity, pack_name) for every rule across the
    /// selected packs — what the review screen lists.
    fn review_rows(&self) -> Vec<(String, String, String, String)> {
        let mut rows = Vec::new();
        for &i in &self.selected {
            let entry = &self.catalog[i];
            if let Ok(pack) = crate::config_packs::resolve_inline_pack(entry.yaml, &entry.name) {
                for r in &pack.pack.rules {
                    if !r.approved {
                        continue;
                    }
                    rows.push((
                        r.id.clone(),
                        r.source_url.clone().unwrap_or_default(),
                        r.severity.clone().unwrap_or_default(),
                        entry.name.clone(),
                    ));
                }
            }
        }
        rows
    }

    fn review_rule_ids(&self) -> Vec<String> {
        self.review_rows().into_iter().map(|(id, ..)| id).collect()
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
        for &i in &self.selected {
            let e = &self.catalog[i];
            if let Err(err) = crate::onboard::import_pack(project_dir, e.dep, e.yaml) {
                self.message = format!("import failed: {err}");
                return;
            }
        }
        let denied: Vec<String> = self.denied_rules.iter().cloned().collect();
        let _ = crate::onboard::add_deny(project_dir, &denied);
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
                Up | Char('k') => self.move_cursor(-1),
                Down | Char('j') => self.move_cursor(1),
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
                        self.conflicts = Some(crate::conflicts::detect(
                            project_dir,
                            None,
                            &self.injected_from_selected(),
                            true,
                        ));
                        self.step = Step::Conflicts;
                    }
                }
                Esc => return Outcome::Exit,
                _ => {}
            },
            Step::Conflicts => match key {
                Enter => {
                    self.step = Step::Review;
                    self.cursor = 0;
                }
                Esc => self.step = Step::Packs,
                _ => {}
            },
            Step::Review => match key {
                Up | Char('k') => self.move_cursor(-1),
                Down | Char('j') => self.move_cursor(1),
                Char(' ') | Char('d') | Char('D') => {
                    let ids = self.review_rule_ids();
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
        Step::Conflicts => render_conflicts(&mut lines, s),
        Step::Review => render_review(&mut lines, s),
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

fn render_conflicts(lines: &mut Vec<Line<'static>>, s: &OnboardState) {
    heading(
        lines,
        "Conflicts",
        "How your selection overlaps existing rules · Enter to continue",
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
    if let Some(cs) = s.conflicts.as_ref().and_then(|c| c.get("conflicts")).and_then(|v| v.as_array()) {
        for c in cs {
            let kind = c.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let id = c.get("rule_id").or_else(|| c.get("option")).and_then(|v| v.as_str()).unwrap_or("");
            let winner = c.get("winner").and_then(|v| v.as_str()).unwrap_or("");
            lines.push(Line::from(Span::styled(
                format!("  ⚠ {kind}: {id}  (winner: {winner})"),
                Style::default().fg(theme::STATUS_WARN),
            )));
        }
    }
}

fn render_review(lines: &mut Vec<Line<'static>>, s: &OnboardState) {
    heading(
        lines,
        "Review — nothing is enforced until you confirm",
        "Space/D opts a rule out (deny) · A approves all · Enter confirms import",
    );
    let rows = s.review_rows();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "No rules selected. Go back (Esc) and pick a pack.",
            Style::default().fg(theme::MUTED),
        )));
        return;
    }
    for (i, (id, cite, severity, pack)) in rows.iter().enumerate() {
        let denied = s.denied_rules.contains(id);
        let cursor = if i == s.cursor { "›" } else { " " };
        let mark = if denied { "✗ deny" } else { "✓ keep" };
        let color = if denied { theme::STATUS_ERR } else { theme::severity_color(severity) };
        let style = if i == s.cursor { theme::selection() } else { Style::default().fg(color) };
        lines.push(Line::from(Span::styled(
            format!("{cursor} {mark} [{severity}] {id}  ({pack})"),
            style,
        )));
        if i == s.cursor && !cite.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("      source: {cite}"),
                Style::default().fg(theme::MUTED),
            )));
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
        "   Verify each with `wh eval`; land keepers with `wh pack import` or a guidance entry.\"",
        Style::default().fg(theme::STATUS_OK),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "New candidates show up in this wizard's Review step next time.",
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
        assert!(!s.review_rows().is_empty(), "review lists the pack's rules with citations");

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
        let first = s.review_rule_ids()[0].clone();
        press(&mut s, &tmp, KeyCode::Char(' ')); // deny the cursor rule
        assert!(s.denied_rules.contains(&first));
        press(&mut s, &tmp, KeyCode::Char('a')); // confirm
        let ws = fs::read_to_string(tmp.join("whetstone/whetstone.yaml")).unwrap();
        assert!(ws.contains(&first) && ws.contains("deny"), "deny entry written: {ws}");
        let _ = fs::remove_dir_all(&tmp);
    }
}
