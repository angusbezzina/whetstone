# Command Taxonomy vNext

> **Status:** draft in implementation · 2026-04-28
> **Tracking:** whetstone-ng9g, whetstone-ng9g.2, whetstone-ng9g.5

## Confirmed decisions

- `wh status` stays top-level.
- `wh reinit` stays separate from `wh init`.
- Personal rules remain local by default; committing them is opt-in.
- Bare `wh` remains the interactive human/TUI entrypoint.
- The explicit `wh tui` command is removed.

## Canonical visible taxonomy

### Core workflow

- `wh init`
- `wh reinit`
- `wh status`
- `wh scan`
- `wh debt`
- `wh actions all`

### Management groups

- `wh rules list | show | query | add | edit | remove | approve | worklist`
- `wh sources list | add | edit | remove | verify`

### Maintenance / advanced

- `wh extract`
- `wh approve`
- `wh validate`
- `wh update`
- `wh status --report [--pr-comment]`

## UX intent

### `wh init`

`wh init` is the onboarding/orchestration verb. It should detect dependencies,
resolve docs and subscribed sources, surface extraction-ready work, and point
users at `wh sources ...` when coverage is weak.

### `wh scan`

`wh scan` is the canonical enforcement verb: “scan this repo against the active
ruleset and tell me what is wrong right now.”

### `wh status`

`wh status` is the system/repo health verb: freshness, coverage, adherence,
drift, and reporting.

### `wh actions`

`wh actions` becomes an explicit subcommand family:

- `wh actions all`
- `wh actions context`
- `wh actions lint`
- `wh actions test`

### `wh rules`

`wh rules` becomes the single obvious home for rule management. `wh approve`
and `wh extract` may remain for compatibility/advanced workflow, but rule review
and mutation should be discoverable under `wh rules` first.

### `wh sources`

`wh sources` becomes the single obvious home for custom source management.
Users should not need to know about `set-sources`.

## Compatibility mapping

| Old form | Preferred form |
|---|---|
| `wh check` | `wh scan` |
| `wh rule ...` | `wh rules ...` |
| `wh source ...` | `wh sources ...` |
| `wh source fetch` | `wh sources verify` |
| `wh context` | `wh actions context` |
| `wh tests` | `wh actions test` |
| `wh lint` | `wh actions lint` |
| `wh actions` | `wh actions all` |
| `wh tui` | bare `wh` |

## Migration rules

1. Keep compatibility aliases where cheap and unambiguous.
2. Prefer canonical names in all help text, docs, generated next-command hints,
   TUI strings, and fixtures.
3. Do not require users to learn hidden commands for the happy path.
4. If a workflow is interactive-first, bare `wh` should expose it; we should
   avoid redundant top-level verbs for the same mode.
