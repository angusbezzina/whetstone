//! The virgil store needs no `bd`: with `whetstone/store.json` naming
//! Virgil and a PATH holding only the fake `virgil` (tests/fake_virgil.py)
//! and the basic tools, init, change, check (with the sweep) and dash run end
//! to end, drafts and receipts land in the rulebook's private scope,
//! accepted records reach its repository scope when they are accepted, `wh
//! push` and `wh pull` are refused by name (their dry runs succeed), and no
//! `.beads` appears.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

const GROUP: &str = "01J8Z0000000000000000000AD";
const REPOSITORY: &str = "01J8Z0000000000000000000RP";

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_whetstone"))
}

fn which(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

fn run(args: &[&str], cwd: &Path, path: &Path, home: &Path) -> Output {
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env("PATH", path)
        .env("HOME", home)
        .env_remove("BEADS_DIR")
        .env_remove("FAKE_VIRGIL_STATE")
        .env_remove("FAKE_VIRGIL_LOG")
        .output()
        .expect("run whetstone")
}

fn json_of(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                found.extend(files_under(&path));
            } else {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(unix)]
#[test]
fn the_virgil_store_runs_the_whole_flow_without_bd() {
    let temp = tempfile::tempdir().expect("temp");
    let tools = temp.path().join("bin");
    fs::create_dir(&tools).expect("bin");
    for program in ["git", "date", "sh", "env", "python3"] {
        let found = which(program).unwrap_or_else(|| panic!("{program} is required"));
        std::os::unix::fs::symlink(found, tools.join(program)).expect("link");
    }
    let fake = tools.join("virgil");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_virgil.py"),
        &fake,
    )
    .expect("fake");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    assert!(!tools.join("bd").exists());
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("whetstone")).expect("project");
    let git = Command::new(tools.join("git"))
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .expect("git");
    assert!(git.success());
    let root = root.canonicalize().expect("root");
    fs::write(
        root.join("whetstone/store.json"),
        "{\"store\":\"virgil\"}\n",
    )
    .expect("switch");
    fs::create_dir_all(root.join(".virgil")).expect("attachment");
    fs::write(
        root.join(".virgil/repo.toml"),
        format!("group = \"{GROUP}\"\nrepository = \"{REPOSITORY}\"\n"),
    )
    .expect("attachment");
    fs::create_dir_all(root.join(".fake-virgil")).expect("fake state");
    fs::write(
        root.join(".fake-virgil/state.json"),
        json!({"knobs": {"attached": [GROUP, REPOSITORY], "fail": {}, "page": 100}}).to_string(),
    )
    .expect("state");
    let wh = |args: &[&str]| run(args, &root, &tools, &home);

    // A test command, so the prove-it-works starter is offered.
    fs::write(root.join("package.json"), r#"{"scripts":{"test":"true"}}"#).expect("package.json");
    let inspected = json_of(&wh(&["init", "--json", "--request-id", "i1"]));
    assert!(
        !inspected.to_string().contains("brew install beads")
            && !inspected.to_string().contains("releases/download"),
        "never offers to install bd under Virgil: {inspected}"
    );
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = json_of(&wh(&[
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
        "No bd needed.",
        "--principle",
        "prove-it-works",
        "--starter",
        "prove-it-works",
    ]));
    assert_eq!(
        agreed["data"]["progress"]["agreement_complete"], true,
        "{agreed}"
    );
    assert_eq!(
        agreed["data"]["records"].as_array().map(Vec::len),
        Some(3),
        "{agreed}"
    );
    // The agreement is accepted, so it reached the group's Records.
    assert_eq!(
        agreed["data"]["shared"]["published"]
            .as_array()
            .map(Vec::len),
        Some(3),
        "{agreed}"
    );

    let change = |request_id: &str, id: &str, content: &str| {
        let definition = r#"{"type":"rule","strength":"must","enforcer":{"kind":"test","command":"git --version"}}"#;
        let base = [
            "change",
            "--json",
            "--request-id",
            request_id,
            "--kind",
            "rule",
            "--record-id",
            id,
            "--content",
            content,
            "--rationale",
            "Git answers.",
            "--definition",
            definition,
        ];
        let probe = json_of(&wh(&base));
        let revision = probe["expected_revision"]
            .as_u64()
            .expect("revision")
            .to_string();
        let resume = probe["resume_token"].as_str().expect("token").to_string();
        let mut args = base.to_vec();
        args.extend(["--expected-revision", &revision, "--resume", &resume]);
        json_of(&wh(&args))
    };
    let changed = change("c1", "rule.git-answers", "Git answers.");
    assert_eq!(changed["state"], "success", "{changed}");
    let proposal = changed["data"]["proposal"]["id"]
        .as_str()
        .expect("proposal")
        .to_string();
    let state = |root: &Path| -> Value {
        serde_json::from_str(
            &fs::read_to_string(root.join(".fake-virgil/state.json")).expect("state"),
        )
        .expect("json")
    };
    let scope_ids = |root: &Path, scope: &str| -> Vec<String> {
        state(root)["rulebook"][scope]
            .as_array()
            .map(|records| {
                records
                    .iter()
                    .filter_map(|record| record["record_id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let shared_ids = |root: &Path| scope_ids(root, "repository");
    assert!(
        scope_ids(&root, "private").contains(&"rule.git-answers".to_string()),
        "the draft is in the private scope"
    );
    assert!(
        !shared_ids(&root).contains(&"rule.git-answers".to_string()),
        "a draft never leaves the machine"
    );
    let accepted = json_of(&wh(&[
        "change",
        "--json",
        "--request-id",
        "a1",
        "--accept",
        &proposal,
    ]));
    assert_eq!(accepted["state"], "success", "{accepted}");
    let ids = shared_ids(&root);
    assert!(ids.contains(&"rule.git-answers".to_string()), "{ids:?}");
    assert!(
        ids.contains(&proposal),
        "the proposal travels with it: {ids:?}"
    );
    let pending = change("c2", "rule.git-still-answers", "Git still answers.");
    assert_eq!(pending["state"], "success", "{pending}");

    let checked = json_of(&wh(&["check", "--json", "--rule", "rule.git-answers"]));
    assert_eq!(checked["state"], "success", "{checked}");
    let swept = wh(&["check", "--sweep", "--format", "findings"]);
    assert_eq!(
        swept.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&swept.stderr)
    );
    let dash = json_of(&wh(&["dash", "--json"]));
    assert_eq!(dash["state"], "success", "{dash}");
    assert_eq!(dash["data"]["current"]["established"], true);
    assert_eq!(dash["data"]["current"]["header"]["drafts"], 1);
    assert_eq!(
        dash["data"]["store"]["kind"], "virgil",
        "{}",
        dash["data"]["store"]
    );
    assert_eq!(
        dash["data"]["store"]["attachment"],
        json!({"group": GROUP, "repository": REPOSITORY})
    );
    assert_eq!(dash["data"]["store"]["left_behind"], json!([]));

    // Drafts and receipts live in the private scope; receipts never reach
    // the repository scope, and nothing is kept as a local revision file.
    let private = state(&root)["rulebook"]["private"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        private
            .iter()
            .any(|record| record["record_id"] == "rule.git-still-answers"),
        "the draft is in the private scope"
    );
    assert!(
        private.iter().any(|record| record["kind"] == "receipt"),
        "the check's receipts are private"
    );
    assert!(state(&root)["rulebook"]["repository"]
        .as_array()
        .expect("repository scope")
        .iter()
        .all(|record| record["kind"] != "receipt"));
    let common = root.join(".git/whetstone");
    assert!(
        !files_under(&common)
            .iter()
            .any(|path| path.components().any(|part| part.as_os_str() == "records")),
        "no local revision files"
    );

    // Sync is the daemon's: refused by name, dry runs succeed.
    for workflow in ["push", "pull"] {
        let refused = wh(&[workflow, "--json"]);
        assert_ne!(refused.status.code(), Some(0), "{workflow}");
        let refused = json_of(&refused);
        assert_eq!(refused["state"], "conflict", "{refused}");
        assert_eq!(
            refused["data"]["refusal"],
            "records_sync_through_the_daemon"
        );
        assert!(
            refused["summary"]
                .as_str()
                .expect("summary")
                .contains("sync through the Virgil daemon"),
            "{refused}"
        );
        let dry = wh(&[workflow, "--json", "--dry-run"]);
        assert_eq!(dry.status.code(), Some(0), "{workflow} --dry-run");
        assert_eq!(
            json_of(&dry)["data"]["refusal"],
            "records_sync_through_the_daemon"
        );
    }

    assert!(
        !files_under(&root)
            .iter()
            .chain(files_under(&common).iter())
            .any(|path| path.components().any(|part| part.as_os_str() == ".beads")),
        "no Beads database anywhere"
    );
}

#[cfg(unix)]
#[test]
fn choosing_virgil_without_virgil_names_what_is_missing() {
    let temp = tempfile::tempdir().expect("temp");
    let tools = temp.path().join("bin");
    fs::create_dir(&tools).expect("bin");
    for program in ["git", "date", "sh", "env"] {
        let found = which(program).unwrap_or_else(|| panic!("{program} is required"));
        std::os::unix::fs::symlink(found, tools.join(program)).expect("link");
    }
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let root = temp.path().join("project");
    fs::create_dir(&root).expect("project");
    assert!(Command::new(tools.join("git"))
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .expect("git")
        .success());
    let output = run(
        &["init", "--json", "--action", "setup", "--store", "virgil"],
        &root,
        &tools,
        &home,
    );
    let refused = json_of(&output);
    assert_eq!(refused["state"], "unavailable", "{refused}");
    assert!(
        refused["summary"]
            .as_str()
            .expect("summary")
            .contains("virgil is not available"),
        "{refused}"
    );
    assert!(!root.join("whetstone/store.json").exists());
    assert!(!refused.to_string().contains("brew install beads"));
    // The read-only form names the store and what can be chosen.
    let shown = json_of(&run(
        &["init", "--json", "--action", "store"],
        &root,
        &tools,
        &home,
    ));
    assert_eq!(shown["data"]["store"]["kind"], "beads", "{shown}");
    assert_eq!(shown["data"]["proposed"], json!(["beads"]));
    // An unknown store is refused by name.
    let unknown = run(
        &["init", "--json", "--action", "store", "--store", "linear"],
        &root,
        &tools,
        &home,
    );
    assert_ne!(unknown.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("linear"));
}

#[cfg(unix)]
#[test]
fn setup_with_store_virgil_flips_an_attached_checkout_and_never_offers_bd() {
    let temp = tempfile::tempdir().expect("temp");
    let tools = temp.path().join("bin");
    fs::create_dir(&tools).expect("bin");
    for program in ["git", "date", "sh", "env", "python3"] {
        let found = which(program).unwrap_or_else(|| panic!("{program} is required"));
        std::os::unix::fs::symlink(found, tools.join(program)).expect("link");
    }
    let fake = tools.join("virgil");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_virgil.py"),
        &fake,
    )
    .expect("fake");
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let home = temp.path().join("home");
    fs::create_dir(&home).expect("home");
    let root = temp.path().join("project");
    fs::create_dir(&root).expect("project");
    assert!(Command::new(tools.join("git"))
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .expect("git")
        .success());
    let root = root.canonicalize().expect("root");
    fs::create_dir_all(root.join(".virgil")).expect("attachment");
    fs::write(
        root.join(".virgil/repo.toml"),
        format!("group = \"{GROUP}\"\nrepository = \"{REPOSITORY}\"\n"),
    )
    .expect("attachment");
    fs::create_dir_all(root.join(".fake-virgil")).expect("fake state");
    fs::write(
        root.join(".fake-virgil/state.json"),
        json!({"knobs": {"attached": [GROUP, REPOSITORY], "fail": {}, "page": 100}}).to_string(),
    )
    .expect("state");

    // A dry run names the switch and writes nothing.
    let dry = json_of(&run(
        &[
            "init",
            "--json",
            "--action",
            "setup",
            "--store",
            "virgil",
            "--dry-run",
        ],
        &root,
        &tools,
        &home,
    ));
    assert_eq!(dry["state"], "needs_decision", "{dry}");
    assert!(!root.join("whetstone/store.json").exists());

    // Nothing in Beads to move: the switch flips, and setup's tools are the
    // virgil CLI, never bd.
    let output = run(
        &["init", "--json", "--action", "setup", "--store", "virgil"],
        &root,
        &tools,
        &home,
    );
    let flipped = json_of(&output);
    assert_eq!(
        fs::read_to_string(root.join("whetstone/store.json")).expect("switch"),
        "{\n  \"store\": \"virgil\"\n}\n"
    );
    assert_eq!(flipped["data"]["store"]["written"], true, "{flipped}");
    assert_eq!(flipped["data"]["store"]["store"]["kind"], "virgil");
    let tools_seen = flipped["data"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .filter_map(|tool| tool["tool"].as_str())
        .collect::<Vec<_>>();
    assert!(tools_seen.contains(&"virgil"), "{tools_seen:?}");
    assert!(!tools_seen.contains(&"bd"), "{tools_seen:?}");
    let text = flipped.to_string();
    assert!(
        !text.contains("brew install beads") && !text.contains("releases/download"),
        "never offers to install bd under Virgil: {text}"
    );
    assert!(!root.join(".beads").exists());
}
