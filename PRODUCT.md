# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Existing: a Rust binary (`wh`) serves a dependency-free local dashboard from
`assets/dashboard/` (plain HTML, CSS and vanilla JS; startup assets under
24 KiB; secondary views and authenticated editing are lazy modules). No
framework, no build step, no third-party requests. The design reference is a
self-contained static HTML file under `planning/direction-demo/`.

## What Whetstone is

Whetstone is a local-first tool that turns a project's agreed intent into
enforced engineering standards and a tight correction loop. A project owner
records a mission, core values, key metrics, rules and guidelines, and
deterministic validation gates as versioned records. Existing coding agents
and humans get the applicable rules; `wh check` runs the deterministic gates
and returns failures to the same worker for bounded repair and recheck. Every
consequential change is an append-only, inspectable decision.

Mechanism that makes it different: judgment (interpreting intent, proposing
rules) stays with the skill/agent; deterministic, replayable work (schemas,
state transitions, check execution, receipts) stays in typed services. Unknown,
stale, skipped or unavailable evidence is never reported as success. Team
policy needs independent review; local drafts stay private until pushed.

Outputs for agents (2026-09-10): a generated, pstack-compatible verification
skill with a feature map (one file per user-facing feature: sub-features, how
a user reaches it, how to drive it, gotchas, plus the versioned "why" in
frontmatter) and a small team-owned driver script. Records live in Beads
(`bd`), not in a Whetstone-owned database and not in Git commits.

## Users and job

Primary user: a software engineer or small engineering team lead who lives in
the terminal and an editor, and opens `wh dash` in a browser tab beside them.
They are also the accountable project owner. In under a minute they want to
know: is the project on track against its own measures, are the safeguards
holding, and what is the one thing they must decide or hand to an agent.

Coding agents use the same typed services through `--json`; the dashboard is
the human view of the same records. The dashboard is not a task tracker, fleet
console, or CI replacement.

## Surfaces (confirmed 2026-09-10 with the owner)

Four views, in this order:

1. **Dashboard** (default): mission-linked key metrics, gate status, and the
   items needing attention, with one primary action. Before `wh init` has
   established the foundations, the dashboard shows only an onboarding path;
   no empty metric or gate panels.
2. **Foundations**: an editable, versioned flow in exactly five stages:
   Mission (desired outcomes shown inside it) -> Core values -> Key metrics ->
   Rules & guidelines (engineering philosophy and advisory guidance) -> Gates
   (deterministic standards; the initial safeguard is simply the first gate).
   Editing creates a local draft with exact before/after review; it never
   silently overwrites active policy.
3. **Checks** (replaces "Enforcement"): execution only. Last run per gate
   (pass / fail / unknown / not checked / stale), what failed, the exact
   recheck route, and the bounded repair brief to hand to the current coding
   agent. Definitions live in Foundations.
4. **Changelog** (replaces "Decisions"): newest-first, grouped by day and by
   atomic operation, human-readable titles, status, owner, area, search and
   "as of" inspection. Exact records only behind disclosure.

## Durable constraints

- Honest states: missing, unknown, stale, not-installed and not-observed are
  explicit and never green. Read-only inspection is not success.
- No blended health score, no gamification, no tracker or fleet-console
  behaviour.
- Keyboard navigation, focus management, live-region feedback, safe rendering
  of user content, and widths of 320, 390, 768 and 1024 px.
- Startup HTML/CSS/JS stays dependency-free and under 24 KiB.
- CLI and dashboard read the same typed projections; the UI invents no state.
- The six public workflow families (`wh init`, `dash`, `change`, `check`,
  `pull`, `push`) support the UI and may be progressively disclosed; they are
  not the human information architecture.

## Terminology

Mission, desired outcome, core value, key metric (definition vs observation),
rule/guideline (advisory; engineering philosophy and guidance records), gate
(deterministic standard with strength must/should/may and a mechanism: test,
validator, lint proxy, formatter, AST query), check (a run of gates), receipt,
local draft, accepted, superseded, owner.

## Brand commitments

None recorded. The prior look (warm cream paper, Georgia headings, forest
green accent) is evidence of the subject, not a commitment; the owner asked
for a complete rework. [Inferred from the 2026-09-10 brief.]

## Open decisions

- Whether team sharing (`wh push`/`wh pull`) appears in the UI before M2; the
  epic says only when the integration is real.
