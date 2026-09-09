//! Lean R0 command surface.
//!
//! The six product workflows are deliberately not implemented during the
//! prune-first phase. The hidden maintenance commands below keep the existing
//! schema, golden and self-scan quality gates honest while the new foundations
//! are built.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::{check, output, rules};

#[derive(Parser)]
#[command(
    name = "whetstone",
    version,
    about = "Project agreements that humans and agents can trust",
    disable_help_subcommand = true
)]
struct Cli {
    /// Emit stable machine-readable output.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Internal schema gate retained during the prune-first migration.
    #[command(hide = true)]
    Validate {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Internal golden-example gate retained during the prune-first migration.
    #[command(hide = true)]
    Eval {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        lang: Option<String>,
    },

    /// Internal deterministic scanner retained during the prune-first migration.
    #[command(hide = true)]
    Scan {
        #[arg(default_value = ".")]
        paths: Vec<PathBuf>,
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        lang: Option<String>,
        #[arg(long = "rule")]
        rules: Vec<String>,
        #[arg(long)]
        no_fail: bool,
    },
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let machine = cli.json || output::is_piped();

    match cli.command {
        None => orientation(machine),
        Some(Command::Validate { project_dir }) => validate(&project_dir, machine),
        Some(Command::Eval { project_dir, lang }) => {
            eval(&project_dir, lang.as_deref(), machine)
        }
        Some(Command::Scan {
            paths,
            project_dir,
            lang,
            rules,
            no_fail,
        }) => scan(
            &project_dir,
            &paths,
            lang.as_deref(),
            &rules,
            machine,
            no_fail,
        ),
    }
}

fn orientation(machine: bool) -> i32 {
    let value = json!({
        "status": "foundation",
        "phase": "r0-prune-first",
        "message": "The legacy product has been removed. The six Whetstone workflows are being rebuilt on the lean kernel.",
        "target_workflows": ["init", "dash", "change", "check", "pull", "push"],
        "available": [],
    });

    if machine {
        output::print_json(&value);
    } else {
        println!("Whetstone · lean foundation");
        println!("The legacy product has been removed.");
        println!("Target workflows: init, dash, change, check, pull, push.");
        println!("Product workflows remain unavailable until their implementation milestones pass.");
    }
    0
}

fn validate(project_dir: &std::path::Path, machine: bool) -> i32 {
    let (report, ok) = rules::validate_schema_and_fixtures(project_dir);
    if machine {
        output::print_json(&json!({
            "status": if ok { "ok" } else { "validation_failed" },
            "ok": ok,
            "report": report,
        }));
    } else {
        print!("{report}");
    }
    if ok { 0 } else { 1 }
}

fn eval(project_dir: &std::path::Path, lang: Option<&str>, machine: bool) -> i32 {
    match check::eval(project_dir, lang) {
        Ok(result) => {
            let ok = result.get("ok").and_then(|value| value.as_bool()) == Some(true);
            if machine {
                output::print_json(&result);
            } else {
                print!("{}", check::format_eval_output(&result));
            }
            if ok { 0 } else { 1 }
        }
        Err(error) => {
            print_error(machine, &error.to_string());
            1
        }
    }
}

fn scan(
    project_dir: &std::path::Path,
    paths: &[PathBuf],
    lang: Option<&str>,
    rule_filter: &[String],
    machine: bool,
    no_fail: bool,
) -> i32 {
    let scan_paths: Vec<PathBuf> = paths
        .iter()
        .map(|path| {
            if path.is_absolute() {
                path.clone()
            } else {
                project_dir.join(path)
            }
        })
        .collect();
    let filter = (!rule_filter.is_empty()).then_some(rule_filter);

    match check::run(check::CheckOptions {
        project_dir,
        scan_paths: &scan_paths,
        lang_filter: lang,
        rule_filter: filter,
    }) {
        Ok(result) => {
            let violations = result
                .get("violations_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let config_issues = result
                .get("config_issues_count")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            if machine {
                output::print_json(&result);
            } else {
                print!("{}", check::format_human_output(&result));
            }
            if !no_fail && (violations > 0 || config_issues > 0) {
                1
            } else {
                0
            }
        }
        Err(error) => {
            print_error(machine, &error.to_string());
            1
        }
    }
}

fn print_error(machine: bool, message: &str) {
    if machine {
        output::print_json(&json!({
            "status": "error",
            "error": message,
        }));
    } else {
        eprintln!("Whetstone: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_scan_paths_are_rooted_at_the_project() {
        let project = std::path::Path::new("/tmp/example");
        let paths = [PathBuf::from("src")];
        let resolved: Vec<_> = paths.iter().map(|path| project.join(path)).collect();
        assert_eq!(resolved, vec![PathBuf::from("/tmp/example/src")]);
    }
}
