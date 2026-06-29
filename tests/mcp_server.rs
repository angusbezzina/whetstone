//! Durable gate for the MCP stdio server (whetstone-b5b): drive `wh mcp` over a
//! real stdin/stdout pipe and verify the JSON-RPC handshake + a tools/call.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn mcp_stdio_handshake_and_tool_call() {
    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"rules_query","arguments":{"lang":"rust"}}}"#,
    ]
    .join("\n");

    let mut child = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(["mcp", "--project-dir", env!("CARGO_MANIFEST_DIR")])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn wh mcp");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("{requests}\n").as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);

    let msgs: Vec<serde_json::Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each line is JSON-RPC"))
        .collect();

    // A notification produces no response, so exactly 3 responses for 3 requests.
    assert_eq!(msgs.len(), 3, "expected 3 responses, got: {stdout}");

    let init = msgs.iter().find(|m| m["id"] == 1).unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "whetstone");

    let list = msgs.iter().find(|m| m["id"] == 2).unwrap();
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"rules_query") && names.contains(&"scan"));

    let call = msgs.iter().find(|m| m["id"] == 3).unwrap();
    let text = call["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(parsed["total"].is_number(), "rules_query payload: {text}");
}
