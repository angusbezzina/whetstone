use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use whetstone::dashboard::{DashboardHandle, DashboardMode};
use whetstone::dashboard_service::CommandDashboardBackend;

fn init_git(root: &Path) {
    let output = Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(root)
        .output()
        .expect("initialize Git fixture");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn available_program(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn real_browser_drives_the_real_dashboard_service_and_persists_results() {
    if !available_program("node") {
        eprintln!("SKIP live dashboard browser test: Node.js is unavailable");
        return;
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test-dashboard-live-browser.mjs");
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let backend = Arc::new(CommandDashboardBackend::new(
        temp.path().to_path_buf(),
        None,
    ));
    let handle = DashboardHandle::start(
        DashboardMode::Local {
            allow_mutations: true,
        },
        backend,
    )
    .expect("start live dashboard");
    let bootstrap = handle
        .take_bootstrap_fragment()
        .expect("one-time dashboard bootstrap");
    let url = format!("{}#bootstrap={bootstrap}", handle.public_url());
    let output = Command::new("node")
        .arg(script)
        .env("WH_DASHBOARD_URL", url)
        .env("WH_PROJECT_ROOT", temp.path())
        .output()
        .expect("run live browser harness");
    if output.status.code() == Some(77) {
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("PASS live dashboard"),
        "live harness did not report its full proof"
    );
}
