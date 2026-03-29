use serde_json::Value;

use super::http::http_get_json;
use super::{content_hash, probe_llms_txt};

/// Resolve documentation for a Rust crate via crates.io.
pub fn resolve(name: &str, version: &str, timeout: u64) -> Value {
    let api_url = format!("https://crates.io/api/v1/crates/{name}");
    let data = match http_get_json(&api_url, timeout) {
        Some(d) => d,
        None => return serde_json::json!({"error": format!("crates.io lookup failed for {name}")}),
    };

    let release_meta = extract_crates_metadata(&data, version);
    let crate_data = data
        .get("crate")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));

    let docs_url = crate_data
        .get("documentation")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            crate_data
                .get("homepage")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .map(String::from)
        .unwrap_or_else(|| format!("https://docs.rs/{name}"));

    // Probe for llms.txt at docs.rs first
    let docsrs_url = format!("https://docs.rs/{name}/latest");
    let (content, llms_url, source_type) = probe_llms_txt(&docsrs_url, timeout);

    let (content, llms_url, source_type) = if content.is_some() {
        (content, llms_url, source_type)
    } else {
        probe_llms_txt(&docs_url, timeout)
    };

    if let Some(content) = content {
        let hash = content_hash(&content);
        let mut result = serde_json::json!({
            "docs_url": docs_url,
            "llms_txt_url": llms_url,
            "source_type": source_type,
            "content": content,
            "content_hash": hash,
        });
        merge_meta(&mut result, &release_meta);
        return result;
    }

    let mut result = serde_json::json!({
        "docs_url": docs_url,
        "llms_txt_url": null,
        "source_type": "docs_url_only",
        "content": null,
        "content_hash": null,
    });
    merge_meta(&mut result, &release_meta);
    result
}

fn extract_crates_metadata(data: &Value, _version: &str) -> Value {
    let mut meta = serde_json::json!({});

    if let Some(versions) = data.get("versions").and_then(|v| v.as_array()) {
        if let Some(first) = versions.first() {
            if let Some(num) = first.get("num").and_then(|v| v.as_str()) {
                meta["latest_version"] = Value::String(num.to_string());
            }
            if let Some(created) = first.get("created_at").and_then(|v| v.as_str()) {
                meta["latest_release_date"] = Value::String(created.to_string());
            }
        }
    }

    meta
}

fn merge_meta(result: &mut Value, meta: &Value) {
    if let (Some(r), Some(m)) = (result.as_object_mut(), meta.as_object()) {
        for (k, v) in m {
            if !v.is_null() {
                r.insert(k.clone(), v.clone());
            }
        }
    }
}
