//! `wh dash --export <dir>`: the rulebook as one static HTML page that opens
//! from disk or GitHub Pages, so contributors and reviewers can read the
//! accepted mission, principles and rules, and the changelog, without
//! installing anything. No server and no scripts: the theme follows the
//! reader's system through CSS alone.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::domain::{PrincipleSource, RecordBody};
use crate::storage::ProjectLayout;

use super::{
    load_records, project_label, storage_error, DashRequest, ServiceResponse, ServiceState,
};

pub const PAGE: &str = "index.html";

pub(crate) fn export(
    project_dir: &Path,
    request_id: Option<String>,
    dir: &Path,
) -> ServiceResponse {
    let request_id = request_id.unwrap_or_else(|| "dash-export".into());
    let layout = match ProjectLayout::resolve(project_dir) {
        Ok(layout) => layout,
        Err(error) => return super::project_error("dash", Some(request_id), error),
    };
    let loaded = match load_records(&layout) {
        Ok(loaded) => loaded,
        Err(error) => return storage_error("dash", request_id, error),
    };
    let state = AgreementState::from_records(loaded.union().0);
    // The changelog is exactly the dashboard's.
    let dashboard = super::dash::dash(DashRequest::basic(
        project_dir.to_path_buf(),
        Some(request_id.clone()),
    ));
    let changelog = dashboard.data["changelog"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|entry| entry["kind"] == "decision")
        .collect::<Vec<_>>();
    let project = project_label(layout.project_root());
    let (html, counts) = render(&state, &project, &changelog);
    let target = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        project_dir.join(dir)
    };
    let written =
        std::fs::create_dir_all(&target).and_then(|()| std::fs::write(target.join(PAGE), &html));
    if let Err(error) = written {
        return super::unknown_response(
            "dash",
            request_id,
            format!(
                "{} could not be written: {error}",
                target.join(PAGE).display()
            ),
        );
    }
    let mut response = ServiceResponse::new(
        request_id,
        "dash",
        ServiceState::Success,
        format!(
            "The rulebook ({} rule(s), {} principle(s), {} changelog entr{}) was written to {}; it opens from disk and needs no server.",
            counts["rules"],
            counts["principles"],
            counts["changelog"],
            if counts["changelog"] == 1 { "y" } else { "ies" },
            target.join(PAGE).display()
        ),
    );
    response.data = json!({
        "path": target.join(PAGE).display().to_string(),
        "counts": counts,
        "read_only": true,
    });
    response
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const STYLE: &str = ":root{color-scheme:light dark;--bg:#faf8f5;--fg:#1c1917;--muted:#6b6560;--card:#f1ede7;--line:#e2dcd4;--must:#b42318;--should:#b54708;--advisory:#475467;--accent:#c2410c}@media (prefers-color-scheme:dark){:root{--bg:#141210;--fg:#ece7e1;--muted:#a49c93;--card:#1f1c19;--line:#34302b;--must:#f97066;--should:#fdb022;--advisory:#98a2b3;--accent:#e66c3a}}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.6 system-ui,-apple-system,'Segoe UI',sans-serif}main{max-width:52rem;margin:0 auto;padding:2.5rem 1.25rem 4rem}header h1{margin:0;font-size:1.9rem;letter-spacing:-.01em}header p{margin:.25rem 0 0;color:var(--muted)}h2{margin:2.4rem 0 .8rem;font-size:1.15rem;border-bottom:1px solid var(--line);padding-bottom:.35rem}h3{margin:1.6rem 0 .6rem;font-size:1rem}h3 small{font-weight:400;color:var(--muted)}.mission{font-size:1.15rem}ul.principles{padding-left:1.1rem}ul.principles small{color:var(--muted)}article.rule{background:var(--card);border-radius:8px;padding:.85rem 1rem;margin:.6rem 0;border-left:3px solid var(--line)}article.rule.must{border-left-color:var(--must)}article.rule.should{border-left-color:var(--should)}article.rule.advisory{border-left-color:var(--advisory)}article.rule h4{margin:0 0 .4rem;font-size:1rem}dl{display:grid;grid-template-columns:7rem 1fr;gap:.2rem .8rem;margin:0;font-size:.9rem}dt{color:var(--muted)}dd{margin:0;overflow-wrap:anywhere}code{font:.85em ui-monospace,SFMono-Regular,Menlo,monospace;background:var(--bg);padding:.05rem .3rem;border-radius:4px}ol.changelog{list-style:none;padding:0}ol.changelog li{padding:.55rem 0;border-bottom:1px solid var(--line)}ol.changelog time{color:var(--muted);font-variant-numeric:tabular-nums;margin-right:.6rem}ol.changelog p{margin:.15rem 0 0;color:var(--muted);font-size:.9rem}.status{font-size:.8rem;color:var(--accent);margin-left:.4rem}footer{margin-top:3rem;color:var(--muted);font-size:.85rem}@media (max-width:36rem){dl{grid-template-columns:1fr}dt{margin-top:.3rem}}";

const GROUPS: [(&str, &str, &str); 3] = [
    ("must", "Must", "a change that breaks it fails the check"),
    ("should", "Should", "flagged for review; never blocks"),
    ("advisory", "Advisory", "guidance for agents; never checked"),
];

/// The page, and how many rules, principles and changelog entries it holds.
pub fn render(state: &AgreementState, project: &str, changelog: &[Value]) -> (String, Value) {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><meta name=color-scheme content=\"light dark\"><title>{} rulebook</title><style>{STYLE}</style><main><header><h1>{} rulebook</h1><p>The rules every agent and person working here is checked against, exported read-only by Whetstone {} on {}.</p></header>",
        escape(project),
        escape(project),
        env!("CARGO_PKG_VERSION"),
        escape(&crate::service::utc_timestamp()),
    );
    if let Some(mission) = crate::domain::RecordId::new(crate::projection::MISSION_ID)
        .ok()
        .and_then(|id| state.in_force(&id))
    {
        if let RecordBody::Mission(body) = &mission.body {
            let _ = write!(
                out,
                "<section id=mission><h2>Mission</h2><p class=mission>{}</p></section>",
                escape(&body.statement)
            );
        }
    }
    let principles = state.in_force_matching(|body| matches!(body, RecordBody::Principle(_)));
    let _ = write!(out, "<section id=principles><h2>Principles</h2>");
    if principles.is_empty() {
        let _ = write!(out, "<p>No principle is accepted yet.</p>");
    } else {
        let _ = write!(out, "<ul class=principles>");
        for record in &principles {
            if let RecordBody::Principle(body) = &record.body {
                let source = match &body.source {
                    PrincipleSource::Pstack { id, version } => format!("pstack {id} {version}"),
                    PrincipleSource::Pack { pack, id, version } => format!("{pack} {id} {version}"),
                    PrincipleSource::Custom => "our own".into(),
                    PrincipleSource::Migrated { .. } => "migrated".into(),
                };
                let _ = write!(
                    out,
                    "<li class=principle>{} <small>({})</small></li>",
                    escape(&body.statement),
                    escape(&source)
                );
            }
        }
        let _ = write!(out, "</ul>");
    }
    let _ = write!(out, "</section><section id=rules><h2>Rules</h2>");
    let mut rules = 0usize;
    let in_force = crate::proof::rule_ids(state)
        .into_iter()
        .filter_map(|id| {
            let record = state.in_force(&id)?;
            let rule = record.body.rule_view()?.into_owned();
            Some((id, rule))
        })
        .collect::<Vec<_>>();
    for (strength, title, why) in GROUPS {
        let group = in_force
            .iter()
            .filter(|(_, rule)| rule.strength.label() == strength)
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }
        let _ = write!(out, "<h3>{title} <small>· {why}</small></h3>");
        for (id, rule) in group {
            rules += 1;
            let (mechanism, command) = crate::gates::mechanism_label(rule);
            let _ = write!(
                out,
                "<article class=\"rule {strength}\" id=\"{}\"><h4>{}</h4><dl><dt>Why</dt><dd>{}</dd><dt>Enforcer</dt><dd>{} <code>{}</code></dd><dt>Covers</dt><dd>{}</dd><dt>Sweep</dt><dd>{}, priority {}</dd><dt>Rule</dt><dd><code>{}</code></dd></dl></article>",
                escape(id.as_str()),
                escape(&rule.statement),
                escape(&rule.rationale),
                escape(&mechanism),
                escape(&command),
                if rule.paths.is_empty() {
                    "every file".to_string()
                } else {
                    rule.paths
                        .iter()
                        .map(|path| format!("<code>{}</code>", escape(path)))
                        .collect::<Vec<_>>()
                        .join(", ")
                },
                escape(&rule.sweep_class().label().replace('_', " ")),
                rule.sweep_priority(),
                escape(id.as_str()),
            );
        }
    }
    if rules == 0 {
        let _ = write!(out, "<p>No rule is accepted yet.</p>");
    }
    let _ = write!(out, "</section><section id=changelog><h2>Changelog</h2>");
    if changelog.is_empty() {
        let _ = write!(out, "<p>No decision is recorded yet.</p>");
    } else {
        let _ = write!(out, "<ol class=changelog>");
        for entry in changelog {
            let text = |field: &str| entry[field].as_str().unwrap_or_default().to_string();
            let _ = write!(
                out,
                "<li class=entry><time datetime=\"{0}\">{0}</time><b>{1}</b><span class=status>{2}</span><p>{3}</p></li>",
                escape(&text("recorded_at")),
                escape(&text("title")),
                escape(entry["status"]["label"].as_str().unwrap_or_default()),
                escape(&text("summary")),
            );
        }
        let _ = write!(out, "</ol>");
    }
    let _ = writeln!(
        out,
        "</section><footer>Explain any rule with <code>wh explain &lt;rule id&gt;</code>. Propose a change with <code>wh change</code>; a person accepts it.</footer></main></html>"
    );
    (
        out,
        json!({"rules": rules, "principles": principles.len(), "changelog": changelog.len()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_the_page_has_no_script() {
        let (page, counts) = render(
            &AgreementState::default(),
            "<demo>",
            &[
                json!({"recorded_at": "2026-10-17T00:00:00Z", "title": "<script>x</script>", "status": {"label": "accepted"}, "summary": "a & b"}),
            ],
        );
        assert!(!page.contains("<script"));
        assert!(page.contains("&lt;demo&gt; rulebook"));
        assert!(page.contains("a &amp; b"));
        assert_eq!(counts["changelog"], 1);
    }
}
