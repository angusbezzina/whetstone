//! The Git pre-commit and pre-push gates.

use super::*;

/// A Git hook: print the findings people and agents need, and block the
/// commit or push unless the must rules hold.
pub(super) fn run_git_hook(
    service: &CommandService,
    request: CheckRequest,
    pre_commit: bool,
) -> i32 {
    let started = Instant::now();
    let response = service.execute(ServiceRequest::Check(request));
    let elapsed = started.elapsed();
    let label = if pre_commit { "pre-commit" } else { "pre-push" };
    let rendered = format_human_response(&response);
    match response.state {
        ServiceState::Success
            if response.data["gates"]
                .as_array()
                .map_or(true, Vec::is_empty) =>
        {
            // Nothing ran here (for example no rule runs at pre-commit): say
            // why rather than claiming zero rules hold.
            eprintln!(
                "whetstone {label}: {} ({:.1}s)",
                response.summary.lines().next().unwrap_or_default(),
                elapsed.as_secs_f64()
            );
            0
        }
        ServiceState::Success => {
            let gates = response.data["gates"].as_array().map_or(0, Vec::len);
            let flags = response.data["flags"].as_array().map_or(0, Vec::len);
            eprintln!(
                "whetstone {label}: {gates} rule(s) hold{} ({:.1}s)",
                if flags > 0 {
                    format!("; {flags} should-rule flag(s) to repair or have the owner label")
                } else {
                    String::new()
                },
                elapsed.as_secs_f64()
            );
            0
        }
        _ => {
            eprint!("{rendered}");
            eprintln!(
                "whetstone {label}: blocked ({:?}). Repair and try again; never bypass the hook with --no-verify.",
                response.state
            );
            1
        }
    }
}
