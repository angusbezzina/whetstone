//! Proofs that cannot be trusted are never passes. A drive proof that still
//! passes with a recorded mutation applied is hollow, and the must rule it
//! proves is not a pass in that same run; a proof the mutation breaks is
//! real. A proof that fails and then passes on its one retry is flaky:
//! quarantined, unknown, and shown as such. Mutations run in an isolated
//! worktree and leave the working tree alone.

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
        .env_remove("BEADS_DIR")
        .env("WHETSTONE_JEV_OFFLINE", "1")
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

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// Node runs the driver; without it these proofs cannot run. CI must have it.
fn node_available() -> bool {
    let found = Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success());
    assert!(
        found || std::env::var_os("CI").is_none(),
        "node is required in CI"
    );
    found
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

fn accept(root: &Path, drafted: &serde_json::Value) {
    assert_eq!(drafted["state"], "success", "{drafted}");
    let proposal = drafted["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal");
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

fn establish(root: &Path) {
    git(root, &["init", "-q"]);
    write_script(root, "#!/bin/sh\necho \"all good\"\nexit 0\n");
    let inspected = json(&run(&["init", "--json", "--request-id", "i1"], root));
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
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
            &revision,
            "--resume",
            &token,
            "--mission",
            "Proofs people can trust.",
        ],
        root,
    ));
    assert_eq!(
        agreed["data"]["records"][0]["id"], "mission.project",
        "{agreed}"
    );
}

fn install_driver(root: &Path, driver: Option<&str>) {
    let verify = root.join("whetstone/verify");
    fs::create_dir_all(&verify).expect("verify dir");
    match driver {
        Some(source) => fs::write(verify.join("drive.mjs"), source).expect("driver"),
        None => {
            fs::copy(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/verify/drive.mjs"),
                verify.join("drive.mjs"),
            )
            .expect("driver");
        }
    }
    fs::write(verify.join("driver.json"), r#"{"surface":"cli"}"#).expect("config");
}

/// A feature proven by a must drive rule `rule.<name>`.
fn map_feature(root: &Path, name: &str, steps: &[&str], mutations: serde_json::Value) {
    let feature = format!("feature.{name}");
    let rule = format!("rule.{name}");
    let definition = serde_json::json!({
        "type": "feature",
        "summary": "Runs the check",
        "area": "CLI",
        "sweep_order": 1,
        "user_path": "Run ./check.sh.",
        "drive_steps": steps,
        "proof": "It prints all good.",
        "entry_points": ["check.sh"],
        "serves": ["mission.project"],
        "proven_by": [rule],
        "mutations": mutations,
    });
    accept(
        root,
        &change(
            root,
            &format!("{name}-feature"),
            &[
                "--kind",
                "feature",
                "--record-id",
                &feature,
                "--content",
                name,
                "--rationale",
                "the core path",
                "--definition",
                &definition.to_string(),
            ],
        ),
    );
    let drive = serde_json::json!({"type": "rule", "strength": "must", "enforcer": {"kind": "drive", "feature": feature}});
    accept(
        root,
        &change(
            root,
            &format!("{name}-rule"),
            &[
                "--kind",
                "rule",
                "--record-id",
                &rule,
                "--content",
                &format!("{name} is proven by driving it"),
                "--rationale",
                "proof over claims",
                "--definition",
                &drive.to_string(),
            ],
        ),
    );
}

fn commit_all(root: &Path, message: &str) {
    git(root, &["add", "-A"]);
    git(
        root,
        &[
            "-c",
            "user.name=Owner",
            "-c",
            "user.email=owner@example.invalid",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            message,
        ],
    );
}

fn trail(root: &Path) -> String {
    let output = run(&["dash", "--trail"], root);
    assert!(output.status.success());
    String::from_utf8(output.stdout).expect("utf8")
}

fn gate<'a>(check: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    check["data"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|gate| gate["id"] == id)
        .unwrap_or_else(|| panic!("{id} did not run: {check}"))
}

#[test]
fn a_proof_that_survives_its_mutation_is_hollow_and_one_it_breaks_is_real() {
    if !node_available() {
        eprintln!("SKIP: node is required for the driver");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    install_driver(root, None);
    let mutation = serde_json::json!([{
        "path": "check.sh",
        "find": "all good",
        "replace": "all bad",
        "description": "the check says all bad",
    }]);
    // Only the exit code is checked, so changing the output survives.
    map_feature(
        root,
        "weak",
        &["run ./check.sh", "expect-exit 0"],
        mutation.clone(),
    );
    // The output is checked, so the same mutation breaks the proof.
    map_feature(
        root,
        "strong",
        &["run ./check.sh", "expect-output all good", "expect-exit 0"],
        mutation,
    );
    commit_all(root, "mapped");
    let status_before = git(root, &["status", "--porcelain"]);
    let script_before = fs::read(root.join("check.sh")).expect("script");

    let plain = json(&run(
        &["check", "--json", "--feature", "feature.weak"],
        root,
    ));
    assert_eq!(
        plain["state"], "success",
        "without --mutate the weak proof passes: {plain}"
    );

    let hollow = json(&run(
        &["check", "--json", "--feature", "feature.weak", "--mutate"],
        root,
    ));
    let mutations = hollow["data"]["mutations"].as_array().expect("mutations");
    assert_eq!(mutations.len(), 1, "{hollow}");
    assert_eq!(mutations[0]["feature"], "feature.weak");
    assert_eq!(mutations[0]["verdict"], "hollow", "{hollow}");
    assert_eq!(
        mutations[0]["proof"]["state"], "pass",
        "the mutated proof passed"
    );
    let weak = gate(&hollow, "rule.weak");
    assert_ne!(
        weak["state"], "pass",
        "a hollow proof is not a pass in the same run: {weak}"
    );
    assert!(
        weak["summary"]
            .as_str()
            .expect("summary")
            .contains("Hollow proof"),
        "{weak}"
    );
    assert_ne!(hollow["state"], "success", "{hollow}");

    let real = json(&run(
        &["check", "--json", "--feature", "feature.strong", "--mutate"],
        root,
    ));
    let mutations = real["data"]["mutations"].as_array().expect("mutations");
    assert_eq!(mutations.len(), 1, "{real}");
    assert_eq!(mutations[0]["verdict"], "killed", "{real}");
    assert_eq!(gate(&real, "rule.strong")["state"], "pass", "{real}");
    assert_eq!(real["state"], "success", "{real}");

    // The mutations touched neither the working tree nor the worktree list.
    assert_eq!(git(root, &["status", "--porcelain"]), status_before);
    assert_eq!(
        fs::read(root.join("check.sh")).expect("script"),
        script_before
    );
    let worktrees = git(root, &["worktree", "list", "--porcelain"]);
    assert_eq!(
        worktrees.matches("worktree ").count(),
        1,
        "no mutation worktree is left behind: {worktrees}"
    );

    let trail = trail(root);
    assert!(
        trail
            .lines()
            .any(|row| row.contains("Mutated feature.weak to test its proof")
                && row.ends_with("hollow: the proof still passed")),
        "{trail}"
    );
    assert!(
        trail.lines().any(
            |row| row.contains("Mutated feature.strong to test its proof")
                && row.ends_with("proof failed under mutation: real")
        ),
        "{trail}"
    );
}

/// Fails on its first run and passes on the second, counting its runs.
const FLAKY_DRIVER: &str = r#"import fs from "node:fs";
import path from "node:path";
const [command] = process.argv.slice(2);
if (command === "doctor") {
  console.log(JSON.stringify({ ok: true, detail: "fine" }));
  process.exit(0);
}
const counter = path.join(".git", "flaky-runs");
const runs = (fs.existsSync(counter) ? Number(fs.readFileSync(counter, "utf8")) : 0) + 1;
fs.writeFileSync(counter, String(runs));
fs.writeFileSync(path.join(process.env.WH_EVIDENCE_DIR, "transcript.json"), JSON.stringify({ runs }));
if (runs === 1) {
  console.log(JSON.stringify({ state: "fail", steps: [], failures: [{ location: "step 1: run ./check.sh", message: "a timing fluke" }] }));
  process.exit(1);
}
console.log(JSON.stringify({ state: "pass", steps: [{ step: "run ./check.sh", ok: true }], failures: [] }));
"#;

#[test]
fn a_proof_that_passes_only_on_retry_is_quarantined_not_passed() {
    if !node_available() {
        eprintln!("SKIP: node is required for the driver");
        return;
    }
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    install_driver(root, Some(FLAKY_DRIVER));
    map_feature(root, "flaky", &["run ./check.sh"], serde_json::json!([]));

    let checked = json(&run(
        &["check", "--json", "--feature", "feature.flaky"],
        root,
    ));
    assert_eq!(checked["state"], "unknown", "{checked}");
    let flaky = gate(&checked, "rule.flaky");
    assert_eq!(flaky["state"], "unknown", "{flaky}");
    assert!(
        flaky["summary"]
            .as_str()
            .expect("summary")
            .contains("quarantined"),
        "{flaky}"
    );
    assert!(
        flaky["evidence"]
            .as_array()
            .expect("evidence")
            .iter()
            .any(|evidence| evidence["system"] == "whetstone_quarantine"),
        "{flaky}"
    );
    assert_eq!(
        fs::read_to_string(root.join(".git/flaky-runs"))
            .expect("run count")
            .trim(),
        "2",
        "the proof ran once and was retried once"
    );

    let dash = json(&run(&["dash", "--json"], root));
    let row = dash["data"]["current"]["checks"]["gates"]
        .as_array()
        .expect("gates")
        .iter()
        .find(|row| row["id"] == "rule.flaky")
        .cloned()
        .unwrap_or_else(|| panic!("no rule.flaky row: {dash}"));
    assert_eq!(row["result"]["label"], "quarantined · flaky", "{row}");

    let trail = trail(root);
    assert!(
        trail.lines().any(|row| row.contains("Ran rule rule.flaky")
            && row.contains("quarantined as flaky (first run:")),
        "{trail}"
    );
}
