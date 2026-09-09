# Repair host transport v1

This is the first provider-independent transport for a host that wants repair
feedback returned to the same worker after an edit. It carries authority to the
typed `RepairHost`; it is not a second policy engine and does not let `wh check`
mint edit permission.

## Launch boundary

The host creates a Unix-domain socket outside the project tree. The socket and
its parent directory must be owned by the project directory's OS owner; the
directory must have mode `0700` and the socket must grant no group or other
access. The host launches Whetstone with:

- `WHETSTONE_REPAIR_AUTHORITY_SOCKET`: absolute socket path.
- `WHETSTONE_REPAIR_AUTHORITY_SECRET`: an unguessable, per-launch value of at
  least 32 non-whitespace bytes.
- `WHETSTONE_REPAIR_LAUNCH`: for session creation only, a bounded JSON object
  containing `task`, `authority_revision`, and the typed `RepairTaskContext`.
  This is inert input: the socket must authenticate the exact object before it
  can be persisted.

Neither value is a project setting. Whetstone never prints or persists them.
The opaque evidence locator supplied to a repair call is an identifier, not a
credential: it has no authority without a successful exchange on this channel.
No adapter, invalid permissions, timeout, malformed response, stale binding, or
denial fails closed before a repair-session mutation.

The private store also holds one permanent, deterministic reservation for each
project/task/authority revision. Whetstone claims it before bootstrap checks,
then atomically binds it to the first session revision, so concurrent process
starts cannot execute parallel bootstraps or mint two independent attempt
budgets. It is operational state, cannot be projected to the shareable store,
and never grants authority. An interrupted bootstrap remains visibly claimed;
the owner must investigate and issue a new authority revision rather than
silently replaying potentially spent work.

The MVP acknowledges the trust limit in ADR-0002: a hostile process running as
the same OS user can inspect another cooperating process's launch environment
on some platforms. Stronger process isolation can replace this transport
without changing `RepairHost`.

## Framing and bounds

Each connection carries one UTF-8 JSON object followed by `\n`, then one UTF-8
JSON response followed by `\n`. A request or response at or above 256 KiB is
rejected. Connect, read, and write operations are bounded to five seconds. All
objects reject unknown fields.

Authority requests use `whetstone.repair-authority-request.v1`:

```json
{
  "schema": "whetstone.repair-authority-request.v1",
  "request_id": "authority-<digest>",
  "one_time_secret": "<launch capability>",
  "evidence_locator": "<opaque host locator>",
  "target": {
    "session_id": "repair-42",
    "task": { "system": "beads", "stable_id": "T-42" },
    "project": "<Whetstone project identity>",
    "authority_revision": 7,
    "context": "<typed RepairTaskContext object>",
    "now_unix": 1788955200
  }
}
```

Completion requests use `whetstone.repair-completion-request.v1`. Their target
binds `session_id`, task, project, authority revision, candidate-workspace
digest, and the complete final check snapshot. Task acceptance and required
review are therefore observations about the exact candidate, not free-form
agent claims.

The host MUST mint completion evidence from a trusted acceptance runner that
independently verifies the exact target, preferably in a clean or isolated
environment. It MUST NOT translate an agent's claimed success into a grant or
reuse evidence from a different workspace digest, snapshot, task, authority
revision, or environment. The Unix-socket adapter authenticates this host
attestation; it does not make an untrusted worker process into that runner.
After attestation returns, Whetstone re-authenticates the standing grant,
re-captures the workspace, and charges the complete elapsed interval and one
resource unit before it can record `verified`.

Responses use the matching `whetstone.repair-authority-response.v1` or
`whetstone.repair-completion-response.v1` schema, echo the exact `request_id`,
and set `state` to `granted`, `denied`, or `unavailable`. Only `granted` carries
a grant. The adapter converts the private wire grant into a non-deserializable
`VerifiedRepairAuthority` or `VerifiedRepairCompletion`; `RepairHost` then
revalidates every target field and capability.

Ordinary repair authority must set only `may_edit_source: true`. Policy, check,
baseline, push, merge, and release capabilities must remain false. A green
scoped checkpoint only advances to broader final verification. Local task
verification does not assert a business outcome and does not authorize merge
or release.

## Host flow

1. The accountable host injects launch context and invokes `wh check
   --repair-session <id> --authority-evidence <locator> --begin-repair`.
   `RepairHost::begin` persists nothing unless the socket authenticates the
   exact task, scope, checks, expiry, and budget.
2. A hook-capable host invokes the existing `wh check` repair-checkpoint mode
   after an edit and returns the resulting `RepairFeedback` to that same worker.
3. A host without post-edit callbacks presents the same command as a visible
   explicit checkpoint. Missing adapter authority is reported as unavailable;
   ordinary observational `wh check` remains usable but grants nothing.
4. Once the scoped checkpoint is green, the host supplies separately verified
   task-acceptance/review evidence for broader final verification.
5. Cancellation, stale revision, scope change, unavailable evidence,
   no-progress, oscillation, or exhausted persistent budget stops the loop and
   produces a bounded owner handoff.

An exact retry returns the response stored with the accepted transition. It
does not rerun a checker or spend an unrecorded resource unit, and a retry that
changes hook-versus-checkpoint presentation is rejected. Workspace capture
content-hashes tracked, untracked, ignored, generated `build`/`dist` output,
and nested-repository worktrees. Conventional dependency caches such as
`node_modules`, `target`, virtual environments, and `.cache` are not task
scope; the host must keep them outside worker write authority and produce final
acceptance evidence from its clean trusted runner.

Each checkpoint and finalization writes a deterministic private operation claim
for the current session revision before invoking a checker or host attestation.
Only one process can claim that work. If it is interrupted before its result is
persisted, later calls stop with `repair_operation_in_progress`; cancellation or
a newly accountable authority revision is required instead of an uncounted
rerun.

Reviewing a handoff reply is observational: Whetstone reauthenticates the
accountable owner, rechecks expiry after that exchange, and compares the fresh
workspace fingerprint with the stored session and handoff. It does not invoke
another checker or consume work outside the stopped session budget.

The host may translate this JSON result for a human, but it must preserve the
same state, actionable finding details, persisted handoff, snapshot identities,
permitted actions, and explicit unknowns returned to an agent.
