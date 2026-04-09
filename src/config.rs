use serde::Deserialize;
use std::path::Path;

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
        let candidates = [
            project_dir.join("whetstone").join("whetstone.yaml"),
            project_dir.join("whetstone.yaml"),
        ];

        for path in &candidates {
            if path.exists() {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(cfg) = serde_yaml::from_str::<WhetstoneConfig>(&text) {
                        return cfg;
                    }
                }
            }
        }

        Self::default()
    }
}
