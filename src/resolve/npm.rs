use serde_json::Value;

use super::http::{http_get_html_as_text, http_get_json};
use super::{content_hash, probe_llms_txt};

/// Resolve documentation for an npm package.
pub fn resolve(name: &str, version: &str, timeout: u64) -> Value {
    let api_url = format!("https://registry.npmjs.org/{name}");
    let data = match http_get_json(&api_url, timeout) {
        Some(d) => d,
        None => return serde_json::json!({"error": format!("npm lookup failed for {name}")}),
    };

    let release_meta = extract_npm_metadata(&data, version);

    // Extract homepage
    let docs_url = data
        .get("homepage")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| {
            let repo = data.get("repository")?;
            if let Some(obj) = repo.as_object() {
                let url = obj.get("url")?.as_str()?;
                let cleaned = url
                    .replace("git+", "")
                    .replace("git://", "https://")
                    .trim_end_matches(".git")
                    .to_string();
                Some(cleaned)
            } else {
                repo.as_str().map(String::from)
            }
        });

    let docs_url = match docs_url {
        Some(url) if !url.is_empty() => url,
        _ => {
            let mut result =
                serde_json::json!({"error": format!("No documentation URL found for {name}")});
            merge_meta(&mut result, &release_meta);
            return result;
        }
    };

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

    // Tier 2: Extract README from the npm registry response (already fetched)
    if let Some(readme) = data.get("readme").and_then(|v| v.as_str()) {
        if readme.len() > 100 {
            let hash = content_hash(readme);
            let mut result = serde_json::json!({
                "docs_url": docs_url,
                "llms_txt_url": null,
                "source_type": "readme",
                "content": readme,
                "content_hash": hash,
            });
            merge_meta(&mut result, &release_meta);
            return result;
        }
    }

    // Tier 3: Fetch docs HTML and convert to text
    if let Some(text) = http_get_html_as_text(&docs_url, timeout) {
        let hash = content_hash(&text);
        let mut result = serde_json::json!({
            "docs_url": docs_url,
            "llms_txt_url": null,
            "source_type": "html_converted",
            "content": text,
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

fn extract_npm_metadata(data: &Value, _version: &str) -> Value {
    let mut meta = serde_json::json!({});

    if let Some(latest) = data
        .get("dist-tags")
        .and_then(|d| d.get("latest"))
        .and_then(|v| v.as_str())
    {
        meta["latest_version"] = Value::String(latest.to_string());

        if let Some(time) = data.get("time").and_then(|t| t.as_object()) {
            if let Some(release_date) = time.get(latest).and_then(|v| v.as_str()) {
                meta["latest_release_date"] = Value::String(release_date.to_string());
            } else if let Some(modified) = time.get("modified").and_then(|v| v.as_str()) {
                meta["latest_release_date"] = Value::String(modified.to_string());
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
