//! The principle catalogue: pstack's principles, pinned to the pstack release
//! Whetstone was built against and compiled into the binary. Owners pick
//! principles from it in `wh init` or add their own.

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
}

/// The catalogue pinned in the binary.
pub fn catalogue() -> Catalogue {
    serde_json::from_str(PINNED).expect("the compiled-in principle catalogue is valid JSON")
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
        let catalogue = catalogue();
        assert_eq!(catalogue.version, "0.15.5");
        assert_eq!(catalogue.principles.len(), 23);
        let prove = catalogue.get("prove-it-works").expect("prove-it-works");
        assert_eq!(prove.enforceable, "mechanical");
        assert!(catalogue.principles.iter().all(|entry| matches!(
            entry.enforceable.as_str(),
            "mechanical" | "question" | "review"
        )));
    }
}
