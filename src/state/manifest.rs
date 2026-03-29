use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{atomic_write, load_json, now_iso};

pub struct ManifestDiff {
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

impl ManifestDiff {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "changed": self.changed,
            "added": self.added,
            "removed": self.removed,
            "unchanged": self.unchanged,
        })
    }
}

pub struct ManifestStore {
    path: PathBuf,
    data: Value,
    loaded: bool,
}

impl ManifestStore {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            data: Value::Object(Default::default()),
            loaded: false,
        }
    }

    pub fn load(&mut self) {
        let raw = load_json(&self.path);
        if raw.get("version").and_then(|v| v.as_i64()) == Some(1) {
            self.data = raw;
        } else {
            self.data = serde_json::json!({"version": 1});
        }
        self.loaded = true;
    }

    pub fn save(&self) {
        let mut data = self.data.clone();
        data["version"] = serde_json::json!(1);
        data["updated_at"] = serde_json::json!(now_iso());
        atomic_write(&self.path, &data);
    }

    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.load();
        }
    }

    fn manifests_mut(&mut self) -> &mut serde_json::Map<String, Value> {
        self.ensure_loaded();
        if self.data.get("manifests").is_none() {
            self.data["manifests"] = Value::Object(Default::default());
        }
        self.data["manifests"].as_object_mut().unwrap()
    }

    fn manifests(&mut self) -> HashMap<String, Value> {
        self.ensure_loaded();
        self.data
            .get("manifests")
            .and_then(|v| v.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    pub fn upsert(&mut self, rel_path: &str, sha256: &str, workspace: &str) {
        let now = now_iso();
        let existing = self.manifests().get(rel_path).cloned();
        let first_seen = existing
            .as_ref()
            .and_then(|e| e.get("first_seen"))
            .and_then(|v| v.as_str())
            .unwrap_or(&now)
            .to_string();

        let entry = serde_json::json!({
            "path": rel_path,
            "sha256": sha256,
            "last_seen": now,
            "workspace": workspace,
            "first_seen": first_seen,
        });

        self.manifests_mut().insert(rel_path.to_string(), entry);
    }

    pub fn compare(&mut self, current: &HashMap<String, String>) -> ManifestDiff {
        let stored = self.manifests();
        let mut changed = Vec::new();
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut unchanged = Vec::new();

        for (path, sha) in current {
            if let Some(entry) = stored.get(path) {
                let stored_sha = entry.get("sha256").and_then(|v| v.as_str()).unwrap_or("");
                if stored_sha != sha {
                    changed.push(path.clone());
                } else {
                    unchanged.push(path.clone());
                }
            } else {
                added.push(path.clone());
            }
        }

        for path in stored.keys() {
            if !current.contains_key(path) {
                removed.push(path.clone());
            }
        }

        ManifestDiff {
            changed,
            added,
            removed,
            unchanged,
        }
    }

    pub fn fingerprint_file(filepath: &Path) -> String {
        let mut hasher = Sha256::new();
        if let Ok(data) = std::fs::read(filepath) {
            hasher.update(&data);
        }
        format!("{:x}", hasher.finalize())
    }
}
