---
version: 1
slug: "planning-direction-demo-index-html"
primary_target: "planning/direction-demo/index.html"
related_targets: ["assets/dashboard/index.html"]
---

# Surface brief: Whetstone dashboard direction reference

Scope: `planning/direction-demo/index.html`, the self-contained behavioural and visual reference for the `wh dash` local UI (implemented later in `assets/dashboard/`). Visitor mode: Operate.

Audience and job: the accountable engineer or team lead opening `wh dash` beside their editor. In under a minute: are the mission's measures moving, are the gates holding, what is the one thing to decide or hand to the coding agent. Coding agents read the same records through `--json`.

Task and content: four views. Dashboard (default; onboarding-only before `wh init`), Foundations (five-stage editable flow: Mission with outcomes, Core values, Key metrics, Rules and guidelines, Gates), Checks (execution only: last run per gate, failures, exact recheck, agent repair brief), Changelog (newest-first, grouped by day and operation, search, as-of, exact records behind disclosure).

Constraints: dependency-free HTML/CSS/JS; no third-party requests, so faces come from the system stack (`ui-sans-serif`/`system-ui` and `ui-monospace`), a pinned product constraint that overrides the own-face rule; startup under 24 KiB in the real product; widths 320/390/768/1024; keyboard, focus, live-region; honest states, never green for unknown; no blended score; no tracker or fleet-console behaviour.

Memorable moment: running checks updates the fixed-column board row by row in place; a failing row opens along one axis into failures, the exact recheck command, and a copyable agent brief.

Unresolved: whether team sharing (push/pull) appears before M2; the epic says only when real.

## Direction contract

THESIS: A quiet engineering journal: the project's records are entries on ruled paper in one calm column, readable in light or dark, with colour spent only on state. It refuses the SaaS dashboard (sidebar of icons, KPI tiles, card grids, shadows, a health score), the graphite console it replaces (panels, seams, tracked uppercase labels, mono as costume), and the cream-paper-with-display-serif template.

OWN-WORLD: Light: paper #fafaf8, well #f0f0ec, rule #e2e1dc, rule-strong #bfbeb8, ink #1e1d1b, ink-soft #5e5c57, ink-faint #6a6963. Dark: paper #181817, well #212120, rule #2d2d2b, rule-strong #484742, ink #e8e6e1, ink-soft #a9a69f, ink-faint #96938c. Tokens on :root, redefined under prefers-color-scheme: dark guarded as :root:not([data-theme="light"]) and again under :root[data-theme="dark"]; a system / light / dark toggle in the header remembers the choice in localStorage. Primary action is ink on paper; accent blue #2b5797 / #8db2ea only for focus, caret and selection; pass green #2e7a3f / #74c682 only for a fresh real pass or an accepted record; ochre #8a5a00 / #d9a94f only for unknown, stale, not run, not observed; red #b3261e / #f0837b only for fail or off target; every text pair in both modes at or above 4.5:1. System serif (ui-serif, New York, Georgia) at 400 for view titles 27px, section titles 19px, day headings 17px; system sans 15px/1.6 for everything else with tabular numerals; mono only for commands, paths, hashes and exact records. All structure drawn with 1px rules (hairline between entries, strong under headings); no boxes, panels or shadows; 4px radius on controls, 6px on recessed wells; drafts and gated tabs marked by dashes; confirmations isolated above their own hairline.

STORY: The owner opens the journal beside their editor, reads the mission line set like a heading in a notebook, sees the single entry that needs them, trusts the gate list because unknown is ochre and never green, hands a failing gate to their agent with one copy, and finds every past decision in the changelog with its exact record one disclosure away, in whichever light the room is in.

FIRST VIEWPORT: A receding demo strip, then a two-line header: serif wordmark and project name at left, agreement state, draft count and the three-button theme toggle at right; beneath it a sticky row of four text tabs (Dashboard, Foundations, Checks, Changelog) on a hairline, the current one in ink with a 1px ink underline. The column is 48rem centred. The mission line as a 27px serif headline with the desired outcome in ink-soft beneath; then Needs attention as a serif heading on a strong rule with the primary item (title, prose, For / Next, one ink action) and the further items as ruled lines with quiet actions; then Key metrics and Gates as ruled lists with state labels flush right; then a single Latest change line. Before init the column holds only the onboarding page: a serif headline, the eight decisions as a numbered ruled list, one ink Establish foundations action and the `wh init` alternative.

FORM: Notebook, owner-pinned on 2026-09-10, replacing Graphite Console.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.