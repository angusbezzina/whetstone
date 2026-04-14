//! Team config resolution via `extends:`.
//!
//! An `extends` entry in `whetstone/whetstone.yaml` names a rule source the
//! project wants to layer in. Supported forms:
//!
//! - `whetstone:recommended` — already provided by the embedded built-in layer; no-op here.
//! - `github.com/<org>/<repo>` — cloned into `whetstone/.cache/teams/<org>/<repo>/`.
//! - `@user/config` — reserved for a future shared registry; currently reported as pending.
//! - `https://.../config.yaml` — single-file HTTP fetch, cached under `.cache/teams/http/`.
//!
//! The shipped binary implements the git-clone path for `github.com/` entries
//! using the user's existing git credentials (no bundled HTTP client for git).
//! TTL refresh is driven by `wh refresh` rewriting the cache; individual team
//! clones are updated with `git pull` if the cache dir already exists.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtendsEntry {
    Builtin,
    Github { owner: String, repo: String },
    Registry { handle: String },
    HttpYaml { url: String },
    Unknown(String),
}

pub fn parse_extends_entry(raw: &str) -> ExtendsEntry {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ExtendsEntry::Unknown(raw.to_string());
    }
    if trimmed == "whetstone:recommended" || trimmed.starts_with("whetstone:recommended/") {
        return ExtendsEntry::Builtin;
    }
    // Accept bare `github.com/...` and `https://github.com/...`.
    for prefix in ["github.com/", "https://github.com/", "http://github.com/"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let rest = rest.trim_end_matches('/').trim_end_matches(".git");
            let mut parts = rest.splitn(3, '/');
            let owner = parts.next().unwrap_or("").to_string();
            let repo = parts.next().unwrap_or("").to_string();
            if !owner.is_empty() && !repo.is_empty() {
                return ExtendsEntry::Github { owner, repo };
            }
        }
    }
    if let Some(handle) = trimmed.strip_prefix('@') {
        if !handle.is_empty() {
            return ExtendsEntry::Registry {
                handle: handle.to_string(),
            };
        }
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return ExtendsEntry::HttpYaml {
            url: trimmed.to_string(),
        };
    }
    ExtendsEntry::Unknown(raw.to_string())
}

pub struct TeamResolution {
    pub rules_dirs: Vec<PathBuf>,
    pub deny: Vec<String>,
    /// Per-entry resolution status — surfaced by the CLI in JSON output.
    #[allow(dead_code)]
    pub statuses: Vec<serde_json::Value>,
}

/// Resolve every `extends:` entry. Returns the on-disk rule directories to
/// feed into `LayerSet`, plus a per-entry status payload that the CLI can
/// surface to the user.
///
/// When `refresh` is true, every git-backed clone gets a `git pull --ff-only`
/// to pick up upstream changes. Otherwise the cache is reused as-is.
pub fn resolve(project_dir: &Path, extends: &[String], refresh: bool) -> Result<TeamResolution> {
    let cache_root = project_dir.join("whetstone").join(".cache").join("teams");
    let mut rules_dirs = Vec::new();
    let mut deny = Vec::new();
    let mut statuses = Vec::new();

    for raw in extends {
        let entry = parse_extends_entry(raw);
        match entry {
            ExtendsEntry::Builtin => {
                statuses.push(serde_json::json!({
                    "entry": raw,
                    "kind": "builtin",
                    "status": "ok",
                    "note": "Provided by the embedded whetstone:recommended layer; no fetch required.",
                }));
            }
            ExtendsEntry::Github { owner, repo } => {
                let dest = cache_root.join(&owner).join(&repo);
                let result = sync_github(&owner, &repo, &dest, refresh);
                match result {
                    Ok(did_fetch) => {
                        // Team rule files can live either at the repo root under
                        // `whetstone/rules/` (mirroring a project layout) or at
                        // `rules/` directly (team-only publisher layout).
                        let candidates = [dest.join("whetstone").join("rules"), dest.join("rules")];
                        let mut selected: Option<PathBuf> = None;
                        for c in &candidates {
                            if c.exists() {
                                selected = Some(c.clone());
                                break;
                            }
                        }
                        if let Some(p) = selected {
                            rules_dirs.push(p.clone());
                            deny.extend(
                                crate::config::WhetstoneConfig::load_project_only(&dest).deny,
                            );
                            statuses.push(serde_json::json!({
                                "entry": raw,
                                "kind": "github",
                                "status": "ok",
                                "rules_dir": p.display().to_string(),
                                "fetched": did_fetch,
                            }));
                        } else {
                            statuses.push(serde_json::json!({
                                "entry": raw,
                                "kind": "github",
                                "status": "no_rules",
                                "note": "Cloned repo does not contain whetstone/rules/ or rules/.",
                                "clone_path": dest.display().to_string(),
                            }));
                        }
                    }
                    Err(e) => {
                        statuses.push(serde_json::json!({
                            "entry": raw,
                            "kind": "github",
                            "status": "error",
                            "error": e.to_string(),
                        }));
                    }
                }
            }
            ExtendsEntry::Registry { handle } => {
                statuses.push(serde_json::json!({
                    "entry": raw,
                    "kind": "registry",
                    "status": "not_implemented",
                    "note": format!("The shared registry is not yet available; @{handle} will be resolvable once the registry ships."),
                }));
            }
            ExtendsEntry::HttpYaml { url } => {
                // Deliberately minimal: a full HTTP fetch pipeline lives in
                // `resolve::http`; we document the gap here rather than
                // silently pretending to fetch it.
                statuses.push(serde_json::json!({
                    "entry": raw,
                    "kind": "http",
                    "status": "not_implemented",
                    "note": "Single-file HTTP extends are not yet resolved; clone a repo via github.com/... instead.",
                    "url": url,
                }));
            }
            ExtendsEntry::Unknown(orig) => {
                statuses.push(serde_json::json!({
                    "entry": orig,
                    "kind": "unknown",
                    "status": "error",
                    "error": "Unrecognised extends entry. Expected whetstone:recommended, github.com/<org>/<repo>, @user/config, or https://… .",
                }));
            }
        }
    }

    Ok(TeamResolution {
        rules_dirs,
        deny,
        statuses,
    })
}

fn sync_github(owner: &str, repo: &str, dest: &Path, refresh: bool) -> Result<bool> {
    if dest.join(".git").exists() {
        if refresh {
            let status = Command::new("git")
                .args(["pull", "--ff-only", "--quiet"])
                .current_dir(dest)
                .status()
                .map_err(|e| anyhow!("git pull failed: {e}"))?;
            if !status.success() {
                return Err(anyhow!("git pull exited non-zero for {owner}/{repo}"));
            }
            return Ok(true);
        }
        return Ok(false);
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let url = format!("https://github.com/{owner}/{repo}.git");
    let status = Command::new("git")
        .args(["clone", "--depth", "1", "--quiet", &url])
        .arg(dest)
        .status()
        .map_err(|e| anyhow!("git clone failed: {e}"))?;
    if !status.success() {
        return Err(anyhow!("git clone exited non-zero for {url}"));
    }
    Ok(true)
}
