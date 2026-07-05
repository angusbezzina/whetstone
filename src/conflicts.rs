//! Cross-layer rule-conflict detection (whetstone-l05).
//!
//! Importing multiple packs (a starter pack + a resources pack + personal taste)
//! WILL collide: two packs defining the same rule id, a local rule shadowing an
//! imported one, personal shadowing project, or two rules binding the same
//! formatter option to different values. Most of these are silent today (the
//! merge just applies precedence). This exposes them as a stable JSON shape the
//! onboarding CONFLICTS step renders — and agents can read the same way.
//!
//! Accepts injected candidate packs (whetstone-dva) so a conflict a *proposed*
//! selection WOULD introduce is visible before import.

use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use crate::config::{ConfigSnapshot, SnapshotOptions};
use crate::config_packs::{pack_display_name, ResolvedConfigPack};
use crate::layers::{resolve_merged_with, LayerPaths};

// Precedence ranks — higher wins (mirrors the merge: personal > project-local >
// later pack > earlier pack).
const RANK_PERSONAL: i64 = 3_000;
const RANK_PROJECT_LOCAL: i64 = 2_000;
const RANK_PACK_BASE: i64 = 1_000;

pub fn detect(
    project_dir: &Path,
    lang_filter: Option<&str>,
    injected: &[ResolvedConfigPack],
    include_personal: bool,
) -> Value {
    let opts = SnapshotOptions {
        read_only: true,
        injected_packs: injected.to_vec(),
    };
    let mut conflicts: Vec<Value> = Vec::new();

    // ── 1. Same-id collisions across every rule source ──
    // (label, precedence). We gather the RAW per-source ids (pre-dedup) so a
    // collision the merge would silently resolve is still reported.
    let mut by_id: BTreeMap<String, Vec<(String, i64)>> = BTreeMap::new();
    let snap = ConfigSnapshot::load_project_snapshot_with(project_dir, &opts);
    for (i, pack) in snap.active_packs.iter().enumerate() {
        let label = format!("pack:{}", pack_display_name(pack));
        for rule in &pack.pack.rules {
            if !rule.approved {
                continue;
            }
            by_id
                .entry(rule.id.clone())
                .or_default()
                .push((label.clone(), RANK_PACK_BASE + i as i64));
        }
    }
    let paths = LayerPaths::for_project(project_dir);
    let (local, _) = crate::rules::load_approved_rules(&paths.project_rules_dir, lang_filter);
    for r in &local {
        by_id
            .entry(r.id.clone())
            .or_default()
            .push(("project-local".to_string(), RANK_PROJECT_LOCAL));
    }
    if include_personal {
        let (personal, _) = crate::rules::load_approved_rules(&paths.personal_rules_dir, lang_filter);
        for r in &personal {
            by_id
                .entry(r.id.clone())
                .or_default()
                .push(("personal".to_string(), RANK_PERSONAL));
        }
    }
    for (id, sources) in &by_id {
        if sources.len() < 2 {
            continue;
        }
        let winner = sources.iter().max_by_key(|(_, p)| *p).unwrap();
        let losers: Vec<String> = sources
            .iter()
            .filter(|(label, _)| label != &winner.0)
            .map(|(label, _)| label.clone())
            .collect();
        conflicts.push(json!({
            "kind": "same-id",
            "rule_id": id,
            "winner": winner.0,
            "losers": losers,
            "layers": sources.iter().map(|(l, _)| l.clone()).collect::<Vec<_>>(),
            "suggested_resolution": "the winning layer applies; add a `deny` or `override` entry for the others if the shadow is unintended",
        }));
    }

    // ── 2. Formatter-option conflicts across the merged (active) rules ──
    let merged =
        resolve_merged_with(project_dir, lang_filter, true, include_personal, false, &opts).merged;
    let mut fmt: BTreeMap<(String, String), Vec<(String, Value)>> = BTreeMap::new();
    for lr in &merged {
        if let Some(f) = &lr.rule.formatter {
            for (k, v) in &f.options {
                fmt.entry((f.tool.clone(), k.clone()))
                    .or_default()
                    .push((lr.rule.id.clone(), v.clone()));
            }
        }
    }
    for ((tool, option), entries) in &fmt {
        let distinct: BTreeSet<String> = entries.iter().map(|(_, v)| v.to_string()).collect();
        if distinct.len() > 1 {
            conflicts.push(json!({
                "kind": "formatter-option",
                "tool": tool,
                "option": option,
                "values": entries
                    .iter()
                    .map(|(rid, v)| json!({ "rule_id": rid, "value": v }))
                    .collect::<Vec<_>>(),
                "suggested_resolution": "override one rule's formatter option so the generated tool config is consistent",
            }));
        }
    }

    json!({
        "status": "ok",
        "conflicts_count": conflicts.len(),
        "conflicts": conflicts,
    })
}
