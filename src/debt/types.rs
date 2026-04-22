//! Schema for the `wh debt` JSON envelope and internal finding types.
//! See `planning/debt.md` §"JSON schema — evidence envelope".

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Dead,
    Dup,
    Deps,
    Hotspots,
}

impl Category {
    pub fn base_weight(self) -> f64 {
        match self {
            Category::Dead => 1.0,
            Category::Dup => 0.8,
            Category::Deps => 1.0,
            Category::Hotspots => 1.2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Category::Dead => "dead",
            Category::Dup => "dup",
            Category::Deps => "deps",
            Category::Hotspots => "hotspots",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Confidence {
    // Order matters: ordering `Medium < High` lets `confidence >= min` comparisons work.
    Medium,
    High,
}

impl Confidence {
    pub fn factor(self) -> f64 {
        match self {
            Confidence::High => 1.0,
            Confidence::Medium => 0.6,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::High => "high",
            Confidence::Medium => "medium",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DebtLabel {
    Low,
    Moderate,
    Elevated,
    High,
}

impl DebtLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            DebtLabel::Low => "low",
            DebtLabel::Moderate => "moderate",
            DebtLabel::Elevated => "elevated",
            DebtLabel::High => "high",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// Evidence kinds. `kind` tags the variant so JSON consumers can switch on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Evidence {
    ManifestEntry {
        snippet: String,
        references: u32,
        locations: Vec<Location>,
    },
    SymbolDef {
        name: String,
        symbol_kind: String,
        references: u32,
        locations: Vec<Location>,
    },
    DuplicateCluster {
        snippet: String,
        normalized_lines: u32,
        occurrences: u32,
        locations: Vec<Location>,
    },
    OrphanedFile {
        path: String,
        locations: Vec<Location>,
    },
    ChurnViolationIntersection {
        changes: u32,
        violations: u32,
        window_days: u32,
        locations: Vec<Location>,
    },
}


/// A single detector finding before ranking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub category: Category,
    /// `<category>.<name>` (e.g. `dead.unused_declared_deps`).
    pub rule_id: String,
    pub title: String,
    pub confidence: Confidence,
    /// Evidence strength before category/confidence weighting (see design doc).
    pub evidence_strength: f64,
    pub files: Vec<String>,
    pub evidence: Evidence,
    pub next_action: String,
}

/// A ranked finding in the final report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hotspot {
    /// Stable synthetic id within the report (`h1`, `h2`, ...).
    pub id: String,
    pub category: Category,
    pub rule_id: String,
    pub title: String,
    pub confidence: Confidence,
    pub rank: u32,
    pub score: f64,
    pub files: Vec<String>,
    pub evidence: Evidence,
    pub next_action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub debt_label: DebtLabel,
    pub hotspot_count: u32,
    pub finding_count: u32,
    pub by_category: BycatCounts,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BycatCounts {
    #[serde(default)]
    pub dead: u32,
    #[serde(default)]
    pub dup: u32,
    #[serde(default)]
    pub deps: u32,
    #[serde(default)]
    pub hotspots: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebtReport {
    pub schema_version: u32,
    pub generated_at: String,
    pub project_dir: String,
    pub summary: Summary,
    pub hotspots: Vec<Hotspot>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// The source files collected for detector passes. Pre-split by language
/// so detectors can iterate the slice they care about without re-walking.
#[derive(Debug, Default, Clone)]
pub struct SourceInventory {
    pub python: Vec<PathBuf>,
    pub typescript: Vec<PathBuf>,
    pub rust: Vec<PathBuf>,
}

impl SourceInventory {
    pub fn all(&self) -> impl Iterator<Item = &PathBuf> {
        self.python
            .iter()
            .chain(self.typescript.iter())
            .chain(self.rust.iter())
    }
}
