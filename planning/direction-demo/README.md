# Whetstone direction demo

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.

The five views are:

- **One-pager:** the enforcement-first direction, one setup flow, and six proposed
  commands: `wh init`, `wh dash`, `wh change`, `wh check`, `wh push`, and `wh pull`.
  “Print one-pager” prints this brief from any view.
- **Workspace:** six fictional standards, filterable by enforcement type, with
  scope, owner, native binding, evidence, and linked rationale. “Run demo checks”
  replays sample receipts; it does not execute a command. “Preview push” lets you
  select shareable drafts, see the outgoing payload, and simulate publication.
  A private preference is always excluded, including its history. Shared
  proposals do not change the accepted team agreement. “Simulate pull” shows an
  incoming conflict while preserving the local version and active agreement.
- **Setup:** inspect, agree, preview wiring, and review. Sample preferences are
  editable and can be downloaded as an explicitly unapproved concept proposal.
  Draft edits do not change the workspace snapshot. Reload resets the draft.
- **Workflows:** engineer, agent, and policy-amendment walkthroughs with selectable
  steps. The public surface has six command families, plus bare `wh` for read-only
  orientation and `--json` for structured agent use. Approval, generation, history,
  and conflict resolution are steps within those workflows, not extra commands.
- **Decisions:** the complete fictional project history, searchable and sortable,
  with historical cutoff views and preserved superseded decisions.

All Northstar data, dates, authors, checks, receipts, and approvals are fictional.
The proposed CLI, governance records, integrations, and permission model are not
implemented. The exported JSON is not an accepted input schema for the current CLI.
The demo does not install skills or change any project policy. Hosted deployment
is not required: the same static file can be hosted behind an appropriate access
boundary if desired.

## State and sharing model (proposed)

- Local Dolt state owns agreements, standards, decisions, and compact receipts.
  Markdown/JSON documents are readable views or exports; their edits enter the
  same local change workflow rather than creating a second source of truth.
- Local saves, checks, and dashboard use do not publish. `wh push` previews and
  shares selected records; `wh pull` only brings in team updates. A remote is
  optional. No code-repository commits are required for Whetstone's records.
- Private data and private ancestry must live outside the replicated sharing
  path; publication cannot be a blind push of the complete local Dolt database.
- Sharing and approval are separate. Required team checks continue using an
  explicitly identified accepted agreement. Local experiments cannot weaken
  protected CI or platform gates. Approval and rollout need real authority and
  verification; database versioning alone does not provide that protection.
- Shared native checks may still need committed CI configuration or equivalent
  protected platform setup. Raw telemetry remains in its existing system.

The HTML itself uses only in-memory sample data, not Dolt or an actual remote.
All simulated sharing, selections, and edits reset on reload. The accepted
history view deliberately remains the fixed fictional v1.3 snapshot.

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

Coverage includes the six-command contract, navigation, standard filters and
details, blocked check replay, selective publication, private-record exclusion,
no implicit approval, pull conflicts and preserved local drafts, dialog
dismissal, setup edits and proposal download, all three workflows, clipboard
fallback, lifetime history and supersession at historical dates,
empty search, narrow layouts, keyboard navigation, print layout, and absence of
external requests or browser errors. This validates the prototype interactions,
not the proposed enforcement backend.
