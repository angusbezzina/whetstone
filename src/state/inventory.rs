use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use super::{atomic_write, load_json, now_iso};
use crate::types::LifecycleState;

/// Result of removing stale entries from the inventory.
pub struct StaleCleanupResult {
    pub removed: Vec<String>,
    pub protected: Vec<String>,
}

pub struct InventoryDiff {
    pub added: Vec<String>,
    pub changed: Vec<String>,
    pub removed: Vec<String>,
    pub unchanged: Vec<String>,
}

impl InventoryDiff {
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "added": self.added,
            "changed": self.changed,
            "removed": self.removed,
            "unchanged": self.unchanged,
        })
    }
}

pub struct InventoryStore {
    path: PathBuf,
    data: Value,
    loaded: bool,
}

impl InventoryStore {
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

    fn key(language: &str, name: &str) -> String {
        format!("{language}:{name}")
    }

    fn deps_map(&mut self) -> HashMap<String, Value> {
        self.ensure_loaded();
        self.data
            .get("dependencies")
            .and_then(|v| v.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    fn deps_mut(&mut self) -> &mut serde_json::Map<String, Value> {
        self.ensure_loaded();
        if self.data.get("dependencies").is_none() {
            self.data["dependencies"] = Value::Object(Default::default());
        }
        self.data["dependencies"].as_object_mut().unwrap()
    }

    pub fn get(&mut self, language: &str, name: &str) -> Option<Value> {
        let deps = self.deps_map();
        deps.get(&Self::key(language, name)).cloned()
    }

    pub fn upsert_dep(&mut self, dep: &Value) {
        let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let key = Self::key(language, name);
        let now = now_iso();

        let existing = self.deps_map().get(&key).cloned();
        let state = existing
            .as_ref()
            .and_then(|e| e.get("state").and_then(|s| s.as_str()))
            .unwrap_or("discovered");
        let first_seen = existing
            .as_ref()
            .and_then(|e| e.get("first_seen").and_then(|s| s.as_str()))
            .unwrap_or(&now)
            .to_string();
        let state_changed = existing
            .as_ref()
            .and_then(|e| e.get("state_changed_at").and_then(|s| s.as_str()))
            .unwrap_or(&now)
            .to_string();

        let entry = serde_json::json!({
            "name": name,
            "language": language,
            "version": dep.get("version").and_then(|v| v.as_str()).unwrap_or(""),
            "dev": dep.get("dev").and_then(|v| v.as_bool()).unwrap_or(false),
            "sources": dep.get("sources").cloned().unwrap_or(Value::Array(vec![])),
            "state": state,
            "first_seen": first_seen,
            "last_seen": now,
            "state_changed_at": state_changed,
        });

        self.deps_mut().insert(key, entry);
    }

    pub fn set_state(&mut self, language: &str, name: &str, state: LifecycleState) -> bool {
        let key = Self::key(language, name);
        let deps = self.deps_mut();
        if let Some(dep) = deps.get_mut(&key) {
            dep["state"] = Value::String(state.to_string());
            dep["state_changed_at"] = Value::String(now_iso());
            true
        } else {
            false
        }
    }

    pub fn bulk_upsert(&mut self, deps: &[Value]) -> InventoryDiff {
        self.ensure_loaded();
        let mut current_keys = HashSet::new();
        let mut added = Vec::new();
        let mut changed = Vec::new();
        let mut unchanged = Vec::new();

        let stored = self.deps_map();

        for dep in deps {
            let language = dep.get("language").and_then(|v| v.as_str()).unwrap_or("");
            let name = dep.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let key = Self::key(language, name);
            current_keys.insert(key.clone());

            if let Some(existing) = stored.get(&key) {
                let old_ver = existing
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let new_ver = dep.get("version").and_then(|v| v.as_str()).unwrap_or("");
                if old_ver != new_ver {
                    changed.push(key);
                } else {
                    unchanged.push(key);
                }
            } else {
                added.push(key);
            }

            self.upsert_dep(dep);
        }

        let removed: Vec<String> = stored
            .keys()
            .filter(|k| !current_keys.contains(*k))
            .cloned()
            .collect();

        InventoryDiff {
            added,
            changed,
            removed,
            unchanged,
        }
    }

    pub fn by_state(&mut self, state: &str) -> Vec<Value> {
        self.ensure_loaded();
        self.deps_map()
            .values()
            .filter(|d| d.get("state").and_then(|s| s.as_str()) == Some(state))
            .cloned()
            .collect()
    }

    pub fn all_deps(&mut self) -> Vec<Value> {
        self.ensure_loaded();
        self.deps_map().values().cloned().collect()
    }

    /// Remove a single dependency entry from the inventory.
    pub fn remove_dep(&mut self, language: &str, name: &str) -> bool {
        let key = Self::key(language, name);
        self.deps_mut().remove(&key).is_some()
    }

    /// Remove inventory entries reported as removed in a diff,
    /// unless they are in the protected set (e.g., deps with approved rules).
    /// Returns which keys were actually removed and which were protected.
    pub fn remove_stale(
        &mut self,
        diff: &InventoryDiff,
        protected_keys: &HashSet<String>,
    ) -> StaleCleanupResult {
        let mut removed = Vec::new();
        let mut protected = Vec::new();

        for key in &diff.removed {
            if protected_keys.contains(key) {
                protected.push(key.clone());
                continue;
            }
            if self.deps_mut().remove(key).is_some() {
                removed.push(key.clone());
            }
        }

        StaleCleanupResult { removed, protected }
    }

    /// Store detected totals from the last detect-deps run so that
    /// status can read them back without re-running detection.
    pub fn set_detected_totals(&mut self, totals: &Value) {
        self.ensure_loaded();
        self.data["detected_totals"] = totals.clone();
    }

    /// Read detected totals persisted by the last detect-deps run.
    pub fn get_detected_totals(&mut self) -> Option<Value> {
        self.ensure_loaded();
        self.data.get("detected_totals").cloned()
    }
}
