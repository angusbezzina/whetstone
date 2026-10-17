//! The virgil store needs no `bd`: with `whetstone/store.json` naming
//! Virgil and a PATH holding only the fake `virgil` (tests/fake_virgil.py)
//! and the basic tools, init, change, check and dash run end to end, drafts
//! and receipts land in the private file store, accepted records reach the
//! group's Records when they are accepted, `wh push` and `wh pull` are
//! refused by name (their dry runs succeed), and no `.beads` appears.

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
        .env("WHETSTONE_JEV_OFFLINE", "1")
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
    fs::create_dir_all(root.join(".fake-virgil")).expect("fake state");
    fs::write(
        root.join(".fake-virgil/state.json"),
        json!({"knobs": {"attached": [GROUP, REPOSITORY], "fail": {}, "stale_get": false, "page": 100}})
            .to_string(),
    )
    .expect("state");
    let wh = |args: &[&str]| run(args, &root, &tools, &home);

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
        "ask-before-public-api",
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
    let shared_ids = |root: &Path| -> Vec<String> {
        state(root)["agreements"]
            .as_array()
            .map(|agreements| {
                agreements
                    .iter()
                    .filter_map(|agreement| agreement["record_id"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
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

    // Drafts and receipts are files in the private store; receipts never
    // reach Records.
    let common = root.join(".git/whetstone/v1");
    let revision_files = files_under(&common)
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
                && path.components().any(|part| part.as_os_str() == "records")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'))
        })
        .collect::<Vec<_>>();
    assert!(
        revision_files
            .iter()
            .any(|path| path.to_string_lossy().contains("rule.git-still-answers")),
        "the draft is a file: {revision_files:?}"
    );
    let kinds = revision_files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .filter_map(|text| serde_json::from_str::<Value>(&text).ok())
        .map(|value| value["wh_kind"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(kinds.iter().any(|kind| kind == "receipt"), "{kinds:?}");
    assert!(state(&root)["agreements"]
        .as_array()
        .expect("agreements")
        .iter()
        .all(|agreement| agreement["kind"] != "receipt"));

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
        &["init", "--json", "--action", "store", "--store", "virgil"],
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
