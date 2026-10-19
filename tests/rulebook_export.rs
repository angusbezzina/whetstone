//! `wh init --action export` on a throwaway repository with a shared Beads
//! database: what the team accepted and shared becomes
//! whetstone/rulebook.json, what is accepted only here or still a draft goes
//! to whetstone/rulebook.drafts.json, a file that differs is replaced only
//! with --yes, and Beads is never written.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use whetstone::domain::{
    LocalReview, LocalReviewVerdict, Proposal, ProposalState, RecordBody, RecordRef, Rule,
    LOCAL_REVIEW_ASSURANCE,
};
use whetstone::service::{agreement_record, utc_timestamp};
use whetstone::storage::{initialize_private, ProjectLayout, RecordStore};

mod support;

fn run(args: &[&str], cwd: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_whetstone"))
        .args(args)
        .current_dir(cwd)
        .env("PATH", support::path_with_bd_template())
        .env_remove("BEADS_DIR")
        .output()
        .expect("run whetstone");
    json(&output)
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "expected JSON ({error})\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn tool(program: &str, root: &Path, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .env("BEADS_DIR", root.join(".beads"))
        .env("BD_NON_INTERACTIVE", "1")
        .output()
        .expect(program);
    assert!(
        output.status.success(),
        "{program} {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// The private store, written directly as the workflows wrote drafts and
/// reviews before the rulebook was a file.
struct Private {
    layout: ProjectLayout,
    store: Box<dyn RecordStore>,
}

impl Private {
    fn open(root: &Path) -> Self {
        let layout = ProjectLayout::resolve(root).expect("layout");
        let store = initialize_private(&layout).expect("private store");
        Self { layout, store }
    }

    fn append(&self, id: &str, body: RecordBody) -> RecordRef {
        let record = agreement_record(&self.layout, id, format!("{id}:test"), body, Some("Owner"))
            .expect("record");
        self.store.append(&record, None).expect("append")
    }

    /// A rule draft and the proposal that carries it; returns the proposal.
    fn rule(&self, id: &str, statement: &str) -> RecordRef {
        let rule: Rule = serde_json::from_value(json!({
            "schema": "whetstone.rule.v2",
            "statement": statement,
            "rationale": "The team relies on it.",
            "strength": "must",
            "enforcer": {"kind": "test", "command": "git --version"},
            "source": {"kind": "owner"},
            "paths": ["src/**"],
        }))
        .expect("rule");
        let drafted = self.append(id, RecordBody::Rule(rule));
        self.append(
            &format!("proposal.{id}"),
            RecordBody::Proposal(Proposal {
                state: ProposalState::Draft,
                title: format!("Rule proposed: {statement}"),
                rationale: "Rationale: The team relies on it.".into(),
                proposed_records: vec![drafted],
                binding: None,
            }),
        )
    }

    /// The owner's acceptance of a proposal.
    fn accept(&self, proposal: &RecordRef) {
        let reviewer = self
            .store
            .get(proposal)
            .expect("read")
            .expect("proposal")
            .owner;
        self.append(
            &format!("review.{}", proposal.id.as_str()),
            RecordBody::LocalReview(LocalReview {
                proposal: proposal.clone(),
                verdict: LocalReviewVerdict::Accept,
                reviewer,
                assurance: LOCAL_REVIEW_ASSURANCE.into(),
                rationale: "Accepted for the test.".into(),
                reviewed_at: utc_timestamp(),
                team_activation_permitted: false,
                cites: Vec::new(),
            }),
        );
    }
}

fn agree(root: &Path) {
    let inspected = run(&["init", "--json", "--request-id", "i1"], root);
    let revision = inspected["expected_revision"]
        .as_u64()
        .expect("revision")
        .to_string();
    let token = inspected["resume_token"]
        .as_str()
        .expect("token")
        .to_string();
    let agreed = run(
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
            "Nothing broken reaches the team.",
        ],
        root,
    );
    assert_eq!(
        agreed["data"]["records"][0]["id"], "mission.project",
        "{agreed}"
    );
}

fn push(root: &Path) {
    let reviewed = run(&["push", "--json"], root);
    assert_eq!(reviewed["state"], "needs_decision", "{reviewed}");
    let token = reviewed["resume_token"].as_str().expect("token");
    let pushed = run(&["push", "--json", "--confirm", token], root);
    assert_eq!(pushed["state"], "success", "{pushed}");
}

/// Every Beads row in the shared database, to prove the export writes none.
fn shared_rows(root: &Path) -> String {
    tool("bd", root, &["list", "--all", "--json", "--limit", "0"])
}

/// A clone of a bare team remote with one commit on main, holding the
/// team's shared Beads database, which pushes to that remote.
fn repository(temp: &Path) -> PathBuf {
    let origin = temp.join("origin.git");
    let root = temp.join("repo");
    let path = |path: &Path| path.to_str().expect("path").to_string();
    tool(
        "git",
        temp,
        &[
            "init",
            "-q",
            "--bare",
            "--initial-branch=main",
            &path(&origin),
        ],
    );
    tool("git", temp, &["clone", "-q", &path(&origin), &path(&root)]);
    tool(
        "git",
        &root,
        &[
            "-c",
            "user.email=owner@example.invalid",
            "-c",
            "user.name=Owner",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ],
    );
    tool("git", &root, &["push", "-q", "origin", "HEAD:main"]);
    tool(
        "bd",
        &root,
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
    tool(
        "bd",
        &root,
        &[
            "dolt",
            "remote",
            "add",
            "origin",
            &format!("git+file://{}", origin.display()),
        ],
    );
    root
}

#[test]
fn the_shared_store_becomes_the_rulebook_and_drafts_stay_out_of_it() {
    // The records this test writes itself go through the same fast bd.
    std::env::set_var("PATH", support::path_with_bd_template());
    let temp = tempfile::tempdir().expect("temp");
    let root = repository(temp.path());
    agree(&root);
    std::env::remove_var("BEADS_DIR");
    let private = Private::open(&root);
    let shared = private.rule("rule.toolchain", "The toolchain answers.");
    private.accept(&shared);
    push(&root);
    // Accepted here but never shared, and a draft never accepted.
    let local = private.rule("rule.local", "Only this checkout agreed.");
    private.accept(&local);
    private.rule("rule.proposed", "Still a proposal.");
    let rows = shared_rows(&root);

    let dry = run(
        &["init", "--json", "--action", "export", "--dry-run"],
        &root,
    );
    assert_eq!(dry["state"], "needs_decision", "{dry}");
    assert!(!root.join("whetstone/rulebook.json").exists());

    let exported = run(&["init", "--json", "--action", "export"], &root);
    assert_eq!(exported["state"], "success", "{exported}");
    let data = &exported["data"];
    assert_eq!(data["rulebook"]["written"], true, "{data}");
    assert_eq!(data["rulebook"]["mission_from"], "shared");
    assert_eq!(
        data["rulebook"]["rules"],
        serde_json::json!(["rule.toolchain"])
    );
    assert_eq!(data["counts"]["rules"], 1);
    assert_eq!(data["counts"]["drafts"], 2);
    assert_eq!(data["beads_written"], false);
    let book = whetstone::rulebook::load(&root).expect("the export reads back");
    assert_eq!(book.mission.statement, "Nothing broken reaches the team.");
    assert_eq!(book.rules[0].rule.statement, "The toolchain answers.");
    assert_eq!(book.rules[0].rule.paths, ["src/**"]);
    let drafts: Value = serde_json::from_str(
        &fs::read_to_string(root.join("whetstone/rulebook.drafts.json")).expect("drafts"),
    )
    .expect("drafts json");
    let states = drafts["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|entry| {
            format!(
                "{} {}",
                entry["record"].as_str().unwrap_or(""),
                entry["state"].as_str().unwrap_or("")
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        ["rule.local accepted_private", "rule.proposed draft"]
    );

    let again = run(&["init", "--json", "--action", "export"], &root);
    assert_eq!(again["state"], "success", "{again}");
    assert_eq!(again["data"]["rulebook"]["existing"], "same");
    assert_eq!(again["data"]["rulebook"]["written"], false);

    let path = root.join("whetstone/rulebook.json");
    let written = fs::read_to_string(&path).expect("rulebook");
    let edited = written.replace("The toolchain answers.", "Edited by hand.");
    fs::write(&path, &edited).expect("edit");
    let refused = run(&["init", "--json", "--action", "export"], &root);
    assert_eq!(refused["state"], "needs_decision", "{refused}");
    assert_eq!(
        refused["data"]["rulebook"]["differences"],
        serde_json::json!(["rule rule.toolchain differs in statement"])
    );
    assert_eq!(fs::read_to_string(&path).expect("rulebook"), edited);
    let replaced = run(&["init", "--json", "--action", "export", "--yes"], &root);
    assert_eq!(replaced["state"], "success", "{replaced}");
    assert_eq!(fs::read_to_string(&path).expect("rulebook"), written);

    assert_eq!(shared_rows(&root), rows, "the export never writes Beads");
}
