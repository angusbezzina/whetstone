//! Rule packs: a self-describing JSON file of accepted, mission-independent
//! rules and principles, with the files their commands run, that another
//! repository imports as drafts.
//!
//! `pack_export` writes a pack from the committed rulebook and `pack_import`
//! reviews and applies one. Reviewing runs nothing; a pack's commands run and
//! its files arrive only with a person's consent, and nothing takes force
//! until a person accepts it. A principle pack
//! (`whetstone.principle-pack.v1`) imported the same way is installed under
//! `whetstone/principles/`. Whetstone ships one starter rule pack per
//! language it parses: `rust`, `typescript` and `python`.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::domain::{
    Enforcer, ExampleVerdict, PrincipleSource, RecordId, Rule, RuleExample, RuleSource,
    RuleSourceKind, Strength, RULE_SCHEMA_V2,
};
use crate::rulebook::{self, Rulebook};

use super::pack_files::{refused_path, PackedFile};
use super::{unknown_response, ChangeRequest, ServiceResponse, ServiceState};

pub const RULE_PACK_SCHEMA: &str = "whetstone.rule-pack.v2";
/// The first shape: the same fields without the optional ones v2 adds.
const RULE_PACK_SCHEMA_V1: &str = "whetstone.rule-pack.v1";
/// The largest pack read or written, from a file, a URL or the dashboard.
pub(super) const MAX_PACK_BYTES: u64 = 1024 * 1024;
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
    /// `sha256:` of the JSON of `principles`, `rules` and any `files`.
    pub digest: String,
    /// Whether every rule keeps the exporting repository's paths.
    #[serde(default)]
    pub includes_paths: bool,
    #[serde(default)]
    pub principles: Vec<PackedPrinciple>,
    pub rules: Vec<PackedRule>,
    /// The files the rules' commands need, one entry per path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PackedFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedPrinciple {
    pub id: String,
    pub statement: String,
    #[serde(default)]
    pub rationale: Option<String>,
    /// Where it came from, when that is not the owner's own words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PrincipleSource>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackedRule {
    pub id: String,
    pub rule: Rule,
    /// The rule had paths where it came from, and they stayed behind, so
    /// here it covers every file.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scoped_at_source: bool,
    /// The paths in the pack's `files` this rule needs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

fn content_digest(
    principles: &[PackedPrinciple],
    rules: &[PackedRule],
    files: &[PackedFile],
) -> String {
    let mut content = json!({"principles": principles, "rules": rules});
    if !files.is_empty() {
        content["files"] = json!(files);
    }
    let bytes = serde_json::to_vec(&content).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

impl RulePack {
    pub(super) fn sealed(
        name: &str,
        version: &str,
        source: &str,
        includes_paths: bool,
        principles: Vec<PackedPrinciple>,
        rules: Vec<PackedRule>,
        files: Vec<PackedFile>,
    ) -> Self {
        Self {
            schema: RULE_PACK_SCHEMA.into(),
            name: name.into(),
            version: version.into(),
            source: source.into(),
            digest: content_digest(&principles, &rules, &files),
            includes_paths,
            principles,
            rules,
            files,
        }
    }

    /// The pack is what it says: schema, digest, valid and unique ids, every
    /// rule valid, and every file allowed, intact and accounted for.
    pub fn verify(&self) -> Result<(), String> {
        if self.schema != RULE_PACK_SCHEMA && self.schema != RULE_PACK_SCHEMA_V1 {
            return Err(format!("a rule pack's schema is {RULE_PACK_SCHEMA}"));
        }
        let expected = content_digest(&self.principles, &self.rules, &self.files);
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
            RecordId::new(id).map_err(|_| format!("{id} is not a valid id"))?;
            if !seen.insert(id) {
                return Err(format!("the pack lists {id} twice"));
            }
        }
        let mut paths = std::collections::BTreeSet::new();
        for file in &self.files {
            if let Some(reason) = refused_path(&file.path) {
                return Err(format!("it carries {}, and {reason}", file.path));
            }
            if !paths.insert(file.path.as_str()) {
                return Err(format!("the pack lists {} twice", file.path));
            }
            if !file.intact() {
                return Err(format!(
                    "the digest of {} does not match its text; it was changed after export",
                    file.path
                ));
            }
        }
        for packed in &self.rules {
            packed
                .rule
                .validate()
                .map_err(|error| format!("{} is not a valid rule: {error}", packed.id))?;
            if let Some(path) = packed
                .files
                .iter()
                .find(|path| !paths.contains(path.as_str()))
            {
                return Err(format!(
                    "{} names the file {path}, which the pack does not carry",
                    packed.id
                ));
            }
        }
        Ok(())
    }
}

pub(super) fn needs(request_id: String, summary: String) -> ServiceResponse {
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        ServiceState::NeedsInput,
        summary.clone(),
    );
    response.blocking_questions = vec![summary];
    response
}

/// A pack id for a principle written in the owner's words: the first words
/// of its statement as a slug, at most 48 characters.
pub(crate) fn principle_slug(statement: &str) -> String {
    let mut slug = String::new();
    let mut dash = false;
    for character in statement.chars().flat_map(char::to_lowercase) {
        if slug.len() >= 48 {
            break;
        }
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() || slug == crate::catalogue::STARTING {
        "principle".into()
    } else {
        slug
    }
}

/// `wh change --export-pack <file> --principles`: the committed rulebook's
/// own principles as a principle pack (`whetstone.principle-pack.v1`), the
/// shape pstack and mattpocock ship in, for another repository to install
/// with `--import-pack` and pick from in `wh init`. A principle picked from
/// a pack is left out: it already has a pack, and a second pack repeating
/// its id would be refused beside the first.
pub(super) fn export_principles(
    root: &Path,
    request_id: String,
    target: &str,
    request: &ChangeRequest,
    book: &Rulebook,
) -> ServiceResponse {
    let mut principles = Vec::new();
    let mut left_out = Vec::new();
    let mut taken = std::collections::BTreeSet::new();
    for entry in &book.principles {
        match &entry.principle.source {
            PrincipleSource::Pstack { id, .. } => {
                left_out.push(json!({"principle": entry.id.as_str(), "reason": format!("it is pstack's {id}; share that pack instead")}));
                continue;
            }
            PrincipleSource::Pack { pack, id, .. } => {
                left_out.push(json!({"principle": entry.id.as_str(), "reason": format!("it is {pack}'s {id}; share that pack instead")}));
                continue;
            }
            PrincipleSource::Custom => {}
        }
        let base = principle_slug(&entry.principle.statement);
        let mut id = base.clone();
        let mut n = 2;
        while !taken.insert(id.clone()) {
            id = format!("{base}-{n}");
            n += 1;
        }
        principles.push(json!({
            "id": id,
            "group": "team",
            "text": entry.principle.statement,
            "enforcer_hint": "none",
            "rationale": entry.principle.rationale,
            "starting": false,
        }));
    }
    if principles.is_empty() {
        let mut response = needs(
            request_id,
            "The accepted rulebook holds no principle in your own words, so there is nothing to put in a pack; write one (wh init --action agree --custom-principle <text>), accept it, then export.".into(),
        );
        response.data = json!({"left_out": left_out});
        return response;
    }
    let name = {
        let raw = root
            .file_name()
            .map(|name| name.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        let slug = principle_slug(&raw);
        if slug == "principle" {
            "principles".to_string()
        } else {
            slug
        }
    };
    let commit = match &book.revision {
        rulebook::Revision::Commit(hash) => Some(hash.clone()),
        rulebook::Revision::Uncommitted => None,
    };
    let version = commit
        .as_deref()
        .map(|hash| hash.chars().take(12).collect::<String>())
        .unwrap_or_else(|| "1".into());
    let mut pack = json!({
        "schema": crate::catalogue::PACK_SCHEMA,
        "name": name,
        "version": version,
        "source": super::check::findings::repository_identity(root),
        "license": "unspecified",
        "principles": principles,
    });
    if let Some(commit) = &commit {
        pack["commit"] = json!(commit);
    }
    let text = serde_json::to_string_pretty(&pack).unwrap_or_default() + "\n";
    let path = if Path::new(target).is_absolute() {
        Path::new(target).to_path_buf()
    } else {
        root.join(target)
    };
    if !request.preview {
        if let Err(error) = std::fs::write(&path, &text) {
            return unknown_response(
                "change",
                request_id,
                format!("{} could not be written: {error}", path.display()),
            );
        }
    }
    let ids = pack["principles"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        if request.preview {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "{} principle(s) {} {} as the principle pack {} {}{}; set its license, groups and enforcer hints in the file. The rulebook was not changed.",
            ids.len(),
            if request.preview {
                "would be written to"
            } else {
                "were written to"
            },
            path.display(),
            pack["name"].as_str().unwrap_or_default(),
            pack["version"].as_str().unwrap_or_default(),
            if left_out.is_empty() {
                String::new()
            } else {
                format!(" ({} left out: picked from a pack)", left_out.len())
            }
        ),
    );
    response.permitted_actions = vec![
        format!(
            "in another repository: wh change --import-pack {} (installs it under whetstone/principles)",
            path.display()
        ),
        format!(
            "then: wh init --action agree --principle {}/<id>",
            pack["name"].as_str().unwrap_or_default()
        ),
    ];
    response.data = json!({
        "pack": path.display().to_string(),
        "schema": crate::catalogue::PACK_SCHEMA,
        "name": pack["name"],
        "version": pack["version"],
        "principles": ids,
        "left_out": left_out,
        "preview_only": request.preview,
    });
    response
}

pub(super) fn install_principle_pack(
    root: &Path,
    request_id: String,
    text: &str,
    preview: bool,
) -> ServiceResponse {
    let pack = match crate::catalogue::Pack::parse(text) {
        Ok(pack) => pack,
        Err(error) => {
            return needs(
                request_id,
                format!("The principle pack was refused: {error}."),
            )
        }
    };
    let relative = format!("{}/{}.json", crate::catalogue::PACK_DIRECTORY, pack.name);
    let mut catalogue = crate::catalogue::catalogue(root);
    let replacing = root.join(&relative).is_file();
    if !replacing {
        if let Err(error) = catalogue.add(pack.clone(), relative.clone()) {
            return needs(
                request_id,
                format!("The principle pack was refused: {error}."),
            );
        }
    }
    if !preview {
        let write = std::fs::create_dir_all(root.join(crate::catalogue::PACK_DIRECTORY))
            .and_then(|()| std::fs::write(root.join(&relative), text));
        if let Err(error) = write {
            return unknown_response(
                "change",
                request_id,
                format!("{relative} could not be written: {error}"),
            );
        }
    }
    let mut response = ServiceResponse::new(
        request_id,
        "change",
        if preview {
            ServiceState::NeedsDecision
        } else {
            ServiceState::Success
        },
        format!(
            "The {} {} principle pack ({} principle(s)) {} {relative}; wh init and the dashboard offer its principles.",
            pack.name,
            pack.version,
            pack.principles.len(),
            if preview { "would be installed at" } else { "was installed at" }
        ),
    );
    response.permitted_actions = vec!["wh init".into(), format!("git add {relative}")];
    response.data = json!({"principle_pack": pack.name, "version": pack.version, "path": relative, "preview_only": preview});
    response
}

/// A pack's text: a built-in name, an https URL (fetched with curl) or a file.
pub(super) fn read_pack(root: &Path, source: &str) -> Result<String, String> {
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
        root.join(source)
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
        sweep: None,
        sweep_priority: None,
        sweep_unsure: Vec::new(),
        exclude: None,
        mode: crate::domain::RuleMode::Tree,
        batch: None,
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
                scoped_at_source: false,
                files: Vec::new(),
            })
            .collect(),
        Vec::new(),
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
