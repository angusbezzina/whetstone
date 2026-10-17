//! The Virgil record store adapter against the fake `virgil`
//! (tests/fake_virgil.py), which speaks the `virgil agreements` contract
//! Whetstone pins: the conformance suite, every exit code, the pinned API
//! version, a Virgil without the collection, paging and `VIRGIL_VIA`.
//!
//! The fake keeps its state in each test's own checkout, so the tests run
//! in parallel; PATH is set once, for the whole binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::{json, Value};
use tempfile::TempDir;
use whetstone::domain::RecordRef;
use whetstone::storage::{RecordStore, StorageError, StoreKind};
use whetstone::store::file::FileStore;
use whetstone::store::virgil::VirgilStore;

mod conformance;

const GROUP: &str = "01J8Z0000000000000000000AD";
const REPOSITORY: &str = "01J8Z0000000000000000000RP";

/// Put the fake first on PATH, once.
fn fake_on_path() {
    static BIN: OnceLock<TempDir> = OnceLock::new();
    BIN.get_or_init(|| {
        let dir = TempDir::new().expect("temp");
        let program = dir.path().join("virgil");
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake_virgil.py"),
            &program,
        )
        .expect("copy fake");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
                .expect("chmod");
        }
        let path = format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap_or_default()
        );
        std::env::set_var("PATH", path);
        dir
    });
}

/// One checkout with its own fake Virgil state.
struct Checkout {
    temp: TempDir,
}

impl Checkout {
    fn new(attached: bool) -> Self {
        fake_on_path();
        let temp = TempDir::new().expect("temp");
        let root = temp.path().join("repo");
        std::fs::create_dir_all(root.join(".fake-virgil")).expect("dir");
        let status = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .expect("git");
        assert!(status.success());
        let checkout = Self { temp };
        checkout.write_state(json!({
            "knobs": {
                "attached": if attached { json!([GROUP, REPOSITORY]) } else { json!(null) },
                "fail": {},
                "stale_get": false,
                "page": 100,
            }
        }));
        checkout
    }

    fn root(&self) -> PathBuf {
        self.temp.path().join("repo")
    }

    fn state_path(&self) -> PathBuf {
        self.root().join(".fake-virgil/state.json")
    }

    fn state(&self) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.state_path()).expect("state"))
            .expect("json")
    }

    fn write_state(&self, state: Value) {
        std::fs::write(self.state_path(), state.to_string()).expect("write state");
    }

    fn knob(&self, name: &str, value: Value) {
        let mut state = self.state();
        state["knobs"][name] = value;
        self.write_state(state);
    }

    fn calls(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root().join(".fake-virgil/calls.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("call"))
            .collect()
    }

    fn store(&self) -> VirgilStore {
        VirgilStore::new(&self.root())
    }
}

struct VirgilHarness {
    checkout: Checkout,
    store: VirgilStore,
    private: FileStore,
}

impl VirgilHarness {
    fn boxed() -> Box<dyn conformance::Harness> {
        let checkout = Checkout::new(true);
        let store = checkout.store();
        let private =
            FileStore::initialize(&checkout.temp.path().join("private/records")).expect("files");
        Box::new(Self {
            checkout,
            store,
            private,
        })
    }
}

impl conformance::Harness for VirgilHarness {
    fn store(&self) -> &dyn RecordStore {
        &self.store
    }

    fn reopen(&self) -> Box<dyn RecordStore> {
        Box::new(self.checkout.store())
    }

    fn copy_pair(&self) -> (&dyn RecordStore, &dyn RecordStore) {
        (&self.private, &self.store)
    }

    fn corrupt(&self, reference: &RecordRef) -> String {
        let mut state = self.checkout.state();
        let agreement = state["agreements"]
            .as_array_mut()
            .expect("agreements")
            .iter_mut()
            .find(|agreement| {
                agreement["record_id"] == reference.id.as_str()
                    && agreement["revision"] == reference.revision
            })
            .expect("stored agreement");
        let edited = agreement["wh_record"]
            .as_str()
            .expect("wh_record")
            .replace("Intact", "Broken");
        agreement["wh_record"] = json!(edited);
        let locator = format!("virgil:{}", agreement["id"].as_str().expect("id"));
        self.checkout.write_state(state);
        locator
    }
}

conformance_tests!(VirgilHarness::boxed);

fn a_rule(id: &str, key: &str) -> whetstone::domain::AgreementRecord {
    conformance::record(id, 1, key, conformance::rule("Keep Virgil honest."))
}

fn virgil_code(error: StorageError) -> String {
    match error {
        StorageError::Virgil { code, .. } => code,
        other => panic!("expected a Virgil refusal, got {other:?}"),
    }
}

#[test]
fn the_store_is_shareable_and_writes_through_input_with_the_pinned_api() {
    let checkout = Checkout::new(true);
    let store = checkout.store();
    assert_eq!(store.kind(), StoreKind::Shareable);
    let record = a_rule("rule.virgil-api", "virgil:api");
    store.append(&record, None).expect("append");
    let puts = checkout
        .calls()
        .into_iter()
        .filter(|call| call["argv"] == json!(["agreements", "put", "--input", "-"]))
        .collect::<Vec<_>>();
    assert_eq!(puts.len(), 1, "one put");
    // Tests run without a terminal, so the write is an agent's.
    assert!(
        puts[0]["via"]
            .as_str()
            .is_some_and(|via| via.starts_with("agent:")),
        "{:?}",
        puts[0]
    );
    assert!(checkout
        .calls()
        .iter()
        .filter(|call| call["argv"][1] == "list")
        .all(|call| call["via"].is_null()));
    let stored = &checkout.state()["agreements"][0];
    assert_eq!(stored["record_id"], "rule.virgil-api");
    assert_eq!(stored["kind"], "record");
    assert_eq!(stored["type"], "rule");
    assert_eq!(stored["lifecycle"], "pending");
    assert!(stored["wh_record"].as_str().expect("record").is_ascii());
}

#[test]
fn every_exit_code_maps_to_a_named_refusal() {
    let cases = [
        (2, "invalid_request"),
        (3, "read_again"),
        (4, "refused_content"),
        (5, "not_permitted"),
        (6, "not_found"),
        (7, "store_unavailable"),
        (1, "store_refused"),
    ];
    for (exit, code) in cases {
        let checkout = Checkout::new(true);
        checkout.knob("fail", json!({"agreements.put": [exit, false]}));
        let error = checkout
            .store()
            .append(&a_rule("rule.virgil-exit", "virgil:exit"), None)
            .expect_err("refused");
        assert_eq!(virgil_code(error), code, "exit {exit}");
        assert!(checkout.state()["agreements"]
            .as_array()
            .map_or(true, Vec::is_empty));
    }
}

#[test]
fn a_write_that_carries_an_id_landed_whatever_the_exit() {
    for exit in [3, 4, 5, 7] {
        let checkout = Checkout::new(true);
        checkout.knob("fail", json!({"agreements.put": [exit, true]}));
        let record = a_rule("rule.virgil-landed", "virgil:landed");
        let written = checkout
            .store()
            .append(&record, None)
            .unwrap_or_else(|error| panic!("exit {exit} with an id landed: {error:?}"));
        assert_eq!(
            checkout.store().get(&written).expect("get").as_ref(),
            Some(&record)
        );
    }
}

#[test]
fn an_unattached_checkout_is_refused_by_name() {
    let checkout = Checkout::new(false);
    let error = checkout.store().all_records().expect_err("unattached");
    let message = error.to_string();
    assert_eq!(virgil_code(error), "unattached");
    assert!(message.contains("virgil repos attach"), "{message}");
}

#[test]
fn another_api_version_or_no_collection_is_a_contract_failure() {
    let checkout = Checkout::new(true);
    checkout.knob("agreements_api", json!("virgil.agreements/v2"));
    assert_eq!(
        virgil_code(checkout.store().all_records().expect_err("api")),
        "store_contract"
    );
    let checkout = Checkout::new(true);
    checkout.knob("no_agreements", json!(true));
    assert_eq!(
        virgil_code(checkout.store().probe().expect_err("no collection")),
        "store_contract"
    );
}

#[test]
fn a_racing_writer_is_refused_never_merged() {
    let checkout = Checkout::new(true);
    let store = checkout.store();
    let first = a_rule("rule.virgil-race", "virgil:race:1");
    store.append(&first, None).expect("first");
    // Another writer's revision 2 lands behind this handle's back.
    let theirs = conformance::next(&first, "virgil:race:theirs", "Theirs.");
    checkout.store().append(&theirs, Some(1)).expect("theirs");
    let ours = conformance::next(&first, "virgil:race:ours", "Ours.");
    assert!(matches!(
        store.append(&ours, Some(1)),
        Err(StorageError::StaleRevision {
            expected: Some(1),
            actual: Some(2)
        })
    ));
    // Even when the client's read was stale, Virgil checks again.
    let mut state = checkout.state();
    let request = json!({
        "agreement": whetstone::store::virgil::wire(&ours, "pending").expect("wire"),
        "expected_revision": 1,
    });
    state["knobs"]["fail"] = json!({});
    checkout.write_state(state);
    let output = Command::new("virgil")
        .args(["agreements", "put", "--input", "-"])
        .current_dir(checkout.root())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write as _;
            child
                .stdin
                .take()
                .expect("stdin")
                .write_all(request.to_string().as_bytes())?;
            child.wait_with_output()
        })
        .expect("fake");
    assert_eq!(output.status.code(), Some(3));
    let answer: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert_eq!(answer["error"]["code"], "stale_revision");
    assert_eq!(answer["error"]["actual"], 2);
}

#[test]
fn listings_follow_every_page() {
    let checkout = Checkout::new(true);
    checkout.knob("page", json!(2));
    let store = checkout.store();
    for index in 0..5 {
        store
            .append(
                &a_rule(
                    &format!("rule.virgil-page-{index}"),
                    &format!("page:{index}"),
                ),
                None,
            )
            .expect("append");
    }
    assert_eq!(store.all_records().expect("all").len(), 5);
    assert!(
        checkout
            .calls()
            .iter()
            .filter(|call| call["argv"][1] == "list")
            .count()
            >= 3
    );
}

#[test]
fn import_beads_reports_each_revision_and_applies_only_when_asked() {
    let checkout = Checkout::new(true);
    let record = a_rule("rule.virgil-import", "virgil:import");
    let export = json!({
        "schema": "whetstone.agreements-export/v1",
        "scope": {"repository": REPOSITORY},
        "agreements": [whetstone::store::virgil::wire(&record, "accepted").expect("wire")],
    });
    let file = checkout.temp.path().join("agreements.json");
    std::fs::write(&file, export.to_string()).expect("export");
    let store = checkout.store();
    let dry = store.import_beads(&file, false).expect("dry run");
    assert_eq!(dry[0]["disposition"], "import");
    assert!(store.all_records().expect("all").is_empty());
    store.import_beads(&file, true).expect("apply");
    let again = store.import_beads(&file, false).expect("again");
    assert_eq!(again[0]["disposition"], "present");
    assert_eq!(
        store
            .fetch(&record.id, 1)
            .expect("fetch")
            .expect("present")
            .reference,
        record.reference().expect("reference")
    );
    // An export inside the checkout is refused, as the lessons import is.
    let inside = checkout.root().join("agreements.json");
    std::fs::write(&inside, export.to_string()).expect("export");
    assert_eq!(
        virgil_code(store.import_beads(&inside, false).expect_err("inside")),
        "invalid_request"
    );
}
