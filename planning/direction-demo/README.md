# Whetstone direction demo

Open [index.html](index.html) directly in a browser. It is a self-contained HTML
artefact: no build, server, network requests, account, or runtime dependencies.

Direction 05 keeps the same five views and six public workflow families.
The proposed implementation is tracked in **Beads epic `whetstone-k5r`**; this
HTML is a behavioral reference, not an implemented enforcement backend.

The five views are:

- **One-pager:** define good, observe trusted signals, authorize bounded work,
  correct it, and follow the outcome; one setup flow and six proposed
  commands: `wh init`, `wh dash`, `wh change`, `wh check`, `wh push`, and `wh pull`.
  “Print one-pager” prints this brief from any view.
- **Workspace:** eight four-stage case replays with human summaries first and
  expandable agent feedback/receipts. The default case starts with a trusted
  runtime signal and an already-approved mandate, not a newly prompted task.
  It reproduces the fault, deduplicates tracked work, repairs within budget,
  verifies, and follows a separately authorized release to an observed effect
  with causal uncertainty and owner review still explicit.
  A personal-to-team case demonstrates compatible local experimentation,
  deliberately selected sharing, independent authenticated human review,
  content-bound acceptance, verified activation, and stale client detection.
  A context-maintenance case proposes archival after a reference audit, obtains
  review, and proves historical retrieval before removing superseded advice
  from active context. No source is deleted based on age or similarity alone.
  Three original cases show an agent repairing its
  mistake without owner interruption, stopping for a real owner decision, and
  stopping after a bounded retry when a required checker is unavailable.
  Two further cases extend this through the outer loop: checks pass but the owner
  redirects a costly approach, and an accepted/released change produces an
  adverse outcome that opens an authorized local repair. Verification,
  authorization, and outcome have separate states. Reverification does not
  approve the revised candidate or resolve a live incident. Sample shortcuts
  foreground signal-triggered work, team-rule decisions, and follow-through.
  Nothing automatically advances: each button simulates the next stage, and
  restart/case selection resets that task. No agent or command actually runs.
  Each standard's expandable inspector explains why it applies, its governing
  goal, evidence versus judgment, and canonical context delivery revisions.
  Matching considers intent, components, operations, and environment, not paths
  alone; an agent's relevance judgment cannot remove mandatory checks.
  Below the replay, six fictional standards belong to a **separate reference PR** with
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
  Expand delegation and follow-through defaults to edit permitted work,
  exclusions, review triggers, budgets, observation source/owner/window, and
  recovery, optional signal triggers, duplicate-work controls, independent
  review, and active-context retention. These are inherited proposals, not per-task forms or permission
  grants. Signal-triggered work requires a separate explicit opt-in; exporting
  these fields does not enable it. The preview distinguishes enforceable host/platform restrictions from
  advisory constraints. Downloading does not activate authority or monitoring.
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

## Implementation epic

`bd show whetstone-k5r` opens the canonical detailed plan; use
`bd list --parent whetstone-k5r --limit 0` to inspect its 30 children.
The epic contains 90 blocking prerequisite links and these delivery gates:

| Stage | Scope                                                                            | Exit / starting issue   |
| ----- | -------------------------------------------------------------------------------- | ----------------------- |
| M0    | Contract/cutover, storage feasibility, trust roots                               | Start `whetstone-k5r.1` |
| M1    | Useful local agreement, native checks, agent loop, dashboard, history            | `whetstone-k5r.16`      |
| M2    | Private-safe sharing, independent approval, protected activation, two hosts      | `whetstone-k5r.20`      |
| M3    | Observations, mandates, deduplicated work, proactive repair, context maintenance | `whetstone-k5r.27`      |
| M4    | Portable integration, migration/distribution, implementation readiness           | `whetstone-k5r.30`      |

Every child has scope, acceptance tests, dependencies and handoff requirements.
The plan explicitly distinguishes the v0.12 CLI from this direction, audits
existing modules for reuse, and leaves all implementation children open.
Dolt feasibility, the first real identity/CI trust root, migration policy, and
dogfood budgets are early owner-reviewed decisions, not assumptions hidden in
the prototype. Existing private-mode and distribution work remains separately
tracked. The Beads epic is authoritative; this is only a navigation index, not a
second implementation backlog. No release/tag is authorized by creating the plan.

All Northstar data, dates, authors, checks, receipts, and approvals are fictional.
The proposed CLI, governance records, integrations, and permission model are not
implemented. The exported JSON is not an accepted input schema for the current CLI.
The demo does not install skills or change any project policy. Hosted deployment
is not required: the same static file can be hosted behind an appropriate access
boundary if desired.

## Feedback and completion model (proposed)

This direction applies the distinction between routine automation and accountable
decisions in [Addy Osmani’s Own the Outer Loop](https://addyosmani.com/blog/own-the-outer-loop/).
The following are Whetstone design proposals, not capabilities implemented here.

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
- Approved opt-in mandates can match fresh trusted observations to bounded work
  without a new prompt. Existing schedulers invoke checks and existing runtimes
  execute; `wh check` never launches repairs itself. Reproduce heuristic findings,
  deduplicate tasks, claim one worker, enforce cooldowns and persistent budgets,
  and revalidate expiry/revocation before side effects. Goals alone grant no
  permission; PR creation, merge, release, and policy sharing are separate actions.
- Inherit explicit standing authority for routine work, with exclusions,
  retry/time/resource limits, and suspension conditions. Bind enforceable limits
  to real host permissions and protected gates; unsupported limits are advisory.
  Routine code-work authority never silently authorizes Whetstone publication.
- Separate verification, authorization, and outcome. Owners may redirect or
  reject technically verified work for cost, priorities, or risk. Consequential
  verdicts recorded through `wh dash` / `wh change` bind the reviewed candidate;
  changed code requires fresh verification and the applicable new verdict.
  A second agent is not a substitute for accountable authority.
- Acceptance includes the expected effect, observation source, release cohort,
  freshness, responsible owner, review window, and recovery action. Existing
  monitoring and release tools remain authoritative for their events. An adverse
  result alerts the operator and feeds a bounded investigation/repair to the
  agent. Missing data stays unknown. Local passing tests do not close a live
  incident; release/rollback needs authority and a fresh observation follows it.
- Missing authority stops immediately. Retry/time budgets and repeated no
  progress bound other repairs. A blocked handoff carries the exact question,
  recommendation, options, evidence, revision, and permitted next action.
- Stable IDs and expected revisions support resumption; stale responses are
  rejected and repeated actions are idempotent. These are illustrative contracts,
  not an implemented agent protocol in this HTML.
- Gradual adoption preserves existing safeguards and makes legacy debt explicit.
  Prove a known-bad change, an authorized repair, a known-good recheck, and a real
  escalation. Mission outcomes are distinct from stage-specific blocking gates.
- Test the loop itself: real mistakes are repaired, valid alternatives survive,
  stale approvals and changed checkers block, user requirements cannot disappear
  behind narrow tests, and authority gaps reach the right owner. Re-evaluate after
  model, skill, or integration changes. Measure false blocks, unnecessary
  interruptions, escaped failures, decision effort, and overdue follow-through—not
  simply the number of green checks or how rarely humans are involved.
- Explain why standards apply, the goal/source they derive from, and what can
  actually be verified. Passing tests cannot prove test-first development without
  process evidence. Every host receives projections of the same approved records
  with policy/checker/adapter revisions and visible delivery freshness; shared
  source material does not promise identical model behavior.
- Pair near-term signals with long-term outcomes and countervailing safeguards.
  Before/after movement alone does not establish causality; no effect is a valid
  result. Metric definition changes need reviewed rationale and preserved history.
- Completion events can prompt archive proposals, not automatic deletion.
  Verify source hashes, unresolved requirements, incoming references and
  recoverability; review the exact source and active-context diff before apply.
  Current context excludes superseded advice, while as-of history retains it.
  Failed archival leaves active material intact; restore through a new change.
- Preserve linked intent, constraints, changes, verification, decisions, and
  outcomes, including accepted/rejected examples with rationale. Supply only
  relevant context to agents. Dolt records and stable portable interfaces suffice;
  no graph database, replacement tracker, fleet manager, or custom telemetry.
  First implementation: one repository and one existing CI/release integration.

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
- Personal preferences can narrow compatible choices but cannot contradict team
  requirements. Team approval defaults to a real authorized human other than the
  proposer; agents inherit their principal's identity. Solo self-approval is
  explicit. Facts, meeting notes and personal memories are candidate sources, not
  implicit authority; never silently ingest or publish raw private memories.
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
All case replays have their own isolated task/decision/observation IDs; their
simulated decisions never append to that separate reference history.

Whetstone remains a governed engineering component: Beads may track work and
Virgil may provide conversation/coordination, but neither is required for local
use. Dolt is the proposed record store. Whether any supported Beads interface can
also serve durable policy storage requires a feasibility decision; the transcript
does not establish that it supplies the necessary governance or retention.

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
scenario isolation, verified-but-redirected work, outcome-triggered repair,
separate authority/outcome states, inherited defaults without permission grants,
standard filters and details, blocked reference check replay,
signal-triggered reproduction/repair/follow-through, independent policy review,
reviewed archival and active/historical separation, applicability and context
freshness, collapsed receipts, optional mandate defaults without activation,
selective publication, private-record exclusion,
no implicit approval, pull conflicts and preserved local drafts, dialog
dismissal, setup edits and proposal download, all three workflows, clipboard
fallback, lifetime history and supersession at historical dates,
empty search, narrow layouts, keyboard navigation, print layout, and absence of
external requests or browser errors. This validates the prototype interactions,
not the proposed enforcement backend.
