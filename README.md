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
observational `check` have an initial local implementation; `dash` exposes the
same service through a lightweight local UI. `pull` and `push` return
`unavailable` until their implementation milestones pass. Bare `wh` is a
read-only orientation and never starts a TUI.

```bash
wh init --json
wh change --json
wh change --json --record-id guidance.example   # bind an exact base
wh check --json --path src
wh dash --json       # inspect locally; editing requires explicit edit mode
wh dash --json --search "architecture" --as-of 2026-09-09T12:00:00Z
wh pull --json       # unavailable until M2.1
wh push --json       # unavailable until M2.1
```

Machine clients consume `whetstone.command-response.v1`; its strict schema is
in `references/command-response-v1.schema.json`. Noninteractive requests never
prompt. They return explicit `needs_input`, `needs_decision`, `stale`,
`conflict`, `unknown`, or `unavailable` states with permitted next actions.
The first `wh init --json` only inspects and previews its private footprint. An
explicit, resumable `--action agree` supplies mission, desired outcome, values,
philosophy, owner, initial safeguard and scope, and revision triggers; it writes
those records atomically without editing project files or creating shared
state. Setup remains incomplete until the bounded repair loop has a current
known-bad to known-good proof for the exact code snapshot.

Interactive `wh dash` serves a dependency-free local UI with four views.
Dashboard shows mission-linked metrics, gate status, what needs attention and
the latest change, or only the onboarding path before `wh init`. Foundations is
an editable, versioned flow from mission and values to key metrics, rules and
guidelines, and gates; edits become governed local drafts with exact
before/after review. Checks shows the last run per gate, failures, the exact
recheck route and the repair brief for the current agent. Changelog is the
complete append-only history, newest first, with exact records behind
disclosure.
The standalone Direction 05 one-pager is a planning artifact, not a product
view. Inspection is credential-free on the loopback listener; mutation
requires the one-time browser handoff and a second explicit edit-mode step.
Search and as-of queries execute through the same typed history service as
`wh dash --json`; they never replace the unfiltered current-state projection.
Foundation editing resumes from persisted owner decisions. Change forms bind a
base before authoring, request a typed server-side before/after preview, then
freeze the exact request and capabilities before confirmation. Pull and push
remain explicitly unavailable in the CLI and service; they are never simulated.

Three hidden developer gates keep the surviving deterministic kernel
non-vacuous while the rest of the MVP is built:

```bash
cargo run --quiet --release -- validate
cargo run --quiet --release -- eval
cargo run --quiet --release -- scan src --lang rust --json --no-fail
```

The kernel provides versioned agreement records, in-force versus draft
resolution, typed rule validation, tree-sitter checks, golden-example
evaluation, bounded gate execution with per-gate receipts and evidence, and a
generated verification skill, feature map and driver (`wh init --action wire`).

Records live in Beads (`bd` 1.1.2 or later, embedded Dolt; no `dolt` binary).
Each record revision is a bead whose metadata carries the typed body, digest,
supersedes reference and idempotency key; lifecycle is a `wh:lifecycle:*`
label. Drafts, receipts and personal experiments stay in a private Beads
database under `.git/whetstone/` that never has a remote. `wh push` shares an
exact, confirmed package of accepted records into the repository's shared
Beads database and runs `bd dolt push`; `wh pull` runs `bd dolt pull` and never
executes, activates or accepts what arrives. Nothing about policy is committed
to the code branch: the team remote carries Beads data in `refs/dolt/data`, and
a fresh clone reaches it with `bd bootstrap` and `wh dash`.

Install requirements: the `wh` binary, `bd` 1.1.2 or later, Node 22 for the
verification driver, and Chrome or Chromium for web surfaces.

The generated `verify-<app>` skill is a valid
[pstack](https://github.com/cursor/plugins/tree/main/pstack) verification
skill: the same four sections per feature file and README shape, so pstack's
`maintain-verification-skill` runs on it unchanged, and
`wh init --action import --from <dir>` reads a pstack-generated skill (or a
maintained one) back as reviewable drafts. Whetstone adds the versioned "why"
behind each feature and the receipts that prove it; pstack supplies the
judgment playbooks. `wh dash --trail` exports the decision history as a
show-me-your-work TSV.

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
