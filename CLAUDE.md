# Claude Code instructions for Whetstone

Follow [AGENTS.md](AGENTS.md) completely. The plan of record is the Beads epic
`whetstone-ppq` (the delegation plan, adopted 2026-09-27), which supersedes
`whetstone-k5r`. Read [planning/direction.md](planning/direction.md) and the
epic description before planning.

In short:

- Rules have one strength (must, should or advisory) and one enforcer:
  mechanical first, then a Jev question, then review.
- Jev runs in the driver. It never passes a must rule and starts in shadow
  mode.
- Checks run at pre-commit, at pre-push and as a required CI status. The Stop
  hook is only the repair loop.
- Whetstone uses pstack's judgment skills and never reimplements them.
- Beads is the only record store.

Do not extend the removed direction: the eight-decision onboarding, values and
metrics, exceptions, scopes, PR-manifest activation, repair-host sockets and
mandates. Their code was deleted in `whetstone-ppq.2`.

Do not restore removed modules, commands, packs, generated artifacts, fixtures,
or compatibility paths for convenience. The last old-product revision is
`1b7fd8c341b8a5aaea742c564092fbcca26b51bb` and may be inspected read-only.
Preserve all project data under `whetstone/`, `.beads/`, and `.git/info/exclude`.

The public surface is `wh init`, `wh dash`, `wh change`, `wh check`,
`wh pull`, and `wh push`. The hidden developer gates are `validate`, `eval`,
and `scan`. Milestone acceptance is tracked by the exits in `whetstone-ppq.10`.

Use `planning/skill-cli-boundary.md` for the skill, driver, kernel and storage
boundary and the pstack interop contract. Use
`planning/direction-demo/index.html` for the dashboard's visual world. Use Beads
for every implementation task and run all eight gates before every push.
