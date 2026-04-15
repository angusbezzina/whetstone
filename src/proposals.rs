//! Structured candidate proposals and deterministic import.
//!
//! The agent's extraction step emits a JSON or YAML file matching the
//! [`ProposalBundle`] schema. `wh propose import <file>` validates the
//! bundle, rejects bad input, and writes candidate rule YAML files into
//! `whetstone/rules/{language}/{dep}.yaml` with `status: candidate`,
//! `approved: false`, `proposed_at`, and `proposed_by` populated
//! automatically — so the agent never hand-authors YAML and the
//! audit trail is consistent.
//!
//! A proposal bundle looks like:
//!
//! ```yaml
//! version: 1
//! proposed_by: whetstone-extraction
//! dependency:
//!   name: reqwest
//!   language: rust
//!   version: 0.12.0
//!   source_url: https://docs.rs/reqwest
//!   content_hash: "sha256:..."
//!   registry: crates_io
//! proposals:
//!   - id: reqwest.set-timeout
//!     severity: must
//!     confidence: high
//!     category: default
//!     description: "Clients MUST set an explicit timeout."
//!     source_url: https://docs.rs/reqwest/latest/reqwest/#timeouts
//!     signals:
//!       - id: no-timeout
//!         strategy: pattern
//!         match: 'Client::new\(\)'
//!         weight: required
//!     golden_examples:
//!       - code: "let c = Client::new();"
//!         verdict: fail
//!         reason: missing timeout
//! ```

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::config::WhetstoneConfig;
use crate::rules::{load_rule_files, LoadedRuleFile, Rule, RuleFile, RuleSource};

pub const BUNDLE_VERSION: u32 = 1;
const VALID_CATEGORIES: &[&str] = &[
    "migration",
    "default",
    "convention",
    "breaking-change",
    "semantic",
];
const VALID_SEVERITIES: &[&str] = &["must", "should", "may"];
const VALID_CONFIDENCES: &[&str] = &["high", "medium"];
const VALID_STRATEGIES: &[&str] = &["ast", "pattern", "lint_proxy", "ai"];
const DEFAULT_MAX_RULES_PER_DEP: u32 = 5;

// ── Schema types ──

/// Top-level proposal bundle emitted by the agent.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposalBundle {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Free-text agent identifier ("whetstone-extraction", "manual", etc.).
    #[serde(default)]
    pub proposed_by: Option<String>,
    /// ISO-8601 timestamp when the proposal was produced. Optional — the
    /// importer defaults to now if missing.
    #[serde(default)]
    pub proposed_at: Option<String>,
    pub dependency: ProposalDependency,
    pub proposals: Vec<ProposedRule>,
}

fn default_version() -> u32 {
    BUNDLE_VERSION
}

/// Dependency metadata the proposal refers to. Determines which rule file
/// the proposal lands in: `whetstone/rules/{language}/{name}.yaml`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposalDependency {
    pub name: String,
    pub language: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub source_url: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub registry: Option<String>,
    #[serde(default)]
    pub resolved_at: Option<String>,
}

/// A single proposed rule inside a bundle. Shape mirrors the rule schema
/// minus status/provenance fields (which the importer sets).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposedRule {
    pub id: String,
    pub severity: String,
    pub confidence: String,
    pub category: String,
    pub description: String,
    pub source_url: String,
    #[serde(default)]
    pub source_quote: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub linter_gap: Option<String>,
    pub signals: Vec<ProposedSignal>,
    pub golden_examples: Vec<ProposedExample>,
    #[serde(default)]
    pub deterministic_pass_threshold: Option<u32>,
    #[serde(default)]
    pub deterministic_fail_threshold: Option<u32>,
    #[serde(default)]
    pub ai_eval: Option<ProposedAiEval>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposedSignal {
    #[serde(default)]
    pub id: Option<String>,
    pub strategy: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub weight: Option<String>,
    #[serde(default, alias = "match")]
    pub match_pattern: Option<String>,
    #[serde(default)]
    pub ast_query: Option<String>,
    #[serde(default)]
    pub ast_scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposedExample {
    pub code: String,
    pub verdict: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProposedAiEval {
    #[serde(default)]
    pub trigger: Option<String>,
    #[serde(default)]
    pub question: Option<String>,
    #[serde(default)]
    pub context_lines: Option<u32>,
}

// ── Import ──

pub struct ImportOptions<'a> {
    pub project_dir: &'a Path,
    pub bundle_path: &'a Path,
    pub dry_run: bool,
    /// When `Some`, overrides the bundle's `proposed_by` field.
    pub actor: Option<&'a str>,
    /// Allow importing proposals whose id already exists as a candidate.
    /// Without this flag the importer errors rather than silently overwriting.
    pub overwrite_candidates: bool,
}

pub fn import(opts: ImportOptions<'_>) -> Result<Value> {
    let bundle = load_bundle(opts.bundle_path)?;
    if bundle.version != BUNDLE_VERSION {
        bail!(
            "unsupported proposal bundle version {} (expected {})",
            bundle.version,
            BUNDLE_VERSION
        );
    }

    let config = WhetstoneConfig::load(opts.project_dir);
    validate_bundle(&bundle, &config)?;

    // Per-dep quota: every rule that lives in the file (approved +
    // candidate) counts against `extraction.max_rules_per_dep`. Denied
    // and deprecated rules are excluded — they are audit history, not
    // live rules. New candidates replace same-id candidates when
    // `--overwrite-candidates` is set, so those don't double-count.
    let max = config
        .extraction
        .max_rules_per_dep
        .unwrap_or(DEFAULT_MAX_RULES_PER_DEP) as usize;
    let existing_live = count_live_rules_for_dep(
        opts.project_dir,
        &bundle.dependency.language,
        &bundle.dependency.name,
    );
    let overwritten: usize = if opts.overwrite_candidates {
        bundle
            .proposals
            .iter()
            .filter(|p| is_candidate_in_dep(opts.project_dir, &bundle.dependency, &p.id))
            .count()
    } else {
        0
    };
    let projected_total = existing_live.saturating_sub(overwritten) + bundle.proposals.len();
    if projected_total > max {
        bail!(
            "importing would push `{}` to {} rules, over extraction.max_rules_per_dep = {} (existing: {}, proposing: {})",
            bundle.dependency.name,
            projected_total,
            max,
            existing_live,
            bundle.proposals.len()
        );
    }

    let now = chrono::Utc::now().to_rfc3339();
    let effective_proposer = opts
        .actor
        .map(String::from)
        .or_else(|| bundle.proposed_by.clone())
        .unwrap_or_else(|| "whetstone-proposal-import".to_string());
    let proposed_at = bundle.proposed_at.clone().unwrap_or_else(|| now.clone());

    let rules_dir = opts
        .project_dir
        .join("whetstone")
        .join("rules")
        .join(&bundle.dependency.language);

    let target_file = rules_dir.join(format!("{}.yaml", bundle.dependency.name));

    // Load existing file (if any) to merge with instead of clobbering.
    let mut existing_ids: BTreeMap<String, String> = BTreeMap::new();
    if target_file.exists() {
        if let Ok(text) = fs::read_to_string(&target_file) {
            if let Ok(rf) = serde_yaml::from_str::<RuleFile>(&text) {
                for rule in &rf.rules {
                    let st = rule
                        .status
                        .clone()
                        .unwrap_or_else(|| if rule.approved { "approved".into() } else { "candidate".into() });
                    existing_ids.insert(rule.id.clone(), st);
                }
            }
        }
    }

    // Guard against overwriting approved/denied rules with the same id.
    let mut blocking: Vec<String> = Vec::new();
    let mut will_replace_candidates: Vec<String> = Vec::new();
    for p in &bundle.proposals {
        match existing_ids.get(&p.id).map(String::as_str) {
            Some("candidate") => {
                if opts.overwrite_candidates {
                    will_replace_candidates.push(p.id.clone());
                } else {
                    blocking.push(format!(
                        "candidate `{}` already exists; pass --overwrite-candidates or pick a new id",
                        p.id
                    ));
                }
            }
            Some(other) => blocking.push(format!(
                "rule `{}` already exists with status `{}`; deprecate/supersede instead",
                p.id, other
            )),
            None => {}
        }
    }
    if !blocking.is_empty() {
        bail!("cannot import proposals:\n  - {}", blocking.join("\n  - "));
    }

    if opts.dry_run {
        return Ok(serde_json::json!({
            "status": "ok",
            "action": "dry_run",
            "bundle_version": bundle.version,
            "dependency": {
                "name": bundle.dependency.name,
                "language": bundle.dependency.language,
                "version": bundle.dependency.version,
            },
            "target_file": target_file.display().to_string(),
            "would_add": bundle.proposals.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
            "would_replace_candidates": will_replace_candidates,
            "existing_rules_in_file": existing_ids.len(),
            "next_command": "re-run without --dry-run to write the file",
        }));
    }

    fs::create_dir_all(&rules_dir).with_context(|| {
        format!(
            "failed to create target rules directory {}",
            rules_dir.display()
        )
    })?;

    // Build the merged YAML text. We only write the new candidates into the
    // file; existing rules are preserved verbatim by working on the parsed
    // `RuleFile` model and re-emitting it. Since the file is machine-written
    // (not hand-edited much for candidate imports), fully re-serializing is
    // safe and avoids the need for a second in-place surgery path.
    let mut target = if target_file.exists() {
        let text = fs::read_to_string(&target_file)?;
        serde_yaml::from_str::<RuleFile>(&text).with_context(|| {
            format!("failed to parse existing rule file {}", target_file.display())
        })?
    } else {
        RuleFile {
            source: RuleSource::default(),
            rules: Vec::new(),
        }
    };

    // Merge dependency source metadata. New fields win when the bundle
    // provides them; otherwise existing values are preserved.
    if target.source.name.is_empty() {
        target.source.name = bundle.dependency.name.clone();
    }
    if let Some(v) = &bundle.dependency.version {
        target.source.version = Some(v.clone());
    }
    if let Some(v) = &bundle.dependency.source_url {
        target.source.docs_url = Some(v.clone());
    }
    if let Some(v) = &bundle.dependency.content_hash {
        target.source.content_hash = Some(v.clone());
    }
    if let Some(v) = &bundle.dependency.registry {
        target.source.registry = Some(v.clone());
    }
    if let Some(v) = &bundle.dependency.resolved_at {
        target.source.resolved_at = Some(v.clone());
    } else if target.source.resolved_at.is_none() {
        target.source.resolved_at = Some(now.clone());
    }

    // Drop existing candidates with matching ids if we are overwriting them.
    if opts.overwrite_candidates && !will_replace_candidates.is_empty() {
        target.rules.retain(|r| {
            !(will_replace_candidates.contains(&r.id) && is_candidate(r))
        });
    }

    for p in &bundle.proposals {
        target.rules.push(proposal_to_rule(
            p,
            &proposed_at,
            &effective_proposer,
        ));
    }

    let serialized = serialize_rule_file(&target)?;
    fs::write(&target_file, serialized)?;

    Ok(serde_json::json!({
        "status": "ok",
        "action": "imported",
        "bundle_version": bundle.version,
        "dependency": {
            "name": bundle.dependency.name,
            "language": bundle.dependency.language,
            "version": bundle.dependency.version,
        },
        "target_file": target_file.display().to_string(),
        "added": bundle.proposals.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        "replaced_candidates": will_replace_candidates,
        "proposed_by": effective_proposer,
        "proposed_at": proposed_at,
        "next_command": "wh review --status=candidate && wh apply <rule-id> --approve",
    }))
}

/// Surface the schema as structured JSON so `wh propose schema` can
/// document it without re-parsing docs.
pub fn schema_json() -> Value {
    serde_json::json!({
        "version": BUNDLE_VERSION,
        "title": "Whetstone proposal bundle",
        "top_level": {
            "version": "integer (required, must be 1)",
            "proposed_by": "string (optional — agent identifier)",
            "proposed_at": "ISO-8601 timestamp (optional — defaults to now)",
            "dependency": "ProposalDependency (required)",
            "proposals": "ProposedRule[] (required, 1..=max_rules_per_dep)",
        },
        "ProposalDependency": {
            "name": "string (required)",
            "language": "string (required — python | typescript | rust | generic)",
            "version": "string (optional)",
            "source_url": "URL (optional)",
            "content_hash": "string (optional)",
            "registry": "string (optional — pypi | npm | crates_io | manual)",
            "resolved_at": "ISO-8601 timestamp (optional)",
        },
        "ProposedRule": {
            "id": "string (required — dotted-id, unique in ruleset)",
            "severity": "enum (required: must | should | may)",
            "confidence": "enum (required: high | medium)",
            "category": "enum (required: migration | default | convention | breaking-change | semantic)",
            "description": "string (required)",
            "source_url": "URL (required)",
            "source_quote": "string (optional)",
            "source_kind": "string (optional)",
            "risk": "string (optional)",
            "linter_gap": "string (optional)",
            "signals": "ProposedSignal[] (required, \u{2265}1 must be ast or pattern)",
            "golden_examples": "ProposedExample[] (required, 3\u{2013}5 mixed pass/fail)",
        },
        "enforcement": {
            "max_rules_per_dep": format!(
                "extraction.max_rules_per_dep (defaults to {})",
                DEFAULT_MAX_RULES_PER_DEP
            ),
            "allowed_categories": "extraction.allowed_categories (defaults to all five)",
            "min_confidence": "extraction.min_confidence (defaults to medium+high)",
        },
        "provenance_auto_populated_on_import": [
            "status=candidate",
            "approved=false",
            "proposed_at=bundle.proposed_at || now",
            "proposed_by=actor-flag || bundle.proposed_by || \"whetstone-proposal-import\"",
        ],
    })
}

fn load_bundle(path: &Path) -> Result<ProposalBundle> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("could not read proposal bundle {}", path.display()))?;

    // Accept either JSON or YAML. We try JSON first because it is a strict
    // subset of YAML, but explicit `.json` files produce better error
    // messages when failing.
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if is_json {
        serde_json::from_str::<ProposalBundle>(&text)
            .with_context(|| format!("invalid JSON proposal bundle {}", path.display()))
    } else {
        serde_yaml::from_str::<ProposalBundle>(&text)
            .or_else(|_| serde_json::from_str::<ProposalBundle>(&text))
            .with_context(|| format!("invalid proposal bundle {}", path.display()))
    }
}

fn validate_bundle(bundle: &ProposalBundle, cfg: &WhetstoneConfig) -> Result<()> {
    if bundle.proposals.is_empty() {
        bail!("proposal bundle has no proposals");
    }

    if bundle.dependency.name.trim().is_empty() {
        bail!("dependency.name is required");
    }
    if bundle.dependency.language.trim().is_empty() {
        bail!("dependency.language is required");
    }

    if !cfg.extraction_allows(&bundle.dependency.name) {
        bail!(
            "dependency `{}` is filtered out by extraction.include / extraction.exclude",
            bundle.dependency.name
        );
    }

    let max = cfg
        .extraction
        .max_rules_per_dep
        .unwrap_or(DEFAULT_MAX_RULES_PER_DEP) as usize;
    if bundle.proposals.len() > max {
        bail!(
            "bundle has {} proposals but extraction.max_rules_per_dep = {}",
            bundle.proposals.len(),
            max
        );
    }

    let allowed_categories: Vec<String> = if cfg.extraction.allowed_categories.is_empty() {
        VALID_CATEGORIES.iter().map(|s| s.to_string()).collect()
    } else {
        cfg.extraction.allowed_categories.clone()
    };
    let min_confidence = cfg.extraction.min_confidence.as_deref();

    let mut ids = std::collections::HashSet::new();
    for (idx, p) in bundle.proposals.iter().enumerate() {
        let ctx = format!("proposal[{idx}] ({})", p.id);
        if p.id.trim().is_empty() {
            bail!("{ctx}: id is required");
        }
        if !ids.insert(p.id.clone()) {
            bail!("{ctx}: duplicate id `{}` within bundle", p.id);
        }
        if !VALID_SEVERITIES.contains(&p.severity.as_str()) {
            bail!("{ctx}: severity `{}` must be one of {VALID_SEVERITIES:?}", p.severity);
        }
        if !VALID_CONFIDENCES.contains(&p.confidence.as_str()) {
            bail!("{ctx}: confidence `{}` must be one of {VALID_CONFIDENCES:?}", p.confidence);
        }
        if let Some(min) = min_confidence {
            // min_confidence=high blocks medium; min_confidence=medium passes both.
            if min == "high" && p.confidence != "high" {
                bail!("{ctx}: confidence `{}` below extraction.min_confidence=high", p.confidence);
            }
        }
        if !allowed_categories.iter().any(|c| c == &p.category) {
            bail!(
                "{ctx}: category `{}` not in extraction.allowed_categories ({:?})",
                p.category,
                allowed_categories
            );
        }
        if p.description.trim().is_empty() {
            bail!("{ctx}: description is required");
        }
        if p.source_url.trim().is_empty() {
            bail!("{ctx}: source_url is required");
        }
        if p.signals.is_empty() {
            bail!("{ctx}: at least one signal is required");
        }
        let has_det = p
            .signals
            .iter()
            .any(|s| matches!(s.strategy.as_str(), "ast" | "pattern"));
        if !has_det {
            bail!(
                "{ctx}: must have at least one ast or pattern signal (hard rule)"
            );
        }
        for s in &p.signals {
            if !VALID_STRATEGIES.contains(&s.strategy.as_str()) {
                bail!(
                    "{ctx}: signal strategy `{}` must be one of {VALID_STRATEGIES:?}",
                    s.strategy
                );
            }
        }
        if p.golden_examples.len() < 3 || p.golden_examples.len() > 5 {
            bail!(
                "{ctx}: must have 3 to 5 golden examples (have {})",
                p.golden_examples.len()
            );
        }
        let has_pass = p.golden_examples.iter().any(|e| e.verdict == "pass");
        let has_fail = p.golden_examples.iter().any(|e| e.verdict == "fail");
        if !has_pass || !has_fail {
            bail!(
                "{ctx}: golden examples must include at least one `pass` and one `fail` verdict"
            );
        }
    }

    Ok(())
}

fn proposal_to_rule(p: &ProposedRule, proposed_at: &str, proposer: &str) -> Rule {
    Rule {
        id: p.id.clone(),
        severity: Some(p.severity.clone()),
        confidence: Some(p.confidence.clone()),
        category: Some(p.category.clone()),
        description: Some(p.description.clone()),
        source_url: Some(p.source_url.clone()),
        source_quote: p.source_quote.clone(),
        risk: p.risk.clone(),
        linter_gap: p.linter_gap.clone(),
        approved: false,
        approved_at: None,
        status: Some("candidate".into()),
        proposed_at: Some(proposed_at.to_string()),
        proposed_by: Some(proposer.to_string()),
        denied_reason: None,
        deprecated_reason: None,
        superseded_by: None,
        source_kind: p.source_kind.clone(),
        deterministic_pass_threshold: p.deterministic_pass_threshold,
        deterministic_fail_threshold: p.deterministic_fail_threshold,
        ai_eval: p.ai_eval.as_ref().map(|a| crate::rules::AiEval {
            trigger: a.trigger.clone().unwrap_or_default(),
            question: a.question.clone().unwrap_or_default(),
            context_lines: a.context_lines,
        }),
        signals: p
            .signals
            .iter()
            .map(|s| crate::rules::Signal {
                id: s.id.clone(),
                strategy: s.strategy.clone(),
                description: s.description.clone(),
                weight: s.weight.clone(),
                match_pattern: s.match_pattern.clone(),
                ast_query: s.ast_query.clone(),
                ast_scope: s.ast_scope.clone(),
            })
            .collect(),
        golden_examples: p
            .golden_examples
            .iter()
            .map(|e| crate::rules::GoldenExample {
                code: e.code.clone(),
                verdict: e.verdict.clone(),
                reason: e.reason.clone(),
                language: e.language.clone(),
            })
            .collect(),
    }
}

fn count_live_rules_for_dep(project_dir: &Path, language: &str, dep: &str) -> usize {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (files, _) = load_rule_files(&rules_dir);
    let mut count = 0usize;
    for lrf in files {
        if lrf.language.as_deref() != Some(language) {
            continue;
        }
        if lrf.rule_file.source.name != dep {
            continue;
        }
        for rule in &lrf.rule_file.rules {
            let status = rule.status.as_deref().unwrap_or({
                if rule.approved {
                    "approved"
                } else {
                    "candidate"
                }
            });
            if matches!(status, "approved" | "candidate") {
                count += 1;
            }
        }
    }
    count
}

fn is_candidate_in_dep(project_dir: &Path, dep: &ProposalDependency, rule_id: &str) -> bool {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (files, _) = load_rule_files(&rules_dir);
    for lrf in files {
        if lrf.language.as_deref() != Some(dep.language.as_str()) {
            continue;
        }
        if lrf.rule_file.source.name != dep.name {
            continue;
        }
        for rule in &lrf.rule_file.rules {
            if rule.id == rule_id && is_candidate(rule) {
                return true;
            }
        }
    }
    false
}

fn is_candidate(r: &Rule) -> bool {
    let status = r.status.as_deref().unwrap_or({
        if r.approved {
            "approved"
        } else {
            "candidate"
        }
    });
    status == "candidate"
}

/// Serialize a `RuleFile` back into YAML text suitable for committing.
/// Uses `serde_yaml` and then prepends a short header so the file is
/// identifiable as machine-emitted.
fn serialize_rule_file(rf: &RuleFile) -> Result<String> {
    let yaml = serde_yaml::to_string(&SerializableRuleFile::from(rf))
        .context("failed to serialize rule file to YAML")?;
    let header = "# Generated/updated by Whetstone. Edit candidates via `wh apply`.\n";
    Ok(format!("{header}{yaml}"))
}

// Local helper mirrors (de)serialization so we can keep `Rule` as-is while
// emitting a stable on-disk layout matching the schema. The `Rule` struct
// does not currently derive Serialize (to avoid accidental use elsewhere).
#[derive(Serialize)]
struct SerializableRuleFile<'a> {
    source: &'a RuleSource,
    rules: Vec<SerializableRule<'a>>,
}

impl<'a> From<&'a RuleFile> for SerializableRuleFile<'a> {
    fn from(rf: &'a RuleFile) -> Self {
        Self {
            source: &rf.source,
            rules: rf.rules.iter().map(SerializableRule::from).collect(),
        }
    }
}

#[derive(Serialize)]
struct SerializableRule<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    severity: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_url: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_quote: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_kind: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    risk: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    linter_gap: &'a Option<String>,
    approved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    approved_at: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proposed_at: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    proposed_by: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    denied_reason: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deprecated_reason: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    superseded_by: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deterministic_pass_threshold: &'a Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deterministic_fail_threshold: &'a Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ai_eval: &'a Option<crate::rules::AiEval>,
    signals: Vec<SerializableSignal<'a>>,
    golden_examples: Vec<SerializableExample<'a>>,
}

impl<'a> From<&'a Rule> for SerializableRule<'a> {
    fn from(r: &'a Rule) -> Self {
        Self {
            id: &r.id,
            severity: &r.severity,
            confidence: &r.confidence,
            category: &r.category,
            description: &r.description,
            source_url: &r.source_url,
            source_quote: &r.source_quote,
            source_kind: &r.source_kind,
            risk: &r.risk,
            linter_gap: &r.linter_gap,
            approved: r.approved,
            approved_at: &r.approved_at,
            status: &r.status,
            proposed_at: &r.proposed_at,
            proposed_by: &r.proposed_by,
            denied_reason: &r.denied_reason,
            deprecated_reason: &r.deprecated_reason,
            superseded_by: &r.superseded_by,
            deterministic_pass_threshold: &r.deterministic_pass_threshold,
            deterministic_fail_threshold: &r.deterministic_fail_threshold,
            ai_eval: &r.ai_eval,
            signals: r.signals.iter().map(SerializableSignal::from).collect(),
            golden_examples: r
                .golden_examples
                .iter()
                .map(SerializableExample::from)
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct SerializableSignal<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: &'a Option<String>,
    strategy: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weight: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "match")]
    match_pattern: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ast_query: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ast_scope: &'a Option<String>,
}

impl<'a> From<&'a crate::rules::Signal> for SerializableSignal<'a> {
    fn from(s: &'a crate::rules::Signal) -> Self {
        Self {
            id: &s.id,
            strategy: &s.strategy,
            description: &s.description,
            weight: &s.weight,
            match_pattern: &s.match_pattern,
            ast_query: &s.ast_query,
            ast_scope: &s.ast_scope,
        }
    }
}

#[derive(Serialize)]
struct SerializableExample<'a> {
    code: &'a str,
    verdict: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    language: &'a Option<String>,
}

impl<'a> From<&'a crate::rules::GoldenExample> for SerializableExample<'a> {
    fn from(e: &'a crate::rules::GoldenExample) -> Self {
        Self {
            code: &e.code,
            verdict: &e.verdict,
            reason: &e.reason,
            language: &e.language,
        }
    }
}

// ── Diff / summary for `wh review diff` (3D.1.3) ──

/// Summarize what would change if a proposal bundle were imported without
/// modifying anything on disk. Used by `wh review diff <bundle>`.
pub fn diff(project_dir: &Path, bundle_path: &Path) -> Result<Value> {
    let bundle = load_bundle(bundle_path)?;
    if bundle.version != BUNDLE_VERSION {
        return Err(anyhow!(
            "unsupported bundle version {} (expected {})",
            bundle.version,
            BUNDLE_VERSION
        ));
    }
    let existing = existing_rule_map(project_dir, &bundle.dependency.language, &bundle.dependency.name);

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut replacing_candidate = Vec::new();
    let mut conflicts = Vec::new();

    for p in &bundle.proposals {
        match existing.get(&p.id) {
            None => added.push(summary_added(p)),
            Some(existing_rule) => {
                let status = existing_rule
                    .status
                    .clone()
                    .unwrap_or_else(|| if existing_rule.approved { "approved".into() } else { "candidate".into() });
                if status == "candidate" {
                    replacing_candidate.push(summary_modified(p, existing_rule));
                } else {
                    conflicts.push(serde_json::json!({
                        "id": p.id,
                        "existing_status": status,
                        "note": "deprecate or supersede the existing rule first",
                    }));
                }
                if matches_without_trivia(existing_rule, p) {
                    // Identical re-import — flag separately so agents can
                    // detect redundant bundles.
                    modified.push(serde_json::json!({
                        "id": p.id,
                        "kind": "no-op",
                        "note": "proposal matches existing candidate exactly",
                    }));
                } else {
                    modified.push(summary_modified(p, existing_rule));
                }
            }
        }
    }

    // Detect deprecation candidates: existing approved rules in the same
    // dep whose ids are not in the new bundle. Policy is advisory — we do
    // not deprecate anything automatically, only flag.
    let incoming_ids: std::collections::HashSet<&str> =
        bundle.proposals.iter().map(|p| p.id.as_str()).collect();
    let candidate_deprecations: Vec<Value> = existing
        .iter()
        .filter(|(id, rule)| {
            !incoming_ids.contains(id.as_str())
                && rule
                    .status
                    .as_deref()
                    .map(|s| s == "approved")
                    .unwrap_or(rule.approved)
        })
        .map(|(id, _)| {
            serde_json::json!({
                "id": id,
                "note": "approved rule not present in incoming bundle; consider `wh apply <id> --deprecate`",
            })
        })
        .collect();

    Ok(serde_json::json!({
        "status": if conflicts.is_empty() { "ok" } else { "conflicts" },
        "bundle_path": bundle_path.display().to_string(),
        "dependency": {
            "name": bundle.dependency.name,
            "language": bundle.dependency.language,
        },
        "summary": {
            "added": added.len(),
            "modified": modified.iter().filter(|m| m.get("kind").and_then(|k| k.as_str()) != Some("no-op")).count(),
            "no_ops": modified.iter().filter(|m| m.get("kind").and_then(|k| k.as_str()) == Some("no-op")).count(),
            "conflicts": conflicts.len(),
            "candidate_deprecations": candidate_deprecations.len(),
        },
        "added": added,
        "modified": modified,
        "replacing_candidate": replacing_candidate,
        "conflicts": conflicts,
        "candidate_deprecations": candidate_deprecations,
        "next_command": "wh propose import <bundle> --overwrite-candidates # (after reviewing)",
    }))
}

fn existing_rule_map(
    project_dir: &Path,
    language: &str,
    dep_name: &str,
) -> BTreeMap<String, Rule> {
    let rules_dir = project_dir.join("whetstone").join("rules");
    let (files, _w) = load_rule_files(&rules_dir);
    let mut out = BTreeMap::new();
    for lrf in files {
        let LoadedRuleFile {
            rule_file, language: lang, ..
        } = lrf;
        if lang.as_deref() != Some(language) {
            continue;
        }
        if rule_file.source.name != dep_name {
            continue;
        }
        for rule in rule_file.rules {
            out.insert(rule.id.clone(), rule);
        }
    }
    out
}

fn summary_added(p: &ProposedRule) -> Value {
    serde_json::json!({
        "id": p.id,
        "kind": "added",
        "severity": p.severity,
        "confidence": p.confidence,
        "category": p.category,
        "source_url": p.source_url,
        "signals": p.signals.iter().map(|s| s.strategy.clone()).collect::<Vec<_>>(),
    })
}

fn summary_modified(p: &ProposedRule, existing: &Rule) -> Value {
    serde_json::json!({
        "id": p.id,
        "kind": "modified",
        "existing_status": existing
            .status
            .clone()
            .unwrap_or_else(|| if existing.approved { "approved".into() } else { "candidate".into() }),
        "changed_fields": changed_fields(existing, p),
    })
}

fn changed_fields(existing: &Rule, p: &ProposedRule) -> Vec<String> {
    let mut out = Vec::new();
    if existing.severity.as_deref() != Some(p.severity.as_str()) {
        out.push("severity".into());
    }
    if existing.confidence.as_deref() != Some(p.confidence.as_str()) {
        out.push("confidence".into());
    }
    if existing.category.as_deref() != Some(p.category.as_str()) {
        out.push("category".into());
    }
    if existing.description.as_deref() != Some(p.description.as_str()) {
        out.push("description".into());
    }
    if existing.source_url.as_deref() != Some(p.source_url.as_str()) {
        out.push("source_url".into());
    }
    let existing_strategies: Vec<&str> = existing.signals.iter().map(|s| s.strategy.as_str()).collect();
    let proposed_strategies: Vec<&str> = p.signals.iter().map(|s| s.strategy.as_str()).collect();
    if existing_strategies != proposed_strategies {
        out.push("signals".into());
    }
    if existing.golden_examples.len() != p.golden_examples.len() {
        out.push("golden_examples".into());
    }
    out
}

fn matches_without_trivia(existing: &Rule, p: &ProposedRule) -> bool {
    existing.severity.as_deref() == Some(p.severity.as_str())
        && existing.confidence.as_deref() == Some(p.confidence.as_str())
        && existing.category.as_deref() == Some(p.category.as_str())
        && existing.description.as_deref() == Some(p.description.as_str())
        && existing.source_url.as_deref() == Some(p.source_url.as_str())
        && existing.signals.len() == p.signals.len()
        && existing.golden_examples.len() == p.golden_examples.len()
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn good_bundle(name: &str) -> ProposalBundle {
        ProposalBundle {
            version: BUNDLE_VERSION,
            proposed_by: Some("test".into()),
            proposed_at: None,
            dependency: ProposalDependency {
                name: name.to_string(),
                language: "python".into(),
                version: Some("1.0".into()),
                source_url: Some("https://example.com".into()),
                content_hash: Some("sha256:abc".into()),
                registry: Some("pypi".into()),
                resolved_at: None,
            },
            proposals: vec![ProposedRule {
                id: format!("{name}.my-rule"),
                severity: "must".into(),
                confidence: "high".into(),
                category: "default".into(),
                description: "desc".into(),
                source_url: "https://example.com/rule".into(),
                source_quote: None,
                source_kind: Some("official_docs".into()),
                risk: None,
                linter_gap: None,
                signals: vec![ProposedSignal {
                    id: Some("s1".into()),
                    strategy: "pattern".into(),
                    description: Some("d".into()),
                    weight: Some("required".into()),
                    match_pattern: Some("x".into()),
                    ast_query: None,
                    ast_scope: None,
                }],
                golden_examples: vec![
                    ProposedExample {
                        code: "".into(),
                        verdict: "pass".into(),
                        reason: Some("r".into()),
                        language: None,
                    },
                    ProposedExample {
                        code: "y".into(),
                        verdict: "fail".into(),
                        reason: Some("bad".into()),
                        language: None,
                    },
                    ProposedExample {
                        code: "z".into(),
                        verdict: "pass".into(),
                        reason: Some("ok".into()),
                        language: None,
                    },
                ],
                deterministic_pass_threshold: None,
                deterministic_fail_threshold: None,
                ai_eval: None,
            }],
        }
    }

    #[test]
    fn validate_accepts_good_bundle() {
        let cfg = WhetstoneConfig::default();
        let b = good_bundle("fastapi");
        validate_bundle(&b, &cfg).unwrap();
    }

    #[test]
    fn validate_rejects_no_deterministic_signal() {
        let cfg = WhetstoneConfig::default();
        let mut b = good_bundle("fastapi");
        b.proposals[0].signals[0].strategy = "ai".into();
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("ast or pattern"));
    }

    #[test]
    fn validate_rejects_excluded_dep() {
        let mut cfg = WhetstoneConfig::default();
        cfg.extraction.exclude = vec!["fastapi".into()];
        let b = good_bundle("fastapi");
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("filtered out"));
    }

    #[test]
    fn validate_enforces_category_allowlist() {
        let mut cfg = WhetstoneConfig::default();
        cfg.extraction.allowed_categories = vec!["migration".into()];
        let b = good_bundle("fastapi");
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("allowed_categories"));
    }

    #[test]
    fn validate_enforces_min_confidence_high() {
        let mut cfg = WhetstoneConfig::default();
        cfg.extraction.min_confidence = Some("high".into());
        let mut b = good_bundle("fastapi");
        b.proposals[0].confidence = "medium".into();
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("min_confidence"));
    }

    #[test]
    fn validate_rejects_too_few_golden_examples() {
        let cfg = WhetstoneConfig::default();
        let mut b = good_bundle("fastapi");
        b.proposals[0].golden_examples.truncate(1);
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("3 to 5 golden examples"), "{err}");
    }

    #[test]
    fn validate_rejects_pass_only_golden_examples() {
        let cfg = WhetstoneConfig::default();
        let mut b = good_bundle("fastapi");
        for e in &mut b.proposals[0].golden_examples {
            e.verdict = "pass".into();
        }
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("pass"), "{err}");
        assert!(err.contains("fail"), "{err}");
    }

    #[test]
    fn validate_enforces_max_rules_per_dep() {
        let mut cfg = WhetstoneConfig::default();
        cfg.extraction.max_rules_per_dep = Some(0);
        let b = good_bundle("fastapi");
        let err = validate_bundle(&b, &cfg).unwrap_err().to_string();
        assert!(err.contains("max_rules_per_dep"));
    }

    #[test]
    fn import_writes_candidate_yaml() {
        let td = tempfile::tempdir().unwrap();
        let bundle_path = td.path().join("bundle.yaml");
        fs::write(
            &bundle_path,
            serde_yaml::to_string(&good_bundle("fastapi")).unwrap(),
        )
        .unwrap();

        let out = import(ImportOptions {
            project_dir: td.path(),
            bundle_path: &bundle_path,
            dry_run: false,
            actor: Some("pytest"),
            overwrite_candidates: false,
        })
        .unwrap();

        assert_eq!(out["status"], "ok");
        assert_eq!(out["action"], "imported");
        let target = td
            .path()
            .join("whetstone/rules/python/fastapi.yaml");
        let text = fs::read_to_string(&target).unwrap();
        assert!(text.contains("fastapi.my-rule"));
        assert!(text.contains("status: candidate"));
        assert!(text.contains("proposed_by: pytest"));
    }

    #[test]
    fn import_refuses_to_overwrite_approved_rule() {
        let td = tempfile::tempdir().unwrap();
        let rules_dir = td.path().join("whetstone/rules/python");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(
            rules_dir.join("fastapi.yaml"),
            r#"source:
  name: fastapi
  docs_url: https://example.com
  version: "1.0"
  content_hash: "sha256:old"
  resolved_at: "2026-01-01T00:00:00Z"
  registry: pypi
rules:
  - id: fastapi.my-rule
    severity: must
    confidence: high
    category: default
    description: prior rule
    source_url: https://example.com/old
    approved: true
    status: approved
    signals:
      - id: s1
        strategy: pattern
        description: x
        weight: required
        match: y
    golden_examples:
      - code: ""
        verdict: pass
        reason: ok
"#,
        )
        .unwrap();

        let bundle_path = td.path().join("bundle.yaml");
        fs::write(
            &bundle_path,
            serde_yaml::to_string(&good_bundle("fastapi")).unwrap(),
        )
        .unwrap();

        let err = import(ImportOptions {
            project_dir: td.path(),
            bundle_path: &bundle_path,
            dry_run: false,
            actor: None,
            overwrite_candidates: false,
        })
        .unwrap_err();
        assert!(err.to_string().contains("approved"), "{err}");
    }

    #[test]
    fn diff_reports_added_and_conflicts() {
        let td = tempfile::tempdir().unwrap();
        let rules_dir = td.path().join("whetstone/rules/python");
        fs::create_dir_all(&rules_dir).unwrap();
        fs::write(
            rules_dir.join("fastapi.yaml"),
            r#"source:
  name: fastapi
  docs_url: https://example.com
  version: "1.0"
  content_hash: "sha256:old"
  resolved_at: "2026-01-01T00:00:00Z"
  registry: pypi
rules:
  - id: fastapi.existing-approved
    severity: must
    confidence: high
    category: default
    description: prior rule
    source_url: https://example.com/old
    approved: true
    status: approved
    signals:
      - id: s1
        strategy: pattern
        weight: required
        match: y
    golden_examples:
      - code: ""
        verdict: pass
        reason: ok
"#,
        )
        .unwrap();

        let mut bundle = good_bundle("fastapi");
        bundle.proposals[0].id = "fastapi.existing-approved".into();
        bundle.proposals.push(ProposedRule {
            id: "fastapi.brand-new".into(),
            severity: "should".into(),
            confidence: "medium".into(),
            category: "convention".into(),
            description: "new".into(),
            source_url: "https://example.com/new".into(),
            source_quote: None,
            source_kind: None,
            risk: None,
            linter_gap: None,
            signals: vec![ProposedSignal {
                id: None,
                strategy: "pattern".into(),
                description: None,
                weight: Some("strong".into()),
                match_pattern: Some("x".into()),
                ast_query: None,
                ast_scope: None,
            }],
            golden_examples: vec![
                ProposedExample {
                    code: "".into(),
                    verdict: "pass".into(),
                    reason: None,
                    language: None,
                },
                ProposedExample {
                    code: "y".into(),
                    verdict: "fail".into(),
                    reason: None,
                    language: None,
                },
                ProposedExample {
                    code: "z".into(),
                    verdict: "pass".into(),
                    reason: None,
                    language: None,
                },
            ],
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
            ai_eval: None,
        });

        let bundle_path = td.path().join("bundle.yaml");
        fs::write(
            &bundle_path,
            serde_yaml::to_string(&bundle).unwrap(),
        )
        .unwrap();

        let out = diff(td.path(), &bundle_path).unwrap();
        assert_eq!(out["status"], "conflicts");
        assert_eq!(out["summary"]["added"].as_u64().unwrap(), 1);
        assert_eq!(out["summary"]["conflicts"].as_u64().unwrap(), 1);
    }
}
