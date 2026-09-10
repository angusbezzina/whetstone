//! pstack interop through the real binary: a pstack-generated verification
//! skill imports as reviewable drafts only, renders back byte-identically
//! once accepted, re-imports without spurious drafts, returns a maintain-pass
//! edit as exactly one draft, retires a feature through review, and exports
//! the decision trail in show-me-your-work's TSV shape.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/pstack-feature-map-example"
);

const PSTACK_SKILL: &str = "---
name: verify-notes
description: \"Drive Notes in a browser and the CLI. Use before claiming a Notes change works.\"
---

# Verify Notes

## Launch

Run `npm run dev -- --port 4173` and wait for `ready on http://127.0.0.1:4173`.

## Doctor

`control-notes doctor` must report the expected URL and a disposable data directory.

## Drive

Use `control-notes browser` for the UI and `control-notes cli -- notes` for the CLI.

## Evidence

Artifacts go to `artifacts/<feature>/`.

## Cleanup

`control-notes cleanup` stops the dev server; artifacts stay.
";

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

fn establish(root: &Path) {
    git(root, &["init", "-q"]);
    let inspected = json(&run(&["init", "--json", "--request-id", "init-1"], root));
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
            "init-1",
            "--expected-revision",
            "0",
            "--resume",
            &token,
            "--mission",
            "Notes people trust.",
            "--desired-outcome",
            "Saved notes are never lost.",
            "--values",
            "Evidence before assertion.",
            "--philosophy",
            "Small, boring pieces.",
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
    assert_eq!(agreed["data"]["records"].as_array().map(Vec::len), Some(5));
}

fn copy_fixture(to: &Path) {
    fs::create_dir_all(to.join("features")).expect("features dir");
    for name in ["README.md", "create-note.md", "search.md"] {
        fs::copy(
            Path::new(FIXTURE).join(name),
            to.join("features").join(name),
        )
        .expect("copy");
    }
    fs::write(to.join("SKILL.md"), PSTACK_SKILL).expect("skill");
}

fn accept(root: &Path, proposals: &[String]) {
    for proposal in proposals {
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

fn proposals(response: &serde_json::Value) -> Vec<String> {
    response["data"]["drafts"]
        .as_array()
        .expect("drafts")
        .iter()
        .map(|draft| {
            draft["proposal"]["id"]
                .as_str()
                .expect("proposal")
                .to_string()
        })
        .collect()
}

/// A rendered feature file's pstack body: everything after the frontmatter.
fn body(text: &str) -> &str {
    let rest = text.strip_prefix("---\n").expect("frontmatter");
    let end = rest.find("\n---\n\n").expect("frontmatter end");
    &rest[end + "\n---\n\n".len()..]
}

#[test]
fn a_pstack_skill_round_trips_through_review_and_the_maintain_path() {
    let temp = tempfile::tempdir().expect("temp");
    let root = temp.path();
    establish(root);
    copy_fixture(&root.join("verify-notes"));

    // Dry run: exact drafts, nothing written.
    let dry = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "import",
            "--from",
            "verify-notes",
            "--dry-run",
        ],
        root,
    ));
    assert_eq!(dry["state"], "needs_decision", "{dry}");
    let planned = dry["data"]["drafts"].as_array().expect("drafts");
    assert_eq!(planned.len(), 3, "the map and two features: {dry}");
    assert!(dry["data"]["accepted"]
        .as_array()
        .expect("accepted")
        .is_empty());
    let before = json(&run(&["dash", "--json"], root));
    assert!(before["data"]["current"]["features"]
        .as_array()
        .expect("features")
        .is_empty());

    // Import records private drafts; nothing is in force.
    let imported = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "import",
            "--from",
            "verify-notes",
            "--request-id",
            "import-1",
        ],
        root,
    ));
    assert_eq!(imported["state"], "needs_decision", "{imported}");
    let drafts = proposals(&imported);
    assert_eq!(drafts.len(), 3);
    let replay = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "import",
            "--from",
            "verify-notes",
            "--request-id",
            "import-1",
        ],
        root,
    ));
    assert_eq!(
        proposals(&replay),
        drafts,
        "a replay returns the same drafts"
    );
    let pending = json(&run(&["dash", "--json"], root));
    assert_eq!(
        pending["data"]["current"]["header"]["drafts"], 3,
        "{pending}"
    );
    for feature in pending["data"]["current"]["features"]
        .as_array()
        .expect("features")
    {
        assert_eq!(feature["lifecycle"], "draft", "imports are never accepted");
    }
    let unproven = json(&run(
        &["check", "--json", "--feature", "feature.search"],
        root,
    ));
    assert_eq!(
        unproven["state"], "unknown",
        "a draft feature cannot be proven"
    );

    // Accepted and wired, the generated skill carries pstack's bytes.
    accept(root, &drafts);
    let wired = json(&run(
        &["init", "--json", "--action", "wire", "--host", "cursor"],
        root,
    ));
    assert_eq!(wired["state"], "success", "{wired}");
    let skill = root.join(".cursor/skills").join(
        fs::read_dir(root.join(".cursor/skills"))
            .expect("skills")
            .flatten()
            .next()
            .expect("verify skill")
            .file_name(),
    );
    for name in ["create-note.md", "search.md"] {
        let rendered = fs::read_to_string(skill.join("features").join(name)).expect("feature");
        let original = fs::read_to_string(Path::new(FIXTURE).join(name)).expect("fixture");
        assert_eq!(
            body(&rendered),
            original,
            "{name} body is pstack's, byte for byte"
        );
        whetstone::feature_map::conforms(&rendered).expect("four H2s");
    }
    let readme = fs::read_to_string(skill.join("features/README.md")).expect("readme");
    let original = fs::read_to_string(Path::new(FIXTURE).join("README.md")).expect("fixture");
    let stamp_end = readme.find("-->\n").expect("stamp") + "-->\n".len();
    assert_eq!(
        &readme[stamp_end..],
        original,
        "README is pstack's, byte for byte"
    );
    let skill_md = fs::read_to_string(skill.join("SKILL.md")).expect("skill");
    assert!(skill_md.contains("## Imported notes"));
    assert!(
        skill_md.contains("control-notes doctor"),
        "imported sections are kept"
    );

    // Re-importing the generated skill changes nothing.
    let unchanged = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "import",
            "--from",
            skill.to_str().expect("path"),
            "--request-id",
            "import-2",
        ],
        root,
    ));
    assert_eq!(unchanged["state"], "success", "{unchanged}");
    assert!(unchanged["data"]["drafts"]
        .as_array()
        .expect("drafts")
        .is_empty());

    // A maintain pass edits one gotcha; it returns as exactly one draft.
    let search = skill.join("features/search.md");
    let edited = fs::read_to_string(&search)
        .expect("search")
        .replace(
            "- Archived notes are excluded unless the user enables `Include archived`.",
            "- Archived notes are excluded unless the user enables `Include archived` in the search dialog.",
        );
    fs::write(&search, edited).expect("edit");
    let maintained = json(&run(
        &[
            "init",
            "--json",
            "--action",
            "import",
            "--from",
            skill.to_str().expect("path"),
            "--request-id",
            "import-3",
        ],
        root,
    ));
    let changed = proposals(&maintained);
    assert_eq!(changed.len(), 1, "{maintained}");
    let diff = &maintained["data"]["plan"][0]["diff"];
    assert_eq!(maintained["data"]["plan"][0]["record"], "feature.search");
    assert!(diff["before"]["record"]["gotchas"]
        .to_string()
        .contains("`Include archived`."));
    assert!(diff["after"]["record"]["gotchas"]
        .to_string()
        .contains("in the search dialog."));
    accept(root, &changed);

    // Retirement is a reviewed draft; accepted, the feature leaves force and
    // the sweep while its history stays.
    let retire = json(&run(
        &[
            "change",
            "--json",
            "--request-id",
            "retire-search",
            "--retire",
            "feature.search",
            "--rationale",
            "Search moves to the command palette.",
        ],
        root,
    ));
    assert_eq!(retire["state"], "success", "{retire}");
    let still = json(&run(&["dash", "--json"], root));
    assert_eq!(
        still["data"]["history_state"], "available",
        "{}",
        still["data"]["history_detail"]
    );
    assert!(
        still["data"]["current"]["features"]
            .as_array()
            .expect("features")
            .iter()
            .any(|feature| feature["id"] == "feature.search" && feature["lifecycle"] == "accepted"),
        "a drafted retirement changes nothing: {still}"
    );
    accept(
        root,
        &[retire["data"]["proposal"]["id"]
            .as_str()
            .expect("proposal")
            .to_string()],
    );
    let after = json(&run(&["dash", "--json"], root));
    assert!(!after["data"]["current"]["features"]
        .as_array()
        .expect("features")
        .iter()
        .any(|feature| feature["id"] == "feature.search"));
    assert!(after["data"]["changelog"]
        .as_array()
        .expect("changelog")
        .iter()
        .any(|entry| entry["title"]
            .as_str()
            .is_some_and(|title| title.starts_with("Feature retirement: Search notes"))));

    // The decision trail is show-me-your-work's TSV.
    let trail = run(&["dash", "--trail"], root);
    assert!(trail.status.success());
    let text = String::from_utf8(trail.stdout).expect("utf8");
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("ts\tphase\tdecision\twhy\tevidence\tresult")
    );
    let rows = lines.collect::<Vec<_>>();
    assert!(
        rows.iter().all(|row| row.split('\t').count() == 6),
        "{text}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("Retired feature.search")),
        "{text}"
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("Feature v2: Search notes")),
        "{text}"
    );
    let enveloped = json(&run(&["dash", "--json", "--trail"], root));
    assert_eq!(enveloped["data"]["trail"].as_str(), Some(text.as_str()));
}
