---
record: "feature.foundations"
area: "Dashboard"
sweep_order: 2
serves: ["mission.project"]
proven_by: ["standard.foundations"]
entry_points: ["assets/dashboard/views.js", "assets/dashboard/views.css", "assets/dashboard/edit.js", "assets/dashboard/index.html", "assets/dashboard/app.js", "src/projection/mod.rs"]
drive_steps: ["open /", "click #tab-rules", "expect text=Mission", "expect text=Principles", "expect text=Must", "expect text=Features", "screenshot rules"]
---

# Foundations

The Rules tab shows the agreement the agents work under: the mission, the principles, then every rule grouped by strength (must, should, advisory) with its one enforcer, where it runs, whether it is in shadow, its false-flag record and its examples. A Features panel follows the rules.

## Sub-features

- `rules-mission` shows the mission and its version.
- `rules-principles` lists the principles, from pstack or the owner's own.
- `rules-by-strength` groups rules as must, should and advisory, each with its enforcer family and mechanism.
- `rules-features` lists every mapped feature after the rules, with its state, version and runbook.
- `rules-edit` opens an inline editor that records a private draft (edit mode only).

## How to get to it (user POV)

- Choose the `Rules` tab.
- Choose a Needs attention action on Home that belongs to a rule; it opens the Rules tab at that rule.

## Driving it with drive.mjs

Preconditions:

- `node whetstone/verify/drive.mjs doctor --json` reports `"ok": true` for a fresh build.
- The agreement is established, with at least one must rule.

- **Open Rules.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-rules" "expect text=Mission" --json`. The mission section is visible.
- **See every section.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-rules" "expect text=Principles" "expect text=Must" "expect text=Features" --json`. Mission, principles, the strength groups and features render in order.
- **Proof.** The sections render in order. Run `wh check --feature feature.foundations`. The receipt names the screenshot.

## Gotchas

- Editing needs the one-time edit handoff; a read-only dashboard shows no editors, which is expected.
- A pending draft never replaces the accepted record on screen; look for "draft vN pending".
- A shadow question rule is shown with a shadow badge: its answers are recorded, not enforced.
