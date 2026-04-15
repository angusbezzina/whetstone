//! Whetstone configuration: global, project, personal.
//!
//! Precedence (later wins): built-in defaults < global (~/.whetstone/config.yaml)
//! < project (whetstone/whetstone.yaml or whetstone.yaml) < personal
//! (whetstone/.personal/config.yaml). Lists generally **append** across
//! layers (deny, sources.custom, extractions includes/excludes) so the
//! layering composes rather than clobbers. Scalar knobs (timeouts, TTLs,
//! quotas) use "last write wins" down the stack.
//!
//! Every key known to the loader is listed in `SUPPORTED_KEYS` and
//! validated on load. Unknown top-level keys and unknown nested keys
//! produce warnings surfaced by `wh config show`.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

// ── Known-key registry ──

/// Every supported config key, expressed as dotted paths. `wh config show`
/// compares a loaded file's keys against this list to warn on unknown
/// fields (typos, stale docs, surprises).
const SUPPORTED_KEYS: &[&str] = &[
    "discovery.exclude",
    "discovery.include",
    "generate.formats",
    "sources.custom",
    "deny",
    "extends",
    "extraction.include",
    "extraction.exclude",
    "extraction.max_rules_per_dep",
    "extraction.allowed_categories",
    "extraction.min_confidence",
    "extraction.preferred_source_kinds",
    "extraction.recency_window_days",
    "resolve.cache_ttl_seconds",
    "resolve.timeout_seconds",
    "resolve.workers",
    "check.paths",
    "check.fail_on",
    "bench.min_f1",
    "bench.corpus_dir",
    // Global-only keys:
    "default_languages",
    "default_formats",
];

/// Keys that only apply in the global `~/.whetstone/config.yaml` file.
/// Surfaced if a project or personal config tries to set them.
const GLOBAL_ONLY_KEYS: &[&str] = &["default_languages", "default_formats"];

/// Keys that only apply in the personal config (gitignored).
/// Empty today — reserved for future personal-only fields.
const PERSONAL_ONLY_KEYS: &[&str] = &[];

const VALID_CATEGORIES: &[&str] = &[
    "migration",
    "default",
    "convention",
    "breaking-change",
    "semantic",
];
const VALID_MIN_CONFIDENCE: &[&str] = &["high", "medium"];
const VALID_FAIL_ON: &[&str] = &["violations", "config_issues", "both", "none"];
const VALID_GENERATE_FORMATS: &[&str] = &[
    "agents.md",
    "claude.md",
    ".cursorrules",
    "copilot-instructions.md",
    ".windsurfrules",
    "codex.md",
];
const MAX_RULES_PER_DEP_HARD_LIMIT: u32 = 5;

// ── Effective config ──

/// Fully-merged configuration used by the rest of the binary.
/// Constructed by [`WhetstoneConfig::load`] after layering global →
/// project → personal config files.
///
/// Also deserializable directly from a single YAML file so callers that
/// only need one layer (e.g. the team resolver inspecting a sibling
/// project) can still `serde_yaml::from_str::<WhetstoneConfig>(...)`.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Deserialize)]
pub struct WhetstoneConfig {
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub generate: GenerateConfig,
    #[serde(default)]
    pub sources: SourcesConfig,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub extends: Vec<String>,
    #[serde(default)]
    pub extraction: ExtractionConfig,
    #[serde(default)]
    pub resolve: ResolveConfig,
    #[serde(default)]
    pub check: CheckConfig,
    #[serde(default)]
    pub bench: BenchConfig,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub include: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct GenerateConfig {
    #[serde(default)]
    pub formats: Vec<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct SourcesConfig {
    #[serde(default)]
    pub custom: Vec<CustomSource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CustomSource {
    pub url: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
}

/// Controls over which dependencies and proposals the extraction workflow
/// considers (3D.2.1). All fields are optional — absent means "no filter".
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ExtractionConfig {
    /// Dependency names to extract rules for. Empty = no restriction.
    #[serde(default)]
    pub include: Vec<String>,
    /// Dependency names to skip entirely.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Hard ceiling on rules per dependency. `None` = defer to the
    /// schema default (5).
    #[serde(default)]
    pub max_rules_per_dep: Option<u32>,
    /// Allowed rule categories. Empty or `None` = all five categories allowed.
    #[serde(default)]
    pub allowed_categories: Vec<String>,
    /// Minimum acceptable confidence for an imported proposal ("high"
    /// or "medium"). `None` accepts both.
    #[serde(default)]
    pub min_confidence: Option<String>,
    /// Preferred `source_kind` values, in priority order. Proposals whose
    /// source_kind is not in this list are still accepted but ranked lower.
    #[serde(default)]
    pub preferred_source_kinds: Vec<String>,
    /// Window (in days) over which a documentation source is considered
    /// "recent enough" to propose rules from. `None` = no window check.
    #[serde(default)]
    pub recency_window_days: Option<u32>,
}

/// Resolve-pipeline defaults (3D.2.2). Override the hardcoded CLI defaults
/// so a project can pin a timeout or cache TTL without every contributor
/// remembering the flag.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Deserialize)]
pub struct ResolveConfig {
    /// Cache TTL in seconds. `None` = 7 days.
    #[serde(default)]
    pub cache_ttl_seconds: Option<u64>,
    /// HTTP timeout in seconds. `None` = 15 seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Parallel resolution workers. `None` = CPU-count based.
    #[serde(default)]
    pub workers: Option<usize>,
}

/// `wh check` defaults (3D.2.2). Let projects codify "always scan src/"
/// without every run needing `wh check src/`.
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Deserialize)]
pub struct CheckConfig {
    /// Default scan paths if none are supplied on the command line.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Default `--fail-on` mode. Accepts "violations", "config_issues",
    /// "both", or "none".
    #[serde(default)]
    pub fail_on: Option<String>,
}

/// `wh bench` defaults (3D.2.2).
#[allow(dead_code)]
#[derive(Debug, Default, Clone, Deserialize)]
pub struct BenchConfig {
    /// Minimum F1 for `--check` gating. `None` = 1.0 (strict).
    #[serde(default)]
    pub min_f1: Option<f64>,
    /// Corpus directory relative to the project root. `None` = benchmarks/.
    #[serde(default)]
    pub corpus_dir: Option<String>,
}

// ── Raw (serde) shapes for each layer ──

#[derive(Debug, Default, Deserialize, Clone)]
struct RawGlobalConfig {
    #[serde(default)]
    default_languages: Vec<String>,
    #[serde(default)]
    default_formats: Vec<String>,
    #[serde(default)]
    sources: SourcesConfig,
    #[serde(default)]
    deny: Vec<String>,
    #[serde(default)]
    extraction: ExtractionConfig,
    #[serde(default)]
    resolve: ResolveConfig,
    #[serde(default)]
    check: CheckConfig,
    #[serde(default)]
    bench: BenchConfig,
}

/// Global per-user config read from `~/.whetstone/config.yaml`.
/// Supplies defaults that apply to every project the user runs Whetstone
/// against. Project/personal overrides stack on top.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct GlobalConfig {
    pub default_languages: Vec<String>,
    pub default_formats: Vec<String>,
    pub sources: SourcesConfig,
    pub deny: Vec<String>,
    pub extraction: ExtractionConfig,
    pub resolve: ResolveConfig,
    pub check: CheckConfig,
    pub bench: BenchConfig,
}

impl GlobalConfig {
    #[allow(dead_code)]
    pub fn load() -> Self {
        let (cfg, _diags) = Self::load_with_diagnostics();
        cfg
    }

    pub fn load_with_diagnostics() -> (Self, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        let Some(path) = global_config_path() else {
            return (Self::default(), diagnostics);
        };
        if !path.exists() {
            return (Self::default(), diagnostics);
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                diagnostics.push(Diagnostic::warning(
                    ConfigLayer::Global,
                    path.clone(),
                    format!("could not read: {e}"),
                ));
                return (Self::default(), diagnostics);
            }
        };
        let parsed = match serde_yaml::from_str::<RawGlobalConfig>(&text) {
            Ok(c) => c,
            Err(e) => {
                diagnostics.push(Diagnostic::error(
                    ConfigLayer::Global,
                    path.clone(),
                    format!("parse error: {e}"),
                ));
                return (Self::default(), diagnostics);
            }
        };
        validate_known_keys(
            &text,
            &path,
            ConfigLayer::Global,
            &allowed_keys_for(ConfigLayer::Global),
            &mut diagnostics,
        );
        let mut cfg = GlobalConfig {
            default_languages: parsed.default_languages,
            default_formats: parsed.default_formats,
            sources: parsed.sources,
            deny: parsed.deny,
            extraction: parsed.extraction,
            resolve: parsed.resolve,
            check: parsed.check,
            bench: parsed.bench,
        };
        validate_global_values(&mut cfg, &path, &mut diagnostics);
        (cfg, diagnostics)
    }
}

/// Per-user, per-project config at `whetstone/.personal/config.yaml`.
/// Gitignored. Carries overrides that should not leak to collaborators.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub struct PersonalConfig {
    pub deny: Vec<String>,
    pub generate: GenerateConfig,
    pub discovery: DiscoveryConfig,
    pub sources: SourcesConfig,
    pub extraction: ExtractionConfig,
    pub resolve: ResolveConfig,
    pub check: CheckConfig,
    pub bench: BenchConfig,
}

impl PersonalConfig {
    pub fn load(path: &Path) -> Self {
        let (cfg, _diags) = Self::load_with_diagnostics(path);
        cfg
    }

    pub fn load_with_diagnostics(path: &Path) -> (Self, Vec<Diagnostic>) {
        let mut diagnostics = Vec::new();
        if !path.exists() {
            return (Self::default(), diagnostics);
        }
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                diagnostics.push(Diagnostic::warning(
                    ConfigLayer::Personal,
                    path.to_path_buf(),
                    format!("could not read: {e}"),
                ));
                return (Self::default(), diagnostics);
            }
        };
        let parsed = match serde_yaml::from_str::<WhetstoneConfig>(&text) {
            Ok(c) => c,
            Err(e) => {
                diagnostics.push(Diagnostic::error(
                    ConfigLayer::Personal,
                    path.to_path_buf(),
                    format!("parse error: {e}"),
                ));
                return (Self::default(), diagnostics);
            }
        };
        validate_known_keys(
            &text,
            path,
            ConfigLayer::Personal,
            &allowed_keys_for(ConfigLayer::Personal),
            &mut diagnostics,
        );
        let mut cfg = PersonalConfig {
            deny: parsed.deny,
            generate: parsed.generate,
            discovery: parsed.discovery,
            sources: parsed.sources,
            extraction: parsed.extraction,
            resolve: parsed.resolve,
            check: parsed.check,
            bench: parsed.bench,
        };
        validate_personal_values(&mut cfg, path, &mut diagnostics);
        (cfg, diagnostics)
    }
}

// ── Diagnostics ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigLayer {
    Global,
    Project,
    Personal,
}

impl ConfigLayer {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfigLayer::Global => "global",
            ConfigLayer::Project => "project",
            ConfigLayer::Personal => "personal",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticLevel {
    Warning,
    Error,
}

impl DiagnosticLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Error => "error",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub level: DiagnosticLevel,
    pub layer: ConfigLayer,
    pub path: PathBuf,
    pub message: String,
}

impl Diagnostic {
    pub fn warning(layer: ConfigLayer, path: PathBuf, message: String) -> Self {
        Self {
            level: DiagnosticLevel::Warning,
            layer,
            path,
            message,
        }
    }
    pub fn error(layer: ConfigLayer, path: PathBuf, message: String) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            layer,
            path,
            message,
        }
    }
}

fn validate_global_values(cfg: &mut GlobalConfig, path: &Path, diagnostics: &mut Vec<Diagnostic>) {
    validate_format_list(
        &mut cfg.default_formats,
        ConfigLayer::Global,
        path,
        "default_formats",
        diagnostics,
    );
    validate_section_values(
        &mut cfg.extraction,
        &mut cfg.resolve,
        &mut cfg.check,
        &mut cfg.bench,
        ConfigLayer::Global,
        path,
        diagnostics,
    );
}

fn validate_personal_values(
    cfg: &mut PersonalConfig,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    validate_format_list(
        &mut cfg.generate.formats,
        ConfigLayer::Personal,
        path,
        "generate.formats",
        diagnostics,
    );
    validate_section_values(
        &mut cfg.extraction,
        &mut cfg.resolve,
        &mut cfg.check,
        &mut cfg.bench,
        ConfigLayer::Personal,
        path,
        diagnostics,
    );
}

fn validate_whetstone_values(
    cfg: &mut WhetstoneConfig,
    layer: ConfigLayer,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    validate_format_list(
        &mut cfg.generate.formats,
        layer,
        path,
        "generate.formats",
        diagnostics,
    );
    validate_section_values(
        &mut cfg.extraction,
        &mut cfg.resolve,
        &mut cfg.check,
        &mut cfg.bench,
        layer,
        path,
        diagnostics,
    );
}

fn validate_section_values(
    extraction: &mut ExtractionConfig,
    resolve: &mut ResolveConfig,
    check: &mut CheckConfig,
    bench: &mut BenchConfig,
    layer: ConfigLayer,
    path: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some(v) = extraction.max_rules_per_dep {
        if v == 0 || v > MAX_RULES_PER_DEP_HARD_LIMIT {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                format!(
                    "`extraction.max_rules_per_dep` must be between 1 and {MAX_RULES_PER_DEP_HARD_LIMIT}; ignoring {v}"
                ),
            ));
            extraction.max_rules_per_dep = None;
        }
    }

    if let Some(v) = extraction.min_confidence.as_deref() {
        if !VALID_MIN_CONFIDENCE.contains(&v) {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                format!(
                    "`extraction.min_confidence` must be one of {VALID_MIN_CONFIDENCE:?}; ignoring `{v}`"
                ),
            ));
            extraction.min_confidence = None;
        }
    }

    if !extraction.allowed_categories.is_empty() {
        let invalid: Vec<String> = extraction
            .allowed_categories
            .iter()
            .filter(|c| !VALID_CATEGORIES.contains(&c.as_str()))
            .cloned()
            .collect();
        if !invalid.is_empty() {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                format!(
                    "`extraction.allowed_categories` contains invalid entries {invalid:?}; valid: {VALID_CATEGORIES:?}"
                ),
            ));
            extraction
                .allowed_categories
                .retain(|c| VALID_CATEGORIES.contains(&c.as_str()));
        }
    }

    if let Some(v) = resolve.cache_ttl_seconds {
        if v == 0 {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                "`resolve.cache_ttl_seconds` must be > 0; ignoring 0".into(),
            ));
            resolve.cache_ttl_seconds = None;
        }
    }

    if let Some(v) = resolve.timeout_seconds {
        if v == 0 {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                "`resolve.timeout_seconds` must be > 0; ignoring 0".into(),
            ));
            resolve.timeout_seconds = None;
        }
    }

    if let Some(v) = resolve.workers {
        if v == 0 {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                "`resolve.workers` must be > 0; ignoring 0".into(),
            ));
            resolve.workers = None;
        }
    }

    if let Some(v) = check.fail_on.as_deref() {
        if !VALID_FAIL_ON.contains(&v) {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                format!("`check.fail_on` must be one of {VALID_FAIL_ON:?}; ignoring `{v}`"),
            ));
            check.fail_on = None;
        }
    }

    if let Some(v) = bench.min_f1 {
        if !(0.0..=1.0).contains(&v) {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                format!("`bench.min_f1` must be between 0.0 and 1.0; ignoring {v}"),
            ));
            bench.min_f1 = None;
        }
    }

    if let Some(v) = bench.corpus_dir.as_deref() {
        if v.trim().is_empty() {
            diagnostics.push(Diagnostic::error(
                layer,
                path.to_path_buf(),
                "`bench.corpus_dir` must not be empty; ignoring empty value".into(),
            ));
            bench.corpus_dir = None;
        }
    }
}

fn validate_format_list(
    formats: &mut Vec<String>,
    layer: ConfigLayer,
    path: &Path,
    key: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if formats.is_empty() {
        return;
    }
    let invalid: Vec<String> = formats
        .iter()
        .filter(|f| !VALID_GENERATE_FORMATS.contains(&f.as_str()))
        .cloned()
        .collect();
    if !invalid.is_empty() {
        diagnostics.push(Diagnostic::error(
            layer,
            path.to_path_buf(),
            format!(
                "`{key}` contains invalid entries {invalid:?}; valid: {VALID_GENERATE_FORMATS:?}"
            ),
        ));
        formats.retain(|f| VALID_GENERATE_FORMATS.contains(&f.as_str()));
    }
}

fn allowed_keys_for(layer: ConfigLayer) -> BTreeSet<&'static str> {
    let mut allowed: BTreeSet<&'static str> = SUPPORTED_KEYS.iter().copied().collect();
    match layer {
        ConfigLayer::Global => {
            for k in PERSONAL_ONLY_KEYS {
                allowed.remove(*k);
            }
        }
        ConfigLayer::Project => {
            for k in GLOBAL_ONLY_KEYS {
                allowed.remove(*k);
            }
            for k in PERSONAL_ONLY_KEYS {
                allowed.remove(*k);
            }
        }
        ConfigLayer::Personal => {
            for k in GLOBAL_ONLY_KEYS {
                allowed.remove(*k);
            }
        }
    }
    allowed
}

fn validate_known_keys(
    text: &str,
    path: &Path,
    layer: ConfigLayer,
    allowed: &BTreeSet<&'static str>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let parsed: serde_yaml::Value = match serde_yaml::from_str(text) {
        Ok(v) => v,
        Err(_) => return, // parse error already reported upstream
    };
    let mapping = match parsed {
        serde_yaml::Value::Mapping(m) => m,
        _ => return,
    };

    let mut observed: Vec<String> = Vec::new();
    collect_dotted_keys(&serde_yaml::Value::Mapping(mapping), "", &mut observed, 2);

    // A key is legal if it matches a supported key exactly, or if it
    // is a parent segment of one (e.g. `discovery` is the parent of
    // `discovery.exclude`). We do NOT accept arbitrary descendants of
    // a known parent — that would swallow typos like
    // `extraction.max_rules_per_deps` (note the trailing `s`).
    // Items inside a list-valued key (sources.custom[].url) are never
    // walked because `collect_dotted_keys` stops at sequence boundaries.
    let allowed_set: BTreeSet<&str> = allowed.iter().copied().collect();
    for key in observed {
        if allowed_set.contains(key.as_str()) {
            continue;
        }
        // Tolerate bare parent keys of a registered dotted key. E.g.
        // `discovery` appears in the walk before `discovery.exclude`;
        // no need to flag it.
        if allowed_set
            .iter()
            .any(|allowed_key| allowed_key.starts_with(&format!("{key}.")))
        {
            continue;
        }
        let suggestion = suggest_similar(&key, &allowed_set);
        let suffix = match suggestion {
            Some(s) => format!(" (did you mean `{s}`?)"),
            None => String::new(),
        };
        diagnostics.push(Diagnostic::warning(
            layer,
            path.to_path_buf(),
            format!("unknown config key `{key}`{suffix}"),
        ));
    }

    // Layer-scope hygiene: flag keys set in the wrong file.
    match layer {
        ConfigLayer::Project | ConfigLayer::Personal => {
            for key in GLOBAL_ONLY_KEYS {
                let root = key.split('.').next().unwrap_or(*key);
                if root_key_present(text, root) {
                    diagnostics.push(Diagnostic::warning(
                        layer,
                        path.to_path_buf(),
                        format!(
                            "key `{root}` only applies in ~/.whetstone/config.yaml; it is ignored here"
                        ),
                    ));
                }
            }
        }
        ConfigLayer::Global => {
            for key in PERSONAL_ONLY_KEYS {
                let root = key.split('.').next().unwrap_or(*key);
                if root_key_present(text, root) {
                    diagnostics.push(Diagnostic::warning(
                        layer,
                        path.to_path_buf(),
                        format!(
                            "key `{root}` only applies in whetstone/.personal/config.yaml; ignored here"
                        ),
                    ));
                }
            }
        }
    }
}

/// Walk a YAML mapping and produce dotted keys up to `max_depth` levels.
/// Values that are sequences or scalars terminate the walk; nested
/// mappings continue. Produced keys match the style used in
/// `SUPPORTED_KEYS` (e.g. `extraction.include`).
fn collect_dotted_keys(
    value: &serde_yaml::Value,
    prefix: &str,
    out: &mut Vec<String>,
    max_depth: usize,
) {
    let serde_yaml::Value::Mapping(map) = value else {
        return;
    };
    for (k, v) in map {
        let Some(key) = k.as_str() else { continue };
        let full = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        out.push(full.clone());
        if max_depth > 1 {
            if let serde_yaml::Value::Mapping(_) = v {
                collect_dotted_keys(v, &full, out, max_depth - 1);
            }
        }
    }
}

fn suggest_similar(key: &str, allowed: &BTreeSet<&str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for candidate in allowed {
        let d = levenshtein(key, candidate);
        if d <= 3 && best.map(|(b, _)| d < b).unwrap_or(true) {
            best = Some((d, candidate));
        }
    }
    best.map(|(_, s)| s.to_string())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let n = ac.len();
    let m = bc.len();
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut curr = vec![0usize; m + 1];
    for i in 1..=n {
        curr[0] = i;
        for j in 1..=m {
            let cost = if ac[i - 1] == bc[j - 1] { 0 } else { 1 };
            curr[j] = std::cmp::min(
                std::cmp::min(curr[j - 1] + 1, prev[j] + 1),
                prev[j - 1] + cost,
            );
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[m]
}

fn root_key_present(text: &str, key: &str) -> bool {
    text.lines().any(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            return false;
        }
        let prefix = format!("{key}:");
        line.starts_with(&prefix) || line.starts_with(&format!("{key} :"))
    })
}

// ── Paths ──

fn global_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".whetstone").join("config.yaml"))
}

fn project_config_candidates(project_dir: &Path) -> [PathBuf; 2] {
    [
        project_dir.join("whetstone").join("whetstone.yaml"),
        project_dir.join("whetstone.yaml"),
    ]
}

fn personal_config_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join("whetstone")
        .join(".personal")
        .join("config.yaml")
}

// ── Loading / merging ──

impl WhetstoneConfig {
    /// Load project + global config, merged into a single effective
    /// `WhetstoneConfig`. Personal overrides are applied via
    /// [`WhetstoneConfig::load_full`] — kept separate so callers that
    /// produce committed output can opt out of personal-layer effects.
    pub fn load(project_dir: &Path) -> Self {
        let snap = ConfigSnapshot::load(project_dir, true, false);
        snap.effective
    }

    /// Project-only load: ignores the global config entirely. Used by
    /// the team resolver when it inspects a sibling project.
    pub fn load_project_only(project_dir: &Path) -> Self {
        let snap = ConfigSnapshot::load(project_dir, false, false);
        snap.effective
    }

    /// Load every layer (global + project + personal). Returns a
    /// [`ConfigSnapshot`] with per-key provenance for `wh config show`.
    pub fn load_full(project_dir: &Path) -> ConfigSnapshot {
        ConfigSnapshot::load(project_dir, true, true)
    }
}

/// Fully-loaded view of the config stack, including diagnostics and
/// provenance. `wh config show` renders this directly.
#[allow(dead_code)]
pub struct ConfigSnapshot {
    pub effective: WhetstoneConfig,
    pub sources: BTreeMap<String, ProvenanceSource>,
    pub diagnostics: Vec<Diagnostic>,
    /// Which config files were actually read (skipping non-existent paths).
    pub loaded_files: Vec<LoadedFile>,
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    pub layer: ConfigLayer,
    pub path: PathBuf,
}

/// Which layer set each key on the effective config.
/// `Default` means nothing in the config files touched the key — the
/// struct default is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvenanceSource {
    /// No file in the stack touched this key; struct default is in force.
    /// Kept so `wh config show` can note "default" explicitly when we
    /// teach it to report unset keys (currently only set keys appear).
    #[allow(dead_code)]
    Default,
    Global,
    Project,
    Personal,
}

impl ProvenanceSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProvenanceSource::Default => "default",
            ProvenanceSource::Global => "global",
            ProvenanceSource::Project => "project",
            ProvenanceSource::Personal => "personal",
        }
    }
}

impl ConfigSnapshot {
    fn load(project_dir: &Path, include_global: bool, include_personal: bool) -> Self {
        let mut snap = ConfigSnapshot {
            effective: WhetstoneConfig::default(),
            sources: BTreeMap::new(),
            diagnostics: Vec::new(),
            loaded_files: Vec::new(),
        };

        // --- Global layer ---
        let mut global_cfg = GlobalConfig::default();
        if include_global {
            let (g, mut diags) = GlobalConfig::load_with_diagnostics();
            snap.diagnostics.append(&mut diags);
            if let Some(path) = global_config_path() {
                if path.exists() {
                    snap.loaded_files.push(LoadedFile {
                        layer: ConfigLayer::Global,
                        path,
                    });
                }
            }
            global_cfg = g;
        }

        // Global -> effective.
        if !global_cfg.default_formats.is_empty() {
            snap.effective.generate.formats = global_cfg.default_formats.clone();
            snap.sources
                .insert("generate.formats".into(), ProvenanceSource::Global);
        }
        if !global_cfg.sources.custom.is_empty() {
            snap.effective
                .sources
                .custom
                .extend(global_cfg.sources.custom.clone());
            snap.sources
                .insert("sources.custom".into(), ProvenanceSource::Global);
        }
        if !global_cfg.deny.is_empty() {
            snap.effective.deny.extend(global_cfg.deny.clone());
            snap.sources.insert("deny".into(), ProvenanceSource::Global);
        }
        apply_extraction(
            &mut snap.effective.extraction,
            &global_cfg.extraction,
            ProvenanceSource::Global,
            &mut snap.sources,
        );
        apply_resolve(
            &mut snap.effective.resolve,
            &global_cfg.resolve,
            ProvenanceSource::Global,
            &mut snap.sources,
        );
        apply_check(
            &mut snap.effective.check,
            &global_cfg.check,
            ProvenanceSource::Global,
            &mut snap.sources,
        );
        apply_bench(
            &mut snap.effective.bench,
            &global_cfg.bench,
            ProvenanceSource::Global,
            &mut snap.sources,
        );

        // --- Project layer ---
        let mut project_cfg: Option<WhetstoneConfig> = None;
        for path in project_config_candidates(project_dir) {
            if !path.exists() {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    snap.diagnostics.push(Diagnostic::warning(
                        ConfigLayer::Project,
                        path.clone(),
                        format!("could not read: {e}"),
                    ));
                    break;
                }
            };
            match serde_yaml::from_str::<WhetstoneConfig>(&text) {
                Ok(mut cfg) => {
                    snap.loaded_files.push(LoadedFile {
                        layer: ConfigLayer::Project,
                        path: path.clone(),
                    });
                    validate_known_keys(
                        &text,
                        &path,
                        ConfigLayer::Project,
                        &allowed_keys_for(ConfigLayer::Project),
                        &mut snap.diagnostics,
                    );
                    validate_whetstone_values(
                        &mut cfg,
                        ConfigLayer::Project,
                        &path,
                        &mut snap.diagnostics,
                    );
                    project_cfg = Some(cfg);
                }
                Err(e) => {
                    snap.diagnostics.push(Diagnostic::error(
                        ConfigLayer::Project,
                        path.clone(),
                        format!("parse error: {e}"),
                    ));
                }
            }
            break;
        }

        if let Some(cfg) = project_cfg {
            if !cfg.generate.formats.is_empty() {
                snap.effective.generate.formats = cfg.generate.formats;
                snap.sources
                    .insert("generate.formats".into(), ProvenanceSource::Project);
            }
            if !cfg.discovery.exclude.is_empty() {
                snap.effective.discovery.exclude = cfg.discovery.exclude;
                snap.sources
                    .insert("discovery.exclude".into(), ProvenanceSource::Project);
            }
            if !cfg.discovery.include.is_empty() {
                snap.effective.discovery.include = cfg.discovery.include;
                snap.sources
                    .insert("discovery.include".into(), ProvenanceSource::Project);
            }
            if !cfg.deny.is_empty() {
                snap.effective.deny.extend(cfg.deny);
                snap.effective.deny.sort();
                snap.effective.deny.dedup();
                snap.sources
                    .insert("deny".into(), ProvenanceSource::Project);
            }
            if !cfg.extends.is_empty() {
                snap.effective.extends = cfg.extends;
                snap.sources
                    .insert("extends".into(), ProvenanceSource::Project);
            }
            if !cfg.sources.custom.is_empty() {
                snap.effective.sources.custom.extend(cfg.sources.custom);
                snap.sources
                    .insert("sources.custom".into(), ProvenanceSource::Project);
            }
            apply_extraction(
                &mut snap.effective.extraction,
                &cfg.extraction,
                ProvenanceSource::Project,
                &mut snap.sources,
            );
            apply_resolve(
                &mut snap.effective.resolve,
                &cfg.resolve,
                ProvenanceSource::Project,
                &mut snap.sources,
            );
            apply_check(
                &mut snap.effective.check,
                &cfg.check,
                ProvenanceSource::Project,
                &mut snap.sources,
            );
            apply_bench(
                &mut snap.effective.bench,
                &cfg.bench,
                ProvenanceSource::Project,
                &mut snap.sources,
            );
        }

        // --- Personal layer ---
        if include_personal {
            let path = personal_config_path(project_dir);
            let (personal_cfg, mut diags) = PersonalConfig::load_with_diagnostics(&path);
            snap.diagnostics.append(&mut diags);
            if path.exists() {
                snap.loaded_files.push(LoadedFile {
                    layer: ConfigLayer::Personal,
                    path,
                });
            }
            if !personal_cfg.deny.is_empty() {
                snap.effective.deny.extend(personal_cfg.deny);
                snap.effective.deny.sort();
                snap.effective.deny.dedup();
                snap.sources
                    .insert("deny".into(), ProvenanceSource::Personal);
            }
            if !personal_cfg.generate.formats.is_empty() {
                snap.effective.generate.formats = personal_cfg.generate.formats;
                snap.sources
                    .insert("generate.formats".into(), ProvenanceSource::Personal);
            }
            if !personal_cfg.discovery.exclude.is_empty() {
                snap.effective.discovery.exclude = personal_cfg.discovery.exclude;
                snap.sources
                    .insert("discovery.exclude".into(), ProvenanceSource::Personal);
            }
            if !personal_cfg.discovery.include.is_empty() {
                snap.effective.discovery.include = personal_cfg.discovery.include;
                snap.sources
                    .insert("discovery.include".into(), ProvenanceSource::Personal);
            }
            if !personal_cfg.sources.custom.is_empty() {
                snap.effective
                    .sources
                    .custom
                    .extend(personal_cfg.sources.custom);
                snap.sources
                    .insert("sources.custom".into(), ProvenanceSource::Personal);
            }
            apply_extraction(
                &mut snap.effective.extraction,
                &personal_cfg.extraction,
                ProvenanceSource::Personal,
                &mut snap.sources,
            );
            apply_resolve(
                &mut snap.effective.resolve,
                &personal_cfg.resolve,
                ProvenanceSource::Personal,
                &mut snap.sources,
            );
            apply_check(
                &mut snap.effective.check,
                &personal_cfg.check,
                ProvenanceSource::Personal,
                &mut snap.sources,
            );
            apply_bench(
                &mut snap.effective.bench,
                &personal_cfg.bench,
                ProvenanceSource::Personal,
                &mut snap.sources,
            );
        }

        snap
    }

    /// Dump the effective config + provenance to JSON for `wh config show`.
    pub fn to_json(&self) -> serde_json::Value {
        let mut source_map = serde_json::Map::new();
        for (k, v) in &self.sources {
            source_map.insert(k.clone(), serde_json::Value::String(v.as_str().into()));
        }
        let diags: Vec<serde_json::Value> = self
            .diagnostics
            .iter()
            .map(|d| {
                serde_json::json!({
                    "level": d.level.as_str(),
                    "layer": d.layer.as_str(),
                    "path": d.path.display().to_string(),
                    "message": d.message,
                })
            })
            .collect();
        let loaded: Vec<serde_json::Value> = self
            .loaded_files
            .iter()
            .map(|f| {
                serde_json::json!({
                    "layer": f.layer.as_str(),
                    "path": f.path.display().to_string(),
                })
            })
            .collect();
        serde_json::json!({
            "status": if self.diagnostics.iter().any(|d| d.level == DiagnosticLevel::Error) {
                "error"
            } else {
                "ok"
            },
            "effective": effective_to_json(&self.effective),
            "sources": serde_json::Value::Object(source_map),
            "diagnostics": diags,
            "loaded_files": loaded,
            "supported_keys": SUPPORTED_KEYS,
            "precedence": ["default", "global", "project", "personal"],
        })
    }
}

fn effective_to_json(cfg: &WhetstoneConfig) -> serde_json::Value {
    serde_json::json!({
        "discovery": {
            "exclude": cfg.discovery.exclude,
            "include": cfg.discovery.include,
        },
        "generate": {
            "formats": cfg.generate.formats,
        },
        "sources": {
            "custom": cfg.sources.custom.iter().map(|c| {
                serde_json::json!({
                    "url": c.url,
                    "name": c.name,
                    "language": c.language,
                    "source_kind": c.source_kind,
                })
            }).collect::<Vec<_>>(),
        },
        "deny": cfg.deny,
        "extends": cfg.extends,
        "extraction": {
            "include": cfg.extraction.include,
            "exclude": cfg.extraction.exclude,
            "max_rules_per_dep": cfg.extraction.max_rules_per_dep,
            "allowed_categories": cfg.extraction.allowed_categories,
            "min_confidence": cfg.extraction.min_confidence,
            "preferred_source_kinds": cfg.extraction.preferred_source_kinds,
            "recency_window_days": cfg.extraction.recency_window_days,
        },
        "resolve": {
            "cache_ttl_seconds": cfg.resolve.cache_ttl_seconds,
            "timeout_seconds": cfg.resolve.timeout_seconds,
            "workers": cfg.resolve.workers,
        },
        "check": {
            "paths": cfg.check.paths,
            "fail_on": cfg.check.fail_on,
        },
        "bench": {
            "min_f1": cfg.bench.min_f1,
            "corpus_dir": cfg.bench.corpus_dir,
        },
    })
}

fn apply_extraction(
    into: &mut ExtractionConfig,
    from: &ExtractionConfig,
    layer: ProvenanceSource,
    sources: &mut BTreeMap<String, ProvenanceSource>,
) {
    if !from.include.is_empty() {
        into.include = from.include.clone();
        sources.insert("extraction.include".into(), layer);
    }
    if !from.exclude.is_empty() {
        into.exclude = from.exclude.clone();
        sources.insert("extraction.exclude".into(), layer);
    }
    if let Some(v) = from.max_rules_per_dep {
        into.max_rules_per_dep = Some(v);
        sources.insert("extraction.max_rules_per_dep".into(), layer);
    }
    if !from.allowed_categories.is_empty() {
        into.allowed_categories = from.allowed_categories.clone();
        sources.insert("extraction.allowed_categories".into(), layer);
    }
    if let Some(ref v) = from.min_confidence {
        into.min_confidence = Some(v.clone());
        sources.insert("extraction.min_confidence".into(), layer);
    }
    if !from.preferred_source_kinds.is_empty() {
        into.preferred_source_kinds = from.preferred_source_kinds.clone();
        sources.insert("extraction.preferred_source_kinds".into(), layer);
    }
    if let Some(v) = from.recency_window_days {
        into.recency_window_days = Some(v);
        sources.insert("extraction.recency_window_days".into(), layer);
    }
}

fn apply_resolve(
    into: &mut ResolveConfig,
    from: &ResolveConfig,
    layer: ProvenanceSource,
    sources: &mut BTreeMap<String, ProvenanceSource>,
) {
    if let Some(v) = from.cache_ttl_seconds {
        into.cache_ttl_seconds = Some(v);
        sources.insert("resolve.cache_ttl_seconds".into(), layer);
    }
    if let Some(v) = from.timeout_seconds {
        into.timeout_seconds = Some(v);
        sources.insert("resolve.timeout_seconds".into(), layer);
    }
    if let Some(v) = from.workers {
        into.workers = Some(v);
        sources.insert("resolve.workers".into(), layer);
    }
}

fn apply_check(
    into: &mut CheckConfig,
    from: &CheckConfig,
    layer: ProvenanceSource,
    sources: &mut BTreeMap<String, ProvenanceSource>,
) {
    if !from.paths.is_empty() {
        into.paths = from.paths.clone();
        sources.insert("check.paths".into(), layer);
    }
    if let Some(ref v) = from.fail_on {
        into.fail_on = Some(v.clone());
        sources.insert("check.fail_on".into(), layer);
    }
}

fn apply_bench(
    into: &mut BenchConfig,
    from: &BenchConfig,
    layer: ProvenanceSource,
    sources: &mut BTreeMap<String, ProvenanceSource>,
) {
    if let Some(v) = from.min_f1 {
        into.min_f1 = Some(v);
        sources.insert("bench.min_f1".into(), layer);
    }
    if let Some(ref v) = from.corpus_dir {
        into.corpus_dir = Some(v.clone());
        sources.insert("bench.corpus_dir".into(), layer);
    }
}

// ── Validation helpers used by callers (check/extraction) ──

impl WhetstoneConfig {
    /// Return true if `dep_name` is allowed by the extraction include/exclude
    /// filters. Include-empty means "all deps". Exclude always wins over include.
    pub fn extraction_allows(&self, dep_name: &str) -> bool {
        if self
            .extraction
            .exclude
            .iter()
            .any(|n| n.eq_ignore_ascii_case(dep_name))
        {
            return false;
        }
        if self.extraction.include.is_empty() {
            return true;
        }
        self.extraction
            .include
            .iter()
            .any(|n| n.eq_ignore_ascii_case(dep_name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn loads_empty_project_defaults() {
        let td = tempdir();
        let snap = ConfigSnapshot::load(td.path(), false, false);
        assert!(snap.effective.deny.is_empty());
        assert!(snap.sources.is_empty());
        assert!(snap.diagnostics.is_empty());
    }

    #[test]
    fn project_config_sets_keys_and_records_provenance() {
        let td = tempdir();
        let wh = td.path().join("whetstone");
        fs::create_dir_all(&wh).unwrap();
        fs::write(
            wh.join("whetstone.yaml"),
            r#"extraction:
  max_rules_per_dep: 3
  include: [fastapi]
"#,
        )
        .unwrap();
        let snap = ConfigSnapshot::load(td.path(), false, false);
        assert_eq!(snap.effective.extraction.max_rules_per_dep, Some(3));
        assert_eq!(snap.effective.extraction.include, vec!["fastapi"]);
        assert_eq!(
            snap.sources
                .get("extraction.max_rules_per_dep")
                .copied()
                .unwrap(),
            ProvenanceSource::Project
        );
    }

    #[test]
    fn personal_overrides_project() {
        let td = tempdir();
        let wh = td.path().join("whetstone");
        fs::create_dir_all(wh.join(".personal")).unwrap();
        fs::write(
            wh.join("whetstone.yaml"),
            "resolve:\n  timeout_seconds: 30\n",
        )
        .unwrap();
        fs::write(
            wh.join(".personal/config.yaml"),
            "resolve:\n  timeout_seconds: 90\n",
        )
        .unwrap();
        let snap = ConfigSnapshot::load(td.path(), false, true);
        assert_eq!(snap.effective.resolve.timeout_seconds, Some(90));
        assert_eq!(
            snap.sources
                .get("resolve.timeout_seconds")
                .copied()
                .unwrap(),
            ProvenanceSource::Personal
        );
    }

    #[test]
    fn unknown_key_produces_warning() {
        let td = tempdir();
        let wh = td.path().join("whetstone");
        fs::create_dir_all(&wh).unwrap();
        fs::write(
            wh.join("whetstone.yaml"),
            "extractoin:\n  max_rules_per_dep: 5\n",
        )
        .unwrap();
        let snap = ConfigSnapshot::load(td.path(), false, false);
        let msgs: Vec<&str> = snap
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            msgs.iter().any(|m| m.contains("extractoin")),
            "expected an unknown-key diagnostic: {msgs:?}"
        );
    }

    #[test]
    fn extraction_allows_respects_include_exclude() {
        let mut cfg = WhetstoneConfig::default();
        cfg.extraction.include = vec!["fastapi".into(), "requests".into()];
        cfg.extraction.exclude = vec!["requests".into()];
        assert!(cfg.extraction_allows("fastapi"));
        assert!(!cfg.extraction_allows("requests"));
        assert!(!cfg.extraction_allows("django"));

        let mut cfg2 = WhetstoneConfig::default();
        cfg2.extraction.exclude = vec!["django".into()];
        assert!(cfg2.extraction_allows("fastapi"));
        assert!(!cfg2.extraction_allows("django"));
    }

    #[test]
    fn global_only_key_in_project_emits_warning() {
        let td = tempdir();
        let wh = td.path().join("whetstone");
        fs::create_dir_all(&wh).unwrap();
        fs::write(wh.join("whetstone.yaml"), "default_formats: [claude.md]\n").unwrap();
        let snap = ConfigSnapshot::load(td.path(), false, false);
        assert!(
            snap.diagnostics
                .iter()
                .any(|d| d.message.contains("default_formats")),
            "expected misplaced-key warning: {:?}",
            snap.diagnostics
        );
    }

    #[test]
    fn invalid_values_emit_errors_and_fall_back_to_defaults() {
        let td = tempdir();
        let wh = td.path().join("whetstone");
        fs::create_dir_all(&wh).unwrap();
        fs::write(
            wh.join("whetstone.yaml"),
            r#"generate:
  formats: [bogus.md, agents.md]
extraction:
  max_rules_per_dep: 0
  allowed_categories: [default, made-up]
  min_confidence: low
resolve:
  timeout_seconds: 0
check:
  fail_on: maybe
bench:
  min_f1: 1.5
  corpus_dir: ""
"#,
        )
        .unwrap();

        let snap = ConfigSnapshot::load(td.path(), false, false);
        assert!(
            snap.diagnostics
                .iter()
                .any(|d| d.level == DiagnosticLevel::Error),
            "expected value-validation errors: {:?}",
            snap.diagnostics
        );
        assert_eq!(snap.effective.generate.formats, vec!["agents.md"]);
        assert_eq!(snap.effective.extraction.max_rules_per_dep, None);
        assert_eq!(
            snap.effective.extraction.allowed_categories,
            vec!["default"]
        );
        assert_eq!(snap.effective.extraction.min_confidence, None);
        assert_eq!(snap.effective.resolve.timeout_seconds, None);
        assert_eq!(snap.effective.check.fail_on, None);
        assert_eq!(snap.effective.bench.min_f1, None);
        assert_eq!(snap.effective.bench.corpus_dir, None);
    }
}
