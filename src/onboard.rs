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
        format!(
            "imported {} starter pack(s): {}",
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
