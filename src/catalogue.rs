//! Principle packs. A pack is a named, versioned list of principles, each
//! with a hint at the enforcer that usually holds it. pstack is the default
//! pack and ships in the binary; a repository adds its own packs as JSON files
//! under `whetstone/principles/` (or with `wh change --import-pack <file>`).
//! Owners pick principles from any loaded pack in onboarding, or write their
//! own.
//!
//! A principle's id is unique across every loaded pack: a pack that repeats an
//! id another pack already holds is refused, so `--principle <id>` always
//! names one principle. `--principle <pack>/<id>` names it explicitly.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const PACK_SCHEMA: &str = "whetstone.principle-pack.v1";
/// The default pack's name; its principles keep their bare record ids.
pub const DEFAULT_PACK: &str = "pstack";
/// Where a repository keeps its own principle packs.
pub const PACK_DIRECTORY: &str = "whetstone/principles";
/// The enforcer kinds a principle may hint at, or `none`.
pub const ENFORCER_HINTS: [&str; 6] = ["ast", "lint", "formatter", "test", "validator", "none"];

const PINNED: &str = include_str!("../assets/pstack/principles.json");

/// One principle in a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackPrinciple {
    pub id: String,
    pub group: String,
    pub text: String,
    /// The enforcer kind that usually holds it: ast, lint, formatter, test,
    /// validator, or none (held by review or not mechanically checkable).
    pub enforcer_hint: String,
    #[serde(default)]
    pub rationale: Option<String>,
}

/// A principle pack file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub schema: String,
    pub name: String,
    pub version: String,
    pub source: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub principles: Vec<PackPrinciple>,
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

impl Pack {
    /// Read and check a pack's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let pack: Self =
            serde_json::from_str(text).map_err(|error| format!("not a principle pack: {error}"))?;
        pack.validate()?;
        Ok(pack)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != PACK_SCHEMA {
            return Err(format!("a principle pack's schema is {PACK_SCHEMA}"));
        }
        if !is_slug(&self.name) {
            return Err(format!(
                "the pack name {:?} must be a lowercase slug (letters, digits, -)",
                self.name
            ));
        }
        if self.version.trim().is_empty() {
            return Err(format!("the pack {} has no version", self.name));
        }
        if self.principles.is_empty() {
            return Err(format!("the pack {} has no principles", self.name));
        }
        let mut seen = std::collections::BTreeSet::new();
        for principle in &self.principles {
            if !is_slug(&principle.id) {
                return Err(format!(
                    "{} in pack {} is not a lowercase slug",
                    principle.id, self.name
                ));
            }
            if !seen.insert(principle.id.as_str()) {
                return Err(format!(
                    "the pack {} lists {} twice",
                    self.name, principle.id
                ));
            }
            if principle.text.trim().is_empty() {
                return Err(format!(
                    "{} in pack {} has no text",
                    principle.id, self.name
                ));
            }
            if !ENFORCER_HINTS.contains(&principle.enforcer_hint.as_str()) {
                return Err(format!(
                    "{} in pack {} hints at {:?}; use one of {}",
                    principle.id,
                    self.name,
                    principle.enforcer_hint,
                    ENFORCER_HINTS.join(", ")
                ));
            }
        }
        Ok(())
    }
}

/// A principle as the picker offers it: its pack beside its fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogueEntry {
    pub pack: String,
    pub pack_version: String,
    pub id: String,
    /// How `--principle` names it: the bare id for the default pack,
    /// `<pack>/<id>` otherwise.
    pub spec: String,
    pub group: String,
    pub text: String,
    pub enforcer_hint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl CatalogueEntry {
    /// The record id a picked principle is stored under.
    pub fn record_id(&self) -> String {
        if self.pack == DEFAULT_PACK {
            format!("principle.{}", self.id)
        } else {
            format!("principle.{}.{}", self.pack, self.id)
        }
    }

    /// Where the principle record says it came from: the default pack keeps
    /// its pstack provenance, any other pack is named with its version.
    pub fn source(&self) -> crate::domain::PrincipleSource {
        if self.pack == DEFAULT_PACK {
            crate::domain::PrincipleSource::Pstack {
                id: self.id.clone(),
                version: self.pack_version.clone(),
            }
        } else {
            crate::domain::PrincipleSource::Pack {
                pack: self.pack.clone(),
                id: self.id.clone(),
                version: self.pack_version.clone(),
            }
        }
    }
}

/// Every loaded pack, in load order (the default pack first).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Catalogue {
    pub packs: Vec<PackSummary>,
    pub principles: Vec<CatalogueEntry>,
    /// Packs found in the repository that could not be loaded, and why.
    pub refused: Vec<RefusedPack>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackSummary {
    pub name: String,
    pub version: String,
    pub source: String,
    pub license: String,
    /// `built in` or the repository path it was read from.
    pub read_from: String,
    pub principles: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusedPack {
    pub path: String,
    pub reason: String,
}

/// The default pack pinned in the binary.
pub fn pinned_pack() -> Pack {
    Pack::parse(PINNED).expect("the compiled-in principle pack is valid")
}

/// The pinned default pack and every pack under `whetstone/principles/`.
pub fn catalogue(project_root: &Path) -> Catalogue {
    let mut catalogue = Catalogue {
        packs: Vec::new(),
        principles: Vec::new(),
        refused: Vec::new(),
    };
    let _ = catalogue.add(pinned_pack(), "built in".into());
    for path in pack_files(project_root) {
        let relative = path
            .strip_prefix(project_root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let result = std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| Pack::parse(&text))
            .and_then(|pack| catalogue.add(pack, relative.clone()));
        if let Err(reason) = result {
            catalogue.refused.push(RefusedPack {
                path: relative,
                reason,
            });
        }
    }
    catalogue
}

fn pack_files(project_root: &Path) -> Vec<PathBuf> {
    let mut files = std::fs::read_dir(project_root.join(PACK_DIRECTORY))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension()
                        .is_some_and(|extension| extension == "json")
                        && path.is_file()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.sort();
    files
}

impl Catalogue {
    /// Load one more pack; a name or principle id already loaded is refused.
    pub fn add(&mut self, pack: Pack, read_from: String) -> Result<(), String> {
        pack.validate()?;
        if self.packs.iter().any(|loaded| loaded.name == pack.name) {
            return Err(format!("a pack named {} is already loaded", pack.name));
        }
        if let Some(duplicate) = pack.principles.iter().find_map(|principle| {
            self.principles
                .iter()
                .find(|loaded| loaded.id == principle.id)
        }) {
            return Err(format!(
                "{} is already a principle in the {} pack; ids are unique across packs",
                duplicate.id, duplicate.pack
            ));
        }
        self.packs.push(PackSummary {
            name: pack.name.clone(),
            version: pack.version.clone(),
            source: pack.source.clone(),
            license: pack.license.clone(),
            read_from,
            principles: pack.principles.len(),
        });
        for principle in pack.principles {
            let spec = if pack.name == DEFAULT_PACK {
                principle.id.clone()
            } else {
                format!("{}/{}", pack.name, principle.id)
            };
            self.principles.push(CatalogueEntry {
                pack: pack.name.clone(),
                pack_version: pack.version.clone(),
                id: principle.id,
                spec,
                group: principle.group,
                text: principle.text,
                enforcer_hint: principle.enforcer_hint,
                rationale: principle.rationale,
            });
        }
        Ok(())
    }

    /// A principle by `<id>` or `<pack>/<id>`.
    pub fn get(&self, spec: &str) -> Option<&CatalogueEntry> {
        let (pack, id) = match spec.split_once('/') {
            Some((pack, id)) => (Some(pack), id),
            None => (None, spec),
        };
        self.principles
            .iter()
            .find(|entry| entry.id == id && pack.map_or(true, |pack| entry.pack == pack))
    }

    /// The loaded packs as `name version`, for messages.
    pub fn describe(&self) -> String {
        self.packs
            .iter()
            .map(|pack| format!("{} {}", pack.name, pack.version))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str, ids: &[&str]) -> String {
        serde_json::json!({
            "schema": PACK_SCHEMA,
            "name": name,
            "version": "1.0.0",
            "source": "test",
            "license": "MIT",
            "principles": ids.iter().map(|id| serde_json::json!({
                "id": id, "group": "team", "text": format!("Principle {id}."), "enforcer_hint": "none", "rationale": null
            })).collect::<Vec<_>>(),
        })
        .to_string()
    }

    #[test]
    fn the_default_pack_is_pstack_and_every_hint_is_an_enforcer_kind() {
        let pack = pinned_pack();
        assert_eq!(pack.name, DEFAULT_PACK);
        assert_eq!(pack.version, "0.15.5");
        assert_eq!(pack.principles.len(), 23);
        let prove = pack
            .principles
            .iter()
            .find(|principle| principle.id == "prove-it-works")
            .expect("prove-it-works");
        assert_eq!(prove.enforcer_hint, "test");
    }

    #[test]
    fn the_pinned_pack_matches_the_schema_in_references() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../references/principle-pack-v1.schema.json"))
                .expect("schema");
        let pack: serde_json::Value = serde_json::from_str(PINNED).expect("pack");
        assert_eq!(schema["properties"]["schema"]["const"], PACK_SCHEMA);
        let keys = |at: &serde_json::Value, field: &str| -> Vec<String> {
            match field {
                "required" => at["required"]
                    .as_array()
                    .expect("required")
                    .iter()
                    .map(|key| key.as_str().expect("key").to_string())
                    .collect(),
                _ => at["properties"]
                    .as_object()
                    .expect("properties")
                    .keys()
                    .cloned()
                    .collect(),
            }
        };
        let item = &schema["properties"]["principles"]["items"];
        for key in keys(&schema, "required") {
            assert!(pack.get(&key).is_some(), "the pack lacks {key}");
        }
        for key in pack.as_object().expect("object").keys() {
            assert!(
                keys(&schema, "properties").contains(key),
                "the schema does not allow {key}"
            );
        }
        let hints = item["properties"]["enforcer_hint"]["enum"]
            .as_array()
            .expect("hints")
            .iter()
            .map(|hint| hint.as_str().expect("hint"))
            .collect::<Vec<_>>();
        assert_eq!(hints, ENFORCER_HINTS);
        for principle in pack["principles"].as_array().expect("principles") {
            for key in keys(item, "required") {
                assert!(principle.get(&key).is_some(), "{principle} lacks {key}");
            }
            for key in principle.as_object().expect("object").keys() {
                assert!(
                    keys(item, "properties").contains(key),
                    "the schema does not allow {key}"
                );
            }
        }
    }

    #[test]
    fn repository_packs_load_beside_the_default_and_duplicates_are_refused() {
        let temp = tempfile::tempdir().expect("temp");
        let directory = temp.path().join(PACK_DIRECTORY);
        std::fs::create_dir_all(&directory).expect("dir");
        std::fs::write(
            directory.join("team.json"),
            pack("team", &["ship-small", "own-the-pager"]),
        )
        .expect("team");
        std::fs::write(
            directory.join("clash.json"),
            pack("clash", &["prove-it-works"]),
        )
        .expect("clash");
        std::fs::write(directory.join("broken.json"), "{").expect("broken");
        let catalogue = catalogue(temp.path());
        assert_eq!(
            catalogue
                .packs
                .iter()
                .map(|pack| pack.name.as_str())
                .collect::<Vec<_>>(),
            ["pstack", "team"]
        );
        assert_eq!(
            catalogue.get("ship-small").map(|entry| entry.pack.as_str()),
            Some("team")
        );
        assert_eq!(
            catalogue
                .get("team/ship-small")
                .map(CatalogueEntry::record_id),
            Some("principle.team.ship-small".into())
        );
        assert!(catalogue.get("pstack/ship-small").is_none());
        assert_eq!(
            catalogue
                .get("prove-it-works")
                .map(CatalogueEntry::record_id),
            Some("principle.prove-it-works".into())
        );
        let refused = catalogue
            .refused
            .iter()
            .map(|refused| (refused.path.as_str(), refused.reason.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(refused.len(), 2, "{refused:?}");
        assert!(refused
            .iter()
            .any(|(path, reason)| path.ends_with("clash.json")
                && reason.contains("unique across packs")));
        assert!(refused
            .iter()
            .any(|(path, _)| path.ends_with("broken.json")));
    }
}
