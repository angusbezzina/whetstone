//! Field-level differences between two versions of the rulebook file, read
//! as plain JSON so an older committed version that no longer reads
//! strictly still compares. The pending-changes bar diffs the working tree
//! against the last commit; History diffs each commit against the one
//! before it.

use serde::Serialize;
use serde_json::Value;

/// One field of one entry, before and after, as display text. `None` is a
/// field that is absent on that side.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldChange {
    pub field: String,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// What happened to one entry (the mission, a principle or a rule).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EntryChange {
    /// `mission`, `principle` or `rule`.
    pub kind: &'static str,
    /// The record id; `mission` for the mission.
    pub id: String,
    /// `added`, `changed`, `retired`, `reinstated` or `removed`.
    pub action: &'static str,
    /// The entry's statement after the change (before it, when it left).
    pub title: String,
    /// The fields that differ; empty for an addition or a removal.
    pub fields: Vec<FieldChange>,
    /// Why it was retired, for a retirement.
    pub reason: Option<String>,
}

/// The order fields are listed in; anything else follows alphabetically.
const FIELD_ORDER: &[&str] = &[
    "statement",
    "rationale",
    "strength",
    "enforcer",
    "paths",
    "examples",
    "source",
    "sweep",
    "sweep_priority",
    "sweep_unsure",
];

fn entries<'a>(book: Option<&'a Value>, list: &str) -> Vec<(&'a str, &'a Value)> {
    book.and_then(|book| book[list].as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| Some((item["id"].as_str()?, item)))
                .collect()
        })
        .unwrap_or_default()
}

fn retired_reason<'a>(book: Option<&'a Value>, id: &str) -> Option<&'a str> {
    book?["retired"]
        .as_array()?
        .iter()
        .find(|entry| entry["id"].as_str() == Some(id))
        .and_then(|entry| entry["reason"].as_str())
}

/// A field's value as one line of text: strings as they are, lists joined,
/// an enforcer as its kind and command, anything else as compact JSON.
pub fn display(field: &str, value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) if items.iter().all(Value::is_string) => Some(
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        Value::Object(object) if field == "enforcer" => {
            let kind = object.get("kind").and_then(Value::as_str).unwrap_or("?");
            let detail = ["tool", "command", "query"]
                .iter()
                .find_map(|key| object.get(*key).and_then(Value::as_str))
                .unwrap_or_default();
            let code = object
                .get("code")
                .and_then(Value::as_str)
                .map(|code| format!(" ({code})"))
                .unwrap_or_default();
            Some(format!("{kind}: {detail}{code}"))
        }
        other => Some(other.to_string()),
    }
}

fn fields(before: &Value, after: &Value) -> Vec<FieldChange> {
    let mut keys = before
        .as_object()
        .into_iter()
        .chain(after.as_object())
        .flat_map(|object| object.keys().cloned())
        .filter(|key| key != "id")
        .collect::<Vec<_>>();
    keys.sort_by_key(|key| {
        (
            FIELD_ORDER
                .iter()
                .position(|known| known == key)
                .unwrap_or(FIELD_ORDER.len()),
            key.clone(),
        )
    });
    keys.dedup();
    keys.into_iter()
        .filter_map(|key| {
            let (old, new) = (&before[&key], &after[&key]);
            (old != new).then(|| FieldChange {
                before: display(&key, old),
                after: display(&key, new),
                field: key,
            })
        })
        .collect()
}

fn statement(entry: &Value) -> String {
    entry["statement"].as_str().unwrap_or_default().to_string()
}

/// Every entry that differs from `before` (nothing, when absent) to `after`:
/// the mission first, then principles and rules by id.
pub fn diff(before: Option<&Value>, after: &Value) -> Vec<EntryChange> {
    let mut changes = Vec::new();
    let mission = |book: Option<&Value>| {
        book.and_then(|book| book["mission"]["statement"].as_str())
            .map(str::to_string)
            .filter(|text| !text.trim().is_empty())
    };
    let (old_mission, new_mission) = (mission(before), mission(Some(after)));
    if old_mission != new_mission {
        changes.push(EntryChange {
            kind: "mission",
            id: "mission".into(),
            action: if old_mission.is_none() {
                "added"
            } else {
                "changed"
            },
            title: new_mission.clone().unwrap_or_default(),
            fields: if old_mission.is_some() {
                vec![FieldChange {
                    field: "statement".into(),
                    before: old_mission,
                    after: new_mission,
                }]
            } else {
                Vec::new()
            },
            reason: None,
        });
    }
    for (list, kind) in [("principles", "principle"), ("rules", "rule")] {
        let old = entries(before, list);
        let new = entries(Some(after), list);
        let mut ids = old
            .iter()
            .chain(&new)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        for id in ids {
            let then = old.iter().find(|(key, _)| *key == id).map(|(_, v)| *v);
            let now = new.iter().find(|(key, _)| *key == id).map(|(_, v)| *v);
            let change = match (then, now) {
                (None, Some(now)) => EntryChange {
                    kind,
                    id: id.into(),
                    action: if retired_reason(before, id).is_some() {
                        "reinstated"
                    } else {
                        "added"
                    },
                    title: statement(now),
                    fields: Vec::new(),
                    reason: None,
                },
                (Some(then), None) => {
                    let reason = retired_reason(Some(after), id);
                    EntryChange {
                        kind,
                        id: id.into(),
                        action: if reason.is_some() {
                            "retired"
                        } else {
                            "removed"
                        },
                        title: statement(then),
                        fields: Vec::new(),
                        reason: reason.map(str::to_string),
                    }
                }
                (Some(then), Some(now)) if then != now => EntryChange {
                    kind,
                    id: id.into(),
                    action: "changed",
                    title: statement(now),
                    fields: fields(then, now),
                    reason: None,
                },
                _ => continue,
            };
            changes.push(change);
        }
    }
    changes
}

/// One line naming every change, ids in backticks so a reader can set them
/// as code: ``added `rule.x`, changed strength of `rule.y`, edited the
/// mission``. More than three entries of one kind under
/// one action are counted instead: `added the mission and 14 rules`.
pub fn summary(changes: &[EntryChange]) -> String {
    let mut groups: Vec<(&str, &str, Vec<&EntryChange>)> = Vec::new();
    for change in changes {
        match groups
            .iter_mut()
            .find(|(action, kind, _)| *action == change.action && *kind == change.kind)
        {
            Some((_, _, members)) => members.push(change),
            None => groups.push((change.action, change.kind, vec![change])),
        }
    }
    let mut parts = Vec::new();
    for (action, kind, members) in &groups {
        if members.len() > 3 {
            parts.push(format!("{action} {} {kind}s", members.len()));
            continue;
        }
        for change in members {
            let what = if change.kind == "mission" {
                "the mission".to_string()
            } else {
                format!("`{}`", change.id)
            };
            parts.push(match (change.action, change.fields.as_slice()) {
                ("changed", _) if change.kind == "mission" => "edited the mission".to_string(),
                ("changed", [only]) => format!("changed {} of {what}", field_label(&only.field)),
                ("changed", _) => format!("edited {what}"),
                (action, _) => format!("{action} {what}"),
            });
        }
    }
    match parts.as_slice() {
        [] => "no change to the rules".into(),
        [first, second]
            if first.starts_with("added the mission") && second.starts_with("added ") =>
        {
            format!("added the mission and {}", &second["added ".len()..])
        }
        _ => parts.join(", "),
    }
}

/// The field's name in plain words.
pub fn field_label(field: &str) -> &str {
    match field {
        "statement" => "statement",
        "rationale" => "rationale",
        "strength" => "strength",
        "enforcer" => "check",
        "paths" => "paths",
        "examples" => "examples",
        "source" => "source",
        "sweep" => "sweep class",
        "sweep_priority" => "sweep priority",
        "sweep_unsure" => "unsure areas",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn book(rules: Value, retired: Value) -> Value {
        json!({"mission": {"statement": "Ship"}, "principles": [], "rules": rules, "retired": retired})
    }

    #[test]
    fn a_diff_names_each_entry_and_field_and_summarises_them() {
        let before = book(
            json!([
                {"id": "rule.a", "statement": "A", "strength": "must", "enforcer": {"kind": "lint", "tool": "clippy", "code": "x"}},
                {"id": "rule.b", "statement": "B", "strength": "should", "rationale": "r"},
                {"id": "rule.c", "statement": "C", "strength": "should"}
            ]),
            json!([]),
        );
        let after = book(
            json!([
                {"id": "rule.a", "statement": "A", "strength": "should", "enforcer": {"kind": "lint", "tool": "clippy", "code": "x"}},
                {"id": "rule.b", "statement": "B2", "strength": "should", "rationale": "r2"},
                {"id": "rule.d", "statement": "D", "strength": "must"}
            ]),
            json!([{"id": "rule.c", "reason": "superseded"}]),
        );
        let changes = diff(Some(&before), &after);
        assert_eq!(
            summary(&changes),
            "changed strength of `rule.a`, edited `rule.b`, retired `rule.c`, added `rule.d`"
        );
        assert_eq!(
            changes[0].fields,
            vec![FieldChange {
                field: "strength".into(),
                before: Some("must".into()),
                after: Some("should".into()),
            }]
        );
        assert_eq!(
            changes[1]
                .fields
                .iter()
                .map(|f| f.field.as_str())
                .collect::<Vec<_>>(),
            ["statement", "rationale"]
        );
        assert_eq!(changes[2].reason.as_deref(), Some("superseded"));
        let many = book(
            json!([{"id": "rule.a"}, {"id": "rule.b"}, {"id": "rule.c"}, {"id": "rule.d"}]),
            json!([]),
        );
        assert_eq!(summary(&diff(None, &many)), "added the mission and 4 rules");
        let first = diff(None, &after);
        assert_eq!(first[0].action, "added");
        assert_eq!(first[0].kind, "mission");
        assert_eq!(
            display("enforcer", &before["rules"][0]["enforcer"]).as_deref(),
            Some("lint: clippy (x)")
        );
    }
}
