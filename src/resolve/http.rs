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
    if let Some(ct) = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
    {
        if ct.contains("text/html") || ct.contains("application/xhtml") {
            return None;
        }
    }

    let body = resp.text().ok()?;

    // Secondary check: reject HTML-looking content
    let stripped = body.trim_start();
    let lower: String = stripped
        .chars()
        .take(100)
        .collect::<String>()
        .to_lowercase();
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

/// Fetch an HTML page, extract main content, and convert to clean text.
/// Returns None if the fetch fails or the page has no extractable content.
pub fn http_get_html_as_text(url: &str, timeout_secs: u64) -> Option<String> {
    let html = http_get(url, timeout_secs)?;
    let text = extract_doc_content(&html);
    if text.len() > 100 {
        Some(text)
    } else {
        None
    }
}

/// Extract the main documentation content from an HTML page, stripping
/// navigation, sidebars, footers, and scripts. Returns clean text with
/// basic structure preserved (headings, code blocks, lists).
fn extract_doc_content(html: &str) -> String {
    use scraper::{Html, Selector};

    let document = Html::parse_document(html);

    // Remove script, style, nav, footer, header elements first
    // We do this by selecting main content instead of stripping noise
    let content_selectors = [
        "main",
        "article",
        "[role=\"main\"]",
        ".body",
        ".markdown-body",
        ".content",
        "#content",
        ".document",
        ".rst-content",
    ];

    let mut content_html = String::new();
    for sel_str in &content_selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            if let Some(el) = document.select(&selector).next() {
                content_html = el.html();
                break;
            }
        }
    }

    // Fallback to body if no content selector matched
    if content_html.is_empty() {
        if let Ok(selector) = Selector::parse("body") {
            if let Some(el) = document.select(&selector).next() {
                content_html = el.html();
            }
        }
    }

    if content_html.is_empty() {
        return String::new();
    }

    // Parse the content fragment and extract structured text
    let fragment = Html::parse_fragment(&content_html);
    let mut output = String::new();
    extract_text_recursive(&fragment.root_element(), &mut output, 0);

    // Clean up excessive whitespace
    let lines: Vec<&str> = output.lines().map(|l| l.trim_end()).collect();

    // Collapse runs of 3+ blank lines to 2
    let mut result = String::new();
    let mut blank_count = 0;
    for line in &lines {
        if line.is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

/// Recursively extract text from HTML elements, preserving headings and code blocks.
fn extract_text_recursive(element: &scraper::ElementRef, output: &mut String, depth: usize) {
    use scraper::Node;

    for child in element.children() {
        match child.value() {
            Node::Text(text) => {
                let t = text.trim();
                if !t.is_empty() {
                    output.push_str(t);
                    output.push(' ');
                }
            }
            Node::Element(el) => {
                if let Some(child_ref) = scraper::ElementRef::wrap(child) {
                    let tag = el.name();

                    // Skip noise elements
                    if matches!(
                        tag,
                        "script"
                            | "style"
                            | "nav"
                            | "footer"
                            | "header"
                            | "noscript"
                            | "svg"
                            | "iframe"
                    ) {
                        continue;
                    }

                    match tag {
                        "h1" => {
                            output.push_str("\n\n# ");
                            extract_text_recursive(&child_ref, output, depth + 1);
                            output.push('\n');
                        }
                        "h2" => {
                            output.push_str("\n\n## ");
                            extract_text_recursive(&child_ref, output, depth + 1);
                            output.push('\n');
                        }
                        "h3" => {
                            output.push_str("\n\n### ");
                            extract_text_recursive(&child_ref, output, depth + 1);
                            output.push('\n');
                        }
                        "h4" | "h5" | "h6" => {
                            output.push_str("\n\n#### ");
                            extract_text_recursive(&child_ref, output, depth + 1);
                            output.push('\n');
                        }
                        "pre" | "code" => {
                            if tag == "pre" {
                                output.push_str("\n```\n");
                                // Get all text content directly
                                let code_text: String = child_ref.text().collect();
                                output.push_str(code_text.trim());
                                output.push_str("\n```\n");
                            } else if depth == 0 || !is_inside_pre(element) {
                                output.push('`');
                                let code_text: String = child_ref.text().collect();
                                output.push_str(&code_text);
                                output.push('`');
                            } else {
                                extract_text_recursive(&child_ref, output, depth + 1);
                            }
                        }
                        "p" | "div" | "section" | "article" | "main" => {
                            output.push('\n');
                            extract_text_recursive(&child_ref, output, depth + 1);
                            output.push('\n');
                        }
                        "li" => {
                            output.push_str("\n- ");
                            extract_text_recursive(&child_ref, output, depth + 1);
                        }
                        "br" => {
                            output.push('\n');
                        }
                        "a" => {
                            extract_text_recursive(&child_ref, output, depth + 1);
                        }
                        _ => {
                            extract_text_recursive(&child_ref, output, depth + 1);
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn is_inside_pre(element: &scraper::ElementRef) -> bool {
    let mut current = element.parent();
    while let Some(node) = current {
        if let Some(el) = scraper::ElementRef::wrap(node) {
            if el.value().name() == "pre" {
                return true;
            }
        }
        current = node.parent();
    }
    false
}
