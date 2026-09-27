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

Whetstone lets people delegate more to coding agents and trust the result. The
owner's mission, the pstack principles they pick and the codebases they admire
become a short list of agreed rules. Every rule has one strength (must, should
or advisory) and exactly one enforcer, chosen in this order: a mechanical
check, a literal yes/no Jev question (TypeSafe System One), or a review by
pstack's `/interrogate` or a person. Agents and people in any tool are briefed
on the rules before building, checked at pre-commit, pre-push and CI, and
told when to raise a hand. Every consequential change is an append-only,
inspectable decision. The direction is in `planning/direction.md` and Beads
epic `whetstone-ppq`.

Mechanism that makes it different:
- Judgment (interpreting intent, proposing rules, briefing) stays with the
  skill and pstack's skills.
- Network calls and driving stay in the driver.
- Deterministic, replayable work (schemas, state transitions, check
  execution, receipts) stays in typed services.
- A Jev answer can raise a flag but never pass a must rule.
- Unknown, stale, skipped or unavailable evidence is never reported as
  success.
- Rule changes, including demotions and promotions, are drafts a person
  accepts.
- Local drafts stay private until pushed.

Outputs for agents: a generated, pstack-compatible verification skill with a
feature map (one file per user-facing feature: sub-features, how a user
reaches it, how to drive it, gotchas, plus the versioned "why" in frontmatter)
and a small team-owned driver script. Records live in Beads (`bd`), not in a
Whetstone-owned database and not in Git commits.

## Users and job

Primary user: a software engineer or small engineering team lead who lives in
the terminal and an editor, and opens `wh dash` in a browser tab beside them.
They are also the accountable project owner. In under a minute they want to
know:
- whether the rules are holding;
- which flags or raised hands need them;
- whether any rule is getting noisy.

Coding agents use the same typed services through `--json`; the dashboard is
the human view of the same records. The dashboard is not a task tracker, fleet
console, or CI replacement.

## Surfaces

The four views confirmed on 2026-09-10 are being reshaped for the delegation
plan (Beads `whetstone-ppq.1.3` updates the reference, and `whetstone-ppq.3.8`
builds it):

1. **Dashboard** (default): what needs the owner (raised hands, flags to
   accept or dismiss, drafts), rule health, and one primary action. Before
   `wh init`, it shows only onboarding: mission, principles, exemplar
   codebases, then accepting each proposed rule with its examples, strength
   and enforcer.
2. **Rules** (replaces the five-stage Foundations): mission and principles,
   then rules grouped by strength, each showing its enforcer, shadow status
   and false-flag rate. Editing creates a local draft with exact before/after
   review; it never silently overwrites an accepted rule.
3. **Checks**: execution only. It shows:
   - the last run per rule (pass, flag, fail, unknown, not checked, stale);
   - what failed;
   - the exact recheck route;
   - the repair brief to hand to the current agent.
4. **Changelog**: the decision log, newest first, grouped by day. It includes
   raised hands and answers, accepted and dismissed flags, strength changes
   and redactions. Exact records sit behind disclosure.

Until that work lands, the shipped dashboard still shows the earlier
eight-decision onboarding and five Foundations stages.

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

Mission, principle (a pstack principle or the owner's own), exemplar codebase,
rule (one strength must/should/advisory and one enforcer: mechanical check,
Jev question or review), shadow mode, flag (accepted or dismissed), raised
hand, brief, check (a run of rules), receipt, local draft, accepted,
superseded, owner.

## Brand commitments

Owner-pinned 2026-09-14: the Whetstone logo (a flat isometric orange stone
on a black base; the wordmark set wide and tracked in caps, WHET in black
and STONE in orange). Core colours are black and orange; light mode is white
and orange. The design system uses ShadCN's token vocabulary (background,
foreground, card, muted, border, input, ring, primary, destructive, radius)
implemented in the dashboard's own dependency-free CSS; ShadCN's React
components are not adopted. The feel is a minimal personal journal. The
visual world chosen on 2026-09-14 is recorded in the surface brief and
DESIGN.md.

## Open decisions

- Whether team sharing (`wh push`/`wh pull`) appears in the UI before the
  team-and-tools milestone (`whetstone-ppq.10.7`).
