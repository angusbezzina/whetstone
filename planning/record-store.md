# The record store: Beads or Virgil, behind one switch

Drafted 2026-10-16, re-cut 2026-10-17 against the narrowing (`whetstone-e8n`)
and the owner's decision of 2026-10-17: the Virgil store lands before
Whetstone is onboarded into Virgil. It amends one fixed decision ("Beads is
the only record store") as the follow-on the Whetstone outline names: Virgil
retired its Beads database (its `.beads` directory was dropped and production
release 12 retired the Beads host), so the rulebook has no shared store to
ride there. Tasks, order and acceptance live in Beads under `whetstone-l6i`.

## What a rulebook stores after the narrowing

Mission, Principle and Rule records, and the decision log: Proposal,
LocalReview and Retirement, plus Retired for kinds that no longer exist. Each
revision is one record with a canonical-JSON body, a digest, a supersedes
reference and an idempotency key; lifecycle is resolved by the kernel.
Drafts live in a private store on the machine; accepted records and their
decisions live in the shared store, which `wh push` and `wh pull` move
between teammates. No receipts, lessons or raised hands remain.

## Decision

**Two record stores, Beads and Virgil, chosen once per repository in
`whetstone/store.json`.** The choice covers every record kind and both the
private and the shared store. Beads stays exactly as it is for a repository
on its own. Virgil is the store for a repository attached to a Virgil group
(`.virgil/repo.toml`): records live in the group's Records, reached only
through the `virgil` CLI on the daemon socket; the daemon stamps provenance
and syncs. `bd` is not a requirement of the Virgil arm.

Not in direction: a tracker (Linear, Jira, GitHub Issues) as a record store.
A rule revision is an immutable, digested, superseding record read on every
`wh check`; a tracker holds tickets. A third store later is one adapter
behind the port below plus one `wh init` option.

## Design

### One port, two adapters

`src/storage.rs` defines `trait RecordStore` with the surface the service
layer uses after the narrowing (`all_records`, `get`, `by_idempotency_key`,
`latest`, `history`, `incomplete_batches`, `append`, `append_batch`,
`sync_lifecycle_labels`, `copy_to`, `kind`). Sync (`push_remote`,
`pull_remote`) is an optional capability of an adapter, not part of the
port, so an adapter can refuse it by name. `src/beads.rs` becomes the Beads
adapter with no behaviour change. `src/store/virgil.rs` is the Virgil
adapter. A conformance suite runs any adapter through the storage
invariants. Domain, agreement and history layers import nothing from either
adapter.

### The switch

`whetstone/store.json`, committed with the repository:

```json
{ "store": "beads" }
{ "store": "virgil" }
```

The Virgil arm names no group: the scope is the attachment found from the
checkout. Absent file means Beads, so no existing repository changes.

| | Beads | Virgil |
| --- | --- | --- |
| Private store (drafts) | `.git/whetstone` Beads database, no remote | the `rulebook` collection in private scope: owned by the signed-in principal, never visible to the group |
| Shared store (accepted records, decisions) | the repository's `.beads` database | the `rulebook` collection scoped to the repository attachment |
| `wh push` / `wh pull` | `bd dolt push` / `bd dolt pull` | refused by name ("records sync through the daemon; nothing to push"); `--dry-run` says so and exits 0 |
| `wh init --action setup` | installs and pins `bd`, writes hooks and CI scaffold | requires the attachment and a reachable daemon; writes hooks and CI scaffold; never installs `bd` |
| `wh dash` | names the store | names the store and the attachment |

Drafts in private scope, not a file store: Virgil Records already has a
private scope (ownership from the authenticated principal, as
`personal_lesson` used it), so one adapter serves both store kinds and no
third store is written. `copy_to` from private to shared is the accept path;
the reverse is refused.

### Virgil's side

A `rulebook` Records collection in `packages/virgil-schema`, registered in
both the repository-attachment scope and private scope: fields `wh_record`
(canonical JSON, ASCII-escaped string), `record_id`, `revision`, `digest`,
`supersedes`, `idempotency_key`, `kind`, `type`, `lifecycle`; keyed by
`record_id` and `revision`; `idempotency_key` unique per scope. A private
row belongs to the signed-in principal and to the repository it was written
from, and never reaches the group. CLI `virgil rulebook
put|get|latest|history|list|lifecycle|mark|markers|import-beads --input -`:
one JSON request in, one JSON line out, `api_version` `virgil.rulebook/v1`,
Virgil's exit codes, and every request names its `"scope"` (`repository` or
`private`; there is no default). `mark` and `markers` hold batch completion
markers per scope; `lifecycle` updates the lifecycle view of one revision.
`src/store/virgil.rs` documents each verb's request and answer, and
`tests/fake_virgil.py` implements them; the Virgil side builds to that
contract. This is a records schema addition, so the door is newer than the
clients: name `writersFrom`, run `task records:released:check`, release the
door to staging before the adapter is verified against it. Planned and cut
in the Virgil repository; tracked here as a dependency.

### Migration

`wh init --action setup --store virgil` (or `--action store`, which skips
the tool setup) on a repository that already holds
Beads records exports the shared store's accepted records and decisions in
the shape `virgil rulebook import-beads` reads, prints the import command,
and flips the switch only on `--yes` after reading every record back from
Records and comparing digests, then copies the private Beads store into the
private scope. Flipping migrates nothing by itself; `wh
dash` reports records left in the other store. Nothing in Beads or Git
history is rewritten. The Virgil repository itself has no Beads records to
migrate: its init on the Virgil store is a fresh rulebook.

### Not changing

Six public commands. Rules change only through `wh change` and the
dashboard. The rulebook shape, the gate, the sweep and `findings.json` are
untouched. The `dolt` binary is still not a requirement.

## Order

After `whetstone-e8n` lands (it removes lessons, `memory.json`, receipts and
hands, and settles the record kinds):

1. the port and the conformance suite (no behaviour change);
2. the switch: `store.json`, `--store` on setup, push/pull refusal, dash;
3. Virgil: the `rulebook` collection and `virgil rulebook` (Virgil repo),
   and the fake extended with the verbs;
4. the Virgil adapter, over the fake then a staging daemon;
5. migration: export, `import-beads`, read-back, the flip;
6. direction documents amended (after `whetstone-e8n.11` rewrites them), and
   the Virgil repository initialised on the Virgil store, which is the
   hand-off to Whetstone's onboarding into Virgil (init with Angus and
   James, the rules, the sweep workflow, Messo's scan).

## Evidence that it works

| Measure | Target |
| --- | --- |
| Beads arm | every existing test passes unchanged against the adapter |
| Virgil arm | the conformance suite passes against the Virgil adapter over the fake and the real-daemon check passes on staging |
| Switch | a repository on the Virgil store runs `wh check --sweep` and a draft `wh change` with no `bd` on PATH and no `.beads` directory |
| Migration | round trip for every record kind: digests equal after read-back |
| Virgil repository | `whetstone/store.json` says `virgil`, `wh push` there is refused, no `.beads` directory returns |
