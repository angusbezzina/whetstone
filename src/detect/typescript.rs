use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Parse dependencies from package.json.
pub fn parse_package_json(filepath: &Path, source: &str) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(filepath)?;
    let data: Value = serde_json::from_str(&text)?;
    let mut deps = Vec::new();

    if let Some(dependencies) = data.get("dependencies").and_then(|d| d.as_object()) {
        for (name, version) in dependencies {
            if is_workspace_ref(version) {
                continue;
            }
            let ver = version.as_str().unwrap_or("*");
            deps.push(serde_json::json!({
                "name": name,
                "version": ver,
                "language": "typescript",
                "dev": false,
                "source": source,
            }));
        }
    }

    if let Some(dev_dependencies) = data.get("devDependencies").and_then(|d| d.as_object()) {
        for (name, version) in dev_dependencies {
            if is_workspace_ref(version) {
                continue;
            }
            let ver = version.as_str().unwrap_or("*");
            deps.push(serde_json::json!({
                "name": name,
                "version": ver,
                "language": "typescript",
                "dev": true,
                "source": source,
            }));
        }
    }

    Ok(deps)
}

fn is_workspace_ref(version: &Value) -> bool {
    match version.as_str() {
        Some(v) => {
            let v = v.trim().to_lowercase();
            v.starts_with("workspace:") || v.starts_with("link:") || v.starts_with("file:")
        }
        None => false,
    }
}
