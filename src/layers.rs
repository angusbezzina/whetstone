//! Two-layer rule resolution: personal > project.
//!
//! Each layer carries its own `deny` list that removes rules by id from
//! the other layer. The merge follows "most specific wins":
//!
//!   personal  (gitignored, local-only — `whetstone/.personal/rules/`)
//!   project   (committed — `whetstone/rules/`)
//!
//! Denies apply at that layer and upwards, i.e. `project.deny: [foo]`
//! removes `foo` from the project pool but personal can still
//! re-introduce `foo` via its own override.
//!
//! Team and built-in layers were removed as part of the lean refactor
//! (bead whetstone-aww); `include_builtin` / `refresh_team` arguments
//! are retained on public APIs for call-site compatibility but ignored.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use crate::config::{PersonalConfig, WhetstoneConfig};
use crate::rules::{load_approved_rules, load_rule_files, ApprovedRule};
use serde_json::Value;

/// Identifies which layer a merged rule came from. Written into generated
/// outputs so users can tell at a glance where a rule originated.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Personal,
    Project,
}

impl Layer {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Layer::Personal => "personal",
            Layer::Project => "project",
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
}

impl LayerPaths {
    pub fn for_project(project_dir: &Path) -> Self {
        let whetstone_dir = project_dir.join("whetstone");
        let personal_dir = whetstone_dir.join(".personal");
        LayerPaths {
            personal_rules_dir: personal_dir.join("rules"),
            personal_config: personal_dir.join("config.yaml"),
            personal_dir,
            project_rules_dir: whetstone_dir.join("rules"),
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
}

/// A fully-merged approved rule, annotated with the layer it came from.
#[allow(dead_code)]
pub struct LayeredRule {
    pub rule: ApprovedRule,
    pub layer: Layer,
}

pub struct LayerSet {
    pub personal: Vec<ApprovedRule>,
    pub project: Vec<ApprovedRule>,
}

pub struct ResolvedLayers {
    pub merged: Vec<LayeredRule>,
    pub warnings: Vec<String>,
}

impl LayerSet {
    /// Load every layer that exists for this project.
    pub fn load(project_dir: &Path, lang_filter: Option<&str>) -> (Self, Vec<String>) {
        let paths = LayerPaths::for_project(project_dir);
        let mut warnings = Vec::new();

        let (project, mut pw) = load_approved_rules(&paths.project_rules_dir, lang_filter);
        warnings.append(&mut pw);

        let (personal, mut person_w) = load_approved_rules(&paths.personal_rules_dir, lang_filter);
        warnings.append(&mut person_w);

        (LayerSet { personal, project }, warnings)
    }

    /// Produce the final merged, layer-annotated rule set.
    ///
    /// Precedence: personal > project. Deny lists at each level excise the
    /// denied id from that level and the broader layer.
    pub fn merge(&self, denies: &LayerDenies) -> Vec<LayeredRule> {
        let personal_ids: HashSet<&str> = self.personal.iter().map(|r| r.id.as_str()).collect();

        let personal_deny: HashSet<&str> = denies.personal.iter().map(String::as_str).collect();
        let project_deny: HashSet<&str> = denies.project.iter().map(String::as_str).collect();

        type Plan<'a> = (&'a Vec<ApprovedRule>, Layer, Vec<&'a HashSet<&'a str>>);
        let plans: [Plan; 2] = [
            (&self.personal, Layer::Personal, vec![&personal_deny]),
            (
                &self.project,
                Layer::Project,
                vec![&personal_deny, &project_deny, &personal_ids],
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
#[allow(dead_code)]
pub fn summary_from(merged: &[LayeredRule]) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for layer in [Layer::Personal, Layer::Project] {
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
pub fn load_denies(project_dir: &Path) -> LayerDenies {
    let project_cfg = WhetstoneConfig::load(project_dir);
    let paths = LayerPaths::for_project(project_dir);
    LayerDenies {
        personal: PersonalConfig::load(&paths.personal_config).deny,
        project: project_cfg.deny,
    }
}

/// Resolve every configured rule layer into a single merged rule set.
///
/// `include_personal=false` strips both personal rules and personal deny-list
/// effects so committed outputs never depend on a user's local-only layer.
/// The `_include_builtin` and `_refresh_team` parameters are retained for
/// backward compatibility but ignored — both layers were removed in the
/// lean refactor.
pub fn resolve_merged(
    project_dir: &Path,
    lang_filter: Option<&str>,
    _include_builtin: bool,
    include_personal: bool,
    _refresh_team: bool,
) -> ResolvedLayers {
    let (mut layers, warnings) = LayerSet::load(project_dir, lang_filter);
    let mut denies = load_denies(project_dir);

    if !include_personal {
        layers.personal.clear();
        denies.personal.clear();
    }

    let merged = layers.merge(&denies);
    ResolvedLayers { merged, warnings }
}

/// Convenience: return only the merged `ApprovedRule` values, dropping the
/// layer annotation. Used by the existing generators that don't yet render
/// layer provenance.
#[allow(dead_code)]
pub fn merge_to_approved(project_dir: &Path, lang_filter: Option<&str>) -> Vec<ApprovedRule> {
    resolve_merged(project_dir, lang_filter, true, true, false)
        .merged
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
    crate::rules::load_approved_rules(&paths.personal_rules_dir, lang_filter)
}

/// Shared helper: locate the YAML file a given rule id lives in. Used by
/// anything that needs to rewrite rule files without re-parsing every layer.
#[allow(dead_code)]
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

// Suppress unused warning on Value import if no call site needs it.
#[allow(dead_code)]
fn _value_marker(_v: Value) {}
