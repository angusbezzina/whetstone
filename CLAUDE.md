# Claude Code instructions for Whetstone

Follow [AGENTS.md](AGENTS.md) completely. Whetstone is in the prune-first R0
foundation of the `whetstone-k5r` implementation epic; the old product is not a
template for the new one.

Do not restore removed modules, commands, packs, generated artifacts, fixtures,
or compatibility paths for convenience. The last old-product revision is
`1b7fd8c341b8a5aaea742c564092fbcca26b51bb` and may be inspected read-only.
Preserve all project data under `whetstone/`, `.beads/`, and `.git/info/exclude`.

The target public surface is `wh init`, `wh dash`, `wh change`, `wh check`,
`wh pull`, and `wh push`; none exists until its Beads acceptance criteria pass.
Current hidden developer gates are `validate`, `eval`, and `scan`.

Use `planning/direction-demo/index.html` for behavior,
`planning/skill-cli-boundary.md` for judgment versus determinism, and
`planning/direction-demo/prune-inventory.md` for the R0 allowlist and recovery
record. Use Beads for every implementation task and run all eight gates before
every push.
