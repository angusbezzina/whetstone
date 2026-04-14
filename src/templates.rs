//! Tera template engine for Whetstone code generation.
//!
//! All context files (CLAUDE.md, AGENTS.md, etc.), eval test files, and linter
//! configs are rendered from embedded `.tera` templates. Language-specific
//! regex escaping is handled by custom Tera filters so templates can embed a
//! raw `match:` pattern without worrying about target-language quoting rules.

use std::collections::HashMap;
use tera::{Context, Tera, Value};

fn register(tera: &mut Tera, name: &str, text: &str) {
    tera.add_raw_template(name, text)
        .unwrap_or_else(|e| panic!("failed to register template {name}: {e}"));
}

/// Build a Tera registry with every embedded Whetstone template and the
/// language-escape filters used by the signal renderers.
pub fn build_tera() -> Tera {
    let mut tera = Tera::default();
    tera.autoescape_on(Vec::new());

    register(
        &mut tera,
        "_context_body.tera",
        include_str!("templates/_context_body.tera"),
    );
    register(
        &mut tera,
        "claude_md.tera",
        include_str!("templates/claude_md.tera"),
    );
    register(
        &mut tera,
        "agents_md.tera",
        include_str!("templates/agents_md.tera"),
    );
    register(
        &mut tera,
        "cursorrules.tera",
        include_str!("templates/cursorrules.tera"),
    );
    register(
        &mut tera,
        "copilot_md.tera",
        include_str!("templates/copilot_md.tera"),
    );
    register(
        &mut tera,
        "windsurfrules.tera",
        include_str!("templates/windsurfrules.tera"),
    );
    register(
        &mut tera,
        "codex_md.tera",
        include_str!("templates/codex_md.tera"),
    );
    register(
        &mut tera,
        "python_test.py.tera",
        include_str!("templates/python_test.py.tera"),
    );
    register(
        &mut tera,
        "typescript_test.ts.tera",
        include_str!("templates/typescript_test.ts.tera"),
    );
    register(
        &mut tera,
        "rust_test.rs.tera",
        include_str!("templates/rust_test.rs.tera"),
    );
    register(
        &mut tera,
        "python_conftest.py.tera",
        include_str!("templates/python_conftest.py.tera"),
    );
    register(
        &mut tera,
        "typescript_setup.ts.tera",
        include_str!("templates/typescript_setup.ts.tera"),
    );
    register(
        &mut tera,
        "ruff_config.tera",
        include_str!("templates/ruff_config.tera"),
    );
    register(
        &mut tera,
        "biome_config.tera",
        include_str!("templates/biome_config.tera"),
    );
    register(
        &mut tera,
        "clippy_config.tera",
        include_str!("templates/clippy_config.tera"),
    );

    tera.register_filter("re_escape_py", re_escape_py);
    tera.register_filter("re_escape_ts", re_escape_ts);
    tera.register_filter("re_escape_rust", re_escape_rust);
    tera.register_filter("ts_escape_quote", ts_escape_quote);

    tera
}

/// Render a registered template into a string, panicking with context if the
/// template fails to render. Failures here indicate a programmer error in the
/// template or the data passed in; they should never depend on user input.
pub fn render(tera: &Tera, name: &str, ctx: &Context) -> String {
    tera.render(name, ctx)
        .unwrap_or_else(|e| panic!("template {name} render failed: {e}"))
}

// ── Custom filters ──
//
// Each filter escapes a regex so it can live inside the target language's
// string delimiter without escape errors. Tera filters must be infallible
// once the input is a string, so we defensively handle non-string input by
// passing it through.

fn as_string(value: &Value) -> String {
    value.as_str().unwrap_or("").to_string()
}

/// Escape a regex for inclusion inside a Python raw triple-quoted string
/// (`r"""..."""`). The only sequence that closes the string is `"""`, so we
/// escape embedded triple-quotes. Backslashes stay literal inside r-strings.
fn re_escape_py(value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
    let s = as_string(value).replace("\"\"\"", "\\\"\\\"\\\"");
    Ok(Value::String(s))
}

/// Escape a regex for inclusion inside a TypeScript template literal
/// (`` `...` ``). We must escape backslashes and backticks so the regex string
/// survives the template-literal parse.
fn re_escape_ts(value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
    let s = as_string(value).replace('\\', "\\\\").replace('`', "\\`");
    Ok(Value::String(s))
}

/// Escape a regex for inclusion inside a Rust raw string literal (`r"..."`).
/// Raw strings cannot contain an unescaped double quote, and there is no way
/// to escape one inside `r"..."`. Fold quotes to single quotes as the legacy
/// code path did — this matches an already-shipped behaviour and keeps the
/// generated regex compilable.
fn re_escape_rust(value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
    let s = as_string(value).replace('"', "'");
    Ok(Value::String(s))
}

/// Escape a single quote for inclusion inside a TypeScript single-quoted
/// string literal. Used for `describe('...')` and `it('...')` strings.
fn ts_escape_quote(value: &Value, _args: &HashMap<String, Value>) -> tera::Result<Value> {
    let s = as_string(value).replace('\'', "\\'");
    Ok(Value::String(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tera_registers_all_templates() {
        let tera = build_tera();
        for name in [
            "claude_md.tera",
            "agents_md.tera",
            "cursorrules.tera",
            "copilot_md.tera",
            "windsurfrules.tera",
            "codex_md.tera",
            "python_test.py.tera",
            "typescript_test.ts.tera",
            "rust_test.rs.tera",
            "ruff_config.tera",
            "biome_config.tera",
            "clippy_config.tera",
            "_context_body.tera",
            "python_conftest.py.tera",
            "typescript_setup.ts.tera",
        ] {
            assert!(tera.get_template(name).is_ok(), "missing template {name}");
        }
    }

    #[test]
    fn re_escape_py_preserves_regex_metacharacters() {
        let v = Value::String(r"\.unwrap\s*\(\)".to_string());
        let out = re_escape_py(&v, &HashMap::new()).unwrap();
        assert_eq!(out.as_str().unwrap(), r"\.unwrap\s*\(\)");
    }

    #[test]
    fn re_escape_py_escapes_triple_quotes() {
        let v = Value::String("a\"\"\"b".to_string());
        let out = re_escape_py(&v, &HashMap::new()).unwrap();
        assert_eq!(out.as_str().unwrap(), "a\\\"\\\"\\\"b");
    }

    #[test]
    fn re_escape_ts_doubles_backslashes_and_escapes_backticks() {
        let v = Value::String("a\\b`c".to_string());
        let out = re_escape_ts(&v, &HashMap::new()).unwrap();
        assert_eq!(out.as_str().unwrap(), "a\\\\b\\`c");
    }

    #[test]
    fn re_escape_rust_folds_quotes_to_single() {
        let v = Value::String(r#"foo"bar"#.to_string());
        let out = re_escape_rust(&v, &HashMap::new()).unwrap();
        assert_eq!(out.as_str().unwrap(), "foo'bar");
    }
}
