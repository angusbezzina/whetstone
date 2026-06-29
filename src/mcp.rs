//! Minimal MCP (Model Context Protocol) stdio server (whetstone-b5b).
//!
//! Exposes Whetstone's deterministic oracles to any MCP-capable agent (Claude
//! Code, Cursor, ...) without bespoke per-tool wiring:
//!   - `rules_query` — JIT lookup of the rules that apply to a file/dep/language
//!   - `scan`        — deterministic violation scan of source paths
//!
//! Transport: newline-delimited JSON-RPC 2.0 over stdin/stdout (the MCP stdio
//! transport). One JSON message per line; responses are single-line JSON.

use anyhow::Result;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use crate::check::{self, CheckOptions};
use crate::rules_query::{self, Detail, Filters, LayerFilter};

const PROTOCOL_VERSION: &str = "2024-11-05";

/// Run the MCP stdio loop until stdin closes.
pub fn serve(project_dir: &Path) -> Result<()> {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                write_msg(&mut out, &error_resp(Value::Null, -32700, "parse error"))?;
                continue;
            }
        };
        if let Some(resp) = handle(project_dir, &req) {
            write_msg(&mut out, &resp)?;
        }
    }
    Ok(())
}

/// Returns Some(response) for requests, None for notifications (no `id`).
fn handle(project_dir: &Path, req: &Value) -> Option<Value> {
    let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = req.get("id").cloned();
    // Notifications have no id and never get a response.
    let is_notification = id.is_none();
    let id = id.unwrap_or(Value::Null);

    match method {
        "initialize" => Some(result_resp(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "whetstone", "version": env!("CARGO_PKG_VERSION") },
            }),
        )),
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(result_resp(id, json!({}))),
        "tools/list" => Some(result_resp(id, json!({ "tools": tools_list() }))),
        "tools/call" => Some(handle_tool_call(project_dir, id, req.get("params"))),
        _ if is_notification => None,
        _ => Some(error_resp(id, -32601, &format!("method not found: {method}"))),
    }
}

fn tools_list() -> Value {
    json!([
        {
            "name": "rules_query",
            "description": "Look up the Whetstone coding rules that apply to a file, dependency, or language — deterministic, doc-cited, JIT. Call this BEFORE editing a source file and follow the returned rules (severity must = non-negotiable, should = strong preference, may = optional).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file": { "type": "string", "description": "Repo-relative path; infers language from the extension." },
                    "dep": { "type": "string", "description": "Filter to a single dependency." },
                    "lang": { "type": "string", "enum": ["python", "typescript", "rust"] },
                    "severity": { "type": "string", "enum": ["must", "should", "may"] },
                    "full": { "type": "boolean", "description": "Include signals + golden examples (default false)." }
                }
            }
        },
        {
            "name": "scan",
            "description": "Scan source paths for violations of the project's approved rules (deterministic tree-sitter / regex / lint_proxy). Use to self-check work before finishing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "paths": { "type": "array", "items": { "type": "string" }, "description": "Paths to scan (defaults to the project dir)." },
                    "lang": { "type": "string", "enum": ["python", "typescript", "rust"] }
                }
            }
        }
    ])
}

fn handle_tool_call(project_dir: &Path, id: Value, params: Option<&Value>) -> Value {
    let params = params.cloned().unwrap_or_else(|| json!({}));
    let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

    let payload = match name {
        "rules_query" => run_rules_query(project_dir, &args),
        "scan" => run_scan(project_dir, &args),
        other => return error_resp(id, -32602, &format!("unknown tool: {other}")),
    };
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".to_string());
    result_resp(
        id,
        json!({ "content": [ { "type": "text", "text": text } ], "isError": false }),
    )
}

fn run_rules_query(project_dir: &Path, args: &Value) -> Value {
    let file = args.get("file").and_then(|v| v.as_str()).map(PathBuf::from);
    let dep = args.get("dep").and_then(|v| v.as_str());
    let lang = args.get("lang").and_then(|v| v.as_str());
    let severity = args.get("severity").and_then(|v| v.as_str());
    let full = args.get("full").and_then(|v| v.as_bool()).unwrap_or(false);
    let detail = if full { Detail::Full } else { Detail::Summary };

    let filters = Filters {
        file: file.as_deref(),
        lang,
        dep,
        severity,
        layer_filter: LayerFilter::All,
    };
    let result = rules_query::query(project_dir, &filters);
    let echo = rules_query::filters_to_json(
        file.as_deref(),
        lang,
        dep,
        severity,
        LayerFilter::All,
        detail,
    );
    rules_query::to_json(&result, detail, echo)
}

fn run_scan(project_dir: &Path, args: &Value) -> Value {
    let paths: Vec<PathBuf> = args
        .get("paths")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    let scan_paths = if paths.is_empty() {
        vec![project_dir.to_path_buf()]
    } else {
        paths
    };
    let lang = args.get("lang").and_then(|v| v.as_str());
    match check::run(CheckOptions {
        project_dir,
        scan_paths: &scan_paths,
        lang_filter: lang,
        rule_filter: None,
    }) {
        Ok(v) => v,
        Err(e) => json!({ "error": e.to_string() }),
    }
}

fn result_resp(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error_resp(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn write_msg(out: &mut impl Write, v: &Value) -> Result<()> {
    let s = serde_json::to_string(v)?;
    out.write_all(s.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_returns_server_info() {
        let req = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}});
        let resp = handle(Path::new("."), &req).unwrap();
        assert_eq!(resp["result"]["serverInfo"]["name"], "whetstone");
        assert_eq!(resp["result"]["protocolVersion"], PROTOCOL_VERSION);
    }

    #[test]
    fn notifications_get_no_response() {
        let req = json!({"jsonrpc":"2.0","method":"notifications/initialized"});
        assert!(handle(Path::new("."), &req).is_none());
    }

    #[test]
    fn tools_list_exposes_both_tools() {
        let req = json!({"jsonrpc":"2.0","id":2,"method":"tools/list"});
        let resp = handle(Path::new("."), &req).unwrap();
        let names: Vec<&str> = resp["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"rules_query"));
        assert!(names.contains(&"scan"));
    }

    #[test]
    fn unknown_tool_is_an_error() {
        let req = json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nope","arguments":{}}});
        let resp = handle(Path::new("."), &req).unwrap();
        assert!(resp["error"].is_object());
    }

    #[test]
    fn rules_query_tool_returns_text_content() {
        // Against this repo: returns a JSON envelope as text content (may be 0 rules).
        let req = json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"rules_query","arguments":{"lang":"rust"}}});
        let resp = handle(Path::new("."), &req).unwrap();
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();
        assert!(parsed.get("total").is_some());
        assert!(parsed.get("rules").is_some());
    }
}
