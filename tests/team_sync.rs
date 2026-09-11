//! Two machines, one team remote (Beads data in the Git remote's
//! `refs/dolt/data`, no policy commit on the code branch): selected accepted
//! records push with an exact confirmed package, a fresh clone reaches them
//! with `bd bootstrap` and `wh dash`, private drafts, a canary and private
//! ancestry never leave, a changed package invalidates confirmation, and pull
//! executes and activates nothing while local drafts survive.

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

fn bd(root: &Path, args: &[&str]) -> String {
    let output = Command::new("bd")
        .args(args)
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect("bd");
    assert!(
        output.status.success(),
        "bd {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
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

fn accept(root: &Path, proposal: &str) {
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

fn agree(root: &Path) {
    let inspected = json(&run(&["init", "--json", "--request-id", "init-a"], root));
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
            "init-a",
            "--expected-revision",
            "0",
            "--resume",
            &token,
            "--mission",
            "A team tool people trust.",
            "--desired-outcome",
            "Regressions never reach users.",
            "--values",
            "Evidence before assertion.",
            "--philosophy",
            "Small, boring pieces.",
            "--owner",
            "Ada",
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
}

#[test]
fn accepted_records_travel_privately_and_pull_changes_nothing_it_should_not() {
    let temp = tempfile::tempdir().expect("temp");
    let origin = temp.path().join("origin.git");
    git(
        temp.path(),
        &["init", "-q", "--bare", origin.to_str().expect("path")],
    );
    let url = format!("git+file://{}", origin.display());

    // Machine A: a repository with the team's shared Beads database.
    let a = temp.path().join("a");
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().expect("path"),
            a.to_str().expect("path"),
        ],
    );
    git(
        &a,
        &[
            "-c",
            "user.email=a@a",
            "-c",
            "user.name=Ada",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    git(&a, &["push", "-q", "origin", "HEAD:main"]);
    let main_before = git(&a, &["rev-parse", "HEAD"]);
    bd(
        &a,
        &[
            "init",
            "--non-interactive",
            "--skip-agents",
            "--skip-hooks",
            "-p",
            "team",
            "-q",
        ],
    );
    bd(&a, &["dolt", "remote", "add", "origin", &url]);
    agree(&a);

    // A private canary draft and a withdrawn-then-redrafted value.
    let canary = change(
        &a,
        "canary",
        &[
            "--kind",
            "value",
            "--record-id",
            "value.secret",
            "--content",
            "PRIVATE-CANARY-7f3a never leaves",
            "--rationale",
            "a private experiment",
        ],
    );
    assert_eq!(canary["state"], "success", "{canary}");
    let speed = change(
        &a,
        "speed",
        &[
            "--kind",
            "value",
            "--record-id",
            "value.speed",
            "--content",
            "Fast feedback",
            "--rationale",
            "speed matters",
        ],
    );
    accept(
        &a,
        speed["data"]["proposal"]["id"].as_str().expect("proposal"),
    );
    // A feature linking to a gate that exists only as a draft.
    let gate = change(
        &a,
        "gate-draft",
        &[
            "--kind",
            "standard",
            "--record-id",
            "standard.journey",
            "--content",
            "The journey is proven",
            "--rationale",
            "proof over claims",
            "--definition",
            r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"test","command_ref":"git --version"}}"#,
        ],
    );
    assert_eq!(gate["state"], "success", "{gate}");
    let feature = change(
        &a,
        "feature",
        &[
            "--kind",
            "feature",
            "--record-id",
            "feature.journey",
            "--content",
            "Journey",
            "--rationale",
            "the core path",
            "--definition",
            r#"{"type":"feature","summary":"The core path","area":"App","user_path":"Open it.","proof":"It opens.","proven_by":["standard.journey"]}"#,
        ],
    );
    accept(
        &a,
        feature["data"]["proposal"]["id"]
            .as_str()
            .expect("proposal"),
    );

    // Review the exact package; the canary, the draft gate and anything
    // depending on it are not in it.
    let reviewed = json(&run(
        &["push", "--json", "--canary", "PRIVATE-CANARY-7f3a"],
        &a,
    ));
    assert_eq!(reviewed["state"], "needs_decision", "{reviewed}");
    let token = reviewed["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let package = reviewed["data"]["package"].to_string();
    assert!(
        package.contains("mission.project") && package.contains("value.speed"),
        "{package}"
    );
    assert!(!package.contains("value.secret"), "a draft is never shared");
    assert!(!package.contains("standard.journey"));
    let blocked = reviewed["data"]["blocked"].to_string();
    assert!(
        blocked.contains("feature.journey") && blocked.contains("standard.journey"),
        "{blocked}"
    );
    assert!(reviewed["data"]["destination"]["remotes"]
        .to_string()
        .contains("origin.git"));

    // Accepting more changes the package: the old confirmation is stale.
    accept(
        &a,
        gate["data"]["proposal"]["id"].as_str().expect("proposal"),
    );
    let stale = json(&run(&["push", "--json", "--confirm", &token], &a));
    assert_eq!(stale["state"], "stale", "{stale}");
    let token = stale["resume_token"].as_str().expect("token").to_string();
    let pushed = json(&run(
        &[
            "push",
            "--json",
            "--confirm",
            &token,
            "--canary",
            "PRIVATE-CANARY-7f3a",
        ],
        &a,
    ));
    assert_eq!(pushed["state"], "success", "{pushed}");
    assert_eq!(pushed["data"]["transmitted"], true);
    let again = json(&run(&["push", "--json"], &a));
    assert_eq!(
        again["state"], "success",
        "a replay shares nothing twice: {again}"
    );
    assert_eq!(again["data"]["records"], serde_json::json!([]));

    // An interrupted push (remote unreachable) copies once and transmits on
    // the next push without copying again.
    let late = change(
        &a,
        "late",
        &[
            "--kind",
            "value",
            "--record-id",
            "value.late",
            "--content",
            "Late but shared",
            "--rationale",
            "arrives after an outage",
        ],
    );
    accept(
        &a,
        late["data"]["proposal"]["id"].as_str().expect("proposal"),
    );
    let parked = temp.path().join("origin-offline.git");
    fs::rename(&origin, &parked).expect("take the remote offline");
    let review = json(&run(&["push", "--json", "--select", "value.late"], &a));
    let token = review["resume_token"].as_str().expect("token").to_string();
    let offline = json(&run(
        &[
            "push",
            "--json",
            "--select",
            "value.late",
            "--confirm",
            &token,
        ],
        &a,
    ));
    assert_eq!(offline["state"], "unavailable", "{offline}");
    assert_eq!(offline["data"]["transmitted"], false);
    fs::rename(&parked, &origin).expect("bring the remote back");
    let resumed = json(&run(&["push", "--json"], &a));
    assert_eq!(resumed["state"], "success", "{resumed}");
    assert_eq!(resumed["data"]["transmitted"], true);
    assert_eq!(
        resumed["data"]["records"],
        serde_json::json!([]),
        "nothing is copied twice"
    );

    // Beads data rides refs/dolt/data; the code branch gained no commit.
    let refs = git(&a, &["ls-remote", origin.to_str().expect("path")]);
    assert!(refs.contains("refs/dolt/data"), "{refs}");
    assert_eq!(
        git(&a, &["rev-parse", "origin/main"]).trim(),
        main_before.trim()
    );
    git(&a, &["fetch", "-q", "origin"]);
    assert_eq!(
        git(&a, &["rev-parse", "origin/main"]).trim(),
        main_before.trim()
    );

    // Machine B: a fresh clone reaches the records with bd bootstrap + wh dash.
    let b = temp.path().join("b");
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().expect("path"),
            b.to_str().expect("path"),
        ],
    );
    bd(&b, &["bootstrap", "--yes"]);
    let dash = json(&run(&["dash", "--json"], &b));
    assert_eq!(dash["data"]["current"]["established"], true, "{dash}");
    assert_eq!(dash["data"]["sync"]["private_store"], false);
    assert!(dash.to_string().contains("A team tool people trust."));
    assert!(dash.to_string().contains("Fast feedback"));
    assert!(
        dash.to_string().contains("Late but shared"),
        "the resumed push arrived"
    );
    let everything = bd(&b, &["list", "--json", "--all", "--limit", "0"]);
    assert!(
        !everything.contains("PRIVATE-CANARY"),
        "the canary never reached the team"
    );
    assert!(!everything.contains("value.secret"));

    // B drafts locally, then A shares a gate whose command would leave a
    // file if anything ran it; B pulls.
    let local = change(
        &b,
        "b-local",
        &[
            "--kind",
            "value",
            "--record-id",
            "value.b-only",
            "--content",
            "B's private idea",
            "--rationale",
            "mine",
        ],
    );
    assert_eq!(local["state"], "success", "{local}");
    let trap = change(
        &a,
        "trap",
        &[
            "--kind",
            "standard",
            "--record-id",
            "standard.trap",
            "--content",
            "Touch a file",
            "--rationale",
            "proves pull never executes",
            "--definition",
            r#"{"type":"standard","strength":"must","enforcement":{"enforcement":"test","command_ref":"touch PULLED-AND-EXECUTED"}}"#,
        ],
    );
    accept(
        &a,
        trap["data"]["proposal"]["id"].as_str().expect("proposal"),
    );
    let review = json(&run(&["push", "--json", "--select", "standard.trap"], &a));
    let token = review["resume_token"].as_str().expect("token").to_string();
    let shared = json(&run(
        &[
            "push",
            "--json",
            "--select",
            "standard.trap",
            "--confirm",
            &token,
        ],
        &a,
    ));
    assert_eq!(shared["state"], "success", "{shared}");

    let pulled = json(&run(&["pull", "--json"], &b));
    assert_eq!(pulled["state"], "success", "{pulled}");
    assert_eq!(pulled["data"]["executes"], false);
    assert_eq!(pulled["data"]["activates"], false);
    assert!(
        pulled["data"]["arrived"]
            .to_string()
            .contains("standard.trap"),
        "{pulled}"
    );
    assert!(!b.join("PULLED-AND-EXECUTED").exists(), "pull ran nothing");
    let after = json(&run(&["dash", "--json"], &b));
    assert!(
        after.to_string().contains("B's private idea"),
        "local drafts survive a pull"
    );
    let b_shared = bd(&b, &["list", "--json", "--all", "--limit", "0"]);
    assert!(
        !b_shared.contains("value.b-only"),
        "B's draft stays out of the shared database"
    );
    let _ = fs::remove_dir_all(temp.path().join("unused"));
}
