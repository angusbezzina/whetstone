use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    Python,
    #[serde(alias = "typescript")]
    TypeScript,
    Rust,
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Language::Python => write!(f, "python"),
            Language::TypeScript => write!(f, "typescript"),
            Language::Rust => write!(f, "rust"),
        }
    }
}

impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::Rust => "rust",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: String,
    pub version: String,
    pub language: Language,
    pub dev: bool,
    #[serde(default)]
    pub sources: Vec<String>,
    /// Internal ranking score (stripped from output).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _score: Option<f64>,
}

/// Lifecycle states for dependency inventory tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Discovered,
    Queued,
    Resolving,
    Resolved,
    ExtractionReady,
    Extracted,
    Approved,
    Stale,
    Failed,
}

impl LifecycleState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Discovered => "discovered",
            Self::Queued => "queued",
            Self::Resolving => "resolving",
            Self::Resolved => "resolved",
            Self::ExtractionReady => "extraction_ready",
            Self::Extracted => "extracted",
            Self::Approved => "approved",
            Self::Stale => "stale",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "discovered" => Self::Discovered,
            "queued" => Self::Queued,
            "resolving" => Self::Resolving,
            "resolved" => Self::Resolved,
            "extraction_ready" => Self::ExtractionReady,
            "extracted" => Self::Extracted,
            "approved" => Self::Approved,
            "stale" => Self::Stale,
            "failed" => Self::Failed,
            _ => Self::Discovered,
        }
    }
}

impl fmt::Display for LifecycleState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
