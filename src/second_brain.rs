use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use glob::Pattern;
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::config::{ResolvedSourceInput, VaultSource};
use crate::detect::walk::SKIP_DIRS;
use crate::state::{atomic_write, load_json};

const GRAPH_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct BuildOutput {
    pub inputs: Vec<ResolvedSourceInput>,
    pub graph: Value,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Authority {
    Draft,
    Synthesized,
    Reviewed,
    Canonical,
}

impl Authority {
    fn from_str(input: &str) -> Option<Self> {
        match input.trim().to_ascii_lowercase().as_str() {
            "draft" => Some(Self::Draft),
            "synthesized" => Some(Self::Synthesized),
            "reviewed" => Some(Self::Reviewed),
            "canonical" => Some(Self::Canonical),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Synthesized => "synthesized",
            Self::Reviewed => "reviewed",
            Self::Canonical => "canonical",
        }
    }

    fn score(self) -> f64 {
        match self {
            Self::Draft => 0.25,
            Self::Synthesized => 0.5,
            Self::Reviewed => 0.8,
            Self::Canonical => 1.0,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct FrontmatterRoot {
    #[serde(default)]
    whetstone: FrontmatterWhetstone,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
struct FrontmatterWhetstone {
    #[serde(default)]
    authority: Option<String>,
    #[serde(default)]
    languages: Vec<String>,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default)]
    upstream: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Debug, Clone)]
struct PageRecord {
    id: String,
    vault_id: String,
    vault_path: String,
    relative_path: String,
    title: String,
    authority: Authority,
    languages: Vec<String>,
    deps: Vec<String>,
    tags: Vec<String>,
    aliases: Vec<String>,
    upstream: Vec<String>,
    links: Vec<String>,
    related_pages: Vec<String>,
    content: String,
    content_hash: String,
    source_kind: String,
}

pub fn build(project_dir: &Path, vaults: &[VaultSource], trigger: &str) -> BuildOutput {
    let mut warnings = Vec::new();
    let mut pages = Vec::new();
    let mut seen_vaults = BTreeMap::new();

    for vault in vaults {
        if let Some(existing_path) = seen_vaults.insert(vault.id.clone(), vault.path.clone()) {
            warnings.push(format!(
                "duplicate second-brain vault id `{}` configured for `{existing_path}` and `{}`; indexing both may duplicate pages",
                vault.id, vault.path
            ));
        }
        match collect_vault_pages(project_dir, vault) {
            Ok(mut collected) => pages.append(&mut collected),
            Err(err) => warnings.push(format!("vault `{}`: {err}", vault.id)),
        }
    }

    let alias_index = build_alias_index(&pages, &mut warnings);
    for page in &mut pages {
        let mut related_pages = Vec::new();
        for link in &page.links {
            match alias_index.get(&normalize_key(link)) {
                Some(ids) if ids.len() == 1 => {
                    if ids[0] != page.id {
                        related_pages.push(ids[0].clone());
                    }
                }
                Some(ids) if ids.len() > 1 => warnings.push(format!(
                    "ambiguous wikilink `[[{}]]` in `{}` resolves to {} pages",
                    link,
                    page.relative_path,
                    ids.len()
                )),
                _ => {}
            }
        }
        page.related_pages = related_pages;
        page.related_pages.sort();
        page.related_pages.dedup();
    }

    let graph = build_graph(project_dir, trigger, &pages, &warnings);
    let inputs = pages.iter().map(page_to_input).collect::<Vec<_>>();

    BuildOutput {
        inputs,
        graph,
        warnings,
    }
}

pub fn write_graph(project_dir: &Path, graph: &Value) {
    atomic_write(&graph_path(project_dir), graph);
}

pub fn graph_path(project_dir: &Path) -> PathBuf {
    project_dir
        .join("whetstone")
        .join(".state")
        .join("knowledge-graph.json")
}

fn collect_vault_pages(project_dir: &Path, vault: &VaultSource) -> anyhow::Result<Vec<PageRecord>> {
    let root = project_dir.join(&vault.path);
    if !root.exists() {
        return Err(anyhow::anyhow!("path `{}` does not exist", vault.path));
    }
    if !root.is_dir() {
        return Err(anyhow::anyhow!("path `{}` is not a directory", vault.path));
    }

    let include = compile_patterns(if vault.include.is_empty() {
        vec!["**/*.md".to_string()]
    } else {
        vault.include.clone()
    })?;
    let exclude = compile_patterns(vault.exclude.clone())?;
    let max_pages = vault.max_pages.unwrap_or(500);
    let default_authority = vault
        .authority
        .as_deref()
        .and_then(Authority::from_str)
        .unwrap_or(Authority::Reviewed);
    let default_language = vault
        .language
        .as_deref()
        .and_then(crate::types::canonical_language);
    let default_source_kind = vault.source_kind.as_deref().unwrap_or("second_brain");

    let skip: HashSet<&str> = SKIP_DIRS
        .iter()
        .filter(|s| !s.contains('/'))
        .copied()
        .collect();
    let skip_multi: Vec<&str> = SKIP_DIRS
        .iter()
        .filter(|s| s.contains('/'))
        .copied()
        .collect();
    let mut pages = Vec::new();

    for entry in WalkDir::new(&root)
        .into_iter()
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            let name = entry.file_name().to_string_lossy();
            !skip.contains(name.as_ref())
        })
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = match entry.path().strip_prefix(&root) {
            Ok(path) => path,
            Err(_) => continue,
        };
        let rel = relative.to_string_lossy().replace('\\', "/");
        if skip_multi
            .iter()
            .any(|pattern| rel == *pattern || rel.starts_with(&format!("{pattern}/")))
        {
            continue;
        }
        if !matches_patterns(&include, &rel) || matches_patterns(&exclude, &rel) {
            continue;
        }
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        if pages.len() >= max_pages {
            break;
        }

        let text = match std::fs::read_to_string(entry.path()) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let (frontmatter, content) = match split_frontmatter(&text) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        let title = extract_title(&content, entry.path());
        let authority = frontmatter
            .whetstone
            .authority
            .as_deref()
            .and_then(Authority::from_str)
            .unwrap_or(default_authority);
        let mut languages = frontmatter
            .whetstone
            .languages
            .iter()
            .filter_map(|language| crate::types::canonical_language(language).map(String::from))
            .collect::<Vec<_>>();
        if languages.is_empty() {
            if let Some(language) = default_language {
                languages.push(language.to_string());
            }
        }
        languages.sort();
        languages.dedup();

        let mut tags = frontmatter.tags;
        tags.extend(frontmatter.whetstone.tags);
        tags.sort();
        tags.dedup();

        let mut deps = frontmatter.whetstone.deps;
        deps.sort();
        deps.dedup();

        let mut upstream = frontmatter.whetstone.upstream;
        upstream.extend(extract_upstream_links(&content));
        upstream.sort();
        upstream.dedup();

        let mut aliases = frontmatter.whetstone.aliases;
        aliases.push(title.clone());
        aliases.sort();
        aliases.dedup();

        let links = extract_wikilinks(&content);
        let page_id = format!("vault:{}:{}", slug(&vault.id), slug(&rel));
        let repo_relative = entry
            .path()
            .strip_prefix(project_dir)
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| entry.path().to_string_lossy().replace('\\', "/"));

        let content_hash = page_payload_hash(
            authority, &languages, &deps, &tags, &aliases, &upstream, &content,
        );

        pages.push(PageRecord {
            id: page_id,
            vault_id: vault.id.clone(),
            vault_path: vault.path.clone(),
            relative_path: repo_relative,
            title,
            authority,
            languages,
            deps,
            tags,
            aliases,
            upstream,
            links,
            related_pages: Vec::new(),
            content_hash,
            content,
            source_kind: default_source_kind.to_string(),
        });
    }

    Ok(pages)
}

fn compile_patterns(patterns: Vec<String>) -> anyhow::Result<Vec<Pattern>> {
    patterns
        .into_iter()
        .map(|pattern| Pattern::new(&pattern).map_err(anyhow::Error::from))
        .collect()
}

fn matches_patterns(patterns: &[Pattern], path: &str) -> bool {
    patterns.iter().any(|pattern| pattern.matches(path))
}

fn split_frontmatter(text: &str) -> anyhow::Result<(FrontmatterRoot, String)> {
    let normalized = text.replace("\r\n", "\n");
    if let Some(rest) = normalized.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let frontmatter_text = &rest[..end];
            let content = rest[end + 5..].to_string();
            let frontmatter = serde_yaml::from_str::<FrontmatterRoot>(frontmatter_text)?;
            return Ok((frontmatter, content));
        }
    }
    Ok((FrontmatterRoot::default(), normalized))
}

fn extract_title(content: &str, path: &Path) -> String {
    content
        .lines()
        .find_map(|line| {
            line.strip_prefix("# ")
                .map(|title| title.trim().to_string())
        })
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("untitled")
                .replace(['_', '-'], " ")
        })
}

fn extract_wikilinks(content: &str) -> Vec<String> {
    let re = Regex::new(r"\[\[([^\]|#]+)(?:#[^\]|]+)?(?:\|[^\]]+)?\]\]").unwrap();
    let mut out = re
        .captures_iter(content)
        .filter_map(|captures| {
            captures
                .get(1)
                .map(|match_| match_.as_str().trim().to_string())
        })
        .filter(|link| !link.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn extract_upstream_links(content: &str) -> Vec<String> {
    let re = Regex::new(r"https?://[^\s)]+").unwrap();
    let mut out = re
        .find_iter(content)
        .map(|match_| {
            match_
                .as_str()
                .trim_end_matches([')', '.', ','])
                .to_string()
        })
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

fn build_alias_index(
    pages: &[PageRecord],
    warnings: &mut Vec<String>,
) -> HashMap<String, Vec<String>> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();
    for page in pages {
        index
            .entry(normalize_key(&page.title))
            .or_default()
            .push(page.id.clone());
        index
            .entry(normalize_key(&page.relative_path))
            .or_default()
            .push(page.id.clone());
        for alias in &page.aliases {
            index
                .entry(normalize_key(alias))
                .or_default()
                .push(page.id.clone());
        }
    }
    for (alias, ids) in &mut index {
        ids.sort();
        ids.dedup();
        if ids.len() > 1 {
            warnings.push(format!(
                "alias/key `{alias}` maps to multiple second-brain pages: {}",
                ids.join(", ")
            ));
        }
    }
    index
}

fn page_payload_hash(
    authority: Authority,
    languages: &[String],
    deps: &[String],
    tags: &[String],
    aliases: &[String],
    upstream: &[String],
    content: &str,
) -> String {
    crate::resolve::content_hash(
        &serde_json::to_string(&json!({
            "authority": authority.as_str(),
            "languages": languages,
            "deps": deps,
            "tags": tags,
            "aliases": aliases,
            "upstream": upstream,
            "content": content,
        }))
        .unwrap_or_default(),
    )
}

fn normalize_key(input: &str) -> String {
    input.trim().to_ascii_lowercase().replace(['_', '-'], " ")
}

fn page_to_input(page: &PageRecord) -> ResolvedSourceInput {
    let primary_language = page.languages.first().cloned();
    let mut metadata = BTreeMap::new();
    metadata.insert("vault_id".into(), json!(page.vault_id));
    metadata.insert("vault_path".into(), json!(page.vault_path));
    metadata.insert("page_id".into(), json!(page.id));
    metadata.insert("page_title".into(), json!(page.title));
    metadata.insert("authority".into(), json!(page.authority.as_str()));
    metadata.insert("authority_score".into(), json!(page.authority.score()));
    metadata.insert("tags".into(), json!(page.tags));
    metadata.insert("aliases".into(), json!(page.aliases));
    metadata.insert("dep_names".into(), json!(page.deps));
    metadata.insert("upstream_urls".into(), json!(page.upstream));
    metadata.insert("wikilinks".into(), json!(page.links));
    metadata.insert("related_pages".into(), json!(page.related_pages));

    ResolvedSourceInput {
        url: page.relative_path.clone(),
        name: Some(page.title.clone()),
        language: primary_language,
        source_kind: Some(page.source_kind.clone()),
        source_origin: "second_brain_page",
        source_ref_id: page.id.clone(),
        pack_id: Some(page.vault_id.clone()),
        pack_name: Some(page.vault_id.clone()),
        member_id: Some(page.id.clone()),
        content_override: Some(page.content.clone()),
        source_type_override: Some("second_brain_page".into()),
        metadata,
    }
}

fn build_graph(
    project_dir: &Path,
    trigger: &str,
    pages: &[PageRecord],
    warnings: &[String],
) -> Value {
    let previous = load_json(&graph_path(project_dir));
    let previous_hashes = previous
        .get("pages")
        .and_then(|value| value.as_array())
        .map(|pages| {
            pages
                .iter()
                .filter_map(|page| {
                    Some((
                        page.get("id")?.as_str()?.to_string(),
                        page.get("content_hash")?.as_str()?.to_string(),
                    ))
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();

    let current_hashes = pages
        .iter()
        .map(|page| (page.id.clone(), page.content_hash.clone()))
        .collect::<HashMap<_, _>>();

    let added_pages = current_hashes
        .keys()
        .filter(|id| !previous_hashes.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();
    let changed_pages = current_hashes
        .iter()
        .filter_map(|(id, hash)| {
            previous_hashes
                .get(id)
                .filter(|previous| *previous != hash)
                .map(|_| id.clone())
        })
        .collect::<Vec<_>>();
    let removed_pages = previous_hashes
        .keys()
        .filter(|id| !current_hashes.contains_key(*id))
        .cloned()
        .collect::<Vec<_>>();

    let pages_json = pages
        .iter()
        .map(|page| {
            json!({
                "id": page.id,
                "vault_id": page.vault_id,
                "path": page.relative_path,
                "title": page.title,
                "authority": page.authority.as_str(),
                "authority_score": page.authority.score(),
                "languages": page.languages,
                "deps": page.deps,
                "tags": page.tags,
                "aliases": page.aliases,
                "upstream_urls": page.upstream,
                "wikilinks": page.links,
                "related_pages": page.related_pages,
                "source_kind": page.source_kind,
                "content_hash": page.content_hash,
            })
        })
        .collect::<Vec<_>>();

    let edges_json = pages
        .iter()
        .flat_map(|page| {
            page.related_pages.iter().map(|related| {
                json!({
                    "kind": "wikilink",
                    "from": page.id,
                    "to": related,
                })
            })
        })
        .collect::<Vec<_>>();

    json!({
        "version": GRAPH_VERSION,
        "generated_at": Utc::now().to_rfc3339(),
        "project_dir": project_dir.display().to_string(),
        "build_trigger": trigger,
        "page_count": pages.len(),
        "pages": pages_json,
        "edges": edges_json,
        "diff": {
            "added_pages": added_pages,
            "changed_pages": changed_pages,
            "removed_pages": removed_pages,
        },
        "warnings": warnings,
    })
}

fn slug(input: &str) -> String {
    input
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn frontmatter_and_wikilinks_parse() {
        let text = "---\nwhetstone:\n  authority: canonical\n  languages: [javascript]\n  deps: [react]\n  upstream:\n    - https://react.dev/\n  tags: [ui]\n  aliases: [React Patterns]\n---\n# React Patterns\nUse [[Event Handling]].\n";
        let (frontmatter, content) = split_frontmatter(text).unwrap();
        assert_eq!(
            frontmatter.whetstone.authority.as_deref(),
            Some("canonical")
        );
        assert_eq!(frontmatter.whetstone.languages, vec!["javascript"]);
        assert!(content.contains("React Patterns"));
        assert_eq!(extract_wikilinks(&content), vec!["Event Handling"]);
    }

    #[test]
    fn build_generates_page_inputs_and_graph() {
        let td = tempdir().unwrap();
        std::fs::create_dir_all(td.path().join("docs/brain")).unwrap();
        std::fs::write(
            td.path().join("docs/brain/react.md"),
            "---\nwhetstone:\n  authority: reviewed\n  languages: [javascript]\n  deps: [react]\n---\n# React Notes\nSee [[HTML Rules]].\n",
        )
        .unwrap();
        std::fs::write(
            td.path().join("docs/brain/html.md"),
            "# HTML Rules\nPrefer semantic tags.\n",
        )
        .unwrap();

        let output = build(
            td.path(),
            &[VaultSource {
                id: "team-brain".into(),
                path: "docs/brain".into(),
                include: vec!["**/*.md".into()],
                exclude: Vec::new(),
                language: Some("html".into()),
                source_kind: Some("second_brain".into()),
                authority: Some("reviewed".into()),
                max_pages: None,
            }],
            "init",
        );
        assert_eq!(output.inputs.len(), 2);
        assert_eq!(output.graph["page_count"], 2);
        assert_eq!(output.inputs[0].source_origin, "second_brain_page");
    }
}
