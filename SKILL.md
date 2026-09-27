---
name: whetstone
description: >-
  Agree, check and share a project's rules for delegated work: mission,
  principles, and rules with one strength and one enforcer each. Use when a
  user wants agent work checked against explicit project rules, asks why a
  rule applies, wants to propose or change a rule, or needs a repair-and-recheck
  loop before handing work back.
license: MIT
metadata:
  author: whetstone
  version: "0.7.0-delegation-plan"
---

# Whetstone

Whetstone is the judgment layer around a deterministic rules kernel. Read the
project's mission, principles and the rules that apply to the task, work
within them, and use `wh check` to prove the work before handing it back. The
direction is in [planning/direction.md](planning/direction.md), tracked as the
Beads epic `whetstone-ppq`.

## Before you build

1. Read the applicable rules: `wh dash --json` for mission, principles and
   rules; `wh dash --trail` for the decision log behind them.
2. Brief yourself with pstack when it is installed: `/how` for how the area
   works, `/why` for why it is built that way, `/blast-radius` for what a
   change could break. Name the existing code, components and design tokens
   you will reuse. Do not write a new helper, component or test until you
   have checked that one doesn't already exist and that the task asks for it.
3. Use the generated `verify-<app>` skill to drive the running app when a
   rule or feature needs proof.

## While you build

- After each bounded edit, run `wh check --json --changed` (add
  `--base <ref>` to include committed work). Read each finding's rule,
  location, expected result and repair direction, make the smallest repair,
  and recheck.
- In Claude Code with `--hooks` wired, the Stop hook runs the same check and
  returns violations to you, up to three times. After that, hand back to the
  owner. Other tools: run the checkpoint yourself before handing back.
- Never weaken a check, edit a rule record by hand, or skip hooks. Rule
  changes go through `wh change` as drafts the owner accepts.

## Raise a hand

Stop and ask instead of guessing when:
- a check is unknown or unavailable for a must rule;
- the same finding survives a second repair;
- the task touches an area a must rule reserves for the owner, such as a
  public API;
- the spec is too vague to satisfy the rules.

File the question as a Beads issue labelled `human` (`bd create --labels
human ...`; the owner sees it in `bd human list`). Include what you were asked,
what you tried, what you recommend, and the one decision needed. Then take other ready
work (`bd ready`). A second agent may advise but cannot decide for the owner.

## Using the CLI

For automation, always pass `--json` and consume the versioned
`whetstone.command-response.v1` envelope. Treat the returned `state` and
`permitted_actions` as the answer, and never infer success from process exit
alone. Resume `needs_input` with the same request ID, expected revision and
resume token. A retry with the same request ID must keep the same input.

- `wh init --json` only inspects, and is always read-only. It resolves the Git
  worktree root from nested directories and reports detected facts separately
  from inferences and unknowns. Proposed defaults are suggestions, not
  answers.
- `wh init --action agree` records the owner's agreement. It still asks for
  the earlier eight decisions until `whetstone-ppq.2.4` replaces them with
  mission, principles and proposed rules, so pass only answers the owner
  gave you.
- `wh init --action wire --host <host> --hooks` writes the skill, feature map,
  driver and Claude Code hooks. `wh init --action import --from <dir>` turns
  pstack-edited skill files into drafts.
- `wh change` drafts, previews, accepts, withdraws or retires records.
  Accepting is the owner's call.
- `wh check` runs the rules: `--changed` or `--base` for the change,
  `--feature` and `--step` to prove one feature, `--sweep` for every mapped
  feature.
- `wh push` previews an exact package of accepted records and needs
  `--confirm <token>`. `wh pull` receives shared records and never executes
  or accepts them.
- Hidden `validate`, `eval` and `scan` are repository gates, not product
  workflows.

Do not use `wh push --propose`, `wh change --activate`, `wh init --ci
--reviewer` or the `wh check` repair-session flags. They belong to the
earlier direction and are being removed.

## Judgment rules

- Prefer five trusted rules to fifty noisy ones.
- Every rule gets one enforcer. Use a mechanical check (native linter, AST,
  test, drive proof) when one can hold the rule. Otherwise use a literal
  yes/no question with examples. Use review only as the last resort.
- Send native-linter concerns to native linters; do not duplicate them. Use
  AST rules only for structural, type-independent properties. Raw regex is
  not enforcement.
- A judgment answer (a Jev question or another model's opinion) can raise a
  flag. It never passes a must rule.
- Unknown, stale, skipped or unavailable evidence is never success.
- Cite current primary sources and say when a capability is unknown.
- Keep active context short, and query deeper history only when relevant.
- A green check is not the business verdict or permission to act beyond the
  task.
