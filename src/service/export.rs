//! `wh dash --export <dir>`: the accepted rulebook as one static HTML page
//! that opens from disk or GitHub Pages, so contributors and reviewers can
//! read the mission, principles and rules as last committed, and the
//! changelog, without installing anything. No server and no scripts: the
//! theme follows the reader's system through CSS alone.

use std::fmt::Write as _;
use std::path::Path;

use serde_json::json;

use crate::domain::PrincipleSource;
use crate::layout::ProjectLayout;
use crate::rulebook::{Rulebook, RULEBOOK_PATH};

use super::change::committed;
use super::{
    project_error, project_label, rulebook_log, unknown_response, utc_now, RulebookCommit,
    ServiceResponse, ServiceState,
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
        Err(error) => return project_error("dash", Some(request_id), error),
    };
    let root = layout.project_root();
    let book = match committed(root) {
        Some(Ok(book)) => Some(book),
        Some(Err(error)) => {
            return unknown_response(
                "dash",
                request_id,
                format!("The committed rulebook cannot be read: {error}"),
            )
        }
        None => None,
    };
    let log = match rulebook_log(root) {
        Ok(log) => log,
        Err(error) => return unknown_response("dash", request_id, error),
    };
    let (html, counts) = render(book.as_ref(), &project_label(root), &log);
    let target = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        project_dir.join(dir)
    };
    let written =
        std::fs::create_dir_all(&target).and_then(|()| std::fs::write(target.join(PAGE), &html));
    if let Err(error) = written {
        return unknown_response(
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
            "The accepted rulebook ({} rule(s), {} principle(s), {} changelog entr{}) was written to {}; it opens from disk and needs no server.",
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

const STYLE: &str = ":root{color-scheme:light dark;--bg:#faf8f5;--fg:#1c1917;--muted:#6b6560;--card:#f1ede7;--line:#e2dcd4;--must:#b42318;--should:#b54708;--accent:#c2410c}@media (prefers-color-scheme:dark){:root{--bg:#141210;--fg:#ece7e1;--muted:#a49c93;--card:#1f1c19;--line:#34302b;--must:#f97066;--should:#fdb022;--accent:#e66c3a}}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.6 system-ui,-apple-system,'Segoe UI',sans-serif}main{max-width:52rem;margin:0 auto;padding:2.5rem 1.25rem 4rem}header h1{margin:0;font-size:1.9rem;letter-spacing:-.01em}header p{margin:.25rem 0 0;color:var(--muted)}h2{margin:2.4rem 0 .8rem;font-size:1.15rem;border-bottom:1px solid var(--line);padding-bottom:.35rem}h3{margin:1.6rem 0 .6rem;font-size:1rem}h3 small{font-weight:400;color:var(--muted)}.mission{font-size:1.15rem}ul.principles{padding-left:1.1rem}ul.principles small{color:var(--muted)}article.rule{background:var(--card);border-radius:8px;padding:.85rem 1rem;margin:.6rem 0;border-left:3px solid var(--line)}article.rule.must{border-left-color:var(--must)}article.rule.should{border-left-color:var(--should)}article.rule h4{margin:0 0 .4rem;font-size:1rem}dl{display:grid;grid-template-columns:7rem 1fr;gap:.2rem .8rem;margin:0;font-size:.9rem}dt{color:var(--muted)}dd{margin:0;overflow-wrap:anywhere}code{font:.85em ui-monospace,SFMono-Regular,Menlo,monospace;background:var(--bg);padding:.05rem .3rem;border-radius:4px}ol.changelog{list-style:none;padding:0}ol.changelog li{padding:.55rem 0;border-bottom:1px solid var(--line)}ol.changelog time{color:var(--muted);font-variant-numeric:tabular-nums;margin-right:.6rem}ol.changelog p{margin:.15rem 0 0;color:var(--muted);font-size:.9rem}.status{font-size:.8rem;color:var(--accent);margin-left:.4rem}footer{margin-top:3rem;color:var(--muted);font-size:.85rem}@media (max-width:36rem){dl{grid-template-columns:1fr}dt{margin-top:.3rem}}";

const GROUPS: [(&str, &str, &str); 2] = [
    ("must", "Must", "a change that breaks it fails the check"),
    ("should", "Should", "flagged for review; never blocks"),
];

/// The page, and how many rules, principles and changelog entries it holds.
pub fn render(
    book: Option<&Rulebook>,
    project: &str,
    log: &[RulebookCommit],
) -> (String, serde_json::Value) {
    let mut out = String::new();
    let _ = write!(
        out,
        "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><meta name=color-scheme content=\"light dark\"><title>{} rulebook</title><style>{STYLE}</style><main><header><h1>{} rulebook</h1><p>The rules every agent and person working here is checked against, as {RULEBOOK_PATH} was last committed, exported read-only by Whetstone {} on {}.</p></header>",
        escape(project),
        escape(project),
        env!("CARGO_PKG_VERSION"),
        escape(&utc_now()),
    );
    if let Some(book) = book {
        let _ = write!(
            out,
            "<section id=mission><h2>Mission</h2><p class=mission>{}</p></section>",
            escape(&book.mission.statement)
        );
    }
    let principles = book.map_or(&[][..], |book| book.principles.as_slice());
    let _ = write!(out, "<section id=principles><h2>Principles</h2>");
    if principles.is_empty() {
        let _ = write!(out, "<p>No principle is accepted yet.</p>");
    } else {
        let _ = write!(out, "<ul class=principles>");
        for entry in principles {
            let source = match &entry.principle.source {
                PrincipleSource::Pstack { id, version } => format!("pstack {id} {version}"),
                PrincipleSource::Pack { pack, id, version } => format!("{pack} {id} {version}"),
                PrincipleSource::Custom => "our own".into(),
            };
            let _ = write!(
                out,
                "<li class=principle>{} <small>({})</small></li>",
                escape(&entry.principle.statement),
                escape(&source)
            );
        }
        let _ = write!(out, "</ul>");
    }
    let _ = write!(out, "</section><section id=rules><h2>Rules</h2>");
    let rules = book.map_or(&[][..], |book| book.rules.as_slice());
    for (strength, title, why) in GROUPS {
        let group = rules
            .iter()
            .filter(|entry| entry.rule.strength.label() == strength)
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }
        let _ = write!(out, "<h3>{title} <small>· {why}</small></h3>");
        for entry in group {
            let rule = &entry.rule;
            let (mechanism, command) = crate::gates::mechanism_label(rule);
            let _ = write!(
                out,
                "<article class=\"rule {strength}\" id=\"{}\"><h4>{}</h4><dl><dt>Why</dt><dd>{}</dd><dt>Enforcer</dt><dd>{} <code>{}</code></dd><dt>Covers</dt><dd>{}</dd><dt>Sweep</dt><dd>{}, priority {}</dd><dt>Rule</dt><dd><code>{}</code></dd></dl></article>",
                escape(entry.id.as_str()),
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
                escape(entry.id.as_str()),
            );
        }
    }
    if rules.is_empty() {
        let _ = write!(out, "<p>No rule is accepted yet.</p>");
    }
    let _ = write!(out, "</section><section id=changelog><h2>Changelog</h2>");
    if log.is_empty() {
        let _ = write!(out, "<p>No commit has touched the rulebook yet.</p>");
    } else {
        let _ = write!(out, "<ol class=changelog>");
        for commit in log {
            let _ = write!(
                out,
                "<li class=entry><time datetime=\"{0}\">{0}</time><b>{1}</b><span class=status>{2}</span><p>{3}</p></li>",
                escape(&commit.date),
                escape(&commit.subject),
                escape(&commit.author),
                escape(commit.body.split("\n\n").next().unwrap_or_default()),
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
        json!({"rules": rules.len(), "principles": principles.len(), "changelog": log.len()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_and_the_page_has_no_script() {
        let (page, counts) = render(
            None,
            "<demo>",
            &[RulebookCommit {
                commit: "a".repeat(40),
                author: "Owner".into(),
                date: "2026-10-17T00:00:00Z".into(),
                subject: "<script>x</script>".into(),
                body: "a & b".into(),
            }],
        );
        assert!(!page.contains("<script"));
        assert!(page.contains("&lt;demo&gt; rulebook"));
        assert!(page.contains("a &amp; b"));
        assert_eq!(counts["changelog"], 1);
    }
}
