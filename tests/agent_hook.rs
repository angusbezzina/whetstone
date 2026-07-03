//! Durable gate for the PostToolUse hook adapter (whetstone-cpt): drive
//! `wh hook posttooluse` over a real stdin pipe and verify it feeds a violation
//! back as advisory context, and stays silent on a clean file.

use std::io::Write;
use std::process::{Command, Stdio};

fn run_hook(project_dir: &std::path::Path, event: &str) -> (String, i32) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args([
            "hook",
            "posttooluse",
            "--project-dir",
            project_dir.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn wh hook");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(event.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

fn setup() -> std::path::PathBuf {
    let tmp = std::env::temp_dir().join(format!(
        "wh_hook_it_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let rules = tmp.join("whetstone").join("rules").join("rust");
    std::fs::create_dir_all(&rules).unwrap();
    std::fs::write(
        rules.join("demo.yaml"),
        "source:\n  name: demo\nrules:\n  - id: demo.no-unwrap\n    severity: should\n    confidence: high\n    category: convention\n    description: Avoid bare unwrap\n    source_url: https://example.com/u\n    approved: true\n    status: approved\n    signals:\n      - id: s\n        strategy: ast\n        weight: required\n        ast_query: '((call_expression function: (field_expression field: (field_identifier) @m (#eq? @m \"unwrap\"))) @match)'\n    golden_examples:\n      - code: \"fn f(){ let _ = x.unwrap(); }\"\n        verdict: fail\n        reason: bare unwrap\n      - code: \"fn f(){ let _ = x.expect(\\\"y\\\"); }\"\n        verdict: pass\n        reason: ok\n",
    )
    .unwrap();
    std::fs::create_dir_all(tmp.join("src")).unwrap();
    std::fs::write(tmp.join("src/bad.rs"), "fn f() { let _ = y.unwrap(); }\n").unwrap();
    std::fs::write(tmp.join("src/ok.rs"), "fn f() { let _ = y.expect(\"z\"); }\n").unwrap();
    tmp
}

#[test]
fn hook_feeds_violation_back_as_advisory_context() {
    let tmp = setup();
    let event = r#"{"hook_event_name":"PostToolUse","tool_name":"Edit","tool_input":{"file_path":"src/bad.rs"}}"#;
    let (stdout, code) = run_hook(&tmp, event);
    assert_eq!(code, 0, "hook must exit 0");
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("hook emits JSON");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap_or("");
    assert!(ctx.contains("demo.no-unwrap"), "stdout: {stdout}");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn hook_is_silent_on_clean_file() {
    let tmp = setup();
    let event = r#"{"hook_event_name":"PostToolUse","tool_name":"Edit","tool_input":{"file_path":"src/ok.rs"}}"#;
    let (stdout, code) = run_hook(&tmp, event);
    assert_eq!(code, 0);
    assert!(stdout.trim().is_empty(), "expected no output, got: {stdout}");
    let _ = std::fs::remove_dir_all(&tmp);
}
