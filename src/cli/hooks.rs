//! The Git pre-commit and pre-push gates.

use serde_json::Value;

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
            eprintln!(
                "whetstone {label}: {} ({:.1}s)",
                held_line(&response.data),
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

/// What a hook that lets the commit or push through says: how many rules
/// held, and which should rules flagged or reached no verdict. A rule that
/// flagged or reached no verdict is not counted as holding.
fn held_line(data: &Value) -> String {
    let held = data["gates"].as_array().map_or(0, |gates| {
        gates.iter().filter(|gate| gate["state"] == "pass").count()
    });
    let mut line = format!("{held} rule(s) hold");
    let flags = data["flags"].as_array().map_or(0, Vec::len);
    if flags > 0 {
        line.push_str(&format!(
            "; {flags} should-rule flag(s) to repair or have the owner label"
        ));
    }
    let unknown = data["no_verdict"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        line.push_str(&format!(
            "; {} should rule(s) reached no verdict, which is not a pass: {}",
            unknown.len(),
            unknown.join(", ")
        ));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rule_that_flagged_or_reached_no_verdict_is_not_counted_as_holding() {
        let data = serde_json::json!({
            "gates": [{"state": "pass"}, {"state": "fail"}, {"state": "unknown"}],
            "flags": ["rule.flagged"],
            "no_verdict": ["rule.needs-a-tool"]
        });
        assert_eq!(
            held_line(&data),
            "1 rule(s) hold; 1 should-rule flag(s) to repair or have the owner label; \
             1 should rule(s) reached no verdict, which is not a pass: rule.needs-a-tool"
        );
        let all_held =
            serde_json::json!({"gates": [{"state": "pass"}], "flags": [], "no_verdict": []});
        assert_eq!(held_line(&all_held), "1 rule(s) hold");
    }
}
