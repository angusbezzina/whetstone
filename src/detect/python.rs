use anyhow::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

/// Parse dependencies from pyproject.toml (PEP 621 + Poetry).
pub fn parse_pyproject_toml(filepath: &Path, source: &str) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(filepath)?;
    let data: toml::Value = toml::from_str(&text)?;
    let mut deps = Vec::new();

    // Collect workspace-internal deps to filter out
    let mut workspace_deps: HashSet<String> = HashSet::new();
    if let Some(sources) = data
        .get("tool")
        .and_then(|t| t.get("uv"))
        .and_then(|u| u.get("sources"))
        .and_then(|s| s.as_table())
    {
        for (dep_name, spec) in sources {
            if let Some(table) = spec.as_table() {
                if table.get("workspace").and_then(|w| w.as_bool()) == Some(true) {
                    workspace_deps.insert(dep_name.to_lowercase());
                }
            }
        }
    }

    // PEP 621: [project].dependencies
    if let Some(project_deps) = data
        .get("project")
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_array())
    {
        for dep_str in project_deps {
            if let Some(s) = dep_str.as_str() {
                let (name, version) = parse_pep508(s);
                if workspace_deps.contains(&name.to_lowercase()) {
                    continue;
                }
                deps.push(dep_json(&name, &version, "python", false, source));
            }
        }
    }

    // PEP 621: [project.optional-dependencies]
    if let Some(opt_deps) = data
        .get("project")
        .and_then(|p| p.get("optional-dependencies"))
        .and_then(|d| d.as_table())
    {
        for group_deps in opt_deps.values() {
            if let Some(arr) = group_deps.as_array() {
                for dep_str in arr {
                    if let Some(s) = dep_str.as_str() {
                        let (name, version) = parse_pep508(s);
                        deps.push(dep_json(&name, &version, "python", true, source));
                    }
                }
            }
        }
    }

    // Poetry: [tool.poetry.dependencies]
    if let Some(poetry_deps) = data
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, spec) in poetry_deps {
            if name.to_lowercase() == "python" {
                continue;
            }
            let version = poetry_version(spec);
            deps.push(dep_json(name, &version, "python", false, source));
        }
    }

    // Poetry: [tool.poetry.dev-dependencies]
    if let Some(dev_deps) = data
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("dev-dependencies"))
        .and_then(|d| d.as_table())
    {
        for (name, spec) in dev_deps {
            let version = poetry_version(spec);
            deps.push(dep_json(name, &version, "python", true, source));
        }
    }

    // Poetry: [tool.poetry.group.*.dependencies]
    if let Some(groups) = data
        .get("tool")
        .and_then(|t| t.get("poetry"))
        .and_then(|p| p.get("group"))
        .and_then(|g| g.as_table())
    {
        for (group_name, group_data) in groups {
            if let Some(group_deps) = group_data.get("dependencies").and_then(|d| d.as_table()) {
                let is_dev = group_name != "main";
                for (name, spec) in group_deps {
                    let version = poetry_version(spec);
                    deps.push(dep_json(name, &version, "python", is_dev, source));
                }
            }
        }
    }

    Ok(deps)
}

/// Parse dependencies from requirements.txt.
pub fn parse_requirements_txt(filepath: &Path, source: &str) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(filepath)?;
    let re = Regex::new(r"^([a-zA-Z0-9_.-]+)\s*([><=!~]+.+)?")?;
    let mut deps = Vec::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        if let Some(caps) = re.captures(line) {
            let name = caps[1].to_string();
            let mut version = caps
                .get(2)
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "*".to_string());
            // Remove environment markers
            if let Some(idx) = version.find(';') {
                version = version[..idx].trim().to_string();
            }
            deps.push(dep_json(&name, &version, "python", false, source));
        }
    }

    Ok(deps)
}

fn parse_pep508(dep_str: &str) -> (String, String) {
    let re = Regex::new(r"^([a-zA-Z0-9_.-]+)(?:\[[^\]]*\])?\s*(.*)").unwrap();
    if let Some(caps) = re.captures(dep_str.trim()) {
        let name = caps[1].trim().to_string();
        let mut version = caps
            .get(2)
            .map(|m| m.as_str().trim().trim_end_matches(';').trim().to_string())
            .unwrap_or_else(|| "*".to_string());
        if let Some(idx) = version.find(';') {
            version = version[..idx].trim().to_string();
        }
        if version.is_empty() {
            version = "*".to_string();
        }
        (name, version)
    } else {
        (dep_str.trim().to_string(), "*".to_string())
    }
}

fn poetry_version(spec: &toml::Value) -> String {
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

fn dep_json(name: &str, version: &str, language: &str, dev: bool, source: &str) -> Value {
    serde_json::json!({
        "name": name,
        "version": version,
        "language": language,
        "dev": dev,
        "source": source,
    })
}
