# Whetstone

Whetstone helps people hand more work to coding agents and trust the result.
Your mission, the [pstack](https://github.com/cursor/plugins/tree/main/pstack)
principles you pick and the codebases you admire become a short list of agreed
rules. Every agent and every person, whatever tool they use, is briefed on
those rules before writing code, checked against them afterwards, and told
when to stop and ask.

It targets the ways delegated work goes wrong: work that falls short, errors,
missed existing code, code or tests nobody asked for, drift from the design
system, and agents guessing instead of asking.

## How it works

1. **Agree the rules once.** `wh init` installs or detects Beads and pstack,
   offers three starter rules, and turns your mission, principles and example
   codebases into proposed rules. You accept each one in the dashboard.
   Accepted rules are records in Beads.
2. **Enforce each rule the cheapest reliable way.** Every rule has one
   strength (must, should or advisory) and one enforcer, chosen in this order:
   a mechanical check (linter, AST, design tokens, public API), then a literal
   yes/no question for Jev (TypeSafe System One) scored against the rule's
   examples, then a review by pstack's `/interrogate` or a person. Jev can
   raise a flag but can never pass a must rule.
3. **Brief before building, prove before shipping.** Agents run pstack's
   `/how`, `/why` and `/blast-radius` first. Checks run at git pre-commit
   (fast, staged files), pre-push (the full `wh check` with fresh proofs) and
   as a required CI status. The generated verification skill proves features
   in the running app.
4. **Raise a hand without stalling.** Low confidence, a second failed repair,
   a vague spec or a must-rule area makes the agent file a `bd human` request
   and move to other ready work.
5. **Get stricter and cheaper over time.** Flags you accept or dismiss give
   each rule a false-flag rate. Noisy rules are proposed for demotion and
   recurring Jev flags are proposed as mechanical checks. You accept every
   change; nothing learns on its own.

The public surface is exactly six commands:

| Command | Purpose |
| --- | --- |
| `wh init` | Set up: install what's missing, agree the mission, principles and rules, wire hooks and the verification skill. |
| `wh dash` | See rules, checks, raised hands and the full decision log, and accept or reject drafts. |
| `wh change` | Propose a change to a rule, principle or feature map as a reviewable draft. |
| `wh check` | Check work against the rules, return failures to the worker, and record receipts. |
| `wh pull` | Receive the team's shared agreement without touching your drafts. |
| `wh push` | Share an exact, confirmed selection of accepted records. |

The direction is recorded in [planning/direction.md](planning/direction.md) and
tracked in Beads under the epic `whetstone-ppq`. That epic supersedes the
earlier `whetstone-k5r` plan.

## Where it stands today

This is a pre-release. The six commands work against the Beads record store
and the plan in epic `whetstone-ppq` is built:

- **Setup.** `wh init --action setup` detects Beads and pstack, shows the
  exact install commands and runs them after one confirmation, and pins what
  it found in `whetstone/tools.lock.json`. Onboarding asks for a one-line
  mission, offers pstack's principles and three starter rules chosen for the
  repository, and turns exemplar codebases (`--action exemplar --from <path>`
  or `--url <git url>`) into rule drafts with provenance.
- **Rules.** One strength and one enforcer per rule
  ([rule schema](references/rule-v2.schema.json)): AST, lint, formatter, test,
  validator, drive, design tokens, public surface or brief; a Jev question
  (in shadow until promoted); or a review by `/interrogate` or a named person
  (`wh check --attest`). Earlier standards and guidance are read as rules, and
  `wh change --migrate` turns earlier values, philosophy and YAML rules into
  drafts.
- **Jev.** Questions go through the driver's `ask` command to TypeSafe System
  One. Text is redacted in Whetstone before the driver sees it (path globs,
  patterns, built-in secret patterns and the key itself), receipts record
  that redaction happened, and a local-only rule never leaves the machine.
  A Jev answer can flag but never pass a must rule; low confidence raises a
  hand.
- **Gates.** `wh init --action wire --hooks` installs git pre-commit (staged
  content, mechanical rules only) and pre-push (`wh check --base <remote>`)
  hooks, chaining any existing hook, plus the Claude, Codex and Cursor agent
  hooks. `--ci` writes the `whetstone/policy` required status workflow.
- **Hands and briefs.** `wh check --raise-hand` files a `bd human` issue and
  the dashboard shows it until `wh change --answer` records the answer.
  `wh check --brief` records a pstack brief that brief-enforced rules require.
- **Learning.** `wh change --accept-flag` and `--dismiss-flag` label flags.
  Each rule then has a false-flag rate, and `wh change --tune` drafts
  demotions for noisy rules and promotions for accurate shadow questions.
  Recurring accepted flags are offered as hardening candidates
  (`wh change --kind rule --hardens <rule>`).
- **Proofs.** Flaky proofs are quarantined as unknown, and
  `wh check --mutate` runs a feature's declared mutations in an isolated
  worktree to catch proofs that pass on broken code.

Still waiting on the owner: a live Jev key run, the milestone walkthroughs and
the two-week tuning experiment, all tracked under `whetstone-ppq.10`.

```bash
wh init                                                     # guided onboarding in the dashboard
wh init --json                                              # inspect: missing steps, starters, tools
wh init --action setup --yes                                # install bd and pstack if missing
wh init --action agree --mission "..." --principle prove-it-works --starter all \
  --expected-revision 0 --resume <token>
wh init --action wire --host claude --host codex --hooks --ci
wh change --kind rule --record-id rule.no-todo --content "..." --rationale "..." \
  --definition '{"type":"rule","strength":"should","enforcer":{"kind":"question","question":"Does this change add a TODO?"}}'
wh check --staged                                           # what pre-commit runs
wh check --base origin/main                                 # what pre-push runs
wh check --sweep                                            # drive every mapped feature
wh check --attest rule.api-review --verdict pass --reviewer interrogate --notes "..."
wh dash                                                     # local dashboard
wh dash --trail                                             # decision trail as TSV
wh push                                                     # review the exact package, then --confirm <token>
wh pull                                                     # receive; never executes or accepts
```

Machine clients read `whetstone.command-response.v1`
([schema](references/command-response-v1.schema.json)). Requests with
`--json` never prompt. They return explicit states such as `needs_input`,
`needs_decision`, `stale`, `conflict`, `unknown` or `unavailable`, along with
the permitted next actions.

### Storage

Records live in Beads (`bd` 1.1.2 or later, embedded Dolt; no `dolt` binary).
Each record revision is a bead whose metadata carries the typed body, digest,
supersedes reference and idempotency key; lifecycle is a `wh:lifecycle:*`
label. Drafts and receipts stay in a private Beads database under
`.git/whetstone/` that never has a remote. `wh push` copies a confirmed
package of accepted records into the repository's shared Beads database and
runs `bd dolt push`. `wh pull` runs `bd dolt pull`. Policy is never committed
to the code branch: the team remote carries Beads data in `refs/dolt/data`.

### Developer gates

Three hidden gates keep the kernel honest: `validate`, `eval` and `scan`.
They run with the other five in [AGENTS.md](AGENTS.md) before every push.

## Install

There is no release of this product yet. The latest GitHub release, v0.12.0,
is the previous product, and `install.sh` installs that release. Build from
source:

```bash
cargo build --release
mkdir -p ~/.local/bin
cp target/release/whetstone ~/.local/bin/whetstone
ln -sf ~/.local/bin/whetstone ~/.local/bin/wh
wh --help   # lists init, dash, change, check, pull and push
```

Requirements: `bd` 1.1.2 or later, Node 22 for the verification driver, and
Chrome or Chromium for web surfaces. Jev rules need a `TYPESAFE_API_KEY` in
the environment; without one they report `unavailable`, which is never a pass.

An earlier install of the previous product also answers to `wh` and would open
its terminal UI instead. If `wh --help` lists commands such as `extract`,
`pack` or `mcp`, the copy above did not replace it (check `which -a wh`).

## Contributing

Use Beads for work tracking and read [AGENTS.md](AGENTS.md) before changing the
project. All eight gates in that file must pass before pushing.
