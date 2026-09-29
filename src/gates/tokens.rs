//! Design-token enforcer: literal colours and spacing or type sizes in
//! stylesheets are violations when the token file defines the vocabulary, and
//! each finding suggests the nearest token.
//!
//! Stylesheets are tokenized (comments, strings, blocks, declarations), not
//! matched with a raw regex. Values inside `var(...)` and custom-property
//! definitions in the token file itself are the vocabulary, never findings.

use std::collections::BTreeMap;

use crate::proof::ArtifactFailure;

/// Properties whose lengths must come from tokens.
const SIZE_PROPERTIES: &[&str] = &[
    "margin",
    "margin-top",
    "margin-right",
    "margin-bottom",
    "margin-left",
    "margin-block",
    "margin-block-start",
    "margin-block-end",
    "margin-inline",
    "margin-inline-start",
    "margin-inline-end",
    "padding",
    "padding-top",
    "padding-right",
    "padding-bottom",
    "padding-left",
    "padding-block",
    "padding-block-start",
    "padding-block-end",
    "padding-inline",
    "padding-inline-start",
    "padding-inline-end",
    "gap",
    "row-gap",
    "column-gap",
    "font-size",
    "border-radius",
    "letter-spacing",
    "inset",
    "top",
    "right",
    "bottom",
    "left",
];

const STYLESHEET_EXTENSIONS: &[&str] = &["css", "scss", "less", "pcss"];

/// Whether a path is a stylesheet this check reads.
pub fn is_stylesheet(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        STYLESHEET_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    /// Red, green, blue and alpha, 0–255 / 0–1.
    Colour([f64; 4]),
    /// A length in CSS pixels (rem and em count as 16px).
    Length(f64),
}

/// The token vocabulary: name to parsed value.
#[derive(Debug, Clone, Default)]
pub struct Tokens {
    values: BTreeMap<String, Value>,
}

impl Tokens {
    /// Read `--name: value;` custom properties from a CSS token file, or a
    /// JSON token file (nested objects; leaf strings are values).
    pub fn parse(path: &str, text: &str) -> Result<Self, String> {
        let mut values = BTreeMap::new();
        if path.ends_with(".json") {
            let json: serde_json::Value = serde_json::from_str(text)
                .map_err(|error| format!("the token file is not valid JSON: {error}"))?;
            let mut stack = vec![(String::new(), &json)];
            while let Some((prefix, node)) = stack.pop() {
                match node {
                    serde_json::Value::Object(map) => {
                        for (key, child) in map {
                            if key == "$value" || key == "value" {
                                stack.push((prefix.clone(), child));
                            } else {
                                let name = if prefix.is_empty() {
                                    key.clone()
                                } else {
                                    format!("{prefix}-{key}")
                                };
                                stack.push((name, child));
                            }
                        }
                    }
                    serde_json::Value::String(value) => {
                        if let Some(parsed) = parse_value(value) {
                            values.insert(format!("--{prefix}"), parsed);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            for declaration in declarations(text) {
                if declaration.property.starts_with("--") {
                    if let Some(parsed) = parse_value(&declaration.value) {
                        values.insert(declaration.property.clone(), parsed);
                    }
                }
            }
        }
        if values.is_empty() {
            return Err(
                "the token file defines no colour or size tokens, so there is no vocabulary to check against"
                    .into(),
            );
        }
        Ok(Self { values })
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    fn nearest(&self, literal: &Value) -> Option<(&str, f64)> {
        self.values
            .iter()
            .filter_map(|(name, value)| {
                let distance = match (literal, value) {
                    (Value::Colour(left), Value::Colour(right)) => {
                        let channel = |index: usize| (left[index] - right[index]).powi(2);
                        (channel(0) + channel(1) + channel(2)).sqrt()
                            + (left[3] - right[3]).abs() * 255.0
                    }
                    (Value::Length(left), Value::Length(right)) => (left - right).abs(),
                    _ => return None,
                };
                Some((name.as_str(), distance))
            })
            .min_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    }
}

#[derive(Debug, Clone)]
struct Declaration {
    property: String,
    value: String,
    line: usize,
}

/// Declarations in a stylesheet, skipping comments and strings, with the line
/// each value starts on.
fn declarations(text: &str) -> Vec<Declaration> {
    let bytes = text.as_bytes();
    let mut result = Vec::new();
    let mut index = 0;
    let mut line = 1;
    let mut statement = String::new();
    let mut statement_line = 1;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index < bytes.len()
                && !(bytes[index] == b'*' && bytes.get(index + 1) == Some(&b'/'))
            {
                if bytes[index] == b'\n' {
                    line += 1;
                }
                index += 1;
            }
            index += 2;
            continue;
        }
        if byte == b'"' || byte == b'\'' {
            let quote = byte;
            statement.push(byte as char);
            index += 1;
            while index < bytes.len() && bytes[index] != quote {
                if bytes[index] == b'\\' {
                    index += 1;
                }
                if index < bytes.len() {
                    if bytes[index] == b'\n' {
                        line += 1;
                    }
                    statement.push(bytes[index] as char);
                }
                index += 1;
            }
            statement.push(quote as char);
            index += 1;
            continue;
        }
        match byte {
            b'{' => {
                statement.clear();
                statement_line = line;
            }
            b';' | b'}' => {
                if let Some((property, value)) = statement.split_once(':') {
                    let property = property.trim();
                    if !property.is_empty()
                        && property
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '-')
                    {
                        result.push(Declaration {
                            property: property.to_ascii_lowercase(),
                            value: value.trim().to_string(),
                            line: statement_line,
                        });
                    }
                }
                statement.clear();
                statement_line = line;
            }
            b'\n' => {
                line += 1;
                if statement.trim().is_empty() {
                    statement_line = line;
                }
                statement.push(' ');
            }
            _ => {
                if statement.trim().is_empty() && !byte.is_ascii_whitespace() {
                    statement_line = line;
                }
                statement.push(byte as char);
            }
        }
        index += 1;
    }
    result
}

fn parse_value(value: &str) -> Option<Value> {
    let value = value.trim().trim_end_matches("!important").trim();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex).map(Value::Colour);
    }
    let lower = value.to_ascii_lowercase();
    if let Some(inner) = lower
        .strip_prefix("rgb(")
        .or_else(|| lower.strip_prefix("rgba("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts = inner
            .split(|character: char| {
                character == ',' || character == '/' || character.is_whitespace()
            })
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() >= 3 {
            let channel = |part: &str| -> Option<f64> {
                if let Some(percent) = part.strip_suffix('%') {
                    percent.parse::<f64>().ok().map(|value| value * 2.55)
                } else {
                    part.parse::<f64>().ok()
                }
            };
            let alpha = parts
                .get(3)
                .and_then(|part| {
                    part.strip_suffix('%')
                        .map(|percent| percent.parse::<f64>().ok().map(|value| value / 100.0))
                        .unwrap_or_else(|| part.parse::<f64>().ok())
                })
                .unwrap_or(1.0);
            return Some(Value::Colour([
                channel(parts[0])?,
                channel(parts[1])?,
                channel(parts[2])?,
                alpha,
            ]));
        }
    }
    parse_length(&lower).map(Value::Length)
}

fn parse_hex(hex: &str) -> Option<[f64; 4]> {
    let digits = hex.chars().collect::<Vec<_>>();
    if !digits.iter().all(char::is_ascii_hexdigit) {
        return None;
    }
    let expand = |text: String| u8::from_str_radix(&text, 16).ok().map(f64::from);
    let pair = |at: usize| expand(digits[at..at + 2].iter().collect());
    let single = |at: usize| expand(format!("{0}{0}", digits[at]));
    match digits.len() {
        3 => Some([single(0)?, single(1)?, single(2)?, 1.0]),
        4 => Some([single(0)?, single(1)?, single(2)?, single(3)? / 255.0]),
        6 => Some([pair(0)?, pair(2)?, pair(4)?, 1.0]),
        8 => Some([pair(0)?, pair(2)?, pair(4)?, pair(6)? / 255.0]),
        _ => None,
    }
}

fn parse_length(value: &str) -> Option<f64> {
    for (unit, scale) in [("px", 1.0), ("rem", 16.0), ("em", 16.0)] {
        if let Some(number) = value.strip_suffix(unit) {
            return number.parse::<f64>().ok().map(|number| number * scale);
        }
    }
    None
}

/// Split a declaration value into literal words, skipping `var(...)`,
/// `calc(...)`-style function bodies that only reference tokens, and strings.
fn literal_words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut depth_skip = 0usize;
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    let flush = |current: &mut String, words: &mut Vec<String>| {
        if !current.trim().is_empty() {
            words.push(current.trim().to_string());
        }
        current.clear();
    };
    while let Some(character) = chars.next() {
        if depth_skip > 0 {
            match character {
                '(' => depth_skip += 1,
                ')' => depth_skip -= 1,
                _ => {}
            }
            continue;
        }
        match character {
            '(' => {
                let function = current.trim().to_ascii_lowercase();
                if function == "var" || function == "env" || function == "url" {
                    current.clear();
                    depth_skip = 1;
                } else if function == "rgb" || function == "rgba" {
                    current.push('(');
                    for inner in chars.by_ref() {
                        current.push(inner);
                        if inner == ')' {
                            break;
                        }
                    }
                    flush(&mut current, &mut words);
                } else {
                    // Other functions (calc, clamp, min, max) keep their
                    // literal arguments visible to the check.
                    current.clear();
                }
            }
            ')' | ',' | ' ' | '/' => flush(&mut current, &mut words),
            '"' | '\'' => {
                for inner in chars.by_ref() {
                    if inner == character {
                        break;
                    }
                }
                current.clear();
            }
            _ => current.push(character),
        }
    }
    flush(&mut current, &mut words);
    words
}

/// Findings for one stylesheet. `is_token_file` exempts the vocabulary's own
/// custom-property definitions.
pub fn check_stylesheet(
    path: &str,
    text: &str,
    tokens: &Tokens,
    is_token_file: bool,
    sizes: bool,
) -> Vec<ArtifactFailure> {
    let mut failures = Vec::new();
    for declaration in declarations(text) {
        // The token file's own custom properties are the vocabulary; one
        // outside it must still reuse tokens.
        if declaration.property.starts_with("--") && is_token_file {
            continue;
        }
        let size_property = SIZE_PROPERTIES.contains(&declaration.property.as_str())
            || declaration.property.starts_with("--");
        for word in literal_words(&declaration.value) {
            let Some(literal) = parse_value(&word) else {
                continue;
            };
            let flagged = match literal {
                Value::Colour(_) => true,
                Value::Length(pixels) => sizes && size_property && pixels != 0.0,
            };
            if !flagged {
                continue;
            }
            let suggestion = tokens.nearest(&literal).map_or_else(
                || "a token from the token file".to_string(),
                |(name, _)| format!("var({name})"),
            );
            failures.push(ArtifactFailure {
                location: format!("{path}:{}", declaration.line),
                message: format!(
                    "{}: literal {} in `{}: {}`; use {suggestion}",
                    match literal {
                        Value::Colour(_) => "colour",
                        Value::Length(_) => "size",
                    },
                    word,
                    declaration.property,
                    declaration.value
                ),
            });
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKENS: &str = ":root {\n  --accent: #ff7a00;\n  --ink: rgb(10, 10, 10);\n  --space-2: 8px;\n  --space-4: 1rem;\n}\n";

    fn tokens() -> Tokens {
        Tokens::parse("tokens.css", TOKENS).expect("tokens")
    }

    #[test]
    fn a_raw_colour_is_flagged_with_the_nearest_token() {
        let failures = check_stylesheet(
            "src/app.css",
            "/* #000 in a comment */\n.button {\n  color: #fe7b01;\n  content: \"#123456\";\n}\n",
            &tokens(),
            false,
            true,
        );
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].location, "src/app.css:3");
        assert!(failures[0].message.contains("var(--accent)"));
    }

    #[test]
    fn tokens_and_zero_and_layout_sizes_pass() {
        let failures = check_stylesheet(
            "src/app.css",
            ".card { color: var(--ink); padding: var(--space-2) 0; width: 320px; margin: 0; }",
            &tokens(),
            false,
            true,
        );
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn a_spacing_literal_suggests_the_nearest_size_token() {
        let failures = check_stylesheet("a.css", ".x { gap: 15px; }", &tokens(), false, true);
        assert_eq!(failures.len(), 1);
        assert!(failures[0].message.contains("var(--space-4)"));
    }

    #[test]
    fn the_token_file_defines_the_vocabulary_without_findings() {
        assert!(check_stylesheet("tokens.css", TOKENS, &tokens(), true, true).is_empty());
        assert!(Tokens::parse("empty.css", ".x { color: red; }").is_err());
    }

    #[test]
    fn colour_only_mode_ignores_sizes() {
        let failures = check_stylesheet(
            "a.css",
            ".x { gap: 15px; color: #fff; }",
            &tokens(),
            false,
            false,
        );
        assert_eq!(failures.len(), 1);
        assert!(failures[0].message.starts_with("colour"));
    }

    #[test]
    fn the_dashboard_uses_only_its_colour_tokens() {
        let app = include_str!("../../assets/dashboard/app.css");
        let views = include_str!("../../assets/dashboard/views.css");
        let vocabulary = Tokens::parse("assets/dashboard/app.css", app).expect("dashboard tokens");
        let mut failures =
            check_stylesheet("assets/dashboard/app.css", app, &vocabulary, true, false);
        failures.extend(check_stylesheet(
            "assets/dashboard/views.css",
            views,
            &vocabulary,
            false,
            false,
        ));
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn json_token_files_are_read_as_a_vocabulary() {
        let tokens = Tokens::parse(
            "tokens.json",
            r##"{"color": {"accent": {"$value": "#ff7a00"}}, "space": {"2": "8px"}}"##,
        )
        .expect("tokens");
        assert_eq!(tokens.len(), 2);
    }
}
