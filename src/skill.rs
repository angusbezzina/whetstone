//! Agent-facing projection of the agreement: the verification skill.
//!
//! `wh init --action wire` renders, from accepted records only:
//!
//! * `SKILL.md` — why the project exists, how to launch, doctor, drive and
//!   prove it, the proof bar, the gates, and the red-flag catalog;
//! * `features/README.md` and one file per feature — the agent runbook with
//!   the four fixed headings plus Why;
//! * `JOURNAL.md` — the versioned decision log behind the current agreement.
//!
//! The same files are written byte-identically into every selected agent host
//! directory, stamped with the agreement digest. The driver script and its
//! configuration are scaffolded once from a repository interview and then
//! belong to the team; they are never overwritten unless asked.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::agreement::AgreementState;
use crate::domain::{Enforcement, Feature, RecordBody, RecordId};
use crate::projection::{self, SkillFile, SkillManifest};
use crate::service::{InitRequest, ServiceResponse, ServiceState};
use crate::storage::{DoltRepository, ProjectLayout, StoreKind};

pub const MANIFEST_FILE: &str = "skill.json";
pub const DRIVER_TEMPLATE: &str = include_str!("../assets/verify/drive.mjs");
pub const DRIVER_CONFIG_RELATIVE: &str = "whetstone/verify/driver.json";
pub const HOSTS: [(&str, &str); 3] = [
    ("claude", ".claude/skills"),
    ("cursor", ".cursor/skills"),
    ("agents", ".agents/skills"),
];

pub fn manifest_path(layout: &ProjectLayout) -> PathBuf {
    layout.projections_path().join(MANIFEST_FILE)
}

pub fn read_manifest(layout: &ProjectLayout) -> Option<SkillManifest> {
    let bytes = fs::read(manifest_path(layout)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub fn app_slug(root: &Path) -> String {
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "app".into());
    let slug = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if slug.is_empty() {
        "app".into()
    } else {
        slug
    }
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

/// What the repository interview found for the driver configuration.
pub fn interview(root: &Path) -> serde_json::Value {
    let exists = |path: &str| root.join(path).exists();
    let package = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let script = |name: &str| {
        package
            .as_ref()
            .and_then(|value| value.pointer(&format!("/scripts/{name}")))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    };
    let runner = if exists("pnpm-lock.yaml") {
        "pnpm"
    } else if exists("yarn.lock") {
        "yarn"
    } else if exists("bun.lockb") || exists("bun.lock") {
        "bun"
    } else {
        "npm"
    };
    if let Some(name) = ["dev", "start", "preview"]
        .into_iter()
        .find(|name| script(name).is_some())
    {
        return json!({
            "surface": "web",
            "launch": {
                "command": [runner, "run", name],
                "ready": "(https?://(?:localhost|127\\.0\\.0\\.1|\\[::1\\]|0\\.0\\.0\\.0):\\d+[^\\s]*)",
                "timeout_seconds": 120
            },
            "viewport": [1280, 900],
            "interview": format!("package.json script '{name}' looks like the local web app; confirm the ready pattern and port."),
        });
    }
    if exists("Cargo.toml") {
        return json!({
            "surface": "cli",
            "launch": null,
            "interview": "Cargo.toml without a web dev script: the primary surface looks like a command-line tool. Drive it with run <argv> steps, or switch to web with a launch command.",
        });
    }
    if exists("pyproject.toml") || exists("setup.py") {
        return json!({
            "surface": "cli",
            "launch": null,
            "interview": "A Python project: drive it with run <argv> steps, or configure a web or http launch command.",
        });
    }
    json!({
        "surface": "cli",
        "launch": null,
        "interview": "No known app surface was detected. Configure surface and launch in driver.json.",
    })
}

struct Rendered {
    relative: String,
    contents: String,
}

fn stamp(agreement_digest: &str, rendered_at: &str) -> String {
    format!(
        "<!-- Generated by Whetstone from agreement {agreement_digest} at {rendered_at}. Do not edit: change records with `wh change`, then run `wh init --action wire`. -->\n"
    )
}

fn in_force_body<'a>(state: &'a AgreementState, id: &str) -> Option<&'a RecordBody> {
    RecordId::new(id)
        .ok()
        .and_then(|id| state.in_force(&id))
        .map(|record| &record.body)
}

fn title_of(state: &AgreementState, id: &RecordId) -> String {
    state
        .in_force(id)
        .map_or_else(|| id.as_str().to_string(), projection::record_title)
}

fn features(state: &AgreementState) -> Vec<(RecordId, Feature)> {
    let mut result = state
        .agreement_ids(|body| matches!(body, RecordBody::Feature(_)))
        .into_iter()
        .filter_map(|id| match state.in_force(&id).map(|record| &record.body) {
            Some(RecordBody::Feature(feature)) => Some((id, feature.clone())),
            _ => None,
        })
        .collect::<Vec<_>>();
    result.sort_by(|left, right| {
        (
            left.1.area.as_str(),
            left.1.sweep_order,
            left.1.name.as_str(),
        )
            .cmp(&(
                right.1.area.as_str(),
                right.1.sweep_order,
                right.1.name.as_str(),
            ))
    });
    result
}

fn skill_markdown(
    state: &AgreementState,
    app: &str,
    config: &serde_json::Value,
    header: &str,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "name: verify-{app}");
    let _ = writeln!(
        out,
        "description: \"Launch, drive and prove {app} the way a user does, and know why each feature exists. Use before claiming a change works, to reproduce a report, or to understand the project's intent. Generated by Whetstone from the accepted agreement.\""
    );
    let _ = writeln!(out, "---\n");
    out.push_str(header);
    let _ = writeln!(out, "\n# Verify {app}\n");
    let _ = writeln!(
        out,
        "This skill is a projection of the project's accepted agreement. It tells you why the project exists, how to reach and drive every mapped feature, and what evidence proves it works. Read the one feature file for the change under test, not the whole map.\n"
    );
    let _ = writeln!(out, "## Why this project exists\n");
    if let Some(RecordBody::Mission(mission)) = in_force_body(state, projection::MISSION_ID) {
        let _ = writeln!(out, "**Mission.** {}\n", mission.statement);
        for outcome in &mission.desired_outcomes {
            let _ = writeln!(out, "- Outcome: {outcome}");
        }
        if !mission.desired_outcomes.is_empty() {
            out.push('\n');
        }
    }
    let values = state.in_force_matching(|body| matches!(body, RecordBody::CoreValue(_)));
    if !values.is_empty() {
        let _ = writeln!(
            out,
            "**Core values** decide between options when goals conflict:\n"
        );
        for record in values {
            let _ = writeln!(out, "- {}", projection::record_title(record));
        }
        out.push('\n');
    }
    let metrics = state.in_force_matching(|body| matches!(body, RecordBody::MetricDefinition(_)));
    if !metrics.is_empty() {
        let _ = writeln!(out, "**Key metrics** say whether the mission is working:\n");
        for record in metrics {
            if let RecordBody::MetricDefinition(metric) = &record.body {
                let _ = writeln!(
                    out,
                    "- {}: {} to {} over {} (source `{}:{}`)",
                    metric.name,
                    format!("{:?}", metric.direction).to_ascii_lowercase(),
                    metric.threshold,
                    metric.window,
                    metric.source.system,
                    metric.source.locator
                );
            }
        }
        out.push('\n');
    }
    let _ = writeln!(out, "## Launch\n");
    let surface = config
        .get("surface")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("cli");
    let _ = writeln!(
        out,
        "The driver owns launch and teardown for the `{surface}` surface; its configuration is `whetstone/verify/driver.json`.\n\n```bash\nnode whetstone/verify/drive.mjs launch --dry-run   # what would start\nnode whetstone/verify/drive.mjs launch             # start and wait until ready\n```\n"
    );
    let _ = writeln!(out, "## Doctor\n");
    let _ = writeln!(
        out,
        "Run this first, and again whenever anything looks off. A drive against a stale build or an unhealthy instance is not evidence.\n\n```bash\nnode whetstone/verify/drive.mjs doctor --json\n```\n"
    );
    let _ = writeln!(out, "## Drive\n");
    let _ = writeln!(
        out,
        "Steps run in one session with evidence after each. Prefer stable handles (ids, ARIA labels, data attributes) over text, and text over coordinates.\n\n```bash\nnode whetstone/verify/drive.mjs step \"open /\" \"expect text=<something visible>\" --json\nnode whetstone/verify/drive.mjs help\n```\n"
    );
    let _ = writeln!(out, "## Evidence\n");
    let _ = writeln!(
        out,
        "`wh check` runs every gate and stores evidence (logs, screenshots, transcripts) in the private evidence directory it prints; cleanup never deletes it. A proof must:\n\n- exercise the production user path, not internal setters or test-only endpoints;\n- cover every reachable entry point the feature file lists, and the success, cancel, error, empty and persistence paths the change can affect;\n- show the action and the resulting state, and verify side effects (files, requests, stored records), not just pixels;\n- use mocks only behind a production boundary that already isolates the external system.\n\nAn unreachable path is reported with its concrete prerequisite (account, entitlement, OS), never skipped silently.\n"
    );
    let _ = writeln!(out, "## Cleanup\n");
    let _ = writeln!(
        out,
        "```bash\nnode whetstone/verify/drive.mjs cleanup   # stops only what the driver started\n```\n"
    );
    let _ = writeln!(out, "## Gates\n");
    let gates = state.in_force_matching(|body| matches!(body, RecordBody::Standard(_)));
    if gates.is_empty() {
        let _ = writeln!(out, "No gate is in force yet.\n");
    } else {
        let _ = writeln!(
            out,
            "These deterministic gates are in force. Never weaken or skip one to make it pass; if a fix needs a policy change, stop and hand back to the owner.\n"
        );
        for record in gates {
            if let RecordBody::Standard(standard) = &record.body {
                let (mechanism, command) = projection::mechanism_label(standard);
                let _ = writeln!(
                    out,
                    "- **{}** ({}, {mechanism}): `{command}` — recheck with `wh check --rule {}`",
                    standard.statement,
                    projection::strength_label(standard.strength),
                    record.id.as_str()
                );
            }
        }
        out.push('\n');
    }
    let rules = state.in_force_matching(|body| {
        matches!(
            body,
            RecordBody::ImplementationPhilosophy(_) | RecordBody::Guidance(_)
        )
    });
    if !rules.is_empty() {
        let _ = writeln!(out, "## Red flags\n");
        let _ = writeln!(
            out,
            "Screen designs and diffs against these before you build. They are judgment, not deterministic checks; a reviewer decides.\n"
        );
        for record in rules {
            let _ = writeln!(out, "- {}", projection::record_title(record));
        }
        out.push('\n');
    }
    let _ = writeln!(out, "## Feature map\n");
    let _ = writeln!(
        out,
        "[`features/README.md`](features/README.md) lists every mapped feature in sweep order. Each file answers: what exists, how a user reaches it, how to drive it, what usually lies, and why it exists.\n\n- Prove one feature: `wh check --feature <feature id>`\n- Prove what your change touched: `wh check --changed`\n- Broad regression: `wh check --sweep`\n\nWhen behaviour moves, update the feature in the same change: `wh change --kind feature --record-id <id> ...` records a private draft the owner accepts. The decision history is in [`JOURNAL.md`](JOURNAL.md).\n"
    );
    out
}

fn features_readme(state: &AgreementState, header: &str) -> String {
    let mut out = String::new();
    out.push_str(header);
    let _ = writeln!(out, "\n# Feature map\n");
    let _ = writeln!(
        out,
        "Behaviour-level inventory. Agents use it to decide what to drive and what evidence counts; people use it as the regression checklist. Walk it top to bottom for a broad sweep (`wh check --sweep`).\n"
    );
    let _ = writeln!(out, "## Entry contract\n");
    let _ = writeln!(
        out,
        "Every feature file uses the same headings: `Why`, `Sub-features`, `How to get to it (user POV)`, `Driving it`, `Proof`, `Gotchas`.\n"
    );
    let mut areas = BTreeMap::<String, Vec<(RecordId, Feature)>>::new();
    for (id, feature) in features(state) {
        areas
            .entry(feature.area.clone())
            .or_default()
            .push((id, feature));
    }
    if areas.is_empty() {
        let _ = writeln!(
            out,
            "No feature is mapped yet. Add one with `wh change --kind feature`.\n"
        );
    }
    for (area, items) in areas {
        let _ = writeln!(out, "## {area}\n");
        for (id, feature) in items {
            let _ = writeln!(
                out,
                "- [{}]({}.md): {}",
                feature.name,
                feature_slug(&id),
                feature.summary
            );
        }
        out.push('\n');
    }
    out
}

fn feature_markdown(
    state: &AgreementState,
    id: &RecordId,
    feature: &Feature,
    header: &str,
) -> String {
    let mut out = String::new();
    out.push_str(header);
    let _ = writeln!(out, "\n# {}\n", feature.name);
    let _ = writeln!(out, "{}\n", feature.summary);
    let _ = writeln!(out, "## Why\n");
    if feature.serves.is_empty() && feature.constrained_by.is_empty() {
        let _ = writeln!(
            out,
            "No outcome is linked yet; ask the owner why this feature exists.\n"
        );
    }
    for link in &feature.serves {
        let _ = writeln!(out, "- Serves: {}", title_of(state, link));
    }
    for link in &feature.constrained_by {
        let _ = writeln!(out, "- Constrained by: {}", title_of(state, link));
    }
    if !feature.serves.is_empty() || !feature.constrained_by.is_empty() {
        out.push('\n');
    }
    let _ = writeln!(out, "## Sub-features\n");
    if feature.sub_features.is_empty() {
        let _ = writeln!(out, "- (none recorded)");
    }
    for item in &feature.sub_features {
        let _ = writeln!(out, "- {item}");
    }
    let _ = writeln!(out, "\n## How to get to it (user POV)\n");
    let _ = writeln!(out, "{}\n", feature.user_path);
    let _ = writeln!(out, "## Driving it\n");
    if feature.drive_steps.is_empty() {
        let _ = writeln!(out, "No drive steps are recorded yet.\n");
    } else {
        let _ = writeln!(out, "```bash");
        let quoted = feature
            .drive_steps
            .iter()
            .map(|step| format!("\"{}\"", step.replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(out, "node whetstone/verify/drive.mjs step {quoted} --json");
        let _ = writeln!(out, "wh check --feature {}", id.as_str());
        let _ = writeln!(out, "```\n");
    }
    let _ = writeln!(out, "## Proof\n");
    let _ = writeln!(out, "{}\n", feature.proof);
    if !feature.proven_by.is_empty() {
        for gate in &feature.proven_by {
            let _ = writeln!(
                out,
                "- Gate: {} (`{}`)",
                title_of(state, gate),
                gate.as_str()
            );
        }
        out.push('\n');
    }
    let _ = writeln!(out, "## Gotchas\n");
    if feature.gotchas.is_empty() {
        let _ = writeln!(out, "- (none recorded)");
    }
    for item in &feature.gotchas {
        let _ = writeln!(out, "- {item}");
    }
    if !feature.entry_points.is_empty() {
        let _ = writeln!(out, "\n## Code entry points\n");
        let _ = writeln!(
            out,
            "Changes under these paths select this feature for `wh check --changed`:\n"
        );
        for entry in &feature.entry_points {
            let _ = writeln!(out, "- `{entry}`");
        }
    }
    out
}

fn journal_markdown(state: &AgreementState, header: &str) -> String {
    let mut out = String::new();
    out.push_str(header);
    let _ = writeln!(out, "\n# Journal\n");
    let _ = writeln!(
        out,
        "How the agreement evolved, newest first. Only accepted revisions and their superseded predecessors are listed; private drafts are not.\n"
    );
    let mut entries = state
        .records()
        .iter()
        .filter(|record| crate::agreement::is_agreement_body(&record.body))
        .filter(|record| state.lifecycle_of(record) == crate::agreement::Lifecycle::Accepted)
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .provenance
            .recorded_at
            .cmp(&left.provenance.recorded_at)
            .then_with(|| left.id.cmp(&right.id))
    });
    for record in entries.into_iter().take(200) {
        let kind = match &record.body {
            RecordBody::Mission(_) => "mission",
            RecordBody::CoreValue(_) => "value",
            RecordBody::ImplementationPhilosophy(_) => "philosophy",
            RecordBody::Guidance(_) => "guidance",
            RecordBody::MetricDefinition(_) => "metric",
            RecordBody::Standard(_) => "standard",
            RecordBody::Feature(_) => "feature",
            _ => "record",
        };
        let status = if state.is_superseded(record) {
            " (superseded)"
        } else {
            ""
        };
        let why = state
            .proposal_for(record)
            .and_then(|proposal| match &proposal.body {
                RecordBody::Proposal(body) => body
                    .rationale
                    .lines()
                    .find_map(|line| line.strip_prefix("Rationale: "))
                    .map(str::to_owned),
                _ => None,
            });
        let _ = writeln!(
            out,
            "- {} · {} v{}{status}: {}{}",
            record
                .provenance
                .recorded_at
                .get(..10)
                .unwrap_or(&record.provenance.recorded_at),
            projection::kind_label(kind),
            record.revision,
            projection::record_title(record),
            why.map(|why| format!(" — {why}")).unwrap_or_default()
        );
    }
    out
}

fn reject_symlinked_ancestors(root: &Path, relative: &str) -> Result<(), String> {
    let mut current = root.to_path_buf();
    for component in Path::new(relative).components() {
        current.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "{} is a symlink; refusing to write through it.",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

fn write_file(root: &Path, relative: &str, contents: &[u8]) -> Result<(), String> {
    reject_symlinked_ancestors(root, relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let staging = path.with_extension("whetstone-staging");
    fs::write(&staging, contents).map_err(|error| error.to_string())?;
    fs::rename(&staging, &path).map_err(|error| error.to_string())
}

fn error_response(request_id: String, state: ServiceState, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new_public(request_id, "init", state, summary);
    response.permitted_actions = vec!["wh init".into()];
    response
}

/// Generate the verification skill, feature map and journal from accepted
/// records, and scaffold the team-owned driver when absent.
pub fn wire(layout: &ProjectLayout, request_id: String, request: &InitRequest) -> ServiceResponse {
    let store = layout.store_path(StoreKind::Private);
    if !store.join(".dolt").is_dir() {
        return error_response(
            request_id,
            ServiceState::NeedsInput,
            "No private agreement exists yet. Establish it with wh init --action agree, then wire."
                .into(),
        );
    }
    let records = match DoltRepository::open_existing(&store, StoreKind::Private)
        .and_then(|repository| repository.all_records())
    {
        Ok(records) => records,
        Err(error) => {
            return error_response(
                request_id,
                ServiceState::Unknown,
                format!("The private agreement could not be read: {error}"),
            )
        }
    };
    let state = AgreementState::from_records(records);
    if state
        .in_force(&RecordId::new(projection::MISSION_ID).expect("constant"))
        .is_none()
    {
        return error_response(
            request_id,
            ServiceState::NeedsInput,
            "The mission is not in force yet; the skill would have no purpose to explain.".into(),
        );
    }
    let root = layout.project_root();
    let mut hosts = Vec::new();
    for host in &request.hosts {
        match HOSTS.iter().find(|(name, _)| name == host) {
            Some(entry) => {
                if !hosts.contains(entry) {
                    hosts.push(*entry);
                }
            }
            None => {
                return error_response(
                    request_id,
                    ServiceState::NeedsInput,
                    format!("Unknown agent host {host}; choose claude, cursor or agents."),
                )
            }
        }
    }
    if hosts.is_empty() {
        hosts = HOSTS
            .iter()
            .copied()
            .filter(|(_, directory)| {
                root.join(directory.split('/').next().unwrap_or(directory))
                    .is_dir()
            })
            .collect();
        if hosts.is_empty() {
            hosts.push(HOSTS[2]);
        }
    }
    let app = app_slug(root);
    let agreement_digest = state.in_force_digest();
    let rendered_at = crate::service::utc_timestamp();
    let header = stamp(&agreement_digest, &rendered_at);
    let config_path = root.join(DRIVER_CONFIG_RELATIVE);
    let existing_config = fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let interviewed = interview(root);
    let config = existing_config
        .clone()
        .unwrap_or_else(|| interviewed.clone());

    let mut generated = vec![
        Rendered {
            relative: "SKILL.md".into(),
            contents: skill_markdown(&state, &app, &config, &header),
        },
        Rendered {
            relative: "features/README.md".into(),
            contents: features_readme(&state, &header),
        },
        Rendered {
            relative: "JOURNAL.md".into(),
            contents: journal_markdown(&state, &header),
        },
    ];
    for (id, feature) in features(&state) {
        generated.push(Rendered {
            relative: format!("features/{}.md", feature_slug(&id)),
            contents: feature_markdown(&state, &id, &feature, &header),
        });
    }
    let mut writes = Vec::new();
    for (host, directory) in &hosts {
        for file in &generated {
            writes.push((
                format!("{directory}/verify-{app}/{}", file.relative),
                file.contents.clone(),
                *host,
                "generated",
            ));
        }
    }
    let driver_path = root.join(crate::gates::DRIVER_RELATIVE);
    let driver_exists = driver_path.is_file();
    if !driver_exists || request.regenerate_driver {
        writes.push((
            crate::gates::DRIVER_RELATIVE.to_string(),
            DRIVER_TEMPLATE.to_string(),
            "repository",
            "scaffolded once; team-owned",
        ));
    }
    if existing_config.is_none() {
        let mut config = interviewed.clone();
        if let Some(object) = config.as_object_mut() {
            object.remove("interview");
        }
        writes.push((
            DRIVER_CONFIG_RELATIVE.to_string(),
            serde_json::to_string_pretty(&config).unwrap_or_default() + "\n",
            "repository",
            "scaffolded once; team-owned",
        ));
    }
    let plan = writes
        .iter()
        .map(|(path, contents, host, ownership)| {
            json!({
                "path": path,
                "host": host,
                "ownership": ownership,
                "bytes": contents.len(),
                "digest": digest(contents.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    let footprint = "The skill files contain your agreement text. They are Git-trackable unless the host directory is ignored; commit them only if the team should share them.";
    if request.dry_run {
        let mut response = ServiceResponse::new_public(
            request_id,
            "init",
            ServiceState::NeedsDecision,
            format!(
                "Dry run: {} file(s) would be written for {}; nothing was written.",
                writes.len(),
                hosts
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        response.permitted_actions =
            vec!["repeat without --dry-run to write exactly these files".into()];
        response.data = json!({
            "dry_run": true,
            "writes": plan,
            "interview": interviewed.get("interview"),
            "agreement_digest": agreement_digest,
            "footprint": footprint,
            "preview": generated.first().map(|file| file.contents.clone()),
        });
        return response;
    }
    for (path, contents, _, _) in &writes {
        if let Err(error) = write_file(root, path, contents.as_bytes()) {
            return error_response(
                request_id,
                ServiceState::Unknown,
                format!("{path} could not be written: {error}"),
            );
        }
    }
    #[cfg(unix)]
    if !driver_exists || request.regenerate_driver {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&driver_path, fs::Permissions::from_mode(0o755));
    }
    let manifest = SkillManifest {
        agreement_digest: agreement_digest.clone(),
        rendered_at: rendered_at.clone(),
        hosts: hosts.iter().map(|(name, _)| (*name).to_string()).collect(),
        files: writes
            .iter()
            .filter(|(_, _, _, ownership)| *ownership == "generated")
            .map(|(path, contents, _, _)| SkillFile {
                path: path.clone(),
                digest: digest(contents.as_bytes()),
            })
            .collect(),
    };
    if let Err(error) = fs::create_dir_all(layout.projections_path())
        .map_err(|error| error.to_string())
        .and_then(|()| {
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    fs::write(manifest_path(layout), bytes).map_err(|error| error.to_string())
                })
        })
    {
        return error_response(
            request_id,
            ServiceState::Unknown,
            format!("The skill manifest could not be recorded: {error}"),
        );
    }
    let proof_ready = state
        .in_force_matching(|body| {
            matches!(body, RecordBody::Standard(standard) if matches!(standard.enforcement, Enforcement::Drive { .. }))
        })
        .len();
    let mut response = ServiceResponse::new_public(
        request_id,
        "init",
        ServiceState::Success,
        format!(
            "The verification skill was written for {} and stamped with the current agreement.",
            hosts
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    response.permitted_actions = vec![
        "node whetstone/verify/drive.mjs doctor --json".into(),
        if proof_ready > 0 {
            "wh check --sweep".into()
        } else {
            "map a feature and a drive gate, then wh check --feature <id>".into()
        },
    ];
    response.data = json!({
        "writes": plan,
        "manifest": manifest,
        "interview": interviewed.get("interview"),
        "driver": {
            "path": crate::gates::DRIVER_RELATIVE,
            "config": DRIVER_CONFIG_RELATIVE,
            "scaffolded": !driver_exists || request.regenerate_driver,
        },
        "footprint": footprint,
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_path_safe() {
        assert_eq!(app_slug(Path::new("/tmp/My App!")), "my-app");
        assert_eq!(
            feature_slug(&RecordId::new("feature.dash_board").expect("id")),
            "dash-board"
        );
    }

    #[test]
    fn interview_prefers_a_web_dev_script() {
        let temp = tempfile::tempdir().expect("temp");
        fs::write(
            temp.path().join("package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .expect("package");
        fs::write(temp.path().join("pnpm-lock.yaml"), "").expect("lock");
        let found = interview(temp.path());
        assert_eq!(found["surface"], "web");
        assert_eq!(found["launch"]["command"][0], "pnpm");
        let temp = tempfile::tempdir().expect("temp");
        fs::write(temp.path().join("Cargo.toml"), "[package]").expect("cargo");
        assert_eq!(interview(temp.path())["surface"], "cli");
    }

    #[cfg(unix)]
    #[test]
    fn writes_never_follow_symlinked_directories() {
        let temp = tempfile::tempdir().expect("temp");
        let outside = tempfile::tempdir().expect("outside");
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".claude")).expect("symlink");
        assert!(write_file(temp.path(), ".claude/skills/x/SKILL.md", b"x").is_err());
        assert!(!outside.path().join("skills").exists());
    }
}
