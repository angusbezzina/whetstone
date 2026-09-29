//! Agent-facing projection of the agreement: the verification skill.
//!
//! `wh init --action wire` renders, from accepted records only:
//!
//! * `SKILL.md`: why the project exists (mission and principles), the rules
//!   in force by strength and enforcer, how to brief before building, prove
//!   before handing back and raise a hand, and how to launch, doctor, drive
//!   and prove the app;
//! * `rules/<rule>.md`: one editable file per rule, returned with import;
//! * `features/README.md` and one file per feature: the pstack runbook with
//!   the four fixed headings;
//! * `whetstone.verify.json`: the versioned source document
//!   (`whetstone.verify.v1`) the Markdown is rendered from;
//! * `JOURNAL.md`: the decision log behind the current agreement.
//!
//! The same files are written byte-identically into every selected agent host
//! directory, stamped with the agreement digest. The driver script and its
//! configuration are scaffolded once from a repository interview and then
//! belong to the team; they are never overwritten unless asked.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::projection::SkillManifest;
use crate::service::{ServiceResponse, ServiceState};
use crate::storage::ProjectLayout;

mod import;
mod render;
pub mod rules;
mod wire;

pub use import::import;
pub use render::verify_document;
pub use wire::wire;

pub const MANIFEST_FILE: &str = "skill.json";
pub const MAP_ID: &str = "map.verification";
pub const DRIVER_TEMPLATE: &str = include_str!("../../assets/verify/drive.mjs");
pub const CI_TEMPLATE: &str = include_str!("../../assets/ci/whetstone.yml");
/// The workflow `wh init --action wire --ci` scaffolds once.
pub const CI_WORKFLOW: &str = ".github/workflows/whetstone.yml";
pub const DRIVER_CONFIG_RELATIVE: &str = "whetstone/verify/driver.json";
/// Agent hosts and the skill directory each reads. Codex reads the shared
/// `.agents/skills` directory and gets its own hooks.
pub const HOSTS: [(&str, &str); 4] = [
    ("claude", ".claude/skills"),
    ("cursor", ".cursor/skills"),
    ("agents", ".agents/skills"),
    ("codex", ".agents/skills"),
];

/// Files `wh init --action wire` writes: the driver, its config, the CI
/// workflow and the verification skill in every host. They are Whetstone's
/// own wiring, not the project's code, so content gates and Jev questions
/// never read them (otherwise the commit that installs the gates is blocked
/// by them).
pub fn is_generated(path: &str) -> bool {
    path == crate::gates::DRIVER_RELATIVE
        || path == DRIVER_CONFIG_RELATIVE
        || path == CI_WORKFLOW
        || HOSTS.iter().any(|(_, directory)| {
            path.strip_prefix(directory)
                .and_then(|rest| rest.strip_prefix("/verify-"))
                .is_some_and(|rest| rest.contains('/'))
        })
}

pub fn manifest_path(layout: &ProjectLayout) -> PathBuf {
    layout.projections_path().join(MANIFEST_FILE)
}

pub fn read_manifest(layout: &ProjectLayout) -> Option<SkillManifest> {
    let bytes = fs::read(manifest_path(layout)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub(crate) fn digest(bytes: &[u8]) -> String {
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

pub(crate) struct Rendered {
    pub(crate) relative: String,
    pub(crate) contents: String,
}

/// Deterministic: the same accepted agreement stamps the same bytes.
pub(crate) fn stamp(agreement_digest: &str) -> String {
    format!(
        "<!-- Generated by Whetstone from agreement {agreement_digest}. Edits must return through Whetstone: `wh init --action import --from <this skill directory>` (or `wh change`); `wh init --action wire` overwrites edits that were not returned. -->\n"
    )
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

pub(crate) fn write_file(root: &Path, relative: &str, contents: &[u8]) -> Result<(), String> {
    reject_symlinked_ancestors(root, relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let staging = path.with_extension("whetstone-staging");
    fs::write(&staging, contents).map_err(|error| error.to_string())?;
    fs::rename(&staging, &path).map_err(|error| error.to_string())
}

pub(crate) fn error_response(
    request_id: String,
    state: ServiceState,
    summary: String,
) -> ServiceResponse {
    let mut response = ServiceResponse::new_public(request_id, "init", state, summary);
    response.permitted_actions = vec!["wh init".into()];
    response
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_whetstone_wiring_counts_as_generated() {
        for path in [
            "whetstone/verify/drive.mjs",
            "whetstone/verify/driver.json",
            ".github/workflows/whetstone.yml",
            ".claude/skills/verify-notes/SKILL.md",
            ".agents/skills/verify-notes/rules/no-todo.md",
        ] {
            assert!(super::is_generated(path), "{path}");
        }
        for path in [
            "src/lib.rs",
            ".claude/skills/other/SKILL.md",
            ".github/workflows/ci.yml",
            "whetstone/verify/extra.mjs",
        ] {
            assert!(!super::is_generated(path), "{path}");
        }
    }

    use super::*;
    use crate::domain::RecordId;

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
