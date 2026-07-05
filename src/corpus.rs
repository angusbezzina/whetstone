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
    /// "starter" (a dependency pack, auto-importable by `wh init --claude`) or
    /// "resource" (a public style-guide pack, opt-in via the wizard picker).
    pub kind: &'static str,
}

pub const PACKS: &[BundledPack] = &[
    BundledPack {
        language: "python",
        dep: "fastapi",
        yaml: include_str!("../packs/python/fastapi.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "python",
        dep: "pydantic",
        yaml: include_str!("../packs/python/pydantic.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "python",
        dep: "sqlalchemy",
        yaml: include_str!("../packs/python/sqlalchemy.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "python",
        dep: "httpx",
        yaml: include_str!("../packs/python/httpx.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "typescript",
        dep: "react",
        yaml: include_str!("../packs/typescript/react.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "typescript",
        dep: "next",
        yaml: include_str!("../packs/typescript/next.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "typescript",
        dep: "zod",
        yaml: include_str!("../packs/typescript/zod.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "typescript",
        dep: "express",
        yaml: include_str!("../packs/typescript/express.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "rust",
        dep: "axum",
        yaml: include_str!("../packs/rust/axum.yaml"),
        kind: "starter",
    },
    BundledPack {
        language: "rust",
        dep: "serde",
        yaml: include_str!("../packs/rust/serde.yaml"),
        kind: "starter",
    },
    // ── Resource packs (public style guides; opt-in via the wizard) ──
    BundledPack {
        language: "typescript",
        dep: "airbnb-js",
        yaml: include_str!("../packs/resources/airbnb-js.yaml"),
        kind: "resource",
    },
];

/// The bundled STARTER pack for a `(language, dependency)` pair, if the corpus
/// covers it. Resource packs are excluded — they are opt-in, never matched to a
/// detected dependency for auto-import.
pub fn for_dep(language: &str, dep: &str) -> Option<&'static BundledPack> {
    PACKS.iter().find(|p| {
        p.kind == "starter"
            && p.language.eq_ignore_ascii_case(language)
            && p.dep.eq_ignore_ascii_case(dep)
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
                kind: p.kind,
                yaml: p.yaml,
            }
        })
        .collect()
}
