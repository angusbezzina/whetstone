//! `wh scan` — scan source files and report rule violations.
//!
//! Signal handling:
//! - `ast` with `ast_query:` → tree-sitter S-expression query evaluated
//!   against the parsed file; every `@match` capture is a violation.
//! - `ast` with `match:` only → regex fallback; documented as weaker than
//!   a real AST check so extractors can upgrade incrementally.
//! - `pattern` with `match:` → regex scan. When `ast_scope:` is set, the
//!   regex is restricted to the source span of AST nodes whose kind
//!   matches (e.g. `function_definition`); this removes comment/no-op
//!   false positives for scope-sensitive rules.
//! - `lint_proxy` → verified against the project's linter config (ruff,
//!   biome). Missing rules surface as `config_issues` so the user gets
//!   actionable guidance rather than a silent pass.
//! - `ai` → skipped with a note; evaluated only via `wh eval run`.

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tree_sitter::{Query, QueryCursor, Tree};

use crate::ast::{self, AstLang};
use crate::detect::walk::SKIP_DIRS;
use crate::layers;
use crate::rules::ApprovedRule;

mod lint_proxy;

pub struct CheckOptions<'a> {
    pub project_dir: &'a Path,
    pub scan_paths: &'a [PathBuf],
    pub lang_filter: Option<&'a str>,
    pub rule_filter: Option<&'a [String]>,
    /// Candidate packs to preview as if imported (whetstone-653). When non-empty,
    /// resolution is READ-ONLY (no pack-cache writes) and each violation is tagged
    /// with `from_candidate`; a `preview` summary is added to the output.
    pub injected_packs: &'a [crate::config_packs::ResolvedConfigPack],
}

pub fn run(opts: CheckOptions<'_>) -> Result<Value> {
    let project_dir = opts.project_dir;
    let project_initialized = layers::project_is_initialized(project_dir);
    let previewing = !opts.injected_packs.is_empty();

    // Rule ids contributed by the candidate packs (post language filter), used to
    // label preview hits. Derived from provenance so the label matches the merge.
    let candidate_ids: BTreeSet<String> = if previewing {
        crate::config_packs::merge_pack_rules_tagged(opts.injected_packs, opts.lang_filter)
            .0
            .into_iter()
            .filter(|(_, origin)| origin.is_candidate())
            .map(|(rule, _)| rule.id)
            .collect()
    } else {
        BTreeSet::new()
    };

    let rules: Vec<ApprovedRule> = if project_initialized || previewing {
        // Preview injects the candidate through the identical merge seam
        // (read-only), so shadowing + denies apply exactly as a real import would.
        let snap_opts = crate::config::SnapshotOptions {
            read_only: previewing,
            injected_packs: opts.injected_packs.to_vec(),
        };
        let merged =
            layers::resolve_merged_with(project_dir, opts.lang_filter, true, true, false, &snap_opts);
        merged.merged.into_iter().map(|lr| lr.rule).collect()
    } else {
        let paths = layers::LayerPaths::for_project(project_dir);
        let (r, _) = crate::rules::load_approved_rules(&paths.project_rules_dir, opts.lang_filter);
        r
    };

    let rule_filter: Option<BTreeSet<&str>> = opts
        .rule_filter
        .map(|ids| ids.iter().map(String::as_str).collect());

    let rules: Vec<&ApprovedRule> = rules
        .iter()
        .filter(|r| {
            rule_filter
                .as_ref()
                .map(|set| set.contains(r.id.as_str()))
                .unwrap_or(true)
        })
        .collect();

    if rules.is_empty() {
        return Ok(json!({
            "status": "ok",
            "violations_count": 0,
            "files_scanned": 0,
            "rules_applied": 0,
            "violations": [],
            "skipped": [],
            "config_issues": [],
            "warnings": ["No approved rules match the supplied filters."],
        }));
    }

    let compiled = compile_rules(&rules);
    let mut violations: Vec<Value> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut files_scanned: usize = 0;

    for crule in &compiled {
        for note in &crule.notes {
            skipped.push(json!({"rule_id": crule.rule.id, "reason": note}));
        }
    }

    let mut config_issues = lint_proxy::verify_lint_proxies(project_dir, &rules);
    config_issues.extend(lint_proxy::verify_formatter_directives(project_dir, &rules));
    config_issues.extend(lint_proxy::verify_test_bindings(project_dir, &rules));
    config_issues.extend(lint_proxy::verify_validator_bindings(project_dir, &rules));
    // In preview mode this must be read-only too, or resolving the effective
    // config re-writes the pack cache and breaks the zero-state promise
    // (whetstone-dva). Non-preview keeps the historical writing behavior.
    let cfg_opts = crate::config::SnapshotOptions {
        read_only: previewing,
        injected_packs: Vec::new(),
    };
    let validator_default_timeout = crate::config::WhetstoneConfig::load_with(project_dir, &cfg_opts)
        .resolve
        .timeout_seconds
        .unwrap_or(15);
    let mut seen_validator_runtime_issues: BTreeSet<String> = BTreeSet::new();

    let files = discover_source_files(opts.scan_paths);
    for (path, language, ast_lang) in &files {
        if let Some(filter) = opts.lang_filter {
            if !crate::types::language_matches_language(language, filter) {
                continue;
            }
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!("Failed to read {}: {e}", path.display()));
                continue;
            }
        };
        files_scanned += 1;

        let applicable: Vec<&CompiledRule> = compiled
            .iter()
            .filter(|cr| rule_applies_to_language(&cr.rule.language, language))
            .collect();
        if applicable.is_empty() {
            continue;
        }

        let tree = if applicable
            .iter()
            .any(|cr| cr.signals.iter().any(|s| s.needs_tree()))
        {
            match ast_lang {
                Some(lang) => match ast::parse(*lang, &text) {
                    Some(t) => Some(t),
                    None => {
                        warnings.push(format!("tree-sitter parse failed for {}", path.display()));
                        None
                    }
                },
                None => None,
            }
        } else {
            None
        };

        for crule in applicable {
            for sig in &crule.signals {
                let hits = apply_signal(sig, *ast_lang, &text, tree.as_ref());
                for hit in hits {
                    violations.push(json!({
                        "rule_id": crule.rule.id,
                        "severity": crule.rule.severity,
                        "category": crule.rule.category,
                        "description": first_line(&crule.rule.description),
                        "source_url": crule.rule.source_url,
                        "language": language,
                        "signal_id": sig.signal_id,
                        "signal_strategy": sig.strategy,
                        "signal_description": sig.description,
                        "signal_check_type": sig.check_kind(),
                        "file": path.display().to_string(),
                        "line": hit.line,
                        "column": hit.column,
                        "match": hit.text,
                    }));
                }
            }

            for validator in &crule.rule.validators {
                if validator.adapter != "command" {
                    continue;
                }
                match run_command_validator(
                    project_dir,
                    path,
                    language,
                    crule.rule,
                    validator,
                    validator_default_timeout,
                ) {
                    Ok(result) => {
                        warnings.extend(result.warnings);
                        for hit in result.violations {
                            violations.push(json!({
                                "rule_id": crule.rule.id,
                                "severity": crule.rule.severity,
                                "category": crule.rule.category,
                                "description": first_line(&crule.rule.description),
                                "source_url": crule.rule.source_url,
                                "language": language,
                                "signal_id": validator.rule,
                                "signal_strategy": format!("validator:{}", validator.adapter),
                                "signal_description": hit.message,
                                "signal_check_type": "validator",
                                "file": path.display().to_string(),
                                "line": hit.line,
                                "column": hit.column,
                                "match": hit.text.unwrap_or_default(),
                                "validator_adapter": validator.adapter,
                                "validator_rule": validator.rule,
                            }));
                        }
                    }
                    Err(issue) => {
                        let key = format!("{}:{}:{}", crule.rule.id, validator.rule, issue);
                        if seen_validator_runtime_issues.insert(key) {
                            config_issues.push(json!({
                                "rule_id": crule.rule.id,
                                "adapter": validator.adapter,
                                "validator_rule": validator.rule,
                                "path": path.display().to_string(),
                                "issue": issue,
                                "fix": "fix the command validator configuration or executable output contract",
                            }));
                        }
                    }
                }
            }
        }
    }

    let violations_count = violations.len() as i64;
    let config_issue_count = config_issues.len() as i64;
    let status = if violations_count == 0 && config_issue_count == 0 {
        "ok"
    } else if violations_count > 0 {
        "violations_found"
    } else {
        "config_issues_found"
    };
    let mut result = json!({
        "status": status,
        "violations_count": violations_count,
        "config_issues_count": config_issue_count,
        "files_scanned": files_scanned,
        "rules_applied": rules.len(),
        "violations": violations,
        "config_issues": config_issues,
        "skipped": skipped,
        "warnings": warnings,
    });

    // Preview mode: tag each hit candidate-vs-configured and add a summary
    // (whetstone-653). Non-preview output is byte-for-byte unchanged.
    if previewing {
        let mut candidate_hits = 0i64;
        if let Some(vs) = result["violations"].as_array_mut() {
            for v in vs.iter_mut() {
                let rid = v.get("rule_id").and_then(|r| r.as_str()).unwrap_or("");
                let from_candidate = candidate_ids.contains(rid);
                if from_candidate {
                    candidate_hits += 1;
                }
                v["from_candidate"] = json!(from_candidate);
            }
        }
        let candidate_rules_active = rules
            .iter()
            .filter(|r| candidate_ids.contains(&r.id))
            .count();
        let names: Vec<String> = opts
            .injected_packs
            .iter()
            .map(crate::config_packs::pack_display_name)
            .collect();
        result["preview"] = json!({
            "candidate_packs": names,
            "candidate_rules": candidate_rules_active,
            "candidate_hits": candidate_hits,
            "configured_hits": violations_count - candidate_hits,
        });
    }

    Ok(result)
}

// ── Golden eval (the rule-quality bar) ──

/// Run every approved rule's golden examples through the REAL scanner and report
/// mismatches (a PASS example that fires a signal, or a FAIL example that does
/// not), plus a per-rule scorecard. This is the deterministic trust gate for the
/// corpus: a rule whose own examples don't behave as claimed is not high-confidence.
///
/// Source-fidelity is split per planning/skill-cli-boundary.md §2: STRUCTURAL
/// fidelity (the `source_quote` appears verbatim in the cached source docs) IS
/// gated here; SEMANTIC fidelity ("the doc actually supports the rule + merits its
/// severity") is judgment, so it is recorded as a skill-attested scorecard field,
/// never adjudicated by this deterministic command.
pub fn eval(project_dir: &Path, lang_filter: Option<&str>) -> Result<Value> {
    let project_initialized = layers::project_is_initialized(project_dir);
    let rules: Vec<ApprovedRule> = if project_initialized {
        let merged = layers::resolve_merged(project_dir, lang_filter, true, true, false);
        merged.merged.into_iter().map(|lr| lr.rule).collect()
    } else {
        let paths = layers::LayerPaths::for_project(project_dir);
        let (r, _) = crate::rules::load_approved_rules(&paths.project_rules_dir, lang_filter);
        r
    };

    let quote_map = load_source_quotes(project_dir);
    let content_map = load_cached_contents(project_dir);

    let mut mismatches: Vec<Value> = Vec::new();
    let mut fidelity_failures: Vec<Value> = Vec::new();
    let mut scorecards: Vec<Value> = Vec::new();

    for rule in &rules {
        let compiled = compile_rules(&[rule]);
        let crule = &compiled[0];
        let scanner_enforced = !crule.signals.is_empty();

        let mut golden_total = 0usize;
        let mut golden_checked = 0usize;
        let mut golden_pass = 0usize;
        for g in &rule.golden_examples {
            golden_total += 1;
            let lang_str = g.language.clone().unwrap_or_else(|| rule.language.clone());
            let Some(ast_lang) = AstLang::from_str(&lang_str) else {
                continue;
            };
            if !scanner_enforced {
                continue; // lint_proxy/formatter/tests-only rule: not scanner-checkable
            }
            let needs_tree = crule.signals.iter().any(|s| s.needs_tree());
            let tree = if needs_tree {
                ast::parse(ast_lang, &g.code)
            } else {
                None
            };
            let fires = crule
                .signals
                .iter()
                .any(|sig| !apply_signal(sig, Some(ast_lang), &g.code, tree.as_ref()).is_empty());
            let expected_fire = g.verdict.eq_ignore_ascii_case("fail");
            golden_checked += 1;
            if fires == expected_fire {
                golden_pass += 1;
            } else {
                mismatches.push(json!({
                    "rule_id": rule.id,
                    "verdict": g.verdict,
                    "expected_fire": expected_fire,
                    "actually_fired": fires,
                    "code": g.code,
                    "issue": if expected_fire {
                        "FAIL example did not trigger any signal"
                    } else {
                        "PASS example incorrectly triggered a signal"
                    },
                }));
            }
        }

        // Structural source-fidelity: the cited quote must appear in the cached docs.
        let mut source_quote_present = false;
        let mut source_quote_found: Option<bool> = None;
        if let Some(quote) = quote_map.get(&rule.id) {
            let quote = quote.trim();
            if !quote.is_empty() {
                source_quote_present = true;
                let key = (rule.language.to_lowercase(), rule.source_name.to_lowercase());
                let content = content_map.get(&key).or_else(|| {
                    content_map
                        .iter()
                        .find(|((_, n), _)| *n == rule.source_name.to_lowercase())
                        .map(|(_, c)| c)
                });
                if let Some(content) = content {
                    let found = content.contains(quote);
                    source_quote_found = Some(found);
                    if !found {
                        fidelity_failures.push(json!({
                            "rule_id": rule.id,
                            "issue": "source_quote not found verbatim in the cached source documentation",
                            "fix": "fix the quote to match the source, or re-resolve docs (wh reinit)",
                        }));
                    }
                }
                // No cached content -> cannot verify offline; left as null (not a failure).
            }
        }

        let deterministic = rule
            .signals
            .iter()
            .any(|s| matches!(s.strategy.as_str(), "ast" | "lint_proxy"))
            || rule.formatter.is_some()
            || !rule.tests.is_empty()
            || !rule.validators.is_empty();

        scorecards.push(json!({
            "rule_id": rule.id,
            "language": rule.language,
            "deterministic": deterministic,
            "scanner_enforced": scanner_enforced,
            "golden_total": golden_total,
            "golden_checked": golden_checked,
            "golden_passed": golden_pass,
            "source_url_present": !rule.source_url.is_empty(),
            "source_quote_present": source_quote_present,
            "source_quote_found_in_docs": source_quote_found,
            "semantic_source_fidelity": "unattested",
        }));
    }

    let ok = mismatches.is_empty() && fidelity_failures.is_empty();
    Ok(json!({
        "status": if ok { "ok" } else { "eval_failed" },
        "ok": ok,
        "rules_evaluated": rules.len(),
        "golden_mismatches": mismatches,
        "golden_mismatch_count": mismatches.len(),
        "source_fidelity_failures": fidelity_failures,
        "source_fidelity_failure_count": fidelity_failures.len(),
        "scorecards": scorecards,
    }))
}

fn load_cached_contents(project_dir: &Path) -> std::collections::HashMap<(String, String), String> {
    let mut sm = crate::state::StateManager::new(project_dir);
    let mut map = std::collections::HashMap::new();
    for entry in sm.cache.all_entries() {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        let lang = entry
            .get("language")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        if name.is_empty() {
            continue;
        }
        if let Some(content) = entry.get("content").and_then(|v| v.as_str()) {
            map.insert((lang, name), content.to_string());
        }
    }
    map
}

fn load_source_quotes(project_dir: &Path) -> std::collections::HashMap<String, String> {
    let paths = layers::LayerPaths::for_project(project_dir);
    let mut map = std::collections::HashMap::new();
    for dir in [paths.project_rules_dir, paths.personal_rules_dir] {
        if !dir.exists() {
            continue;
        }
        let (files, _) = crate::rules::load_rule_files(&dir);
        for lf in files {
            for r in &lf.rule_file.rules {
                if r.id.is_empty() {
                    continue;
                }
                if let Some(q) = &r.source_quote {
                    map.insert(r.id.clone(), q.clone());
                }
            }
        }
    }
    map
}

pub fn format_eval_output(result: &Value) -> String {
    let ok = result.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let evaluated = result
        .get("rules_evaluated")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let mism = result
        .get("golden_mismatch_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let fid = result
        .get("source_fidelity_failure_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let mut out = format!(
        "Whetstone eval: {} rule(s) — {} golden mismatch(es), {} source-fidelity failure(s)\n",
        evaluated, mism, fid
    );
    if let Some(arr) = result.get("golden_mismatches").and_then(|v| v.as_array()) {
        for m in arr {
            out.push_str(&format!(
                "  GOLDEN  {}: {}\n",
                m.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?"),
                m.get("issue").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
    }
    if let Some(arr) = result.get("source_fidelity_failures").and_then(|v| v.as_array()) {
        for f in arr {
            out.push_str(&format!(
                "  SOURCE  {}: {}\n",
                f.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?"),
                f.get("issue").and_then(|v| v.as_str()).unwrap_or("")
            ));
        }
    }
    out.push_str(if ok {
        "All rules pass their golden examples and structural source-fidelity.\n"
    } else {
        "Eval FAILED — fix the rules above.\n"
    });
    out
}

// ── Human formatter ──

pub fn format_human_output(result: &Value) -> String {
    let mut out = String::new();
    let count = result
        .get("violations_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let config_count = result
        .get("config_issues_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let files = result
        .get("files_scanned")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let rules = result
        .get("rules_applied")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);

    if count == 0 && config_count == 0 {
        out.push_str(&format!(
            "wh scan: no violations ({rules} rule(s), {files} file(s))\n"
        ));
        return out;
    }

    out.push_str(&format!(
        "wh scan: {count} violation(s), {config_count} config issue(s) across {files} file(s) against {rules} rule(s)\n"
    ));
    if let Some(arr) = result.get("violations").and_then(|v| v.as_array()) {
        for v in arr {
            let file = v.get("file").and_then(|v| v.as_str()).unwrap_or("?");
            let line = v.get("line").and_then(|v| v.as_i64()).unwrap_or(0);
            let sev = v.get("severity").and_then(|v| v.as_str()).unwrap_or("?");
            let id = v.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?");
            let desc = v.get("description").and_then(|v| v.as_str()).unwrap_or("");
            out.push_str(&format!("  {file}:{line}: [{sev}] {id} — {desc}\n"));
        }
    }
    if let Some(arr) = result.get("config_issues").and_then(|v| v.as_array()) {
        for v in arr {
            let id = v.get("rule_id").and_then(|v| v.as_str()).unwrap_or("?");
            let issue = v.get("issue").and_then(|v| v.as_str()).unwrap_or("?");
            if let (Some(linter), Some(code)) = (
                v.get("linter").and_then(|v| v.as_str()),
                v.get("code").and_then(|v| v.as_str()),
            ) {
                out.push_str(&format!("  config[{linter}:{code}] {id}: {issue}\n"));
            } else if let (Some(tool), Some(option)) = (
                v.get("tool").and_then(|v| v.as_str()),
                v.get("option").and_then(|v| v.as_str()),
            ) {
                out.push_str(&format!("  formatter[{tool}:{option}] {id}: {issue}\n"));
            } else if let (Some(runner), Some(path)) = (
                v.get("runner").and_then(|v| v.as_str()),
                v.get("path").and_then(|v| v.as_str()),
            ) {
                out.push_str(&format!("  test[{runner}:{path}] {id}: {issue}\n"));
            } else {
                out.push_str(&format!("  config {id}: {issue}\n"));
            }
        }
    }
    out
}

// ── Internal types ──

struct CompiledRule<'a> {
    rule: &'a ApprovedRule,
    signals: Vec<CompiledSignal>,
    notes: Vec<String>,
}

struct CompiledSignal {
    signal_id: String,
    description: String,
    strategy: String,
    regex: Option<Regex>,
    ast_query: Option<String>,
    ast_scope: Option<String>,
}

impl CompiledSignal {
    fn needs_tree(&self) -> bool {
        self.ast_query.is_some() || self.ast_scope.is_some()
    }
    fn check_kind(&self) -> &'static str {
        if self.ast_query.is_some() {
            "ast_query"
        } else if self.ast_scope.is_some() {
            "ast_scoped_regex"
        } else {
            "regex"
        }
    }

    #[cfg(test)]
    fn new_regex(strategy: &str, pattern: &str) -> Self {
        CompiledSignal {
            signal_id: "s1".into(),
            description: "test".into(),
            strategy: strategy.into(),
            regex: Some(regex::Regex::new(pattern).unwrap()),
            ast_query: None,
            ast_scope: None,
        }
    }
}

#[derive(Debug)]
struct SignalHit {
    line: usize,
    column: usize,
    text: String,
}

struct ValidatorViolation {
    line: usize,
    column: usize,
    text: Option<String>,
    message: String,
}

struct CommandValidatorResult {
    violations: Vec<ValidatorViolation>,
    warnings: Vec<String>,
}

fn compile_rules<'a>(rules: &[&'a ApprovedRule]) -> Vec<CompiledRule<'a>> {
    let mut out = Vec::new();
    for rule in rules {
        let mut signals = Vec::new();
        let mut notes = Vec::new();
        for sig in &rule.signals {
            match sig.strategy.as_str() {
                "pattern" | "ast" => {
                    let regex = match sig.match_pattern.as_deref() {
                        Some(pat) => match Regex::new(pat) {
                            Ok(re) => Some(re),
                            Err(e) => {
                                notes.push(format!(
                                    "signal {}: invalid regex `{}` — {e}",
                                    sig.id, pat
                                ));
                                None
                            }
                        },
                        None => None,
                    };
                    if sig.strategy == "ast" && sig.ast_query.is_none() && regex.is_none() {
                        notes.push(format!(
                            "signal {}: ast signal has neither `ast_query:` nor `match:`; cannot enforce",
                            sig.id
                        ));
                        continue;
                    }
                    if sig.strategy == "pattern" && regex.is_none() {
                        notes.push(format!(
                            "signal {}: pattern signal has no `match:` regex; cannot enforce",
                            sig.id
                        ));
                        continue;
                    }
                    signals.push(CompiledSignal {
                        signal_id: sig.id.clone(),
                        description: sig.description.clone(),
                        strategy: sig.strategy.clone(),
                        regex,
                        ast_query: sig.ast_query.clone(),
                        ast_scope: sig.ast_scope.clone(),
                    });
                }
                "lint_proxy" => {
                    notes.push(format!(
                        "signal {}: lint_proxy checked via linter config; see config_issues",
                        sig.id
                    ));
                }
                other => notes.push(format!("signal {}: unknown strategy {other}", sig.id)),
            }
        }
        out.push(CompiledRule {
            rule,
            signals,
            notes,
        });
    }
    out
}

fn apply_signal(
    sig: &CompiledSignal,
    lang: Option<AstLang>,
    text: &str,
    tree: Option<&Tree>,
) -> Vec<SignalHit> {
    // Preferred path: tree-sitter query. If the tree is available, we trust
    // its result and return. If parsing failed (tree is None), fall through
    // so a `match:` regex can still enforce the rule rather than silently
    // letting violations slip past.
    if let Some(query_src) = &sig.ast_query {
        if let (Some(lang), Some(tree)) = (lang, tree) {
            return run_ast_query(query_src, lang, tree, text);
        }
    }
    let re = match &sig.regex {
        Some(r) => r,
        None => return Vec::new(),
    };
    if let Some(scope_kind) = &sig.ast_scope {
        if let Some(tree) = tree {
            return scan_with_ast_scope(re, scope_kind, tree, text);
        }
        // Tree parse failed; treat the signal as if `ast_scope` were absent
        // rather than dropping the check entirely.
    }
    scan_lines(re, text)
}

fn scan_lines(re: &Regex, text: &str) -> Vec<SignalHit> {
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for m in re.find_iter(line) {
            hits.push(SignalHit {
                line: i + 1,
                column: m.start() + 1,
                text: m.as_str().to_string(),
            });
        }
    }
    hits
}

fn run_command_validator(
    project_dir: &Path,
    file_path: &Path,
    language: &str,
    rule: &ApprovedRule,
    validator: &crate::rules::ApprovedValidatorBinding,
    default_timeout_seconds: u64,
) -> Result<CommandValidatorResult, String> {
    let timeout_seconds = validator
        .config
        .get("timeout_seconds")
        .and_then(|value| value.as_u64())
        .unwrap_or(default_timeout_seconds);
    let payload = json!({
        "rule_id": rule.id,
        "validator_rule": validator.rule,
        "adapter": validator.adapter,
        "language": language,
        "project_dir": project_dir.display().to_string(),
        "file": {
            "path": file_path.display().to_string(),
            "relative_path": file_path
                .strip_prefix(project_dir)
                .ok()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| file_path.display().to_string()),
        },
        "config": validator.config,
    });

    let mut command = if let Some(command_str) = validator
        .config
        .get("command")
        .and_then(|value| value.as_str())
    {
        if validator
            .config
            .get("allow_shell")
            .and_then(|value| value.as_bool())
            != Some(true)
        {
            return Err(
                "command validator using config.command requires config.allow_shell=true".into(),
            );
        }
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(command_str);
        command
    } else if let Some(path) = validator
        .config
        .get("path")
        .and_then(|value| value.as_str())
    {
        if !is_safe_repo_relative(path) {
            return Err("command validator path must stay within the repo".into());
        }
        let full_path = project_dir.join(path);
        if !full_path.exists() {
            return Err("command validator path does not exist".into());
        }
        if !path_resolves_within_repo(project_dir, &full_path) {
            return Err("command validator path resolves outside the repo".into());
        }
        Command::new(full_path)
    } else {
        return Err("command validator requires config.path or config.command".into());
    };

    let mut child = command
        .current_dir(project_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| format!("failed to spawn command validator: {err}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin
            .write_all(payload.to_string().as_bytes())
            .map_err(|err| format!("failed to write command validator payload: {err}"))?;
    }
    drop(child.stdin.take());

    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("failed waiting for command validator: {err}"))?
        {
            let output = child
                .wait_with_output()
                .map_err(|err| format!("failed to read command validator output: {err}"))?;
            if !status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if stderr.is_empty() {
                    format!("command validator exited with status {status}")
                } else {
                    format!("command validator exited with status {status}: {stderr}")
                });
            }
            return parse_command_validator_output(&output.stdout);
        }
        if started.elapsed() > Duration::from_secs(timeout_seconds.max(1)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "command validator timed out after {} second(s)",
                timeout_seconds.max(1)
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn parse_command_validator_output(stdout: &[u8]) -> Result<CommandValidatorResult, String> {
    if stdout.is_empty() {
        return Ok(CommandValidatorResult {
            violations: Vec::new(),
            warnings: Vec::new(),
        });
    }

    let parsed: Value = serde_json::from_slice(stdout)
        .map_err(|err| format!("command validator returned invalid JSON: {err}"))?;
    let violations_value = if parsed.is_array() {
        &parsed
    } else {
        parsed.get("violations").unwrap_or(&Value::Null)
    };
    let warnings = parsed
        .get("warnings")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let Some(violations_array) = violations_value.as_array() else {
        return Err(
            "command validator JSON must contain a `violations` array or be a JSON array".into(),
        );
    };

    let mut violations = Vec::new();
    for violation in violations_array {
        let line = violation
            .get("line")
            .and_then(|value| value.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let column = violation
            .get("column")
            .and_then(|value| value.as_u64())
            .unwrap_or(1)
            .max(1) as usize;
        let message = violation
            .get("message")
            .and_then(|value| value.as_str())
            .unwrap_or("command validator reported a violation")
            .to_string();
        let text = violation
            .get("match")
            .or_else(|| violation.get("text"))
            .and_then(|value| value.as_str())
            .map(String::from);

        violations.push(ValidatorViolation {
            line,
            column,
            text,
            message,
        });
    }

    Ok(CommandValidatorResult {
        violations,
        warnings,
    })
}

fn is_safe_repo_relative(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
}

fn path_resolves_within_repo(project_dir: &Path, full_path: &Path) -> bool {
    let Ok(repo_root) = std::fs::canonicalize(project_dir) else {
        return false;
    };
    let Ok(resolved) = std::fs::canonicalize(full_path) else {
        return false;
    };
    resolved.starts_with(repo_root)
}

/// Run a raw tree-sitter query and turn every `@match` capture into a hit.
fn run_ast_query(query_src: &str, lang: AstLang, tree: &Tree, source: &str) -> Vec<SignalHit> {
    let language = match lang {
        AstLang::Python => tree_sitter_python::language(),
        AstLang::TypeScript => tree_sitter_typescript::language_tsx(),
        AstLang::Rust => tree_sitter_rust::language(),
    };
    let query = match Query::new(&language, query_src) {
        Ok(q) => q,
        Err(e) => {
            eprintln!(
                "Whetstone: skipping malformed ast_query for {}: {e}",
                lang.as_str()
            );
            return Vec::new();
        }
    };
    let match_index = query.capture_index_for_name("match");
    let mut cursor = QueryCursor::new();
    let bytes = source.as_bytes();
    let mut hits = Vec::new();
    for m in cursor.matches(&query, tree.root_node(), bytes) {
        for cap in m.captures {
            if let Some(ix) = match_index {
                if cap.index != ix {
                    continue;
                }
            }
            let start = cap.node.start_position();
            let range = cap.node.byte_range();
            let text = source.get(range.clone()).unwrap_or("").to_string();
            hits.push(SignalHit {
                line: start.row + 1,
                column: start.column + 1,
                text,
            });
        }
    }
    hits
}

/// For `ast_scope:` pattern signals, walk the tree and apply the regex only
/// to the source span of nodes whose kind matches `scope_kind`.
fn scan_with_ast_scope(re: &Regex, scope_kind: &str, tree: &Tree, source: &str) -> Vec<SignalHit> {
    let mut hits = Vec::new();
    let root = tree.root_node();
    let mut cursor = root.walk();
    walk_nodes(&mut cursor, scope_kind, source, re, &mut hits);
    hits
}

fn walk_nodes(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    scope_kind: &str,
    source: &str,
    re: &Regex,
    hits: &mut Vec<SignalHit>,
) {
    let node = cursor.node();
    if node.kind() == scope_kind {
        let start_byte = node.start_byte();
        let start_line = node.start_position().row;
        let span = source.get(node.byte_range()).unwrap_or("");
        for m in re.find_iter(span) {
            let offset = m.start();
            let (line_offset, col) = line_col_within(span, offset);
            let absolute_line = start_line + line_offset;
            let absolute_col = if line_offset == 0 {
                node.start_position().column + col
            } else {
                col
            };
            hits.push(SignalHit {
                line: absolute_line + 1,
                column: absolute_col + 1,
                text: m.as_str().to_string(),
            });
        }
        let _ = start_byte;
    }
    if cursor.goto_first_child() {
        loop {
            walk_nodes(cursor, scope_kind, source, re, hits);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

/// Given a substring `span` and a byte offset inside it, return the 0-based
/// (line_offset_within_span, column) pair. Used to turn an intra-span regex
/// hit into absolute source coordinates.
fn line_col_within(span: &str, offset: usize) -> (usize, usize) {
    let prefix = &span[..offset.min(span.len())];
    let line_offset = prefix.bytes().filter(|b| *b == b'\n').count();
    let col = match prefix.rfind('\n') {
        Some(nl) => prefix.len() - nl - 1,
        None => prefix.len(),
    };
    (line_offset, col)
}

// ── File discovery ──

fn discover_source_files(roots: &[PathBuf]) -> Vec<(PathBuf, String, Option<AstLang>)> {
    let mut out = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_entry(|e| {
                if e.depth() == 0 {
                    return true;
                }
                let name = e.file_name().to_string_lossy();
                !SKIP_DIRS.iter().any(|d| *d == name)
            })
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            if let Some(language) = crate::types::source_language_for_path(entry.path()) {
                let ast_lang = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .and_then(AstLang::from_extension);
                out.push((entry.into_path(), language.to_string(), ast_lang));
            }
        }
    }
    out
}

fn rule_applies_to_language(rule_lang: &str, file_lang: &str) -> bool {
    crate::types::language_matches_language(rule_lang, file_lang)
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn rule_with(id: &str, lang: &str, strategy: &str) -> ApprovedRule {
        ApprovedRule {
            id: id.into(),
            severity: "must".into(),
            confidence: "high".into(),
            category: "default".into(),
            description: "test rule".into(),
            source_url: "https://example".into(),
            source_name: "demo".into(),
            language: lang.into(),
            languages: vec![lang.into()],
            signals: vec![crate::rules::ApprovedSignal {
                id: "s1".into(),
                strategy: strategy.into(),
                description: "signal".into(),
                weight: "required".into(),
                match_pattern: None,
                ast_query: None,
                ast_scope: None,
                lint: None,
            }],
            formatter: None,
            tests: Vec::new(),
            validators: Vec::new(),
            provenance: None,
            golden_examples: Vec::new(),
            deterministic_pass_threshold: None,
            deterministic_fail_threshold: None,
        }
    }

    #[test]
    fn pattern_signal_fires_on_match() {
        let mut rule = rule_with("demo.unwrap", "rust", "pattern");
        rule.signals[0].match_pattern = Some(r"\.unwrap\(\)".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let hits = apply_signal(
            &compiled[0].signals[0],
            Some(AstLang::Rust),
            "let v = x.unwrap();\n",
            None,
        );
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].text, ".unwrap()");
    }

    #[test]
    fn invalid_regex_skipped_with_note() {
        let mut rule = rule_with("demo.bad", "python", "pattern");
        rule.signals[0].match_pattern = Some("[invalid".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        assert!(compiled[0].signals.is_empty());
        assert!(compiled[0]
            .notes
            .iter()
            .any(|n| n.contains("invalid regex")));
    }

    #[test]
    fn ast_query_reports_function_definitions_only() {
        // Python rule whose ast_query matches every def foo() in the file.
        let mut rule = rule_with("demo.nofun", "python", "ast");
        rule.signals[0].ast_query = Some("(function_definition) @match".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let source = "def a():\n    pass\n\ndef b():\n    pass\n\nx = 1\n";
        let tree = ast::parse(AstLang::Python, source).unwrap();
        let hits = apply_signal(
            &compiled[0].signals[0],
            Some(AstLang::Python),
            source,
            Some(&tree),
        );
        assert_eq!(hits.len(), 2, "got: {hits:?}");
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 4);
    }

    #[test]
    fn ast_query_matches_typescript_member_access() {
        // TypeScript bucket-3 demonstrator: a type-independent structural query
        // (window.* member access) fires precisely and ignores other receivers.
        let mut rule = rule_with("demo.win", "typescript", "ast");
        rule.signals[0].ast_query = Some(
            r#"((member_expression object: (identifier) @_o) (#eq? @_o "window")) @match"#.into(),
        );
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let source = "const x = window.location;\nconst y = local.value;\n";
        let tree = ast::parse(AstLang::TypeScript, source).unwrap();
        let hits = apply_signal(
            &compiled[0].signals[0],
            Some(AstLang::TypeScript),
            source,
            Some(&tree),
        );
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!(hits[0].line, 1);
    }

    #[test]
    fn ast_scope_restricts_regex_to_nodes_of_kind() {
        // Pattern rule that should only flag TODO inside function bodies,
        // not inside module-level comments.
        let mut rule = rule_with("demo.todo", "python", "pattern");
        rule.signals[0].match_pattern = Some("TODO".into());
        rule.signals[0].ast_scope = Some("function_definition".into());
        let rules = vec![&rule];
        let compiled = compile_rules(&rules);
        let source = "# module-level TODO should be ignored\n\ndef foo():\n    # TODO inside body should fire\n    pass\n";
        let tree = ast::parse(AstLang::Python, source).unwrap();
        let hits = apply_signal(
            &compiled[0].signals[0],
            Some(AstLang::Python),
            source,
            Some(&tree),
        );
        assert_eq!(hits.len(), 1, "got: {hits:?}");
        assert_eq!(hits[0].line, 4);
    }

    #[test]
    fn ast_query_falls_back_to_regex_when_tree_is_none() {
        // Simulate a parse failure (tree = None). With both ast_query and a
        // `match:` regex configured, the runner must fall back to the regex
        // so a grammar hiccup does not silently disable enforcement.
        let sig = CompiledSignal {
            signal_id: "s1".into(),
            description: "unwrap".into(),
            strategy: "ast".into(),
            regex: Some(regex::Regex::new(r"\.unwrap\(\)").unwrap()),
            ast_query: Some("(call_expression) @match".into()),
            ast_scope: None,
        };
        let hits = apply_signal(&sig, Some(AstLang::Rust), "let x = y.unwrap();\n", None);
        assert_eq!(
            hits.len(),
            1,
            "regex fallback should fire when tree is None"
        );
    }

    #[test]
    fn compiled_signal_new_regex_builds_a_usable_regex_signal() {
        let sig = CompiledSignal::new_regex("pattern", r"\.unwrap\(\)");
        assert!(sig.regex.is_some());
        assert!(sig.ast_query.is_none());
    }

    #[test]
    fn discover_filters_by_extension_and_skip_dirs() {
        let tmp = tempdir();
        std::fs::create_dir_all(tmp.join("src")).unwrap();
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        write_file(&tmp.join("src/a.py"), "x = 1\n");
        write_file(&tmp.join("src/b.rs"), "fn main() {}\n");
        write_file(&tmp.join("node_modules/c.ts"), "const x = 1;\n");
        write_file(
            &tmp.join("src/index.html"),
            "<button onclick=\"x()\"></button>\n",
        );
        let files = discover_source_files(std::slice::from_ref(&tmp));
        let names: Vec<_> = files
            .iter()
            .map(|(p, _, _)| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"a.py".to_string()));
        assert!(names.contains(&"b.rs".to_string()));
        assert!(names.contains(&"index.html".to_string()));
        assert!(!names.contains(&"c.ts".to_string()));
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "wh-check-test-{}",
            std::process::id() as u128 * 1_000_000 + rand_part()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    fn rand_part() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    fn write_file(path: &Path, body: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    const GOOD_RULE: &str = "source:\n  name: good\nrules:\n  - id: good.snake\n    severity: should\n    confidence: high\n    category: convention\n    description: snake_case function names\n    source_url: https://example.com\n    approved: true\n    status: approved\n    signals:\n      - id: s\n        strategy: ast\n        weight: required\n        ast_query: '((function_definition name: (identifier) @match) (#match? @match \"[A-Z]\"))'\n    golden_examples:\n      - code: \"def read_config():\\n    pass\\n\"\n        verdict: pass\n        reason: snake ok\n      - code: \"def ReadConfig():\\n    pass\\n\"\n        verdict: fail\n        reason: pascal bad\n";

    #[test]
    fn eval_passes_correct_goldens() {
        let tmp = tempdir();
        let dir = tmp.join("whetstone").join("rules").join("python");
        std::fs::create_dir_all(&dir).unwrap();
        write_file(&dir.join("good.yaml"), GOOD_RULE);
        let res = eval(&tmp, None).unwrap();
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(res["golden_mismatch_count"], 0);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn eval_fails_on_mislabeled_golden() {
        let tmp = tempdir();
        let dir = tmp.join("whetstone").join("rules").join("python");
        std::fs::create_dir_all(&dir).unwrap();
        // Same signal, but the PASS example is PascalCase -> it actually fires.
        let bad = GOOD_RULE.replace(
            "      - code: \"def read_config():\\n    pass\\n\"\n        verdict: pass\n        reason: snake ok\n",
            "      - code: \"def ReadConfig():\\n    pass\\n\"\n        verdict: pass\n        reason: MISLABELED\n",
        );
        write_file(&dir.join("bad.yaml"), &bad);
        let res = eval(&tmp, None).unwrap();
        assert_eq!(res["ok"], false, "{res}");
        assert!(res["golden_mismatch_count"].as_i64().unwrap() >= 1, "{res}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn eval_skips_lint_proxy_rules_from_golden_scan() {
        // A lint_proxy-only rule is not scanner-enforced; its goldens are not
        // run through the scanner (golden_checked == 0) and it must not fail eval.
        let tmp = tempdir();
        let dir = tmp.join("whetstone").join("rules").join("rust");
        std::fs::create_dir_all(&dir).unwrap();
        let rule = "source:\n  name: anyhow\nrules:\n  - id: anyhow.unwrap\n    severity: should\n    confidence: high\n    category: convention\n    description: prefer expect\n    source_url: https://example.com\n    approved: true\n    status: approved\n    signals:\n      - id: s\n        strategy: lint_proxy\n        weight: required\n        lint:\n          tool: clippy\n          code: unwrap_used\n    golden_examples:\n      - code: \"let x = y.unwrap();\"\n        verdict: fail\n        reason: bare unwrap\n";
        write_file(&dir.join("anyhow.yaml"), rule);
        let res = eval(&tmp, None).unwrap();
        assert_eq!(res["ok"], true, "{res}");
        let card = &res["scorecards"][0];
        assert_eq!(card["golden_checked"], 0, "{res}");
        assert_eq!(card["scanner_enforced"], false);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
