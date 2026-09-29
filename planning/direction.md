# Direction: the delegation plan

Adopted by the owner on 2026-09-27. The work is tracked in Beads under the epic
`whetstone-ppq`, which supersedes `whetstone-k5r`. This file records the
direction and the fixed decisions. It is not a backlog: tasks, order and
acceptance live in Beads.

## Goal

People should be able to hand more work to agents without the usual failures:
work that falls short, errors, missed existing code, unneeded code or tests,
design-system drift, and guessing instead of asking. The solution has to be,
in priority order, simple, customisable, versatile, strong and scalable.

## The plan

1. **Agree the rules once.** `wh init` installs or detects Beads and pstack,
   offers three starter rules, and turns the owner's mission, chosen pstack
   principles and exemplar codebases into proposed rules. The owner accepts
   each one. Accepted rules are Beads records.
2. **Enforce each rule the cheapest reliable way.** Each rule has one
   strength (must, should or advisory) and exactly one enforcer:
   - a mechanical check (native linter, AST, formatter, test, validator,
     drive proof, design tokens, public surface), preferred whenever it can
     hold the rule;
   - a literal yes/no question for Jev (TypeSafe System One), defined by the
     rule's labelled examples and scored by `wh eval`;
   - a review by pstack's `/interrogate` or a named person.
3. **Brief before building, prove before shipping.** Agents run pstack's
   `/how`, `/why` and `/blast-radius` and record the brief. Checks run in
   three layers:
   - git pre-commit: mechanical checks on staged files, within about two
     seconds;
   - pre-push: the full `wh check`, including Jev questions, review
     attestations and fresh proofs;
   - CI: a required status check.

   The Claude Code Stop hook stays as the fast repair loop.
4. **Raise a hand without stalling.** Any of these triggers makes the agent
   file `bd human` and take other ready work:
   - low Jev confidence;
   - a second failed repair on the same flag;
   - a must-rule area;
   - a vague spec.
5. **Get stricter and cheaper over time.**
   - Flags are accepted or dismissed, giving each rule a false-flag rate.
   - Noisy rules are proposed for demotion.
   - Recurring Jev flags are proposed as mechanical checks.
   - `/reflect` and `maintain-verification-skill` edits come back as drafts.

   The owner accepts every change. Nothing learns on its own.

The interactive version of this plan, including the gate simulator and the
comparison with existing approaches, is at
https://claude.ai/artifact/J9Z4jFMRZMizJGVRDHJE2S (private to the owner).

## Fixed decisions

### Public surface
Six commands: `wh init`, `wh dash`, `wh change`, `wh check`, `wh pull`,
`wh push`. The hidden developer gates stay `validate`, `eval` and `scan`. New
behaviour goes into these families, never into new top-level commands.

### Records
Beads is the only record store. Rules change through `wh change` and are
never edited by hand.

### Layers
- The skill owns judgment.
- The driver owns driving and every network call, including Jev.
- The Rust kernel owns validation, state, check execution and receipts, and
  stays deterministic.

### Jev
- Jev can raise a flag but can never pass a must rule. It reports
  `unavailable` when offline or without `TYPESAFE_API_KEY`.
- Receipts pin the model version and carry the question and input digests.
- New Jev rules start in shadow mode, with answers recorded but not
  enforced, until `wh eval` shows enough precision and the owner accepts the
  promotion.

### Data sent to TypeSafe
Owner decision, 2026-09-27: rule and diff text may be sent to TypeSafe.
- Only the unit a question needs is sent (one string, function or hunk).
- Users can redact by path glob or pattern.
- A rule can be marked local-only, which skips Jev and routes the rule to
  review.
- Receipts record when redaction happened.
- API keys never reach records, receipts or logs.

### pstack
Whetstone uses pstack's `how`, `why`, `blast-radius`, `interrogate`,
`reflect` and `maintain-verification-skill` and never reimplements them.
Whetstone owns its verification-skill format. pstack's four-heading layout is
a projection and import format, covered by a round-trip test in `wh eval`.

### Team review
Team review uses the platform's own branch protection plus the required
`wh check` status. Whetstone keeps no second trust root, activation state
machine or authority file.

### Grok Bot
Grok Bot (pstack's `/make-bot-ui`) is optional. Without it, dashboard buttons
queue a Beads task that any agent can pick up.

## Removed from direction

The following are removed from direction, and their code was removed in
`whetstone-ppq.2` (stored records of these kinds are read as retired and can
never be written):

- the eight-decision onboarding interview;
- values, key metrics and outcome observations as record kinds;
- policy exceptions;
- scopes and deployment environments;
- PR-manifest activation (`wh push --propose`, `wh change --activate`,
  `authority.json`);
- the repair-host socket protocol and repair-session flags;
- mandates, and the old M3 and M4 plans.

Existing values and philosophy records become proposed principles. Nothing in
Beads or Git history is rewritten.

## Evidence that it works

| Measure | Target |
| --- | --- |
| Time to first agreed rule | Under five minutes on a clean repo |
| Seeded violations missed | Zero on must rules |
| False flags | Under one per ten commits per rule |
| Consistency across tools | Same verdict on one task in Claude Code, Codex and Cursor |
| Repair turns and raised hands | Tracked per task |
| Jev cost | Tracked per check |

Three milestones in the epic prove the plan in order:

1. solo alpha on this repository;
2. judgment rules;
3. team and tools.

## Still-authoritative references

- [skill-cli-boundary.md](skill-cli-boundary.md): the skill, driver, kernel
  and storage boundary, and the pstack interop contract.
- [direction-demo/index.html](direction-demo/index.html): the dashboard
  reference. Its visual world is current. Its onboarding and Foundations
  content predate this plan and are replaced by task `whetstone-ppq.1.3`.
- `PRODUCT.md` and `DESIGN.md`: product truth and design tokens.
- Superseded history is in [archive/](archive/), including the storage and
  trust-boundary ADRs and the R0 prune inventory.
