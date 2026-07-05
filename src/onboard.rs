//! One-command agent onboarding (whetstone-0cj): `wh init --claude`.
//!
//! Wiring a repo for agent governance is otherwise ~5 manual steps. This chains
//! them: detect deps → import matching starter packs → generate context →
//! register the MCP server → install the SessionStart + PostToolUse hooks →
//! report what got wired.
//!
//! The individual steps are public (whetstone-if6) so the TUI onboarding wizard
//! drives the SAME code — "one state, two front doors": whichever door writes,
//! the artifacts (packs, `extends`, context, `.mcp.json`, hooks) are identical.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// The result of importing one pack (whetstone-if6).
#[derive(Debug, Clone)]
pub struct PackImport {
    pub pack_name: String,
    pub pack_path: PathBuf,
    pub extends_ref: String,
    /// True if the `extends` entry already existed (import was idempotent).
    pub already_present: bool,
}

/// Import a pack from in-memory YAML: write `whetstone/packs/<name>.yaml` and add
/// its `extends` entry idempotently. The shared primitive behind both
/// `wh init --claude` and the wizard's review-confirm (whetstone-if6).
pub fn import_pack(project_dir: &Path, pack_name: &str, yaml: &str) -> Result<PackImport> {
    let packs_dir = project_dir.join("whetstone").join("packs");
    std::fs::create_dir_all(&packs_dir)
        .with_context(|| format!("create {}", packs_dir.display()))?;
    let pack_path = packs_dir.join(format!("{pack_name}.yaml"));
    std::fs::write(&pack_path, yaml).with_context(|| format!("write {}", pack_path.display()))?;
    let extends_ref = format!("path:./whetstone/packs/{pack_name}.yaml");
    let already_present = add_extends_entry(project_dir, &extends_ref)?;
    Ok(PackImport {
        pack_name: pack_name.to_string(),
        pack_path,
        extends_ref,
        already_present,
    })
}

/// Import a pack from a file on disk (the `wh pack import` oracle, whetstone-if6).
/// Validates it parses as a RulePack, derives the pack name from the file stem,
/// and delegates to [`import_pack`].
pub fn import_pack_from_file(project_dir: &Path, file: &Path) -> Result<PackImport> {
    // Validate shape before writing anything.
    let _ = crate::config_packs::resolve_local_pack(file)
        .with_context(|| format!("{} is not a valid RulePack", file.display()))?;
    let content = std::fs::read_to_string(file)
        .with_context(|| format!("read {}", file.display()))?;
    let name = file
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("pack")
        .to_string();
    import_pack(project_dir, &name, &content)
}

/// Generate terse agent context (shared step, whetstone-if6).
pub fn generate_context_step(project_dir: &Path) -> Result<Value> {
    crate::generate_context::generate_context(project_dir, None, None, false, false, true)
}

/// Derive the onboarding checklist from real artifacts — never a stored state
/// file (whetstone-suk). The single source of truth the TUI wizard, the skill,
/// and Janitor all read to decide "how set up is this repo?". Write-free.
pub fn setup_status(project_dir: &Path) -> Value {
    // extends + setup.dismissed come straight from whetstone.yaml (no snapshot,
    // no cache writes).
    let ws_path = project_dir.join("whetstone").join("whetstone.yaml");
    let ws_doc: Value = std::fs::read_to_string(&ws_path)
        .ok()
        .and_then(|s| serde_yaml::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let extends_count = ws_doc
        .get("extends")
        .and_then(|e| e.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let dismissed = ws_doc
        .get("setup")
        .and_then(|s| s.get("dismissed"))
        .and_then(|d| d.as_bool())
        .unwrap_or(false);

    // Detected deps (manifests only, no network).
    let deps_detected = crate::detect::detect_deps(project_dir, false, &[], &[], false)
        .ok()
        .and_then(|d| d.get("dependencies").and_then(|a| a.as_array()).map(|a| a.len()))
        .unwrap_or(0);

    // Active rules via the read-only merge seam (whetstone-dva) — no writes.
    let rules_active = if crate::layers::project_is_initialized(project_dir) {
        let opts = crate::config::SnapshotOptions {
            read_only: true,
            injected_packs: Vec::new(),
        };
        crate::layers::resolve_merged_with(project_dir, None, true, true, false, &opts)
            .merged
            .len()
    } else {
        0
    };

    // Context files.
    let context_dir = project_dir.join("whetstone").join("context");
    let context_files: Vec<String> = std::fs::read_dir(&context_dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| n.ends_with(".md"))
                .collect()
        })
        .unwrap_or_default();

    // Hooks: PostToolUse entry in .claude/settings.json.
    let settings: Value = std::fs::read_to_string(project_dir.join(".claude").join("settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let hooks_installed = settings
        .get("hooks")
        .and_then(|h| h.get("PostToolUse"))
        .and_then(|p| p.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    // MCP: whetstone server in .mcp.json.
    let mcp: Value = std::fs::read_to_string(project_dir.join(".mcp.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));
    let mcp_registered = mcp
        .get("mcpServers")
        .and_then(|m| m.get("whetstone"))
        .is_some();

    // The counted milestones (each a user-completable step).
    let items = json!([
        {
            "key": "rules_active",
            "done": rules_active > 0,
            "evidence": format!("{rules_active} active rule(s)"),
            "next_command": "wh  (run the wizard) — or wh pack import <file>",
        },
        {
            "key": "context_generated",
            "done": !context_files.is_empty(),
            "evidence": context_files.join(", "),
            "next_command": "wh actions context",
        },
        {
            "key": "hooks_installed",
            "done": hooks_installed,
            "evidence": if hooks_installed { ".claude/settings.json PostToolUse" } else { "" },
            "next_command": "wh init --hooks",
        },
        {
            "key": "mcp_registered",
            "done": mcp_registered,
            "evidence": if mcp_registered { ".mcp.json mcpServers.whetstone" } else { "" },
            "next_command": "wh init --claude",
        },
    ]);
    let done = items
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["done"].as_bool().unwrap_or(false))
        .count();
    let total = items.as_array().unwrap().len();

    json!({
        "status": "ok",
        "done": done,
        "total": total,
        "complete": done == total,
        "dismissed": dismissed,
        "dependencies_detected": deps_detected,
        "packs_imported": extends_count,
        "items": items,
    })
}

/// Install the SessionStart advisory + PostToolUse enforcement hooks (shared step).
pub fn install_hooks_step(project_dir: &Path) -> Result<Value> {
    crate::triggers::install_hooks(project_dir, &crate::triggers::HookOptions::all())
}

pub fn claude(project_dir: &Path) -> Result<Value> {
    let mut wired: Vec<String> = Vec::new();

    // 1. Detect dependencies (no network — just manifests).
    let detected = crate::detect::detect_deps(project_dir, false, &[], &[], false)?;
    let deps = detected
        .get("dependencies")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    // 2. Import the bundled starter packs that match detected deps.
    let mut imported: Vec<String> = Vec::new();
    for dep in &deps {
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let lang = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(pack) = crate::corpus::for_dep(lang, name) {
            import_pack(project_dir, name, pack.yaml)?;
            imported.push(name.to_string());
        }
    }
    imported.sort();
    imported.dedup();

    // 3. Generate terse agent context.
    let context = generate_context_step(project_dir)?;

    // 4. Register the MCP server.
    register_mcp(project_dir)?;

    // 5. Install the hooks (SessionStart advisory + PostToolUse enforcement).
    let hooks = install_hooks_step(project_dir)?;

    wired.push(if imported.is_empty() {
        "no matching starter packs for detected deps — extract your own with `wh extract`".to_string()
    } else {
        // Consent asymmetry (whetstone-5ie): the agent door imports pre-verified
        // rules without a review moment, so point at the human review surface.
        format!(
            "imported {} pre-verified starter pack(s): {} — review them anytime: run `wh` (Review)",
            imported.len(),
            imported.join(", ")
        )
    });
    let context_written = context
        .get("generated")
        .and_then(|g| g.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    wired.push(if context_written {
        "generated terse agent context (whetstone/context/AGENTS.md)".to_string()
    } else {
        "no agent context generated yet (no approved rules or guidance — import a pack or run `wh extract`)".to_string()
    });
    wired.push("registered the MCP server in .mcp.json (rules_query + scan)".to_string());
    wired.push(
        "installed hooks: SessionStart advisory + PostToolUse in-session enforcement".to_string(),
    );

    Ok(json!({
        "status": "ok",
        "imported_packs": imported,
        "detected_dependencies": deps.len(),
        "context": context,
        "hooks": hooks,
        "wired": wired,
        "next_command": "Restart your Claude Code session (or reload) so the hooks + MCP server load, then edit a source file — any rule violation is fed back in the same turn.",
    }))
}

/// Add one `extends` entry to whetstone/whetstone.yaml idempotently (creating the
/// file with `version: 1` if absent). Returns true if the entry already existed.
/// Preserves any unknown keys (e.g. `setup:`) by round-tripping the whole doc.
/// Works via serde_json (valid YAML) so object/array edits stay simple.
fn add_extends_entry(project_dir: &Path, ref_str: &str) -> Result<bool> {
    let path = project_dir.join("whetstone").join("whetstone.yaml");
    let mut doc: Value = if path.exists() {
        serde_yaml::from_str(&std::fs::read_to_string(&path)?).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !doc.is_object() {
        doc = json!({});
    }
    let obj = doc.as_object_mut().unwrap();
    obj.insert("version".to_string(), json!(1));

    let extends = obj.entry("extends").or_insert_with(|| json!([]));
    if !extends.is_array() {
        *extends = json!([]);
    }
    let arr = extends.as_array_mut().unwrap();
    let already = arr
        .iter()
        .any(|e| e.get("ref").and_then(|r| r.as_str()) == Some(ref_str));
    if !already {
        arr.push(json!({ "scope": "project", "ref": ref_str }));
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_yaml::to_string(&doc).context("serialize whetstone.yaml")?;
    std::fs::write(&path, yaml)?;
    Ok(already)
}

/// Persist the onboarding "skip" decision as `setup.dismissed` in
/// whetstone.yaml — a real, reversible config key, NOT a TUI-only state file
/// (whetstone-arx). The TUI writes it only through this oracle. Round-trips the
/// whole doc so `extends`/other keys survive.
pub fn set_dismissed(project_dir: &Path, dismissed: bool) -> Result<()> {
    let path = project_dir.join("whetstone").join("whetstone.yaml");
    let mut doc: Value = if path.exists() {
        serde_yaml::from_str(&std::fs::read_to_string(&path)?).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !doc.is_object() {
        doc = json!({});
    }
    let obj = doc.as_object_mut().unwrap();
    obj.insert("version".to_string(), json!(1));
    let setup = obj.entry("setup").or_insert_with(|| json!({}));
    if !setup.is_object() {
        *setup = json!({});
    }
    setup
        .as_object_mut()
        .unwrap()
        .insert("dismissed".to_string(), json!(dismissed));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_yaml::to_string(&doc).context("serialize whetstone.yaml")?)?;
    Ok(())
}

/// Add rule ids to the project's `deny` list in whetstone.yaml idempotently —
/// the wizard's per-rule opt-out during review (whetstone-eg4). Preserves other
/// keys via a whole-doc round-trip.
pub fn add_deny(project_dir: &Path, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let path = project_dir.join("whetstone").join("whetstone.yaml");
    let mut doc: Value = if path.exists() {
        serde_yaml::from_str(&std::fs::read_to_string(&path)?).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !doc.is_object() {
        doc = json!({});
    }
    let obj = doc.as_object_mut().unwrap();
    obj.insert("version".to_string(), json!(1));
    let deny = obj.entry("deny").or_insert_with(|| json!([]));
    if !deny.is_array() {
        *deny = json!([]);
    }
    let arr = deny.as_array_mut().unwrap();
    for id in ids {
        if !arr.iter().any(|e| e.as_str() == Some(id.as_str())) {
            arr.push(json!(id));
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, serde_yaml::to_string(&doc).context("serialize whetstone.yaml")?)?;
    Ok(())
}

/// Register the Whetstone MCP server in .mcp.json, preserving any existing
/// servers. Shared step (whetstone-if6).
pub fn register_mcp(project_dir: &Path) -> Result<()> {
    let path = project_dir.join(".mcp.json");
    let mut doc: Value = if path.exists() {
        serde_json::from_str(&std::fs::read_to_string(&path)?).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !doc.is_object() {
        doc = json!({});
    }
    let servers = doc
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if !servers.is_object() {
        *servers = json!({});
    }
    servers.as_object_mut().unwrap().insert(
        "whetstone".to_string(),
        json!({ "command": "wh", "args": ["mcp", "--project-dir", "."] }),
    );

    let mut out = serde_json::to_string_pretty(&doc)?;
    out.push('\n');
    std::fs::write(&path, out)?;
    Ok(())
}
