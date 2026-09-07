# Whetstone direction demo

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.

The five views are:

- **One-pager:** the enforcement-first direction and a condensed setup explanation.
  “Print one-pager” prints this brief from any view.
- **Workspace:** six fictional standards, filterable by enforcement type, with
  scope, owner, native binding, evidence, and linked rationale. “Run demo checks”
  replays sample receipts; it does not execute a command.
- **Setup:** inspect, agree, preview wiring, and review. Sample preferences are
  editable and can be downloaded as an explicitly unapproved concept proposal.
  Draft edits do not change the workspace snapshot. Reload resets the draft.
- **Workflows:** engineer, agent, and policy-amendment walkthroughs with selectable
  steps and illustrative CLI contracts. The reference distinguishes proposed
  commands from the existing v0.12 dependency-rule workflow.
- **Decisions:** the complete fictional project history, searchable and sortable,
  with historical cutoff views and preserved superseded decisions.

All Northstar data, dates, authors, checks, receipts, and approvals are fictional.
The proposed CLI, governance records, integrations, and permission model are not
implemented. The exported JSON is not an accepted input schema for the current CLI.
The demo does not install skills or change any project policy. Hosted deployment
is not required: the same static file can be hosted behind an appropriate access
boundary if desired.

## Browser smoke test

`smoke.cjs` uses Playwright and an installed Chrome browser. It adds no dependency
to Whetstone. With Playwright available in the current Node resolution path:

```sh
node planning/direction-demo/smoke.cjs
```

Alternatively, set `WHETSTONE_PLAYWRIGHT_MODULE` to an existing Playwright module
directory. Set `WHETSTONE_BROWSER_CHANNEL` to another installed Playwright channel
if needed; the default is `chrome`. The test creates screenshots and a printable
PDF in a uniquely named temporary directory and prints its location.

Coverage includes view navigation, standard filters and details, blocked check
replay, dialog dismissal, setup edits and proposal download, all three workflows,
clipboard fallback, lifetime history and supersession at historical dates,
empty search, narrow layouts, keyboard navigation, print layout, and absence of
external requests or browser errors. This validates the prototype interactions,
not the proposed enforcement backend.
