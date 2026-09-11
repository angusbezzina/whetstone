//! The feature map in pstack's verification-skill format.
//!
//! pstack (`cursor/plugins/pstack`, skills `create-verification-skill` and
//! `maintain-verification-skill`) fixes the shape: `features/README.md` with
//! baseline preconditions, driving conventions, proof and skip reporting, the
//! feature entry contract and the feature list, and one file per feature with
//! an H1, one paragraph and exactly four H2 sections in order. Whetstone's own
//! fields (record id and revision, why, links, drift, entry points) travel in
//! YAML frontmatter so pstack's maintain pass reads the body unchanged.
//!
//! Rendering and parsing are pure: the same records produce the same bytes,
//! and a pstack-generated map parses into feature records that render back to
//! the original bytes.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::domain::{Feature, RecordId, SkillNote, VerificationMap};

pub const SUB_FEATURES: &str = "Sub-features";
pub const HOW_TO_GET_THERE: &str = "How to get to it (user POV)";
pub const DRIVING_PREFIX: &str = "Driving it with ";
pub const GOTCHAS: &str = "Gotchas";
pub const DEFAULT_HARNESS: &str = "drive.mjs";
pub const DRIVER_COMMAND: &str = "node whetstone/verify/drive.mjs";

/// pstack's entry contract, followed by Whetstone's frontmatter note.
pub const STANDARD_ENTRY_CONTRACT: &str = "Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order.

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with <harness>` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

Keep implementation details out of the map. Name only user paths, stable handles, required state, commands, and observable proof.

Whetstone keeps each feature's record id, revision, why it exists, links, proving gates, drift and code entry points in YAML frontmatter above the H1, never as extra sections. Change a feature with `wh change --kind feature`, never by editing these files.";

/// A feature ready to render: the record plus what the renderer needs to know
/// about its identity and links.
pub struct RenderFeature<'a> {
    pub id: &'a RecordId,
    pub feature: &'a Feature,
    /// Frontmatter lines (already rendered as `key: value`), or none for a
    /// plain pstack body.
    pub frontmatter: Option<String>,
}

pub fn feature_slug(id: &RecordId) -> String {
    id.as_str()
        .trim_start_matches("feature.")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn bullets(out: &mut String, items: &[String], empty: &str) {
    if items.is_empty() {
        let _ = writeln!(out, "- {empty}");
    }
    for item in items {
        let _ = writeln!(out, "- {item}");
    }
}

fn quote_step(step: &str) -> String {
    format!("\"{}\"", step.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The pstack body of one feature file: H1, paragraph, four H2s.
pub fn render_feature_body(id: &RecordId, feature: &Feature) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", feature.name);
    let _ = writeln!(out, "{}\n", feature.summary);
    let _ = writeln!(out, "## {SUB_FEATURES}\n");
    bullets(
        &mut out,
        &feature.sub_features,
        "None broken out yet; the whole feature is one behavior.",
    );
    let _ = writeln!(out, "\n## {HOW_TO_GET_THERE}\n");
    let path = feature.user_path.trim();
    if path
        .lines()
        .all(|line| line.starts_with("- ") || line.starts_with("  "))
    {
        let _ = writeln!(out, "{path}");
    } else {
        let _ = writeln!(out, "- {path}");
    }
    let harness = feature.harness.as_deref().unwrap_or(DEFAULT_HARNESS);
    let _ = writeln!(out, "\n## {DRIVING_PREFIX}{harness}\n");
    let preconditions = if feature.preconditions.is_empty() && feature.drive_recipe.is_empty() {
        vec![format!(
            "`{DRIVER_COMMAND} doctor --json` reports `\"ok\": true` for a fresh build."
        )]
    } else {
        feature.preconditions.clone()
    };
    if !preconditions.is_empty() {
        let _ = writeln!(out, "Preconditions:\n");
        for item in &preconditions {
            let _ = writeln!(out, "- {item}");
        }
        out.push('\n');
    }
    if feature.drive_recipe.is_empty() {
        if feature.drive_steps.is_empty() {
            let _ = writeln!(
                out,
                "- **Drive.** No executable steps are recorded yet. Add them with `wh change --kind feature --record-id {}`; until then the feature cannot be proven.",
                id.as_str()
            );
        } else {
            let _ = writeln!(
                out,
                "- **Drive.** Run `{DRIVER_COMMAND} drive {} --json`. Every step reports `ok` and leaves evidence (a screenshot per step for web, a transcript for command-line and HTTP surfaces).",
                feature
                    .drive_steps
                    .iter()
                    .map(|step| quote_step(step))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        let _ = writeln!(
            out,
            "- **Proof.** {} Run `wh check --feature {}`. The receipt names the evidence files, which survive cleanup.",
            feature.proof.trim(),
            id.as_str()
        );
    } else {
        for item in &feature.drive_recipe {
            let _ = writeln!(out, "- {item}");
        }
    }
    let _ = writeln!(out, "\n## {GOTCHAS}\n");
    bullets(&mut out, &feature.gotchas, "None recorded yet.");
    out
}

/// A complete feature file: optional frontmatter, then the pstack body.
pub fn render_feature_file(item: &RenderFeature<'_>) -> String {
    let mut out = String::new();
    if let Some(frontmatter) = &item.frontmatter {
        let _ = writeln!(out, "---");
        out.push_str(frontmatter);
        if !frontmatter.ends_with('\n') {
            out.push('\n');
        }
        let _ = writeln!(out, "---\n");
    }
    out.push_str(&render_feature_body(item.id, item.feature));
    out
}

/// One `key: value` frontmatter line; values are JSON, which YAML reads.
pub fn frontmatter_line(key: &str, value: &serde_json::Value) -> String {
    format!(
        "{key}: {}\n",
        serde_json::to_string(value).unwrap_or_else(|_| "null".into())
    )
}

/// The default conventions for a Whetstone-generated map.
pub fn default_map(app: &str, surface: &str) -> VerificationMap {
    let launch = format!("Launch {app} with `{DRIVER_COMMAND} launch`; it starts an isolated instance (checkout-derived port and data directory) and waits until it is ready.");
    VerificationMap {
        title: format!("{app} verification map"),
        intro: format!("This directory is the maintained source for verifying the user-facing behavior of {app}. Read the index before driving the app, then use the matching feature file as the recipe. Whetstone generates it from accepted records; to correct a feature, edit its file and return it with `wh init --action import`, or use `wh change`."),
        baseline_preconditions: vec![
            launch,
            format!("Run `{DRIVER_COMMAND} doctor --json` and require `\"ok\": true`; a stale build or unhealthy instance is not worth driving."),
            "Never drive an instance that was not started by this verification run.".into(),
        ],
        driving_conventions: vec![
            "Start every recipe from the baseline state unless its preconditions say otherwise.".into(),
            if surface == "web" {
                "Prefer ids, ARIA roles and accessible names over CSS classes, text over coordinates.".into()
            } else {
                "Treat every command as literal. Keep quoted names and flags unchanged.".into()
            },
            format!("Run actions through `{DRIVER_COMMAND} drive <step> ...`; `{DRIVER_COMMAND} help` lists the steps."),
            format!("Clean up with `{DRIVER_COMMAND} cleanup`; it stops only what the driver started and never removes evidence."),
        ],
        proof_reporting: vec![
            "Capture the user action and the resulting state, not only the final screen.".into(),
            "Verify side effects (files, requests, stored records) alongside what is visible.".into(),
            "`wh check --feature <id>` stores evidence under the run directory it prints; a pass without evidence is unknown.".into(),
            "Report an unreachable path with the attempted command and the unmet precondition.".into(),
            "Do not report a skipped entry point as verified through a different path.".into(),
        ],
        entry_contract: None,
        skill_notes: Vec::new(),
    }
}

/// `features/README.md`: conventions, then the features in sweep order.
pub fn render_readme(map: &VerificationMap, features: &[(&RecordId, &Feature)]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", map.title);
    let _ = writeln!(out, "{}\n", map.intro);
    for (heading, items) in [
        ("Baseline preconditions", &map.baseline_preconditions),
        ("Driving conventions", &map.driving_conventions),
        ("Proof and skip reporting", &map.proof_reporting),
    ] {
        let _ = writeln!(out, "## {heading}\n");
        bullets(&mut out, items, "None recorded yet.");
        out.push('\n');
    }
    let _ = writeln!(out, "## Feature entry contract\n");
    let _ = writeln!(
        out,
        "{}\n",
        map.entry_contract
            .as_deref()
            .unwrap_or(STANDARD_ENTRY_CONTRACT)
            .trim()
    );
    let _ = writeln!(out, "## Features\n");
    if features.is_empty() {
        let _ = writeln!(
            out,
            "No feature is mapped yet. Add one with `wh change --kind feature`."
        );
        return out;
    }
    let mut areas = Vec::<(&str, Vec<&(&RecordId, &Feature)>)>::new();
    for item in features {
        match areas.iter_mut().find(|(area, _)| *area == item.1.area) {
            Some((_, list)) => list.push(item),
            None => areas.push((item.1.area.as_str(), vec![item])),
        }
    }
    let grouped = areas.len() > 1;
    for (index, (area, items)) in areas.iter().enumerate() {
        if grouped {
            if index > 0 {
                out.push('\n');
            }
            let _ = writeln!(out, "### {area}\n");
        }
        for (id, feature) in items {
            let line = feature
                .index_summary
                .clone()
                .unwrap_or_else(|| format!("— {}", feature.summary.trim()));
            let _ = writeln!(
                out,
                "- [{}](./{}.md) {line}",
                feature.name,
                feature_slug(id)
            );
        }
    }
    out
}

// ---------- parsing ----------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Section {
    heading: String,
    body: String,
}

/// Split a Markdown document into its optional frontmatter, H1, the paragraph
/// before the first H2, and H2 sections.
fn split_document(text: &str) -> (Option<String>, Option<String>, String, Vec<Section>) {
    let mut rest = text;
    let mut frontmatter = None;
    if let Some(after) = rest.strip_prefix("---\n") {
        if let Some(end) = after.find("\n---\n") {
            frontmatter = Some(after[..end].to_string());
            rest = after[end + 5..].trim_start_matches('\n');
        }
    }
    // Skip generated HTML comments (stamps) before the H1.
    while let Some(after) = rest.strip_prefix("<!--") {
        match after.find("-->") {
            Some(end) => rest = after[end + 3..].trim_start_matches('\n'),
            None => break,
        }
    }
    let mut title = None;
    let mut lead = Vec::new();
    let mut sections = Vec::<Section>::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in rest.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some((heading, lines)) = current.take() {
                sections.push(Section {
                    heading,
                    body: trim_blank(&lines),
                });
            }
            current = Some((heading.trim().to_string(), Vec::new()));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        } else if title.is_none() && line.starts_with("# ") {
            title = Some(line[2..].trim().to_string());
        } else {
            lead.push(line);
        }
    }
    if let Some((heading, lines)) = current.take() {
        sections.push(Section {
            heading,
            body: trim_blank(&lines),
        });
    }
    (frontmatter, title, trim_blank(&lead), sections)
}

fn trim_blank(lines: &[&str]) -> String {
    let start = lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(lines.len());
    let end = lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map_or(start, |index| index + 1);
    lines[start..end].join("\n")
}

/// List items of a Markdown bullet list; continuation lines stay attached.
fn list_items(body: &str) -> Option<Vec<String>> {
    let mut items = Vec::<String>::new();
    for line in body.lines() {
        if let Some(item) = line.strip_prefix("- ") {
            items.push(item.to_string());
        } else if (line.starts_with("  ") || line.is_empty()) && !items.is_empty() {
            let last = items.last_mut().expect("checked non-empty");
            last.push('\n');
            last.push_str(line);
        } else {
            return None;
        }
    }
    for item in &mut items {
        let trimmed = item.trim_end_matches('\n').to_string();
        *item = trimmed;
    }
    Some(items)
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn frontmatter_map(frontmatter: Option<&str>) -> BTreeMap<String, serde_json::Value> {
    let mut map = BTreeMap::new();
    for line in frontmatter.unwrap_or_default().lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let parsed = serde_json::from_str::<serde_json::Value>(value)
            .unwrap_or_else(|_| serde_json::Value::String(value.trim_matches('"').to_string()));
        map.insert(key.trim().to_string(), parsed);
    }
    map
}

fn string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn id_list(value: Option<&serde_json::Value>) -> Vec<RecordId> {
    string_list(value)
        .into_iter()
        .filter_map(|id| RecordId::new(id).ok())
        .collect()
}

/// A feature file read from a pstack (or Whetstone) verification skill.
#[derive(Debug, Clone)]
pub struct ParsedFeature {
    pub id: RecordId,
    pub feature: Feature,
    /// Sections that were not pstack's four; kept as gotchas, never dropped.
    pub preserved: Vec<String>,
}

/// Parse one feature file. `stem` is the file name without `.md`.
pub fn parse_feature(stem: &str, text: &str) -> Result<ParsedFeature, String> {
    let (frontmatter, title, lead, sections) = split_document(text);
    let meta = frontmatter_map(frontmatter.as_deref());
    let id = meta
        .get("record")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            format!(
                "feature.{}",
                stem.chars()
                    .map(
                        |character| if character.is_ascii_alphanumeric() || character == '-' {
                            character.to_ascii_lowercase()
                        } else {
                            '-'
                        }
                    )
                    .collect::<String>()
            )
        });
    let id = RecordId::new(id).map_err(|error| format!("{stem}: {error}"))?;
    let name = title.ok_or_else(|| format!("{stem}.md has no H1 title"))?;
    let mut feature = Feature {
        name: name.clone(),
        summary: if lead.trim().is_empty() { name } else { lead },
        area: meta
            .get("area")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Features")
            .to_string(),
        sweep_order: meta
            .get("sweep_order")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0),
        sub_features: Vec::new(),
        user_path: String::new(),
        drive_steps: string_list(meta.get("drive_steps")),
        proof: String::new(),
        gotchas: Vec::new(),
        entry_points: string_list(meta.get("entry_points")),
        serves: id_list(meta.get("serves")),
        constrained_by: id_list(meta.get("constrained_by")),
        proven_by: id_list(meta.get("proven_by")),
        index_summary: None,
        harness: None,
        preconditions: Vec::new(),
        drive_recipe: Vec::new(),
    };
    let mut preserved = Vec::new();
    let mut seen = Vec::new();
    for section in &sections {
        let heading = section.heading.as_str();
        let known = if heading == SUB_FEATURES {
            match list_items(&section.body) {
                Some(items) => {
                    feature.sub_features = items;
                    true
                }
                None => false,
            }
        } else if heading == HOW_TO_GET_THERE {
            feature.user_path = section.body.clone();
            true
        } else if let Some(harness) = heading.strip_prefix(DRIVING_PREFIX) {
            feature.harness = Some(harness.trim().to_string());
            parse_driving(&section.body, &mut feature)
        } else if heading == GOTCHAS {
            match list_items(&section.body) {
                Some(items) => {
                    feature.gotchas.extend(items);
                    true
                }
                None => false,
            }
        } else {
            false
        };
        if known {
            seen.push(heading.to_string());
        } else {
            preserved.push(format!(
                "Imported section \u{201c}{heading}\u{201d} kept verbatim: {}",
                one_line(&section.body)
            ));
        }
    }
    feature.gotchas.extend(preserved.iter().cloned());
    if feature.user_path.trim().is_empty() {
        feature.user_path = "Not stated in the imported map.".into();
    }
    if feature.proof.trim().is_empty() {
        feature.proof =
            "Not stated in the imported map; the recipe's final observable state is the proof."
                .into();
    }
    // A Whetstone-generated file renders defaults (harness, recipe, empty
    // placeholders) from a native record; read it back into that shape.
    if meta.contains_key("record") {
        if feature.harness.as_deref() == Some(DEFAULT_HARNESS)
            && feature
                .drive_recipe
                .iter()
                .any(|item| item.starts_with("**Drive.** "))
        {
            if let Some(proof) = feature
                .proof
                .split(" Run `wh check --feature ")
                .next()
                .filter(|proof| !proof.trim().is_empty())
            {
                feature.proof = proof.trim().to_string();
            }
            feature.harness = None;
            feature.drive_recipe.clear();
            feature.preconditions.clear();
        }
        feature
            .sub_features
            .retain(|item| !item.starts_with("None broken out yet"));
        feature.gotchas.retain(|item| item != "None recorded yet.");
        if let Some(single) = feature
            .user_path
            .strip_prefix("- ")
            .filter(|rest| !rest.contains('\n'))
        {
            feature.user_path = single.to_string();
        }
    }
    let _ = seen;
    Ok(ParsedFeature {
        id,
        feature,
        preserved,
    })
}

fn parse_driving(body: &str, feature: &mut Feature) -> bool {
    let mut rest = body;
    if let Some(after) = rest.strip_prefix("Preconditions:") {
        let after = after.trim_start_matches('\n');
        let (list, tail) = match after.find("\n\n") {
            Some(split) => (&after[..split], after[split + 2..].trim_start_matches('\n')),
            None => (after, ""),
        };
        match list_items(list) {
            Some(items) => feature.preconditions = items,
            None => return false,
        }
        rest = tail;
    }
    if rest.trim().is_empty() {
        return true;
    }
    match list_items(rest) {
        Some(items) => {
            if let Some(proof) = items
                .iter()
                .find_map(|item| item.strip_prefix("**Proof.** "))
            {
                feature.proof = one_line(proof);
            }
            feature.drive_recipe = items;
            true
        }
        None => false,
    }
}

/// `features/README.md` read back into map conventions and the feature order.
#[derive(Debug, Clone)]
pub struct ParsedReadme {
    pub map: VerificationMap,
    /// (file stem, link text, text after the link), in sweep order.
    pub entries: Vec<(String, String, String, Option<String>)>,
    pub preserved: Vec<String>,
}

pub fn parse_readme(text: &str) -> Result<ParsedReadme, String> {
    let (_, title, intro, sections) = split_document(text);
    let mut map = VerificationMap {
        title: title.ok_or("features/README.md has no H1 title")?,
        intro: if intro.trim().is_empty() {
            "Imported verification map.".into()
        } else {
            intro
        },
        baseline_preconditions: Vec::new(),
        driving_conventions: Vec::new(),
        proof_reporting: Vec::new(),
        entry_contract: None,
        skill_notes: Vec::new(),
    };
    let mut entries = Vec::new();
    let mut preserved = Vec::new();
    for section in sections {
        let target = match section.heading.as_str() {
            "Baseline preconditions" => Some(&mut map.baseline_preconditions),
            "Driving conventions" => Some(&mut map.driving_conventions),
            "Proof and skip reporting" => Some(&mut map.proof_reporting),
            _ => None,
        };
        if let Some(target) = target {
            match list_items(&section.body) {
                Some(items) => *target = items,
                None => preserved.push(format!("{}: {}", section.heading, one_line(&section.body))),
            }
            continue;
        }
        match section.heading.as_str() {
            "Feature entry contract" => map.entry_contract = Some(section.body.clone()),
            "Features" => {
                let mut area = None;
                for line in section.body.lines() {
                    if let Some(heading) = line.strip_prefix("### ") {
                        area = Some(heading.trim().to_string());
                        continue;
                    }
                    let Some(item) = line.strip_prefix("- [") else {
                        continue;
                    };
                    let Some((label, rest)) = item.split_once("](") else {
                        continue;
                    };
                    let Some((target, tail)) = rest.split_once(')') else {
                        continue;
                    };
                    let stem = target
                        .trim_start_matches("./")
                        .trim_end_matches(".md")
                        .to_string();
                    entries.push((
                        stem,
                        label.to_string(),
                        tail.trim_start().to_string(),
                        area.clone(),
                    ));
                }
            }
            other => preserved.push(format!("{other}: {}", one_line(&section.body))),
        }
    }
    for note in &preserved {
        map.skill_notes.push(SkillNote {
            section: "features/README.md".into(),
            text: note.clone(),
        });
    }
    Ok(ParsedReadme {
        map,
        entries,
        preserved,
    })
}

/// SKILL.md sections (Launch, Doctor, Drive, Evidence, Cleanup, Helpers and
/// any other) kept verbatim as map notes.
pub fn parse_skill(text: &str) -> (Option<String>, Vec<SkillNote>) {
    let (frontmatter, _, _, sections) = split_document(text);
    let name = frontmatter_map(frontmatter.as_deref())
        .get("name")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let notes = sections
        .into_iter()
        .filter(|section| !section.body.trim().is_empty())
        .take(16)
        .map(|section| SkillNote {
            section: section.heading,
            text: section.body,
        })
        .collect();
    (name, notes)
}

/// The `## Imported notes` section of a Whetstone-generated SKILL.md, read
/// back into the notes it was rendered from (one `###` per note).
pub fn parse_imported_notes(text: &str) -> Vec<SkillNote> {
    let (_, _, _, sections) = split_document(text);
    let Some(section) = sections
        .into_iter()
        .find(|section| section.heading == "Imported notes")
    else {
        return Vec::new();
    };
    let mut notes = Vec::new();
    let mut current: Option<(String, Vec<&str>)> = None;
    for line in section.body.lines() {
        if let Some(heading) = line.strip_prefix("### ") {
            if let Some((heading, lines)) = current.take() {
                notes.push(SkillNote {
                    section: heading,
                    text: trim_blank(&lines),
                });
            }
            current = Some((heading.trim().to_string(), Vec::new()));
        } else if let Some((_, lines)) = current.as_mut() {
            lines.push(line);
        }
    }
    if let Some((heading, lines)) = current {
        notes.push(SkillNote {
            section: heading,
            text: trim_blank(&lines),
        });
    }
    notes
}

/// Every H2 of a rendered feature file, in order; the conformance check.
pub fn h2_headings(text: &str) -> Vec<String> {
    let (_, _, _, sections) = split_document(text);
    sections
        .into_iter()
        .map(|section| section.heading)
        .collect()
}

/// Whether a feature file has exactly pstack's four H2s in order.
pub fn conforms(text: &str) -> Result<(), String> {
    let headings = h2_headings(text);
    let ok = headings.len() == 4
        && headings[0] == SUB_FEATURES
        && headings[1] == HOW_TO_GET_THERE
        && headings[2].starts_with(DRIVING_PREFIX)
        && headings[2].len() > DRIVING_PREFIX.len()
        && headings[3] == GOTCHAS;
    if ok {
        Ok(())
    } else {
        Err(format!(
            "expected exactly `{SUB_FEATURES}`, `{HOW_TO_GET_THERE}`, `{DRIVING_PREFIX}<harness>`, `{GOTCHAS}`; found {headings:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_README: &str =
        include_str!("../tests/fixtures/pstack-feature-map-example/README.md");
    const EXAMPLE_CREATE: &str =
        include_str!("../tests/fixtures/pstack-feature-map-example/create-note.md");
    const EXAMPLE_SEARCH: &str =
        include_str!("../tests/fixtures/pstack-feature-map-example/search.md");

    fn roundtrip(stem: &str, text: &str) {
        let parsed = parse_feature(stem, text).expect("parse");
        assert!(parsed.preserved.is_empty(), "{:?}", parsed.preserved);
        crate::domain::RecordBody::Feature(parsed.feature.clone())
            .validate()
            .expect("imported feature is a valid record");
        let rendered = render_feature_body(&parsed.id, &parsed.feature);
        assert_eq!(rendered, text, "{stem} did not re-render byte-identically");
        conforms(&rendered).expect("four H2s");
    }

    #[test]
    fn pstack_feature_files_round_trip_byte_identically() {
        roundtrip("create-note", EXAMPLE_CREATE);
        roundtrip("search", EXAMPLE_SEARCH);
    }

    #[test]
    fn pstack_readme_round_trips_byte_identically() {
        let readme = parse_readme(EXAMPLE_README).expect("readme");
        assert!(readme.preserved.is_empty());
        let mut features = Vec::new();
        for (stem, text) in [("create-note", EXAMPLE_CREATE), ("search", EXAMPLE_SEARCH)] {
            let mut parsed = parse_feature(stem, text).expect("feature");
            let entry = readme
                .entries
                .iter()
                .find(|entry| entry.0 == stem)
                .expect("listed");
            parsed.feature.index_summary = Some(entry.2.clone());
            features.push(parsed);
        }
        let refs = features
            .iter()
            .map(|parsed| (&parsed.id, &parsed.feature))
            .collect::<Vec<_>>();
        assert_eq!(render_readme(&readme.map, &refs), EXAMPLE_README);
    }

    #[test]
    fn the_conformance_check_rejects_extra_or_reordered_sections() {
        conforms(EXAMPLE_CREATE).expect("example conforms");
        let extra = EXAMPLE_CREATE.replace("## Gotchas", "## Why\n\nBecause.\n\n## Gotchas");
        assert!(conforms(&extra).is_err());
        let reordered = EXAMPLE_CREATE
            .replace("## Sub-features", "## TMP")
            .replace("## Gotchas", "## Sub-features")
            .replace("## TMP", "## Gotchas");
        assert!(conforms(&reordered).is_err());
        let unnamed =
            EXAMPLE_CREATE.replace("## Driving it with control-notes", "## Driving it with ");
        assert!(conforms(&unnamed).is_err());
    }

    #[test]
    fn unknown_sections_are_kept_as_gotchas_never_dropped() {
        let text = EXAMPLE_SEARCH.replace(
            "## Gotchas",
            "## Accessibility\n\nThe dialog traps focus.\nEscape closes it.\n\n## Gotchas",
        );
        let parsed = parse_feature("search", &text).expect("parse");
        assert_eq!(parsed.preserved.len(), 1);
        assert!(parsed.feature.gotchas.last().expect("gotcha").contains(
            "Accessibility\u{201d} kept verbatim: The dialog traps focus. Escape closes it."
        ));
    }

    #[test]
    fn generated_features_conform_and_reimport_to_the_same_record() {
        let id = RecordId::new("feature.dashboard-home").expect("id");
        let feature = Feature {
            name: "Dashboard home".into(),
            summary: "The default view shows the mission and what needs attention.".into(),
            area: "Dashboard".into(),
            sweep_order: 1,
            sub_features: vec!["`home-mission` shows the mission headline.".into()],
            user_path: "Run `wh dash` and read the default view.".into(),
            drive_steps: vec!["open /".into(), "expect text=Mission".into()],
            proof: "The mission headline renders.".into(),
            gotchas: Vec::new(),
            entry_points: vec!["assets/dashboard/".into()],
            serves: vec![RecordId::new("mission.project").expect("id")],
            constrained_by: Vec::new(),
            proven_by: vec![RecordId::new("standard.dashboard-home").expect("id")],
            index_summary: None,
            harness: None,
            preconditions: Vec::new(),
            drive_recipe: Vec::new(),
        };
        let mut frontmatter = String::new();
        frontmatter.push_str(&frontmatter_line("record", &serde_json::json!(id.as_str())));
        frontmatter.push_str(&frontmatter_line("area", &serde_json::json!(feature.area)));
        frontmatter.push_str(&frontmatter_line("sweep_order", &serde_json::json!(1)));
        frontmatter.push_str(&frontmatter_line(
            "drive_steps",
            &serde_json::json!(feature.drive_steps),
        ));
        frontmatter.push_str(&frontmatter_line(
            "entry_points",
            &serde_json::json!(feature.entry_points),
        ));
        frontmatter.push_str(&frontmatter_line(
            "serves",
            &serde_json::json!(["mission.project"]),
        ));
        frontmatter.push_str(&frontmatter_line(
            "proven_by",
            &serde_json::json!(["standard.dashboard-home"]),
        ));
        let text = render_feature_file(&RenderFeature {
            id: &id,
            feature: &feature,
            frontmatter: Some(frontmatter),
        });
        conforms(&text).expect("conforms");
        assert!(text.starts_with("---\nrecord: \"feature.dashboard-home\"\n"));
        let parsed = parse_feature("dashboard-home", &text).expect("reimport");
        assert_eq!(parsed.id, id);
        assert_eq!(parsed.feature.drive_steps, feature.drive_steps);
        assert_eq!(parsed.feature.serves, feature.serves);
        assert_eq!(parsed.feature.harness, None);
        assert!(parsed.feature.drive_recipe.is_empty());
        assert_eq!(parsed.feature, feature);
    }
}
