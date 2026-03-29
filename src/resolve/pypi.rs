use serde_json::Value;

use super::http::http_get_json;
use super::{content_hash, probe_llms_txt};

/// Resolve documentation for a Python package via PyPI.
pub fn resolve(name: &str, version: &str, timeout: u64) -> Value {
    let api_url = format!("https://pypi.org/pypi/{name}/json");
    let data = match http_get_json(&api_url, timeout) {
        Some(d) => d,
        None => return serde_json::json!({"error": format!("PyPI lookup failed for {name}")}),
    };

    let info = data
        .get("info")
        .cloned()
        .unwrap_or(Value::Object(Default::default()));
    let release_meta = extract_pypi_metadata(&data, version);

    // Extract docs URL from project_urls or home_page
    let docs_url = find_docs_url(&info);

    let docs_url = match docs_url {
        Some(url) => url,
        None => {
            let mut result =
                serde_json::json!({"error": format!("No documentation URL found for {name}")});
            merge_meta(&mut result, &release_meta);
            return result;
        }
    };

    // Probe for llms.txt
    let (content, llms_url, source_type) = probe_llms_txt(&docs_url, timeout);

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

fn find_docs_url(info: &Value) -> Option<String> {
    let project_urls = info.get("project_urls").and_then(|v| v.as_object());
    if let Some(urls) = project_urls {
        for key in &[
            "Documentation",
            "Docs",
            "documentation",
            "docs",
            "Homepage",
            "homepage",
            "Home",
            "home",
        ] {
            if let Some(url) = urls.get(*key).and_then(|v| v.as_str()) {
                if !url.is_empty() {
                    return Some(url.to_string());
                }
            }
        }
    }

    if let Some(url) = info.get("home_page").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }

    if let Some(url) = info.get("project_url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            return Some(url.to_string());
        }
    }

    None
}

fn extract_pypi_metadata(data: &Value, _version: &str) -> Value {
    let mut meta = serde_json::json!({});

    if let Some(info) = data.get("info") {
        if let Some(ver) = info.get("version").and_then(|v| v.as_str()) {
            meta["latest_version"] = Value::String(ver.to_string());

            // Release date from releases
            if let Some(releases) = data.get("releases").and_then(|r| r.as_object()) {
                if let Some(files) = releases.get(ver).and_then(|f| f.as_array()) {
                    if let Some(first) = files.first() {
                        if let Some(upload_time) = first.get("upload_time").and_then(|t| t.as_str())
                        {
                            meta["latest_release_date"] = Value::String(upload_time.to_string());
                        }
                    }
                }
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
