use serde::Deserialize;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct WhetstoneConfig {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub generate: GenerateConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    /// Rule IDs to exclude from generation (preparation for layer system)
    #[serde(default)]
    pub deny: Vec<String>,
    /// Team/registry references. Parsed by `src/team.rs` and layered in
    /// alongside project rules.
    #[serde(default)]
    pub extends: Vec<String>,
}

/// Global per-user config read from `~/.whetstone/config.yaml`.
///
/// Supplies defaults that apply to every project the user runs Whetstone
/// against. Project-level `whetstone.yaml` overrides any value set here.
#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub struct GlobalConfig {
    #[serde(default)]
    pub default_languages: Vec<String>,
    #[serde(default)]
    pub default_formats: Vec<String>,
    #[serde(default)]
    pub sources: SourcesConfig,
    /// Rule ids the user wants silenced everywhere, equivalent to the
    /// project-level deny list but applied globally.
    #[serde(default)]
    pub deny: Vec<String>,
}

impl GlobalConfig {
    pub fn load() -> Self {
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            let path = home.join(".whetstone").join("config.yaml");
            if let Ok(text) = std::fs::read_to_string(&path) {
                if let Ok(cfg) = serde_yaml::from_str::<GlobalConfig>(&text) {
                    return cfg;
                }
            }
        }
        Self::default()
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct GenerateConfig {
    #[serde(default)]
    pub formats: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub custom: Vec<CustomSource>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CustomSource {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
}

impl WhetstoneConfig {
    pub fn load(project_dir: &Path) -> Self {
        let mut merged = Self::default();

        // Global defaults kick in first; project overrides pile on top.
        let global = GlobalConfig::load();
        if !global.default_formats.is_empty() {
            merged.generate.formats = global.default_formats.clone();
        }
        merged.sources.custom.extend(global.sources.custom.clone());
        merged.deny.extend(global.deny.clone());

        let candidates = [
            project_dir.join("whetstone").join("whetstone.yaml"),
            project_dir.join("whetstone.yaml"),
        ];

        for path in &candidates {
            if path.exists() {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(cfg) = serde_yaml::from_str::<WhetstoneConfig>(&text) {
                        if !cfg.generate.formats.is_empty() {
                            merged.generate.formats = cfg.generate.formats;
                        }
                        if !cfg.discovery.exclude.is_empty() {
                            merged.discovery.exclude = cfg.discovery.exclude;
                        }
                        if !cfg.discovery.include.is_empty() {
                            merged.discovery.include = cfg.discovery.include;
                        }
                        // Append, not replace: global + project denies add up.
                        merged.deny.extend(cfg.deny);
                        merged.deny.sort();
                        merged.deny.dedup();
                        merged.extends = cfg.extends;
                        merged.sources.custom.extend(cfg.sources.custom);
                        return merged;
                    }
                }
            }
        }

        merged
    }
}
