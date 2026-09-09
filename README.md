# Whetstone

Whetstone is becoming a lightweight, local-first system for keeping human and
agent work aligned with a project's mission, engineering values, and approved
ways of working. It should turn durable agreements into applicable context,
deterministic safeguards, repair feedback, and an inspectable decision history.

The product is being rebuilt from a deliberately small foundation. The old
dependency-extraction application has been removed; its last complete revision
is `1b7fd8c341b8a5aaea742c564092fbcca26b51bb` and its last release is v0.12.0.
Project data under `whetstone/`, Beads history, and Git history were preserved.

## Current state: R0 foundation

The current binary is not yet the new MVP. Bare `wh` reports that state
honestly. Three hidden developer gates keep the surviving deterministic kernel
non-vacuous while the new record model is built:

```bash
cargo run --quiet --release -- validate
cargo run --quiet --release -- eval
cargo run --quiet --release -- scan src --lang rust --json --no-fail
```

The kernel currently provides typed rule validation, tree-sitter checks, golden
example evaluation, and verification of native lint, formatter, test, and
validator bindings. It does not expose legacy product commands or pretend the
target workflows exist.

## Target product

The public surface will contain six workflow families:

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

## Build and install the foundation

```bash
cargo build --release
./target/release/whetstone
```

To exercise the release installer against a published version:

```bash
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh
```

The installer places one `whetstone` binary and creates `wh` as a symlink. It
verifies published SHA-256 checksums and fails closed when verification is not
available.

## Contributing

Use Beads for work tracking and read [AGENTS.md](AGENTS.md) before changing the
project. All eight local gates in that file must pass before pushing.
