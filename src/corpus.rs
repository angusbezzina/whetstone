//! The trusted rule corpus, embedded into the binary (whetstone-0cj).
//!
//! `wh init --claude` imports the starter packs that match a project's detected
//! dependencies. Embedding them via `include_str!` means that works offline and
//! from an installed binary with no repo checkout. The source of truth is
//! `packs/<lang>/<dep>.yaml`; `tests/corpus_packs.rs` keeps them eval-clean, and
//! `tests/corpus_bundle.rs` keeps this list in sync with the directory.

pub struct BundledPack {
    pub language: &'static str,
    pub dep: &'static str,
    pub yaml: &'static str,
}

pub const PACKS: &[BundledPack] = &[
    BundledPack {
        language: "python",
        dep: "fastapi",
        yaml: include_str!("../packs/python/fastapi.yaml"),
    },
    BundledPack {
        language: "python",
        dep: "pydantic",
        yaml: include_str!("../packs/python/pydantic.yaml"),
    },
    BundledPack {
        language: "python",
        dep: "sqlalchemy",
        yaml: include_str!("../packs/python/sqlalchemy.yaml"),
    },
    BundledPack {
        language: "python",
        dep: "httpx",
        yaml: include_str!("../packs/python/httpx.yaml"),
    },
    BundledPack {
        language: "typescript",
        dep: "react",
        yaml: include_str!("../packs/typescript/react.yaml"),
    },
    BundledPack {
        language: "typescript",
        dep: "next",
        yaml: include_str!("../packs/typescript/next.yaml"),
    },
    BundledPack {
        language: "typescript",
        dep: "zod",
        yaml: include_str!("../packs/typescript/zod.yaml"),
    },
    BundledPack {
        language: "typescript",
        dep: "express",
        yaml: include_str!("../packs/typescript/express.yaml"),
    },
    BundledPack {
        language: "rust",
        dep: "axum",
        yaml: include_str!("../packs/rust/axum.yaml"),
    },
    BundledPack {
        language: "rust",
        dep: "serde",
        yaml: include_str!("../packs/rust/serde.yaml"),
    },
];

/// The bundled pack for a `(language, dependency)` pair, if the corpus covers it.
pub fn for_dep(language: &str, dep: &str) -> Option<&'static BundledPack> {
    PACKS.iter().find(|p| {
        p.language.eq_ignore_ascii_case(language) && p.dep.eq_ignore_ascii_case(dep)
    })
}

/// A browsable summary of a bundled pack (whetstone-dyg): the onboarding packs
/// picker renders this without re-parsing YAML in the view.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
    pub dep: &'static str,
    pub language: &'static str,
    pub name: String,
    pub rule_count: usize,
    /// "starter" (dependency pack) or "resource" (style-guide pack).
    pub kind: &'static str,
    pub yaml: &'static str,
}

/// Parse every bundled pack into a browsable catalog entry.
pub fn catalog() -> Vec<CatalogEntry> {
    PACKS
        .iter()
        .map(|p| {
            let parsed: Option<crate::config_packs::RulePackFile> = serde_yaml::from_str(p.yaml).ok();
            let (name, rule_count) = parsed
                .as_ref()
                .map(|pk| {
                    (
                        pk.metadata.name.clone().unwrap_or_else(|| p.dep.to_string()),
                        pk.rules.iter().filter(|r| r.approved).count(),
                    )
                })
                .unwrap_or_else(|| (p.dep.to_string(), 0));
            CatalogEntry {
                dep: p.dep,
                language: p.language,
                name,
                rule_count,
                kind: "starter",
                yaml: p.yaml,
            }
        })
        .collect()
}
