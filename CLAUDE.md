# Claude Code instructions for Whetstone

Follow [AGENTS.md](AGENTS.md) completely. The plan of record is the Beads epic
whetstone-sol (rules as files). Read [planning/direction.md](planning/direction.md)
before planning.

In short:

- Whetstone is a repository's rulebook: `whetstone/rulebook.json`, committed
  in the repository. A revision is a commit, the decision log is Git, and a
  rule change is a pull request.
- Each rule has one strength (must blocks, should reports) and one
  mechanical enforcer (ast, lint, formatter, test, validator), plus its sweep
  fields.
- Checks run at pre-commit, at pre-push and as the required CI status
  `whetstone/policy`. `wh check --sweep` prints `findings.json` for any host.
- The public surface is `wh init`, `wh dash`, `wh change` and `wh check`.
- Whetstone keeps no store and no host-specific code. Beads tracks this
  project's tasks only.

Do not restore removed modules, commands, fixtures or compatibility paths for
convenience. Preserve `whetstone/rulebook.json`, `.beads/` and
`.git/info/exclude`. Use `planning/skill-cli-boundary.md` for the boundary
between Whetstone and the hosts that consume it, and `DESIGN.md` for the
dashboard's visual world. Run the three gates in AGENTS.md before every push.
