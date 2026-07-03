//! Per-project taste-guidance store (whetstone-7bo).
//!
//! Not every standard is deterministically enforceable — some are judgment
//! ("keep route handlers thin", "prefer composition over inheritance here").
//! Those can't be an `ast_query` or a `lint_proxy`, so instead of forcing them
//! into low-confidence scannable rules (violating "high confidence or silence"),
//! they live here as *guidance*: injected into generated agent context and
//! surfaced by `wh rules query` / the MCP server, but never scanned.
//!
//! Store layout (schema: `references/guidance-schema.yaml`):
//!   whetstone/guidance/<name>.yaml            (project, committed)
//!   whetstone/.personal/guidance/<name>.yaml  (personal, gitignored)

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GuidanceEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub text: String,
    /// Languages this guidance applies to (empty = all languages).
    #[serde(default)]
    pub languages: Vec<String>,
    /// Dependencies this guidance relates to (empty = all deps).
    #[serde(default)]
    pub deps: Vec<String>,
    /// "project" | "personal" — set at load time, not authored.
    #[serde(default)]
    pub layer: String,
}

#[derive(Deserialize)]
struct GuidanceFile {
    #[serde(default)]
    guidance: Vec<GuidanceEntry>,
}

fn read_dir_guidance(dir: &Path, layer: &str, out: &mut Vec<GuidanceEntry>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("yaml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        let Ok(file) = serde_yaml::from_str::<GuidanceFile>(&text) else {
            continue;
        };
        for mut g in file.guidance {
            if g.id.trim().is_empty() {
                continue;
            }
            g.layer = layer.to_string();
            out.push(g);
        }
    }
}

/// Load guidance entries from the selected layers, filtered by language (entries
/// with an empty `languages` list match every language). Sorted by id.
pub fn load(
    project_dir: &Path,
    lang_filter: Option<&str>,
    include_project: bool,
    include_personal: bool,
) -> Vec<GuidanceEntry> {
    let paths = crate::layers::LayerPaths::for_project(project_dir);
    let mut out = Vec::new();
    if include_project {
        read_dir_guidance(&paths.whetstone_dir.join("guidance"), "project", &mut out);
    }
    if include_personal {
        read_dir_guidance(&paths.personal_dir.join("guidance"), "personal", &mut out);
    }
    out.retain(|g| lang_matches(g, lang_filter));
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

pub fn lang_matches(g: &GuidanceEntry, lang: Option<&str>) -> bool {
    match lang {
        None => true,
        Some(l) => {
            g.languages.is_empty() || g.languages.iter().any(|x| x.eq_ignore_ascii_case(l))
        }
    }
}

pub fn dep_matches(g: &GuidanceEntry, dep: Option<&str>) -> bool {
    match dep {
        None => true,
        Some(d) => g.deps.is_empty() || g.deps.iter().any(|x| x.eq_ignore_ascii_case(d)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }

    #[test]
    fn loads_and_filters_by_language_and_layer() {
        let tmp = std::env::temp_dir().join(format!("wh_guid_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        write(
            &tmp.join("whetstone/guidance"),
            "g.yaml",
            "guidance:\n  - id: thin-handlers\n    title: Keep handlers thin\n    languages: [python]\n    deps: [fastapi]\n    text: Delegate to a service layer.\n  - id: any-lang\n    title: Prefer small functions\n    text: Applies everywhere.\n",
        );
        write(
            &tmp.join("whetstone/.personal/guidance"),
            "p.yaml",
            "guidance:\n  - id: my-pref\n    title: My personal preference\n    text: Local only.\n",
        );

        // project only, python: thin-handlers + any-lang (empty languages matches)
        let py = load(&tmp, Some("python"), true, false);
        let ids: Vec<&str> = py.iter().map(|g| g.id.as_str()).collect();
        assert!(ids.contains(&"thin-handlers") && ids.contains(&"any-lang"));
        assert!(!ids.contains(&"my-pref"), "personal excluded");

        // project only, rust: python-scoped one drops out, any-lang stays
        let rs = load(&tmp, Some("rust"), true, false);
        let ids: Vec<&str> = rs.iter().map(|g| g.id.as_str()).collect();
        assert!(!ids.contains(&"thin-handlers") && ids.contains(&"any-lang"));

        // include personal
        let all = load(&tmp, None, true, true);
        assert!(all.iter().any(|g| g.id == "my-pref" && g.layer == "personal"));

        assert!(dep_matches(&py[0], Some("fastapi")) || dep_matches(&py[1], Some("fastapi")));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
