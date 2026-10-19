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

## While you build

- After each bounded edit, run `wh check --json --changed` (add
  `--base <ref>` to include committed work). Read each finding's rule,
  location, expected result and repair direction, make the smallest repair,
  and recheck.
- With `--hooks` wired, the Git pre-commit and pre-push gates run the same
  rules and block until the must rules hold. If the same finding survives
  two repairs, hand back to the owner.
- Never weaken a check, edit a rule record by hand, or skip hooks. Rule
  changes go through `wh change` as drafts the owner accepts.

## Where records live

Every record lives behind one switch, `whetstone/store.json`: Beads
(standalone), or the attached Virgil repository's `rulebook` Records
(`wh init --action setup --store virgil`), where drafts live in your private
scope, accepted records reach the group when they are accepted, and `wh push`
and `wh pull` are the daemon's. Drafts are never shared until accepted either
way. Flipping to Virgil needs the checkout attached to a Virgil repository
(`virgil repos attach`); when Beads holds records it writes them, outside the
repository, as the file `virgil rulebook import-beads` reads, prints the
dry-run and apply commands, and flips only when a person runs the same command
with `--yes` after the apply: every revision must read back with its digest.
Never run `--yes` or the import yourself; ask the owner.

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
  `--action setup` detects Beads and (with `--yes`) installs it.
- `wh init --action wire --hooks` installs the git pre-commit and pre-push
  hooks. `--ci` adds the required CI status.
- `wh change` edits `whetstone/rulebook.json` (`--kind
  mission|principle|rule`, `--retire <id>`): the edit in the working tree is
  the draft, `--preview` shows it without writing, `--accept` commits the
  file and `--withdraw` restores its last commit. Accepting is the owner's
  call.
- `wh check` runs the rules: `--staged` for pre-commit, `--changed` or
  `--base` for the change. `--sweep` runs only the rules below must and
  prints a JSON array (`--format json` gives the envelope instead), one finding per flag (`fingerprint`, `subjectDigest`, `class`,
  `priority`, `title`, `responsibility`, and `unsure` with a reason where the
  rule doubts its own flag), for a sweep's `findings.json`; a rule's `sweep` class (`draft_pr`,
  `confirm_first`, `discuss`, the default) says how far the sweep may go,
  and only a person widens it. Its `sweep_priority` (0 most urgent to 4,
  default 2) orders the work, and `sweep_unsure` (`[{"paths": [...] or
  "matching": "<regex>", "reason": "..."}]`) parks the doubtful flags until a
  person raises them; a doubt that covers every flag is refused.
- Before building, run pstack's `/how`, `/why` and `/blast-radius`.
- `wh push` previews an exact package of accepted records and needs
  `--confirm <token>`. `wh pull` receives shared records and never executes
  or accepts them.

The earlier direction's `wh push --propose`, `wh change --activate`,
`wh init --ci --reviewer` and repair-session flags have been removed.

## Judgment rules

- Prefer five trusted rules to fifty noisy ones.
- Every rule gets one mechanical enforcer (native linter, formatter, AST,
  test, validator). A concern no check can hold stays guidance,
  not a rule.
- Send native-linter concerns to native linters; do not duplicate them. Use
  AST rules only for structural, type-independent properties. Raw regex is
  not enforcement.
- Another model's opinion can raise a flag. It never passes a must rule.
- Unknown, stale, skipped or unavailable evidence is never success.
- Cite current primary sources and say when a capability is unknown.
- Keep active context short, and query deeper history only when relevant.
- A green check is not the business verdict or permission to act beyond the
  task.
