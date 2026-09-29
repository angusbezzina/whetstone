//! The principle catalogue: pstack's principles, pinned to the pstack release
//! Whetstone was built against and refreshed from the installed pstack when
//! it is present. Owners pick principles in onboarding or add their own.

use std::path::Path;

use serde::{Deserialize, Serialize};

const PINNED: &str = include_str!("../assets/pstack/principles.json");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogueEntry {
    pub id: String,
    pub group: String,
    pub rule: String,
    /// How the principle can be held: mechanical, question or review.
    pub enforceable: String,
    #[serde(default)]
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalogue {
    pub schema: String,
    pub source: String,
    pub version: String,
    pub commit: String,
    pub license: String,
    pub principles: Vec<CatalogueEntry>,
    /// Where the rule text came from: `pinned` or the installed skills.
    #[serde(default = "pinned")]
    pub read_from: String,
}

fn pinned() -> String {
    "pinned".into()
}

/// The catalogue pinned in the binary.
pub fn pinned_catalogue() -> Catalogue {
    serde_json::from_str(PINNED).expect("the compiled-in principle catalogue is valid JSON")
}

/// The pinned catalogue, with each principle's text refreshed from an
/// installed `principle-<id>/SKILL.md` description when one exists, so an
/// upgraded pstack shows its own wording.
pub fn catalogue(project_root: &Path) -> Catalogue {
    let mut catalogue = pinned_catalogue();
    let mut refreshed = 0;
    for entry in &mut catalogue.principles {
        for host in ["claude", "cursor", "agents"] {
            let found =
                crate::setup::find_skill(project_root, host, &format!("principle-{}", entry.id));
            let Some(path) = found.path else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(description) = frontmatter_description(&text) {
                entry.rule = description;
                refreshed += 1;
                break;
            }
        }
    }
    if refreshed > 0 {
        catalogue.read_from = format!("installed pstack ({refreshed} principle(s) refreshed)");
    }
    catalogue
}

fn frontmatter_description(text: &str) -> Option<String> {
    let body = text.strip_prefix("---\n")?;
    let end = body.find("\n---")?;
    body[..end].lines().find_map(|line| {
        line.strip_prefix("description:").map(|value| {
            value
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .to_string()
        })
    })
}

impl Catalogue {
    pub fn get(&self, id: &str) -> Option<&CatalogueEntry> {
        self.principles.iter().find(|entry| entry.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pinned_catalogue_lists_every_pstack_principle() {
        let catalogue = pinned_catalogue();
        assert_eq!(catalogue.version, crate::setup::PSTACK_VERSION);
        assert_eq!(catalogue.principles.len(), 23);
        let prove = catalogue.get("prove-it-works").expect("prove-it-works");
        assert_eq!(prove.enforceable, "mechanical");
        assert!(catalogue.principles.iter().all(|entry| matches!(
            entry.enforceable.as_str(),
            "mechanical" | "question" | "review"
        )));
    }

    #[test]
    fn an_installed_principle_refreshes_its_wording() {
        let temp = tempfile::tempdir().expect("temp");
        let dir = temp
            .path()
            .join(".agents/skills/principle-laziness-protocol");
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(
            dir.join("SKILL.md"),
            "---\nname: principle-laziness-protocol\ndescription: \"Delete first.\"\n---\n# Laziness\n",
        )
        .expect("write");
        let catalogue = catalogue(temp.path());
        assert_eq!(
            catalogue
                .get("laziness-protocol")
                .map(|entry| entry.rule.as_str()),
            Some("Delete first.")
        );
        assert!(catalogue.read_from.starts_with("installed"));
    }
}
