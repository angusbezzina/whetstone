# Platform + Registry Design

> **Status:** active design · 2026-05-06
> **Tracking:** whetstone-s2a, whetstone-s60, whetstone-hcr, whetstone-gtd, whetstone-y4x, whetstone-33v

## Goal

Define the post-local expansion path for Whetstone once the core local product is
stable: shared registry, publishing, signal promotion, rule evolution, and an
optional hosted/app model.

This document is intentionally **platform design**, not a promise that all of it
ships in the local CLI first.

## Product stance

Whetstone remains **local-first and approval-first**.

- The canonical project entrypoint is still `whetstone/whetstone.yaml`.
- Teams can still work entirely without any registry or hosted account.
- Registry/platform features should accelerate discovery and reuse, not replace
  local approval.

## Workstream 4.1 — Rule registry design + storage API

The registry stores **published packs**, not arbitrary repo state.

### Registry unit

The primary publishable object is a versioned **RulePack**:

- publisher namespace
- pack name
- semver version
- scope metadata
- source metadata
- rules
- overrides/denies
- scorecard metadata

### Required indexes

The registry must index by:

- publisher/name/version
- dependency/source name
- language
- category
- confidence
- source kind
- pack popularity / rank

### Read model

Users should be able to:

- fetch a specific pack version
- fetch the latest version matching a semver range
- search packs by dependency/language/use case
- see ranking and provenance before adopting a pack

### Trust model

Registry results are **suggestions**. Local Whetstone still decides whether to:

- import a pack
- extract fresh local rules anyway
- approve the resulting ruleset

## Workstream 4.2 — Publishing rulesets

Publishing should feel like sharing a reusable Whetstone taste pack, not like
syncing raw repo state.

### Publishable input

Users publish a cleaned, versioned pack derived from:

- trusted sources
- approved rules
- pack metadata
- optional overrides / deny list

### Publisher model

- personal namespace: `@user/name`
- org/team namespace: `@org/name`
- immutable versions once published
- optional `latest` tag / channels later

### Adoption model

Projects should eventually be able to reference published packs from
`whetstone/whetstone.yaml` via a registry ref, for example:

```yaml
extends:
  - scope: org
    ref: registry://acme/python-base@^1
```

Local approval and local overrides still apply after import.

## Workstream 4.3 — Signal promotion (AI → deterministic)

Signal promotion exists to reduce reliance on expensive or subjective judgment.

### Promotion loop

1. Whetstone records repeated judgment outcomes for a rule family.
2. It clusters recurring fail/pass patterns.
3. It proposes deterministic signal candidates.
4. The user reviews and approves the promoted signal.
5. The rule moves more of its enforcement into `ast`, `pattern`, or
   `lint_proxy`.

### Safety rule

Promotion is always **proposal-based**, never automatic mutation of approved
rules.

### Output shape

Promotions should surface as candidate patches or candidate bundles rather than
in-place rewrites.

## Workstream 4.4 — Rule evolution + violation tracking

The platform should help answer:

- which rules are violated most often?
- which rules are ignored by agents repeatedly?
- which rules need clearer descriptions or examples?

### Minimum tracked dimensions

- rule id
- dependency/source
- severity
- violation frequency over time
- clean/adoption trend
- agent/tool origin when available
- rule revision history

### Evolution surfaces

Future evolution flows may include:

- `wh evolve` for local inspection
- richer `wh status` / `wh report` trends
- rule scorecards in the registry

### Constraint

Evolution suggestions should improve clarity and determinism, not create churn
for stable rules that are performing well.

## Workstream 4.5 — Hosted service / GitHub App

The hosted/app model is optional and downstream of the registry.

### Responsibilities

- monitor dependency changes
- run freshness checks
- open recommendations/issues/PR comments
- reuse registry packs where appropriate
- request extraction only when local/shared reuse is insufficient

### Security / trust stance

- public repos can use a low-friction hosted path
- private repos require explicit permission and billing model
- the app suggests and automates review loops, but local project owners still
  retain approval control over rule changes

## Rollout order

Recommended order:

1. registry/storage design
2. pack publishing model
3. violation/evolution data model
4. signal promotion workflow
5. optional hosted/app layer

This keeps the platform grounded in reusable artifacts before adding hosted
automation.

## Non-goals

- forcing every user onto hosted infrastructure
- replacing local extraction/approval
- sharing private repo contents by default
- auto-applying rule changes without explicit review

## Deliverables satisfied by this design pass

This design pass locks:

- the registry object model
- publishing/adoption direction
- signal-promotion safety model
- rule-evolution tracking direction
- hosted/app architecture boundaries

Implementation can now proceed later without re-deciding the overall product
shape.
