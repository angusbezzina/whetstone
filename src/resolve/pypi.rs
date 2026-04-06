use serde_json::Value;

use super::changelog::probe_github_changelog;
use super::http::{http_get_html_as_text, http_get_json};
use super::{build_sections, content_hash, probe_llms_txt};

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

    // Extract repository URL for changelog probing
    let repo_url = find_repo_url(&info);

    let changelog_section = repo_url
        .as_deref()
        .and_then(|url| probe_github_changelog(url, timeout));

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
        result["sections"] = build_sections(&result, changelog_section);
        merge_meta(&mut result, &release_meta);
        return result;
    }

    // Tier 2: Extract description from PyPI response (often the README)
    if let Some(description) = info.get("description").and_then(|v| v.as_str()) {
        if description.len() > 100 {
            let hash = content_hash(description);
            let mut result = serde_json::json!({
                "docs_url": docs_url,
                "llms_txt_url": null,
                "source_type": "readme",
                "content": description,
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

    // Changelog-only fallback
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

/// Extract repository URL from PyPI project_urls for changelog probing.
fn find_repo_url(info: &Value) -> Option<String> {
    let project_urls = info.get("project_urls").and_then(|v| v.as_object())?;
    for key in &[
        "Repository",
        "Source",
        "Source Code",
        "GitHub",
        "Code",
        "repository",
        "source",
    ] {
        if let Some(url) = project_urls.get(*key).and_then(|v| v.as_str()) {
            if !url.is_empty() && url.contains("github.com") {
                return Some(url.to_string());
            }
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
