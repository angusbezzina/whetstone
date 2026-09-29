# Whetstone dashboard direction reference

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.
It is the visual and information-architecture reference for the `wh dash` local
UI (Beads `whetstone-ppq.1.3`); the implementation lives in `assets/dashboard/`.

> **Status (2026-09-28).** Current for the delegation plan
> (`planning/direction.md`). The page is generated: it embeds the product's own
> dashboard assets (`app.css`, `views.css`, `app.js`, `views.js`, `edit.js`)
> with an in-page demo backend that answers the same requests `wh dash` does.

## Rebuilding

- `mock.js` is the demo backend: the fictional project, its records and how
  each dashboard request (inspect, init, change, check) changes them, in
  memory. It must return the same JSON shapes as `wh dash --json` and the
  dashboard service, so update it when those shapes change.
- `build.py` splices the dashboard assets and `mock.js` into `index.html`,
  loading the lazy modules through blob URLs instead of the server paths.

After changing either, or any file in `assets/dashboard/`, run:

```sh
python3 planning/direction-demo/build.py
node planning/direction-demo/smoke.mjs
```

When the dashboard's information architecture changes, change the demo data
in `mock.js` with it and keep `smoke.mjs` passing. Do not edit `index.html`
by hand; the next build overwrites it.

All sample data (project name, owner, records, dates, checks, receipts,
requests) is fictional. Nothing in the file runs a check, saves a record, or
contacts a service; its state lives in memory and resets on reload. The strip
at the top labelled **Demo controls** switches between a fresh project and an
established one and resets the sample data; it is not part of the product.

## Views

Five views, in this order. Navigation is a row of five text tabs beneath the
header that stays at the top while the page scrolls and wraps under 390 px.
Checks, Changelog and Requests are gated until the agreement has a mission and
at least one rule.

- **Home** (default). Before `wh init` it shows onboarding only: the six steps
  (Beads and pstack, mission, principles, exemplars, rules, gates) with their
  state, any missing tool with its exact fix, a *Start onboarding* action and
  the `wh init` alternative. Onboarding asks for the mission, offers the pinned
  pstack principle catalogue and custom principles, preselects the three
  starter rules with their strength, enforcer and labelled examples, drafts
  rules from an exemplar codebase, and names the hook command. *Review
  agreement* shows the exact records; *Record agreement* writes them. Once
  agreed, Home shows the mission, a *Needs attention* list with exactly one
  primary action (raised hands and failing must rules first), every rule in
  force with its strength, enforcer, where it runs and its result, any setup
  step still to do, and the latest change. There is no blended health score.
- **Rules.** Mission, principles, then rules grouped by strength: must (fails
  the check), should (flags, never blocks) and advisory (briefed, never
  checked). Each rule shows its one enforcer family and mechanism, where it
  runs, its shadow or local-only status, when it raises a hand, its record of
  checks and flags with the false-flag rate and Jev cost, its source, and its
  labelled examples behind a disclosure. *Edit* opens an inline editor for the
  statement, strength, enforcer and paths; *Review draft* shows the exact
  before/after; *Record local draft* writes a private draft that is not in
  force until it is accepted. Suggestions (demotion, promotion, hardening) come
  from each rule's flags and are recorded as drafts, never applied.
- **Checks.** Execution only. A fixed-column board of checked rules with
  strength, family, shadow status, last run and result; unknown, not-run,
  quarantined and stale results are never green. A failing rule opens into its
  failures, the exact recheck command, a copyable repair brief and its evidence.
  *Run checks* runs every rule, what changed, the staged files or a feature
  sweep. *Flags to label* lists each unlabelled flag with *Right* and *False
  flag*; each label needs a reason and feeds the false-flag rate.
- **Changelog.** The decision trail, newest first, grouped by day: title,
  version change, status, owner, area, summary, and the exact records behind a
  disclosure. Drafts are accepted or withdrawn here. Search and *As of* query
  the trail; the trail exports as TSV.
- **Requests.** Raised hands: the agent's question, trigger, rule, what it
  tried and what it recommends, filed as a Beads issue labelled `human`. Open
  requests come first and are answered in place (or with the shown
  `wh change --answer` command); answered ones keep the answer, who gave it and
  when.

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

It asserts the gated fresh state and the six onboarding steps, the onboarding
agreement flow, one primary action, honest state marks, rules grouped by
strength with enforcer, shadow status and false-flag rate, the edit → review →
draft flow, the failing rule's brief, labelling a flag, the run-checks update,
answering a request, accepting a draft, changelog search and as-of, keyboard
navigation of the rail, and the absence of horizontal overflow at 320, 390, 768
and 1280 px in both themes, and writes screenshots to a temporary directory it
prints. It validates the reference, not the product; the product's own browser
proofs are `scripts/test-dashboard-browser.mjs` and
`scripts/test-dashboard-live-browser.mjs`.

## Related planning material

- `planning/direction.md`: the current plan and fixed decisions.
- `planning/skill-cli-boundary.md`: judgment versus deterministic work.
- `planning/archive/`: superseded material, including the R0 prune
  inventory and the storage and trust-boundary ADRs.
