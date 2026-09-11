//! The deterministic half of map maintenance: `wh check --sweep` reports
//! every mapped feature in map order (journeys last) as proven, failed at a
//! named step, unreachable with its prerequisite, or skipped with a reason;
//! hygiene findings name the feature; a maintain pass's outcome becomes a
//! receipt that needs evidence to count.

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
        .env_remove("WH_SWEEP_NEVER_SET")
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

fn git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(root)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?}");
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

fn accept_all(root: &Path) {
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

fn feature(
    root: &Path,
    id: &str,
    area: &str,
    order: u32,
    steps: &[&str],
    extra: serde_json::Value,
) {
    let mut definition = serde_json::json!({
        "type": "feature",
        "summary": format!("{id} summary"),
        "area": area,
        "sweep_order": order,
        "user_path": "Run ./check.sh.",
        "drive_steps": steps,
        "proof": "It prints all good.",
        "entry_points": ["check.sh"],
        "serves": ["mission.project"],
    });
    if let (Some(target), Some(extra)) = (definition.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    let recorded = change(
        root,
        &format!("{id}-f"),
        &[
            "--kind",
            "feature",
            "--record-id",
            id,
            "--content",
            id,
            "--rationale",
            "mapped",
            "--definition",
            &definition.to_string(),
        ],
    );
    assert_eq!(recorded["state"], "success", "{recorded}");
}

fn drive_gate(root: &Path, feature: &str) {
    let gate = format!("standard.{}", feature.trim_start_matches("feature."));
    let definition = serde_json::json!({"type": "standard", "strength": "must", "enforcement": {"enforcement": "drive", "feature": feature}});
    let recorded = change(
        root,
        &format!("{feature}-g"),
        &[
            "--kind",
            "standard",
            "--record-id",
            &gate,
            "--content",
            &format!("{feature} is proven by driving it"),
            "--rationale",
            "proof",
            "--definition",
            &definition.to_string(),
        ],
    );
    assert_eq!(recorded["state"], "success", "{recorded}");
}

fn establish(root: &Path) {
    git(root, &["init", "-q"]);
    fs::write(
        root.join("check.sh"),
        "#!/bin/sh\necho \"all good\"\nexit 0\n",
    )
    .expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(root.join("check.sh"), fs::Permissions::from_mode(0o755))
            .expect("chmod");
    }
    let inspected = json(&run(&["init", "--json", "--request-id", "i1"], root));
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
            "i1",
            "--expected-revision",
            "0",
            "--resume",
            &token,
            "--mission",
            "Sweep it all.",
            "--desired-outcome",
            "Nothing rots.",
            "--values",
            "Evidence.",
            "--philosophy",
            "Small.",
            "--owner",
            "Owner",
            "--initial-safeguard",
            "Git answers.",
            "--safeguard-scope",
            "repository",
            "--revision-triggers",
            "a gate fails",
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
}

#[test]
fn the_sweep_reports_every_feature_honestly_and_hygiene_names_the_feature() {
    if !node_available() {
        eprintln!("SKIP: node is required for the driver");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);

    let ok = ["run ./check.sh", "expect-output all good", "expect-exit 0"];
    feature(
        root,
        "feature.journey",
        "Journeys",
        1,
        &ok,
        serde_json::json!({}),
    );
    drive_gate(root, "feature.journey");
    feature(root, "feature.alpha", "App", 1, &ok, serde_json::json!({}));
    drive_gate(root, "feature.alpha");
    feature(
        root,
        "feature.broken",
        "App",
        2,
        &["run ./check.sh", "expect-output something else"],
        serde_json::json!({}),
    );
    drive_gate(root, "feature.broken");
    feature(
        root,
        "feature.remote",
        "App",
        3,
        &["require env:WH_SWEEP_NEVER_SET", "run ./check.sh"],
        serde_json::json!({}),
    );
    drive_gate(root, "feature.remote");
    feature(
        root,
        "feature.unmapped",
        "App",
        4,
        &[],
        serde_json::json!({"serves": ["mission.nope"], "entry_points": ["nowhere/**"]}),
    );
    accept_all(root);
    fs::create_dir_all(root.join("whetstone/verify")).expect("verify");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/verify/drive.mjs"),
        root.join("whetstone/verify/drive.mjs"),
    )
    .expect("driver");
    fs::write(
        root.join("whetstone/verify/driver.json"),
        r#"{"surface":"cli"}"#,
    )
    .expect("config");

    let swept = json(&run(
        &["check", "--json", "--sweep", "--timeout", "120"],
        root,
    ));
    let sweep = &swept["data"]["sweep"];
    let order = sweep["order"].as_array().expect("order");
    assert_eq!(
        order.last().and_then(|id| id.as_str()),
        Some("feature.journey"),
        "journeys finish the sweep: {sweep}"
    );
    assert_eq!(
        order.first().and_then(|id| id.as_str()),
        Some("feature.alpha")
    );
    let outcome = |id: &str| {
        sweep["features"]
            .as_array()
            .expect("features")
            .iter()
            .find(|row| row["feature"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id} missing: {sweep}"))
    };
    assert_eq!(outcome("feature.alpha")["outcome"], "proven");
    assert_eq!(outcome("feature.journey")["outcome"], "proven");
    let broken = outcome("feature.broken");
    assert_eq!(broken["outcome"], "failed");
    assert!(
        broken["step"].as_str().expect("step").starts_with("step 2"),
        "{broken}"
    );
    let remote = outcome("feature.remote");
    assert_eq!(remote["outcome"], "unreachable");
    assert!(remote["detail"]
        .as_str()
        .expect("detail")
        .contains("WH_SWEEP_NEVER_SET"));
    let unmapped = outcome("feature.unmapped");
    assert_eq!(unmapped["outcome"], "skipped");
    assert!(unmapped["detail"]
        .as_str()
        .expect("detail")
        .contains("no drive steps"));
    assert_eq!(
        swept["state"], "violated",
        "a failing feature fails the sweep: {}",
        swept["summary"]
    );
    assert_eq!(sweep["edits_product_code"], false);

    // Evidence of the proven features survives the driver's cleanup.
    let evidence_root = PathBuf::from(swept["data"]["evidence_root"].as_str().expect("root"));
    for locator in outcome("feature.alpha")["evidence"]
        .as_array()
        .expect("evidence")
    {
        assert!(evidence_root
            .join(locator.as_str().expect("locator"))
            .is_file());
    }

    // Hygiene: every seeded defect is named against its feature.
    let kinds = sweep["hygiene"]
        .as_array()
        .expect("hygiene")
        .iter()
        .filter(|finding| finding["feature"] == "feature.unmapped")
        .map(|finding| finding["kind"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    for kind in [
        "missing_steps",
        "missing_gate",
        "dangling_link",
        "entry_point_matches_nothing",
    ] {
        assert!(
            kinds.iter().any(|found| found == kind),
            "{kind} not found: {kinds:?}"
        );
    }
    let dash = json(&run(&["dash", "--json"], root));
    assert!(dash["data"]["current"]["attention"]
        .as_array()
        .expect("attention")
        .iter()
        .any(|item| item["kind"] == "map_hygiene" && item["focus"] == "feature.unmapped"));

    // README/file mismatches and stale frontmatter in the rendered skill.
    let wired = json(&run(
        &["init", "--json", "--action", "wire", "--host", "agents"],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");
    let skill = fs::read_dir(root.join(".agents/skills"))
        .expect("skills")
        .flatten()
        .next()
        .expect("skill")
        .path();
    fs::remove_file(skill.join("features/alpha.md")).expect("remove");
    fs::write(skill.join("features/stray.md"), "# Stray\n\nNot mapped.\n").expect("stray");
    let journey = skill.join("features/journey.md");
    let text = fs::read_to_string(&journey).expect("journey");
    fs::write(&journey, text.replace("sweep_order: 1", "sweep_order: 9")).expect("edit");
    let again = json(&run(&["dash", "--json"], root));
    let findings = again["data"]["current"]["hygiene"].to_string();
    for kind in [
        "readme_entry_without_file",
        "file_without_readme_entry",
        "frontmatter_disagrees",
    ] {
        assert!(findings.contains(kind), "{kind} missing: {findings}");
    }

    // A maintain pass outcome is a receipt; without evidence it is unknown.
    let bare = json(&run(
        &["check", "--json", "--maintain-outcome", "clean"],
        root,
    ));
    assert_eq!(bare["state"], "unknown", "{bare}");
    fs::write(root.join("maintain-notes.md"), "features covered: all\n").expect("notes");
    let evidenced = json(&run(
        &[
            "check",
            "--json",
            "--maintain-outcome",
            "changed",
            "--maintain-evidence",
            "maintain-notes.md",
        ],
        root,
    ));
    assert_eq!(evidenced["state"], "success", "{evidenced}");
    let blocked = json(&run(
        &[
            "check",
            "--json",
            "--maintain-outcome",
            "blocked",
            "--maintain-evidence",
            "maintain-notes.md",
        ],
        root,
    ));
    assert_eq!(blocked["state"], "unknown");
    let journal = json(&run(&["dash", "--json"], root));
    assert!(journal["data"]["changelog"]
        .as_array()
        .expect("changelog")
        .iter()
        .any(|entry| entry["title"] == "Maintain pass: changed"));
    let trail = String::from_utf8(run(&["dash", "--trail"], root).stdout).expect("utf8");
    assert!(
        trail.contains("\tmaintain\tRecorded the maintain pass outcome: changed\t"),
        "{trail}"
    );
}

fn install_cli_driver(root: &Path) {
    fs::create_dir_all(root.join("whetstone/verify")).expect("verify");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/verify/drive.mjs"),
        root.join("whetstone/verify/drive.mjs"),
    )
    .expect("driver");
    fs::write(
        root.join("whetstone/verify/driver.json"),
        r#"{"surface":"cli"}"#,
    )
    .expect("config");
}

/// A change is proven with its own assertions on top of the accepted drive,
/// every distinct check is its own receipt, and a committed change selects
/// the features it touched once a base revision is named.
#[test]
fn a_change_is_proven_with_its_own_steps_and_committed_work_selects_its_features() {
    if !node_available() {
        eprintln!("SKIP: node is required for the driver");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    let ok = ["run ./check.sh", "expect-output all good", "expect-exit 0"];
    feature(root, "feature.alpha", "App", 1, &ok, serde_json::json!({}));
    drive_gate(root, "feature.alpha");
    accept_all(root);
    install_cli_driver(root);

    let accepted_only = json(&run(
        &["check", "--json", "--feature", "feature.alpha"],
        root,
    ));
    assert_eq!(accepted_only["state"], "success", "{accepted_only}");
    assert_eq!(
        accepted_only["data"]["scanner_included"], false,
        "a feature proof is about the feature"
    );
    let initial = json(&run(
        &["check", "--json", "--rule", "standard.initial-gate"],
        root,
    ));
    assert_eq!(initial["state"], "success", "{initial}");
    assert_ne!(
        initial["data"]["gate_receipts"], accepted_only["data"]["gate_receipts"],
        "two checks of one snapshot that ran different gates are two receipts"
    );

    let with_change = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.alpha",
            "--step",
            "run ./check.sh --verbose",
            "--step",
            "expect-output all good",
        ],
        root,
    ));
    assert_eq!(with_change["state"], "success", "{with_change}");
    assert!(
        with_change["summary"]
            .to_string()
            .contains("change-specific")
            || with_change["data"]
                .to_string()
                .contains("2 change-specific step(s)"),
        "the receipt names the change steps: {with_change}"
    );
    assert_ne!(
        with_change["data"]["gate_receipts"], accepted_only["data"]["gate_receipts"],
        "the change-specific run is its own receipt"
    );
    let run_id = with_change["data"]["run_id"].as_str().expect("run id");
    let evidence_root = PathBuf::from(with_change["data"]["evidence_root"].as_str().expect("root"));
    let steps: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(evidence_root.join(run_id).join("standard_alpha.steps.json"))
            .expect("steps evidence"),
    )
    .expect("steps json");
    assert_eq!(steps["accepted_steps"], 3);
    assert_eq!(steps["change_steps"][1], "expect-output all good");

    let regressed = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.alpha",
            "--step",
            "expect-output all bad",
        ],
        root,
    ));
    assert_eq!(regressed["state"], "violated", "{regressed}");
    assert!(
        regressed["data"].to_string().contains("step 4"),
        "the failing change step is named: {regressed}"
    );
    let orphan = json(&run(
        &[
            "check",
            "--json",
            "--feature",
            "feature.alpha",
            "--feature",
            "feature.none",
            "--step",
            "expect-output all good",
        ],
        root,
    ));
    assert_eq!(orphan["state"], "unknown", "{orphan}");

    // Committed work: invisible to the working tree, selected with --base.
    let identity = [
        "-c",
        "user.name=Owner",
        "-c",
        "user.email=owner@example.invalid",
    ];
    git(root, &["add", "-A"]);
    git(root, &[&identity[..], &["commit", "-qm", "base"]].concat());
    fs::write(
        root.join("check.sh"),
        "#!/bin/sh\n# quieter\necho \"all good\"\nexit 0\n",
    )
    .expect("edit");
    git(
        root,
        &[&identity[..], &["commit", "-qam", "change"]].concat(),
    );
    let affected = |value: &serde_json::Value| {
        value["data"]["selection"]["features_affected"]
            .as_array()
            .map(|rows| rows.iter().any(|row| row["feature"] == "feature.alpha"))
            .unwrap_or(false)
    };
    let working = json(&run(&["check", "--json", "--changed", "--dry-run"], root));
    assert!(!affected(&working), "{working}");
    let committed = json(&run(
        &[
            "check",
            "--json",
            "--changed",
            "--base",
            "HEAD~1",
            "--dry-run",
        ],
        root,
    ));
    assert!(affected(&committed), "{committed}");
    assert!(committed["data"]["selection"]["changed_paths"]
        .to_string()
        .contains("check.sh"));
    let unknown_base = json(&run(
        &["check", "--json", "--changed", "--base", "no-such-branch"],
        root,
    ));
    assert_eq!(unknown_base["state"], "unknown", "{unknown_base}");
}

/// The changelog search finds what users see: a journal title the
/// projection builds, not only text stored inside records.
#[test]
fn the_changelog_search_finds_visible_titles() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    let everything = json(&run(&["dash", "--json"], root));
    let titles = everything["data"]["changelog"]
        .as_array()
        .expect("changelog")
        .iter()
        .map(|entry| entry["title"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(
        titles
            .iter()
            .any(|title| title == "Foundations established"),
        "{titles:?}"
    );
    assert_eq!(everything["data"]["changelog_truncated"], false);
    let found = json(&run(
        &["dash", "--json", "--search", "foundations ESTABLISHED"],
        root,
    ));
    let found = found["data"]["changelog"].as_array().expect("changelog");
    assert!(
        found
            .iter()
            .any(|entry| entry["title"] == "Foundations established"),
        "{found:?}"
    );
    let none = json(&run(&["dash", "--json", "--search", "zqxj-nothing"], root));
    assert_eq!(none["data"]["changelog"].as_array().map(Vec::len), Some(0));
}
