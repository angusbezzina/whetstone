use std::time::Duration;

const USER_AGENT: &str = "whetstone/0.1.0 (https://github.com/whetstone)";

/// Fetch URL content. Returns None on any error.
pub fn http_get(url: &str, timeout_secs: u64) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent(USER_AGENT)
        .build()
        .ok()?;

    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().ok()
}

/// Fetch URL content, rejecting HTML responses (for llms.txt probing).
pub fn http_get_plain_text(url: &str, timeout_secs: u64) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .user_agent(USER_AGENT)
        .build()
        .ok()?;

    let resp = client.get(url).send().ok()?;
    if !resp.status().is_success() {
        return None;
    }

    // Check content-type header
    if let Some(ct) = resp.headers().get("content-type").and_then(|v| v.to_str().ok()) {
        if ct.contains("text/html") || ct.contains("application/xhtml") {
            return None;
        }
    }

    let body = resp.text().ok()?;

    // Secondary check: reject HTML-looking content
    let stripped = body.trim_start();
    let lower: String = stripped.chars().take(100).collect::<String>().to_lowercase();
    if lower.starts_with("<!doctype") || lower.starts_with("<html") {
        return None;
    }

    Some(body)
}

/// Fetch URL and parse as JSON.
pub fn http_get_json(url: &str, timeout_secs: u64) -> Option<serde_json::Value> {
    let body = http_get(url, timeout_secs)?;
    serde_json::from_str(&body).ok()
}
