# CLI vNext Migration Guide

> **Status:** active migration guide · 2026-05-06
> **Tracking:** whetstone-ng9g.1.3

## Who this is for

- existing Whetstone users
- repos with shell scripts or CI calling older verbs
- teams adopting the new canonical shareability model

## What changed

The vNext CLI keeps the same product shape, but standardizes the public surface
around a smaller set of canonical verbs:

- `wh init`
- `wh reinit`
- `wh status`
- `wh scan`
- `wh debt`
- `wh actions all|context|test|lint`
- `wh rules ...`
- `wh sources ...`
- `wh config show|validate`

Bare `wh` remains the human/TUI entrypoint.

## Canonical shareability story

The default shareable artifact is:

```text
whetstone/whetstone.yaml
```

That file is the copy/paste entrypoint between repos. Optional `extends:` pack
refs compose underneath it.

## Old → new command mapping

| Old | Canonical now | Status |
|-----|---------------|--------|
| `wh check` | `wh scan` | compatibility alias remains |
| `wh rule ...` | `wh rules ...` | compatibility alias remains |
| `wh source ...` | `wh sources ...` | compatibility alias remains |
| `wh source fetch` | `wh sources verify` | compatibility alias remains |
| `wh approve ...` | `wh rules approve ...` | top-level compatibility entrypoint remains |
| `wh context` | `wh actions context` | hidden compatibility entrypoint remains |
| `wh tests` | `wh actions test` | hidden compatibility entrypoint remains |
| `wh lint` | `wh actions lint` | hidden compatibility entrypoint remains |
| repo-root `whetstone.yaml` | `whetstone/whetstone.yaml` | compatibility fallback remains |

## Adoption steps for an existing repo

1. **Move to canonical config placement**
   - Prefer `whetstone/whetstone.yaml`
   - Keep repo-root `whetstone.yaml` only as a temporary fallback

2. **Normalize command calls**
   - prefer `wh scan`
   - prefer `wh rules ...`
   - prefer `wh sources ...`
   - prefer `wh actions ...`

3. **Inspect the active config stack**

```bash
wh config show
wh config validate
```

4. **Regenerate outputs**

```bash
wh actions all
```

5. **Verify enforcement**

```bash
wh scan src/
wh status
```

## Automation guidance

- JSON contracts remain the machine interface. Keep using `--json`.
- Compatibility aliases still work today, but new automation should use the
  canonical verbs above.
- Prefer `wh rules worklist` after `wh init` when an agent needs to decide what
  to extract next.

## Deprecation stance

Compatibility aliases are still callable in the current build, but docs and
help now teach only the canonical surface. New user education, scripts, and CI
examples should migrate now rather than waiting for alias removal.

## Explicit non-goals in this rollout

- no MCP requirement
- no registry/publishing requirement
- no forceful removal of compatibility aliases in the same rollout

## Release rollout checklist

- update first-party docs and help surfaces
- update CLI/TUI tests that lock canonical verbs
- note config-pack support and formatter-backed overlays in release notes
- call out `whetstone/whetstone.yaml` as the canonical config path
- keep MCP explicitly deferred
