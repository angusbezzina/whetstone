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

use crate::config::{PersonalConfig, WhetstoneConfig};
use crate::rules::{self, approved_from_loaded, load_approved_rules, load_rule_files, ApprovedRule};

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
    pub whetstone_dir: PathBuf,
    pub personal_dir: PathBuf,
    pub personal_rules_dir: PathBuf,
    pub personal_config: PathBuf,
    pub project_rules_dir: PathBuf,
    pub team_staging_rules_dir: PathBuf,
    pub team_rules_dirs: Vec<PathBuf>,
}

impl LayerPaths {
    pub fn for_project(project_dir: &Path) -> Self {
        let whetstone_dir = project_dir.join("whetstone");
        let personal_dir = whetstone_dir.join(".personal");
        let team_staging = whetstone_dir.join(".team").join("rules");
        let mut team_rules_dirs = Vec::new();
        if team_staging.exists() {
            team_rules_dirs.push(team_staging.clone());
        }
        LayerPaths {
            personal_rules_dir: personal_dir.join("rules"),
            personal_config: personal_dir.join("config.yaml"),
            personal_dir,
            project_rules_dir: whetstone_dir.join("rules"),
            team_staging_rules_dir: team_staging,
            team_rules_dirs,
            whetstone_dir,
        }
    }

    pub fn personal_context(&self) -> PathBuf {
        self.personal_dir.join("context")
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

        let (personal, mut person_w) =
            load_approved_rules(&paths.personal_rules_dir, lang_filter);
        warnings.append(&mut person_w);

        let mut team = Vec::new();
        for dir in &paths.team_rules_dirs {
            let (mut rules, mut w) = load_approved_rules(dir, lang_filter);
            team.append(&mut rules);
            warnings.append(&mut w);
        }

        let builtin = if include_builtin {
            let (b, _) = approved_from_loaded(&crate::builtin::load_builtin_rules(), lang_filter);
            b
        } else {
            Vec::new()
        };

        (
            LayerSet {
                personal,
                project,
                team,
                builtin,
            },
            warnings,
        )
    }

    /// Produce the final merged, layer-annotated rule set.
    ///
    /// Precedence: personal > project > team > built-in. Deny lists at each
    /// level excise the denied id from that level and all broader levels.
    pub fn merge(&self, denies: &LayerDenies) -> Vec<LayeredRule> {
        let personal_ids: HashSet<&str> = self.personal.iter().map(|r| r.id.as_str()).collect();
        let project_ids: HashSet<&str> = self.project.iter().map(|r| r.id.as_str()).collect();
        let team_ids: HashSet<&str> = self.team.iter().map(|r| r.id.as_str()).collect();

        let personal_deny: HashSet<&str> = denies.personal.iter().map(String::as_str).collect();
        let project_deny: HashSet<&str> = denies.project.iter().map(String::as_str).collect();
        let team_deny: HashSet<&str> = denies.team.iter().map(String::as_str).collect();

        // Each layer's entry lists every id-set whose membership means "skip
        // this rule". Narrower denies cascade outward: project deny silences
        // the project/team/built-in layers; a personal override shadows the
        // same id in every broader layer.
        type Plan<'a> = (&'a Vec<ApprovedRule>, Layer, Vec<&'a HashSet<&'a str>>);
        let plans: [Plan; 4] = [
            (&self.personal, Layer::Personal, vec![&personal_deny]),
            (
                &self.project,
                Layer::Project,
                vec![&personal_deny, &project_deny, &personal_ids],
            ),
            (
                &self.team,
                Layer::Team,
                vec![
                    &personal_deny,
                    &project_deny,
                    &team_deny,
                    &personal_ids,
                    &project_ids,
                ],
            ),
            (
                &self.builtin,
                Layer::BuiltIn,
                vec![
                    &personal_deny,
                    &project_deny,
                    &team_deny,
                    &personal_ids,
                    &project_ids,
                    &team_ids,
                ],
            ),
        ];

        let mut merged = Vec::new();
        for (rules, layer, excludes) in plans {
            for rule in rules {
                if excludes.iter().any(|s| s.contains(rule.id.as_str())) {
                    continue;
                }
                merged.push(LayeredRule {
                    rule: rule.clone(),
                    layer,
                });
            }
        }
        merged
    }
}

/// Summary keyed by `Layer::as_str()` plus a `"total"` entry.
pub fn summary_from(merged: &[LayeredRule]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for layer in [Layer::Personal, Layer::Project, Layer::Team, Layer::BuiltIn] {
        out.insert(layer.as_str().to_string(), 0);
    }
    for lr in merged {
        *out.entry(lr.layer.as_str().to_string()).or_insert(0) += 1;
    }
    out.insert("total".to_string(), merged.len());
    out
}

/// Load deny lists from the relevant config files:
/// - `whetstone/whetstone.yaml` (project layer — merges global config too)
/// - `whetstone/.personal/config.yaml` (personal layer)
/// - team denies travel with team configs; for extends-driven layers the
///   caller is expected to hydrate `denies.team` if they want that semantics.
pub fn load_denies(project_dir: &Path) -> LayerDenies {
    let project_cfg = WhetstoneConfig::load(project_dir);
    let paths = LayerPaths::for_project(project_dir);
    LayerDenies {
        personal: PersonalConfig::load(&paths.personal_config).deny,
        project: project_cfg.deny,
        team: Vec::new(),
    }
}

/// Convenience: return only the merged `ApprovedRule` values, dropping the
/// layer annotation. Used by the existing generators that don't yet render
/// layer provenance.
#[allow(dead_code)]
pub fn merge_to_approved(project_dir: &Path, lang_filter: Option<&str>) -> Vec<ApprovedRule> {
    let (layers, _) = LayerSet::load(project_dir, lang_filter, true);
    let denies = load_denies(project_dir);
    layers
        .merge(&denies)
        .into_iter()
        .map(|lr| lr.rule)
        .collect()
}

/// Load just the personal approved rules (no merging). Used by personal
/// output routing so outputs at `.personal/` contain ONLY the personal rules.
pub fn load_personal_only(
    project_dir: &Path,
    lang_filter: Option<&str>,
) -> (Vec<ApprovedRule>, Vec<String>) {
    let paths = LayerPaths::for_project(project_dir);
    rules::load_approved_rules(&paths.personal_rules_dir, lang_filter)
}

/// Shared helper: locate the YAML file a given rule id lives in. Used by the
/// `promote` command to rewrite rule files without re-parsing every layer.
pub fn find_rule_file(rules_dir: &Path, rule_id: &str) -> Option<PathBuf> {
    let (files, _) = load_rule_files(rules_dir);
    files.into_iter().find_map(|lrf| {
        lrf.rule_file
            .rules
            .iter()
            .any(|r| r.id == rule_id)
            .then(|| PathBuf::from(&lrf.file_path))
    })
}
