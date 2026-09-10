# ADR-0001: direct Dolt storage with physically separate private and shareable histories

- Status: superseded on 2026-09-10 (see below); kept as the M1 record
- Date: 2026-09-09
- Accepted baseline: `2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48`
- Beads task: `whetstone-k5r.2`
- Decision owner: Angus Bezzina

## Superseded 2026-09-10: Beads-backed storage

The owner reversed this decision in epic `whetstone-k5r` (section "Beads
storage and pstack interop", task `whetstone-k5r.17`). Whetstone's own Dolt
repositories and the standalone `dolt` binary are removed. Records become
`record` beads in Beads (`bd` 1.1.x, embedded Dolt): typed body, revision,
digest, supersedes and idempotency key in JSON metadata, lifecycle as a label,
proposals and acceptances as `decision` beads, receipts as ephemeral `receipt`
beads. Private drafts live in a second Beads database under
`.git/whetstone/private` that never gets a remote; `wh push` copies selected
accepted records into the repository's shared Beads database and calls
`bd dolt push`.

Why the reversal: the concerns below about a task-shaped interface were
re-tested on 2026-09-10. Custom types (`types.custom`), arbitrary JSON metadata
round-tripping byte-for-byte, lifecycle labels and per-record history
(`bd history`) all work through the public CLI. The owner's requirements
changed the weighting: no policy in Git commits, one dependency the team
already runs, and sync (`bd dolt push`/`pull`, federation, `bd bootstrap`)
that already exists. Accepted costs: no SQL in embedded mode (each read or
write is one `bd` call), sequential multi-record writes with idempotency keys
instead of one transaction, integrity by digest and history rather than
database permissions, and a pinned minimum `bd` version. The original text
follows unchanged.

## Decision (superseded)

Whetstone will own its records in direct Dolt repositories behind a small typed `StorageRepository` interface. Beads remains the project's issue tracker and optional future event source; Whetstone will not read or write Beads' private tables, store policy as issues or memories, or depend on Beads retention and compaction behavior.

Private and shareable state are different physical Dolt repositories with unrelated commit graphs. A share operation constructs a new, allowlisted projection in the shareable staging repository. It never clones, branches, filters, or pushes the private repository. This invariant is structural: neither current rows nor old commits in the shareable store may contain private content or ancestry.

The first implementation will pin standalone Dolt `2.2.3`, the exact version tested here, and disable Dolt event flushing in child processes. Whetstone will refuse unsupported or unverified versions. It will manage one project-local server when concurrent writers are required and use the MySQL protocol through a typed adapter; storage callers will never concatenate untrusted SQL. Solo/offline maintenance may use the same repository without the server, but only after acquiring the repository lock, and the service must not create two sources of truth.

Initial remote support is deliberately narrow:

- Local filesystem remotes are proven and supported for tests, backup, and recovery.
- A credentialed team remote is not claimed by this spike. M2 must prove the selected transport, authentication, stale-base handling, and private-history exclusion before `wh push` is available.
- Credentials never enter Dolt rows, receipts, logs, environment snapshots, or Whetstone configuration. A future remote adapter may consume an existing platform credential helper or narrowly scoped process environment at call time.
- Remote pull conflicts remain explicit unresolved state. Whetstone must not choose ours/theirs automatically for policy or decision records.

## Why direct Dolt, not Beads-backed storage

The installed Beads `1.1.2` successfully created and read an isolated issue through its public CLI, but that interface is task-shaped rather than an arbitrary durable-record API. Its database schema is private implementation detail, embedded mode is single-writer, and its task lifecycle includes archival/compaction concepts that Whetstone cannot inherit for permanent decisions. The installed CLI also reported Dolt auto-commit as off by default in its help, while current upstream documentation describes defaults that vary by mode; Whetstone must own and test its commit policy instead of inheriting a drifting tracker default.

Beads is still valuable as a later integration boundary: Whetstone may consume stable exported task/event identifiers and emit proposals through documented commands. That dependency remains optional and replaceable. There is no shared database, schema, process lifecycle, retention clock, or authority model.

Direct Dolt adds an external runtime and is not “zero dependency.” The measured cost is acceptable against the five-minute onboarding and two-second warm-check budgets, while preserving versioned SQL, branches, cell-aware merges, full backups, and offline reads. SQLite plus an append-only event log would be the fallback if Dolt installation or supported-platform coverage fails later; Git-native files were rejected because they cannot provide the required local draft/share staging separation and concurrent structured history without rebuilding database semantics.

## Executable spike

The spike used fresh data only under `/private/tmp/whetstone-dolt-spike.Kpp89P`; it did not open, migrate, or delete the real `.beads` or `whetstone` stores. Commands were run on a MacBook Pro (Mac16,6, Apple M4 Max, 16 cores, 48 GB) with macOS Darwin 25.5.0, standalone Dolt `2.2.3`, and Beads `1.1.2`.

### Dataset and schema migration

The private repository held 1,000 shared decisions and one private canary across four commits. The schema was migrated in place from v1 to v2 by adding `scope` and `proposal_digest`; decision 500 moved from revision 1 to 2. A full backup restored 1,001 rows, all four reachable commits, the private canary, and both the v1 and v2 values through `AS OF` queries. `dolt fsck` reported no problems.

The independent shareable repository was created from an allowlisted projection of only the 1,000 shared records. It had its own initialization and projection commits. Three independent checks found zero canary occurrences:

```sql
SELECT COUNT(*) FROM records
WHERE body LIKE '%WH_PRIVATE_CANARY%';

SELECT COUNT(*) FROM dolt_history_records
WHERE body LIKE '%WH_PRIVATE_CANARY%';
```

A binary scan of the entire shareable repository also found no canary bytes. The shareable repository was 68 KiB; the private 1,001-record repository was 168 KiB; its full backup was 60 KiB and restored repository 88 KiB.

### Concurrency, durability, conflicts, and lifecycle

- Two concurrent SQL-server sessions inserted and Dolt-committed 500 unique rows each into one table; the final grouped count was 500 for each writer and four total commits.
- The server was killed with `SIGKILL`. A serverless offline query immediately recovered all 1,000 rows and four commits, and a restarted server returned the same counts.
- Starting a second server on the occupied loopback port failed explicitly with `Port 34119 already in use`; it did not attach to or mutate the wrong repository.
- Two clones changed the same policy cell from the same base. After client A pushed, client B's pull stopped with one unresolved row in `dolt_conflicts_policies`, preserving base, ours, and theirs. No data was silently selected.
- `dolt backup sync` to a filesystem backup and `dolt backup restore` into a fresh directory preserved the commit graph and historical values, not just a current-row export.
- Offline reads against 1,001 records took 0.07 seconds in both measured cold and immediate repeat CLI runs.
- A bounded server start/readiness-query/graceful-stop lifecycle took 0.24 seconds. Three warm server queries took 0.04, 0.03, and 0.03 seconds.
- The warmed server used 86,000 KiB RSS; an earlier sample after concurrent writes used 96,592 KiB RSS. The 1,000-row server repository occupied 84 KiB. This process cost must remain visible in `wh dash`/status diagnostics.
- Fresh installation is one pinned Dolt binary (`brew install dolt` was used here) plus Whetstone. M1 must provide platform-specific checksum verification and a clear missing-version error; it must not silently download or execute a different version.

## Repository interface and invariants

M1 may change the internal client library, but callers receive this semantic interface:

```text
StorageRepository
  begin(expected_head) -> Transaction
  read(record_id, as_of?) -> VersionedRecord?
  query(scope, applicability, as_of?) -> [VersionedRecord]
  append(record, expected_previous_revision) -> VersionedRecord
  commit(message, actor_evidence) -> CommitId
  history(record_id) -> [VersionedRecord]
  export_projection(allowlist, destination_store) -> ProjectionReceipt
  backup(destination) -> BackupReceipt
  verify() -> StorageHealth
```

Required invariants:

1. Every successful logical write ends in both an atomic SQL transaction and a named Dolt commit before success is reported.
2. `expected_head` and record revision provide optimistic concurrency; stale writes fail with structured conflict state.
3. Migrations are ordered, checksummed records applied transactionally before normal access. Newer unknown schemas fail closed; downgrade is read-only recovery only.
4. Decision and activation records are append-only. Corrections supersede; they do not rewrite or compact historical decisions.
5. Private and shareable repository roots, remotes, backups, locks, server identities, and encryption/access policy are independent.
6. Projection starts from an explicit allowlist and new destination history, then scans current rows, Dolt history, refs, and repository bytes for private canaries before a share receipt can succeed.
7. Server ownership is proven by repository identity plus a task-owned lock, not merely a PID or open port. Unknown/foreign processes are never killed.
8. Remote, backup, and restore are explicit operations. Local commits are not proof of remote durability.

## Operational limits

- Dolt data is plaintext and inherits local-user filesystem authority. Whetstone cannot sandbox another process running as the same OS user; private-store confidentiality requires directory permissions and a separate shareable store.
- Loopback is not authentication. The production local server should prefer a per-project Unix socket where the selected Rust MySQL client supports it; otherwise it needs generated credentials, restrictive files, explicit host binding, and a random verified port. The spike's unauthenticated loopback server is evidence for behavior, not a production security configuration.
- The server warned that empty `secure_file_priv` lets a FILE-privileged SQL user read process-accessible files. M1 must set a non-existent or task-owned import directory and avoid granting FILE.
- Dolt has two persistence layers: SQL transaction commit and Dolt history commit. Success requires both. Working-set-only writes are recoverable but do not satisfy Whetstone durability or receipt requirements.
- Current Beads documentation pins standalone Dolt `2.2.0` and records a `2.3.x` hard-reset regression. Whetstone tested `2.2.3`; version advancement requires the same crash/reset/restore/concurrency suite, not “latest.”
- The tested filesystem remote has no authentication and proves mechanics only. Hosted, Git-SSH, cloud, and TLS transports remain unsupported until their own negative tests pass.
- Backups are point-in-time and explicit. Retention defaults to no history rewriting; backup rotation and destructive garbage collection require a later owner-approved policy.

## Recovery and rollback

On startup, verify the repository with a read-only health query and compare the schema migration ledger. On corruption or an unknown schema, stop writes, preserve the directory, and restore into a new path from the latest full Dolt backup. Verify record count, commit graph, current head, historical sentinel values, and private/shareable classification before atomically switching the configured root. Never restore a private backup into the shareable root.

If the direct Dolt approach later fails installation, platform, or resource budgets, keep the `StorageRepository` contract and migrate through a typed read-only exporter plus a separately verified importer. Do not make Beads internals the emergency fallback.

## Sources checked on 2026-09-09

- [Dolt 2.2.3 release](https://github.com/dolthub/dolt/releases/tag/v2.2.3)
- [Dolt SQL transaction support and two persistence layers](https://www.dolthub.com/docs/sql-reference/sql-support/supported-statements/)
- [Dolt merge and conflict behavior](https://www.dolthub.com/docs/sql-reference/version-control/merges/)
- [Dolt conflict model](https://www.dolthub.com/docs/concepts/dolt/git/conflicts/)
- [Dolt system history and conflict tables](https://www.dolthub.com/docs/sql-reference/version-control/dolt-system-tables/)
- [Dolt remotes](https://www.dolthub.com/docs/concepts/dolt/git/remotes/)
- [Dolt backups](https://www.dolthub.com/docs/sql-reference/server/backups/)
- [Beads Dolt architecture, version pin, modes, backup, and remote lifecycle](https://github.com/gastownhall/beads/blob/main/docs/architecture/dolt.md)
- [Beads configuration and Dolt commit policy](https://github.com/gastownhall/beads/blob/main/docs/CONFIG.md)
- [Beads security and storage limitations](https://github.com/gastownhall/beads/blob/main/SECURITY.md)

No source older than the current 2026 product line was used as implementation authority. The installed `bd backup --help`, `dolt sql-server --help`, `dolt remote --help`, and `dolt backup --help` were treated as the authoritative local-version contract where upstream prose differed.
