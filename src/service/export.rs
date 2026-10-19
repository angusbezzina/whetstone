//! `wh init --action export`: the repository's Beads stores written as
//! `whetstone/rulebook.json`, once, before the store goes. The rulebook holds
//! what the team accepted and shared (with the mission accepted here when the
//! team never shared one); what is accepted only here or still a draft goes to `whetstone/rulebook.drafts.json` for the owner to read. A
//! rulebook file that differs is replaced only with `--yes`. Nothing in
//! Beads is written.

use serde_json::json;

use crate::rulebook::{self, Revision, RULEBOOK_PATH};
use crate::storage::ProjectLayout;
use crate::store::rulebook::{self as beads_export, DRAFTS_PATH};

use super::{storage_error, unknown_response, InitRequest, ServiceResponse, ServiceState};

pub(super) fn export(
    layout: &ProjectLayout,
    request_id: String,
    request: &InitRequest,
) -> ServiceResponse {
    let root = layout.project_root();
    let loaded = match beads_export::read(layout) {
        Ok(loaded) => loaded,
        Err(error) => return storage_error("init", request_id, error),
    };
    let export = beads_export::build(&loaded);
    let counts = json!({
        "shared_records": export.shared_records,
        "private_records": export.private_records,
        "mission": usize::from(export.mission.is_some()),
        "principles": export.book.as_ref().map_or(0, |book| book.principles.len()),
        "rules": export.book.as_ref().map_or(0, |book| book.rules.len()),
        "drafts": export.drafts.len(),
        "not_exported": export.not_exported.len(),
    });
    let report = |written: bool, existing: &str, differences: &[String], drafts_written: bool| {
        json!({
            "rulebook": {
                "path": RULEBOOK_PATH,
                "written": written,
                "existing": existing,
                "differences": differences,
                "mission": export.mission.as_ref().map(|id| id.as_str()),
                "mission_from": export.mission.as_ref().map(|_| {
                    if export.mission_shared { "shared" } else { "private" }
                }),
                "principles": export.book.as_ref().map_or(Vec::new(), |book| {
                    book.principles.iter().map(|entry| entry.id.as_str()).collect()
                }),
                "rules": export.book.as_ref().map_or(Vec::new(), |book| {
                    book.rules.iter().map(|entry| entry.id.as_str()).collect()
                }),
            },
            "drafts": {
                "path": DRAFTS_PATH,
                "written": drafts_written,
                "entries": export.drafts,
            },
            "not_exported": export.not_exported,
            "counts": counts,
            "beads_written": false,
        })
    };
    let Some(book) = export.book.as_ref() else {
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            ServiceState::NeedsInput,
            if !loaded.exists() {
                "This repository has no Beads store, so there is nothing to export; nothing was written.".to_string()
            } else {
                "Neither Beads store holds an accepted mission, and a rulebook needs one; nothing was written.".to_string()
            },
        );
        response.blocking_questions =
            vec!["What is the project's mission, in one sentence?".into()];
        response.permitted_actions = vec![format!(
            "write {RULEBOOK_PATH} by hand (references/rulebook.schema.json)"
        )];
        response.data = report(false, "unread", &[], false);
        return response;
    };
    let text = match rulebook::render(book) {
        Ok(text) => text,
        Err(error) => {
            return unknown_response(
                "init",
                request_id,
                format!("The export cannot be written: {error}"),
            )
        }
    };
    if let Err(error) = rulebook::parse(&text, Revision::Uncommitted) {
        return unknown_response(
            "init",
            request_id,
            format!("The export does not read back as a rulebook, so nothing was written: {error}"),
        );
    }
    let path = root.join(RULEBOOK_PATH);
    let (existing, differences) = match std::fs::read_to_string(&path) {
        Ok(on_disk) => {
            let differences = beads_export::differences(&on_disk, book);
            (
                if differences.is_empty() {
                    "same"
                } else {
                    "differs"
                },
                differences,
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ("absent", Vec::new()),
        Err(error) => {
            return unknown_response(
                "init",
                request_id,
                format!("{RULEBOOK_PATH} cannot be read: {error}"),
            )
        }
    };
    let tally = format!(
        "the {} mission, {} principles and {} rules ({} drafts, {} records not exported)",
        if export.mission_shared {
            "shared"
        } else {
            "privately accepted"
        },
        book.principles.len(),
        book.rules.len(),
        export.drafts.len(),
        export.not_exported.len()
    );
    if existing == "differs" && !request.yes {
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            ServiceState::NeedsDecision,
            format!(
                "{RULEBOOK_PATH} differs from what the shared Beads store accepted ({}); nothing was written. Replace it with --yes.",
                differences.join("; ")
            ),
        );
        response.permitted_actions = vec!["wh init --action export --yes".into()];
        response.data = report(false, existing, &differences, false);
        return response;
    }
    let write_rulebook = existing != "same";
    let write_drafts = !export.drafts.is_empty();
    if request.dry_run {
        let mut response = ServiceResponse::new(
            request_id,
            "init",
            if write_rulebook {
                ServiceState::NeedsDecision
            } else {
                ServiceState::Success
            },
            format!(
                "The export holds {tally}; {}nothing was written.",
                if write_rulebook {
                    format!("{RULEBOOK_PATH} would be written; ")
                } else {
                    format!("{RULEBOOK_PATH} already holds it; ")
                }
            ),
        );
        response.permitted_actions = vec!["wh init --action export".into()];
        response.data = report(false, existing, &differences, false);
        response.data["rulebook"]["text"] = json!(text);
        return response;
    }
    let write = |relative: &str, contents: &str| {
        let file = root.join(relative);
        if let Some(directory) = file.parent() {
            std::fs::create_dir_all(directory)
                .map_err(|error| format!("{}: {error}", directory.display()))?;
        }
        std::fs::write(&file, contents).map_err(|error| format!("{relative}: {error}"))
    };
    if write_rulebook {
        if let Err(error) = write(RULEBOOK_PATH, &text) {
            return unknown_response(
                "init",
                request_id,
                format!("The export was not written: {error}"),
            );
        }
    }
    if write_drafts {
        if let Err(error) = write(DRAFTS_PATH, &beads_export::drafts_text(&export)) {
            return unknown_response(
                "init",
                request_id,
                format!("{RULEBOOK_PATH} was written, the drafts were not: {error}"),
            );
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "init",
        ServiceState::Success,
        format!(
            "{} {tally}.{} Nothing in Beads was changed.",
            if write_rulebook {
                format!("Wrote {RULEBOOK_PATH} from the shared Beads store:")
            } else {
                format!("{RULEBOOK_PATH} already holds what the shared Beads store accepted, so it was not rewritten:")
            },
            if write_drafts {
                format!(" The drafts are in {DRAFTS_PATH} for you to read; nothing loads it.")
            } else {
                String::new()
            }
        ),
    );
    response.permitted_actions = if write_rulebook {
        vec![
            format!("git add {RULEBOOK_PATH} && git commit"),
            "wh check".into(),
        ]
    } else {
        vec!["wh check".into()]
    };
    response.data = report(write_rulebook, existing, &differences, write_drafts);
    response
}
