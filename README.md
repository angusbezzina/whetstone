# Whetstone

Whetstone is becoming a lightweight, local-first system for keeping human and
agent work aligned with a project's mission, engineering values, and approved
ways of working. It should turn durable agreements into applicable context,
deterministic safeguards, repair feedback, and an inspectable decision history.

The product is being rebuilt from a deliberately small foundation. The old
dependency-extraction application has been removed; its last complete revision
is `1b7fd8c341b8a5aaea742c564092fbcca26b51bb` and its last release is v0.12.0.
Project data under `whetstone/`, Beads history, and Git history were preserved.

## Current state: local-alpha foundation

The current binary is not yet the complete MVP. It does expose the six stable
workflow families and a versioned JSON envelope. `init`, `change`, and the
observational `check` have an initial local implementation; `dash`, `pull`, and
`push` return `unavailable` until their implementation milestones pass. Bare
`wh` is a read-only orientation and never starts a TUI.

```bash
wh init --json
wh change --json
wh check --json --path src
wh dash --json       # unavailable until M1.10
wh pull --json       # unavailable until M2.1
wh push --json       # unavailable until M2.1
```

Machine clients consume `whetstone.command-response.v1`; its strict schema is
in `references/command-response-v1.schema.json`. Noninteractive requests never
prompt. They return explicit `needs_input`, `needs_decision`, `stale`,
`conflict`, `unknown`, or `unavailable` states with permitted next actions.

Three hidden developer gates keep the surviving deterministic kernel
non-vacuous while the rest of the MVP is built:

```bash
cargo run --quiet --release -- validate
cargo run --quiet --release -- eval
cargo run --quiet --release -- scan src --lang rust --json --no-fail
```

The kernel provides versioned agreement records, separate private/shareable
local Dolt stores, typed rule validation, tree-sitter checks, golden-example
evaluation, and verification of native lint, formatter, test, and validator
bindings. Public `wh check` cannot execute command validators until an exact
checker is trusted through the later execution-adapter milestone.

## Target product

The public surface contains six workflow families:

| Workflow | Purpose |
| --- | --- |
| `wh init` | Establish mission, values, engineering philosophy, safeguards, and authority. |
| `wh dash` | Inspect and edit the current system, evidence, and complete decision history. |
| `wh change` | Propose and govern a deliberate change without silently weakening policy. |
| `wh check` | Return actionable failures to the worker, repair, recheck, and record evidence. |
| `wh pull` | Receive verified shared changes without destroying local drafts. |
| `wh push` | Optionally share an explicitly selected, private-safe proposal for review. |

The canonical interactive specification is the standalone
[Direction 05 demo](planning/direction-demo/index.html). The implementation is
tracked in Beads under `whetstone-k5r`; the prune boundary and recovery evidence
are in [the R0 inventory](planning/direction-demo/prune-inventory.md).

## Product invariants

- Human-readable records and stable JSON must describe the same source of truth.
- Guidance, deterministic checks, evidence, and accountable human verdicts stay distinct.
- Failures return to the worker in a check, repair, recheck loop.
- Unknown, stale, skipped, or unavailable required evidence cannot become success.
- Local drafts and personal preferences remain private until explicitly selected for sharing.
- Team policy needs independent review and protected activation; code and policy cannot approve themselves.
- Every consequential decision remains inspectable from project start, including rationale and supersession.
- Existing trackers, repository controls, and native linters remain the execution and enforcement authorities.

## Build the foundation

```bash
cargo build --release
./target/release/whetstone
```

There is no published Direction 05 release yet. The existing v0.12.0 release is
the recoverable legacy product, not this foundation. The retained release
installer is tested against the locally built binary; future releases will
place one `whetstone` binary, expose `wh` as a symlink, verify SHA-256 checksums,
and fail closed when verification is unavailable.

## Contributing

Use Beads for work tracking and read [AGENTS.md](AGENTS.md) before changing the
project. All eight local gates in that file must pass before pushing.
