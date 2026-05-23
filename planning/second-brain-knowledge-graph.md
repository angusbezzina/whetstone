# Trusted Second-Brain Knowledge Graph Sources

> **Status:** active implementation design · 2026-05-21
> **Tracking:** whetstone-456, whetstone-456.1, whetstone-456.2, whetstone-456.3, whetstone-456.4, whetstone-456.5, whetstone-456.6
> **Related:** `planning/language-packs-validator-adapters.md`, `references/handoff-schema.md`, `references/rule-schema.yaml`

## Goal

Allow Whetstone to consume a local markdown-based second brain / LLM wiki as a
trusted knowledge source without weakening provenance, reviewability, or source
authority discipline.

## Product stance

- local-first, repo-relative vaults first
- snapshot/index driven, not live RAG
- internal knowledge is useful, but still graded by authority
- upstream docs remain preferred when stronger external backing exists

## Config shape

Vaults live under:

```yaml
sources:
  vaults:
    - id: team-brain
      path: docs/brain
      include: ["**/*.md"]
      exclude: ["**/templates/**"]
      language: any
      source_kind: second_brain
      authority: reviewed
      max_pages: 200
```

## Page contract

Markdown pages may carry optional frontmatter:

```yaml
---
whetstone:
  authority: canonical
  languages: [javascript, html]
  deps: [react]
  upstream:
    - https://react.dev/
  tags: [frontend, ui]
  aliases: [React Patterns]
---
```

Wikilinks like `[[Event Handling]]` are indexed as graph relationships.

## Authority model

Supported levels:

- `draft`
- `synthesized`
- `reviewed`
- `canonical`

### Preference rules

1. `canonical` > `reviewed` > `synthesized` > `draft`
2. pages that also carry upstream links rank higher than isolated notes
3. dependency-specific rules should still prefer explicit upstream citations
   when available
4. internal pages can supplement upstream docs and preserve local rationale

## Runtime model

Second-brain pages are expanded into resolved source inputs with:

- stable `source_ref` ids
- `source_origin: second_brain_page`
- `source_type: second_brain_page`
- authority, tags, deps, upstream URLs, and related-page metadata

## State artifact

`whetstone/.state/knowledge-graph.json` stores:

- page nodes
- wikilink edges
- page hashes
- related-page relationships
- incremental diff (`added_pages`, `changed_pages`, `removed_pages`)

## Rule provenance

Graph-derived rules may carry optional provenance:

- `source_page_id`
- `source_page_path`
- `source_authority`
- `source_line_start` / `source_line_end`
- `upstream_urls`

## Initial rollout

1. local markdown vault ingestion
2. graph artifact + drift
3. extraction/worklist integration
4. rule provenance surfaces
5. docs and UX polish

## Non-goals

- live remote wiki crawling
- opaque retrieval-only rule derivation
- replacing upstream docs with internal notes
- hidden confidence calculations with no provenance trail
