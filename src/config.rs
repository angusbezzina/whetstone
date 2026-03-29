use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Default, Deserialize)]
pub struct WhetstoneConfig {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub generate: GenerateConfig,
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
