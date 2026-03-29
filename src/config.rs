use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
pub struct WhetstoneConfig {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
}

#[derive(Debug, Default, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
}

impl WhetstoneConfig {
    pub fn load(project_dir: &Path) -> Self {
        let candidates = [
            project_dir.join("whetstone").join("whetstone.yaml"),
            project_dir.join("whetstone.yaml"),
        ];

        for path in &candidates {
            if path.exists() {
                if let Ok(text) = std::fs::read_to_string(path) {
                    // Use serde_json-style parsing for YAML subset we care about
                    // (discovery.exclude/include are simple string lists)
                    if let Ok(cfg) = serde_yaml_minimal(&text) {
                        return cfg;
                    }
                }
            }
        }

        Self::default()
    }
}

/// Minimal YAML-subset parser for whetstone.yaml config.
/// Handles only the fields we need (discovery.exclude, discovery.include).
fn serde_yaml_minimal(text: &str) -> Result<WhetstoneConfig, ()> {
    let mut cfg = WhetstoneConfig::default();
    let mut in_discovery = false;
    let mut in_exclude = false;
    let mut in_include = false;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Top-level section detection
        if !line.starts_with(' ') && !line.starts_with('\t') {
            in_discovery = trimmed.starts_with("discovery:");
            in_exclude = false;
            in_include = false;
            continue;
        }

        if in_discovery {
            let stripped = trimmed;
            if stripped.starts_with("exclude:") {
                in_exclude = true;
                in_include = false;
                continue;
            }
            if stripped.starts_with("include:") {
                in_include = true;
                in_exclude = false;
                continue;
            }

            if stripped.starts_with("- ") {
                let value = stripped.trim_start_matches("- ").trim().trim_matches('"').trim_matches('\'').to_string();
                if in_exclude {
                    cfg.discovery.exclude.push(value);
                } else if in_include {
                    cfg.discovery.include.push(value);
                }
            } else {
                // New key under discovery — stop list parsing
                in_exclude = false;
                in_include = false;
            }
        }
    }

    Ok(cfg)
}
