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

use crate::config::{ConfigSnapshot, PersonalConfig, SnapshotOptions, WhetstoneConfig};
use crate::config_packs;
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
    /// Load every layer, resolving imported packs with explicit snapshot options
    /// (read-only / injected candidate packs — whetstone-dva).
    pub fn load_with(
        project_dir: &Path,
        lang_filter: Option<&str>,
        opts: &SnapshotOptions,
    ) -> (Self, Vec<String>) {
        let paths = LayerPaths::for_project(project_dir);
        let mut warnings = Vec::new();

        let (project_local, mut pw) = load_approved_rules(&paths.project_rules_dir, lang_filter);
        warnings.append(&mut pw);

        let (mut imported, mut import_warnings) =
            load_imported_pack_rules_with(project_dir, lang_filter, opts);
        warnings.append(&mut import_warnings);

        let local_ids: HashSet<&str> = project_local.iter().map(|r| r.id.as_str()).collect();
        imported.retain(|r| !local_ids.contains(r.id.as_str()));

        let mut project = project_local;
        project.extend(imported);

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

fn load_imported_pack_rules_with(
    project_dir: &Path,
    lang_filter: Option<&str>,
    opts: &SnapshotOptions,
) -> (Vec<ApprovedRule>, Vec<String>) {
    let snapshot = ConfigSnapshot::load_project_snapshot_with(project_dir, opts);
    let mut warnings: Vec<String> = snapshot
        .diagnostics
        .iter()
        .filter(|d| d.layer == crate::config::ConfigLayer::Project)
        .map(|d| format!("{}: {}", d.path.display(), d.message))
        .collect();
    let (rules, mut pack_warnings) =
        config_packs::merge_pack_rules(&snapshot.active_packs, lang_filter);
    warnings.append(&mut pack_warnings);
    (rules, warnings)
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

/// Load deny lists from the project (`whetstone/whetstone.yaml`, merges global)
/// and personal (`whetstone/.personal/config.yaml`) config, using explicit
/// snapshot options so a read-only preview never writes the pack cache while
/// resolving the effective config (whetstone-dva).
pub fn load_denies_with(project_dir: &Path, opts: &SnapshotOptions) -> LayerDenies {
    let project_cfg = WhetstoneConfig::load_with(project_dir, opts);
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
    include_builtin: bool,
    include_personal: bool,
    refresh_team: bool,
) -> ResolvedLayers {
    resolve_merged_with(
        project_dir,
        lang_filter,
        include_builtin,
        include_personal,
        refresh_team,
        &SnapshotOptions::default(),
    )
}

/// Like `resolve_merged` but resolves imported packs with explicit snapshot
/// options — read-only and/or with injected candidate packs (whetstone-dva).
/// Candidate rules flow through the identical merge/shadow/deny path, so a
/// preview sees exactly what a real import would produce.
pub fn resolve_merged_with(
    project_dir: &Path,
    lang_filter: Option<&str>,
    _include_builtin: bool,
    include_personal: bool,
    _refresh_team: bool,
    opts: &SnapshotOptions,
) -> ResolvedLayers {
    let (mut layers, warnings) = LayerSet::load_with(project_dir, lang_filter, opts);
    let mut denies = load_denies_with(project_dir, opts);

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

/// Has the project been initialized as a whetstone project? True if ANY of
/// these exists: whetstone/whetstone.yaml, whetstone.yaml, whetstone/rules/,
/// or whetstone/.personal/rules/. Callers use this to decide whether to
/// call `resolve_merged` (which walks both layers) or fall back to a
/// direct load. Previously callers only checked for whetstone.yaml, which
/// meant personal-only projects (`wh rule add --personal` without explicit
/// init) silently dropped their rules from every generator. Epic 3E follow-up.
pub fn project_is_initialized(project_dir: &Path) -> bool {
    let paths = LayerPaths::for_project(project_dir);
    paths.whetstone_dir.join("whetstone.yaml").exists()
        || project_dir.join("whetstone.yaml").exists()
        || paths.project_rules_dir.exists()
        || paths.personal_rules_dir.exists()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn imported_pack_rules_join_project_layer() {
        let td = tempfile::tempdir().unwrap();
        let wh = td.path().join("whetstone");
        let packs = wh.join("packs");
        fs::create_dir_all(&packs).unwrap();
        fs::write(
            packs.join("base.yaml"),
            r#"apiVersion: whetstone/v1alpha1
kind: RulePack
metadata:
  name: acme.base
  scope: org
language: python
rules:
  - id: fastapi.async-routes
    severity: must
    confidence: high
    category: convention
    description: Route handlers must use async def.
    source_url: https://example.com/async
    approved: true
    status: approved
    signals:
      - id: sync-def
        strategy: pattern
        description: Detect sync route handlers
        weight: required
        match: '@app\\.(get|post).*\\ndef '
    golden_examples:
      - code: |
          @app.get("/")
          async def index(): ...
        verdict: pass
        reason: async route
"#,
        )
        .unwrap();
        fs::write(
            wh.join("whetstone.yaml"),
            r#"version: 1
extends:
  - scope: org
    ref: path:./whetstone/packs/base.yaml
"#,
        )
        .unwrap();

        let resolved = resolve_merged(td.path(), Some("python"), true, true, false);
        assert!(resolved
            .merged
            .iter()
            .any(|lr| lr.rule.id == "fastapi.async-routes" && lr.layer == Layer::Project));
    }

    // ── whetstone-dva: read-only + injectable snapshot seam ──

    fn candidate_pack(dir: &Path, rule_id: &str) -> PathBuf {
        let p = dir.join("cand.yaml");
        fs::write(
            &p,
            format!(
                r#"apiVersion: whetstone/v1alpha1
kind: RulePack
metadata:
  name: acme.candidate
  scope: candidate
language: python
rules:
  - id: {rule_id}
    severity: should
    confidence: high
    category: convention
    description: Candidate rule.
    source_url: https://example.com/cand
    approved: true
    status: approved
    signals:
      - id: s
        strategy: ast
        description: match
        weight: required
        ast_query: '(function_definition) @match'
    golden_examples:
      - code: "def f(): pass"
        verdict: fail
        reason: y
"#
            ),
        )
        .unwrap();
        p
    }

    fn hash_state_dir(project_dir: &Path) -> String {
        let state = project_dir.join("whetstone").join(".state");
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
            if let Ok(rd) = fs::read_dir(dir) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, base, out);
                    } else if let Ok(bytes) = fs::read(&p) {
                        let rel = p.strip_prefix(base).unwrap().display().to_string();
                        out.push((rel, bytes));
                    }
                }
            }
        }
        walk(&state, &state, &mut entries);
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        format!("{entries:?}").len().to_string() + &format!("{entries:?}")
    }

    /// A project with one imported pack, so scanning writes a pack cache under
    /// whetstone/.state.
    fn project_with_import(td: &Path) {
        let wh = td.join("whetstone");
        let packs = wh.join("packs");
        fs::create_dir_all(&packs).unwrap();
        fs::write(
            packs.join("base.yaml"),
            r#"apiVersion: whetstone/v1alpha1
kind: RulePack
metadata:
  name: acme.base
  scope: project
language: python
rules:
  - id: acme.base-rule
    severity: must
    confidence: high
    category: convention
    description: Base.
    source_url: https://example.com/base
    approved: true
    status: approved
    signals:
      - id: s
        strategy: ast
        description: m
        weight: required
        ast_query: '(pass_statement) @match'
    golden_examples:
      - code: "def f(): pass"
        verdict: fail
        reason: y
"#,
        )
        .unwrap();
        fs::write(
            wh.join("whetstone.yaml"),
            "version: 1\nextends:\n  - scope: project\n    ref: path:./whetstone/packs/base.yaml\n",
        )
        .unwrap();
    }

    #[test]
    fn readonly_preview_leaves_state_byte_identical() {
        let td = tempfile::tempdir().unwrap();
        project_with_import(td.path());

        // Warm the cache with a normal (writing) resolve.
        let _ = resolve_merged(td.path(), Some("python"), true, true, false);
        let before = hash_state_dir(td.path());

        // A read-only preview with an injected candidate must not touch .state.
        let cand = candidate_pack(td.path(), "acme.candidate-rule");
        let opts = SnapshotOptions {
            read_only: true,
            injected_packs: vec![crate::config_packs::resolve_local_pack(&cand).unwrap()],
        };
        let merged = resolve_merged_with(td.path(), Some("python"), true, true, false, &opts);
        assert!(merged.merged.iter().any(|lr| lr.rule.id == "acme.candidate-rule"));
        assert!(merged.merged.iter().any(|lr| lr.rule.id == "acme.base-rule"));

        let after = hash_state_dir(td.path());
        assert_eq!(before, after, "read-only preview mutated whetstone/.state");
    }

    #[test]
    fn injected_candidate_shadows_same_id_and_respects_deny() {
        let td = tempfile::tempdir().unwrap();
        project_with_import(td.path());

        // Candidate redefines the configured id — "later wins" means candidate wins.
        let cand = candidate_pack(td.path(), "acme.base-rule");
        let inject = crate::config_packs::resolve_local_pack(&cand).unwrap();
        let opts = SnapshotOptions {
            read_only: true,
            injected_packs: vec![inject],
        };
        let merged = resolve_merged_with(td.path(), Some("python"), true, true, false, &opts);
        let base = merged
            .merged
            .iter()
            .find(|lr| lr.rule.id == "acme.base-rule")
            .expect("base-rule present");
        assert_eq!(base.rule.description, "Candidate rule.", "candidate should shadow");

        // A project deny excises the injected rule exactly as a real import would.
        let wh = td.path().join("whetstone");
        fs::write(
            wh.join("whetstone.yaml"),
            "version: 1\ndeny:\n  - acme.base-rule\nextends:\n  - scope: project\n    ref: path:./whetstone/packs/base.yaml\n",
        )
        .unwrap();
        let cand2 = candidate_pack(td.path(), "acme.base-rule");
        let opts2 = SnapshotOptions {
            read_only: true,
            injected_packs: vec![crate::config_packs::resolve_local_pack(&cand2).unwrap()],
        };
        let merged2 = resolve_merged_with(td.path(), Some("python"), true, true, false, &opts2);
        assert!(
            !merged2.merged.iter().any(|lr| lr.rule.id == "acme.base-rule"),
            "project deny must excise the injected candidate too"
        );
    }

    #[test]
    fn tagged_merge_marks_candidate_provenance() {
        let td = tempfile::tempdir().unwrap();
        let cand = candidate_pack(td.path(), "acme.candidate-rule");
        let inject = crate::config_packs::resolve_local_pack(&cand).unwrap();
        let (tagged, _w) =
            crate::config_packs::merge_pack_rules_tagged(&[inject], Some("python"));
        let (_rule, origin) = tagged
            .iter()
            .find(|(r, _)| r.id == "acme.candidate-rule")
            .expect("candidate merged");
        assert!(origin.is_candidate(), "origin should be the candidate scope");
    }
}
