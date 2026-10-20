//! The one-page dashboard's view model: a header line, the pending changes,
//! one health line, the rulebook (mission, then rules that block and rules
//! that report), suggestions for Add rule, History, and, before a mission
//! and a rule are committed, the guided setup.
//!
//! Every label is plain words the browser shows as is; ids and digests ride
//! along for the details toggle and for the edit forms.

use serde::Serialize;
use serde_json::{json, Value};

use super::diff::{self, EntryChange, FieldChange};
use super::{examples, rule_edit, starter_view, EditBase, StarterView};
use crate::domain::{Enforcer, PrincipleSource, Rule, Strength, SweepClass, SweepUnsure};
use crate::rulebook::{self, Rulebook};

/// One commit that touched the rulebook, with what it changed.
#[derive(Debug, Clone, Serialize)]
pub struct HistoryEntry {
    pub commit: String,
    pub short: String,
    pub author: String,
    pub at: String,
    pub subject: String,
    pub body: String,
    /// `added rule.x, changed strength of rule.y, edited the mission`.
    pub summary: String,
    pub changes: Vec<EntryChange>,
    /// False when the file at this commit was not JSON, so nothing was
    /// compared.
    pub readable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct LastChange {
    pub commit: String,
    pub author: String,
    pub at: String,
    pub subject: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Counts {
    pub rules: usize,
    pub blocks: usize,
    pub reports: usize,
    pub principles: usize,
    pub retired: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageHeader {
    pub project: String,
    pub branch: Option<String>,
    /// The commit that last touched the rulebook, when it was ever committed.
    pub revision: Option<String>,
    /// `14 rules: 7 hard, 7 soft`.
    pub summary: String,
    pub counts: Counts,
    pub last_change: Option<LastChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub subject: String,
    pub body: String,
}

/// The working tree's edits to the rulebook that are not committed yet.
#[derive(Debug, Clone, Serialize)]
pub struct Pending {
    pub count: usize,
    pub summary: String,
    pub changes: Vec<EntryChange>,
    /// The commit message accepting would use.
    pub message: Message,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthItem {
    /// `gates`, `ci`, `rulebook`, `coverage` or `principles`.
    pub kind: &'static str,
    /// `fail` (the rulebook cannot be read) or `warn`.
    pub tone: &'static str,
    pub text: String,
    /// The command that fixes it, when one does.
    pub fix: Option<String>,
    pub detail: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Health {
    pub items: Vec<HealthItem>,
    /// The one line shown when nothing is wrong.
    pub healthy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissionView {
    pub statement: String,
    pub lifecycle: &'static str,
    pub edit: EditBase,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrincipleView {
    pub id: String,
    pub statement: String,
    pub rationale: Option<String>,
    pub source: String,
    pub lifecycle: &'static str,
    /// The principle as `wh change` takes it, for editing in place.
    pub edit: EditBase,
    /// The ids of the rules that serve it, in file order.
    pub served_by: Vec<String>,
    /// Its pack's enforcer hint (`ast`, `lint`, `formatter`, `test`,
    /// `validator` or `none`), when it came from a loaded pack.
    pub enforcer_hint: Option<String>,
    /// What kind of rule could serve it, in plain words.
    pub hint: String,
}

/// Where the chain from principle to rule breaks (`Rulebook::coverage`).
#[derive(Debug, Clone, Default, Serialize)]
pub struct CoverageView {
    pub unserved: Vec<CoverageEntry>,
    pub unanchored: Vec<CoverageEntry>,
    /// Whether `wh change --accept` refuses a rulebook with gaps.
    pub require_served: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CoverageEntry {
    pub id: String,
    /// The principle's or the rule's statement.
    pub statement: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// The enforcer kind: ast, lint, formatter, test or validator.
    pub kind: &'static str,
    /// `a lint`, `a script`, `a code pattern` and so on.
    pub label: &'static str,
    pub command: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stage {
    /// `commit`, `push` or `ci`.
    pub key: &'static str,
    pub label: &'static str,
    pub runs: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceView {
    pub kind: &'static str,
    pub label: String,
    pub reference: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SweepView {
    pub class: &'static str,
    pub label: &'static str,
    pub priority: u8,
    pub unsure: Vec<SweepUnsure>,
}

/// One commit in a rule's own history.
#[derive(Debug, Clone, Serialize)]
pub struct RuleEvent {
    pub commit: String,
    pub short: String,
    pub author: String,
    pub at: String,
    pub subject: String,
    pub action: &'static str,
    pub fields: Vec<FieldChange>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleView {
    pub id: String,
    pub statement: String,
    pub rationale: String,
    pub strength: &'static str,
    pub check: Check,
    pub stages: Vec<Stage>,
    pub paths: Vec<String>,
    pub examples: Vec<Value>,
    pub source: SourceView,
    /// The principle the rule serves, when its source names one held here.
    pub serves: Option<ServesView>,
    pub sweep: SweepView,
    /// `accepted` (as committed), `changed` (edited since) or `new`.
    pub lifecycle: &'static str,
    pub digest: String,
    pub history: Vec<RuleEvent>,
    pub edit: EditBase,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServesView {
    pub id: String,
    pub statement: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuleGroup {
    pub strength: &'static str,
    /// `Hard rules` or `Soft rules`.
    pub label: &'static str,
    pub hint: &'static str,
    pub rules: Vec<RuleView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetiredView {
    pub id: String,
    pub reason: String,
    pub retired_in: Option<LastChange>,
}

/// Something Add rule offers: a starter rule detected for this repository,
/// or a principle from a loaded pack.
#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    /// `starter` or `principle`.
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub detail: String,
    /// `must` or `should` for a starter; none for a principle.
    pub strength: Option<&'static str>,
    /// How it would be checked, in plain words.
    pub check: String,
    pub group: Option<String>,
    /// A principle its pack offers as a starting point.
    pub starting: bool,
    pub edit: EditBase,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupStep {
    pub key: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// `done` (committed), `drafted` (in the working tree only) or `todo`.
    pub state: &'static str,
    pub command: Option<&'static str>,
}

/// The guided setup, shown only until a mission and a rule are committed.
#[derive(Debug, Clone, Serialize)]
pub struct Setup {
    pub steps: Vec<SetupStep>,
    pub starters: Vec<StarterView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PageView {
    pub header: PageHeader,
    pub pending: Option<Pending>,
    pub health: Health,
    pub mission: Option<MissionView>,
    pub principles: Vec<PrincipleView>,
    pub principle_coverage: CoverageView,
    pub groups: Vec<RuleGroup>,
    pub retired: Vec<RetiredView>,
    pub suggestions: Vec<Suggestion>,
    pub history: Vec<HistoryEntry>,
    pub setup: Option<Setup>,
}

/// What the service reads before projecting the page.
pub struct PageInput<'a> {
    pub project_label: String,
    pub branch: Option<String>,
    /// The rulebook as last committed, and its raw JSON.
    pub head: Option<&'a Rulebook>,
    pub head_json: Option<&'a Value>,
    /// The working tree's rulebook file as raw JSON.
    pub book_json: Option<&'a Value>,
    /// The commit message accepting the drafts would use.
    pub message: Option<(String, String)>,
    pub hooks: crate::wire::GitHooks,
    /// Whether a CI workflow runs Whetstone's check.
    pub ci_wired: bool,
    /// Uncovered files and rules that read nothing (`crate::coverage`).
    pub coverage: Option<Value>,
    pub starters: Vec<crate::starter::Starter>,
    pub catalogue: crate::catalogue::Catalogue,
    /// The rulebook's Git history, newest first.
    pub history: Vec<HistoryEntry>,
}

/// How a rule is checked, in plain words.
pub fn check_label(enforcer: &Enforcer) -> &'static str {
    match enforcer {
        _ if enforcer.is_script() => "a script",
        Enforcer::Ast { .. } => "a code pattern",
        Enforcer::Lint { .. } => "a lint",
        Enforcer::Formatter { .. } => "a formatter",
        Enforcer::Test { .. } => "a test",
        Enforcer::Validator { .. } => "a command",
    }
}

fn sweep_label(class: SweepClass) -> &'static str {
    match class {
        SweepClass::Discuss => "Discuss before fixing",
        SweepClass::ConfirmFirst => "Fix once a person approves the plan",
        SweepClass::DraftPr => "An agent may open a fix as a draft PR",
    }
}

fn stages(rule: &Rule) -> Vec<Stage> {
    vec![
        Stage {
            key: "commit",
            label: "commit",
            runs: rule.enforcer.runs_staged(),
        },
        Stage {
            key: "push",
            label: "push",
            runs: true,
        },
        Stage {
            key: "ci",
            label: "CI",
            runs: true,
        },
    ]
}

fn check(rule: &Rule) -> Check {
    let (_, command) = crate::gates::mechanism_label(rule);
    Check {
        kind: rule.enforcer.kind(),
        label: check_label(&rule.enforcer),
        command,
        language: match &rule.enforcer {
            Enforcer::Ast { language, .. } => language.clone(),
            _ => None,
        },
    }
}

fn source(rule: &Rule) -> SourceView {
    let kind = rule.source.kind.label();
    let label = match kind {
        "owner" => "Written by the owner",
        "starter" => "A Whetstone starter rule",
        "pack" => "Imported from a rule pack",
        "principle" => "Derived from a principle",
        "mission" => "Derived from the mission",
        "prose" => "Compiled from prose guidance",
        "finding" => "Drafted from a recurring finding",
        _ => kind,
    };
    SourceView {
        kind,
        label: label.into(),
        reference: rule.source.reference.clone(),
    }
}

fn last_change(entry: &HistoryEntry) -> LastChange {
    LastChange {
        commit: entry.commit.clone(),
        author: entry.author.clone(),
        at: entry.at.clone(),
        subject: entry.subject.clone(),
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn health(input: &PageInput<'_>, book: Option<&Rulebook>) -> Health {
    let mut items = Vec::new();
    let hooks = &input.hooks;
    let missing = [
        ("pre-commit", hooks.pre_commit),
        ("pre-push", hooks.pre_push),
        ("CI", input.ci_wired),
    ]
    .into_iter()
    .filter(|(_, installed)| !installed)
    .map(|(name, _)| name)
    .collect::<Vec<_>>();
    if book.is_some() && !missing.is_empty() {
        let hooks_missing = missing.iter().any(|name| *name != "CI");
        let flags = match (hooks_missing, !input.ci_wired) {
            (true, true) => "--hooks --ci",
            (true, false) => "--hooks",
            _ => "--ci",
        };
        let listed = match missing.as_slice() {
            [only] => (*only).to_string(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
            [] => String::new(),
        };
        items.push(HealthItem {
            kind: "gates",
            tone: "warn",
            text: format!(
                "The {listed} gate{} not installed, so {}.",
                if missing.len() == 1 { " is" } else { "s are" },
                if hooks_missing {
                    "rules run only when someone runs `wh check`"
                } else {
                    "pull requests are not checked"
                }
            ),
            fix: Some(format!("wh init --action wire {flags}")),
            detail: Vec::new(),
        });
    }
    if let Some(coverage) = &input.coverage {
        let dead = coverage["dead_rules"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if !dead.is_empty() {
            items.push(HealthItem {
                kind: "coverage",
                tone: "warn",
                text: format!(
                    "{} no file in this repository.",
                    plural(dead.len(), "rule reads", "rules read")
                ),
                fix: None,
                detail: dead
                    .iter()
                    .map(|rule| {
                        format!(
                            "{}: {}",
                            rule["rule"].as_str().unwrap_or_default(),
                            rule["why"].as_str().unwrap_or_default()
                        )
                    })
                    .collect(),
            });
        }
        let uncovered = coverage["uncovered"]
            .as_array()
            .map(|groups| {
                groups
                    .iter()
                    .filter(|group| group["language"] != "other")
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let files = uncovered
            .iter()
            .filter_map(|group| group["files"].as_u64())
            .sum::<u64>() as usize;
        if files > 0 {
            items.push(HealthItem {
                kind: "coverage",
                tone: "warn",
                text: format!(
                    "{} in {} no rule reads.",
                    plural(files, "source file", "source files"),
                    plural(uncovered.len(), "place", "places")
                ),
                fix: None,
                detail: uncovered
                    .iter()
                    .map(|group| {
                        format!(
                            "{} ({}): {}",
                            group["directory"].as_str().unwrap_or_default(),
                            group["language"].as_str().unwrap_or_default(),
                            plural(
                                group["files"].as_u64().unwrap_or_default() as usize,
                                "file",
                                "files"
                            )
                        )
                    })
                    .collect(),
            });
        }
    }
    if let Some(sentence) = book.and_then(|book| book.coverage().sentence()) {
        items.push(HealthItem {
            kind: "principles",
            tone: "warn",
            text: format!("{sentence}."),
            fix: None,
            detail: Vec::new(),
        });
    }
    let healthy = (book.is_some() && items.is_empty()).then(|| {
        "Gates installed at commit, push and CI; every rule reads files here.".to_string()
    });
    Health { items, healthy }
}

/// The enforcer hint a held principle's pack gives it, when the pack is
/// loaded.
fn pack_hint(input: &PageInput<'_>, source: &PrincipleSource) -> Option<String> {
    let (pack, id) = match source {
        PrincipleSource::Pstack { id, .. } => (crate::catalogue::DEFAULT_PACK, id.as_str()),
        PrincipleSource::Pack { pack, id, .. } => (pack.as_str(), id.as_str()),
        PrincipleSource::Custom => return None,
    };
    input
        .catalogue
        .principles
        .iter()
        .find(|entry| entry.pack == pack && entry.id == id)
        .map(|entry| entry.enforcer_hint.clone())
}

/// What kind of rule could serve a principle with this enforcer hint.
fn hint_text(hint: &str) -> String {
    match hint {
        "none" => {
            "No tool checks this; keep it as a principle, or write a rule that serves it".into()
        }
        hint => format!(
            "Could become a rule checked by {}",
            match hint {
                "lint" => "a lint",
                "test" => "a test",
                "ast" => "a code pattern",
                "formatter" => "a formatter",
                _ => "a script",
            }
        ),
    }
}

/// The chain from principle to rule as the page shows it.
fn coverage_view(book: Option<&Rulebook>) -> CoverageView {
    let Some(book) = book else {
        return CoverageView::default();
    };
    let coverage = book.coverage();
    let principle = |id: &crate::domain::RecordId| CoverageEntry {
        id: id.as_str().into(),
        statement: book
            .principles
            .iter()
            .find(|entry| &entry.id == id)
            .map(|entry| entry.principle.statement.clone())
            .unwrap_or_default(),
    };
    let rule = |id: &crate::domain::RecordId| CoverageEntry {
        id: id.as_str().into(),
        statement: book
            .rules
            .iter()
            .find(|entry| &entry.id == id)
            .map(|entry| entry.rule.statement.clone())
            .unwrap_or_default(),
    };
    CoverageView {
        unserved: coverage.unserved.iter().map(principle).collect(),
        unanchored: coverage.unanchored.iter().map(rule).collect(),
        require_served: book.require_served,
    }
}

fn suggestions(input: &PageInput<'_>, book: Option<&Rulebook>) -> Vec<Suggestion> {
    let held = |id: &str| {
        book.is_some_and(|book| {
            book.rules.iter().any(|entry| entry.id.as_str() == id)
                || book.principles.iter().any(|entry| entry.id.as_str() == id)
        })
    };
    let starters = input
        .starters
        .iter()
        .filter(|starter| !held(starter.id))
        .map(|starter| Suggestion {
            kind: "starter",
            id: starter.id.into(),
            title: starter.title.into(),
            detail: starter.detected.clone(),
            strength: Some(starter.rule.strength.label()),
            check: format!("Checked by {}", check_label(&starter.rule.enforcer)),
            group: None,
            starting: false,
            edit: rule_edit(starter.id, &starter.rule),
        });
    let principles = input
        .catalogue
        .principles
        .iter()
        .filter(|entry| !held(&entry.record_id()))
        .map(|entry| Suggestion {
            kind: "principle",
            id: entry.record_id(),
            title: entry.text.clone(),
            detail: format!("{} {}", entry.pack, entry.pack_version),
            strength: None,
            check: hint_text(&entry.enforcer_hint),
            group: Some(entry.group.clone()),
            starting: entry.starting,
            edit: EditBase {
                kind: "principle",
                record_id: None,
                content: entry.text.clone(),
                rationale: entry.rationale.clone(),
                definition: json!({"type": "principle", "principle": entry.spec}),
            },
        });
    starters.chain(principles).collect()
}

/// Project the page from the working tree's rulebook (absent before one is
/// written).
pub fn page_view(book: Option<&Rulebook>, input: PageInput<'_>) -> PageView {
    let lifecycle = |id: &str| -> &'static str {
        let now = book.and_then(|book| rulebook::entry(book, id));
        match input.head.and_then(|head| rulebook::entry(head, id)) {
            None => "new",
            Some(then) if Some(&then) == now.as_ref() => "accepted",
            Some(_) => "changed",
        }
    };
    let events = |id: &str| -> Vec<RuleEvent> {
        input
            .history
            .iter()
            .filter_map(|entry| {
                let change = entry.changes.iter().find(|change| change.id == id)?;
                Some(RuleEvent {
                    commit: entry.commit.clone(),
                    short: entry.short.clone(),
                    author: entry.author.clone(),
                    at: entry.at.clone(),
                    subject: entry.subject.clone(),
                    action: change.action,
                    fields: change.fields.clone(),
                })
            })
            .collect()
    };
    let mission = book
        .filter(|book| !book.mission.statement.trim().is_empty())
        .map(|book| MissionView {
            statement: book.mission.statement.clone(),
            lifecycle: match input.head.map(|head| head.mission.statement.as_str()) {
                Some(then) if then == book.mission.statement => "accepted",
                Some(then) if !then.trim().is_empty() => "changed",
                _ => "new",
            },
            edit: EditBase {
                kind: "mission",
                record_id: None,
                content: book.mission.statement.clone(),
                rationale: None,
                definition: Value::Null,
            },
        });
    let principles = book
        .map(|book| book.principles.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|entry| PrincipleView {
            id: entry.id.as_str().into(),
            statement: entry.principle.statement.clone(),
            rationale: entry.principle.rationale.clone(),
            source: match &entry.principle.source {
                PrincipleSource::Pstack { version, .. } => format!("pstack {version}"),
                PrincipleSource::Pack { pack, version, .. } => format!("{pack} {version}"),
                PrincipleSource::Custom => "the owner's own".into(),
            },
            lifecycle: lifecycle(entry.id.as_str()),
            edit: EditBase {
                kind: "principle",
                record_id: Some(entry.id.as_str().into()),
                content: entry.principle.statement.clone(),
                rationale: entry.principle.rationale.clone(),
                definition: json!({
                    "type": "principle",
                    "principle": match &entry.principle.source {
                        PrincipleSource::Pstack { id, .. } => Some(id.clone()),
                        PrincipleSource::Pack { pack, id, .. } => Some(format!("{pack}/{id}")),
                        PrincipleSource::Custom => None,
                    },
                    "rationale": entry.principle.rationale,
                }),
            },
            served_by: book
                .map(|book| book.served_by(&entry.id))
                .unwrap_or_default()
                .iter()
                .map(|rule| rule.id.as_str().to_string())
                .collect(),
            enforcer_hint: pack_hint(&input, &entry.principle.source),
            hint: hint_text(
                pack_hint(&input, &entry.principle.source)
                    .as_deref()
                    .unwrap_or("none"),
            ),
        })
        .collect::<Vec<_>>();
    let mut rules = book
        .map(|book| book.rules.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|entry| {
            let id = entry.id.as_str();
            let rule = &entry.rule;
            RuleView {
                id: id.into(),
                statement: rule.statement.clone(),
                rationale: rule.rationale.clone(),
                strength: rule.strength.label(),
                check: check(rule),
                stages: stages(rule),
                paths: rule.paths.clone(),
                examples: examples(rule),
                source: source(rule),
                serves: book
                    .and_then(|book| book.principle_for(rule))
                    .map(|entry| ServesView {
                        id: entry.id.as_str().into(),
                        statement: entry.principle.statement.clone(),
                    }),
                sweep: SweepView {
                    class: rule.sweep_class().label(),
                    label: sweep_label(rule.sweep_class()),
                    priority: rule.sweep_priority(),
                    unsure: rule.sweep_unsure.clone(),
                },
                lifecycle: lifecycle(id),
                digest: entry.digest.as_str().into(),
                history: events(id),
                edit: rule_edit(id, rule),
            }
        })
        .collect::<Vec<_>>();
    rules.sort_by(|left, right| left.id.cmp(&right.id));
    let (blocks, reports): (Vec<_>, Vec<_>) = rules
        .into_iter()
        .partition(|rule| rule.strength == Strength::Must.label());
    let counts = Counts {
        rules: blocks.len() + reports.len(),
        blocks: blocks.len(),
        reports: reports.len(),
        principles: principles.len(),
        retired: book.map_or(0, |book| book.retired.len()),
    };
    let groups = vec![
        RuleGroup {
            strength: "must",
            label: "Hard rules",
            hint: "Must hold. A violation fails the commit, the push or the CI check.",
            rules: blocks,
        },
        RuleGroup {
            strength: "should",
            label: "Soft rules",
            hint: "Should hold. A violation is reported as a finding and never blocks.",
            rules: reports,
        },
    ];
    let pending = match (input.book_json, &input.message) {
        (Some(after), Some((subject, body))) => {
            let changes = diff::diff(input.head_json, after);
            (!changes.is_empty()).then(|| Pending {
                count: changes.len(),
                summary: diff::summary(&changes),
                changes,
                message: Message {
                    subject: subject.clone(),
                    body: body.clone(),
                },
            })
        }
        _ => None,
    };
    let retired =
        book.map(|book| book.retired.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|entry| RetiredView {
                id: entry.id.as_str().into(),
                reason: entry.reason.clone(),
                retired_in: input
                    .history
                    .iter()
                    .find(|history| {
                        history.changes.iter().any(|change| {
                            change.id == entry.id.as_str() && change.action == "retired"
                        })
                    })
                    .map(last_change),
            })
            .collect();
    let committed = input
        .head
        .is_some_and(|head| !head.mission.statement.trim().is_empty() && !head.rules.is_empty());
    let step_state = |done: bool, drafted: bool| match (done, drafted) {
        (true, _) => "done",
        (false, true) => "drafted",
        _ => "todo",
    };
    let setup = (!committed).then(|| Setup {
        steps: vec![
            SetupStep {
                key: "mission",
                label: "Mission",
                hint: "What this project exists to do, in one sentence.",
                state: step_state(
                    input
                        .head
                        .is_some_and(|head| !head.mission.statement.trim().is_empty()),
                    mission.is_some(),
                ),
                command: None,
            },
            SetupStep {
                key: "rules",
                label: "Rules",
                hint: "Adopt the starter rules detected here, or write your own.",
                state: step_state(
                    input.head.is_some_and(|head| !head.rules.is_empty()),
                    counts.rules > 0,
                ),
                command: None,
            },
            SetupStep {
                key: "gates",
                label: "Gates",
                hint: "Install the pre-commit and pre-push hooks that run the rules.",
                state: step_state(input.hooks.pre_commit && input.hooks.pre_push, false),
                command: Some("wh init --action wire --hooks"),
            },
        ],
        starters: input
            .starters
            .iter()
            .map(|starter| {
                starter_view(
                    starter,
                    book.is_some_and(|book| {
                        book.rules
                            .iter()
                            .any(|entry| entry.id.as_str() == starter.id)
                    }),
                )
            })
            .collect(),
    });
    let summary = format!(
        "{}: {} hard, {} soft",
        plural(counts.rules, "rule", "rules"),
        counts.blocks,
        counts.reports
    );
    PageView {
        header: PageHeader {
            project: input.project_label.clone(),
            branch: input.branch.clone(),
            revision: input.history.first().map(|entry| entry.commit.clone()),
            summary,
            counts,
            last_change: input.history.first().map(last_change),
        },
        pending,
        health: health(&input, book),
        mission,
        principles,
        principle_coverage: coverage_view(book),
        groups,
        retired,
        suggestions: suggestions(&input, book),
        history: input.history.clone(),
        setup,
    }
}
