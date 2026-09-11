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

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::agreement::AgreementState;
use crate::domain::{Enforcement, Feature, RecordBody, RecordId};
use crate::feature_map;
use crate::projection::{self, SkillFile, SkillManifest};
use crate::service::{InitRequest, ServiceResponse, ServiceState};
use crate::storage::{ProjectLayout, RecordStore, StoreKind};

pub const MANIFEST_FILE: &str = "skill.json";
pub const MAP_ID: &str = "map.verification";
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

pub use crate::feature_map::feature_slug;

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
                "env": {"PORT": "{port}"},
                "ready": "(https?://(?:localhost|127\\.0\\.0\\.1|\\[::1\\]|0\\.0\\.0\\.0):\\d+[^\\s]*)",
                "timeout_seconds": 120
            },
            "viewport": [1280, 900],
            "interview": format!("package.json script '{name}' looks like the local web app. The driver passes a checkout-derived port as PORT; if the dev server ignores PORT, add its port flag with {{port}} to launch.command, and confirm the ready pattern."),
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

/// Deterministic: the same accepted agreement stamps the same bytes.
fn stamp(agreement_digest: &str) -> String {
    format!(
        "<!-- Generated by Whetstone from agreement {agreement_digest}. Do not edit: change records with `wh change`, then run `wh init --action wire`. -->\n"
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
    map: &crate::domain::VerificationMap,
) -> String {
    let driver = feature_map::DRIVER_COMMAND;
    let mut out = String::new();
    let _ = writeln!(out, "---");
    let _ = writeln!(out, "name: verify-{app}");
    let _ = writeln!(
        out,
        "description: \"Launch, drive and prove {app} the way a user does, and know why each feature exists. Use before claiming a change works, to reproduce a report, or to understand the project's intent. Generated by Whetstone from the accepted agreement; a pstack verification skill.\""
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
    let _ = writeln!(
        out,
        "The durable \"why\" is the decision history: [`JOURNAL.md`](JOURNAL.md) here, and `wh dash --json` (`data.changelog`, or `wh dash --trail` for a show-me-your-work TSV). Read it before guessing at intent.\n"
    );
    let surface = config
        .get("surface")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("cli");
    let _ = writeln!(out, "## Launch\n");
    let _ = writeln!(
        out,
        "The driver owns launch and teardown for the `{surface}` surface; its configuration is `whetstone/verify/driver.json`. Each launch gets its own port and data directory, so two checkouts never share an instance.\n\n```bash\n{driver} launch --dry-run   # what would start\n{driver} launch             # start and wait until ready\n```\n"
    );
    let _ = writeln!(out, "## Doctor\n");
    let _ = writeln!(
        out,
        "Run this first, again after any failed drive, and whenever anything looks off. A drive against a stale build or an unhealthy instance is not evidence.\n\n```bash\n{driver} doctor --json\n```\n"
    );
    let _ = writeln!(out, "## Drive\n");
    let _ = writeln!(
        out,
        "The driver speaks pstack's control-adapter vocabulary: `doctor`, `launch`, `drive`, `inspect`, `screenshot`, `cleanup`, with `prove` as the composite `wh check` uses. Steps run in one session with evidence after each. Prefer stable handles (ids, ARIA labels, data attributes) over text, and text over coordinates.\n\n```bash\n{driver} drive \"open /\" \"expect text=<something visible>\" --json\n{driver} inspect / --json          # read-only: title, headings, landmarks, visible text\n{driver} screenshot / --json       # capture the current state of a path\n{driver} help                      # every step and flag\n```\n\nIf cursor-team-kit's `control-ui` or `control-cli` is installed you may drive with it instead; the evidence rules below still apply.\n"
    );
    let _ = writeln!(out, "## Evidence\n");
    let _ = writeln!(
        out,
        "`wh check` runs every gate and stores evidence (logs, screenshots, transcripts) in the private evidence directory it prints; cleanup never deletes it. A proof must:\n\n- exercise the production user path, not internal setters or test-only endpoints;\n- cover every reachable entry point the feature file lists, and the success, cancel, error, empty and persistence paths the change can affect;\n- show the action and the resulting state, and verify side effects (files, requests, stored records), not just pixels;\n- use mocks only behind a production boundary that already isolates the external system.\n\nAn unreachable path is reported with its concrete prerequisite (account, entitlement, OS), never skipped silently. A pass without evidence is unknown, never green.\n"
    );
    let _ = writeln!(out, "## Cleanup\n");
    let _ = writeln!(
        out,
        "```bash\n{driver} cleanup --dry-run   # what would stop\n{driver} cleanup             # stops only what the driver started; evidence stays\n```\n"
    );
    let _ = writeln!(out, "## Helpers\n");
    let _ = writeln!(
        out,
        "- `{driver}` is executable with Node 22 and has no dependencies; `{driver} help` documents every command.\n- `wh check --feature <id>` proves one feature through its drive gate and records a receipt; `wh check --changed` proves what your change touched; `wh check --sweep` drives every feature in the order below.\n- `wh check --dry-run` shows exactly which gates and commands would run.\n"
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
        "[`features/README.md`](features/README.md) lists every mapped feature in sweep order. Each feature file follows pstack's entry contract (four sections: Sub-features; How to get to it (user POV); Driving it with <harness>; Gotchas) and carries its record id, why it exists, links, proving gates and drift in frontmatter.\n\nWhen behaviour moves, update the feature in the same change: `wh change --kind feature --record-id <id> ...` records a private draft the owner accepts.\n"
    );
    let _ = writeln!(out, "## Judgment skills\n");
    let _ = writeln!(
        out,
        "Whetstone owns the deterministic half: records, gates, receipts, the sweep and map hygiene. When the pstack plugin is installed, its skills carry the judgment; they read this project's records first as the \"why\" source.\n\n- `how` explains how a feature works from source; `why` explains why it is the way it is (start from `JOURNAL.md` and `wh dash --json`).\n- `interrogate` reviews a change against the red flags above; `reflect` reviews a finished run.\n- `maintain-verification-skill` is the periodic maintenance pass for this skill (see below).\n"
    );
    let _ = writeln!(out, "## Maintenance\n");
    let _ = writeln!(
        out,
        "Run pstack's `/maintain-verification-skill` on this directory as the periodic pass. Its edit scope is this skill directory only (SKILL.md and `features/`); it never edits product code. Classify each difference as doc drift, harness gap or product gap:\n\n- doc drift and map corrections: edit the feature files here, then return them through Whetstone with `wh init --action import --from <this directory>`, which records each edited feature as a private draft with an exact before/after for the owner to accept (`wh change --accept <proposal>`). Regenerate with `wh init --action wire` afterwards; hand edits are otherwise overwritten.\n- harness gaps: fix `whetstone/verify/drive.mjs` or `driver.json` (team-owned) and re-drive.\n- product gaps: file them in the tracker (Beads) with the evidence paths; never paper over them in the map.\n\nThe deterministic half runs with `wh check --sweep`: one receipt per feature (proven, failed with the step, unreachable with the prerequisite, or skipped with the reason) plus hygiene findings. Report the pass outcome (clean, changed or blocked) with `wh check --maintain-outcome <clean|changed|blocked> --json` so it is recorded as a receipt.\n"
    );
    if !map.skill_notes.is_empty() {
        let _ = writeln!(out, "## Imported notes\n");
        let _ = writeln!(
            out,
            "Kept verbatim from the imported verification skill; fold them into the records above with `wh change` and they disappear from here.\n"
        );
        for note in &map.skill_notes {
            let _ = writeln!(out, "### {}\n\n{}\n", note.section, note.text.trim());
        }
    }
    out
}

/// The map conventions in force, or defaults derived from the driver config.
fn map_conventions(
    state: &AgreementState,
    app: &str,
    config: &serde_json::Value,
) -> crate::domain::VerificationMap {
    state
        .in_force_matching(|body| matches!(body, RecordBody::VerificationMap(_)))
        .into_iter()
        .find_map(|record| match &record.body {
            RecordBody::VerificationMap(map) => Some(map.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            feature_map::default_map(
                app,
                config
                    .get("surface")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("cli"),
            )
        })
}

fn features_readme(
    state: &AgreementState,
    map: &crate::domain::VerificationMap,
    header: &str,
) -> String {
    let features = features(state);
    let refs = features
        .iter()
        .map(|(id, feature)| (id, feature))
        .collect::<Vec<_>>();
    format!("{header}{}", feature_map::render_readme(map, &refs))
}

/// Why a feature exists, in one line, from the records it links.
fn why_line(state: &AgreementState, feature: &Feature) -> String {
    let serves = feature
        .serves
        .iter()
        .map(|id| title_of(state, id))
        .collect::<Vec<_>>();
    let constrained = feature
        .constrained_by
        .iter()
        .map(|id| title_of(state, id))
        .collect::<Vec<_>>();
    match (serves.is_empty(), constrained.is_empty()) {
        (true, true) => "No outcome is linked yet; ask the owner why this feature exists.".into(),
        (false, true) => format!("Serves: {}", serves.join("; ")),
        (true, false) => format!("Constrained by: {}", constrained.join("; ")),
        (false, false) => format!(
            "Serves: {}. Constrained by: {}",
            serves.join("; "),
            constrained.join("; ")
        ),
    }
}

fn feature_markdown(
    state: &AgreementState,
    id: &RecordId,
    feature: &Feature,
    agreement_digest: &str,
    proof: &projection::FeatureProof,
) -> String {
    let revision = state.in_force(id).map_or(0, |record| record.revision);
    let mut frontmatter = String::new();
    for (key, value) in [
        ("record", json!(id.as_str())),
        ("revision", json!(revision)),
        ("area", json!(feature.area)),
        ("sweep_order", json!(feature.sweep_order)),
        ("why", json!(why_line(state, feature))),
        ("serves", json!(feature.serves)),
        ("constrained_by", json!(feature.constrained_by)),
        ("proven_by", json!(feature.proven_by)),
        ("last_proven", json!(proof.last_proven)),
        ("drift", json!(proof.drift)),
        ("entry_points", json!(feature.entry_points)),
        ("drive_steps", json!(feature.drive_steps)),
        ("agreement", json!(agreement_digest)),
        (
            "generated_by",
            json!("Whetstone; change with `wh change --kind feature`, never by hand"),
        ),
    ] {
        frontmatter.push_str(&feature_map::frontmatter_line(key, &value));
    }
    feature_map::render_feature_file(&feature_map::RenderFeature {
        id,
        feature,
        frontmatter: Some(frontmatter),
    })
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
            RecordBody::VerificationMap(_) => "map",
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
    let store = layout.private_store();
    if !crate::beads::is_initialized(&store) {
        return error_response(
            request_id,
            ServiceState::NeedsInput,
            "No private agreement exists yet. Establish it with wh init --action agree, then wire."
                .into(),
        );
    }
    let records = match RecordStore::open_existing(&store, StoreKind::Private)
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
    let header = stamp(&agreement_digest);
    let config_path = root.join(DRIVER_CONFIG_RELATIVE);
    let existing_config = fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let interviewed = interview(root);
    let config = existing_config
        .clone()
        .unwrap_or_else(|| interviewed.clone());

    let map = map_conventions(&state, &app, &config);
    let proofs = projection::feature_proofs(&state, &projection::changes_for(&state, root));
    let mut generated = vec![
        Rendered {
            relative: "SKILL.md".into(),
            contents: skill_markdown(&state, &app, &config, &header, &map),
        },
        Rendered {
            relative: "features/README.md".into(),
            contents: features_readme(&state, &map, &header),
        },
        Rendered {
            relative: "JOURNAL.md".into(),
            contents: journal_markdown(&state, &header),
        },
    ];
    for (id, feature) in features(&state) {
        let proof = proofs.get(&id).cloned().unwrap_or_default();
        generated.push(Rendered {
            relative: format!("features/{}.md", feature_slug(&id)),
            contents: feature_markdown(&state, &id, &feature, &agreement_digest, &proof),
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

/// Every private record, or none when no private store exists yet.
fn private_records(layout: &ProjectLayout) -> Result<Vec<crate::domain::AgreementRecord>, String> {
    let store = layout.private_store();
    if !crate::beads::is_initialized(&store) {
        return Ok(Vec::new());
    }
    RecordStore::open_existing(&store, StoreKind::Private)
        .and_then(|repository| repository.all_records())
        .map_err(|error| error.to_string())
}

/// Whether this import request already recorded the record (a replay).
fn replayed(state: &AgreementState, request_id: &str, id: &str) -> bool {
    let key = format!("import:{request_id}:{id}");
    state
        .records()
        .iter()
        .any(|record| record.idempotency_key == key)
}

/// One record the importer would record as a private draft.
struct ImportCandidate {
    id: RecordId,
    body: RecordBody,
    source: String,
    source_digest: String,
}

fn import_error(request_id: String, state: ServiceState, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new_public(request_id, "init", state, summary);
    response.permitted_actions =
        vec!["wh init --action import --from <verify-skill directory> --dry-run".into()];
    response
}

/// Read a pstack verification skill directory (SKILL.md, features/README.md,
/// features/*.md) into private drafts: feature records and the map's
/// conventions. Nothing is accepted; each draft carries an exact
/// before/after for the owner. Re-importing a Whetstone-generated skill after
/// pstack's maintain pass records only what the pass changed.
pub fn import(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
) -> ServiceResponse {
    let root = layout.project_root();
    let Some(from) = request.import_from.as_ref() else {
        return import_error(
            request_id,
            ServiceState::NeedsInput,
            "Name the verification skill directory with --from <dir> (it holds SKILL.md and features/).".into(),
        );
    };
    let directory = if from.is_absolute() {
        from.clone()
    } else {
        root.join(from)
    };
    let directory = match directory.canonicalize() {
        Ok(path) if path.is_dir() => path,
        _ => {
            return import_error(
                request_id,
                ServiceState::NeedsInput,
                format!("{} is not a readable directory.", from.display()),
            )
        }
    };
    let features_dir = directory.join("features");
    if !features_dir.is_dir() {
        return import_error(
            request_id,
            ServiceState::NeedsInput,
            format!(
                "{} has no features/ directory; it is not a pstack verification skill.",
                directory.display()
            ),
        );
    }
    let read = |path: &Path| -> Result<(String, String), String> {
        let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() > 512 * 1024 {
            return Err(format!(
                "{} is not a regular file under 512 KiB",
                path.display()
            ));
        }
        let bytes = fs::read(path).map_err(|error| error.to_string())?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| format!("{} is not UTF-8", path.display()))?;
        Ok((text, digest(&bytes)))
    };
    let relative = |path: &Path| -> String {
        path.strip_prefix(root).map_or_else(
            |_| path.display().to_string(),
            |path| path.display().to_string(),
        )
    };
    let records = match private_records(layout) {
        Ok(records) => records,
        Err(error) => {
            return import_error(
                request_id,
                ServiceState::Unknown,
                format!("The private agreement could not be read: {error}"),
            )
        }
    };
    let state = AgreementState::from_records(records);
    let config = fs::read_to_string(root.join(DRIVER_CONFIG_RELATIVE))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .unwrap_or_else(|| interview(root));
    let app = app_slug(root);

    let mut candidates = Vec::<ImportCandidate>::new();
    let mut unchanged = Vec::new();
    let mut preserved = Vec::new();
    let readme_path = features_dir.join("README.md");
    let readme = if readme_path.is_file() {
        match read(&readme_path).and_then(|(text, digest)| {
            feature_map::parse_readme(&text).map(|parsed| (parsed, digest))
        }) {
            Ok(readme) => Some(readme),
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        }
    } else {
        None
    };
    let mut skill_notes = Vec::new();
    let skill_path = directory.join("SKILL.md");
    if skill_path.is_file() {
        match read(&skill_path) {
            Ok((text, _)) => {
                let generated = text.contains("<!-- Generated by Whetstone from agreement");
                skill_notes = if generated {
                    feature_map::parse_imported_notes(&text)
                } else {
                    feature_map::parse_skill(&text).1
                };
            }
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        }
    }
    if let Some((parsed, readme_digest)) = &readme {
        let mut map = parsed.map.clone();
        if map.entry_contract.as_deref() == Some(feature_map::STANDARD_ENTRY_CONTRACT) {
            map.entry_contract = None;
        }
        map.skill_notes.extend(skill_notes.iter().cloned());
        preserved.extend(parsed.preserved.iter().cloned());
        let map_id = RecordId::new(MAP_ID).expect("constant");
        let effective = state
            .pending(&map_id)
            .or_else(|| state.in_force(&map_id))
            .and_then(|record| match &record.body {
                RecordBody::VerificationMap(map) => Some(map.clone()),
                _ => None,
            })
            .unwrap_or_else(|| map_conventions(&state, &app, &config));
        if map == effective && !replayed(&state, &request_id, MAP_ID) {
            unchanged.push(json!({"record": MAP_ID, "file": relative(&readme_path)}));
        } else {
            candidates.push(ImportCandidate {
                id: RecordId::new(MAP_ID).expect("constant"),
                body: RecordBody::VerificationMap(map),
                source: relative(&readme_path),
                source_digest: readme_digest.clone(),
            });
        }
    }
    let mut files = match fs::read_dir(&features_dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|extension| extension == "md")
                    && path.file_name().is_some_and(|name| name != "README.md")
            })
            .collect::<Vec<_>>(),
        Err(error) => return import_error(request_id, ServiceState::Unknown, error.to_string()),
    };
    files.sort();
    if files.len() > 200 {
        return import_error(
            request_id,
            ServiceState::NeedsInput,
            "The map has more than 200 feature files; import it in parts.".into(),
        );
    }
    for path in &files {
        let stem = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        let (text, file_digest) = match read(path) {
            Ok(read) => read,
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        };
        let mut parsed = match feature_map::parse_feature(&stem, &text) {
            Ok(parsed) => parsed,
            Err(error) => return import_error(request_id, ServiceState::NeedsInput, error),
        };
        if let Some((readme, _)) = &readme {
            if let Some((index, entry)) = readme
                .entries
                .iter()
                .enumerate()
                .find(|(_, entry)| entry.0 == stem)
            {
                let default_line = format!("— {}", parsed.feature.summary.trim());
                if entry.2 != default_line && !entry.2.trim().is_empty() {
                    parsed.feature.index_summary = Some(entry.2.clone());
                }
                if parsed.feature.sweep_order == 0 {
                    parsed.feature.sweep_order = u32::try_from(index + 1).unwrap_or(u32::MAX);
                }
                if let Some(area) = &entry.3 {
                    parsed.feature.area = area.clone();
                }
            } else {
                preserved.push(format!("{stem}.md is not listed in features/README.md"));
            }
        }
        preserved.extend(
            parsed
                .preserved
                .iter()
                .map(|note| format!("{stem}.md: {note}")),
        );
        let body = RecordBody::Feature(parsed.feature);
        if let Err(error) = crate::domain::RecordBody::validate(&body) {
            return import_error(
                request_id,
                ServiceState::NeedsInput,
                format!("{stem}.md does not make a valid feature record: {error}"),
            );
        }
        let current = state
            .pending(&parsed.id)
            .or_else(|| state.in_force(&parsed.id));
        if current.is_some_and(|record| record.body == body)
            && !replayed(&state, &request_id, parsed.id.as_str())
        {
            unchanged.push(json!({"record": parsed.id.as_str(), "file": relative(path)}));
            continue;
        }
        candidates.push(ImportCandidate {
            id: parsed.id,
            body,
            source: relative(path),
            source_digest: file_digest,
        });
    }
    let plan = candidates
        .iter()
        .map(|candidate| {
            let before = state
                .pending(&candidate.id)
                .or_else(|| state.in_force(&candidate.id))
                .map(|record| &record.body);
            json!({
                "record": candidate.id.as_str(),
                "file": candidate.source,
                "file_digest": candidate.source_digest,
                "diff": {"before": before, "after": &candidate.body},
            })
        })
        .collect::<Vec<_>>();
    if request.dry_run || candidates.is_empty() {
        let mut response = ServiceResponse::new_public(
            request_id,
            "init",
            if candidates.is_empty() {
                ServiceState::Success
            } else {
                ServiceState::NeedsDecision
            },
            if candidates.is_empty() {
                format!(
                    "Nothing to import: {} record(s) already match the imported files.",
                    unchanged.len()
                )
            } else {
                format!(
                    "Dry run: {} private draft(s) would be recorded; nothing was written or accepted.",
                    candidates.len()
                )
            },
        );
        response.permitted_actions = if candidates.is_empty() {
            vec!["wh init --action wire".into()]
        } else {
            vec!["repeat without --dry-run to record exactly these drafts".into()]
        };
        response.data = json!({
            "dry_run": request.dry_run,
            "drafts": plan,
            "unchanged": unchanged,
            "preserved_as_gotchas": preserved,
            "accepted": [],
        });
        return response;
    }
    let private = match crate::storage::RecordStore::initialize(
        &layout.private_store(),
        StoreKind::Private,
    ) {
        Ok(private) => private,
        Err(error) => {
            return crate::service::storage_error("init", request_id, error);
        }
    };
    let mut drafts = Vec::new();
    for candidate in &candidates {
        let key = format!("import:{request_id}:{}", candidate.id.as_str());
        let existing = match private.by_idempotency_key(&key) {
            Ok(existing) => existing,
            Err(error) => return crate::service::storage_error("init", request_id, error),
        };
        let (record, reference) = match existing {
            Some(existing) if existing.body == candidate.body => match existing.reference() {
                Ok(reference) => (existing, reference),
                Err(error) => return crate::service::domain_response("init", request_id, error),
            },
            Some(_) => {
                return import_error(
                    request_id,
                    ServiceState::Conflict,
                    format!("This import request id already recorded different content for {}; use a new --request-id.", candidate.id.as_str()),
                )
            }
            None => {
                let latest = match private.latest(&candidate.id) {
                    Ok(latest) => latest,
                    Err(error) => return crate::service::storage_error("init", request_id, error),
                };
                let owner = state
                    .in_force(&RecordId::new(projection::MISSION_ID).expect("constant"))
                    .and_then(|record| record.owner.display_name.clone());
                let mut record = match crate::service::agreement_record(
                    layout,
                    candidate.id.as_str(),
                    key.clone(),
                    candidate.body.clone(),
                    owner.as_deref(),
                ) {
                    Ok(record) => record,
                    Err(error) => return crate::service::domain_response("init", request_id, error),
                };
                record.revision = latest.as_ref().map_or(1, |record| record.revision + 1);
                record.supersedes = match latest.as_ref().map(crate::domain::AgreementRecord::reference) {
                    Some(Ok(reference)) => Some(reference),
                    Some(Err(error)) => return crate::service::domain_response("init", request_id, error),
                    None => None,
                };
                record.provenance.kind = crate::domain::ProvenanceKind::ImportedNote;
                record.provenance.authority = crate::domain::ProvenanceAuthority::CandidateOnly;
                record.provenance.sources = vec![crate::domain::EvidenceRef {
                    system: "verification_skill_import".into(),
                    locator: candidate.source.clone(),
                    digest: crate::domain::ContentDigest::new(candidate.source_digest.clone()).ok(),
                }];
                let reference = match private.append(&record, latest.as_ref().map(|record| record.revision)) {
                    Ok(reference) => reference,
                    Err(error) => return crate::service::storage_error("init", request_id, error),
                };
                (record, reference)
            }
        };
        let narrative = crate::service::ChangeNarrative {
            rationale: format!("Imported from {} for review.", candidate.source),
            source: candidate.source.clone(),
            expected_effect: "The verification map matches the imported skill once accepted."
                .into(),
            impact: "not stated".into(),
            examples: Vec::new(),
            conflicts: Vec::new(),
        };
        let proposal = match crate::service::ensure_local_change_proposal(
            &private,
            &record,
            &reference,
            &narrative,
            record.revision.saturating_sub(1),
            &key,
        ) {
            Ok(proposal) => proposal,
            Err(error) => return crate::service::storage_error("init", request_id, error),
        };
        drafts.push(json!({
            "record": reference,
            "proposal": proposal,
            "file": candidate.source,
        }));
    }
    let mut response = ServiceResponse::new_public(
        request_id,
        "init",
        ServiceState::NeedsDecision,
        format!(
            "{} private draft(s) were recorded from the imported skill. Nothing is accepted until you review each one.",
            drafts.len()
        ),
    );
    response.permitted_actions = drafts
        .iter()
        .filter_map(|draft| draft["proposal"]["id"].as_str())
        .map(|proposal| format!("wh change --accept {proposal}"))
        .collect();
    response.data = json!({
        "drafts": drafts,
        "plan": plan,
        "unchanged": unchanged,
        "preserved_as_gotchas": preserved,
        "accepted": [],
        "shared": false,
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
