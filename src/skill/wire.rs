//! `wh init --action wire`: write the verification skill into each agent
//! host, scaffold the team-owned driver once, and (with `--hooks`) install the
//! Git gates and each host's own hooks; with `--ci`, the required check.

use std::fs;

use serde_json::json;

use crate::agreement::AgreementState;
use crate::domain::RecordId;
use crate::projection::{self, SkillFile, SkillManifest};
use crate::service::{InitRequest, ServiceResponse, ServiceState};
use crate::storage::{ProjectLayout, RecordStore, StoreKind};

use super::render::{
    feature_markdown, features, features_readme, journal_markdown, map_conventions, skill_markdown,
    verify_document,
};
use super::{
    app_slug, digest, error_response, feature_slug, interview, manifest_path, read_manifest, stamp,
    write_file, Rendered, CI_TEMPLATE, CI_WORKFLOW, DRIVER_CONFIG_RELATIVE, DRIVER_TEMPLATE, HOSTS,
};

/// Generate the verification skill, feature map and journal from accepted
/// records, and scaffold the team-owned driver when absent.
pub fn wire(layout: &ProjectLayout, request_id: String, request: &InitRequest) -> ServiceResponse {
    let store = layout.private_store();
    if !crate::beads::is_initialized(&store) {
        return error_response(
            request_id,
            ServiceState::NeedsInput,
            "No private agreement exists yet. Establish it with wh init --action agree, then wire."
                .into(),
        );
    }
    let records = match RecordStore::open_existing(&store, StoreKind::Private)
        .and_then(|repository| repository.all_records())
    {
        Ok(records) => records,
        Err(error) => {
            return error_response(
                request_id,
                ServiceState::Unknown,
                format!("The private agreement could not be read: {error}"),
            )
        }
    };
    let state = AgreementState::from_records(records);
    if state
        .in_force(&RecordId::new(projection::MISSION_ID).expect("constant"))
        .is_none()
    {
        return error_response(
            request_id,
            ServiceState::NeedsInput,
            "The mission is not in force yet; the skill would have no purpose to explain.".into(),
        );
    }
    let root = layout.project_root();
    let mut hosts = Vec::new();
    for host in &request.hosts {
        match HOSTS.iter().find(|(name, _)| name == host) {
            Some(entry) => {
                if !hosts.contains(entry) {
                    hosts.push(*entry);
                }
            }
            None => {
                return error_response(
                    request_id,
                    ServiceState::NeedsInput,
                    format!("Unknown agent host {host}; choose claude, cursor, codex or agents."),
                )
            }
        }
    }
    if hosts.is_empty() {
        // Regenerate exactly the hosts that were wired before; a host
        // directory that merely exists (another tool's skills) is no request.
        if let Some(previous) = read_manifest(layout) {
            hosts = HOSTS
                .iter()
                .copied()
                .filter(|(name, _)| previous.hosts.iter().any(|host| host == name))
                .collect();
        }
    }
    if hosts.is_empty() {
        hosts = HOSTS
            .iter()
            .copied()
            .filter(|(_, directory)| {
                root.join(directory.split('/').next().unwrap_or(directory))
                    .is_dir()
            })
            .collect();
        if hosts.is_empty() {
            hosts.push(HOSTS[2]);
        }
    }
    let app = app_slug(root);
    let agreement_digest = state.in_force_digest();
    let rendered_at = crate::service::utc_timestamp();
    let header = stamp(&agreement_digest);
    let config_path = root.join(DRIVER_CONFIG_RELATIVE);
    let existing_config = fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let interviewed = match existing_config.as_ref() {
        // The team's configuration is the answer; the interview only
        // describes a fresh scaffold.
        Some(existing) => json!({
            "interview": format!(
                "Using the team's {DRIVER_CONFIG_RELATIVE} (surface: {}); edit it to change how the app launches.",
                existing.get("surface").and_then(serde_json::Value::as_str).unwrap_or("unset")
            ),
        }),
        None => interview(root),
    };
    let config = existing_config
        .clone()
        .unwrap_or_else(|| interviewed.clone());

    let map = map_conventions(&state, &app, &config);
    let proofs = crate::proof::feature_proofs(&state, &crate::proof::changes_for(&state, root));
    let mut generated = vec![
        Rendered {
            relative: "SKILL.md".into(),
            contents: skill_markdown(&state, &app, &config, &header, &map),
        },
        Rendered {
            relative: "features/README.md".into(),
            contents: features_readme(&state, &map, &header),
        },
        Rendered {
            relative: "JOURNAL.md".into(),
            contents: journal_markdown(&state, &header),
        },
        Rendered {
            relative: "whetstone.verify.json".into(),
            contents: serde_json::to_string_pretty(&verify_document(&state, &app, &map))
                .unwrap_or_default()
                + "\n",
        },
    ];
    for id in crate::proof::rule_ids(&state) {
        let Some(record) = state.in_force(&id) else {
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        generated.push(Rendered {
            relative: format!("rules/{}.md", super::rules::rule_slug(&id)),
            contents: super::rules::render_rule_file(&id, record.revision, &rule, &header),
        });
    }
    for (id, feature) in features(&state) {
        let proof = proofs.get(&id).cloned().unwrap_or_default();
        generated.push(Rendered {
            relative: format!("features/{}.md", feature_slug(&id)),
            contents: feature_markdown(&state, &id, &feature, &agreement_digest, &proof),
        });
    }
    let mut writes = Vec::new();
    let mut directories = std::collections::BTreeSet::new();
    for (host, directory) in &hosts {
        // Codex and other agents share `.agents/skills`: one copy.
        if !directories.insert(*directory) {
            continue;
        }
        for file in &generated {
            writes.push((
                format!("{directory}/verify-{app}/{}", file.relative),
                file.contents.clone(),
                *host,
                "generated",
            ));
        }
    }
    let driver_path = root.join(crate::gates::DRIVER_RELATIVE);
    let driver_exists = driver_path.is_file();
    if !driver_exists || request.regenerate_driver {
        writes.push((
            crate::gates::DRIVER_RELATIVE.to_string(),
            DRIVER_TEMPLATE.to_string(),
            "repository",
            "scaffolded once; team-owned",
        ));
    }
    if existing_config.is_none() {
        let mut config = interviewed.clone();
        if let Some(object) = config.as_object_mut() {
            object.remove("interview");
        }
        writes.push((
            DRIVER_CONFIG_RELATIVE.to_string(),
            serde_json::to_string_pretty(&config).unwrap_or_default() + "\n",
            "repository",
            "scaffolded once; team-owned",
        ));
    }
    let mut integration_notes = Vec::new();
    let mut git_hooks = Vec::new();
    if request.hooks {
        type Merge = fn(Option<&str>) -> Result<(String, bool), String>;
        for (host, path, merge, label) in [
            (
                "claude",
                crate::hosts::CLAUDE_SETTINGS,
                crate::hosts::merged_claude_settings as Merge,
                "Claude Code",
            ),
            (
                "codex",
                crate::hosts::CODEX_HOOKS,
                crate::hosts::merged_codex_hooks as Merge,
                "Codex",
            ),
            (
                "cursor",
                crate::hosts::CURSOR_HOOKS,
                crate::hosts::merged_cursor_hooks as Merge,
                "Cursor",
            ),
        ] {
            if !hosts.iter().any(|(name, _)| *name == host) {
                continue;
            }
            let existing = fs::read_to_string(root.join(path)).ok();
            match merge(existing.as_deref()) {
                Ok((text, true)) => writes.push((
                    path.to_string(),
                    text,
                    host,
                    "merged; the team's other settings and hooks are kept",
                )),
                Ok((_, false)) => {
                    integration_notes.push(format!("{label} hooks are already installed."))
                }
                Err(error) => return error_response(request_id, ServiceState::NeedsInput, error),
            }
        }
        if hosts.iter().any(|(name, _)| *name == "codex") {
            integration_notes.push("Codex runs a project's hooks only after you trust them: open /hooks in Codex and approve the Whetstone hooks (again after any change).".to_string());
        }
        if hosts.iter().any(|(name, _)| *name == "cursor")
            && hosts.iter().any(|(name, _)| *name == "claude")
        {
            integration_notes.push("Cursor also reads .claude/settings.json; the Whetstone Claude hooks step aside when Cursor runs them, so each check runs once.".to_string());
        }
        match crate::hosts::git_hook_plan(root) {
            Ok(plan) => git_hooks = plan,
            Err(error) => return error_response(request_id, ServiceState::NeedsInput, error),
        }
        if git_hooks.is_empty() {
            integration_notes
                .push("The Git pre-commit and pre-push gates are already installed.".to_string());
        }
        for hook in &git_hooks {
            if let Some(chained) = &hook.chains {
                integration_notes.push(format!(
                    "{} already existed; it is kept as {chained} and runs first, so nothing is replaced.",
                    hook.path
                ));
            }
        }
    }
    if request.ci {
        if !root.join(CI_WORKFLOW).exists() {
            writes.push((
                CI_WORKFLOW.to_string(),
                CI_TEMPLATE
                    .replace("__WHETSTONE_VERSION__", env!("CARGO_PKG_VERSION"))
                    .replace("__BD_VERSION__", crate::beads::MIN_BD_VERSION),
                "repository",
                "scaffolded once; team-owned; protect with CODEOWNERS",
            ));
        }
        integration_notes.push(format!("Make whetstone/policy a required status check on the default branch, require a reviewing approval, and protect {CI_WORKFLOW} with CODEOWNERS. Branch protection is the team's review; Whetstone keeps no second trust root."));
    }
    let plan = writes
        .iter()
        .map(|(path, contents, host, ownership)| {
            json!({
                "path": path,
                "host": host,
                "ownership": ownership,
                "bytes": contents.len(),
                "digest": digest(contents.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    let footprint = "The skill files contain your agreement text. They are Git-trackable unless the host directory is ignored; commit them only if the team should share them.";
    if request.dry_run {
        let mut response = ServiceResponse::new_public(
            request_id,
            "init",
            ServiceState::NeedsDecision,
            format!(
                "Dry run: {} file(s) would be written for {}; nothing was written.",
                writes.len(),
                hosts
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
        response.permitted_actions =
            vec!["repeat without --dry-run to write exactly these files".into()];
        response.data = json!({
            "dry_run": true,
            "writes": plan,
            "interview": interviewed.get("interview"),
            "agreement_digest": agreement_digest,
            "footprint": footprint,
            "integrations": integration_notes,
            "git_hooks": git_hooks,
            "preview": generated.first().map(|file| file.contents.clone()),
        });
        return response;
    }
    for (path, contents, _, _) in &writes {
        if let Err(error) = write_file(root, path, contents.as_bytes()) {
            return error_response(
                request_id,
                ServiceState::Unknown,
                format!("{path} could not be written: {error}"),
            );
        }
    }
    if let Err(error) = crate::hosts::install_git_hooks(root, &git_hooks) {
        return error_response(
            request_id,
            ServiceState::Unknown,
            format!("The Git gates could not be installed: {error}"),
        );
    }
    #[cfg(unix)]
    if !driver_exists || request.regenerate_driver {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&driver_path, fs::Permissions::from_mode(0o755));
    }
    let manifest = SkillManifest {
        agreement_digest: agreement_digest.clone(),
        rendered_at: rendered_at.clone(),
        hosts: hosts.iter().map(|(name, _)| (*name).to_string()).collect(),
        files: writes
            .iter()
            .filter(|(_, _, _, ownership)| *ownership == "generated")
            .map(|(path, contents, _, _)| SkillFile {
                path: path.clone(),
                digest: digest(contents.as_bytes()),
            })
            .collect(),
    };
    if let Err(error) = fs::create_dir_all(layout.projections_path())
        .map_err(|error| error.to_string())
        .and_then(|()| {
            serde_json::to_vec_pretty(&manifest)
                .map_err(|error| error.to_string())
                .and_then(|bytes| {
                    fs::write(manifest_path(layout), bytes).map_err(|error| error.to_string())
                })
        })
    {
        return error_response(
            request_id,
            ServiceState::Unknown,
            format!("The skill manifest could not be recorded: {error}"),
        );
    }
    let proof_ready = crate::proof::rule_ids(&state)
        .into_iter()
        .filter_map(|id| state.in_force(&id))
        .filter(|record| {
            record
                .body
                .rule_view()
                .is_some_and(|rule| matches!(rule.enforcer, crate::domain::Enforcer::Drive { .. }))
        })
        .count();
    let mut response = ServiceResponse::new_public(
        request_id,
        "init",
        ServiceState::Success,
        format!(
            "The verification skill was written for {} and stamped with the current agreement.",
            hosts
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    response.permitted_actions = vec![
        "node whetstone/verify/drive.mjs doctor --json".into(),
        if proof_ready > 0 {
            "wh check --sweep".into()
        } else {
            "map a feature and a drive rule, then wh check --feature <id>".into()
        },
    ];
    response.data = json!({
        "writes": plan,
        "manifest": manifest,
        "interview": interviewed.get("interview"),
        "driver": {
            "path": crate::gates::DRIVER_RELATIVE,
            "config": DRIVER_CONFIG_RELATIVE,
            "scaffolded": !driver_exists || request.regenerate_driver,
        },
        "footprint": footprint,
        "integrations": integration_notes,
        "git_hooks": git_hooks,
    });
    response
}
