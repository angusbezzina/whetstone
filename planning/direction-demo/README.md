# Whetstone dashboard direction reference

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.
It is the visual reference for the `wh dash` local UI; the implementation
lives in `assets/dashboard/`.

> **Status (2026-09-27).** The visual world is current. The content predates
> the delegation plan (`planning/direction.md`): the eight owner decisions,
> key metrics, and the five Foundations stages. Beads task
> `whetstone-ppq.1.3` replaces that content with onboarding (mission,
> principles, exemplar codebases, accepting proposed rules), Rules (grouped
> by strength with enforcer, shadow status and false-flag rate), Checks,
> Changelog and Requests. `whetstone-ppq.3.8` then builds it. Until then, the
> views below describe the reference as it stands.

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

Struck Cathode Gauze, chosen by the owner on 2026-09-14: the logo's black and
orange (white and orange in light mode) in ShadCN token names. Every option
sits as a quiet ghost, and the one live thing is struck orange. There are no
boxes or rules, and states are marks rather than hues. The binding decisions
are in `DESIGN.md` and the surface brief under `.impeccable/surfaces/`.

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

- `planning/direction.md`: the current plan and fixed decisions.
- `planning/skill-cli-boundary.md`: judgment versus deterministic work.
- `planning/archive/`: superseded material, including the R0 prune
  inventory and the storage and trust-boundary ADRs.
