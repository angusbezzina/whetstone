---
name: whetstone
description: >-
  Establish, inspect, change, check, and selectively share a project's mission,
  engineering agreements, safeguards, and decision history. Use when a user
  wants human and agent work governed by explicit project standards, asks to
  explain why a standard applies, or needs a repair-and-recheck loop.
license: MIT
metadata:
  author: whetstone
  version: "0.5.0-local-alpha"
---

# Whetstone

Whetstone is the judgment layer around a deterministic project-agreement
kernel. Read the project's mission, values, engineering philosophy, relevant
decisions, and evidence; apply only the smallest relevant slice to the work;
then use deterministic checks to close the loop.

## Current implementation boundary

This checkout is a local-alpha foundation, not the complete MVP. All six public
workflow families exist. `init`, `change`, and observational `check` have an
initial local implementation. `dash`, `pull`, and `push` deliberately return
`unavailable`; do not simulate them by editing legacy YAML, copying private
records, or claiming remote/approval effects. Hidden `validate`, `eval`, and
`scan` commands are repository-maintenance gates, not extra product workflows.

For automation, always pass `--json` and consume the versioned
`whetstone.command-response.v1` envelope. Treat nonzero states according to the
returned `state` and `permitted_actions`; never infer success from process
completion alone. Resume `needs_input` with the same request ID, expected
revision, and resume token. A retry with the same request ID must keep the same
input.

Use [planning/direction-demo/index.html](planning/direction-demo/index.html) for
target behavior and
[planning/skill-cli-boundary.md](planning/skill-cli-boundary.md) for the binding
contract. Track implementation through the `whetstone-k5r` Beads epic.

## Target workflow

1. `wh init`: help the user state the mission, values, implementation
   philosophy, outcomes, safeguards, scope, and authority boundaries.
2. `wh dash`: present the current system and complete decision history in a
   lightweight local UI. Read mode is the default; editing is explicit.
3. `wh change`: surface only decisions requiring accountable judgment, record
   rationale and expected effects, and preserve local drafts until accepted.
4. `wh check`: select applicable rules, run native and structural gates, return
   failures to the worker, repair, and recheck the exact candidate.
5. `wh push`: preview and explicitly confirm a selected, private-safe proposal;
   sharing does not approve or activate it.
6. `wh pull`: verify incoming records, preserve drafts, stage conflicts, and
   never execute or activate received content implicitly.

## Judgment rules

- Ask humans only for mission, taste, risk, exception, and consequential tradeoff decisions.
- Prefer five high-confidence safeguards to fifty noisy rules.
- Send native-linter concerns to native linters; do not duplicate them.
- Use AST rules only when the property is structural and type-independent.
- Carry taste or type-aware guidance as guidance; never label it deterministic.
- Cite current primary sources and say when a claim or capability is unknown.
- Keep active context concise and query deeper rationale/history only when relevant.
- Never treat green checks as the business verdict or as authorization to act.

## Feedback loop

Before changing code, load the applicable accepted policy and its exact
revision. After each bounded edit, run the relevant safeguard. On failure,
return rule ID, rationale, location, expected repair, policy/checker revision,
and evidence freshness to the same worker. Recheck after repair. Escalate only
when the conflict requires human judgment; never silently weaken or skip a
mandatory safeguard.
