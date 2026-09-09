//! Language identifiers shared by the retained rule and scanner kernel.

use std::path::Path;

pub const SHARED_LANGUAGE_DIR: &str = "shared";

struct LanguageSpec {
    id: &'static str,
    aliases: &'static [&'static str],
    extensions: &'static [&'static str],
    compatible: &'static [&'static str],
}

const LANGUAGES: &[LanguageSpec] = &[
    LanguageSpec {
        id: "python",
        aliases: &["py"],
        extensions: &["py", "pyi"],
        compatible: &["python"],
    },
    LanguageSpec {
        id: "typescript",
        aliases: &["ts", "tsx"],
        extensions: &["ts", "tsx"],
        compatible: &["typescript", "javascript"],
    },
    LanguageSpec {
        id: "javascript",
        aliases: &["js", "jsx", "mjs", "cjs"],
        extensions: &["js", "jsx", "mjs", "cjs"],
        compatible: &["javascript", "typescript"],
    },
    LanguageSpec {
        id: "html",
        aliases: &["htm"],
        extensions: &["html", "htm"],
        compatible: &["html"],
    },
    LanguageSpec {
        id: "css",
        aliases: &["scss", "sass", "less"],
        extensions: &["css", "scss", "sass", "less"],
        compatible: &["css"],
    },
    LanguageSpec {
        id: "rust",
        aliases: &["rs"],
        extensions: &["rs"],
        compatible: &["rust"],
    },
];

pub fn canonical_language(input: &str) -> Option<&'static str> {
    let normalized = input.trim().to_ascii_lowercase();
    LANGUAGES.iter().find_map(|language| {
        (language.id == normalized || language.aliases.iter().any(|alias| *alias == normalized))
            .then_some(language.id)
    })
}

pub fn all_supported_languages() -> Vec<String> {
    LANGUAGES
        .iter()
        .map(|language| language.id.to_string())
        .collect()
}

pub fn source_language_for_extension(extension: &str) -> Option<&'static str> {
    let normalized = extension.trim_start_matches('.').to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|language| {
            language
                .extensions
                .iter()
                .any(|candidate| *candidate == normalized)
        })
        .map(|language| language.id)
}

pub fn source_language_for_path(path: &Path) -> Option<&'static str> {
    path.extension()
        .and_then(|extension| extension.to_str())
        .and_then(source_language_for_extension)
}

pub fn language_matches_language(candidate: &str, target: &str) -> bool {
    let candidate = canonical_language(candidate).unwrap_or(candidate);
    let target = canonical_language(target).unwrap_or(target);
    candidate == target
        || LANGUAGES
            .iter()
            .find(|language| language.id == candidate)
            .is_some_and(|language| language.compatible.contains(&target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_and_compatible_languages_are_canonical() {
        assert_eq!(canonical_language("TSX"), Some("typescript"));
        assert!(language_matches_language("javascript", "typescript"));
        assert!(language_matches_language("typescript", "js"));
        assert!(!language_matches_language("rust", "python"));
    }

    #[test]
    fn paths_map_only_supported_source_extensions() {
        assert_eq!(
            source_language_for_path(Path::new("src/main.rs")),
            Some("rust")
        );
        assert_eq!(source_language_for_path(Path::new("README.md")), None);
    }
}
