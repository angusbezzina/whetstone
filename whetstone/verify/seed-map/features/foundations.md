---
record: "feature.foundations"
area: "Dashboard"
sweep_order: 2
serves: ["mission.project"]
proven_by: ["standard.foundations"]
entry_points: ["assets/dashboard/views.js", "assets/dashboard/views.css", "assets/dashboard/edit.js"]
drive_steps: ["open /", "click #tab-foundations", "expect #st-mission", "expect #st-values", "expect #st-metrics", "expect #st-rules", "expect #st-gates", "screenshot foundations"]
---

# Foundations

Foundations shows the agreement as five stages, each justifying the next: mission, core values, key metrics, rules and guidelines, and gates, with every record's version and lifecycle.

## Sub-features

- `stages-order` renders mission, values, metrics, rules and gates in that order.
- `stages-versions` shows each record's version and whether a draft is pending.
- `stages-edit` opens an inline editor that records a private draft (edit mode only).

## How to get to it (user POV)

- Choose the `Foundations` tab.

## Driving it with drive.mjs

Preconditions:

- `node whetstone/verify/drive.mjs doctor --json` reports `"ok": true` for a fresh build.
- The agreement is established.

- **Open Foundations.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-foundations" "expect #st-mission" --json`. The mission stage is visible.
- **See every stage.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-foundations" "expect #st-values" "expect #st-metrics" "expect #st-rules" "expect #st-gates" --json`. All five stages render in order.
- **Proof.** The five stages render in order. Run `wh check --feature feature.foundations`. The receipt names the screenshot.

## Gotchas

- Editing needs the one-time edit handoff; a read-only dashboard shows no editors, which is expected.
- A pending draft never replaces the accepted record on screen; look for "draft vN pending".
