//! The repository's Beads stores written as the rulebook file: the latest
//! accepted revision of the mission, each principle and each rule the team
//! shared becomes `whetstone/rulebook.json` (the mission accepted here stands
//! in when the team never shared one); what is accepted only here, a
//! pending draft or a private retirement goes to
//! `whetstone/rulebook.drafts.json` for the owner to read (nothing loads
//! it). A record the file cannot hold is named, never dropped silently.
//! Beads is only read.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::agreement::AgreementState;
use crate::beads::BeadsStore;
use crate::domain::{AgreementRecord, PrincipleSource, RecordBody, RecordId};
use crate::rulebook::{self, PrincipleEntry, Revision, RuleEntry, Rulebook};
use crate::service::LoadedRecords;
use crate::storage::{record_class, ProjectLayout, StorageError, StoreKind};

/// Where the drafts go, relative to the project root.
pub const DRAFTS_PATH: &str = "whetstone/rulebook.drafts.json";
pub const DRAFTS_SCHEMA: &str = "whetstone.rulebook-drafts.v1";

/// What the export found in the two stores.
#[derive(Debug, Clone)]
pub struct Export {
    /// The rulebook, when either store holds an accepted mission.
    pub book: Option<Rulebook>,
    pub mission: Option<RecordId>,
    /// Whether the mission came from the shared store; when the team never
    /// shared one, the mission accepted in this checkout stands in.
    pub mission_shared: bool,
    /// Accepted-but-private revisions, pending drafts and private
    /// retirements, each with the entry the file would hold.
    pub drafts: Vec<Value>,
    /// Records the file cannot hold, by id, kind and why.
    pub not_exported: Vec<Value>,
    pub shared_records: usize,
    pub private_records: usize,
}

/// Read the repository's shared Beads database and the private store.
pub fn read(layout: &ProjectLayout) -> Result<LoadedRecords, StorageError> {
    let shared = match layout.shared_store() {
        Some(dir) => Some(BeadsStore::open_existing(&dir, StoreKind::Shareable)?.all_records()?),
        None => None,
    };
    let private = if crate::beads::is_initialized(&layout.private_store()) {
        Some(
            BeadsStore::open_existing(&layout.private_store(), StoreKind::Private)?
                .all_records()?,
        )
    } else {
        None
    };
    Ok(LoadedRecords { private, shared })
}

fn unexported(record: &AgreementRecord, reason: String) -> Value {
    json!({
        "record": record.id.as_str(),
        "revision": record.revision,
        "kind": record.body.type_name(),
        "reason": reason,
    })
}

/// A mission, principle or rule as the file would hold it, or why it cannot.
fn file_entry(record: &AgreementRecord) -> Result<Value, String> {
    match &record.body {
        RecordBody::Mission(mission) => Ok(json!({"statement": mission.statement})),
        RecordBody::Principle(principle) => {
            if let PrincipleSource::Migrated { record: earlier } = &principle.source {
                return Err(format!(
                    "its source is the migrated earlier record {}@{}, and the file holds only pstack and custom principles; add it again with wh change as a custom principle to keep it",
                    earlier.id.as_str(),
                    earlier.revision
                ));
            }
            if let PrincipleSource::Pstack { id, .. } = &principle.source {
                let catalogue = crate::catalogue::catalogue();
                if catalogue.get(id).is_none() {
                    return Err(format!(
                        "its pstack principle {id} is not in the pinned catalogue (version {})",
                        catalogue.version
                    ));
                }
            }
            principle.validate().map_err(|error| error.to_string())?;
            rulebook::principle_value(&PrincipleEntry {
                id: record.id.clone(),
                principle: principle.clone(),
            })
            .map_err(|error| error.to_string())
        }
        body => {
            let Some(rule) = body.rule_view() else {
                return Err("the file holds a mission, principles and rules only".into());
            };
            rule.validate().map_err(|error| error.to_string())?;
            RuleEntry::new(record.id.clone(), rule.into_owned())
                .map(|entry| rulebook::rule_value(&entry))
                .map_err(|error| error.to_string())
        }
    }
}

/// Build the export from the records the stores hold, writing nothing.
pub fn build(loaded: &LoadedRecords) -> Export {
    let shared_records = loaded.shared.as_ref().map_or(0, Vec::len);
    let private_records = loaded.private.as_ref().map_or(0, Vec::len);
    let team = AgreementState::from_records(loaded.shared.clone().unwrap_or_default());
    let mut not_exported = Vec::new();
    let (union, conflicts) = loaded.union();
    for conflict in conflicts {
        not_exported.push(json!({
            "record": conflict.record,
            "revision": conflict.revision,
            "kind": "conflict",
            "reason": format!(
                "the private store holds revision {} with digest {} and the shared store with {}; the shared one was read",
                conflict.revision, conflict.private_digest, conflict.shared_digest
            ),
        }));
    }
    let local = AgreementState::from_records(union);

    // A rulebook needs a mission: the team's, or else the one accepted here.
    let is_mission = |body: &RecordBody| matches!(body, RecordBody::Mission(_));
    let mut missions = team.in_force_matching(is_mission);
    let mission_shared = !missions.is_empty();
    if !mission_shared {
        missions = local.in_force_matching(is_mission);
    }
    let chosen = missions
        .iter()
        .position(|record| record.id.as_str() == "mission.project")
        .unwrap_or(0);
    let mut mission = None;
    for (index, record) in missions.iter().enumerate() {
        match &record.body {
            RecordBody::Mission(body) if index == chosen => {
                mission = Some((record.id.clone(), body.clone()));
            }
            _ => not_exported.push(unexported(
                record,
                format!(
                    "the file holds one mission, and {} was exported",
                    missions[chosen].id.as_str()
                ),
            )),
        }
    }

    let mut principles = Vec::new();
    for record in team.in_force_matching(|body| matches!(body, RecordBody::Principle(_))) {
        let RecordBody::Principle(principle) = &record.body else {
            continue;
        };
        match file_entry(record) {
            Ok(_) => principles.push(PrincipleEntry {
                id: record.id.clone(),
                principle: principle.clone(),
            }),
            Err(reason) => not_exported.push(unexported(record, reason)),
        }
    }

    let mut rules = Vec::new();
    for id in team.rule_ids() {
        let Some(record) = team.in_force(&id) else {
            continue;
        };
        let Some(rule) = record.body.rule_view() else {
            continue;
        };
        let entry = rule
            .validate()
            .map_err(|error| error.to_string())
            .and_then(|()| {
                RuleEntry::new(id, rule.into_owned()).map_err(|error| error.to_string())
            });
        match entry {
            Ok(entry) => rules.push(entry),
            Err(reason) => not_exported.push(unexported(record, reason)),
        }
    }

    // Earlier record kinds the kernel no longer reads as agreement content,
    // named once per record at their latest revision.
    let mut earlier = BTreeMap::<&RecordId, &AgreementRecord>::new();
    for record in team.records() {
        if matches!(record.body, RecordBody::Retired(_))
            && record_class(&record.body) == "record"
            && earlier
                .get(&record.id)
                .map_or(true, |seen| seen.revision < record.revision)
        {
            earlier.insert(&record.id, record);
        }
    }
    for record in earlier.into_values() {
        not_exported.push(unexported(
            record,
            format!(
                "{} is an earlier record kind the file has no place for; it stays in Beads as history",
                record.body.type_name()
            ),
        ));
    }

    let mut drafts = Vec::new();
    let draft = |record: &AgreementRecord, state: &str| {
        let mut value = json!({
            "record": record.id.as_str(),
            "revision": record.revision,
            "kind": record.body.type_name(),
            "state": state,
        });
        if let Some(proposal) = local.proposal_for(record) {
            value["proposal"] = json!(proposal.id.as_str());
        }
        match file_entry(record) {
            Ok(entry) => value["entry"] = entry,
            Err(reason) => value["reason"] = json!(reason),
        }
        value
    };
    for id in local.agreement_ids(|_| true) {
        let shared_ref = team
            .in_force(&id)
            .and_then(|record| record.reference().ok());
        let in_file = mission.as_ref().is_some_and(|(mission, _)| *mission == id);
        match local.in_force(&id) {
            Some(record) if record.reference().ok() != shared_ref && !in_file => {
                drafts.push(draft(record, "accepted_private"));
            }
            None => {
                if let Some(record) = team.in_force(&id) {
                    drafts.push(draft(record, "retired_private"));
                }
            }
            Some(_) => {}
        }
        if let Some(record) = local.pending(&id) {
            drafts.push(draft(record, "draft"));
        }
    }

    Export {
        book: mission.as_ref().map(|(_, mission)| Rulebook {
            mission: mission.clone(),
            principles,
            rules,
            revision: Revision::Uncommitted,
        }),
        mission: mission.map(|(id, _)| id),
        mission_shared,
        drafts,
        not_exported,
        shared_records,
        private_records,
    }
}

/// The drafts file's text.
pub fn drafts_text(export: &Export) -> String {
    let mut text = serde_json::to_string_pretty(&json!({
        "schema": DRAFTS_SCHEMA,
        "note": "Whetstone does not load this file. Each entry is a record accepted only in this checkout's private store, a pending draft or a private retirement, with the entry the rulebook would hold; copy what you keep into whetstone/rulebook.json, then delete this file.",
        "entries": export.drafts,
    }))
    .expect("the drafts always encode");
    text.push('\n');
    text
}

/// How the rulebook file on disk differs from the export, in words; empty
/// when they hold the same mission, principles and rules.
pub fn differences(existing: &str, book: &Rulebook) -> Vec<String> {
    let on_disk = match rulebook::parse(existing, Revision::Uncommitted) {
        Ok(on_disk) => on_disk,
        Err(error) => return vec![error.to_string()],
    };
    let mut differences = Vec::new();
    if on_disk.mission.statement != book.mission.statement {
        differences.push("mission.statement differs".to_string());
    }
    let principles = |book: &Rulebook| {
        book.principles
            .iter()
            .filter_map(|entry| {
                rulebook::principle_value(entry)
                    .ok()
                    .map(|value| (entry.id.as_str().to_string(), value))
            })
            .collect::<BTreeMap<_, _>>()
    };
    let rules = |book: &Rulebook| {
        book.rules
            .iter()
            .map(|entry| (entry.id.as_str().to_string(), rulebook::rule_value(entry)))
            .collect::<BTreeMap<_, _>>()
    };
    compare(
        "principle",
        &principles(&on_disk),
        &principles(book),
        &mut differences,
    );
    compare("rule", &rules(&on_disk), &rules(book), &mut differences);
    differences
}

fn compare(
    kind: &str,
    on_disk: &BTreeMap<String, Value>,
    exported: &BTreeMap<String, Value>,
    differences: &mut Vec<String>,
) {
    for (id, value) in on_disk {
        match exported.get(id) {
            None => differences.push(format!("{kind} {id} is in the file, not in the store")),
            Some(other) if other != value => {
                let fields = |value: &Value| value.as_object().cloned().unwrap_or_default();
                let (left, right) = (fields(value), fields(other));
                let mut changed = left
                    .keys()
                    .chain(right.keys())
                    .filter(|key| left.get(*key) != right.get(*key))
                    .cloned()
                    .collect::<Vec<_>>();
                changed.sort();
                changed.dedup();
                differences.push(format!("{kind} {id} differs in {}", changed.join(", ")));
            }
            Some(_) => {}
        }
    }
    for id in exported.keys() {
        if !on_disk.contains_key(id) {
            differences.push(format!("{kind} {id} is in the store, not in the file"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Enforcer, EvidenceRef, Mission, PrincipalKind, PrincipalRef, Principle, Provenance,
        ProvenanceAuthority, ProvenanceKind, RetiredRecord, Rule, RuleSource, Scope, Strength,
        RULE_SCHEMA_V2, SCHEMA_VERSION_V1,
    };

    fn record(
        id: &str,
        revision: u64,
        authority: ProvenanceAuthority,
        body: RecordBody,
    ) -> AgreementRecord {
        let owner = PrincipalRef {
            kind: PrincipalKind::LocalUser,
            stable_id: "local:owner".into(),
            display_name: None,
        };
        AgreementRecord {
            schema_version: SCHEMA_VERSION_V1,
            id: RecordId::new(id).expect("id"),
            revision,
            scope: Scope::project("project-test"),
            owner: owner.clone(),
            provenance: Provenance {
                kind: ProvenanceKind::HumanAuthored,
                recorded_by: owner,
                recorded_at: format!("2026-10-19T10:00:0{revision}Z"),
                sources: vec![EvidenceRef {
                    system: "test".into(),
                    locator: "test".into(),
                    digest: None,
                }],
                authority,
            },
            supersedes: None,
            idempotency_key: format!("{id}:{revision}"),
            body,
        }
    }

    fn accepted(id: &str, revision: u64, body: RecordBody) -> AgreementRecord {
        record(id, revision, ProvenanceAuthority::OwnerAuthored, body)
    }

    fn mission(statement: &str) -> RecordBody {
        RecordBody::Mission(Mission {
            statement: statement.into(),
            desired_outcomes: vec!["kept out of the file".into()],
        })
    }

    fn principle(statement: &str, source: PrincipleSource) -> RecordBody {
        RecordBody::Principle(Principle {
            statement: statement.into(),
            source,
            rationale: None,
        })
    }

    fn rule(statement: &str, strength: Strength) -> RecordBody {
        RecordBody::Rule(Rule {
            schema: RULE_SCHEMA_V2.into(),
            statement: statement.into(),
            rationale: "Because it matters.".into(),
            strength,
            enforcer: Enforcer::Test {
                command: "git --version".into(),
            },
            examples: Vec::new(),
            source: RuleSource::owner(),
            paths: vec!["src/**".into()],
            sweep: None,
            sweep_priority: None,
            sweep_unsure: Vec::new(),
        })
    }

    fn shared() -> Vec<AgreementRecord> {
        let earlier = accepted(
            "core_value.speed",
            1,
            RecordBody::Retired(RetiredRecord {
                record_type: "core_value".into(),
                record: json!({"statement": "Fast"}),
            }),
        );
        vec![
            accepted("mission.project", 1, mission("Ship code people can trust.")),
            accepted(
                "rule.zeta",
                1,
                rule("Zeta first in time, last by id.", Strength::Must),
            ),
            accepted(
                "rule.alpha",
                1,
                rule("Alpha, first version.", Strength::Should),
            ),
            accepted(
                "rule.alpha",
                2,
                rule("Alpha, second version.", Strength::Must),
            ),
            accepted(
                "principle.mine",
                1,
                principle("Small steps.", PrincipleSource::Custom),
            ),
            accepted(
                "principle.old",
                1,
                principle(
                    "Carried from a value.",
                    PrincipleSource::Migrated {
                        record: earlier.reference().expect("ref"),
                    },
                ),
            ),
            earlier,
        ]
    }

    fn loaded(shared: Vec<AgreementRecord>, private: Vec<AgreementRecord>) -> LoadedRecords {
        LoadedRecords {
            shared: Some(shared),
            private: Some(private),
        }
    }

    fn reasons(export: &Export) -> Vec<String> {
        export
            .not_exported
            .iter()
            .map(|item| {
                format!(
                    "{} {}",
                    item["record"].as_str().unwrap_or(""),
                    item["kind"].as_str().unwrap_or("")
                )
            })
            .collect()
    }

    #[test]
    fn the_shared_latest_accepted_revisions_become_the_rulebook_sorted_by_id() {
        let export = build(&loaded(shared(), Vec::new()));
        let book = export.book.as_ref().expect("a mission");
        assert!(export.mission_shared);
        assert_eq!(book.mission.statement, "Ship code people can trust.");
        let ids = book
            .rules
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["rule.alpha", "rule.zeta"]);
        assert_eq!(book.rules[0].rule.statement, "Alpha, second version.");
        assert_eq!(book.principles.len(), 1);
        assert_eq!(
            reasons(&export),
            ["principle.old principle", "core_value.speed core_value"]
        );
        assert!(export.not_exported[0]["reason"]
            .as_str()
            .expect("reason")
            .contains("migrated"));
        assert!(export.drafts.is_empty());
        let text = rulebook::render(book).expect("renders");
        let read = rulebook::parse(&text, Revision::Uncommitted).expect("reads back");
        assert_eq!(read.rules, book.rules);
        assert!(!text.contains("desired_outcomes") && !text.contains("whetstone.rule.v2"));
        assert!(differences(&text, book).is_empty());
    }

    #[test]
    fn private_acceptances_and_drafts_go_to_the_drafts_not_the_rulebook() {
        let draft = record(
            "rule.draft",
            1,
            ProvenanceAuthority::CandidateOnly,
            rule("Only proposed.", Strength::Must),
        );
        let private = vec![
            accepted(
                "rule.zeta",
                2,
                rule("Zeta, accepted here only.", Strength::Must),
            ),
            accepted("rule.local", 1, rule("Local only.", Strength::Should)),
            draft,
        ];
        let export = build(&loaded(shared(), private));
        let book = export.book.as_ref().expect("a mission");
        let zeta = book
            .rules
            .iter()
            .find(|entry| entry.id.as_str() == "rule.zeta")
            .expect("zeta");
        assert_eq!(zeta.rule.statement, "Zeta first in time, last by id.");
        assert!(book
            .rules
            .iter()
            .all(|entry| entry.id.as_str() != "rule.local"));
        let drafts = export
            .drafts
            .iter()
            .map(|item| {
                format!(
                    "{}@{} {}",
                    item["record"].as_str().unwrap_or(""),
                    item["revision"],
                    item["state"].as_str().unwrap_or("")
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            drafts,
            [
                "rule.draft@1 draft",
                "rule.local@1 accepted_private",
                "rule.zeta@2 accepted_private"
            ]
        );
        assert_eq!(
            export.drafts[2]["entry"]["statement"],
            "Zeta, accepted here only."
        );
        let text = drafts_text(&export);
        assert!(text.contains(DRAFTS_SCHEMA) && text.contains("does not load this file"));
    }

    #[test]
    fn the_mission_accepted_here_stands_in_when_the_team_never_shared_one() {
        let team = shared()
            .into_iter()
            .filter(|record| record.id.as_str() != "mission.project")
            .collect();
        let private = vec![accepted("mission.project", 1, mission("Our own mission."))];
        let export = build(&loaded(team, private));
        assert!(!export.mission_shared);
        assert_eq!(
            export.book.expect("a mission").mission.statement,
            "Our own mission."
        );
        assert!(
            export.drafts.is_empty(),
            "the mission is in the file, not a draft"
        );
        let none = build(&loaded(Vec::new(), Vec::new()));
        assert!(none.book.is_none() && none.mission.is_none());
    }

    #[test]
    fn differences_name_the_mission_and_each_entry() {
        let export = build(&loaded(shared(), Vec::new()));
        let book = export.book.as_ref().expect("a mission");
        let mut value: Value =
            serde_json::from_str(&rulebook::render(book).expect("renders")).expect("json");
        value["mission"]["statement"] = json!("Something else.");
        value["rules"][0]["strength"] = json!("should");
        value["rules"].as_array_mut().expect("rules").remove(1);
        value["principles"] = json!([]);
        assert_eq!(
            differences(&value.to_string(), book),
            [
                "mission.statement differs",
                "principle principle.mine is in the store, not in the file",
                "rule rule.alpha differs in strength",
                "rule rule.zeta is in the store, not in the file",
            ]
        );
        assert!(differences("{}", book)[0].contains("not a rulebook"));
    }
}
