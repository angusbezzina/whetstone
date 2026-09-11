---
record: "feature.dashboard-home"
area: "Dashboard"
sweep_order: 1
serves: ["mission.project"]
proven_by: ["standard.dashboard-home"]
entry_points: ["assets/dashboard/index.html", "assets/dashboard/app.js", "assets/dashboard/app.css", "src/projection.rs", "src/dashboard.rs", "src/dashboard_service.rs"]
drive_steps: ["open /", "expect #mission-line", "expect text=Needs attention", "expect text=Key metrics", "screenshot home"]
---

# Dashboard home

The default view shows the project's mission as the headline, the one thing that needs attention first, key metrics and gates, so the owner knows in a minute whether the project is on track.

## Sub-features

- `home-mission` shows the mission statement as the headline.
- `home-attention` lists what needs attention with exactly one primary action.
- `home-metrics` shows key metrics and gates with honest states.
- `home-latest` shows the latest change; choosing it opens the Changelog.

## How to get to it (user POV)

- Run `wh dash`; the Dashboard tab is the default view.
- Choose the `Dashboard` tab from any other view.

## Driving it with drive.mjs

Preconditions:

- `node whetstone/verify/drive.mjs doctor --json` reports `"ok": true` for a fresh build.
- The agreement is established (`wh dash --json` shows `data.current.established: true`).

- **Open the dashboard.** Run `node whetstone/verify/drive.mjs drive "open /" "expect #mission-line" --json`. The mission headline is visible.
- **Read attention and measures.** Run `node whetstone/verify/drive.mjs drive "open /" "expect text=Needs attention" "expect text=Key metrics" --json`. Both panels render.
- **Proof.** The mission headline, the attention panel and key metrics render. Run `wh check --feature feature.dashboard-home`. The receipt names the screenshot.

## Gotchas

- Before `wh init` the dashboard shows only onboarding; `#mission-line` does not exist then.
- Metric and gate states are honest: "not observed" and "not run" are expected on a new agreement, never green.
- With nothing needing attention the panel reads "Nothing needs you right now." and has no primary action.
