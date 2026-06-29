//! Durable gate for the shipped rule corpus (epic whetstone-5ox).
//!
//! Every pack under `packs/<lang>/<dep>.yaml` must pass `wh eval` through the
//! real import path (whetstone.yaml `extends` -> merge -> eval): each rule's
//! golden examples run through the actual scanner, a `pass` example must not
//! fire and a `fail` example must fire. This keeps the corpus honest as it grows.

use std::path::PathBuf;
use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_whetstone")
}

fn discover_packs() -> Vec<PathBuf> {
    let packs_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("packs");
    let mut out = Vec::new();
    let Ok(langs) = std::fs::read_dir(&packs_dir) else {
        return out;
    };
    for lang in langs.flatten() {
        if !lang.path().is_dir() {
            continue;
        }
        if let Ok(files) = std::fs::read_dir(lang.path()) {
            for f in files.flatten() {
                let p = f.path();
                if p.extension().and_then(|e| e.to_str()) == Some("yaml") {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

#[test]
fn all_shipped_packs_pass_eval() {
    let packs = discover_packs();
    assert!(!packs.is_empty(), "no packs found under packs/");

    let mut total_rules = 0i64;
    for pack in &packs {
        let dep = pack.file_stem().unwrap().to_string_lossy().to_string();
        let tmp = std::env::temp_dir().join(format!(
            "wh_pack_{}_{}_{}",
            dep,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let pdir = tmp.join("whetstone").join("packs");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::copy(pack, pdir.join(format!("{dep}.yaml"))).unwrap();
        std::fs::write(
            tmp.join("whetstone").join("whetstone.yaml"),
            format!(
                "version: 1\nextends:\n  - scope: project\n    ref: path:./whetstone/packs/{dep}.yaml\n"
            ),
        )
        .unwrap();

        let out = Command::new(bin())
            .args(["eval", "--project-dir", tmp.to_str().unwrap(), "--json"])
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!("pack {dep}: eval did not produce JSON. stderr: {stderr}")
        });

        assert_eq!(v["ok"], true, "pack {dep} failed eval: {v}");
        assert_eq!(
            v["golden_mismatch_count"], 0,
            "pack {dep} has golden mismatches: {v}"
        );
        assert!(
            !stderr.contains("malformed ast_query"),
            "pack {dep} has a malformed ast_query: {stderr}"
        );

        let cards = v["scorecards"].as_array().cloned().unwrap_or_default();
        let checked: i64 = cards
            .iter()
            .map(|c| c["golden_checked"].as_i64().unwrap_or(0))
            .sum();
        assert!(
            checked > 0,
            "pack {dep} ran zero goldens through the scanner (vacuous): {v}"
        );
        total_rules += v["rules_evaluated"].as_i64().unwrap_or(0);
        let _ = std::fs::remove_dir_all(&tmp);
    }
    assert!(
        total_rules >= 20,
        "expected a substantive corpus, only {total_rules} rules across {} packs",
        packs.len()
    );
}
