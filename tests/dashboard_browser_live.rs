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

/// A throwaway Beads tracker in the temporary repository (never this
/// one's), so an agent can raise a hand the Requests view answers.
fn init_beads(root: &Path) -> bool {
    Command::new("bd")
        .args([
            "init",
            "--non-interactive",
            "--skip-agents",
            "--skip-hooks",
            "-p",
            "live",
            "-q",
        ])
        .current_dir(root)
        .env_remove("BEADS_DIR")
        .env_remove("BEADS_DB")
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .is_ok_and(|output| output.status.success())
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
        assert!(
            std::env::var_os("CI").is_none(),
            "a tool this test needs is missing in CI"
        );
        eprintln!("SKIP live dashboard browser test: Node.js is unavailable");
        return;
    }
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/test-dashboard-live-browser.mjs");
    let temp = tempfile::tempdir().expect("temp project");
    init_git(temp.path());
    let beads = available_program("bd") && init_beads(temp.path());
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
    let mut node = Command::new("node");
    node.arg(script)
        .env("WH_DASHBOARD_URL", url)
        .env("WH_PROJECT_ROOT", temp.path());
    if beads {
        node.env("WH_BIN", env!("CARGO_BIN_EXE_whetstone"));
    }
    let output = node.output().expect("run live browser harness");
    if output.status.code() == Some(77) {
        assert!(
            std::env::var_os("CI").is_none(),
            "the browser could not start in CI: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        return;
    }
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("PASS live dashboard"),
        "live harness did not report its full proof"
    );
    if beads {
        assert!(
            stdout.contains("request answered"),
            "a raised hand was not answered from Requests:\n{stdout}"
        );
    } else {
        eprintln!("bd is not installed; the Requests answer was not exercised");
    }
}
