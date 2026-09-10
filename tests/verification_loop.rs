//! End-to-end verification loop through the real binary: agree with a first
//! gate, map a feature and a drive gate, accept both, wire the skill, prove
//! the feature, break it, and prove honesty on every unknown path.

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

fn write_script(root: &Path, body: &str) {
    let path = root.join("check.sh");
    fs::write(&path, body).expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
}

/// Two-step change: inspect for the base revision and token, then record.
fn change(root: &Path, request: &str, args: &[&str]) -> serde_json::Value {
    let mut probe = vec!["change", "--json", "--request-id", request];
    probe.extend_from_slice(args);
    let inspected = json(&run(&probe, root));
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("base revision")
        .to_string();
    let token = inspected["resume_token"]
        .as_str()
        .expect("resume token")
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

fn establish(root: &Path) {
    git(root, &["init", "-q"]);
    fs::create_dir_all(root.join("src")).expect("src");
    fs::write(root.join("src/main.rs"), "fn main() {}\n").expect("source");
    write_script(root, "#!/bin/sh\necho \"all good\"\nexit 0\n");
    git(root, &["add", "-A"]);
    git(
        root,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    );
    let inspected = json(&run(&["init", "--json", "--request-id", "init-1"], root));
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreement = [
        "init",
        "--json",
        "--action",
        "agree",
        "--request-id",
        "init-1",
        "--expected-revision",
        "0",
        "--resume",
        &token,
        "--mission",
        "Ship a tiny tool people trust.",
        "--desired-outcome",
        "Regressions never reach users.",
        "--values",
        "Evidence before assertion.",
        "--philosophy",
        "Small scripts over frameworks.",
        "--owner",
        "Owner",
        "--initial-safeguard",
        "The check script passes.",
        "--safeguard-scope",
        "repository",
        "--revision-triggers",
        "a gate fails twice",
        "--gate-command",
        "./check.sh",
    ];
    let mut dry = agreement.to_vec();
    dry.push("--dry-run");
    let preview = json(&run(&dry, root));
    assert_eq!(preview["state"], "needs_decision");
    assert_eq!(preview["data"]["records"].as_array().map(Vec::len), Some(5));
    assert!(
        !root.join(".git/whetstone").exists(),
        "a dry run must not create the private store"
    );
    let agreed = json(&run(&agreement, root));
    assert_eq!(agreed["data"]["records"].as_array().map(Vec::len), Some(5));
}

#[test]
fn first_gate_runs_without_a_shell_and_draft_gates_never_run() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    let check = json(&run(&["check", "--json"], root));
    assert_eq!(check["state"], "success", "{check}");
    assert_eq!(check["data"]["gates"][0]["id"], "standard.initial-gate");
    assert_eq!(check["data"]["gates"][0]["state"], "pass");
    assert!(!check["data"]["gates"][0]["evidence"]
        .as_array()
        .expect("evidence")
        .is_empty());

    let refused = change(
        root,
        "shell-gate",
        &[
            "--kind",
            "standard",
            "--record-id",
            "standard.piped",
            "--content",
            "Piped gate",
            "--rationale",
            "tries a shell",
            "--definition",
            r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"test","command_ref":"cargo test | tee out"}}"#,
        ],
    );
    assert_eq!(refused["state"], "needs_input");
    assert!(refused["blocking_questions"][0]
        .as_str()
        .expect("question")
        .contains("without a shell"));

    write_script(root, "#!/bin/sh\necho \"src/main.rs:1: broken\"\nexit 1\n");
    let failing = json(&run(&["check", "--json"], root));
    assert_eq!(failing["state"], "violated");
    assert_eq!(
        failing["data"]["gates"][0]["failures"][0]["location"],
        "src/main.rs:1"
    );
    let dash = json(&run(&["dash", "--json"], root));
    let top = &dash["data"]["current"]["attention"][0];
    assert_eq!(top["kind"], "agent_repair");
    assert!(top["agent_instruction"]
        .as_str()
        .expect("brief")
        .contains("recheck   wh check --rule standard.initial-gate"));
}

#[test]
fn features_are_mapped_accepted_wired_proven_and_honest_when_evidence_is_missing() {
    if !node_available() {
        eprintln!("SKIP: node is unavailable for the verification driver");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    let feature = change(
        root,
        "feature-1",
        &[
            "--kind",
            "feature",
            "--record-id",
            "feature.check-script",
            "--content",
            "Check script",
            "--rationale",
            "The core journey",
            "--definition",
            r#"{"type":"feature","summary":"Runs the check","area":"CLI","sweep_order":1,"user_path":"Run ./check.sh.","drive_steps":["run ./check.sh","expect-output all good","expect-exit 0"],"proof":"It prints all good and exits 0.","entry_points":["check.sh"],"serves":["mission.project"],"proven_by":["standard.check-journey"]}"#,
        ],
    );
    assert_eq!(feature["state"], "success", "{feature}");
    let gate = change(
        root,
        "gate-1",
        &[
            "--kind",
            "standard",
            "--record-id",
            "standard.check-journey",
            "--content",
            "The check journey is proven by driving it",
            "--rationale",
            "Proof over claims",
            "--definition",
            r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"drive","feature":"feature.check-script"}}"#,
        ],
    );
    assert_eq!(gate["state"], "success", "{gate}");

    let before = json(&run(&["dash", "--json"], root));
    assert_eq!(before["data"]["current"]["header"]["drafts"], 2);
    assert_eq!(
        before["data"]["current"]["features"][0]["lifecycle"], "draft",
        "an unaccepted feature is shown as a draft"
    );
    let unproven = json(&run(
        &["check", "--json", "--feature", "feature.check-script"],
        root,
    ));
    assert_eq!(
        unproven["state"], "unknown",
        "a draft feature cannot be proven"
    );
    accept_all(root);

    let dry = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "wire",
            "--dry-run",
            "--host",
            "agents",
        ],
        root,
    ));
    assert_eq!(dry["state"], "needs_decision");
    assert!(!root.join(".agents").exists(), "a dry run writes nothing");
    let wired = json(&run(
        &["init", "--json", "--action", "wire", "--host", "agents"],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");
    let skill = root.join(".agents/skills");
    let app_dir = fs::read_dir(&skill)
        .expect("skills")
        .flatten()
        .next()
        .expect("verify skill")
        .path();
    let skill_md = fs::read_to_string(app_dir.join("SKILL.md")).expect("SKILL.md");
    assert!(skill_md.starts_with("---\nname: verify-"));
    assert!(skill_md.contains("Ship a tiny tool people trust."));
    let feature_md = fs::read_to_string(app_dir.join("features/check-script.md")).expect("feature");
    // pstack's entry contract: frontmatter, then exactly four H2s in order.
    assert!(feature_md.starts_with("---\nrecord: \"feature.check-script\"\n"));
    whetstone::feature_map::conforms(&feature_md).expect("four pstack H2s");
    assert!(feature_md.contains("\nwhy: \"Serves: "), "{feature_md}");
    let readme = fs::read_to_string(app_dir.join("features/README.md")).expect("readme");
    for heading in [
        "## Baseline preconditions",
        "## Driving conventions",
        "## Proof and skip reporting",
        "## Feature entry contract",
        "## Features",
    ] {
        assert!(readme.contains(heading), "README misses {heading}");
    }
    assert!(readme.contains("(./check-script.md)"));
    assert!(skill_md.contains("maintain-verification-skill"));
    for vocabulary in [
        "doctor",
        "launch",
        "drive",
        "inspect",
        "screenshot",
        "cleanup",
    ] {
        assert!(
            skill_md.contains(&format!("drive.mjs {vocabulary}")),
            "{vocabulary}"
        );
    }
    assert!(root.join("whetstone/verify/drive.mjs").is_file());
    assert!(root.join("whetstone/verify/driver.json").is_file());

    let proven = json(&run(
        &["check", "--json", "--feature", "feature.check-script"],
        root,
    ));
    assert_eq!(proven["state"], "success", "{proven}");
    let evidence_root = PathBuf::from(proven["data"]["evidence_root"].as_str().expect("root"));
    for evidence in proven["data"]["gates"][0]["evidence"]
        .as_array()
        .expect("evidence")
    {
        let path = evidence_root.join(evidence["locator"].as_str().expect("locator"));
        assert!(
            path.is_file(),
            "evidence {} survives cleanup",
            path.display()
        );
    }

    // The drive receipt names what drove it: driver script and config
    // digests, the exact map revision, and the doctor verdict it followed.
    let dash = json(&run(&["dash", "--json"], root));
    let receipt = dash["data"]["changelog"]
        .as_array()
        .expect("changelog")
        .iter()
        .flat_map(|entry| entry["records"].as_array().cloned().unwrap_or_default())
        .filter_map(|item| item.get("record").cloned())
        .find(|record| {
            record["record_type"] == "verification_receipt"
                && record["record"]["subject"]["stable_id"] == "gate:standard.check-journey"
        })
        .unwrap_or_else(|| panic!("no drive receipt: {dash}"));
    let systems = receipt["record"]["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .map(|evidence| evidence["system"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    for system in [
        "whetstone_evidence",
        "whetstone_driver",
        "whetstone_driver_config",
        "whetstone_map",
        "whetstone_doctor",
        "git_head",
    ] {
        assert!(
            systems.iter().any(|found| found == system),
            "{system} missing: {systems:?}"
        );
    }
    let map = receipt["record"]["evidence"]
        .as_array()
        .expect("evidence")
        .iter()
        .find(|evidence| evidence["system"] == "whetstone_map")
        .expect("map evidence");
    assert_eq!(map["locator"], "feature.check-script@r1");

    // A committed change under the feature's entry points after its proof is
    // flagged as possible drift, naming the feature and the file.
    write_script(
        root,
        "#!/bin/sh\necho \"all good\"\necho \"extra\"\nexit 0\n",
    );
    git(root, &["add", "check.sh"]);
    git(
        root,
        &[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "tweak",
        ],
    );
    let drifted = json(&run(&["dash", "--json"], root));
    let drift = drifted["data"]["current"]["attention"]
        .as_array()
        .expect("attention")
        .iter()
        .find(|item| item["kind"] == "feature_drift")
        .cloned()
        .unwrap_or_else(|| panic!("no drift item: {drifted}"));
    assert!(drift["title"]
        .as_str()
        .expect("title")
        .contains("Check script"));
    assert!(drift["text"].as_str().expect("text").contains("check.sh"));
    assert_eq!(
        drifted["data"]["current"]["features"][0]["drift"],
        serde_json::json!(["check.sh"])
    );
    let reproven = json(&run(
        &["check", "--json", "--feature", "feature.check-script"],
        root,
    ));
    assert_eq!(reproven["state"], "success");
    let settled = json(&run(&["dash", "--json"], root));
    assert!(
        !settled["data"]["current"]["attention"]
            .as_array()
            .expect("attention")
            .iter()
            .any(|item| item["kind"] == "feature_drift"),
        "re-proving at the new commit clears the drift"
    );

    // Behaviour moves: --changed selects the feature and reports map review.
    write_script(root, "#!/bin/sh\necho \"nope\"\nexit 0\n");
    let changed = json(&run(&["check", "--json", "--changed"], root));
    assert_eq!(changed["state"], "violated", "{changed}");
    assert_eq!(
        changed["data"]["selection"]["features_affected"][0]["feature"],
        "feature.check-script"
    );

    // A driver that claims success without evidence is unknown, not a pass.
    write_script(root, "#!/bin/sh\necho \"all good\"\nexit 0\n");
    fs::write(
        root.join("whetstone/verify/drive.mjs"),
        "const a=process.argv.slice(2);console.log(JSON.stringify(a[0]==='doctor'?{ok:true,detail:'fine'}:{state:'pass',steps:[],failures:[]}));",
    )
    .expect("fake driver");
    let hollow = json(&run(
        &["check", "--json", "--feature", "feature.check-script"],
        root,
    ));
    assert_eq!(hollow["state"], "unknown", "{hollow}");
    assert!(hollow["data"]["gates"][0]["summary"]
        .as_str()
        .expect("summary")
        .contains("no evidence"));

    // A failing doctor blocks the drive entirely.
    fs::write(
        root.join("whetstone/verify/drive.mjs"),
        "console.log(JSON.stringify({ok:false,detail:'STALE BUILD'}));process.exit(1);",
    )
    .expect("unhealthy driver");
    let unhealthy = json(&run(
        &["check", "--json", "--feature", "feature.check-script"],
        root,
    ));
    assert_eq!(unhealthy["state"], "unknown");
    assert!(unhealthy["data"]["gates"][0]["summary"]
        .as_str()
        .expect("summary")
        .contains("Doctor failed"));

    // Accepting new content makes the rendered skill stale.
    let revised = change(
        root,
        "value-2",
        &[
            "--kind",
            "value",
            "--record-id",
            "value.speed",
            "--content",
            "Fast feedback beats perfect feedback.",
            "--rationale",
            "Speed matters",
        ],
    );
    assert_eq!(revised["state"], "success");
    accept_all(root);
    let stale = json(&run(&["dash", "--json"], root));
    assert_eq!(stale["data"]["current"]["skill"]["current"], false);
    assert_eq!(
        stale["data"]["current"]["skill"]["label"]["label"],
        "stale · regenerate"
    );
}
