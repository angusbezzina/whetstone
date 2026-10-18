//! Rule packs: a self-describing JSON file of accepted, mission-independent
//! rules and principles that another repository imports as drafts.
//!
//! `wh push --pack <file>` exports what is in force here (never drafts, the
//! decision log, drive proofs, local-only rules, or repository paths unless
//! `--include-paths`), with a content digest. `wh pull --pack <file|url|name>`
//! verifies the digest and records every rule and principle as a draft, with
//! the pack as its provenance; nothing is accepted automatically. A principle
//! pack (`whetstone.principle-pack.v1`) pulled the same way is installed under
//! `whetstone/principles/`. Whetstone ships one starter rule pack per language
//! it parses: `rust`, `typescript` and `python`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::agreement::AgreementState;
use crate::domain::{
    Enforcer, ExampleVerdict, Principle, PrincipleSource, Privacy, RecordBody, RecordId, Rule,
    RuleExample, RuleSource, RuleSourceKind, Strength, RULE_SCHEMA_V2,
};
use crate::storage::{ProjectLayout, RecordStore, StoreKind};

use super::change::{draft_with_proposal, state_of};
use super::record::ChangeNarrative;
use super::{load_records, storage_error, utc_timestamp, ServiceResponse, ServiceState};

pub const RULE_PACK_SCHEMA: &str = "whetstone.rule-pack.v1";
/// The largest pack read, from a file or a URL.
const MAX_PACK_BYTES: u64 = 1024 * 1024;
/// The packs that ship in the binary.
pub const BUILT_IN: [&str; 3] = ["rust", "typescript", "python"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RulePack {
    pub schema: String,
    pub name: String,
    pub version: String,
    /// The repository (or `whetstone`) the pack was exported from.
    pub source: String,
    /// `sha256:` of the canonical JSON of `principles` and `rules`.
    pub digest: String,
    /// Whether rules keep the exporting repository's paths.
    #[serde(default)]
    pub includes_paths: bool,
    #[serde(default)]
    pub principles: Vec<PackedPrinciple>,
    pub rules: Vec<PackedRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedPrinciple {
    pub id: String,
    pub statement: String,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedRule {
    pub id: String,
    pub rule: Rule,
}

fn content_digest(principles: &[PackedPrinciple], rules: &[PackedRule]) -> String {
    let bytes =
        serde_json::to_vec(&json!({"principles": principles, "rules": rules})).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

impl RulePack {
    fn sealed(
        name: &str,
        version: &str,
        source: &str,
        includes_paths: bool,
        principles: Vec<PackedPrinciple>,
        rules: Vec<PackedRule>,
    ) -> Self {
        Self {
            schema: RULE_PACK_SCHEMA.into(),
            name: name.into(),
            version: version.into(),
            source: source.into(),
            digest: content_digest(&principles, &rules),
            includes_paths,
            principles,
            rules,
        }
    }

    /// The pack is what it says: schema, digest, valid and unique ids, and
    /// every rule valid.
    pub fn verify(&self) -> Result<(), String> {
        if self.schema != RULE_PACK_SCHEMA {
            return Err(format!("a rule pack's schema is {RULE_PACK_SCHEMA}"));
        }
        let expected = content_digest(&self.principles, &self.rules);
        if self.digest != expected {
            return Err(format!(
                "the pack's digest {} does not match its content ({expected}); it was changed after export",
                self.digest
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for id in self
            .principles
            .iter()
            .map(|principle| principle.id.as_str())
            .chain(self.rules.iter().map(|rule| rule.id.as_str()))
        {
            RecordId::new(id).map_err(|_| format!("{id} is not a record id"))?;
            if !seen.insert(id) {
                return Err(format!("the pack lists {id} twice"));
            }
        }
        for packed in &self.rules {
            packed
                .rule
                .validate()
                .map_err(|error| format!("{} is not a valid rule: {error}", packed.id))?;
            if matches!(packed.rule.enforcer, Enforcer::Drive { .. }) {
                return Err(format!(
                    "{} proves a feature of another repository; a pack never carries drive rules",
                    packed.id
                ));
            }
        }
        Ok(())
    }
}

/// `wh push --pack <file>`: export the accepted, mission-independent rules.
pub(crate) fn export(
    project_dir: &Path,
    request_id: String,
    target: &str,
    include_paths: bool,
    dry_run: bool,
) -> ServiceResponse {
    let layout = match ProjectLayout::resolve(project_dir) {
        Ok(layout) => layout,
        Err(error) => return super::project_error("push", Some(request_id), error),
    };
    let loaded = match load_records(&layout) {
        Ok(loaded) => loaded,
        Err(error) => return storage_error("push", request_id, error),
    };
    let state = AgreementState::from_records(loaded.union().0);
    let mut principles = Vec::new();
    for record in state.in_force_matching(|body| matches!(body, RecordBody::Principle(_))) {
        if let RecordBody::Principle(principle) = &record.body {
            principles.push(PackedPrinciple {
                id: record.id.as_str().into(),
                statement: principle.statement.clone(),
                rationale: principle.rationale.clone(),
            });
        }
    }
    let mut rules = Vec::new();
    let mut left_out = Vec::new();
    for id in crate::proof::rule_ids(&state) {
        let Some(record) = state.in_force(&id) else {
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        let mut rule = rule.into_owned();
        let reason = if matches!(rule.enforcer, Enforcer::Drive { .. }) {
            Some("it proves a feature of this repository")
        } else if rule.privacy.local_only {
            Some("it is local-only")
        } else if rule.source.kind == RuleSourceKind::Mission {
            Some("it is derived from this repository's mission")
        } else {
            None
        };
        if let Some(reason) = reason {
            left_out.push(json!({"rule": id.as_str(), "reason": reason}));
            continue;
        }
        if !include_paths {
            rule.paths.clear();
            rule.sweep_unsure.clear();
            for example in &mut rule.examples {
                example.path = None;
            }
        }
        rules.push(PackedRule {
            id: id.as_str().into(),
            rule,
        });
    }
    if rules.is_empty() {
        let mut response = ServiceResponse::new(
            request_id,
            "push",
            ServiceState::NeedsInput,
            "No accepted rule can leave this repository yet; accept one first (wh dash).",
        );
        response.data = json!({"left_out": left_out});
        return response;
    }
    let root = layout.project_root();
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_else(|| "rules".into());
    let pack = RulePack::sealed(
        &name,
        &utc_timestamp(),
        &super::check::findings::repository_identity(root),
        include_paths,
        principles,
        rules,
    );
    let text = serde_json::to_string_pretty(&pack).unwrap_or_default() + "\n";
    let path = if Path::new(target).is_absolute() {
        Path::new(target).to_path_buf()
    } else {
        project_dir.join(target)
    };
    if !dry_run {
        if let Err(error) = std::fs::write(&path, &text) {
            return super::unknown_response(
                "push",
                request_id,
                format!("{} could not be written: {error}", path.display()),
            );
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "push",
        if dry_run {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "{} rule(s) and {} principle(s) {} {} (digest {}); nothing was shared with the team's remote.",
            pack.rules.len(),
            pack.principles.len(),
            if dry_run { "would be written to" } else { "were written to" },
            path.display(),
            pack.digest
        ),
    );
    response.permitted_actions = vec![format!(
        "in another repository: wh pull --pack {}",
        path.display()
    )];
    response.data = json!({
        "pack": path.display().to_string(),
        "digest": pack.digest,
        "rules": pack.rules.iter().map(|rule| rule.id.as_str()).collect::<Vec<_>>(),
        "principles": pack.principles.iter().map(|principle| principle.id.as_str()).collect::<Vec<_>>(),
        "includes_paths": include_paths,
        "left_out": left_out,
        "dry_run": dry_run,
    });
    response
}

/// `wh pull --pack <file|url|name>`: import a pack's rules and principles as
/// drafts, or install a principle pack.
pub(crate) fn import(
    project_dir: &Path,
    request_id: String,
    source: &str,
    dry_run: bool,
) -> ServiceResponse {
    let needs = |request_id: String, summary: String| {
        let mut response = ServiceResponse::new(
            request_id,
            "pull",
            ServiceState::NeedsInput,
            summary.clone(),
        );
        response.blocking_questions = vec![summary];
        response
    };
    let layout = match ProjectLayout::resolve(project_dir) {
        Ok(layout) => layout,
        Err(error) => return super::project_error("pull", Some(request_id), error),
    };
    let text = match read_pack(project_dir, source) {
        Ok(text) => text,
        Err(error) => return needs(request_id, error),
    };
    let value: Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(error) => return needs(request_id, format!("{source} is not JSON: {error}")),
    };
    if value.get("schema").and_then(Value::as_str) == Some(crate::catalogue::PACK_SCHEMA) {
        return install_principle_pack(&layout, request_id, &text, dry_run);
    }
    let pack: RulePack = match serde_json::from_value(value) {
        Ok(pack) => pack,
        Err(error) => return needs(request_id, format!("{source} is not a rule pack: {error}")),
    };
    if let Err(error) = pack.verify() {
        return needs(request_id, format!("The pack was refused: {error}."));
    }
    let state = match state_of(&layout) {
        Ok(state) => state,
        Err(error) => return storage_error("pull", request_id, error),
    };
    let private = if dry_run {
        None
    } else {
        match RecordStore::initialize(&layout.private_store(), StoreKind::Private) {
            Ok(store) => Some(store),
            Err(error) => return storage_error("pull", request_id, error),
        }
    };
    let provenance = format!("{}@{}", pack.name, pack.version);
    let mut drafted = Vec::new();
    let mut skipped = Vec::new();
    let mut bodies = Vec::new();
    for principle in &pack.principles {
        bodies.push((
            principle.id.clone(),
            "principle",
            RecordBody::Principle(Principle {
                statement: principle.statement.clone(),
                source: PrincipleSource::Custom,
                rationale: Some(format!(
                    "{} (imported from the {} pack, {})",
                    principle
                        .rationale
                        .as_deref()
                        .unwrap_or("A principle the pack holds."),
                    provenance,
                    pack.source
                )),
            }),
        ));
    }
    for packed in &pack.rules {
        let mut rule = packed.rule.clone();
        rule.rationale = format!(
            "{} (imported from the {} pack, {})",
            rule.rationale, provenance, pack.source
        );
        rule.source = RuleSource {
            kind: RuleSourceKind::Pack,
            reference: Some(provenance.clone()),
            provenance: Vec::new(),
        };
        bodies.push((packed.id.clone(), "rule", RecordBody::Rule(rule)));
    }
    for (id, kind, body) in bodies {
        let Ok(record_id) = RecordId::new(id.as_str()) else {
            continue;
        };
        if state.latest(&record_id).is_some() {
            skipped.push(json!({"id": id, "reason": "a record with this id is already here"}));
            continue;
        }
        let Some(private) = private.as_ref() else {
            drafted.push(json!({"id": id, "kind": kind}));
            continue;
        };
        let narrative = ChangeNarrative {
            rationale: format!("Imported from the {provenance} pack ({}).", pack.source),
            source: format!("pack {provenance}"),
            expected_effect:
                "A draft to accept, edit or withdraw; nothing changes until a person accepts it."
                    .into(),
            impact: "none until accepted".into(),
            examples: Vec::new(),
            conflicts: Vec::new(),
        };
        let key = format!("pack:{}:{id}", pack.digest);
        match draft_with_proposal(&layout, private, &key, &record_id, body, &narrative) {
            Ok((record, proposal)) => drafted
                .push(json!({"id": id, "kind": kind, "record": record, "proposal": proposal})),
            Err(error) => return storage_error("pull", request_id, error),
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "pull",
        if dry_run {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "The {provenance} pack: {} draft(s) {}, {} skipped because the record is already here. Nothing is in force until a person accepts it.",
            drafted.len(),
            if dry_run { "would be recorded" } else { "recorded" },
            skipped.len()
        ),
    );
    response.permitted_actions = vec![
        "probe each imported rule here: wh change --kind rule --record-id <id> ... --probe".into(),
        "wh dash (accept or withdraw each draft)".into(),
    ];
    response.data = json!({
        "pack": {"name": pack.name, "version": pack.version, "source": pack.source, "digest": pack.digest},
        "drafts": drafted,
        "skipped": skipped,
        "dry_run": dry_run,
    });
    response
}

fn install_principle_pack(
    layout: &ProjectLayout,
    request_id: String,
    text: &str,
    dry_run: bool,
) -> ServiceResponse {
    let refuse = |request_id: String, summary: String| {
        let mut response = ServiceResponse::new(
            request_id,
            "pull",
            ServiceState::NeedsInput,
            summary.clone(),
        );
        response.blocking_questions = vec![summary];
        response
    };
    let pack = match crate::catalogue::Pack::parse(text) {
        Ok(pack) => pack,
        Err(error) => {
            return refuse(
                request_id,
                format!("The principle pack was refused: {error}."),
            )
        }
    };
    let root = layout.project_root();
    let relative = format!("{}/{}.json", crate::catalogue::PACK_DIRECTORY, pack.name);
    let mut catalogue = crate::catalogue::catalogue(root);
    let replacing = root.join(&relative).is_file();
    if !replacing {
        if let Err(error) = catalogue.add(pack.clone(), relative.clone()) {
            return refuse(
                request_id,
                format!("The principle pack was refused: {error}."),
            );
        }
    }
    if !dry_run {
        let write = std::fs::create_dir_all(root.join(crate::catalogue::PACK_DIRECTORY))
            .and_then(|()| std::fs::write(root.join(&relative), text));
        if let Err(error) = write {
            return super::unknown_response(
                "pull",
                request_id,
                format!("{relative} could not be written: {error}"),
            );
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "pull",
        if dry_run {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "The {} {} principle pack ({} principle(s)) {} {relative}; wh init and the dashboard offer its principles.",
            pack.name,
            pack.version,
            pack.principles.len(),
            if dry_run { "would be installed at" } else { "was installed at" }
        ),
    );
    response.permitted_actions = vec!["wh init".into()];
    response.data = json!({"principle_pack": pack.name, "version": pack.version, "path": relative, "dry_run": dry_run});
    response
}

/// A pack's text: a built-in name, an https URL (fetched with curl) or a file.
fn read_pack(project_dir: &Path, source: &str) -> Result<String, String> {
    if let Some(pack) = built_in(source) {
        return Ok(serde_json::to_string_pretty(&pack).unwrap_or_default());
    }
    if source.starts_with("https://") {
        let limit = MAX_PACK_BYTES.to_string();
        let output = std::process::Command::new("curl")
            .args([
                "-fsSL",
                "--proto",
                "=https",
                "--max-time",
                "60",
                "--max-filesize",
                &limit,
                source,
            ])
            .output()
            .map_err(|error| format!("curl could not run to fetch {source}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "{source} could not be fetched: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        return String::from_utf8(output.stdout).map_err(|_| format!("{source} is not text"));
    }
    if source.contains("://") {
        return Err(format!("{source}: only https URLs are fetched"));
    }
    let path = if Path::new(source).is_absolute() {
        Path::new(source).to_path_buf()
    } else {
        project_dir.join(source)
    };
    let metadata = std::fs::metadata(&path).map_err(|error| {
        format!(
            "{source} is not a file, an https URL or a built-in pack ({}): {error}",
            BUILT_IN.join(", ")
        )
    })?;
    if metadata.len() > MAX_PACK_BYTES {
        return Err(format!("{source} is larger than 1 MiB"));
    }
    std::fs::read_to_string(&path).map_err(|error| format!("{source} could not be read: {error}"))
}

fn starter_rule(
    statement: &str,
    rationale: &str,
    strength: Strength,
    enforcer: Enforcer,
    examples: Vec<(&str, ExampleVerdict, &str)>,
    pack: &str,
) -> Rule {
    Rule {
        schema: RULE_SCHEMA_V2.into(),
        statement: statement.into(),
        rationale: rationale.into(),
        strength,
        enforcer,
        examples: examples
            .into_iter()
            .map(|(input, expected, reason)| RuleExample {
                input: input.into(),
                expected,
                reason: reason.into(),
                path: None,
            })
            .collect(),
        source: RuleSource {
            kind: RuleSourceKind::Pack,
            reference: Some(format!("{pack}@1")),
            provenance: Vec::new(),
        },
        paths: Vec::new(),
        hand_raise: Vec::new(),
        privacy: Privacy::default(),
        shadow: false,
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
    }
}

fn ast(query: &str, language: &str) -> Enforcer {
    Enforcer::Ast {
        query: query.into(),
        language: Some(language.into()),
    }
}

/// A starter rule pack that ships in the binary.
pub fn built_in(name: &str) -> Option<RulePack> {
    use ExampleVerdict::{Flag, Pass};
    let rules: Vec<(&str, Rule)> = match name {
        "rust" => vec![
            ("rule.rust-formatted", starter_rule(
                "Rust code is formatted with rustfmt.",
                "One format means diffs show changes, not whitespace.",
                Strength::Must,
                Enforcer::Formatter { tool: "cargo fmt --all -- --check".into() },
                vec![("fn main() {}\n", Pass, "rustfmt's own output."), ("fn main(){}\n", Flag, "rustfmt would rewrite it.")],
                name,
            )),
            ("rule.rust-no-dbg", starter_rule(
                "No dbg! is left in the code.",
                "dbg! is a debugging aid that prints to stderr in production.",
                Strength::Must,
                ast("((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))", "rust"),
                vec![("fn f() -> u8 { 1 }\n", Pass, "No dbg!."), ("fn f() -> u8 { dbg!(1) }\n", Flag, "A dbg! was left in.")],
                name,
            )),
            ("rule.rust-no-unwrap", starter_rule(
                "Prefer handling an error to calling .unwrap().",
                "A panic is not an error message; propagate with ? or say why with expect.",
                Strength::Should,
                ast("((call_expression function: (field_expression field: (field_identifier) @method)) @match (#eq? @method \"unwrap\"))", "rust"),
                vec![("fn f(x: Option<u8>) -> u8 { x.unwrap_or(0) }\n", Pass, "The None case is handled."), ("fn f(x: Option<u8>) -> u8 { x.unwrap() }\n", Flag, "It panics on None.")],
                name,
            )),
            ("rule.rust-tests-pass", starter_rule(
                "The test suite passes before a push.",
                "Work is done when its tests pass, not when it compiles.",
                Strength::Must,
                Enforcer::Test { command: "cargo test".into() },
                vec![("cargo test: ok", Pass, "The suite passed."), ("cargo test: 1 failed", Flag, "A test failed.")],
                name,
            )),
        ],
        "typescript" => vec![
            ("rule.ts-no-console-log", starter_rule(
                "No console.log is left in the code.",
                "Logging belongs to the logger; console.log is a debugging aid.",
                Strength::Should,
                ast("((call_expression function: (member_expression object: (identifier) @object property: (property_identifier) @property)) @match (#eq? @object \"console\") (#eq? @property \"log\"))", "typescript"),
                vec![("logger.info(\"x\");\n", Pass, "The logger is used."), ("console.log(\"x\");\n", Flag, "console.log was left in.")],
                name,
            )),
            ("rule.ts-no-debugger", starter_rule(
                "No debugger statement is left in the code.",
                "A debugger statement stops every reader's browser.",
                Strength::Must,
                ast("(debugger_statement) @match", "typescript"),
                vec![("const x = 1;\n", Pass, "No debugger."), ("debugger;\n", Flag, "A debugger statement was left in.")],
                name,
            )),
            ("rule.ts-no-any", starter_rule(
                "Prefer a precise type to any.",
                "any switches the type checker off where it matters most.",
                Strength::Should,
                ast("((predefined_type) @match (#eq? @match \"any\"))", "typescript"),
                vec![("let x: unknown = 1;\n", Pass, "unknown keeps the checker on."), ("let x: any = 1;\n", Flag, "any switches it off.")],
                name,
            )),
        ],
        "python" => vec![
            ("rule.py-formatted", starter_rule(
                "Python code is formatted with ruff format.",
                "One format means diffs show changes, not whitespace.",
                Strength::Must,
                Enforcer::Formatter { tool: "ruff format --check .".into() },
                vec![("x = 1\n", Pass, "ruff's own output."), ("x=1\n", Flag, "ruff would rewrite it.")],
                name,
            )),
            ("rule.py-no-unused-imports", starter_rule(
                "No unused imports.",
                "An unused import is dead weight that hides real dependencies.",
                Strength::Should,
                Enforcer::Lint { tool: "ruff check .".into(), code: "F401".into() },
                vec![("import os\nos.getcwd()\n", Pass, "os is used."), ("import os\n", Flag, "os is unused.")],
                name,
            )),
            ("rule.py-no-print", starter_rule(
                "Prefer logging to print().",
                "print() output is lost or mixed into data; logging can be routed and silenced.",
                Strength::Should,
                ast("((call function: (identifier) @function) @match (#eq? @function \"print\"))", "python"),
                vec![("log.info(\"x\")\n", Pass, "The logger is used."), ("print(\"x\")\n", Flag, "print() was used.")],
                name,
            )),
        ],
        _ => return None,
    };
    Some(RulePack::sealed(
        name,
        "1",
        "whetstone",
        false,
        Vec::new(),
        rules
            .into_iter()
            .map(|(id, rule)| PackedRule {
                id: id.into(),
                rule,
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_pack_verifies_and_its_queries_compile() {
        for name in BUILT_IN {
            let pack = built_in(name).expect("built in");
            pack.verify()
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            for packed in &pack.rules {
                if let Enforcer::Ast { query, language } = &packed.rule.enforcer {
                    let lang =
                        crate::ast::AstLang::from_str(language.as_deref().expect("language"))
                            .expect("language");
                    crate::ast::compile_query(lang, query)
                        .unwrap_or_else(|error| panic!("{}: {error}", packed.id));
                }
                assert!(packed.rule.examples.len() >= 2, "{}", packed.id);
            }
        }
        assert!(built_in("cobol").is_none());
    }

    #[test]
    fn a_changed_pack_fails_its_digest() {
        let mut pack = built_in("rust").expect("rust");
        pack.rules[0].rule.strength = Strength::Should;
        assert!(pack
            .verify()
            .expect_err("tampered")
            .contains("does not match"));
    }
}
