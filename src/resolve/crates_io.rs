use serde_json::Value;

use super::changelog::probe_github_changelog;
use super::http::{http_get, http_get_html_as_text, http_get_json};
use super::{build_sections, content_hash, probe_llms_txt};

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

    // Extract repository URL for changelog probing
    let repo_url = crate_data
        .get("repository")
        .and_then(|v| v.as_str())
        .map(String::from);

    // Probe for changelog (runs alongside content tiers)
    let changelog_section = repo_url
        .as_deref()
        .and_then(|url| probe_github_changelog(url, timeout));

    // Tier 1: Probe for llms.txt
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
        result["sections"] = build_sections(&result, changelog_section);
        merge_meta(&mut result, &release_meta);
        return result;
    }

    // Tier 2: Try crates.io README endpoint
    let latest_ver = release_meta
        .get("latest_version")
        .and_then(|v| v.as_str())
        .unwrap_or(version);
    let readme_url = format!("https://crates.io/api/v1/crates/{name}/{latest_ver}/readme");
    if let Some(readme) = http_get(&readme_url, timeout) {
        if readme.len() > 100 && !readme.trim_start().to_lowercase().starts_with("<!doctype") {
            let hash = content_hash(&readme);
            let mut result = serde_json::json!({
                "docs_url": docs_url,
                "llms_txt_url": null,
                "source_type": "readme",
                "content": readme,
                "content_hash": hash,
            });
            result["sections"] = build_sections(&result, changelog_section);
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
        result["sections"] = build_sections(&result, changelog_section);
        merge_meta(&mut result, &release_meta);
        return result;
    }

    // If we have a changelog but no other content, use the changelog as primary
    if let Some(ref cl) = changelog_section {
        if let Some(cl_content) = cl.get("content").and_then(|v| v.as_str()) {
            let hash = content_hash(cl_content);
            let mut result = serde_json::json!({
                "docs_url": docs_url,
                "llms_txt_url": null,
                "source_type": "changelog",
                "content": cl_content,
                "content_hash": hash,
                "sections": [cl],
            });
            merge_meta(&mut result, &release_meta);
            return result;
        }
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
