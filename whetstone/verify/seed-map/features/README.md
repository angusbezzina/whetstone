# whetstone verification map

This directory is the maintained source for verifying the user-facing behavior of the Whetstone dashboard. Read the index before driving the app, then use the matching feature file as the recipe.

## Baseline preconditions

- Build the binary the driver launches with `cargo build --release`; `node whetstone/verify/drive.mjs doctor --json` reports a stale build otherwise.
- The repository has a private agreement (`wh init`), so the dashboard is established and shows the mission.
- Launch with `node whetstone/verify/drive.mjs launch`; it starts `wh dash --no-open --read-only` on a loopback port of its own.
- Never drive an instance that was not started by this verification run.

## Driving conventions

- Start every recipe from the default view unless its preconditions say otherwise.
- Prefer element ids and ARIA roles (`#tab-foundations`, `role=tab`) over classes, text over coordinates.
- Run actions through `node whetstone/verify/drive.mjs drive <step> ...`; `node whetstone/verify/drive.mjs help` lists the steps.
- Drive read-only: editing needs the one-time edit handoff and is not part of these recipes.

## Proof and skip reporting

- Capture the view after each navigation with a `screenshot` step, and the final state.
- Assert the view's own landmark (a stage, the gate board, a journal entry), not only that a tab was clicked.
- `wh check --feature <id>` stores evidence under the run directory it prints; a pass without evidence is unknown.
- Report an unreachable path with the attempted command and the unmet precondition.

## Feature entry contract

Each feature file starts with an H1 title and one paragraph describing the user-visible behavior. It then uses exactly four H2 sections in this order.

1. `Sub-features` lists short IDs with one line for each behavior.
2. `How to get to it (user POV)` lists every user entry point.
3. `Driving it with <harness>` starts with `Preconditions:` and uses labeled bullets that pair each user action with an exact command and observable result.
4. `Gotchas` lists traps that can waste or invalidate a verification run.

Keep implementation details out of the map. Name only user paths, stable handles, required state, commands, and observable proof.

## Features

- [Dashboard home](./dashboard-home.md) covers the mission headline, what needs attention, key metrics and gates.
- [Foundations](./foundations.md) covers the five stages from mission to gates, then the feature list.
- [Checks](./checks.md) covers the gate board and the last run per gate.
- [Changelog](./changelog.md) covers the journal, search and the decision trail export.
