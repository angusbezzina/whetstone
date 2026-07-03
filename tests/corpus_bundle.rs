//! Gate: the binary-embedded corpus (src/corpus.rs) must list every shipped pack
//! under packs/<lang>/<dep>.yaml, so `wh init --claude` can import all of them.

use std::collections::BTreeSet;

#[test]
fn embedded_corpus_matches_packs_dir() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs");
    let mut on_disk = BTreeSet::new();
    for lang in std::fs::read_dir(&root).unwrap().flatten() {
        let ln = lang.file_name().to_string_lossy().to_string();
        if ln == "templates" || !lang.path().is_dir() {
            continue; // templates are not part of the auto-import corpus
        }
        for f in std::fs::read_dir(lang.path()).unwrap().flatten() {
            let p = f.path();
            if p.extension().and_then(|e| e.to_str()) == Some("yaml") {
                on_disk.insert(format!(
                    "{}/{}",
                    ln,
                    p.file_stem().unwrap().to_string_lossy()
                ));
            }
        }
    }
    // Re-derive the bundled set by reading src/corpus.rs's include_str! lines.
    let corpus_rs = std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/corpus.rs"),
    )
    .unwrap();
    let mut bundled = BTreeSet::new();
    for line in corpus_rs.lines() {
        if let Some(rest) = line.trim().strip_prefix("yaml: include_str!(\"../packs/") {
            if let Some(path) = rest.strip_suffix(".yaml\"),") {
                bundled.insert(path.to_string());
            }
        }
    }
    assert_eq!(
        on_disk, bundled,
        "src/corpus.rs bundle is out of sync with packs/ — update PACKS.\n  on_disk: {on_disk:?}\n  bundled: {bundled:?}"
    );
}
