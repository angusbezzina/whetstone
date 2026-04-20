//! `wh actions` / `wh gen` — run context, tests, and lint generation in sequence.
//!
//! Introduced by bead whetstone-beh. The command is a thin orchestrator over
//! the three underlying generators; it fails fast on the first error so the
//! caller sees exactly where the chain stopped.

use anyhow::Result;
use serde_json::Value;
use std::path::Path;

/// Run context -> tests -> lint generation for the given project.
///
/// `lang` optionally filters by language; `dry_run` previews without writing;
/// `personal` routes every generator to `whetstone/.personal/…` instead of
/// `whetstone/…`.
pub fn run(
    project_dir: &Path,
    lang: Option<&str>,
    dry_run: bool,
    personal: bool,
) -> Result<Value> {
    let context = crate::generate_context::generate_context(project_dir, None, lang, dry_run, personal)?;
    let tests = crate::generate_tests::generate_tests(project_dir, lang, dry_run, personal)?;
    let lint = crate::generate_lint::generate_lint(project_dir, lang, dry_run, personal)?;

    Ok(serde_json::json!({
        "status": "ok",
        "context": context,
        "tests": tests,
        "lint": lint,
        "next_command": "wh check src/",
    }))
}
