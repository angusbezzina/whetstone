# The record store: Beads or Virgil, behind one switch

Drafted 2026-10-16 for the owner's review. It amends one fixed decision in
`planning/direction.md` ("Beads is the only record store") and is tracked in
Beads under the epic named at the end. It is not a backlog: tasks, order and
acceptance live in Beads.

## Problem

Every Whetstone record (rule, principle, proposal, review, decision, receipt)
is a bead. `src/storage.rs` re-exports `beads::RecordStore`, a concrete struct;
`wh push` and `wh pull` wrap `bd dolt push` and `bd dolt pull`; `wh init`
installs `bd` when it is missing. Lessons alone have a switch
(`whetstone/memory.json`, `beads` or `virgil`), and raised hands follow it.

A team that runs Whetstone inside Virgil therefore keeps two stores for one
contract: lessons and raised hands in the group's Records, every rule in a
Beads database the team is retiring (Virgil cut its plans over to native tasks
on 2026-10-15). `wh push` there writes rule revisions nobody will read.

## Decision proposed

**Whetstone has two record stores, Beads and Virgil, chosen once per
repository.** The choice covers every record kind together: rules, principles,
proposals, reviews, decisions, receipts, lessons and raised hands. Nothing is
half in one store.

- **Beads** is the standalone store: no network, no Virgil, `bd` 1.1.2 or
  later. It stays exactly as `planning/skill-cli-boundary.md` describes it.
- **Virgil** is the store for a repository attached to a Virgil group
  (`.virgil/repo.toml`): records live in the group's Records, reached only
  through the `virgil` CLI on the daemon socket, the way `virgil lessons`
  works today. The daemon stamps provenance; sync is the daemon's.

Not in direction: Linear, Jira, GitHub Issues or any tracker as a record
store. A rule revision is an immutable, digested, superseding record read on
every `wh check`; a tracker holds tickets a person acts on. The one
ticket-shaped thing Whetstone emits, a raised hand, already goes to the
tracker the store implies (`bd human`, `virgil tasks create`). A third store
is added by implementing the port below and adding one `wh init` option;
nothing else changes.

## Design

### One port, two adapters

`src/storage.rs` defines `trait RecordStore` with the surface the service
layer uses today (`all_records`, `get`, `by_idempotency_key`, `latest`,
`history`, `incomplete_batches`, `append`, `append_batch`,
`sync_lifecycle_labels`, `copy_to`, and the private/shareable `kind`).
`src/beads.rs` becomes the Beads adapter, unchanged in behaviour.
`src/virgil_store.rs` (or `src/store/virgil.rs`) is the Virgil adapter. The
domain and agreement layers stay storage-agnostic, as the 2026-09-10 decision
already requires; the kernel keeps in-force resolution, digests and the
draft-never-displaces-accepted invariant, whichever adapter is underneath.

`LessonStore` (`src/memory/store.rs`) stays as it is: it is the lesson
contract over whichever record store is chosen, and `BeadsLessons` and the
Virgil lessons backend already exist.

### The switch

`whetstone/memory.json` becomes `whetstone/store.json`:

```json
{ "store": "beads" }
{ "store": "virgil" }
```

Committed with the repository so the whole team reads one store. The Virgil
arm names no group: the scope is the attachment found from the checkout, as
`virgil lessons` resolves it today (the `group` field in `memory.json` was
needed before `virgil lessons` dropped `--group`; it is dropped here).
`memory.json` is read as `store.json` for one release, with a `wh dash`
notice, and `wh init --action memory` becomes `wh init --action store`.

### What each arm does

| | Beads | Virgil |
| --- | --- | --- |
| Rules, decisions, receipts | shared `.beads` database; drafts and receipts in the private store under `.git/whetstone` | the group's Records `agreement` collection; drafts stay private (see below) |
| Lessons | private Beads store; admitted group lessons travel with `wh push` | Records `lesson`, through `virgil lessons` (unchanged) |
| Raised hands | `bd human` | `virgil tasks create` (unchanged) |
| `wh push` / `wh pull` | `bd dolt push` / `bd dolt pull` | refused by name: "records sync through the daemon"; `--dry-run` says so and exits 0 |
| `wh init` | detects or installs `bd` | detects the attachment and the daemon socket; never installs `bd` |

**Drafts stay private in both arms.** A proposal or a receipt nobody has
accepted never leaves the machine today (the private Beads store under
`.git/whetstone`). Under Virgil the private store is still Beads-free: a
file-backed private store under `.git/whetstone/private/` with the same
digest and idempotency rules, so `bd` is not a requirement of the Virgil arm.
Accepted records and decisions go to Records; receipts a check produces go to
Records only when the rule they attest is shared, as `wh push` decides today.

### Virgil's side

Virgil adds an `agreement` Records collection with the shape the Beads
metadata carries now: `wh_record` (canonical JSON, ASCII-escaped), `revision`,
`digest`, `supersedes`, `idempotency_key`, `lifecycle`, keyed by the record id
and revision, scoped to the repository attachment like `lesson`. The commands
are `virgil agreements <put|get|latest|history|list|import-beads> --input -`,
one JSON request in, one JSON line out, Virgil's exit codes mapped to
Whetstone's refusals as `src/memory/virgil.rs` maps them now. Whetstone pins
`virgil.agreements/v1` beside `virgil.lessons/v1`. That is Virgil work, filed
in Virgil, and the Whetstone adapter is tested against `tests/fake_virgil.py`
extended with the new verbs until the real daemon is attached.

### Migration

`wh init --action store --store virgil` on a repository with Beads records:

1. exports the shared store's accepted records and decisions in the shape
   `virgil agreements import-beads` reads (`src/memory/export.rs` already
   does this for lessons; the same builder, one more kind);
2. prints the `virgil agreements import-beads` and `virgil lessons
   import-beads` commands;
3. flips the switch only on `--yes`, after reading every exported record back
   from Records and comparing digests.

Flipping migrates nothing by itself, and `wh dash` reports records left in the
other store, as it does for lessons today. Nothing in Beads or Git history is
rewritten.

### Not changing

The public surface stays six commands. Rules still change only through
`wh change`. Jev, the enforcement ladder, gates and pstack interop are
untouched. The `dolt` binary is still not a requirement.

## Evidence that it works

| Measure | Target |
| --- | --- |
| Beads arm | every existing test passes unchanged against the adapter |
| Virgil arm | the same service tests pass against the Virgil adapter over the fake, and `whetstone-jpd.13`'s real-daemon check extends to agreements |
| Switch | a repository flipped to Virgil runs `wh check` with no `bd` on PATH |
| Migration | round trip: export, import, read back, digests equal, for every record kind |
| Virgil repo | `wh push` there is refused, and no rule revision lands in `.beads` after the flip |

## Settled while building the Whetstone side (2026-10-17)

These follow from the design above; the owner confirms them with the
amended decision.

- **Sharing under Virgil happens at acceptance.** With `wh push` refused, the
  accepted package `wh push` would build (accepted revisions with the
  proposals and reviews that accepted them; never receipts, draft ancestry or
  lessons) is copied into Records after every `wh init` and `wh change`.
  It is idempotent; a failure leaves the records on the machine, says so,
  and the next `wh change` shares them.
- **A legacy `memory.json` naming Virgil moves lessons only.** Reading it as
  a full `store.json` would hide every rule from `wh check` until a
  migration ran. Its records stay in Beads, `wh dash` says so, and
  `wh init --action store --store virgil` moves them.
- **The contract Whetstone pins** (`virgil.agreements/v1`, in
  `src/store/virgil.rs` and `tests/fake_virgil.py`): `put` (with
  `expected_revision`, or `copy` for exact revisions), `get`, `latest`,
  `history`, `list` (paged, filterable by idempotency key), `mark` and
  `markers` (batch completion markers), `lifecycle` (the lifecycle view) and
  `import-beads` (`{"file","apply"}`), all on `--input -`. `mark`, `markers`
  and `lifecycle` are additions to the verbs listed above; the Virgil plan
  should carry them or say what replaces them.
- **No canaries on the Virgil arm.** `wh push --canary` has no equivalent
  when sharing happens at acceptance; private text is kept out by keeping it
  out of accepted records (or out of the rule with `privacy`).
- **Copies only go into a shareable store.** The port's `copy_to` refuses a
  private destination as well as the same store; no caller ever copied into
  a private store, so the Beads arm is unchanged.
- **Migration refuses collisions and needs a person.** An accepted private
  revision whose id and revision the shared Beads database holds with other
  content is named and the flip refused until it is rebased; confirming
  with `--yes` is a person's, as it moves every record the team reads. The
  export carries each revision's resolved lifecycle view.
- **Migration also moves the private store.** After the read-back, the
  private Beads store's revisions (drafts, receipts) are copied exactly into
  the private file store, so nothing local is left behind.

Still open: the Virgil-side collection and verbs, the staging-daemon checks,
the direction documents (after the owner's written confirmation) and the
Virgil repository's own flip. A CI runner under the Virgil store needs a
daemon to read Records from; until one exists there, keep CI repositories on
Beads.

## Order

1. The port: `trait RecordStore`, Beads behind it, all tests green (no
   behaviour change).
2. The switch: `store.json`, `memory.json` read-through, `wh init --action
   store`, `wh push`/`wh pull` refusal under Virgil, `wh dash` reporting.
3. The private file store for drafts and receipts.
4. Virgil: the `agreement` collection and `virgil agreements` (filed in the
   Virgil repository; lands before 5 is verified against a real daemon).
5. The Virgil adapter over the fake, then over the daemon.
6. Migration: export, import, read-back, the flip.
7. Direction: amend `planning/direction.md` "Records", `AGENTS.md`,
   `CLAUDE.md`, `README.md` "Storage" and `planning/skill-cli-boundary.md`
   "Storage" and "Memory"; flip the Virgil repository.

Epic: `whetstone-l6i`.
