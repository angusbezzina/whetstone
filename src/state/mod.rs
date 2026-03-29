pub mod cache;
pub mod inventory;
pub mod manifest;
pub mod refresh;

use std::path::{Path, PathBuf};

use cache::SourceCacheStore;
use inventory::InventoryStore;
use manifest::ManifestStore;
use refresh::RefreshLog;

/// Facade for all Whetstone state operations.
pub struct StateManager {
    pub state_dir: PathBuf,
    pub manifests: ManifestStore,
    pub inventory: InventoryStore,
    pub cache: SourceCacheStore,
    pub refresh_log: RefreshLog,
}

impl StateManager {
    pub fn new(project_dir: &Path) -> Self {
        let project = project_dir.canonicalize().unwrap_or_else(|_| project_dir.to_path_buf());
        let state_dir = project.join("whetstone").join(".state");
        Self {
            manifests: ManifestStore::new(state_dir.join("manifests.json")),
            inventory: InventoryStore::new(state_dir.join("inventory.json")),
            cache: SourceCacheStore::new(state_dir.join("source-cache.json")),
            refresh_log: RefreshLog::new(state_dir.join("refresh-log.json")),
            state_dir,
        }
    }

    pub fn ensure_dir(&self) {
        let _ = std::fs::create_dir_all(&self.state_dir);
    }

    pub fn load_all(&mut self) {
        self.manifests.load();
        self.inventory.load();
        self.cache.load();
        self.refresh_log.load();
    }

    pub fn save_all(&mut self) {
        self.ensure_dir();
        self.manifests.save();
        self.inventory.save();
        self.cache.save();
        self.refresh_log.save();
    }
}

// --- Shared helpers ---

use chrono::Utc;
use serde_json::Value;
use std::fs;
use std::io::Write;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

pub fn atomic_write(path: &Path, data: &Value) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = path.with_extension("tmp");
    if let Ok(mut f) = fs::File::create(&tmp) {
        if let Ok(s) = serde_json::to_string_pretty(data) {
            if f.write_all(s.as_bytes()).is_ok() && f.write_all(b"\n").is_ok() {
                let _ = fs::rename(&tmp, path);
                return;
            }
        }
    }
    let _ = fs::remove_file(&tmp);
}

pub fn load_json(path: &Path) -> Value {
    if !path.exists() {
        return Value::Object(Default::default());
    }
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| Value::Object(Default::default())),
        Err(_) => Value::Object(Default::default()),
    }
}
