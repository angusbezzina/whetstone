//! What each accepted rule needs to travel: the page's picker and
//! `wh change --export-pack <file> --preview --json` read the same plan.

use serde::Serialize;

/// The pack a selection would make, rule by rule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PackPlan {
    /// The rulebook's commit, the pack's default version.
    pub revision: Option<String>,
    /// The name and version a pack gets unless a person gives others.
    pub name: String,
    pub version: String,
    pub rules: Vec<RulePlan>,
    pub principles: Vec<PrinciplePlan>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RulePlan {
    pub id: String,
    pub statement: String,
    pub strength: &'static str,
    pub enforcer: &'static str,
    pub language: Option<String>,
    /// The exact query, tool or command.
    pub target: String,
    pub can_leave: bool,
    /// Why it stays, when it cannot leave.
    pub reason: Option<String>,
    pub serves: Option<ServedPrinciple>,
    /// Files its command names, carried with it.
    pub files: Vec<String>,
    /// Files those mention, for a person to include or leave.
    pub suggested_files: Vec<String>,
    /// The programs its command starts, which the other repository needs.
    pub tools: Vec<String>,
    pub paths: Vec<String>,
    pub portability: Portability,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ServedPrinciple {
    pub id: String,
    pub statement: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PrinciplePlan {
    pub id: String,
    pub statement: String,
    /// `custom`, `pstack` or the pack it came from.
    pub origin: String,
    /// The rules that serve it.
    pub rules: Vec<String>,
}

/// How far a rule is from working elsewhere.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Portability {
    /// It cannot leave.
    Stays,
    /// Its command runs files the pack brings.
    BringsFiles,
    /// Its command starts tools the other repository must have.
    NeedsTools,
    /// It has paths that may not exist elsewhere.
    Scoped,
    /// A query over every file: nothing more is needed.
    Portable,
}

/// The label, from the facts alone.
pub fn portability(
    can_leave: bool,
    files: &[String],
    tools: &[String],
    paths: &[String],
) -> Portability {
    if !can_leave {
        Portability::Stays
    } else if !files.is_empty() {
        Portability::BringsFiles
    } else if !tools.is_empty() {
        Portability::NeedsTools
    } else if !paths.is_empty() {
        Portability::Scoped
    } else {
        Portability::Portable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_label_follows_from_the_facts() {
        let some = || vec!["x".to_string()];
        let none = Vec::new;
        assert_eq!(
            portability(false, &some(), &some(), &some()),
            Portability::Stays
        );
        assert_eq!(
            portability(true, &some(), &some(), &some()),
            Portability::BringsFiles
        );
        assert_eq!(
            portability(true, &none(), &some(), &some()),
            Portability::NeedsTools
        );
        assert_eq!(
            portability(true, &none(), &none(), &some()),
            Portability::Scoped
        );
        assert_eq!(
            portability(true, &none(), &none(), &none()),
            Portability::Portable
        );
    }
}
