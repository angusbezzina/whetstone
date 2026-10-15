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

File the question with `wh check --raise-hand` (below): standalone it is a
Beads issue labelled `human` (the owner sees it in `bd human list`); with the
memory switch naming a Virgil group it is a task in the group's channel. Include what you were asked,
what you tried, what you recommend, and the one decision needed. Then take other ready
work (`bd ready`). A second agent may advise but cannot decide for the owner.

## Remember what you learn

When you learn something the next agent or person should know (a convention
the code does not show, a trap, why something is the way it is), propose it
as a lesson: `wh change --json --kind lesson --content "<the lesson in
words>" --definition '{"type":"lesson","about":[{"kind":"path","path":"src/x.rs"}],"concepts":["..."]}'`.
Name the files it depends on in `about` (or `anchors`): Whetstone hashes them
and flags the lesson as possibly stale when they change. Never supply who
wrote it, trust, status or a decision: the store stamps provenance, and a
request carrying them is refused.

You propose; a person admits. `wh change --admit` from an agent is refused
by design. Correct a lesson with `wh change --supersede <key> --content
"..."` and propose forgetting one with `wh change --forget <key> --rationale
"..."`; both wait for a person. Admitted lessons arrive at session start;
treat one marked "may be stale" as a lead to check, never as a rule. A
lesson that keeps holding can become a rule only as a draft in shadow:
`wh change --kind rule --from-lesson <key> ...`.

Where admitted lessons live is one switch, `whetstone/memory.json`: Whetstone's
own Beads store (standalone), or a Virgil group's lessons (`wh init --action
memory --store virgil --group <ULID>`). Proposals stay in Whetstone's store
either way; inside Virgil a person's admit is their `virgil lessons
remember`, Virgil derives trust from the writer (`--trust` is refused there),
and a correction rewrites the lesson in place. Flipping to Virgil needs the
checkout attached to a Virgil repository (`virgil repos attach`); when
Whetstone's store holds admitted lessons it writes them, outside the
repository, as the file `virgil lessons import-beads` reads, prints the dry-run
and apply commands, and flips only when a person runs the same command with
`--yes` after the apply: every key must read back, and anchored lessons are
re-anchored. Never run `--yes` or the import yourself; raise a hand.

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
- `wh init --action agree` records the owner's agreement: `--mission`,
  optional `--principle <pstack id>` and `--custom-principle`, and
  `--starter <id|all>`. Pass only answers the owner gave you.
  `--action setup` detects and (with `--yes`) installs Beads and pstack;
  `--action exemplar --from <path>` turns a codebase the owner admires into
  rule drafts.
- `wh init --action wire --host <claude|codex|cursor|agents> --hooks` writes
  the skill (with `rules/*.md` and `whetstone.verify.json`), feature map,
  driver, the agent hooks and the git pre-commit and pre-push hooks. `--ci`
  adds the required CI status. `wh init --action import --from <dir>` turns
  pstack-edited skill files, including rule files, into drafts.
- `wh change` drafts, previews, accepts, withdraws or retires records
  (`--kind mission|principle|rule|feature|map`), and remembers lessons
  (`--kind lesson`; `--admit`, `--supersede` and `--forget` are a person's). `--accept-flag` and
  `--dismiss-flag` label a flag, `--answer <issue>` answers a raised hand, and
  `--tune` and `--migrate` record drafts. Accepting is the owner's call.
- `wh check` runs the rules: `--staged` for pre-commit, `--changed` or
  `--base` for the change, `--feature` and `--step` to prove one feature,
  `--sweep` for every mapped feature, `--mutate` to test the proofs.
  `--sweep --format findings` runs only the rules below must and prints a
  JSON array, one finding per flag (`fingerprint`, `subjectDigest`, `class`,
  `title`, `responsibility`), for a night sweep's `findings.json`; a rule's `sweep` class (`draft_pr`,
  `confirm_first`, `discuss`, the default) says how far the sweep may go,
  and only a person widens it.
- Before building, run pstack's `/how`, `/why` and `/blast-radius` and record
  them with `wh check --brief --area <feature|path> --skill how --skill why`.
- When confidence is low, a repair fails twice, the spec is vague or the area
  is under a must rule, raise a hand and move on to other ready work:
  `wh check --raise-hand --question "..." --tried "..." --recommend "..."
  --trigger <low-confidence|second-failed-repair|must-rule-area|vague-spec>`.
- A review rule passes only with an attestation at this commit:
  `wh check --attest <rule> --verdict pass|fail --reviewer interrogate
  --notes "..."`.
- `wh push` previews an exact package of accepted records and needs
  `--confirm <token>`. `wh pull` receives shared records and never executes
  or accepts them.
- Hidden `validate`, `eval` and `scan` are repository gates, not product
  workflows.

The earlier direction's `wh push --propose`, `wh change --activate`,
`wh init --ci --reviewer` and repair-session flags have been removed.

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
