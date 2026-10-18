//! Tool setup for `wh init` and bare `wh`: detect the tool the record store
//! needs (Beads, or the virgil CLI under the virgil store), report what is
//! missing or out of date, and name the exact command that fixes it.
//!
//! Detection is read-only. Offline, or without a tool, the answer is
//! `missing` or `unknown` with a command, never a guess.

pub use crate::beads::MIN_BD_VERSION;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolState {
    pub tool: String,
    /// `ok`, `missing`, `outdated` or `unknown`.
    pub state: String,
    pub found: Option<String>,
    pub detail: String,
    /// The exact command that fixes it, when one exists.
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Setup {
    pub tools: Vec<ToolState>,
    /// Every problem and its fix, in the order to run them.
    pub fixes: Vec<String>,
    pub ready: bool,
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Whether `bd` is installed at the pinned minimum.
pub fn bd_ready() -> bool {
    crate::beads::ensure_bd_version().is_ok()
}

/// The installed `bd` version, if any.
pub fn bd_version() -> Option<String> {
    crate::beads::ensure_bd_version().ok().or_else(|| {
        let output = Command::new("bd")
            .arg("version")
            .env("BD_NON_INTERACTIVE", "1")
            .output()
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))
            .map(str::to_owned)
    })
}

fn on_path(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(program).is_file())
    })
}

/// The command that installs `bd` on this machine.
pub fn bd_install_command() -> String {
    if cfg!(target_os = "macos") && on_path("brew") {
        "brew install beads".into()
    } else {
        let os = if cfg!(target_os = "macos") {
            "darwin"
        } else {
            "linux"
        };
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "amd64"
        };
        let version = crate::beads::MIN_BD_VERSION;
        format!(
            "curl -fsSL -o /tmp/bd.tar.gz https://github.com/gastownhall/beads/releases/download/v{version}/beads_{version}_{os}_{arch}.tar.gz && tar -xzf /tmp/bd.tar.gz -C ~/.local/bin bd"
        )
    }
}

/// The command that brings an older `bd` up to the pinned minimum.
pub fn bd_upgrade_command() -> String {
    if cfg!(target_os = "macos") && on_path("brew") {
        "brew upgrade beads".into()
    } else {
        bd_install_command()
    }
}

/// Detect the tool the record store needs.
pub fn detect(project_root: &Path) -> Setup {
    let mut tools = Vec::new();
    let mut fixes = Vec::new();
    // The virgil store keeps every record in Virgil (drafts in its private scope),
    // so bd is never needed there and never offered.
    let virgil_store = crate::storage::read_config(project_root)
        .is_ok_and(|config| config == crate::storage::StoreConfig::Virgil);
    if virgil_store {
        tools.push(virgil_tool(project_root));
    }
    match bd_version() {
        _ if virgil_store => {}
        Some(version) if crate::beads::ensure_bd_version().is_ok() => tools.push(ToolState {
            tool: "bd".into(),
            state: "ok".into(),
            found: Some(version),
            detail: format!(
                "Beads {} or later is installed.",
                crate::beads::MIN_BD_VERSION
            ),
            fix: None,
        }),
        Some(version) => {
            let fix = bd_upgrade_command();
            fixes.push(fix.clone());
            tools.push(ToolState {
                tool: "bd".into(),
                state: "outdated".into(),
                found: Some(version),
                detail: format!(
                    "Whetstone needs Beads {} or later.",
                    crate::beads::MIN_BD_VERSION
                ),
                fix: Some(fix),
            });
        }
        None => {
            let fix = bd_install_command();
            fixes.push(fix.clone());
            tools.push(ToolState {
                tool: "bd".into(),
                state: "missing".into(),
                found: None,
                detail: "Beads is Whetstone's record store and is not installed.".into(),
                fix: Some(fix),
            });
        }
    }
    let ready = tools.iter().all(|tool| tool.state == "ok");
    Setup {
        tools,
        fixes,
        ready,
    }
}

/// The `virgil` CLI the virgil store reaches Records through, asked from
/// the checkout, as every store call asks it.
fn virgil_tool(project_root: &Path) -> ToolState {
    let found = Command::new("virgil")
        .arg("--version")
        .current_dir(project_root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .find(|word| word.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .unwrap_or("unknown")
                .to_string()
        });
    match found {
        Some(version) => ToolState {
            tool: "virgil".into(),
            state: "ok".into(),
            found: Some(version),
            detail: "The store switch names Virgil; records reach the group's Records through the virgil CLI, and bd is not needed.".into(),
            fix: None,
        },
        None => ToolState {
            tool: "virgil".into(),
            state: "missing".into(),
            found: None,
            detail: "The store switch names Virgil, but the virgil CLI does not answer. Install or start Virgil, or switch back with wh init --action setup --store beads.".into(),
            fix: None,
        },
    }
}

/// Run an install command that needs no interaction, without a shell: `&&`
/// chains become literal argv sequences.
pub fn run_install(project_root: &Path, command: &str) -> Result<String, String> {
    let command = command.replace("~/", &format!("{}/", home().unwrap_or_default().display()));
    let sequences = crate::gates::parse_command(&command)?;
    let mut output = String::new();
    for argv in sequences {
        let program = crate::gates::resolve_program(project_root, &argv[0])?;
        let result = crate::execution::run_bounded(
            &program,
            &argv[1..],
            project_root,
            &crate::gates::gate_environment(&[]),
            std::time::Duration::from_secs(600),
            1024 * 1024,
            1024 * 1024,
        )
        .map_err(|error| format!("{} could not start: {error}", argv[0]))?;
        output.push_str(&result.stdout.text);
        output.push_str(&result.stderr.text);
        if !result.status.success() {
            return Err(format!(
                "{} failed: {}",
                argv.join(" "),
                output.lines().last().unwrap_or_default()
            ));
        }
    }
    Ok(output)
}
