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

## Public product surface

The target has exactly six workflow families: `init`, `dash`, `change`, `check`,
`pull`, and `push`. Bare `wh` is read-only orientation. Stable JSON is the agent
interface; the dashboard and CLI call the same typed services. During R0 these
six workflows remain unavailable rather than being backed by compatibility
aliases.
