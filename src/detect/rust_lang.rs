use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Parse dependencies from Cargo.toml.
pub fn parse_cargo_toml(filepath: &Path, source: &str) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(filepath)?;
    let data: toml::Value = toml::from_str(&text)?;
    let mut deps = Vec::new();

    if let Some(dependencies) = data.get("dependencies").and_then(|d| d.as_table()) {
        for (name, spec) in dependencies {
            // Skip path-only dependencies (workspace-internal)
            if let Some(table) = spec.as_table() {
                if table.contains_key("path") && !table.contains_key("version") {
                    continue;
                }
            }
            let version = cargo_version(spec);
            deps.push(serde_json::json!({
                "name": name,
                "version": version,
                "language": "rust",
                "dev": false,
                "source": source,
            }));
        }
    }

    if let Some(dev_dependencies) = data.get("dev-dependencies").and_then(|d| d.as_table()) {
        for (name, spec) in dev_dependencies {
            if let Some(table) = spec.as_table() {
                if table.contains_key("path") && !table.contains_key("version") {
                    continue;
                }
            }
            let version = cargo_version(spec);
            deps.push(serde_json::json!({
                "name": name,
                "version": version,
                "language": "rust",
                "dev": true,
                "source": source,
            }));
        }
    }

    Ok(deps)
}

fn cargo_version(spec: &toml::Value) -> String {
    match spec {
        toml::Value::String(s) => s.clone(),
        toml::Value::Table(t) => t
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("*")
            .to_string(),
        _ => "*".to_string(),
    }
}
