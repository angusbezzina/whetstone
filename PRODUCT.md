# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

Existing: a Rust binary (`wh`) serves a dependency-free local dashboard from
`assets/dashboard/` (plain HTML, CSS and vanilla JS; startup assets under
24 KiB; secondary views and editing are lazy modules). No framework, no build
step, no third-party requests.

## What Whetstone is

Whetstone is a repository's rulebook: enforced rules with a rationale and an
agent protocol. It sits between `AGENTS.md` (prose an agent may read, which
nothing enforces) and a linter (enforced, with no rationale and nothing an
agent can act on).

The rulebook is a file, `whetstone/rulebook.json`, committed in the
repository. It holds the mission, the chosen principles and the rules. Every
rule has one strength (must blocks the gate, should reports and never
blocks), one mechanical enforcer (ast, lint, formatter, test, validator) and
sweep fields (class, priority, unsure). A revision is a commit, the decision
log is Git history, and a rule change is a pull request. The rules are
enforced at pre-commit, pre-push and as the required CI status
`whetstone/policy`.

Hosts consume Whetstone through three things: the rulebook file,
`findings.json` (`wh check --sweep`: fingerprint, subjectDigest, class,
priority, title, responsibility, unsure) and the JSON envelope every command
returns. A host may be Virgil's Messo, a GitHub Action or an agent session.
Whetstone keeps no store and no host-specific code; hosts own memory, tasks
and the agent.

What makes it different:
- Only a mechanical pass passes a must rule.
- Unknown, stale, skipped or unavailable evidence is never reported as
  success.
- Every rule carries its rationale, and every flag names the rule
  responsible and how to recheck it.
- A rule changes only by an edit a person commits.

## Users and job

Primary user: a software engineer or small engineering team lead who lives in
the terminal and an editor, and opens `wh dash` in a browser tab beside them.
They are also the accountable project owner. In under a minute they want to
know:
- what the rules are and why each exists;
- which drafts are waiting to be accepted;
- what changed in the rulebook, when and by whom.

Coding agents read the same file and the same services through `--json`; the
dashboard is the human view of the same rulebook. The dashboard is not a task
tracker, a fleet console or a CI replacement.

## Surfaces

The dashboard has three views:

1. **Home** (default): what needs the owner (drafts to accept), the rules in
   force, setup still to do, and one primary action. Before there is a
   rulebook, it shows only onboarding: mission, principles, starter rules and
   gates.
2. **Rules**: mission and principles, then rules grouped by strength, each
   showing its enforcer, its paths and its sweep class, and whether it is
   accepted, edited or new against the file's last commit. Edit, Retire,
   Accept and Withdraw are `wh change` calls with an exact before/after
   review; nothing overwrites the committed rulebook silently.
3. **Changelog**: every commit that touched the rulebook, newest first,
   grouped by day, with the retired entries and their reasons beneath, and a
   TSV export of the decision trail.

## Durable constraints

- Honest states: missing, unknown, stale and not-run are explicit and never
  green. Read-only inspection is not success.
- No blended health score, no gamification, no tracker or fleet-console
  behaviour.
- Keyboard navigation, focus management, live-region feedback, safe rendering
  of user content, and widths of 320, 390, 768 and 1024 px.
- Startup HTML/CSS/JS stays dependency-free and under 24 KiB.
- CLI and dashboard read the same typed projections; the UI invents no state.
- The four public commands (`wh init`, `dash`, `change`, `check`) support the
  UI and may be progressively disclosed; they are not the human information
  architecture.

## Terminology

Mission, principle (a catalogue principle or the owner's own), rule (one
strength must/should and one mechanical enforcer), rulebook, draft (an
uncommitted edit to the rulebook), accepted (committed), retired, check,
finding, sweep, owner.

## Brand commitments

Owner-pinned 2026-09-14: the Whetstone logo (a flat isometric orange stone
on a black base; the wordmark set wide and tracked in caps, WHET in black
and STONE in orange). Core colours are black and orange; light mode is white
and orange. The design system uses ShadCN's token vocabulary (background,
foreground, card, muted, border, input, ring, primary, destructive, radius)
implemented in the dashboard's own dependency-free CSS; ShadCN's React
components are not adopted. The feel is a minimal personal journal. The
visual world chosen on 2026-09-14 is recorded in DESIGN.md.
