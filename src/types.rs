use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

pub const ALL_LANGUAGE_META: &str = "all";
pub const ANY_LANGUAGE_META: &str = "any";
pub const SHARED_LANGUAGE_DIR: &str = "shared";

#[allow(dead_code)]
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
        f.write_str(self.as_str())
    }
}

#[allow(dead_code)]
impl Language {
    pub fn as_str(&self) -> &'static str {
        match self {
            Language::Python => "python",
            Language::TypeScript => "typescript",
            Language::Rust => "rust",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageCapabilities {
    pub dependency_manifests: bool,
    pub source_files: bool,
    pub custom_sources: bool,
    pub regex_scan: bool,
    pub tree_sitter: bool,
    pub lint_generation: bool,
    pub formatter_generation: bool,
    pub test_generation: bool,
    pub context_generation: bool,
    pub participates_in_all_scope: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageSpec {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub manifest_names: &'static [&'static str],
    pub source_extensions: &'static [&'static str],
    pub registry: &'static str,
    pub compatible_languages: &'static [&'static str],
    pub lint_tools: &'static [&'static str],
    pub formatter_tools: &'static [&'static str],
    pub test_runners: &'static [&'static str],
    pub capabilities: LanguageCapabilities,
}

const LANGUAGE_SPECS: &[LanguageSpec] = &[
    LanguageSpec {
        id: "python",
        aliases: &["py"],
        manifest_names: &["pyproject.toml", "requirements.txt"],
        source_extensions: &["py", "pyi"],
        registry: "pypi",
        compatible_languages: &["python"],
        lint_tools: &["ruff"],
        formatter_tools: &["ruff"],
        test_runners: &["pytest"],
        capabilities: LanguageCapabilities {
            dependency_manifests: true,
            source_files: true,
            custom_sources: true,
            regex_scan: true,
            tree_sitter: true,
            lint_generation: true,
            formatter_generation: true,
            test_generation: true,
            context_generation: true,
            participates_in_all_scope: true,
        },
    },
    LanguageSpec {
        id: "typescript",
        aliases: &["ts", "tsx"],
        manifest_names: &["package.json"],
        source_extensions: &["ts", "tsx"],
        registry: "npm",
        compatible_languages: &["typescript", "javascript"],
        lint_tools: &["biome"],
        formatter_tools: &["biome"],
        test_runners: &["vitest"],
        capabilities: LanguageCapabilities {
            dependency_manifests: true,
            source_files: true,
            custom_sources: true,
            regex_scan: true,
            tree_sitter: true,
            lint_generation: true,
            formatter_generation: true,
            test_generation: true,
            context_generation: true,
            participates_in_all_scope: true,
        },
    },
    LanguageSpec {
        id: "javascript",
        aliases: &["js", "jsx", "mjs", "cjs"],
        manifest_names: &[],
        source_extensions: &["js", "jsx", "mjs", "cjs"],
        registry: "npm",
        compatible_languages: &["javascript", "typescript"],
        lint_tools: &["biome"],
        formatter_tools: &["biome"],
        test_runners: &["vitest"],
        capabilities: LanguageCapabilities {
            dependency_manifests: false,
            source_files: true,
            custom_sources: true,
            regex_scan: true,
            tree_sitter: true,
            lint_generation: true,
            formatter_generation: true,
            test_generation: true,
            context_generation: true,
            participates_in_all_scope: false,
        },
    },
    LanguageSpec {
        id: "html",
        aliases: &["htm"],
        manifest_names: &[],
        source_extensions: &["html", "htm"],
        registry: "manual",
        compatible_languages: &["html"],
        lint_tools: &[],
        formatter_tools: &[],
        test_runners: &[],
        capabilities: LanguageCapabilities {
            dependency_manifests: false,
            source_files: true,
            custom_sources: true,
            regex_scan: true,
            tree_sitter: false,
            lint_generation: false,
            formatter_generation: false,
            test_generation: false,
            context_generation: true,
            participates_in_all_scope: false,
        },
    },
    LanguageSpec {
        id: "css",
        aliases: &["scss", "sass", "less"],
        manifest_names: &[],
        source_extensions: &["css", "scss", "sass", "less"],
        registry: "manual",
        compatible_languages: &["css"],
        lint_tools: &[],
        formatter_tools: &[],
        test_runners: &[],
        capabilities: LanguageCapabilities {
            dependency_manifests: false,
            source_files: true,
            custom_sources: true,
            regex_scan: true,
            tree_sitter: false,
            lint_generation: false,
            formatter_generation: false,
            test_generation: false,
            context_generation: true,
            participates_in_all_scope: false,
        },
    },
    LanguageSpec {
        id: "rust",
        aliases: &["rs"],
        manifest_names: &["Cargo.toml"],
        source_extensions: &["rs"],
        registry: "crates_io",
        compatible_languages: &["rust"],
        lint_tools: &["clippy"],
        formatter_tools: &["rustfmt"],
        test_runners: &["cargo"],
        capabilities: LanguageCapabilities {
            dependency_manifests: true,
            source_files: true,
            custom_sources: true,
            regex_scan: true,
            tree_sitter: true,
            lint_generation: true,
            formatter_generation: true,
            test_generation: true,
            context_generation: true,
            participates_in_all_scope: true,
        },
    },
];

pub fn language_spec(input: &str) -> Option<&'static LanguageSpec> {
    let canonical = canonical_language(input)?;
    LANGUAGE_SPECS.iter().find(|spec| spec.id == canonical)
}

pub fn canonical_language(input: &str) -> Option<&'static str> {
    let normalized = input.trim().to_ascii_lowercase();
    LANGUAGE_SPECS.iter().find_map(|spec| {
        if spec.id == normalized || spec.aliases.iter().any(|alias| *alias == normalized) {
            Some(spec.id)
        } else {
            None
        }
    })
}

pub fn normalize_language_or_meta(input: &str, meta: &[&'static str]) -> Option<&'static str> {
    let normalized = input.trim().to_ascii_lowercase();
    canonical_language(&normalized).or_else(|| {
        meta.iter()
            .copied()
            .find(|candidate| *candidate == normalized)
    })
}

pub fn supported_language_ids() -> Vec<&'static str> {
    LANGUAGE_SPECS.iter().map(|spec| spec.id).collect()
}

pub fn supported_language_ids_with_meta(meta: &[&'static str]) -> Vec<&'static str> {
    let mut out = supported_language_ids();
    out.extend(meta.iter().copied());
    out
}

pub fn supported_language_display_list(meta: &[&'static str]) -> String {
    supported_language_ids_with_meta(meta).join(", ")
}

pub fn all_supported_languages() -> Vec<String> {
    let mut ids: Vec<&'static str> = LANGUAGE_SPECS
        .iter()
        .filter(|spec| spec.capabilities.participates_in_all_scope)
        .map(|spec| spec.id)
        .collect();
    ids.sort_by_key(|language| match *language {
        "python" => 0,
        "rust" => 1,
        "typescript" => 2,
        _ => 3,
    });
    ids.into_iter().map(String::from).collect()
}

pub fn supported_manifest_names() -> Vec<&'static str> {
    let mut manifests = Vec::new();
    for spec in LANGUAGE_SPECS {
        for manifest in spec.manifest_names {
            if !manifests.contains(manifest) {
                manifests.push(*manifest);
            }
        }
    }
    manifests
}

pub fn supported_manifest_display_list() -> String {
    supported_manifest_names().join(", ")
}

pub fn source_language_for_extension(ext: &str) -> Option<&'static str> {
    let normalized = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    LANGUAGE_SPECS.iter().find_map(|spec| {
        if spec
            .source_extensions
            .iter()
            .any(|candidate| *candidate == normalized)
        {
            Some(spec.id)
        } else {
            None
        }
    })
}

pub fn source_language_for_path(path: &Path) -> Option<&'static str> {
    source_language_for_extension(path.extension()?.to_str()?)
}

pub fn language_matches_language(candidate: &str, target: &str) -> bool {
    let Some(candidate) = canonical_language(candidate) else {
        return false;
    };
    let Some(target) = canonical_language(target) else {
        return false;
    };
    candidate == target
        || language_spec(candidate)
            .map(|spec| spec.compatible_languages.contains(&target))
            .unwrap_or(false)
}

pub fn language_supports_lint_tool(language: &str, tool: &str) -> bool {
    language_spec(language)
        .map(|spec| spec.lint_tools.contains(&tool))
        .unwrap_or(false)
}

pub fn language_supports_formatter_tool(language: &str, tool: &str) -> bool {
    language_spec(language)
        .map(|spec| spec.formatter_tools.contains(&tool))
        .unwrap_or(false)
}

pub fn language_supports_test_runner(language: &str, runner: &str) -> bool {
    language_spec(language)
        .map(|spec| spec.test_runners.contains(&runner))
        .unwrap_or(false)
}

pub fn language_capabilities(language: &str) -> Option<LanguageCapabilities> {
    language_spec(language).map(|spec| spec.capabilities)
}

pub fn registry_for_language(language: &str) -> Option<&'static str> {
    language_spec(language).map(|spec| spec.registry)
}

pub fn language_for_registry(registry: &str) -> Option<&'static str> {
    LANGUAGE_SPECS
        .iter()
        .find(|spec| spec.registry == registry)
        .map(|spec| spec.id)
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn canonical_language_accepts_aliases() {
        assert_eq!(canonical_language("python"), Some("python"));
        assert_eq!(canonical_language("py"), Some("python"));
        assert_eq!(canonical_language("ts"), Some("typescript"));
        assert_eq!(canonical_language("js"), Some("javascript"));
        assert_eq!(canonical_language("javascript"), Some("javascript"));
        assert_eq!(canonical_language("html"), Some("html"));
        assert_eq!(canonical_language("scss"), Some("css"));
        assert_eq!(canonical_language("rs"), Some("rust"));
        assert_eq!(canonical_language("lolcode"), None);
    }

    #[test]
    fn supported_manifest_names_cover_current_languages() {
        let manifests = supported_manifest_names();
        assert_eq!(
            manifests,
            vec![
                "pyproject.toml",
                "requirements.txt",
                "package.json",
                "Cargo.toml"
            ]
        );
    }

    #[test]
    fn registry_round_trip_matches_current_languages() {
        assert_eq!(registry_for_language("python"), Some("pypi"));
        assert_eq!(registry_for_language("javascript"), Some("npm"));
        assert_eq!(registry_for_language("html"), Some("manual"));
        assert_eq!(registry_for_language("rust"), Some("crates_io"));
        assert_eq!(language_for_registry("npm"), Some("typescript"));
        assert_eq!(language_for_registry("manual"), Some("html"));
    }

    #[test]
    fn source_extensions_map_to_profiles() {
        assert_eq!(source_language_for_extension("py"), Some("python"));
        assert_eq!(source_language_for_extension("js"), Some("javascript"));
        assert_eq!(source_language_for_extension("tsx"), Some("typescript"));
        assert_eq!(source_language_for_extension("html"), Some("html"));
        assert_eq!(source_language_for_extension("scss"), Some("css"));
        assert_eq!(source_language_for_extension("rs"), Some("rust"));
        assert_eq!(source_language_for_extension("md"), None);
    }

    #[test]
    fn path_language_detection_uses_extensions() {
        assert_eq!(
            source_language_for_path(&PathBuf::from("web/index.html")),
            Some("html")
        );
        assert_eq!(
            source_language_for_path(&PathBuf::from("web/app.js")),
            Some("javascript")
        );
    }

    #[test]
    fn language_matches_language_uses_compatibility_sets() {
        assert!(language_matches_language("typescript", "javascript"));
        assert!(language_matches_language("javascript", "typescript"));
        assert!(language_matches_language("javascript", "javascript"));
        assert!(!language_matches_language("html", "css"));
    }

    #[test]
    fn normalize_language_or_meta_preserves_meta_languages() {
        assert_eq!(
            normalize_language_or_meta("all", &[ALL_LANGUAGE_META, ANY_LANGUAGE_META]),
            Some(ALL_LANGUAGE_META)
        );
        assert_eq!(
            normalize_language_or_meta("any", &[ALL_LANGUAGE_META, ANY_LANGUAGE_META]),
            Some(ANY_LANGUAGE_META)
        );
        assert_eq!(
            normalize_language_or_meta("js", &[ALL_LANGUAGE_META]),
            Some("javascript")
        );
    }

    #[test]
    fn all_scope_languages_stay_on_current_supported_core() {
        assert_eq!(
            all_supported_languages(),
            vec![
                "python".to_string(),
                "rust".to_string(),
                "typescript".to_string()
            ]
        );
    }

    #[test]
    fn javascript_profile_supports_current_js_tooling() {
        assert!(language_supports_lint_tool("javascript", "biome"));
        assert!(language_supports_formatter_tool("javascript", "biome"));
        assert!(language_supports_test_runner("javascript", "vitest"));
        assert!(language_capabilities("javascript")
            .map(|cap| cap.tree_sitter)
            .unwrap_or(false));
    }
}
