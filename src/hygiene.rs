//! Deterministic feature-map hygiene: the half of pstack's maintain pass
//! that needs no judgment. Every finding names the feature and says what to
//! do; nothing here edits the map, the driver or product code.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Serialize;

use crate::agreement::AgreementState;
use crate::domain::{Enforcement, Feature, RecordBody, RecordId};
use crate::projection::SkillManifest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HygieneFinding {
    /// The feature the finding is about, or the skill file for index issues.
    pub feature: String,
    pub kind: &'static str,
    pub detail: String,
    pub next: String,
}

fn finding(feature: &str, kind: &'static str, detail: String, next: String) -> HygieneFinding {
    HygieneFinding {
        feature: feature.to_string(),
        kind,
        detail,
        next,
    }
}

/// Repository files for entry-point matching, bounded and without VCS or
/// build output.
fn repository_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !(entry.depth() > 0
                && entry.file_type().is_dir()
                && matches!(
                    name.as_ref(),
                    ".git" | "target" | "node_modules" | ".beads" | "dist" | "build"
                ))
        })
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            if let Ok(relative) = entry.path().strip_prefix(root) {
                files.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
        if files.len() > 50_000 {
            break;
        }
    }
    files
}

fn resolves(state: &AgreementState, id: &RecordId) -> bool {
    state.in_force(id).is_some() || state.pending(id).is_some()
}

/// Every hygiene finding for the in-force map, plus index mismatches in the
/// rendered skill directories when a manifest says where they are.
pub fn check(
    state: &AgreementState,
    project_root: &Path,
    manifest: Option<&SkillManifest>,
) -> Vec<HygieneFinding> {
    let mut findings = Vec::new();
    let features = state
        .agreement_ids(|body| matches!(body, RecordBody::Feature(_)))
        .into_iter()
        .filter_map(|id| match state.in_force(&id).map(|record| &record.body) {
            Some(RecordBody::Feature(feature)) => Some((id, feature.clone())),
            _ => None,
        })
        .collect::<Vec<(RecordId, Feature)>>();
    let drive_gates = state
        .in_force_matching(|body| matches!(body, RecordBody::Standard(_)))
        .into_iter()
        .filter_map(|record| match &record.body {
            RecordBody::Standard(standard) => match &standard.enforcement {
                Enforcement::Drive { feature } => Some(feature.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let files = repository_files(project_root);
    for (id, feature) in &features {
        let name = id.as_str();
        if feature.drive_steps.is_empty() {
            findings.push(finding(
                name,
                "missing_steps",
                format!(
                    "{} has no executable drive steps, so no sweep can prove it.",
                    feature.name
                ),
                format!("Add drive steps with wh change --kind feature --record-id {name}."),
            ));
        }
        let proven = feature
            .proven_by
            .iter()
            .any(|gate| state.in_force(gate).is_some())
            || drive_gates.contains(id);
        if !proven {
            findings.push(finding(
                name,
                "missing_gate",
                format!("{} has no accepted gate that proves it.", feature.name),
                format!(
                    "Record a drive gate for it with wh change --kind standard and enforcement {{\"enforcement\":\"drive\",\"feature\":\"{name}\"}}, then accept it."
                ),
            ));
        }
        for (relation, links) in [
            ("serves", &feature.serves),
            ("constrained_by", &feature.constrained_by),
            ("proven_by", &feature.proven_by),
        ] {
            for link in links.iter().filter(|link| !resolves(state, link)) {
                findings.push(finding(
                    name,
                    "dangling_link",
                    format!("{relation} names {}, which is not a record in force or in review.", link.as_str()),
                    format!("Fix the link with wh change --kind feature --record-id {name}, or record {}.", link.as_str()),
                ));
            }
        }
        for entry in &feature.entry_points {
            let pattern = entry.trim_start_matches("./");
            if !files
                .iter()
                .any(|file| crate::domain::entry_point_matches(pattern, file))
            {
                findings.push(finding(
                    name,
                    "entry_point_matches_nothing",
                    format!("Entry point {entry} matches no file in the repository."),
                    format!(
                        "Correct the entry point with wh change --kind feature --record-id {name}."
                    ),
                ));
            }
        }
    }
    if let Some(manifest) = manifest {
        let rendered = manifest
            .files
            .iter()
            .filter_map(|file| {
                file.path
                    .split_once("/features/")
                    .map(|(dir, _)| dir.to_string())
            })
            .collect::<BTreeSet<_>>();
        for skill_dir in rendered {
            findings.extend(index_findings(
                state,
                &project_root.join(&skill_dir),
                &skill_dir,
            ));
        }
    }
    findings
}

/// README entries without a file, files without an entry, and frontmatter
/// that disagrees with the accepted record.
fn index_findings(state: &AgreementState, dir: &Path, label: &str) -> Vec<HygieneFinding> {
    let mut findings = Vec::new();
    let features_dir = dir.join("features");
    let Ok(readme) = fs::read_to_string(features_dir.join("README.md")) else {
        return findings;
    };
    let listed = crate::feature_map::parse_readme(&readme)
        .map(|parsed| {
            parsed
                .entries
                .into_iter()
                .map(|entry| entry.0)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let mut present = BTreeMap::new();
    if let Ok(entries) = fs::read_dir(&features_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
                .unwrap_or_default();
            if path.extension().is_some_and(|extension| extension == "md") && stem != "README" {
                present.insert(stem, path);
            }
        }
    }
    for stem in listed.iter().filter(|stem| !present.contains_key(*stem)) {
        findings.push(finding(
            &format!("{label}/features/{stem}.md"),
            "readme_entry_without_file",
            format!("features/README.md lists {stem}.md, which does not exist."),
            "Regenerate the skill with wh init --action wire, or return the edit with wh init --action import.".into(),
        ));
    }
    for (stem, path) in &present {
        if !listed.contains(stem) {
            findings.push(finding(
                &format!("{label}/features/{stem}.md"),
                "file_without_readme_entry",
                format!("{stem}.md is not listed in features/README.md."),
                "List it through its record (wh change --kind feature) and regenerate, or remove the stray file.".into(),
            ));
        }
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let Ok(parsed) = crate::feature_map::parse_feature(stem, &text) else {
            continue;
        };
        let Some(record) = state.in_force(&parsed.id) else {
            continue;
        };
        let RecordBody::Feature(accepted) = &record.body else {
            continue;
        };
        let revision = text
            .lines()
            .find_map(|line| line.strip_prefix("revision: "))
            .and_then(|value| value.trim().parse::<u64>().ok());
        let disagrees = revision.is_some_and(|revision| revision != record.revision)
            || parsed.feature.area != accepted.area
            || parsed.feature.sweep_order != accepted.sweep_order
            || parsed.feature.entry_points != accepted.entry_points
            || parsed.feature.drive_steps != accepted.drive_steps
            || parsed.feature.serves != accepted.serves
            || parsed.feature.proven_by != accepted.proven_by;
        if disagrees {
            findings.push(finding(
                parsed.id.as_str(),
                "frontmatter_disagrees",
                format!(
                    "{label}/features/{stem}.md frontmatter disagrees with {} v{} (stale projection or a hand edit).",
                    parsed.id.as_str(),
                    record.revision
                ),
                "Regenerate with wh init --action wire; if the file was edited on purpose, return it with wh init --action import.".into(),
            ));
        }
    }
    findings
}
