//! Four-layer rule resolution: personal > project > team > built-in.
//!
//! Each layer carries its own `deny` list that removes rules by id from
//! any broader layer. The merge follows "most specific wins":
//!
//!   personal  (gitignored, local-only — `whetstone/.personal/rules/`)
//!   project   (committed — `whetstone/rules/`)
//!   team      (committed or fetched via `extends:` — `whetstone/.cache/teams/...`)
//!   built-in  (embedded in the binary — `src/builtin/*.yaml`)
//!
//! Denies apply at that layer and upwards, i.e. `project.deny: [foo]`
//! removes `foo` from the project/team/built-in pool but personal can
//! still re-introduce `foo` via its own override.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::WhetstoneConfig;
use crate::rules::{
    self, approved_from_loaded, load_approved_rules, load_rule_files, ApprovedExample,
    ApprovedRule, ApprovedSignal,
};

/// Identifies which layer a merged rule came from. Written into generated
/// outputs so users can tell at a glance where a rule originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Personal,
    Project,
    Team,
    BuiltIn,
}

impl Layer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Layer::Personal => "personal",
            Layer::Project => "project",
            Layer::Team => "team",
            Layer::BuiltIn => "built-in",
        }
    }
}

/// Directory paths for each layer. Missing directories are treated as empty.
pub struct LayerPaths {
    pub personal_rules_dir: PathBuf,
    pub project_rules_dir: PathBuf,
    pub team_rules_dirs: Vec<PathBuf>,
}

impl LayerPaths {
    pub fn for_project(project_dir: &Path) -> Self {
        let base = project_dir.join("whetstone");
        let mut team_rules_dirs = Vec::new();
        // Local team-publish staging (`wh promote --to team` writes here) feeds
        // the team layer alongside anything pulled in by `extends:`.
        let local_team = base.join(".team").join("rules");
        if local_team.exists() {
            team_rules_dirs.push(local_team);
        }
        LayerPaths {
            personal_rules_dir: base.join(".personal").join("rules"),
            project_rules_dir: base.join("rules"),
            team_rules_dirs,
        }
    }
}

/// Per-layer deny lists, pulled from the relevant config files.
#[derive(Debug, Default, Clone)]
pub struct LayerDenies {
    pub personal: Vec<String>,
    pub project: Vec<String>,
    pub team: Vec<String>,
}

/// A fully-merged approved rule, annotated with the layer it came from.
pub struct LayeredRule {
    pub rule: ApprovedRule,
    pub layer: Layer,
}

pub struct LayerSet {
    pub personal: Vec<ApprovedRule>,
    pub project: Vec<ApprovedRule>,
    pub team: Vec<ApprovedRule>,
    pub builtin: Vec<ApprovedRule>,
    pub denies: LayerDenies,
}

impl LayerSet {
    /// Load every layer that exists for this project.
    pub fn load(
        project_dir: &Path,
        lang_filter: Option<&str>,
        include_builtin: bool,
    ) -> (Self, Vec<String>) {
        let paths = LayerPaths::for_project(project_dir);
        let mut warnings = Vec::new();

        let (project, mut pw) = load_approved_rules(&paths.project_rules_dir, lang_filter);
        warnings.append(&mut pw);

        // Personal layer: load only if the directory exists. Rules are loaded
        // with the same schema + validation as project rules.
        let (personal, mut person_w) = if paths.personal_rules_dir.exists() {
            load_approved_rules(&paths.personal_rules_dir, lang_filter)
        } else {
            (Vec::new(), Vec::new())
        };
        warnings.append(&mut person_w);

        // Team layers: merge all team extensions into a single Vec.
        let mut team = Vec::new();
        for dir in &paths.team_rules_dirs {
            if dir.exists() {
                let (mut rules, mut w) = load_approved_rules(dir, lang_filter);
                team.append(&mut rules);
                warnings.append(&mut w);
            }
        }

        let builtin = if include_builtin {
            let (b, _) = approved_from_loaded(&crate::builtin::load_builtin_rules(), lang_filter);
            b
        } else {
            Vec::new()
        };

        let denies = load_denies(project_dir);

        (
            LayerSet {
                personal,
                project,
                team,
                builtin,
                denies,
            },
            warnings,
        )
    }

    /// Produce the final merged, layer-annotated rule set.
    ///
    /// Precedence: personal > project > team > built-in. Deny lists at each
    /// level excise the denied id from that level and all broader levels.
    pub fn merge(&self) -> Vec<LayeredRule> {
        let personal_ids: HashSet<&str> = self.personal.iter().map(|r| r.id.as_str()).collect();
        let project_ids: HashSet<&str> = self.project.iter().map(|r| r.id.as_str()).collect();
        let team_ids: HashSet<&str> = self.team.iter().map(|r| r.id.as_str()).collect();

        let personal_deny: HashSet<&str> =
            self.denies.personal.iter().map(|s| s.as_str()).collect();
        let project_deny: HashSet<&str> =
            self.denies.project.iter().map(|s| s.as_str()).collect();
        let team_deny: HashSet<&str> = self.denies.team.iter().map(|s| s.as_str()).collect();

        // Any id that a narrower deny excludes is removed from broader layers too.
        let mut merged = Vec::new();

        for rule in &self.personal {
            if personal_deny.contains(rule.id.as_str()) {
                continue;
            }
            merged.push(LayeredRule {
                rule: clone_rule(rule),
                layer: Layer::Personal,
            });
        }

        for rule in &self.project {
            if personal_deny.contains(rule.id.as_str())
                || project_deny.contains(rule.id.as_str())
                || personal_ids.contains(rule.id.as_str())
            {
                continue;
            }
            merged.push(LayeredRule {
                rule: clone_rule(rule),
                layer: Layer::Project,
            });
        }

        for rule in &self.team {
            if personal_deny.contains(rule.id.as_str())
                || project_deny.contains(rule.id.as_str())
                || team_deny.contains(rule.id.as_str())
                || personal_ids.contains(rule.id.as_str())
                || project_ids.contains(rule.id.as_str())
            {
                continue;
            }
            merged.push(LayeredRule {
                rule: clone_rule(rule),
                layer: Layer::Team,
            });
        }

        for rule in &self.builtin {
            if personal_deny.contains(rule.id.as_str())
                || project_deny.contains(rule.id.as_str())
                || team_deny.contains(rule.id.as_str())
                || personal_ids.contains(rule.id.as_str())
                || project_ids.contains(rule.id.as_str())
                || team_ids.contains(rule.id.as_str())
            {
                continue;
            }
            merged.push(LayeredRule {
                rule: clone_rule(rule),
                layer: Layer::BuiltIn,
            });
        }

        merged
    }

    /// Summary used by the CLI and tests — tallies by layer and total after merge.
    pub fn summary(&self) -> BTreeMap<String, usize> {
        let merged = self.merge();
        let mut out = BTreeMap::new();
        out.insert("personal".to_string(), 0);
        out.insert("project".to_string(), 0);
        out.insert("team".to_string(), 0);
        out.insert("built-in".to_string(), 0);
        for lr in &merged {
            *out.entry(lr.layer.as_str().to_string()).or_insert(0) += 1;
        }
        out.insert("total".to_string(), merged.len());
        out
    }
}

/// Load deny lists from the three relevant config files:
/// - `whetstone/whetstone.yaml` (project layer)
/// - `whetstone/.personal/config.yaml` (personal layer)
/// - team denies are embedded in team configs — `LayerSet::load` currently
///   leaves this empty; hydrate via `LayerSet::denies.team` before calling
///   `merge()` if you want team-level deny semantics.
pub fn load_denies(project_dir: &Path) -> LayerDenies {
    let project_cfg = WhetstoneConfig::load(project_dir);
    let personal_cfg_path = project_dir
        .join("whetstone")
        .join(".personal")
        .join("config.yaml");
    let personal = if personal_cfg_path.exists() {
        load_personal_deny(&personal_cfg_path)
    } else {
        Vec::new()
    };
    LayerDenies {
        personal,
        project: project_cfg.deny,
        team: Vec::new(),
    }
}

fn load_personal_deny(path: &Path) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Shape {
        #[serde(default)]
        deny: Vec<String>,
    }
    match std::fs::read_to_string(path) {
        Ok(text) => serde_yaml::from_str::<Shape>(&text)
            .map(|s| s.deny)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Convenience: return only the merged `ApprovedRule` values, dropping the
/// layer annotation. Used by the existing generators that don't yet render
/// layer provenance.
#[allow(dead_code)]
pub fn merge_to_approved(project_dir: &Path, lang_filter: Option<&str>) -> Vec<ApprovedRule> {
    let (layers, _) = LayerSet::load(project_dir, lang_filter, true);
    layers
        .merge()
        .into_iter()
        .map(|lr| lr.rule)
        .collect()
}

// ── helpers ──

/// `ApprovedRule` does not implement `Clone`, so spell it out here. Keep this
/// in sync with `ApprovedRule` — if you add a field there, mirror it here.
pub fn clone_rule(rule: &ApprovedRule) -> ApprovedRule {
    ApprovedRule {
        id: rule.id.clone(),
        severity: rule.severity.clone(),
        confidence: rule.confidence.clone(),
        category: rule.category.clone(),
        description: rule.description.clone(),
        source_url: rule.source_url.clone(),
        source_name: rule.source_name.clone(),
        language: rule.language.clone(),
        signals: rule
            .signals
            .iter()
            .map(|s| ApprovedSignal {
                id: s.id.clone(),
                strategy: s.strategy.clone(),
                description: s.description.clone(),
                weight: s.weight.clone(),
                match_pattern: s.match_pattern.clone(),
            })
            .collect(),
        golden_examples: rule
            .golden_examples
            .iter()
            .map(|e| ApprovedExample {
                code: e.code.clone(),
                verdict: e.verdict.clone(),
                reason: e.reason.clone(),
                language: e.language.clone(),
            })
            .collect(),
        risk: rule.risk.clone(),
        linter_gap: rule.linter_gap.clone(),
        deterministic_pass_threshold: rule.deterministic_pass_threshold,
        deterministic_fail_threshold: rule.deterministic_fail_threshold,
        ai_eval: rule.ai_eval.clone(),
    }
}

/// Load just the personal approved rules (no merging). Used by personal
/// output routing so outputs at `.personal/` contain ONLY the personal rules.
pub fn load_personal_only(
    project_dir: &Path,
    lang_filter: Option<&str>,
) -> (Vec<ApprovedRule>, Vec<String>) {
    let paths = LayerPaths::for_project(project_dir);
    if !paths.personal_rules_dir.exists() {
        return (Vec::new(), Vec::new());
    }
    rules::load_approved_rules(&paths.personal_rules_dir, lang_filter)
}

/// Shared helper: locate the YAML file a given rule id lives in. Used by the
/// `promote` command to rewrite rule files without re-parsing every layer.
pub fn find_rule_file(rules_dir: &Path, rule_id: &str) -> Option<PathBuf> {
    if !rules_dir.exists() {
        return None;
    }
    let (files, _) = load_rule_files(rules_dir);
    for lrf in &files {
        for rule in &lrf.rule_file.rules {
            if rule.id == rule_id {
                return Some(PathBuf::from(&lrf.file_path));
            }
        }
    }
    None
}
