//! One-command agent onboarding (whetstone-0cj): `wh init --claude`.
//!
//! Wiring a repo for agent governance is otherwise ~5 manual steps. This chains
//! them: detect deps → import matching starter packs → generate context →
//! register the MCP server → install the SessionStart + PostToolUse hooks →
//! report what got wired.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::path::Path;

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
    let packs_dir = project_dir.join("whetstone").join("packs");
    for dep in &deps {
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let lang = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        if let Some(pack) = crate::corpus::for_dep(lang, name) {
            std::fs::create_dir_all(&packs_dir)?;
            std::fs::write(packs_dir.join(format!("{name}.yaml")), pack.yaml)?;
            imported.push(name.to_string());
        }
    }
    imported.sort();
    imported.dedup();

    // 3. Merge whetstone.yaml: version + extends for each imported pack.
    ensure_whetstone_yaml(project_dir, &imported)?;

    // 4. Generate terse agent context.
    let context = crate::generate_context::generate_context(project_dir, None, None, false, false, true)?;

    // 5. Register the MCP server.
    write_mcp_json(project_dir)?;

    // 6. Install the hooks (SessionStart advisory + PostToolUse enforcement).
    let hooks = crate::triggers::install_hooks(project_dir, &crate::triggers::HookOptions::all())?;

    wired.push(if imported.is_empty() {
        "no matching starter packs for detected deps — extract your own with `wh extract`".to_string()
    } else {
        format!(
            "imported {} starter pack(s): {}",
            imported.len(),
            imported.join(", ")
        )
    });
    wired.push("generated terse agent context (whetstone/context/AGENTS.md)".to_string());
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

/// Merge (or create) whetstone/whetstone.yaml with `version: 1` and an `extends`
/// entry per imported pack, idempotently. Works via serde_json (valid YAML) so
/// the object/array manipulation stays simple, then serializes back to YAML.
fn ensure_whetstone_yaml(project_dir: &Path, imported: &[String]) -> Result<()> {
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
    for dep in imported {
        let ref_str = format!("path:./whetstone/packs/{dep}.yaml");
        let already = arr
            .iter()
            .any(|e| e.get("ref").and_then(|r| r.as_str()) == Some(ref_str.as_str()));
        if !already {
            arr.push(json!({ "scope": "project", "ref": ref_str }));
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_yaml::to_string(&doc).context("serialize whetstone.yaml")?;
    std::fs::write(&path, yaml)?;
    Ok(())
}

/// Register the Whetstone MCP server in .mcp.json, preserving any existing servers.
fn write_mcp_json(project_dir: &Path) -> Result<()> {
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
