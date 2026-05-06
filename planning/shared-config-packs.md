# Shared Config Packs vNext

> **Status:** design draft · 2026-05-06
> **Tracking:** whetstone-ng9g.4, whetstone-ng9g.4.1, whetstone-ng9g.4.2

## Goal

Make Whetstone configuration and rules easy to share across personal, project,
team, and org scopes using YAML packs with explicit provenance and precedence.

## Canonical shareability story

The **default shareable artifact** should be a single committed project file:

```text
whetstone/whetstone.yaml
```

That file is the copy/paste entrypoint. A team should be able to hand someone a
`whetstone/whetstone.yaml`, drop it into another repo, run Whetstone, and get
the same source-driven taste model.

Packs exist to make that file reusable and composable — not to replace it as the
primary UX. In other words:

- **copy/paste first**
- **extends/import second**
- **registry/publishing later**

The repo-root `whetstone.yaml` path may remain as a compatibility fallback while
the CLI settles, but docs and generation should treat `whetstone/whetstone.yaml`
as canonical.

## Scope model

Precedence is:

```text
org < team < project < personal
```

### Scope defaults

| Scope | Default location | Committed by default | Notes |
|---|---|---:|---|
| org | remote or checked-in pack reference | yes | Broadest standards |
| team | remote or checked-in pack reference | yes | Team/domain standards |
| project | `whetstone/whetstone.yaml` | yes | Canonical copy/paste entrypoint |
| personal | `whetstone/.personal/` | no | Local by default; commit is opt-in |

## Important product rule

Personal rules are **not committed by default**.

If a user wants to share a personal pack, they must opt in explicitly by either:

1. moving it to a committed path, or
2. marking the pack reference as `commit: true`.

## Repository config shape

The canonical project file is both:

1. the repo's **copy/pasteable shareability entrypoint**, and
2. the place where optional shared packs are imported.

Minimal starter shape:

```yaml
version: 1

sources:
  custom:
    - url: https://internal.acme.dev/style
      name: acme-style
      language: python
      source_kind: team_guide

extraction:
  min_confidence: high

generate:
  formats: [agents.md, claude.md]
```

Extended shape with imported packs:

```yaml
version: 1

sources:
  custom:
    - url: https://internal.acme.dev/style
      name: acme-style
      language: python
      source_kind: team_guide

extraction:
  min_confidence: high

extends:
  - scope: org
    ref: github://acme/whetstone-config//packs/org/base.yaml@v1

  - scope: team
    ref: github://acme/whetstone-config//packs/team/payments.yaml@main

  - scope: project
    ref: path:./whetstone/packs/project.yaml

  - scope: personal
    ref: path:./whetstone/.personal/packs/angus.yaml
    commit: false
```

## Pack file shape

```yaml
apiVersion: whetstone/v1alpha1
kind: RulePack
language: python   # required when `rules:` is non-empty

metadata:
  name: acme.payments
  version: 1.2.0
  scope: team
  owner: payments-platform

rules:
  - id: fastapi.async-routes
    severity: must
    confidence: high
    category: convention
    description: Route handlers MUST use async def.
    source_url: https://fastapi.tiangolo.com/async/
    approved: true
    status: approved
    signals:
      - id: is-sync-function
        strategy: ast
        description: Route decorator on non-async function
        weight: required

sources:
  custom:
    - url: https://internal.acme.dev/python-style
      name: acme-python-style
      language: python
      source_kind: team_guide

deny:
  - legacy.rule-id

overrides:
  - id: fastapi.async-routes
    severity: should
```

## Merge contract

1. Start from the broadest imported pack.
2. Apply later packs in declared `extends` order.
3. Apply inline project config from `whetstone/whetstone.yaml`.
4. Apply local project rules.
5. Apply personal rules last.
6. `deny` removes a rule from broader scopes.
7. `overrides` may change severity/confidence/description/source metadata
   without redefining the full rule.
8. Every effective rule should retain provenance:
   - source pack
   - canonical project file
   - scope
   - original rule id
   - override chain

## Resolution contract

Supported pack refs should eventually include:

- `path:./relative/file.yaml`
- `file:///absolute/file.yaml`
- `github://owner/repo//path/to/file.yaml@ref`

Each resolved pack should be cached with:

- resolved ref
- content hash
- fetched timestamp
- scope
- pack metadata

## CLI implications

The CLI now has a first-class inspection surface:

```bash
wh config show
wh config validate
```

These commands should explain:

- whether `whetstone/whetstone.yaml` exists and is canonical
- active scopes and packs
- effective precedence
- ignored or shadowed fields
- invalid refs
- conflicting rule ids
- whether a personal pack is local-only or explicitly committable

## Non-goals for the first implementation tranche

- full registry/publishing ecosystem
- signatures/trust model
- automatic org/team pack publishing
- background update daemons

## Near-term product rule

If we have to choose between a smarter pack system and a simpler copy/paste
story, prefer the simpler copy/paste story first. Shared packs should make the
canonical `whetstone/whetstone.yaml` more powerful, but never harder to adopt.
