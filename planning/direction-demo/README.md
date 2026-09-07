# Whetstone direction demo

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.

The five views are:

- **One-pager:** the closed correction loop, one setup flow, and six proposed
  commands: `wh init`, `wh dash`, `wh change`, `wh check`, `wh push`, and `wh pull`.
  “Print one-pager” prints this brief from any view.
- **Workspace:** a four-stage task replay: intent, feedback, response, and fresh
  verification or a precise blocker. Three cases show an agent repairing its
  mistake without owner interruption, stopping for a real owner decision, and
  stopping after a bounded retry when a required checker is unavailable.
  Nothing automatically advances: each button simulates the next stage, and
  restart/case selection resets that task. No agent or command actually runs.
  Below it, six fictional standards belong to a **separate reference PR** with
  unresolved gates; completing the task replay does not mark that PR healthy.
  “Replay reference checks” replays its sample receipts. “Preview push” lets you
  select shareable drafts, see the outgoing payload, and simulate publication.
  A private preference is always excluded, including its history. Shared
  proposals do not change the accepted team agreement. “Simulate pull” shows an
  incoming conflict while preserving the draft and distinguishing accepted,
  required, and installed policy revisions. It does not activate the new policy.
- **Setup:** inspect, agree, preview wiring, and prove feedback → repair → recheck.
  Mission, values, philosophy, owner, desired outcome, and safeguard are
  editable and can be downloaded as an explicitly unapproved concept proposal.
  Draft edits do not change the workspace snapshot. Reload resets the draft.
- **Workflows:** ordinary engineer and agent correction loops, plus a separate
  policy-amendment walkthrough, with selectable
  steps. The public surface has six command families, plus bare `wh` for read-only
  orientation and `--json` for structured agent use. Approval, generation, history,
  and conflict resolution are steps within those workflows, not extra commands.
  An expandable operating contract covers completion, authority, task scope,
  trusted execution, resumability, metrics, and the small first-release boundary.
- **Decisions:** the complete fictional project history, searchable and sortable,
  with historical cutoff views and preserved superseded decisions.

All Northstar data, dates, authors, checks, receipts, and approvals are fictional.
The proposed CLI, governance records, integrations, and permission model are not
implemented. The exported JSON is not an accepted input schema for the current CLI.
The demo does not install skills or change any project policy. Hosted deployment
is not required: the same static file can be hosted behind an appropriate access
boundary if desired.

## Feedback and completion model (proposed)

- Fast relevant checks feed failures into the active worker's context. Use host
  hooks where supported and explicit CLI checkpoints elsewhere. Broader checks
  and an acceptance/scope review are required before task completion.
- The skill directs authorized repairs using the agent's existing tools. The
  deterministic CLI checks the result; `wh check` does not silently fix code,
  weaken policy, approve, or publish. Human users get the same feedback and
  explicitly authorize any agent edits.
- Receipts bind code, required policy, checker configuration, scope, and freshness.
  Changed inputs invalidate affected receipts. Unknown or unavailable required
  checks and missing approvals cannot become success. Native protected CI is
  independent of whether an agent follows its instructions.
- Reuse the existing request or tracker for task intent. No new policy decision
  is needed for every edit. Explicit path/operation limits can be checked;
  semantic task relevance and taste remain judgment, not deterministic claims.
  Task-scope support is a proposed expansion beyond the current shipped boundary.
- Missing authority stops immediately. Retry/time budgets and repeated no
  progress bound other repairs. A blocked handoff carries the exact question,
  recommendation, options, evidence, revision, and permitted next action.
- Stable IDs and expected revisions support resumption; stale responses are
  rejected and repeated actions are idempotent. These are illustrative contracts,
  not an implemented agent protocol in this HTML.
- Gradual adoption preserves existing safeguards and makes legacy debt explicit.
  Prove a known-bad change, an authorized repair, a known-good recheck, and a real
  escalation. Mission outcomes are distinct from stage-specific blocking gates.

## State and sharing model (proposed)

- Local Dolt state owns agreements, standards, decisions, and compact receipts.
  Markdown/JSON documents are readable views or exports; their edits enter the
  same local change workflow rather than creating a second source of truth.
- Local saves, checks, and dashboard use do not publish. `wh push` previews and
  shares selected records; `wh pull` only brings in team updates. A remote is
  optional. No code-repository commits are required for Whetstone's records.
- Private data and private ancestry must live outside the replicated sharing
  path; publication cannot be a blind push of the complete local Dolt database.
- Selected packages need complete references. Private or unavailable dependencies
  block publication rather than being auto-included or silently omitted. Bind
  confirmation to the exact package, destination, and base revision.
- Sharing and approval are separate. Required team checks continue using an
  explicitly identified accepted agreement. Local experiments cannot weaken
  protected CI or platform gates. Approval and rollout need real authority and
  verification; database versioning alone does not provide that protection.
- Accepted policy, required policy for a code scope/environment, installed local
  wiring, and experiments are distinct. A protected activation record selects
  the required policy/checker digests. Local conflicts do not set team authority;
  evaluate experimental results separately from unchanged mandatory team checks.
- Imported or modified executables remain inert until explicitly trusted. Review
  commands, configuration, working directory, outputs, and access; reuse runner
  isolation and restricted credentials. Hosted inspection is read-only by default.
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

Coverage includes the six-command contract, navigation, the bounded repair loop,
fresh re-verification, owner escalation, unavailable-checker blocking, reset and
scenario isolation, standard filters and details, blocked reference check replay,
selective publication, private-record exclusion,
no implicit approval, pull conflicts and preserved local drafts, dialog
dismissal, setup edits and proposal download, all three workflows, clipboard
fallback, lifetime history and supersession at historical dates,
empty search, narrow layouts, keyboard navigation, print layout, and absence of
external requests or browser errors. This validates the prototype interactions,
not the proposed enforcement backend.
