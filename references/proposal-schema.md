# Whetstone Proposal Bundle Schema

> The machine-readable contract an extraction agent emits, and that
> `wh propose import` turns into candidate rule YAML files. Structured
> proposals replace hand-authored YAML so the provenance, lifecycle
> status, and timestamp columns stay consistent across every run.

A **proposal bundle** is a single JSON or YAML document produced per
dependency per extraction run. It has one version field, one
`dependency` block, and one `proposals` array (max 5 by default — see
`extraction.max_rules_per_dep`).

## Shape

```yaml
version: 1
proposed_by: whetstone-extraction   # optional; CLI --actor overrides
proposed_at: "2026-04-15T12:00:00Z" # optional; defaults to now
dependency:
  name: reqwest
  language: rust
  version: "0.12.0"
  source_url: https://docs.rs/reqwest
  content_hash: "sha256:abcdef..."
  registry: crates_io              # pypi | npm | crates_io | manual
  resolved_at: "2026-04-15T11:45:00Z"
proposals:
  - id: reqwest.set-timeout
    severity: must                 # must | should | may
    confidence: high               # high | medium
    category: default              # migration | default | convention | breaking-change | semantic
    description: "Clients MUST set an explicit timeout."
    source_url: https://docs.rs/reqwest/latest/reqwest/#timeouts
    source_quote: "Client::new() has no timeout configured by default."
    source_kind: official_docs     # optional, free-text
    risk: "Unbounded network call under load."
    linter_gap: "Clippy has no rule for this."
    signals:
      - id: no-timeout
        strategy: pattern          # ast | pattern | lint_proxy | ai
        description: "Client construction without .timeout()"
        weight: required           # required | strong | moderate
        match: 'Client::new\(\)'
    golden_examples:
      - code: "let c = Client::new();"
        verdict: fail
        reason: no timeout
      - code: "let c = Client::builder().timeout(Duration::from_secs(5)).build()?;"
        verdict: pass
        reason: explicit timeout
```

## Auto-populated provenance

On import, Whetstone writes these fields automatically so the agent never
hand-authors them:

| Rule YAML field | Source |
|---|---|
| `status: candidate` | always set |
| `approved: false` | always set |
| `proposed_at` | bundle.`proposed_at`  → else now |
| `proposed_by` | CLI `--actor` → bundle.`proposed_by` → `whetstone-proposal-import` |

## Hard validations

The importer rejects bundles that violate any of the following:

1. **Version** must be `1`.
2. **Dependency** allowed by `extraction.include` / `extraction.exclude`.
3. **Bundle size** — `len(proposals) <= extraction.max_rules_per_dep` (default 5).
4. **Per-dep quota** — `existing_live − overwritten + len(proposals) <= extraction.max_rules_per_dep`,
   where `existing_live` counts approved + candidate rules already in the dep's file
   and `overwritten` subtracts candidates the bundle is replacing via `--overwrite-candidates`.
   Denied and deprecated rules are excluded (they are audit history, not live rules).
5. **Category** in `extraction.allowed_categories` (default: all five).
6. **Confidence** at or above `extraction.min_confidence` (default: accept both).
7. **Signals** — every proposal has at least one `ast` or `pattern` signal.
8. **Golden examples** — every proposal has 3–5 examples with at least one `pass`
   verdict and at least one `fail` verdict.
9. **Uniqueness** — no duplicate ids within the bundle.
10. **No clobber** — ids that match an already-`approved` or `denied` rule
    are refused; deprecate/supersede through `wh apply` instead.
11. **Candidate replacement** — ids that match an existing *candidate* are
    refused unless `--overwrite-candidates` is passed.

## Commands

```bash
# See the schema without reading this file
wh propose schema

# Preview what a bundle would change, without writing
wh propose diff bundle.yaml --project-dir .

# Import (writes whetstone/rules/{language}/{dep}.yaml with status=candidate)
wh propose import bundle.yaml --project-dir . \
  [--dry-run] [--actor <name>] [--overwrite-candidates]
```

After import:

```bash
wh review --status=candidate
wh apply <rule-id> --approve
```

## Why this exists

Before 3D.1, extraction ended in the agent writing YAML by hand. That
made the agent responsible for correct `status`, `approved`,
`proposed_at`, `proposed_by`, and file placement — every one a source
of drift. The bundle schema makes those fields deterministic and forces
every rule through the same two-step review loop (`import → apply`).
