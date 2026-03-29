use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

use super::{atomic_write, load_json, now_iso};

pub const DEFAULT_TTL: u64 = 604800; // 7 days

pub struct CacheStats {
    pub hits: usize,
    pub misses: usize,
    pub stale: usize,
    pub total: usize,
}

pub struct SourceCacheStore {
    path: PathBuf,
    data: Value,
    loaded: bool,
}

impl SourceCacheStore {
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

    fn key(language: &str, name: &str, version: &str) -> String {
        format!("{language}:{name}:{version}")
    }

    fn entries_map(&mut self) -> HashMap<String, Value> {
        self.ensure_loaded();
        self.data
            .get("entries")
            .and_then(|v| v.as_object())
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default()
    }

    fn entries_mut(&mut self) -> &mut serde_json::Map<String, Value> {
        self.ensure_loaded();
        if self.data.get("entries").is_none() {
            self.data["entries"] = Value::Object(Default::default());
        }
        self.data["entries"].as_object_mut().unwrap()
    }

    pub fn get(&mut self, language: &str, name: &str, version: &str) -> Option<Value> {
        let entries = self.entries_map();
        entries.get(&Self::key(language, name, version)).cloned()
    }

    pub fn is_fresh(
        &mut self,
        language: &str,
        name: &str,
        version: &str,
        ttl_seconds: Option<u64>,
    ) -> bool {
        let entry = match self.get(language, name, version) {
            Some(e) => e,
            None => return false,
        };

        if entry
            .get("errors")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false)
        {
            return false;
        }

        let fetch_ts = match entry.get("fetch_timestamp").and_then(|v| v.as_str()) {
            Some(ts) => ts,
            None => return false,
        };

        let ttl = ttl_seconds.unwrap_or(DEFAULT_TTL);
        parse_age_seconds(fetch_ts)
            .map(|age| age < ttl as f64)
            .unwrap_or(false)
    }

    pub fn upsert(&mut self, entry: Value) {
        let language = entry.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let version = entry.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let key = Self::key(language, name, version);
        self.entries_mut().insert(key, entry);
    }

    pub fn invalidate_by_version(&mut self, language: &str, name: &str, old_version: &str) -> bool {
        let old_key = Self::key(language, name, old_version);
        let entries = self.entries_mut();
        entries.remove(&old_key).is_some()
    }

    pub fn stats(&mut self, ttl_seconds: Option<u64>) -> CacheStats {
        self.ensure_loaded();
        let ttl = ttl_seconds.unwrap_or(DEFAULT_TTL);
        let entries = self.entries_map();
        let mut hits = 0usize;
        let mut stale = 0usize;

        for entry in entries.values() {
            if entry
                .get("errors")
                .and_then(|v| v.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false)
            {
                stale += 1;
                continue;
            }
            let fetch_ts = entry.get("fetch_timestamp").and_then(|v| v.as_str());
            match fetch_ts.and_then(parse_age_seconds) {
                Some(age) if age < ttl as f64 => hits += 1,
                _ => stale += 1,
            }
        }

        CacheStats {
            hits,
            misses: 0,
            stale,
            total: entries.len(),
        }
    }

    pub fn all_entries(&mut self) -> Vec<Value> {
        self.ensure_loaded();
        self.entries_map().values().cloned().collect()
    }
}

fn parse_age_seconds(ts: &str) -> Option<f64> {
    let parsed: DateTime<Utc> = ts.parse().ok().or_else(|| {
        // Try chrono's flexible parsing
        DateTime::parse_from_rfc3339(ts)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
    })?;
    let age = Utc::now() - parsed;
    Some(age.num_seconds() as f64)
}
