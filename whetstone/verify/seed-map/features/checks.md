---
record: "feature.checks"
area: "Dashboard"
sweep_order: 3
serves: ["mission.project"]
proven_by: ["standard.checks"]
entry_points: ["assets/dashboard/views.js", "src/gates.rs", "src/service.rs", "assets/dashboard/index.html", "assets/dashboard/app.js", "src/projection.rs"]
drive_steps: ["open /", "click #tab-checks", "expect #checks .board", "expect #checks .brow", "screenshot checks", "click #checks [aria-controls^=det-][aria-expanded=false]", "expect #checks .bdetail.open", "screenshot checks-detail"]
---

# Checks

Checks shows the last run of every gate, what failed and where, the exact recheck command and the repair brief to hand to the current agent.

## Sub-features

- `checks-board` lists every gate with its strength, mechanism and command, last run and result.
- `checks-detail` opens a gate's result or failures, recheck command and, for a failing gate, the repair brief.

## How to get to it (user POV)

- Choose the `Checks` tab.
- Choose `Open checks` on a never-run or stale gate item under Needs attention on the Dashboard; it opens the Checks board at that gate. A failing gate's item says `Open the failing gate` instead.

## Driving it with drive.mjs

Preconditions:

- `node whetstone/verify/drive.mjs doctor --json` reports `"ok": true` for a fresh build.
- At least one gate is in force (the first gate from `wh init`).

- **Open Checks.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-checks" "expect #checks .board" --json`. The gate board renders.
- **Find a gate row.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-checks" "expect #checks .brow" --json`. At least one gate row is present.
- **Open a gate's detail.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-checks" "click #checks [aria-controls^=det-][aria-expanded=false]" "expect #checks .bdetail.open" "inspect checks-detail" --json`. The detail shows the result or failures, the recheck command and, after a driven run, its evidence. The row itself is not a control; its details button is.
- **Proof.** The gate board shows at least one gate row. Run `wh check --feature feature.checks`. The receipt names the screenshot.

## Gotchas

- A gate that never ran shows "not run", never pass.
- A failing gate's detail is already open; the details step opens the first closed one.
- Running checks from the dashboard needs edit mode; read-only drives only inspect.
