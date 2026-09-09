# Whetstone skill and kernel boundary

This contract prevents judgment from being disguised as enforcement and keeps
the CLI small. Direction 05 is skill-first: the agent interprets intent; the
kernel performs deterministic, replayable work.

## Skill owns judgment

The skill helps people articulate mission, values, engineering philosophy,
outcomes, safeguards, and authority. It proposes changes, identifies conflicts,
selects relevant context, explains rationale, judges source meaning and
currency, and asks an accountable person for consequential decisions.

The skill may recommend native tools, guidance, or structural checks. It must
label semantic and type-aware review as judgment, label missing evidence as
unknown, and never claim that prompt delivery or a green local check proves
enforcement, authorization, or a business outcome.

## Kernel owns deterministic work

The CLI validates typed records, computes content identities, resolves scope and
supersession, renders stable JSON/views, runs structural and native checks,
binds receipts to exact candidate/policy/checker revisions, manages local
transactions, and applies verified synchronization rules. Identical inputs must
produce identical results.

The retained R0 kernel is narrower: typed legacy rule validation, tree-sitter
queries, golden examples, and verification of native lint/formatter/test/
validator bindings. It exists only to preserve a non-vacuous safety bar while
the new record model is built.

## Signal audit

Every proposed safeguard belongs to one bucket:

1. Existing linter or formatter capability: configure and verify the native tool.
2. Taste, semantics, or type resolution: guidance or accountable review, never a deterministic pass.
3. Type-independent structure unavailable natively: a real AST query with known-good and known-bad examples.

Raw regex is not an enforcement strategy. An unavailable check is unavailable,
not passing. All executable validators must be explicitly configured,
repository-contained where required, time-bounded, and treated as untrusted.

## Authority and history

A proposal, acceptance, activation, installation, check receipt, and verdict are
different records. Policy authors cannot self-approve protected activation.
Local preferences cannot weaken required team policy. Every accepted change is
append-only and linked to what it supersedes; corrections are new records, not
rewrites. `wh push` shares selected proposals only, and `wh pull` never executes
or activates incoming content.

## Agent feedback

Applicable rules are selected by scope, not by dumping the whole graph into a
prompt. Failures return to the same worker with stable IDs and repair guidance;
the repaired candidate is checked again. Evidence must expose exact revisions,
age, unavailable checks, and unsupported host capabilities.

The provider-neutral host adapter accepts only opaque authority evidence and
re-verifies the exact task revision, objective, non-goals, guidance, edit scope,
check scope, expiry, and capability set at every checkpoint. Its private Dolt
records persist attempts and elapsed/resource budgets across process restarts,
but those records cannot grant authority. A post-edit hook is one checkpoint
transport, not a separate policy engine. Hosts without that adapter use the
visible `wh check --json` fallback and must report that durable hook/session
support is unavailable. Checking never implies edit, policy, publication,
merge, release, deployment, or outcome authority.

The first transport keeps this lifecycle inside the `check` family: an
authenticated host begins with `--repair-session ... --begin-repair`, invokes
the same family with `--post-edit` for in-session feedback, and supplies
separate opaque completion evidence through `--finalize-with` for the broader
gate. Socket credentials are inherited launch capabilities, never project
configuration or persisted JSON; the exact wire contract is
`references/repair-host-transport.md`.

The first transition and its deterministic task-authority reservation are one
claimed-before-work lifecycle, preventing parallel bootstraps or sessions from
resetting the budget. Each later check/final operation also claims its exact
session revision before execution; interruption stops for accountable recovery
rather than rerunning invisibly. Accepted responses are stored for exact replay
rather than recomputed outside the budget. Final acceptance includes the
complete host-attestation interval and a post-attestation authority/workspace
recheck.
Handoff replies are validated from fresh workspace fingerprints and the stored
reviewed evidence after authority is reauthenticated; they never rerun a check
outside the stopped budget.

## Public product surface

The target has exactly six workflow families: `init`, `dash`, `change`, `check`,
`pull`, and `push`. Bare `wh` is read-only orientation. Stable JSON is the agent
interface; the dashboard and CLI call the same typed services. During R0 these
six workflows remain unavailable rather than being backed by compatibility
aliases.
