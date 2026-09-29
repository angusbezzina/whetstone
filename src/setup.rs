//! Tool setup for `wh init` and bare `wh`: detect Beads and the pstack skills
//! each agent host needs, pin what was found in a lock file, report drift,
//! and name the exact command that fixes anything missing.
//!
//! Detection is read-only. Offline, or without a tool, the answer is
//! `missing` or `unknown` with a command, never a guess.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The pinned lock file, committed with the repository.
pub const LOCK_RELATIVE: &str = "whetstone/tools.lock.json";
pub const LOCK_SCHEMA: &str = "whetstone.tools-lock.v1";
/// The pstack release Whetstone's catalogue and fixtures are pinned to.
pub const PSTACK_VERSION: &str = "0.15.5";
pub const PSTACK_COMMIT: &str = "12d587dfb20741cafc376c42c696c5f6e2a64487";

/// pstack skills the plan uses; Whetstone never reimplements them.
pub const PSTACK_SKILLS: &[&str] = &[
    "how",
    "why",
    "blast-radius",
    "interrogate",
    "reflect",
    "create-verification-skill",
    "maintain-verification-skill",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolState {
    pub tool: String,
    /// `ok`, `missing`, `outdated`, `drifted` or `unknown`.
    pub state: String,
    pub found: Option<String>,
    pub detail: String,
    /// The exact command that fixes it, when one exists.
    pub fix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillState {
    pub skill: String,
    pub host: String,
    pub path: Option<String>,
    pub digest: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Setup {
    pub tools: Vec<ToolState>,
    pub skills: Vec<SkillState>,
    pub lock: Option<Lock>,
    /// Every problem and its fix, in the order to run them.
    pub fixes: Vec<String>,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    pub schema: String,
    pub bd: String,
    pub pstack_version: String,
    pub pstack_commit: String,
    /// skill name to SKILL.md digest, as installed when the lock was written.
    pub skills: BTreeMap<String, String>,
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Skill directories a host reads, project first, then the user's.
fn skill_roots(project_root: &Path, host: &str, home: Option<&Path>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let project = match host {
        "claude" => ".claude/skills",
        "cursor" => ".cursor/skills",
        _ => ".agents/skills",
    };
    roots.push(project_root.join(project));
    if let Some(home) = home {
        roots.push(home.join(project));
        if host == "claude" {
            // Claude Code plugins record where they were installed.
            let record = home.join(".claude/plugins/installed_plugins.json");
            if let Ok(text) = fs::read_to_string(record) {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(plugins) = value.get("plugins").and_then(|value| value.as_object())
                    {
                        for (name, installs) in plugins {
                            if !name.starts_with("pstack@") {
                                continue;
                            }
                            for install in installs.as_array().into_iter().flatten() {
                                if let Some(path) =
                                    install.get("installPath").and_then(|value| value.as_str())
                                {
                                    roots.push(PathBuf::from(path).join("skills"));
                                }
                            }
                        }
                    }
                }
            }
        }
        if host == "cursor" {
            roots.push(home.join(".cursor/plugins/local/pstack/skills"));
        }
    }
    roots
}

/// Where a pstack skill is installed for a host, and its digest.
pub fn find_skill(project_root: &Path, host: &str, skill: &str) -> SkillState {
    find_skill_in(project_root, host, skill, home().as_deref())
}

fn find_skill_in(project_root: &Path, host: &str, skill: &str, home: Option<&Path>) -> SkillState {
    for root in skill_roots(project_root, host, home) {
        let path = root.join(skill).join("SKILL.md");
        if let Ok(bytes) = fs::read(&path) {
            return SkillState {
                skill: skill.into(),
                host: host.into(),
                path: Some(path.display().to_string()),
                digest: Some(digest(&bytes)),
            };
        }
    }
    SkillState {
        skill: skill.into(),
        host: host.into(),
        path: None,
        digest: None,
    }
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

/// How pstack is installed for a host. Upstream ships a Cursor plugin; the
/// Claude Code and Codex installs are community ports, named as such.
pub fn pstack_install_command(host: &str) -> String {
    match host {
        "cursor" => "In Cursor, run /add-plugin pstack".into(),
        "claude" => "claude plugin marketplace add michael-denyer/pstack-claude && claude plugin install pstack@pstack-claude (community port of upstream pstack)".into(),
        _ => "npx skills add mdsmithaustin/pstack (community port of upstream pstack)".into(),
    }
}

pub fn read_lock(project_root: &Path) -> Option<Lock> {
    let text = fs::read_to_string(project_root.join(LOCK_RELATIVE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// Detect everything setup needs for the given hosts (default: the host
/// directories that exist, else `agents`).
pub fn detect(project_root: &Path, hosts: &[String]) -> Setup {
    detect_in(project_root, hosts, home().as_deref())
}

/// [`detect`] with an explicit user directory (none: project only).
pub fn detect_in(project_root: &Path, hosts: &[String], home: Option<&Path>) -> Setup {
    let hosts = if hosts.is_empty() {
        let found = ["claude", "cursor", "agents"]
            .into_iter()
            .filter(|host| {
                let directory = match *host {
                    "claude" => ".claude",
                    "cursor" => ".cursor",
                    _ => ".agents",
                };
                project_root.join(directory).is_dir()
            })
            .map(str::to_owned)
            .collect::<Vec<_>>();
        if found.is_empty() {
            vec!["agents".to_string()]
        } else {
            found
        }
    } else {
        hosts.to_vec()
    };
    let lock = read_lock(project_root);
    let mut tools = Vec::new();
    let mut fixes = Vec::new();
    match bd_version() {
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
            let fix = bd_install_command();
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
    let mut skills = Vec::new();
    for host in &hosts {
        let found = PSTACK_SKILLS
            .iter()
            .map(|skill| find_skill_in(project_root, host, skill, home))
            .collect::<Vec<_>>();
        let missing = found
            .iter()
            .filter(|skill| skill.path.is_none())
            .map(|skill| skill.skill.clone())
            .collect::<Vec<_>>();
        let drifted = lock
            .as_ref()
            .map(|lock| {
                found
                    .iter()
                    .filter(|skill| {
                        skill.digest.as_ref().is_some_and(|digest| {
                            lock.skills
                                .get(&skill.skill)
                                .is_some_and(|pinned| pinned != digest)
                        })
                    })
                    .map(|skill| skill.skill.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (state, detail, fix) = if !missing.is_empty() {
            let fix = pstack_install_command(host);
            (
                "missing",
                format!("pstack skills missing for {host}: {}.", missing.join(", ")),
                Some(fix),
            )
        } else if !drifted.is_empty() {
            (
                "drifted",
                format!(
                    "pstack skills changed since {LOCK_RELATIVE} was written: {}. Re-pin with wh init --action setup.",
                    drifted.join(", ")
                ),
                Some("wh init --action setup".to_string()),
            )
        } else {
            (
                "ok",
                format!("pstack skills are installed for {host}."),
                None,
            )
        };
        if let Some(fix) = &fix {
            fixes.push(fix.clone());
        }
        tools.push(ToolState {
            tool: format!("pstack:{host}"),
            state: state.into(),
            found: lock.as_ref().map(|lock| lock.pstack_version.clone()),
            detail,
            fix,
        });
        skills.extend(found);
    }
    if lock.is_none() && tools.iter().all(|tool| tool.state == "ok") {
        fixes.push("wh init --action setup (pin the versions found)".into());
    }
    let ready = tools.iter().all(|tool| tool.state == "ok");
    Setup {
        tools,
        skills,
        lock,
        fixes,
        ready,
    }
}

/// The lock for what is installed now.
pub fn lock_for(setup: &Setup) -> Option<Lock> {
    let bd = setup
        .tools
        .iter()
        .find(|tool| tool.tool == "bd" && tool.state == "ok")
        .and_then(|tool| tool.found.clone())?;
    let mut skills = BTreeMap::new();
    for skill in &setup.skills {
        if let Some(digest) = &skill.digest {
            skills
                .entry(skill.skill.clone())
                .or_insert_with(|| digest.clone());
        }
    }
    Some(Lock {
        schema: LOCK_SCHEMA.into(),
        bd,
        pstack_version: PSTACK_VERSION.into(),
        pstack_commit: PSTACK_COMMIT.into(),
        skills,
    })
}

pub fn write_lock(project_root: &Path, lock: &Lock) -> Result<(), String> {
    let path = project_root.join(LOCK_RELATIVE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(lock).map_err(|error| error.to_string())? + "\n";
    fs::write(path, text).map_err(|error| error.to_string())
}

/// Run an install command that needs no interaction, without a shell: `&&`
/// chains become literal argv sequences. Commands that only a host app can
/// run (Cursor's `/add-plugin`) are reported, not attempted.
pub fn run_install(project_root: &Path, command: &str) -> Result<String, String> {
    if command.starts_with("In Cursor") {
        return Err(format!("Run this yourself: {command}"));
    }
    let command = command
        .split(" (community port")
        .next()
        .unwrap_or(command)
        .replace("~/", &format!("{}/", home().unwrap_or_default().display()));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_skill_is_found_and_digested() {
        let temp = tempfile::tempdir().expect("temp");
        let skill = temp.path().join(".agents/skills/how");
        fs::create_dir_all(&skill).expect("dir");
        fs::write(skill.join("SKILL.md"), "---\nname: how\n---\n").expect("write");
        let found = find_skill_in(temp.path(), "agents", "how", None);
        assert!(found.path.is_some());
        assert!(found
            .digest
            .is_some_and(|digest| digest.starts_with("sha256:")));
        assert!(find_skill_in(temp.path(), "agents", "why", None)
            .path
            .is_none());
    }

    #[test]
    fn a_removed_or_changed_skill_is_reported_with_a_fix() {
        let temp = tempfile::tempdir().expect("temp");
        for skill in PSTACK_SKILLS {
            let dir = temp.path().join(".agents/skills").join(skill);
            fs::create_dir_all(&dir).expect("dir");
            fs::write(dir.join("SKILL.md"), format!("# {skill}\n")).expect("write");
        }
        let setup = detect_in(temp.path(), &["agents".into()], None);
        let pstack = setup
            .tools
            .iter()
            .find(|tool| tool.tool == "pstack:agents")
            .expect("pstack");
        assert_eq!(pstack.state, "ok");
        let mut lock = Lock {
            schema: LOCK_SCHEMA.into(),
            bd: "1.3.0".into(),
            pstack_version: PSTACK_VERSION.into(),
            pstack_commit: PSTACK_COMMIT.into(),
            skills: setup
                .skills
                .iter()
                .filter_map(|skill| Some((skill.skill.clone(), skill.digest.clone()?)))
                .collect(),
        };
        lock.skills.insert("why".into(), "sha256:changed".into());
        write_lock(temp.path(), &lock).expect("lock");
        let drifted = detect_in(temp.path(), &["agents".into()], None);
        assert!(drifted
            .tools
            .iter()
            .any(|tool| tool.tool == "pstack:agents" && tool.state == "drifted"));
        fs::remove_dir_all(temp.path().join(".agents/skills/interrogate")).expect("remove");
        let missing = detect_in(temp.path(), &["agents".into()], None);
        let pstack = missing
            .tools
            .iter()
            .find(|tool| tool.tool == "pstack:agents")
            .expect("pstack");
        assert_eq!(pstack.state, "missing");
        assert!(pstack.detail.contains("interrogate"));
        assert!(pstack.fix.is_some());
        assert!(!missing.ready);
    }
}
