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
  version: "0.6.0-local-alpha"
---

# Whetstone

Whetstone is the judgment layer around a deterministic project-agreement
kernel. Read the project's mission, values, engineering philosophy, relevant
decisions, and evidence; apply only the smallest relevant slice to the work;
then use deterministic checks to close the loop.

## Current implementation boundary

This checkout is an evolving local alpha, not the complete MVP. All six public
workflow families exist. `init`, `change`, observational `check`, and the local
inspect-first `dash` have initial implementations. `pull` and `push`
deliberately return `unavailable`; do not simulate them by editing legacy YAML,
copying private records, or claiming remote/approval effects. Hidden `validate`,
`eval`, and `scan` commands are repository-maintenance gates, not extra product
workflows.

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

Before changing code, record the user's task, non-goals, allowed paths and the
applicable active policy/checker snapshot. Missing edit authority is a blocker,
not an invitation to ask another agent to act. A task's edit permission never
includes policy, check, baseline, publication, merge, release or deployment.

After each bounded edit, run `wh check --json` on the relevant scope. Read its
stable finding ID, rationale, location, observed and expected result, repair
direction, snapshot digests and permitted action. Apply the smallest repair
with the current worker's ordinary editing tools, then recheck the exact changed
snapshot. A per-edit hook may accelerate this; the explicit checkpoint is the
portable fallback and final verification is always broader than the edit hook.

Persist the repair session and increment its attempt budget before another edit.
Do not reset the budget after a restart. Stop on cancellation, missing authority,
scope growth, unavailable required evidence, repeated no-progress, oscillation,
or the configured attempt limit. Group repeated findings that require the same
owner decision into one handoff containing the reviewed snapshot, stable IDs,
one question, recommendation, alternatives, impact, evidence and the only
permitted next step. A second agent may advise but cannot approve for the owner.

### Host capability boundary

The provider-neutral repair host API authenticates an opaque task grant, binds
the objective, non-goals, applicable guidance, allowed and excluded paths,
required checks, authority revision and expiry, and stores its counters in the
private Dolt history. Persisted records are observations, never authority. A
supported host calls the post-edit checkpoint and returns its `RepairFeedback`
to the same worker in-session. That feedback can authorize another source edit;
it never authorizes policy/check/baseline changes, publication, merge, release,
deployment, or a business outcome claim.

The initial provider-neutral adapter uses an owner-only Unix socket outside the
project plus a per-launch secret inherited from the host; neither credential is
stored. The host injects inert task context and begins through `wh check
--repair-session <id> --authority-evidence <locator> --begin-repair`. Whetstone
persists nothing until the socket authenticates that exact context. The begin
response returns structured callback fields; a hook-capable host invokes the
same `wh check` family with the returned revision and `--post-edit`, then sends
the JSON response back to the active worker. A green scoped checkpoint still
requires `--finalize-with <opaque-acceptance-locator>` for broader verification.
See `references/repair-host-transport.md` for the bounded wire contract.

Session creation atomically reserves one durable budget for the authenticated
project/task/authority revision. Exact lost-response retries return the stored
check response without rerunning work. Broader completion rechecks authority,
budget, and the candidate workspace after host attestation returns; a change,
expiry, or revocation becomes an owner handoff rather than verified success.

If the current host has not integrated that API, say so and use
`wh check --json --path <scope>` as the explicit visible checkpoint after each
edit. This fallback provides the same deterministic finding details to humans
and agents, but it does not provide durable host-managed repair budgets. Do not
claim that a hook or durable session is installed. Onboarding may offer and
prove an adapter later; until then, final task-scope verification remains
mandatory and edit permission must be granted separately from checking.
