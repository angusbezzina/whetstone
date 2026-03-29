use serde_json::Value;
use std::path::PathBuf;

use super::{atomic_write, load_json, now_iso};

const MAX_ENTRIES: usize = 200;

pub struct RefreshLog {
    path: PathBuf,
    data: Value,
    loaded: bool,
}

impl RefreshLog {
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

    fn signals(&mut self) -> Vec<Value> {
        self.ensure_loaded();
        self.data
            .get("signals")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    }

    pub fn record(&mut self, signal_type: &str, target: &str, detail: &str) {
        self.ensure_loaded();
        let signal = serde_json::json!({
            "timestamp": now_iso(),
            "type": signal_type,
            "target": target,
            "detail": detail,
        });

        let signals = self.data.get_mut("signals").and_then(|v| v.as_array_mut());

        match signals {
            Some(arr) => {
                arr.push(signal);
                if arr.len() > MAX_ENTRIES {
                    let start = arr.len() - MAX_ENTRIES;
                    *arr = arr[start..].to_vec();
                }
            }
            None => {
                self.data["signals"] = serde_json::json!([signal]);
            }
        }
    }

    pub fn recent(&mut self, n: usize) -> Vec<Value> {
        let signals = self.signals();
        let start = signals.len().saturating_sub(n);
        signals[start..].to_vec()
    }
}
