//! Six public workflows backed by one typed service boundary.

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::json;

use crate::service::{
    BasicRequest, ChangeKind, ChangeRequest, CheckRequest, CommandService, InitAction, InitRequest,
    ServiceRequest, ServiceResponse,
};
use crate::{check, output, rules};

#[derive(Parser)]
#[command(
    name = "whetstone",
    version,
    about = "Project agreements that humans and agents can trust",
    disable_help_subcommand = true
)]
struct Cli {
    /// Emit the stable whetstone.command-response.v1 JSON envelope.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Inspect or establish the local project agreement.
    Init {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long, value_enum, default_value_t = InitActionArg::Inspect)]
        action: InitActionArg,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long = "resume")]
        resume_token: Option<String>,
        #[arg(long)]
        mission: Option<String>,
        #[arg(long)]
        values: Option<String>,
        #[arg(long)]
        philosophy: Option<String>,
    },

    /// Open the inspectable local dashboard when its milestone is installed.
    Dash {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
    },

    /// Propose or record a bounded local agreement change.
    Change {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long, value_enum)]
        kind: Option<ChangeKindArg>,
        #[arg(long)]
        record_id: Option<String>,
        #[arg(long)]
        content: Option<String>,
        #[arg(long)]
        rationale: Option<String>,
        #[arg(long)]
        source: Option<String>,
        #[arg(long)]
        expected_effect: Option<String>,
        #[arg(long)]
        impact: Option<String>,
        #[arg(long = "example")]
        examples: Vec<String>,
        #[arg(long = "conflict")]
        conflicts: Vec<String>,
        #[arg(long)]
        expected_revision: Option<u64>,
        #[arg(long = "resume")]
        resume_token: Option<String>,
    },

    /// Evaluate applicable deterministic rules without repairing or publishing.
    Check {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
        #[arg(long = "path")]
        paths: Vec<PathBuf>,
        #[arg(long = "lang")]
        language: Option<String>,
        #[arg(long = "rule")]
        rules: Vec<String>,
    },

    /// Receive approved team agreement changes when team sync is configured.
    Pull {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
    },

    /// Submit selected local proposals for team review when sync is configured.
    Push {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        request_id: Option<String>,
    },

    /// Internal schema gate retained for repository maintenance.
    #[command(hide = true)]
    Validate {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
    },

    /// Internal golden-example gate retained for repository maintenance.
    #[command(hide = true)]
    Eval {
        #[arg(long, default_value = ".")]
        project_dir: PathBuf,
        #[arg(long)]
        lang: Option<String>,
    },

    /// Internal deterministic scanner retained for repository maintenance.
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
        rules_dir: Option<PathBuf>,
        #[arg(long)]
        no_fail: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InitActionArg {
    Inspect,
    Agree,
}

impl From<InitActionArg> for InitAction {
    fn from(value: InitActionArg) -> Self {
        match value {
            InitActionArg::Inspect => Self::Inspect,
            InitActionArg::Agree => Self::Agree,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ChangeKindArg {
    Mission,
    Value,
    Philosophy,
    Guidance,
    Standard,
}

impl From<ChangeKindArg> for ChangeKind {
    fn from(value: ChangeKindArg) -> Self {
        match value {
            ChangeKindArg::Mission => Self::Mission,
            ChangeKindArg::Value => Self::Value,
            ChangeKindArg::Philosophy => Self::Philosophy,
            ChangeKindArg::Guidance => Self::Guidance,
            ChangeKindArg::Standard => Self::Standard,
        }
    }
}

pub fn run() -> i32 {
    let cli = Cli::parse();
    let machine = cli.json || output::is_piped();
    let service = CommandService;
    let response = match cli.command {
        None => service.execute(ServiceRequest::Orientation),
        Some(Command::Init {
            project_dir,
            action,
            request_id,
            expected_revision,
            resume_token,
            mission,
            values,
            philosophy,
        }) => service.execute(ServiceRequest::Init(InitRequest {
            project_dir,
            request_id,
            action: action.into(),
            expected_revision,
            resume_token,
            mission,
            values,
            philosophy,
        })),
        Some(Command::Dash {
            project_dir,
            request_id,
        }) => service.execute(ServiceRequest::Dash(BasicRequest {
            project_dir,
            request_id,
        })),
        Some(Command::Change {
            project_dir,
            request_id,
            kind,
            record_id,
            content,
            rationale,
            source,
            expected_effect,
            impact,
            examples,
            conflicts,
            expected_revision,
            resume_token,
        }) => service.execute(ServiceRequest::Change(ChangeRequest {
            project_dir,
            request_id,
            kind: kind.map(Into::into),
            record_id,
            content,
            rationale,
            source,
            expected_effect,
            impact,
            examples,
            conflicts,
            expected_revision,
            resume_token,
        })),
        Some(Command::Check {
            project_dir,
            request_id,
            paths,
            language,
            rules,
        }) => service.execute(ServiceRequest::Check(CheckRequest {
            project_dir,
            request_id,
            paths,
            language,
            rules,
        })),
        Some(Command::Pull {
            project_dir,
            request_id,
        }) => service.execute(ServiceRequest::Pull(BasicRequest {
            project_dir,
            request_id,
        })),
        Some(Command::Push {
            project_dir,
            request_id,
        }) => service.execute(ServiceRequest::Push(BasicRequest {
            project_dir,
            request_id,
        })),
        Some(Command::Validate { project_dir }) => return validate(&project_dir, machine),
        Some(Command::Eval { project_dir, lang }) => {
            return eval(&project_dir, lang.as_deref(), machine)
        }
        Some(Command::Scan {
            paths,
            project_dir,
            lang,
            rules,
            rules_dir,
            no_fail,
        }) => {
            return scan(
                &project_dir,
                &paths,
                lang.as_deref(),
                &rules,
                rules_dir.as_deref(),
                machine,
                no_fail,
            )
        }
    };
    emit_response(&response, machine);
    response.state.exit_code()
}

fn emit_response(response: &ServiceResponse, machine: bool) {
    if machine {
        let value = serde_json::to_value(response).unwrap_or_else(|error| {
            json!({
                "schema": "whetstone.command-response.v1",
                "schema_version": 1,
                "request_id": response.request_id,
                "workflow": response.workflow,
                "state": "unknown",
                "summary": format!("Response serialization failed: {error}"),
                "evidence": [],
                "blocking_questions": [],
                "permitted_actions": ["report this Whetstone defect"],
                "data": {},
            })
        });
        output::print_json(&value);
        return;
    }
    println!("Whetstone · {} · {:?}", response.workflow, response.state);
    println!("{}", response.summary);
    if let Some(revision) = response.expected_revision {
        println!("Expected revision: {revision}");
    }
    if let Some(token) = &response.resume_token {
        println!("Resume: {token}");
    }
    for question in &response.blocking_questions {
        println!("Needs: {question}");
    }
    for action in &response.permitted_actions {
        println!("Next: {action}");
    }
}

fn validate(project_dir: &Path, machine: bool) -> i32 {
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
    i32::from(!ok)
}

fn eval(project_dir: &Path, lang: Option<&str>, machine: bool) -> i32 {
    let result = check::eval(project_dir, lang);
    let ok = result.get("ok").and_then(|value| value.as_bool()) == Some(true);
    if machine {
        output::print_json(&result);
    } else {
        print!("{}", check::format_eval_output(&result));
    }
    i32::from(!ok)
}

fn scan(
    project_dir: &Path,
    paths: &[PathBuf],
    lang: Option<&str>,
    rule_filter: &[String],
    rules_dir: Option<&Path>,
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
    let result = check::run(check::CheckOptions {
        project_dir,
        rules_dir,
        scan_paths: &scan_paths,
        lang_filter: lang,
        rule_filter: filter,
        execute_command_validators: true,
    });
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
    i32::from(!no_fail && (violations > 0 || config_issues > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argument_enums_map_to_service_types() {
        assert_eq!(InitAction::from(InitActionArg::Agree), InitAction::Agree);
        assert_eq!(
            ChangeKind::from(ChangeKindArg::Guidance),
            ChangeKind::Guidance
        );
    }
}
