---
record: "feature.checks"
area: "Dashboard"
sweep_order: 3
serves: ["mission.project"]
proven_by: ["standard.checks"]
entry_points: ["assets/dashboard/views.js", "src/gates.rs", "src/service.rs"]
drive_steps: ["open /", "click #tab-checks", "expect #checks .board", "expect #checks .brow", "screenshot checks"]
---

# Checks

Checks shows the last run of every gate, what failed and where, the exact recheck command and the repair brief to hand to the current agent.

## Sub-features

- `checks-board` lists every gate with mechanism, scope, last run and result.
- `checks-detail` opens a gate's failures, recheck command and repair brief.

## How to get to it (user POV)

- Choose the `Checks` tab.
- Choose a failing-gate item under Needs attention on the Dashboard.

## Driving it with drive.mjs

Preconditions:

- `node whetstone/verify/drive.mjs doctor --json` reports `"ok": true` for a fresh build.
- At least one gate is in force (the first gate from `wh init`).

- **Open Checks.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-checks" "expect #checks .board" --json`. The gate board renders.
- **Find a gate row.** Run `node whetstone/verify/drive.mjs drive "open /" "click #tab-checks" "expect #checks .brow" --json`. At least one gate row is present.
- **Proof.** The gate board shows at least one gate row. Run `wh check --feature feature.checks`. The receipt names the screenshot.

## Gotchas

- A gate that never ran shows "not run", never pass.
- Running checks from the dashboard needs edit mode; read-only drives only inspect.
