//! The generated web driver proves a feature by driving a real app in a real
//! browser. The app under test is the project's own `wh dash`, launched by
//! the driver, so the proof exercises launch, doctor, CDP drive, screenshot
//! evidence and cleanup end to end.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run whetstone")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn chrome_available() -> bool {
    std::env::var_os("CHROME_BIN").is_some()
        || [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
        ]
        .iter()
        .any(|path| Path::new(path).exists())
}

fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn change(root: &Path, request: &str, args: &[&str]) -> serde_json::Value {
    let mut probe = vec!["change", "--json", "--request-id", request];
    probe.extend_from_slice(args);
    let inspected = json(&run(&probe, root));
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let mut record = probe.clone();
    record.extend_from_slice(&["--expected-revision", &revision, "--resume", &token]);
    json(&run(&record, root))
}

fn map_feature(root: &Path, id: &str, steps: &[&str]) {
    let gate = format!("standard.{}", id.trim_start_matches("feature."));
    let definition = serde_json::json!({
        "type": "feature",
        "summary": "The default dashboard view",
        "area": "Dashboard",
        "sweep_order": 1,
        "user_path": "Run wh dash and read the default view.",
        "drive_steps": steps,
        "proof": "The mission headline and the five-stage flow render.",
        "entry_points": ["assets/dashboard/"],
        "serves": ["mission.project"],
        "proven_by": [gate],
    })
    .to_string();
    let feature = change(
        root,
        &format!("{id}-feature"),
        &[
            "--kind",
            "feature",
            "--record-id",
            id,
            "--content",
            "Dashboard home",
            "--rationale",
            "The first thing a person sees",
            "--definition",
            &definition,
        ],
    );
    assert_eq!(feature["state"], "success", "{feature}");
    let drive = serde_json::json!({"type": "standard", "strength": "must", "enforcement": {"enforcement": "drive", "feature": id}}).to_string();
    let gate_record = change(
        root,
        &format!("{id}-gate"),
        &[
            "--kind",
            "standard",
            "--record-id",
            &gate,
            "--content",
            "The dashboard home is proven by driving it",
            "--rationale",
            "Proof over claims",
            "--definition",
            &drive,
        ],
    );
    assert_eq!(gate_record["state"], "success", "{gate_record}");
    let dash = json(&run(&["dash", "--json"], root));
    for entry in dash["data"]["changelog"].as_array().expect("journal") {
        if let Some(proposal) = entry["proposal"].as_str() {
            let accepted = json(&run(
                &[
                    "change",
                    "--json",
                    "--request-id",
                    &format!("accept-{proposal}"),
                    "--accept",
                    proposal,
                ],
                root,
            ));
            assert_eq!(accepted["state"], "success", "{accepted}");
        }
    }
}

#[test]
fn the_web_driver_proves_a_dashboard_feature_with_screenshots_and_locates_failures() {
    if !node_available() || !chrome_available() {
        eprintln!("SKIP: node and Chrome are required for the web driver proof");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .expect("git")
        .success());
    let inspected = json(&run(&["init", "--json", "--request-id", "web-init"], root));
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "agree",
            "--request-id",
            "web-init",
            "--expected-revision",
            "0",
            "--resume",
            &token,
            "--mission",
            "Keep project intent inspectable.",
            "--desired-outcome",
            "Routine drift is repaired before handoff.",
            "--values",
            "Evidence before assertion.",
            "--philosophy",
            "Typed services own deterministic work.",
            "--owner",
            "Owner",
            "--initial-safeguard",
            "The toolchain answers.",
            "--safeguard-scope",
            "repository",
            "--revision-triggers",
            "a gate fails twice",
            "--gate-command",
            "git --version",
        ],
        root,
    ));
    assert_eq!(
        agreed["data"]["records"].as_array().map(Vec::len),
        Some(5),
        "{agreed}"
    );

    // The project's driver launches this project's own dashboard.
    let verify = root.join("whetstone/verify");
    fs::create_dir_all(&verify).expect("verify dir");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/verify/drive.mjs"),
        verify.join("drive.mjs"),
    )
    .expect("driver");
    let config = serde_json::json!({
        "surface": "web",
        "launch": {
            "command": [bin().display().to_string(), "dash", "--no-open", "--read-only", "--project-dir", "{root}"],
            "ready": "Whetstone dashboard: (http://127\\.0\\.0\\.1:\\d+)",
            "timeout_seconds": 60
        },
        "viewport": [1280, 900]
    });
    fs::write(
        verify.join("driver.json"),
        serde_json::to_string_pretty(&config).expect("config"),
    )
    .expect("driver config");

    map_feature(
        root,
        "feature.dashboard-home",
        &[
            "open /",
            "expect #mission-line",
            "expect text=Needs attention",
            "screenshot home",
            "click #tab-foundations",
            "expect #st-gates",
            "expect-count #foundations .panel 6",
            "screenshot foundations",
        ],
    );
    let proven = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.dashboard-home",
            "--timeout",
            "120",
        ],
        root,
    ));
    assert_eq!(proven["state"], "success", "{proven}");
    let evidence_root = PathBuf::from(proven["data"]["evidence_root"].as_str().expect("root"));
    let gate = &proven["data"]["gates"][0];
    let screenshots = gate["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .filter_map(|evidence| evidence["locator"].as_str())
        .filter(|locator| locator.ends_with(".png"))
        .collect::<Vec<_>>();
    assert!(
        screenshots
            .iter()
            .any(|locator| locator.ends_with("home.png"))
            && screenshots
                .iter()
                .any(|locator| locator.ends_with("final.png")),
        "{gate}"
    );
    for locator in &screenshots {
        let bytes = fs::read(evidence_root.join(locator)).expect("screenshot survives cleanup");
        assert!(bytes.starts_with(b"\x89PNG"), "{locator} is a PNG");
    }

    // Change-specific steps extend the accepted drive in the same session,
    // including keyboard editing, and a value a field rejects fails loudly.
    let edited = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.dashboard-home",
            "--timeout",
            "120",
            "--step",
            "click #tab-changelog",
            "--step",
            "type #cl-q zzz",
            "--step",
            "press Backspace",
            "--step",
            "eval document.querySelector('#cl-q').value === 'zz'",
            "--step",
            "press Meta+A",
            "--step",
            "press Backspace",
            "--step",
            "eval document.querySelector('#cl-q').value === ''",
            "--step",
            "type #cl-q abc",
            "--step",
            "clear #cl-q",
            "--step",
            "eval document.querySelector('#cl-q').value === ''",
            "--step",
            "inspect changelog",
        ],
        root,
    ));
    assert_eq!(edited["state"], "success", "{edited}");
    assert!(edited["data"]["gates"][0]["summary"]
        .as_str()
        .expect("summary")
        .contains("11 change-specific step(s)"));
    let rejected = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.dashboard-home",
            "--timeout",
            "120",
            "--step",
            "click #tab-changelog",
            "--step",
            "type #cl-asof 010120001200A",
        ],
        root,
    ));
    assert_eq!(rejected["state"], "violated", "{rejected}");
    assert!(
        rejected["data"]["gates"][0]["failures"][0]["message"]
            .as_str()
            .expect("message")
            .contains("ISO"),
        "{rejected}"
    );

    // A proof that cannot be reached fails at the exact step.
    map_feature(
        root,
        "feature.missing-panel",
        &[
            "open /",
            "expect #mission-line",
            "expect #a-panel-that-does-not-exist",
        ],
    );
    let failing = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.missing-panel",
            "--timeout",
            "120",
        ],
        root,
    ));
    assert_eq!(failing["state"], "violated", "{failing}");
    assert_eq!(
        failing["data"]["gates"][0]["failures"][0]["location"],
        "step 3: expect #a-panel-that-does-not-exist"
    );
    assert!(
        failing["data"]["gates"][0]["evidence"]
            .as_array()
            .expect("evidence")
            .iter()
            .any(|evidence| evidence["locator"]
                .as_str()
                .is_some_and(|locator| locator.ends_with("final.png"))),
        "a failing drive still captures its final state"
    );
}

/// The seed map shipped for this repository (whetstone/verify/seed-map)
/// imports as drafts with its drive gates, and once accepted a sweep proves
/// every dashboard feature against the real `wh dash` service in Chrome.
#[test]
fn the_repository_seed_map_sweeps_the_real_dashboard() {
    if !node_available() || !chrome_available() {
        eprintln!("SKIP: node and Chrome are required for the seed map sweep");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    assert!(Command::new("git")
        .args(["init", "-q"])
        .current_dir(root)
        .status()
        .expect("git")
        .success());
    let inspected = json(&run(&["init", "--json", "--request-id", "seed-init"], root));
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "agree",
            "--request-id",
            "seed-init",
            "--expected-revision",
            "0",
            "--resume",
            &token,
            "--mission",
            "Keep project intent inspectable.",
            "--desired-outcome",
            "Routine drift is repaired before handoff.",
            "--values",
            "Evidence before assertion.",
            "--philosophy",
            "Typed services own deterministic work.",
            "--owner",
            "Owner",
            "--initial-safeguard",
            "The toolchain answers.",
            "--safeguard-scope",
            "repository",
            "--revision-triggers",
            "a gate fails twice",
            "--gate-command",
            "git --version",
        ],
        root,
    ));
    assert_eq!(
        agreed["data"]["records"].as_array().map(Vec::len),
        Some(5),
        "{agreed}"
    );
    let verify = root.join("whetstone/verify");
    fs::create_dir_all(&verify).expect("verify dir");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/verify/drive.mjs"),
        verify.join("drive.mjs"),
    )
    .expect("driver");
    let config = serde_json::json!({
        "surface": "web",
        "launch": {
            "command": [bin().display().to_string(), "dash", "--no-open", "--read-only", "--project-dir", "{root}"],
            "ready": "Whetstone dashboard: (http://127\\.0\\.0\\.1:\\d+)",
            "timeout_seconds": 60
        },
        "viewport": [1280, 900]
    });
    fs::write(
        verify.join("driver.json"),
        serde_json::to_string_pretty(&config).expect("config"),
    )
    .expect("driver config");
    let seed = Path::new(env!("CARGO_MANIFEST_DIR")).join("whetstone/verify/seed-map");
    let imported = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "import",
            "--from",
            seed.to_str().expect("path"),
            "--request-id",
            "seed",
        ],
        root,
    ));
    assert_eq!(imported["state"], "needs_decision", "{imported}");
    let drafts = imported["data"]["drafts"]
        .as_array()
        .expect("drafts")
        .clone();
    assert_eq!(
        drafts.len(),
        9,
        "the map, four features and four drive gates: {imported}"
    );
    for draft in &drafts {
        let proposal = draft["proposal"]["id"].as_str().expect("proposal");
        let accepted = json(&run(
            &[
                "change",
                "--json",
                "--request-id",
                &format!("accept-{proposal}"),
                "--accept",
                proposal,
            ],
            root,
        ));
        assert_eq!(accepted["state"], "success", "{accepted}");
    }
    let wired = json(&run(
        &["init", "--json", "--action", "wire", "--host", "agents"],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");
    let swept = json(&run(
        &["check", "--json", "--sweep", "--timeout", "180"],
        root,
    ));
    let sweep = &swept["data"]["sweep"];
    for feature in [
        "feature.dashboard-home",
        "feature.foundations",
        "feature.checks",
        "feature.changelog",
    ] {
        let row = sweep["features"]
            .as_array()
            .expect("features")
            .iter()
            .find(|row| row["feature"] == feature)
            .unwrap_or_else(|| panic!("{feature} missing: {sweep}"));
        assert_eq!(row["outcome"], "proven", "{feature}: {row}");
        assert!(
            row["evidence"].to_string().contains(".png"),
            "{feature} has screenshots"
        );
    }
    assert_eq!(swept["state"], "success", "{}", swept["summary"]);
}
