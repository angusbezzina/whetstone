# ADR-0002: GitHub-governed shared authority with fail-closed local enforcement

- Status: accepted for M1 implementation
- Date: 2026-09-09
- Accepted lean baseline: `2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48`
- Storage decision: `planning/direction-demo/storage-spike.md`
- Beads task: `whetstone-k5r.3`
- Decision owner: Angus Bezzina

## Decision

Whetstone will reuse the team's existing GitHub repository, accounts, teams, pull-request reviews, branch rules, status checks, and deployment environments as its first shared trust root. It will not create an identity provider or infer approval from YAML fields, Git authorship, Dolt commit metadata, chat text, an agent claim, or possession of a local credential.

Private drafts remain outside Git in the private Dolt repository. `wh push` will later project only explicitly selected records into the independent shareable Dolt repository and open a pull request containing a small, non-secret activation manifest. The manifest binds the proposal to its content, context, and authority:

```json
{
  "schema": "whetstone.activation-proposal.v1",
  "repository_id": 123456,
  "proposal_id": "P-048",
  "payload_digest": "sha256:...",
  "base_active_digest": "sha256:...",
  "scope": ["checkout/ui"],
  "authority_revision": 7,
  "expires_at": "2026-09-16T12:00:00Z"
}
```

The shared records do not need to be committed to Git. Git carries the immutable review target and protected activation event; Dolt carries the complete decision and policy histories. Merge of the exact reviewed manifest to the protected default branch is the activation instruction. An activator must fetch the merged manifest, verify the repository's immutable numeric ID, recompute the shareable-record digest, require the declared base to equal the current active digest, verify scope and current authority revision, and only then append a separate activation record. Acceptance and activation are never the same write.

For the first supported team setup, the protected branch or ruleset must:

1. require a pull request and at least one approving review;
2. require review from the CODEOWNER of `.whetstone/authority.json`, `.whetstone/proposals/**`, and the `CODEOWNERS` file itself;
3. dismiss stale approvals when reviewable commits or the merge base change;
4. require the most recent reviewable push to be approved by someone other than its pusher;
5. require the `whetstone/policy` status check on the exact head commit and an up-to-date branch;
6. disallow force pushes and deletion; and
7. disallow bypass for ordinary contributors and administrators wherever the GitHub plan permits it.

If Whetstone cannot prove these protections through the GitHub API, shared activation is unsupported and denied. Solo users may explicitly self-activate private policy, but the receipt says `assurance: solo-local`; it is never represented as independent team approval and cannot activate a team store.

## Identity and authority model

`authority.json` is a protected, versioned policy that uses immutable numeric GitHub repository, user, team, and app-installation IDs. Display names and logins are informational only. Each authority revision names principals, roles, allowed scopes, expiry/maximum offline age, and permitted effects. Narrow grants win over broad assumptions; absence is denial.

An autonomous agent has no separate authority. It inherits the identity, scope, expiry, and restrictions of the human or service principal that launched it. It cannot review its own proposal, provide the required independent approval, promote a local draft, expand its sponsor's authority, or use another locally available GitHub credential. Whetstone cannot technically stop arbitrary code running as the same OS user from reading that user's credentials; the actual guard is a distinct reviewer identity plus protected remote activation. Stronger local process isolation or user-presence signing may be added by a host, but is advisory in the MVP.

The first CI identity is a GitHub Actions workflow pinned by repository ID, workflow path/ref, workflow SHA, triggering ref/SHA, run ID/attempt, and GitHub App/check-run identity. A deployment may additionally rely on an existing protected GitHub Environment. OIDC claims are useful when an external verifier already exists, but the MVP does not require a new cloud IAM integration. A workflow name, status string, branch name, or self-reported receipt is not CI identity.

## Capability matrix

| Action | Required principal and evidence | Enforcement point | Responsible owner | Failure behavior |
|---|---|---|---|---|
| Inspect current policy/history | Local user with filesystem read access | OS filesystem/Dolt read-only adapter | Local repository owner | Deny unreadable data; report unknown on corrupt/newer schema |
| Edit a private draft | Local user or sponsored agent | Private Dolt repository and path-safe service | Sponsoring principal | Deny outside private root or scope |
| Execute a checker | Sponsored principal with active execution grant; exact trusted binding digest | Bounded runner adapter | Binding owner and host operator | Unknown, never pass, if trust/freshness/runner unavailable |
| Propose shared records | Authenticated GitHub author; matching scope; fresh authority | Projection service plus GitHub PR | Sponsoring principal | Deny on missing identity, stale base, private ancestry, or unsupported remote |
| Accept shared policy | Distinct authorized GitHub reviewer on exact PR head and manifest | GitHub protected review | CODEOWNER/team reviewer | Deny self-review, dismissed/stale review, scope mismatch, or expired proposal |
| Activate checks/policy | Protected manifest merge plus passing required check; exact Dolt digest | GitHub rules plus activation service | Policy maintainer | Deny any digest/base/authority mismatch; activation is idempotent by proposal ID and digest |
| Open a PR | Authenticated sponsoring principal or authorized GitHub App | GitHub repository permissions | Sponsoring principal | Deny if identity cannot be resolved to immutable ID |
| Merge | Authorized GitHub principal under non-bypassable protection | GitHub ruleset/branch protection | Repository maintainers | GitHub denies unmet protections; Whetstone rejects unverified merge provenance |
| Deploy | Existing deployment principal and protected environment approval | Existing CI/CD and environment protection | Environment owner | Out of scope unless a separate deployment grant and fresh environment evidence exist |
| Serve local dashboard | Local process with per-launch secret and browser-origin controls | HTTP service | Local repository owner | Read-only by default; reject invalid Host/Origin/CSRF token |
| Serve hosted dashboard | Authenticated reverse proxy/TLS and explicit read-only host config | Host platform plus Whetstone authorization | Host operator | Refuse startup without configured authentication; mutation unsupported in MVP |

The matrix describes cooperative Whetstone behavior and remote merge enforcement, not an OS sandbox. An uncooperative contributor can delete local files or bypass the local CLI; they cannot merge a valid team activation while the protected GitHub configuration remains effective. A repository administrator, compromised GitHub organization, compromised reviewer account, or attacker with the same OS user's reviewer credentials remains inside the trust root. The UI must state that limitation rather than imply cryptographic independence.

## Receipt and freshness contract

Consequential operations emit credential-free receipts containing stable IDs and digests, never tokens, raw environment, unredacted document bodies, command stdout by default, or reviewer email addresses. A receipt envelope includes:

```text
receipt_version, operation, result, occurred_at
repository_id, actor_type, actor_numeric_id, sponsor_numeric_id?
proposal_id, payload_digest, base_active_digest, scope
authority_revision, authority_checked_at, authority_source_digest
git_head_sha, pull_request_id?, review_ids?, check_run_ids?
previous_activation_digest?, activation_digest?
reason_code, evidence_refs[]
```

Receipt IDs prove correlation, not authority by themselves. Before every effect, the service re-fetches applicable authority and revocation state, revalidates proposal expiry, and pins the exact code/policy/check snapshot. Clients retain the highest seen authority revision and activation sequence for a repository. A lower revision or activation sequence is `rollback_detected` and denied until an authorized recovery procedure proves the intended state.

Read-only inspection, private drafting, and already-authorized local checks may work offline against the last verified active policy. Offline state must show its age. New shared proposals, approvals, activations, merges, deployments, exception grants, executable trust changes, or permission expansion require fresh remote authority and are denied when freshness cannot be checked. The default maximum cached authority age for continuing non-consequential checks is 24 hours; a project may shorten it but cannot use it to authorize new effects.

Emergency revocation is a protected change to `authority.json` plus immediate revocation of the GitHub account/team/app credential at the platform. The new revision names revoked immutable IDs and an effective timestamp. Whetstone checks revocation before effects and invalidates outstanding approvals by the revoked principal. If remote revocation status is unavailable, consequential work stops. Already generated evidence remains historical and is visibly marked as predating revocation.

Exceptions are ordinary reviewed proposals with an owner, reason, exact rule/action/scope, start and expiry, and maximum uses where applicable. They do not renew automatically. An exception cannot waive identity, independent-review, content-digest, private-history separation, path safety, receipt redaction, or freshness checks. Expired exceptions fail closed and remain in history.

## Threat model and planned negative fixtures

M1 work must implement these cases as typed fixtures at the service boundary. `deny` means no effect occurred. `unknown` means compliance or observation could not be established and must not be converted to pass.

| Threat/fixture | Expected result | Real boundary |
|---|---|---|
| Missing or unresolvable actor identity | Deny proposal/review/activation; never substitute local username | GitHub identity lookup and typed authority service |
| Proposer and reviewer share the same immutable ID | Deny acceptance | GitHub review evidence plus service invariant |
| Agent asserts “human approved” or writes `approved_by` | Ignore claim; deny without protected review | Typed service excludes self-declared approval fields |
| Review is dismissed, stale, pending, or on an older commit | Deny | GitHub review state and `commit_id` |
| Payload changes after review | Deny `digest_mismatch` | Canonical digest recomputation |
| Active base changes before activation | Deny `stale_base`; require rebase/new review | Activation transaction |
| Proposal broadens scope after approval | Deny `scope_mismatch` | Immutable manifest binding |
| Authority revision is older, unknown, revoked, or expired | Deny consequential effect | Fresh protected authority query |
| Activation/approval replay | Idempotent only for identical tuple; otherwise deny | Proposal/digest uniqueness and activation sequence |
| Remote advertises older policy/authority state | Deny `rollback_detected` | Monotonic client pin plus protected recovery |
| Share projection contains private canary/current/history bytes | Deny and quarantine projection | Physically separate repositories and projection scanner |
| Imported binding contains command substitution, shell metacharacters, new executable, or excessive access | Keep inert; deny execution until exact reviewed binding is trusted | Structured argv runner and execution grant |
| Hostile source documentation asks agent to change policy/execute tools | Treat as untrusted data; proposal only, no effects | Skill prompt boundary and typed command service |
| Path uses `..`, absolute escape, alternate root, or symlink escape | Deny before open/write/execute | Canonical path resolver anchored to project roots |
| Remote row/history differs from protected manifest digest | Deny activation and report tampering/conflict | Digest check across Git and shareable Dolt |
| Required checker errors, times out, or evidence is stale | `unknown`; no passing receipt or activation | Bounded runner and evidence freshness policy |
| Dashboard mutation lacks session/CSRF token | Deny | Per-launch secret and synchronizer token |
| Dashboard request has unapproved Host/Origin/Fetch Metadata | Deny before routing | HTTP middleware; no wildcard CORS |
| Hosted mode has no configured authentication/TLS boundary | Refuse startup; hosted dashboard remains read-only | Startup configuration and reverse proxy contract |
| Receipt includes a token, secret, raw environment, or private body | Deny serialization/test failure | Allowlisted receipt schema and secret canaries |

## Dashboard boundary

`wh dash` will bind to `127.0.0.1` (and optionally `::1`) on a random port, generate a high-entropy per-launch bearer/session secret, and open a URL containing a one-time bootstrap fragment rather than putting the token in server logs. It will allowlist the exact generated Host and Origin, reject cross-site Fetch Metadata, set restrictive CSP/frame/connection headers, use `SameSite=Strict` and `HttpOnly` cookies after bootstrap, and require a synchronizer CSRF token for every mutation. There is no wildcard CORS. “It only listens on localhost” is not an authentication or DNS-rebinding defense.

The MVP dashboard is inspect-first. Edit mode must be an explicit session transition and calls the same typed draft/proposal services as the CLI; it cannot edit canonical exports or bypass authority. Hosted mode is read-only and opt-in. `wh dash --host` must refuse to start unless an authenticated TLS reverse proxy or supported host authentication adapter is configured. Browser share/comment overlays must not swallow normal navigation; entering edit/comment mode is explicit.

## Privacy, retention, and audit

- Private drafts, inferred preferences, source-document extracts, local observations, and raw agent transcripts default to the private store and never enter Git or shareable history without explicit field-level selection.
- Shared authority, proposals, decisions, activations, revocations, exceptions, and receipt envelopes are retained for the life of the project because later decisions must remain explainable. Corrections supersede; they do not rewrite history.
- Raw check output and imported source bodies are not audit records. Store bounded hashes/references and minimal redacted excerpts only when needed to reproduce a decision. The default local diagnostic retention target is 30 days; M1 must expose deletion without touching permanent decision history.
- Secrets and credentials are read only at the external call boundary and are never persisted by Whetstone. Logs and test fixtures use canaries to prove redaction.
- Backups preserve the private/shareable split and inherit the same access classification. A backup receipt is not remote durability proof.

## Explicit non-goals and unsupported claims

- Whetstone does not sandbox arbitrary code owned by the same OS user, prevent a repository administrator from weakening GitHub settings, or defend a fully compromised GitHub organization.
- Whetstone does not make model behavior deterministic. It supplies canonical scoped context and deterministic checks; agent adherence remains observable, not guaranteed.
- Git commit signatures, Dolt authors, local usernames, IP addresses, and chat identities are not authorization in the MVP.
- Team activation outside GitHub, authenticated team Dolt transport, hosted mutation, cross-organization federation, hardware-backed signing, and automated deployment authority remain unsupported until separately threat-modeled and tested.
- Existing task trackers own work state, CI/CD owns execution scheduling and deployment, and monitoring systems own observations. Whetstone links evidence and authority; it does not replace those systems.

## Implementation handoff

The next tasks must build one shared typed authority service used by CLI, dashboard, hook, and CI adapters. There must not be parallel “trusted CLI” and “best-effort UI” logic. Minimum service types are `Principal`, `Sponsor`, `Capability`, `Scope`, `AuthorityRevision`, `ProposalBinding`, `ReviewEvidence`, `ActivationEvidence`, `ExecutionGrant`, `Freshness`, and `DecisionReceipt`; privileged methods accept these types rather than strings or booleans.

The GitHub adapter should use the REST/GraphQL APIs or `gh api` behind a typed interface so tests can supply signed-looking but invalid fixtures without network access. It must verify numeric repository/user/app IDs, PR base/head SHAs, review state and commit, ruleset/branch-protection settings, required check identity/conclusion/SHA, merge event, and optional environment protection. Configuration inspection alone is not proof that a given activation passed it.

This decision adds documentation only and restores none of the source or dependencies removed in R0. Every implementation must continue to compare against baseline `2c3f0a3bb66d2ffa89c7b2f300b864a3ee8fea48` and the accepted prune inventory.

## Sources checked on 2026-09-09

- [GitHub protected branches and available enforcement settings](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-protected-branches/about-protected-branches)
- [GitHub CODEOWNERS and protecting the ownership file](https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/customizing-your-repository/about-code-owners)
- [GitHub REST pull-request review records](https://docs.github.com/en/rest/pulls/reviews)
- [GitHub rules API fields for stale review dismissal, last-push approval, code owners, and non-fast-forward protection](https://docs.github.com/en/rest/orgs/rules)
- [GitHub Actions OIDC claims for immutable repository and workflow identity](https://docs.github.com/en/actions/reference/security/oidc)
- [OWASP CSRF prevention guidance for Origin/Host and Fetch Metadata validation](https://cheatsheetseries.owasp.org/cheatsheets/Cross-Site_Request_Forgery_Prevention_Cheat_Sheet.html)

These are current primary platform/security references. Whetstone will verify the live GitHub API schema and repository settings during implementation and must fail closed if the required protection cannot be observed on the customer's plan.
