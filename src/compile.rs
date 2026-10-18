//! Compile prose into candidate rules, without a model.
//!
//! Teams already write rules down: CONTRIBUTING, AGENTS.md, a style guide,
//! review comments. This splits such text into candidate rules (list items,
//! and sentences that use rule words such as must, never or should) and, for
//! each, infers an enforcer only where the text names one outright: a
//! formatter, a lint code, a command the repository runs, a script under
//! `whetstone/checks/`, or a construct with a known AST query. Everything else
//! is returned as needing an enforcer, with its prose kept, for the user's
//! agent to complete. Whetstone never guesses with a model: the agent
//! compiles, the probe validates and a person accepts.

use std::path::Path;

use serde::Serialize;

use crate::domain::{Enforcer, Strength};

/// The most candidates one input yields.
pub const MAX_CANDIDATES: usize = 64;
/// The largest prose input read.
pub const MAX_INPUT_BYTES: usize = 256 * 1024;

/// One rule found in prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    /// The 1-based line the rule starts on.
    pub line: usize,
    /// The text exactly as written (bullets and emphasis removed).
    pub prose: String,
    /// The strength its wording suggests; the person decides.
    pub strength: Strength,
    /// The enforcer the text names, if any.
    pub enforcer: Option<Enforcer>,
    /// Which row of the inference table chose the enforcer.
    pub inferred: Option<String>,
    /// Directories the text names, as path globs.
    pub paths: Vec<String>,
}

/// Rule words: a sentence outside a list counts only when it uses one.
const RULE_WORDS: [&str; 16] = [
    "must",
    "never",
    "always",
    "should",
    "shall",
    "avoid",
    "prefer",
    "don't",
    "do not",
    "required",
    "forbidden",
    "not allowed",
    "only",
    "no ",
    "keep",
    "run ",
];
/// Words that make a rule a must rather than a should.
const MUST_WORDS: [&str; 10] = [
    "must",
    "never",
    "always",
    "shall",
    "required",
    "forbidden",
    "not allowed",
    "do not",
    "don't",
    "no ",
];

/// Split prose into candidates and infer what each names.
pub fn candidates(project_root: &Path, text: &str) -> Vec<Candidate> {
    split(text)
        .into_iter()
        .map(|(line, prose)| {
            let (enforcer, inferred) = match infer(project_root, &prose) {
                Some((enforcer, row)) => (Some(enforcer), Some(row)),
                None => (None, None),
            };
            Candidate {
                line,
                strength: strength_of(&prose),
                paths: named_paths(project_root, &prose),
                prose,
                enforcer,
                inferred,
            }
        })
        .collect()
}

/// List items, and sentences with a rule word, with their line numbers.
/// Headings, code blocks, tables, HTML comments and Whetstone's own contract
/// block are skipped.
pub fn split(text: &str) -> Vec<(usize, String)> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut in_code = false;
    let mut in_contract = false;
    let mut paragraph: Vec<(usize, String)> = Vec::new();
    let mut item: Option<(usize, String)> = None;
    let flush_paragraph = |paragraph: &mut Vec<(usize, String)>,
                           found: &mut Vec<(usize, String)>| {
        if paragraph.is_empty() {
            return;
        }
        let start = paragraph[0].0;
        let joined = paragraph
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        for sentence in sentences(&joined) {
            if has_rule_word(&sentence) && word_count(&sentence) >= 3 {
                found.push((start, sentence));
            }
        }
        paragraph.clear();
    };
    let flush_item = |item: &mut Option<(usize, String)>, found: &mut Vec<(usize, String)>| {
        if let Some((line, text)) = item.take() {
            let text = clean(&text);
            if word_count(&text) >= 3 {
                found.push((line, text));
            }
        }
    };
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with(crate::contract::START) {
            in_contract = true;
        }
        if in_contract {
            if trimmed.starts_with(crate::contract::END) {
                in_contract = false;
            }
            continue;
        }
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            flush_item(&mut item, &mut found);
            flush_paragraph(&mut paragraph, &mut found);
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        if trimmed.is_empty()
            || trimmed.starts_with('#')
            || trimmed.starts_with('|')
            || trimmed.starts_with("<!--")
            || trimmed.starts_with('>')
            || trimmed.chars().all(|character| "-=*_ ".contains(character))
        {
            flush_item(&mut item, &mut found);
            flush_paragraph(&mut paragraph, &mut found);
            continue;
        }
        if let Some(rest) = list_item(trimmed) {
            flush_item(&mut item, &mut found);
            flush_paragraph(&mut paragraph, &mut found);
            item = Some((number, rest.to_string()));
            continue;
        }
        match item.as_mut() {
            // An indented line continues the list item above it.
            Some((_, text)) if line.starts_with(char::is_whitespace) => {
                text.push(' ');
                text.push_str(trimmed);
            }
            _ => {
                flush_item(&mut item, &mut found);
                paragraph.push((number, trimmed.to_string()));
            }
        }
        if found.len() >= MAX_CANDIDATES {
            break;
        }
    }
    flush_item(&mut item, &mut found);
    flush_paragraph(&mut paragraph, &mut found);
    found.truncate(MAX_CANDIDATES);
    found
}

fn list_item(line: &str) -> Option<&str> {
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest.trim_start_matches("[ ] ").trim_start_matches("[x] "));
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        let rest = &line[digits..];
        if let Some(rest) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some(rest);
        }
    }
    None
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let characters = text.chars().collect::<Vec<_>>();
    for (index, character) in characters.iter().enumerate() {
        current.push(*character);
        if *character == '`' {
            quoted = !quoted;
        }
        let ends = matches!(character, '.' | '!' | '?')
            && !quoted
            && characters
                .get(index + 1)
                .map_or(true, |next| next.is_whitespace());
        if ends {
            out.push(clean(&current));
            current.clear();
        }
    }
    if !current.trim().is_empty() {
        out.push(clean(&current));
    }
    out.into_iter()
        .filter(|sentence| !sentence.is_empty())
        .collect()
}

fn clean(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("**", "")
        .replace("__", "")
        .trim()
        .to_string()
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn lower(text: &str) -> String {
    format!(" {} ", text.to_ascii_lowercase().replace('\u{2019}', "'"))
}

fn has_rule_word(text: &str) -> bool {
    let text = lower(text);
    RULE_WORDS.iter().any(|word| contains_word(&text, word))
}

fn contains_word(text: &str, word: &str) -> bool {
    let word = word.trim_end();
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back().unwrap_or(' ');
        let after = text[at + word.len()..].chars().next().unwrap_or(' ');
        !before.is_ascii_alphanumeric() && !after.is_ascii_alphanumeric()
    })
}

/// must when the wording is absolute; should otherwise.
pub fn strength_of(prose: &str) -> Strength {
    let text = lower(prose);
    if MUST_WORDS.iter().any(|word| contains_word(&text, word)) {
        Strength::Must
    } else {
        Strength::Should
    }
}

/// Repository directories the prose names in backticks (`src/`, `tests`).
fn named_paths(project_root: &Path, prose: &str) -> Vec<String> {
    let mut paths = code_spans(prose)
        .into_iter()
        .map(|span| span.trim_matches('/').trim_start_matches("./").to_string())
        .filter(|span| {
            !span.is_empty()
                && !span.contains(char::is_whitespace)
                && crate::domain::safe_relative_path(span)
                && project_root.join(span).is_dir()
        })
        .map(|span| format!("{span}/**"))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    paths
}

fn code_spans(prose: &str) -> Vec<String> {
    prose
        .split('`')
        .enumerate()
        .filter(|(index, _)| index % 2 == 1)
        .map(|(_, span)| span.trim().to_string())
        .filter(|span| !span.is_empty())
        .collect()
}

/// The languages the repository is written in, by the files at its root.
fn languages(project_root: &Path) -> Vec<&'static str> {
    let mut found = Vec::new();
    if project_root.join("Cargo.toml").is_file() {
        found.push("rust");
    }
    if project_root.join("package.json").is_file() || project_root.join("tsconfig.json").is_file() {
        found.push("typescript");
    }
    if project_root.join("pyproject.toml").is_file()
        || project_root.join("setup.py").is_file()
        || project_root.join("requirements.txt").is_file()
    {
        found.push("python");
    }
    found
}

/// Programs whose backticked commands are taken as the rule's own check.
const RUNNERS: [&str; 22] = [
    "cargo", "npm", "pnpm", "yarn", "bun", "npx", "make", "just", "task", "python", "python3",
    "pytest", "go", "ruff", "black", "eslint", "biome", "prettier", "tox", "uv", "deno", "sh",
];

/// The enforcer the prose names outright, with the inference row that chose it.
pub fn infer(project_root: &Path, prose: &str) -> Option<(Enforcer, String)> {
    let text = lower(prose);
    let has = |word: &str| contains_word(&text, word);
    let spans = code_spans(prose);
    // 1. A script under whetstone/checks.
    for span in &spans {
        let path = span.trim_start_matches("./");
        if path.starts_with(crate::domain::SCRIPTS_DIR) && !path.contains(char::is_whitespace) {
            return Some((
                Enforcer::Validator {
                    command: format!("sh {path}"),
                },
                "names a script under whetstone/checks".into(),
            ));
        }
    }
    // 2. A formatter.
    let formatter = if has("rustfmt") || has("cargo fmt") {
        Some("cargo fmt --all -- --check")
    } else if has("ruff format") {
        Some("ruff format --check .")
    } else if has("black") && (has("format") || has("formatted")) {
        Some("black --check .")
    } else if has("prettier") {
        Some("npx prettier --check .")
    } else if has("biome format") {
        Some("npx biome format .")
    } else {
        None
    };
    if let Some(tool) = formatter {
        return Some((
            Enforcer::Formatter { tool: tool.into() },
            "names a formatter".into(),
        ));
    }
    // 3. A lint code.
    for token in prose
        .split(|character: char| character.is_whitespace() || "`(),;\"'".contains(character))
        .map(|token| token.trim_end_matches(['.', ':']))
    {
        if let Some(name) = token.strip_prefix("clippy::") {
            if !name.is_empty() {
                return Some((
                    Enforcer::Lint {
                        tool: "cargo clippy --all-targets".into(),
                        code: format!("clippy::{name}"),
                    },
                    "names a clippy lint".into(),
                ));
            }
        }
        let letters = token.chars().take_while(char::is_ascii_uppercase).count();
        let digits = token.len() - letters;
        if (1..=3).contains(&letters)
            && (3..=4).contains(&digits)
            && token[letters..]
                .chars()
                .all(|character| character.is_ascii_digit())
            && (has("ruff")
                || has("flake8")
                || has("lint")
                || languages(project_root).contains(&"python"))
        {
            return Some((
                Enforcer::Lint {
                    tool: "ruff check .".into(),
                    code: token.into(),
                },
                "names a ruff code".into(),
            ));
        }
    }
    if has("eslint") {
        if let Some(rule) = spans.iter().find(|span| {
            span.contains('-')
                && span
                    .chars()
                    .all(|character| character.is_ascii_lowercase() || "-/@".contains(character))
        }) {
            return Some((
                Enforcer::Lint {
                    tool: "npx eslint .".into(),
                    code: rule.clone(),
                },
                "names an eslint rule".into(),
            ));
        }
    }
    // 4. A command the repository runs, in backticks.
    for span in &spans {
        let program = span.split_whitespace().next().unwrap_or_default();
        if RUNNERS.contains(&program) && crate::gates::parse_command(span).is_ok() {
            let test = span.contains("test") || program == "pytest" || program == "tox";
            return Some((
                if test {
                    Enforcer::Test {
                        command: span.clone(),
                    }
                } else {
                    Enforcer::Validator {
                        command: span.clone(),
                    }
                },
                "names a command the repository runs".into(),
            ));
        }
    }
    // 5. The test suite, by name.
    if (has("tests") || has("test suite")) && (has("pass") || has("run") || has("green")) {
        if let Some((command, from)) = crate::starter::detected_test_command(project_root) {
            return Some((
                Enforcer::Test { command },
                format!("asks for the test suite ({from})"),
            ));
        }
    }
    // 6. A construct with a known AST query.
    construct(project_root, &text)
}

/// Named constructs with a known query, per language.
fn construct(project_root: &Path, text: &str) -> Option<(Enforcer, String)> {
    let languages = languages(project_root);
    let has = |word: &str| text.contains(word);
    let rust_method = |method: &str| {
        format!(
            "((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method \"{method}\"))"
        )
    };
    let rust_macro = |name: &str| {
        format!("((macro_invocation macro: (identifier) @name) @match (#eq? @name \"{name}\"))")
    };
    let mut table: Vec<(&str, bool, String, &str)> = Vec::new();
    if languages.contains(&"rust") {
        table.extend([
            (
                "rust",
                has("unwrap"),
                rust_method("unwrap"),
                "names .unwrap()",
            ),
            (
                "rust",
                has(".expect(") || has("expect()"),
                rust_method("expect"),
                "names .expect()",
            ),
            ("rust", has("dbg!"), rust_macro("dbg"), "names dbg!"),
            ("rust", has("todo!"), rust_macro("todo"), "names todo!"),
            (
                "rust",
                has("unimplemented!"),
                rust_macro("unimplemented"),
                "names unimplemented!",
            ),
            (
                "rust",
                has("println!"),
                rust_macro("println"),
                "names println!",
            ),
            (
                "rust",
                has("unsafe"),
                "(unsafe_block) @match".to_string(),
                "names unsafe blocks",
            ),
        ]);
    }
    if languages.contains(&"typescript") {
        table.extend([
            (
                "typescript",
                has("console.log"),
                "((call_expression function: (member_expression object: (identifier) @object property: (property_identifier) @property)) @match (#eq? @object \"console\") (#eq? @property \"log\"))".to_string(),
                "names console.log",
            ),
            (
                "typescript",
                has("debugger"),
                "(debugger_statement) @match".to_string(),
                "names debugger statements",
            ),
            (
                "typescript",
                has(" any ") || has("`any`") || has(": any"),
                "((predefined_type) @match (#eq? @match \"any\"))".to_string(),
                "names the any type",
            ),
        ]);
    }
    if languages.contains(&"python") {
        table.extend([(
            "python",
            has("print(") || has("print statement") || has("print calls"),
            "((call function: (identifier) @function) @match (#eq? @function \"print\"))"
                .to_string(),
            "names print()",
        )]);
    }
    let eval = has("eval(") || has("`eval`") || has(" eval ");
    for language in ["typescript", "python"] {
        if eval && languages.contains(&language) {
            let query = if language == "python" {
                "((call function: (identifier) @function) @match (#eq? @function \"eval\"))"
            } else {
                "((call_expression function: (identifier) @function) @match (#eq? @function \"eval\"))"
            };
            table.push((language, true, query.to_string(), "names eval()"));
        }
    }
    table
        .into_iter()
        .find(|(_, named, _, _)| *named)
        .map(|(language, _, query, row)| {
            (
                Enforcer::Ast {
                    query,
                    language: Some(language.into()),
                },
                row.into(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTRIBUTING: &str = "# Contributing\n\nThanks for helping. Please be kind.\n\n## Rules\n\n- Format with `cargo fmt` before every commit.\n- Never call `.unwrap()` in library code under `src`.\n- All tests must pass: run `cargo test`.\n- Clippy's clippy::dbg_macro lint must stay clean.\n- Keep pull requests small enough to review in one sitting.\n- Prefer descriptive names over abbreviations.\n\n```sh\n- not a rule: this is code\n```\n\nYou should document every public function.\n";

    #[test]
    fn prose_splits_into_list_items_and_rule_sentences_only() {
        let found = split(CONTRIBUTING);
        let texts = found
            .iter()
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            texts,
            [
                "Format with `cargo fmt` before every commit.",
                "Never call `.unwrap()` in library code under `src`.",
                "All tests must pass: run `cargo test`.",
                "Clippy's clippy::dbg_macro lint must stay clean.",
                "Keep pull requests small enough to review in one sitting.",
                "Prefer descriptive names over abbreviations.",
                "You should document every public function.",
            ]
        );
        assert_eq!(found[0].0, 7);
        assert_eq!(found[6].0, 18);
    }

    #[test]
    fn the_inference_table_names_only_what_the_text_names() {
        let temp = tempfile::tempdir().expect("temp");
        std::fs::write(temp.path().join("Cargo.toml"), "[package]\nname = \"x\"\n").expect("cargo");
        std::fs::create_dir_all(temp.path().join("src")).expect("src");
        let found = candidates(temp.path(), CONTRIBUTING);
        let kinds = found
            .iter()
            .map(|candidate| candidate.enforcer.as_ref().map(Enforcer::kind))
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            [
                Some("formatter"),
                Some("ast"),
                Some("test"),
                Some("lint"),
                None,
                None,
                None,
            ]
        );
        assert_eq!(found[1].paths, ["src/**"]);
        assert_eq!(found[1].strength, Strength::Must);
        assert_eq!(found[5].strength, Strength::Should);
        assert!(
            matches!(&found[3].enforcer, Some(Enforcer::Lint { code, .. }) if code == "clippy::dbg_macro")
        );
    }

    #[test]
    fn every_construct_query_compiles_for_its_language() {
        let temp = tempfile::tempdir().expect("temp");
        for file in ["Cargo.toml", "package.json", "pyproject.toml"] {
            std::fs::write(temp.path().join(file), "{}").expect("marker");
        }
        for prose in [
            "never .unwrap()",
            "no .expect( calls",
            "no dbg! left",
            "no todo! left",
            "no unimplemented! left",
            "no println! in libraries",
            "no unsafe code",
            "never console.log",
            "never debugger",
            "never use `any`",
            "no print( calls",
            "never eval(",
        ] {
            let (enforcer, _) =
                construct(temp.path(), &lower(prose)).unwrap_or_else(|| panic!("{prose}"));
            let Enforcer::Ast { query, language } = enforcer else {
                panic!("{prose}");
            };
            let lang = crate::ast::AstLang::from_str(language.as_deref().expect("language"))
                .expect("language");
            crate::ast::compile_query(lang, &query)
                .unwrap_or_else(|error| panic!("{prose}: {error}"));
        }
    }

    #[test]
    fn the_contract_block_is_never_read_as_rules() {
        let text = format!(
            "# Ours\n\n- Ship it small.\n\n{}",
            crate::contract::block("origin/main")
        );
        let found = split(&text);
        assert_eq!(found.len(), 1, "{found:?}");
    }
}
