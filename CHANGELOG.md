# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This log starts with the rebuilt product. Release notes for the previous
product line (0.1.0 to 0.12.0) are in Git history: `git show 1b7fd8c:CHANGELOG.md`.

## [Unreleased]

Whetstone is rebuilt around six workflows and a verification skill that any
agent can use. Nothing here has been released yet.

### Added

- **Six workflows** with one JSON envelope (`whetstone.command-response.v1`):
  `wh init`, `wh dash`, `wh change`, `wh check`, `wh pull` and `wh push`. Every
  workflow that writes accepts `--dry-run`.
- **A private project agreement.** `wh init` records the owner's eight
  decisions (mission, desired outcome, values, philosophy, owner, first
  safeguard and its scope, revision triggers, first gate) in a private store
  under `.git/whetstone/`. Nothing is committed or shared.
- **A verification skill for any agent.** `wh init --action wire` generates a
  skill, feature map and driver from the accepted records and writes the same
  bytes into `.claude/skills`, `.cursor/skills` or `.agents/skills`. Feature
  files follow pstack's four-section format, so pstack's skills work on them
  unchanged. `wh init --action import` turns edited or pstack-generated skills
  back into drafts.
- **Proof, not claims.** `wh check` runs every gate and records receipts bound
  to their evidence. A pass without evidence is unknown, never green.
  - `--feature <id>` drives one feature; `--step` adds the change's own
    assertions to the receipt.
  - `--changed` proves what a change touched; `--base <revision>` includes
    committed work.
  - `--sweep` drives every feature and reports each as proven, failed,
    unreachable or skipped, with map hygiene findings.
- **A verification driver** (`whetstone/verify/drive.mjs`, Node 22, no
  dependencies) for web, CLI and HTTP apps: isolated launches, doctor checks,
  screenshots and cleanup that keeps evidence.
- **A dashboard** (`wh dash`) with four views: Dashboard (mission, what needs
  attention, metrics, gates), Foundations, Checks (every gate's last run and
  its evidence) and Changelog (searchable decision history). `wh dash --trail`
  exports the decision trail as TSV.
- **Agent hosts.** Claude Code gets hooks: session context at start and failed
  checks handed back to the same session at stop, at most three times. Other
  agents check in with `wh check --changed --host <name>`.
- **Team sync.** `wh push` shares an exact, confirmed package, and private
  drafts never leave the machine. `wh pull` never runs anything it receives.
- **Team activation.** A shared rule becomes binding only through a pull
  request approved by a different account on a protected branch. CI enforces
  activated rules with `wh check --required`, restores tampered checkers from
  their pinned versions, and reports reviewed, expiring exceptions as excepted,
  never passing.
- **Maintenance.** Map drift and hygiene show up as attention items, and
  `wh check --maintain-outcome` records the result of a maintain pass.

### Changed

- Records are stored in Beads (`bd` 1.1.2 or later) instead of Whetstone's own
  Dolt repository. The `dolt` binary is no longer needed.

### Removed

- The previous product line: its terminal UI, packs, legacy commands, Python
  runtime and generated artifacts. See the Git history above for its notes.
