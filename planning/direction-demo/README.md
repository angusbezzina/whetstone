# Whetstone dashboard direction reference

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.
It is the behavioural and visual reference for the `wh dash` local UI; the
implementation lives in `assets/dashboard/` and is tracked in Beads epic
`whetstone-k5r` (see `whetstone-k5r.15` and its children).

All sample data (project name, records, dates, checks, receipts) is fictional.
Nothing in the file runs a check, saves a record, or contacts a service. The
strip at the top labelled **Demo controls** switches between a fresh project and
an established one and resets the sample data; it is not part of the product.

## Views

Four views, in this order. Navigation is a row of four text tabs beneath the
header that stays at the top while the page scrolls and wraps under 390 px.

- **Dashboard** (default). Before `wh init` has established the foundations
  it shows one panel only: what the eight owner decisions are, an
  *Establish foundations* action, and the `wh init` alternative. Checks and
  Changelog are gated until then. Once established it shows the mission line,
  a *Needs attention* panel with exactly one primary action, the key metrics
  and gates with their current state, and the latest change. There is no
  blended health score.
- **Foundations.** A five-stage flow: Mission (with its desired outcomes) →
  Core values → Key metrics → Rules & guidelines (engineering philosophy and
  advisory guidance) → Gates (deterministic standards). Every record shows its
  version and lifecycle. *Edit* opens an inline editor; *Review draft* shows the
  exact before/after and the effects of confirming; *Record local draft* writes
  a private draft, prepends a changelog entry, and leaves the accepted record in
  force until the draft is accepted.
- **Checks.** Execution only. A fixed-column board of gates with mechanism,
  scope, last run and result; unknown, not-run and stale results are amber and
  never green. A failing gate opens along one axis into its failures, the exact
  recheck command, and a copyable repair brief for the current coding agent.
  *Run checks* updates rows in place. Advisory rules are named but never
  counted as checks. Receipts sit behind a disclosure.
- **Changelog.** Every consequential change from project start, newest first,
  grouped by day: human title, version change, status (draft, accepted,
  superseded), owner, area, summary, and the exact records behind a disclosure.
  Search filters entries; *As of* dims entries recorded after the chosen time.

## Visual world

The Notebook world pinned by the owner on 2026-09-10, replacing the earlier
Graphite Console; the durable decisions are recorded in `DESIGN.md` and the
surface brief under `.impeccable/surfaces/`. In short: one calm column of
paper with generous margins, every section a serif heading on a rule and every
record an entry on a ruled line, no boxes, panels or shadows; light and dark
from one token set, following the system or pinned by a small toggle in the
header, with every text pair verified at 4.5:1 or better in both; system faces
only (a book serif for headings, the system sans for everything else, mono only
for commands and exact records); the primary action in ink; and colour spent
only on state by law: green for a fresh real pass or an accepted record, ochre
for unknown, stale or not run, red for fail or off target, dashed rules and
rings for drafts and anything not yet in force.

## Smoke test

`smoke.mjs` drives the file in headless Chrome over the DevTools protocol with
no npm dependencies (Node 22 or later):

```sh
node planning/direction-demo/smoke.mjs
```

It asserts the gated fresh state, the five-stage order, one primary action,
honest state colours, the edit → review → draft flow, the run-checks update,
changelog search and as-of, keyboard navigation of the rail, and the absence of
horizontal overflow at 320, 390, 768, 1024 and 1440 px, and writes screenshots
to a temporary directory it prints. It validates the reference, not the
product.

## Related planning material

- `planning/skill-cli-boundary.md`: judgment versus deterministic work.
- `prune-inventory.md`: the R0 allowlist and recovery record.
- `storage-spike.md` and `trust-boundary.md`: M0 decisions.
- Older direction material lives in `planning/archive/`.
