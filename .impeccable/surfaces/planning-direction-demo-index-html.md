---
version: 1
slug: "planning-direction-demo-index-html"
primary_target: "planning/direction-demo/index.html"
related_targets: ["assets/dashboard/index.html"]
---

# Surface brief: Whetstone dashboard direction reference

Scope: `planning/direction-demo/index.html`, the self-contained behavioural and visual reference for the `wh dash` local UI (implemented in `assets/dashboard/`). Visitor mode: Operate.

Audience and job: the accountable engineer or team lead opening `wh dash` beside their editor. In under a minute: are the mission's measures moving, are the gates holding, what is the one thing to decide or hand to the coding agent. Coding agents read the same records through `--json`.

Task and content: four views. Dashboard (default; onboarding-only before `wh init`), Foundations (five-stage editable flow: Mission with outcomes, Core values, Key metrics, Rules and guidelines, Gates), Checks (execution only: last run per gate, failures, exact recheck, agent repair brief), Changelog (newest-first, grouped by day and operation, search, as-of, exact records behind disclosure).

Constraints: dependency-free HTML/CSS/JS; no third-party requests, so faces come from the system stack (a condensed system face where the OS has one, `system-ui` otherwise, `ui-monospace` for code), a pinned product constraint that overrides the own-face rule; startup under 24 KiB in the real product; widths 320/390/768/1024; keyboard, focus, live-region; honest states, never a lit pass for unknown; no blended score; no tracker or fleet-console behaviour. Owner-pinned 2026-09-14: black and orange, white and orange in light mode, ShadCN token vocabulary, the logo's stone and WHET/STONE wordmark, a minimal personal-journal feel.

Memorable moment: running checks strikes each row in place as its result lands; a failing row is struck orange with a halo that decays over one beat, and opens in one step into failures, the exact recheck command, and a copyable agent brief.

Unresolved: whether team sharing (push/pull) appears before M2; the epic says only when real.

## Direction contract

THESIS: A struck cathode gauze: every option and every record is present at once as a quiet ghost on an unframed black plane, and the one thing that is live, the selected view, the item that needs you, a failing gate, the primary action, is struck forward in the logo's orange with a halo. No box, rule or divider anywhere; grouping is spacing and depth. It refuses the SaaS card grid, the ruled paper journal it replaces, and the neon-console costume.

OWN-WORLD: ShadCN tokens. Dark: background #0a0b0e, foreground bone #eae6dd, muted ground #15171c, muted-foreground #a19d94, primary #e66c3a with primary-foreground #0a0b0e, halo orange at 45%, a fine bronze gauze in the margins only. Light: background #ffffff, foreground #0a0a0a, muted #efeeea, muted-foreground #616161, primary #e66c3a for fills and marks, #c2410c for orange text, no halo. States are marks, not hues: pass is a lit foreground disc, fail an orange cross, unknown and stale a ghost ring, draft a dashed ghost ring. Condensed system display face for the view title, mission, section and day headings, tallies and the tracked WHET/STONE wordmark; system sans body at 15px/1.6 with tabular numerals; mono only for commands, paths, hashes, records. Radius 6px; no borders; inputs are recessed muted fields with an orange caret and a struck ring on focus.

STORY: The owner opens the plane beside their editor, sees the one struck item and knows it is the thing that needs them, trusts the gate list because a pass is lit and everything unmeasured stays a ghost, hands a struck failing gate to their agent with one copy, and finds every decision in the changelog as a ghost that lights when opened, in whichever light the room is in.

FIRST VIEWPORT: A one-line header: the stone mark and the tracked WHET/STONE wordmark (STONE in orange), the project name in ghost, then at right agreement state, drafts and the three-button theme toggle with only the chosen one struck. Beneath, four view names on one line, the current one struck orange, the rest ghosts; no underline, no rule. The column is 48rem centred on the clean plane; the gauze runs in the margins past every edge. The mission as a condensed display title in foreground with its outcome in ghost beneath; then Needs attention, with the primary item struck: title lit, prose bone, For / Next in ghost, one orange primary action with a halo; then Key metrics and Gates as spaced lines with marks flush right; then a single Latest change line. Before init the column holds only the onboarding page: display headline, the eight decisions as spaced ghost lines that light as recorded, one struck Establish foundations action and the `wh init` alternative.

FORM: Struck Cathode Gauze, catalog id operate-b-struck-cathode-gauze, a competitive challenger the owner chose over the assigned Drawing Sheet in round 3 of seed d4a17bcf; kept raises: only the chosen one is struck; colour only at marks and edges; reveals deploy in one step; a running row pulses on its own beat.

FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance.
