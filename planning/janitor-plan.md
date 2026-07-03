# Janitor — moved

The Janitor plan bootstrapped into its own project on 2026-07-03, per the
plan's own note ("this doc moves at bootstrap").

- **Repo:** `~/Development/janitor` (remote to be added by the owner)
- **Plan:** `PLAN.md` there (authoritative; this copy is retired)
- **Work:** beads epic `janitor-ygz` — 12 dependency-ordered children; V1 exit
  test: *one clean night on a real repo, repeatably*

Whetstone's side of the contract is unchanged: Janitor consumes `wh scan`
(queue), `wh eval` (rule quarantine), `wh ci` (drift), and `wh status`
(digest metrics) as deterministic oracles — see `planning/skill-cli-boundary.md`
for why orchestration lives outside this binary.
