# Command Taxonomy Cleanup

> **Status:** stage 1 implemented · 2026-04-22
> **Tracking:** whetstone-8to1

## Goal

Reduce the visible Whetstone command surface to the core happy path while
preserving backward compatibility for existing scripts and operator muscle
memory.

## Desired visible taxonomy

### Core workflow

- `wh init`
- `wh extract`
- `wh approve`
- `wh actions`
- `wh check`
- `wh reinit`
- `wh status`
- `wh debt`
- `wh tui`

### Advanced grouped commands

- `wh rule add | edit | query | review | worklist`
- `wh source add | list | remove | fetch`
- `wh actions --only context|tests|lint`
- `wh status --report [--pr-comment]`

### Maintenance

- `wh validate`
- `wh update`

## Stage 1 migration plan

1. **Hide duplicated top-level commands from default help**
   - `set-sources`
   - `context`
   - `tests`
   - `lint`
   - `ci`
   - `review`
   - `rules`
   - `report`
2. **Keep them callable for compatibility** so existing scripts do not break.
3. **Promote grouped entrypoints**
   - add `wh rule query`
   - add `wh rule review`
   - add `wh rule worklist`
   - add `wh actions --only ...`
   - add `wh status --report`
4. **Update help surfaces**
   - `wh --help`
   - TUI help overlay

## Compatibility mapping

| Legacy/top-level form | Preferred form |
|-----------------------|----------------|
| `wh set-sources` | `wh init` |
| `wh context` | `wh actions --only context` |
| `wh tests` | `wh actions --only tests` |
| `wh lint` | `wh actions --only lint` |
| `wh rules query` | `wh rule query` |
| `wh review` | `wh rule review` |
| `wh review worklist` | `wh rule worklist` |
| `wh report` | `wh status --report` |
| `wh ci` | hidden CI-specialized command (still supported) |

## Non-goal for stage 1

Do not remove compatibility entrypoints yet. This is a help/discovery cleanup
first, not a breaking CLI rewrite.
