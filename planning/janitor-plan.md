# Janitor — the night shift for your repos

> **Plan, v0.1 · 2026-07-03.** A cron-driven, forge-agnostic, agent-agnostic
> harness that turns deterministic Whetstone findings into tracked issues, then
> opens small, cited, self-verified PRs overnight. This doc lives here until the
> `janitor` repo exists; it moves there at bootstrap.

## 0. Assumptions (decided while you were AFK — all overridable)

Asked as clarifying questions; no response, so the plan proceeds on the
recommended options. Flip any of these and the plan flexes as noted in §9.

| Fork | Assumed | Why |
|---|---|---|
| Home | **Standalone project** (`janitor` repo, own CLI) that *consumes* `wh` | Keeps Whetstone role-thin per its boundary doc; orchestration policy ≠ deterministic substrate |
| Runtime v1 | **Your machine/server via cron/launchd**, CI-schedulable later | Full agent CLIs + local secrets + any forge reachable; hours trivially configurable |
| Autonomy | **Opens PRs, never merges** (v1); caps everywhere | Trust is earned; one bad auto-merge poisons the well early |
| v1 queue | **Whetstone-only** findings | Every PR traces to a cited, eval-verified rule — the differentiator |

## 1. The one-liner and the gap

**Janitor**: while you sleep, it sweeps your repos with Whetstone, files the
findings as issues, dispatches a coding agent to fix each one in an isolated
worktree, verifies the fix *deterministically*, and leaves small, cited PRs and
a morning report.

The 2026 landscape (validated by current research — see §10):

- **Renovate / Dependabot** — scheduled + reliable, but *dependencies only*.
- **Copilot coding agent, Claude Code GH Actions, Jules, Codex cloud** —
  real agents, but forge- or vendor-locked, and *reactive* (assign an issue,
  mention a bot) rather than scheduled and self-directed.
- **Free-roaming "fix my repo" agents** — maximum recall, minimum precision;
  the PR-spam failure mode that erodes trust fastest.

**The unserved square:** (i) schedule-driven, (ii) forge-agnostic, (iii)
agent-agnostic, (iv) **work queue generated from deterministic, reviewable rule
findings** rather than open-ended LLM judgment. Janitor's thesis in one line:

> **Deterministic queue → agentic fix → deterministic verify.**
> The agent is used for exactly the one step agents are good at.

Whetstone is what makes (iv) possible: doc-cited rules, golden-verified by
`wh eval`, scanned by tree-sitter — a work queue you can trust before any
agent runs.

And the timing matters: 2026's defining OSS story is the **AI PR-slop
backlash** (GitHub shipped maintainer throttles; projects are banning AI
contributions outright — §10). The market is rejecting open-ended agent PRs
*while still wanting the labor*. A janitor whose every PR cites a
deterministic finding and caps its own output is the design that survives
that backlash.

## 2. One night on shift (the target behavior)

```
02:00  SWEEP    pull default branches; wh scan/ci/eval per repo; hash findings
02:04  TRIAGE   dedupe vs ledger + open issues/PRs; policy filter; rank; cap
02:06  FILE     create/update tracker items (rule id, citation, snippet)
02:10  FIX      per item: fresh worktree + branch → headless agent with a
                bounded brief → agent edits (governed by whetstone's own
                PostToolUse hook — the janitor's agent is also on-standard)
02:30  VERIFY   deterministic gate: target rule clean, no NEW violations,
                project tests + lint green, diff within caps, scope respected
02:35  PROPOSE  commit, push, open PR: one finding-class per PR, citation +
                before/after in the body, linked issue, `janitor` label
06:55  REPORT   morning digest: PRs opened, items skipped & why, adherence
                trend, budget spent, failures — to a file/issue/webhook
```

Anything that fails VERIFY is abandoned (worktree deleted, issue annotated
"attempted, failed verification") — a failed fix never becomes a PR.

## 3. Architecture: a thin core and four adapter seams

```
janitor (single binary, Rust)
├── core: schedule loop · ledger (SQLite) · policy engine · budgets · digest
└── adapters/
    ├── findings:  whetstone            (v1: scan | drift | eval | coverage)
    ├── forge:     github (gh) | gitlab (glab) | git-only fallback
    ├── tracker:   gh-issues | beads | gitlab-issues | none (PR-only)
    └── agent:     claude | codex | custom command template
```

The **agnosticism contract** is the adapter interfaces, not abstractions for
their own sake:

- **FindingSource** → `list(repo) -> [Finding { hash, kind, rule_id, severity,
  file, line, citation_url, snippet }]`. v1 ships one source: `wh … --json`.
- **Forge** → `ensure_clone`, `default_branch`, `push_branch`, `open_pr`,
  `list_janitor_prs`, `comment`. Implemented over `gh` / `glab` CLIs. The
  **git-only fallback** (push branch + write a PR-body file for manual opening)
  is the true agnostic floor — Janitor degrades gracefully on any bare remote.
- **Tracker** → `file(finding) -> item_id`, `annotate`, `is_dismissed`.
  `beads` adapter uses `bd` (nice for your own repos); `none` = PR-only mode.
- **Agent** → *"run this brief in this worktree, headless; exit code + JSON
  transcript."* Built-ins for `claude -p --output-format json` and
  `codex exec`; plus a **command template** adapter
  (`command = "my-agent run --prompt {prompt_file} --dir {worktree}"`) so any
  present or future CLI qualifies. Per-fix wall-clock timeout + turn/token caps.

## 4. The fix brief (what the agent actually receives)

A tightly bounded, generated prompt — not "clean up the repo":

```
You are fixing exactly one finding in an isolated worktree.

FINDING   fastapi.lifespan-over-on-event (should, migration)
WHERE     src/app.py:12 — @app.on_event("startup")
WHY       Deprecated; docs recommend the lifespan context manager.
CITE      https://fastapi.tiangolo.com/advanced/events/

RULES     Fix only this finding class. Minimal diff. Touch only implicated
          files. Follow whetstone/context/AGENTS.md conventions.
DONE WHEN `wh scan <files> --rule fastapi.lifespan-over-on-event` is clean,
          no new violations anywhere, and the project's tests pass.
```

Batching: the same rule across ≤N files may share one PR (still one
finding-*class* per PR); everything else is one-finding-one-PR.

## 5. The trust envelope (what makes it survive contact with reality)

| Failure mode | Janitor's answer |
|---|---|
| PR spam | Ledger idempotency: a finding-hash with an open PR/issue, or dismissed by a human, is never re-filed. Caps: `max_prs_per_night`, per-repo. |
| Low-trust diffs | Deterministic VERIFY gate — the agent is never trusted; `wh scan` + tests decide. Diff-size + file-scope caps. Citations in every PR body. |
| Broken rules pushing bad fixes | Pre-flight `wh eval`: a rule failing its own goldens is quarantined for the night, never acted on. |
| Merge-conflict rot | Janitor rebases or closes-and-replaces *its own* stale PRs; never touches human branches. |
| Cost blowout | Per-fix and per-night token/turn budgets; spend printed in the digest; hard stop when exhausted. |
| Runaway agent | Fresh worktree per fix (blast radius = one directory), wall-clock timeout, allowlisted tools, no network beyond the repo unless granted. |
| "Just make it stop" | `janitor pause [repo]`, a `.janitor-pause` marker file, and label-based dismissal (`janitor-ignore` on an issue = permanent skip). |

Autonomy ladder (each rung *earned* by a clean streak, configured per repo):
1. **v1 default:** file issues + open PRs. Human merges.
2. Opt-in: auto-merge a whitelisted trivial class (mechanical single-rule fix,
   diff < 50 lines, CI green) after ≥2 weeks of 100%-accepted PRs.
3. Never: force-push, edit human branches, merge anything failing checks.

## 6. Whetstone synergy (why this pairing is more than the sum)

- **`wh scan --json`** is the queue; rule ids + `source_url` citations are the
  PR trust story.
- **`wh eval`** guards the guards: broken goldens quarantine a rule.
- **`wh ci` content-hash drift** → Janitor files *re-extraction* issues and can
  dispatch the agent to re-validate a rule against the new docs — this
  automates the pack-currency cadence (whetstone-ry6) that's currently manual.
- **Coverage gaps** (deps with no rules) → nightly *extraction proposal* issues
  (never auto-PRs — extraction needs your approval loop).
- **Recursive governance:** the overnight agent runs with whetstone's own
  PostToolUse hook active, so the janitor's fixes are themselves policed
  in-session before VERIFY even runs.
- **Digest metrics** come free: adherence score trend, violations burn-down.

## 7. Configuration sketch

```toml
# ~/.janitor/janitor.toml
[defaults]
agent   = "claude"            # claude | codex | <custom template name>
tracker = "gh-issues"         # gh-issues | beads | gitlab-issues | none
max_prs_per_night = 3
max_diff_lines    = 200
severity_floor    = "should"  # ignore `may` findings at night
budget_tokens     = 400_000   # per night, all repos

[repos.myapp]
path   = "~/code/myapp"       # local clone (or url = "..." to manage a clone)
forge  = "github"
hours  = "02:00-06:00"        # shift window; cron fires entry, window bounds it
rules_deny = ["zod.record-two-args"]   # per-repo opt-outs

[agents.claude]
command = "claude -p {prompt_file} --output-format json --max-turns 30"

[agents.custom-example]
command = "opencode run --dir {worktree} --prompt {prompt_file}"
```

Scheduling is deliberately *not* reinvented: a documented `crontab` /
`launchd` / `systemd-timer` line calls `janitor run`; the `hours` window
bounds the shift regardless of what fires it. The same binary runs under a
forge CI scheduler later (Phase 3) — same config, different clock.

## 8. Build plan (phased, each phase ends usable)

**Phase 1 — one clean night (MVP).** Rust binary. One repo, `github`+`gh`,
`claude` adapter, `whetstone.scan` source, SQLite ledger, worktree fix loop,
VERIFY gate, PR + markdown digest. *Exit test: wake up to ≥1 correct, cited,
green PR from your own repo, zero spam.*

**Phase 2 — trust + breadth.** Trackers (gh-issues, beads), drift + eval +
coverage finding kinds, multi-repo, caps/policy engine, stale-PR hygiene,
pause/dismiss controls, digest to webhook (Slack/email).

**Phase 3 — full agnosticism.** `glab` + git-only forge adapters, `codex` +
custom-template agent adapters, CI-scheduler mode (`janitor gha-init` /
`glab-init` emit workflow YAML), re-extraction dispatch (pack currency),
auto-merge rung 2 behind config.

**Phase 4 — polish.** Parallel fixes (worktree pool), flaky-verify retry
policy, digest trends over weeks, multi-machine ledger sync if ever needed.

Non-goals (deliberate): no scope-discipline enforcement (out of scope per
whetstone boundary §10), no general free-roam mode, no forge webhooks/daemon
in v1 (cron is the contract), no dashboard before the digest proves itself.

## 9. If the assumptions flip

- **`wh janitor` instead of standalone** → same architecture, but adapters live
  behind a `janitor` module; accept the boundary-doc exception explicitly.
- **CI-scheduler-first** → Phase 3's `*-init` generators move into Phase 1;
  local mode becomes the later addition; secrets story moves to forge vaults.
- **Auto-merge sooner** → rung 2 config ships in Phase 2 instead of 3; the
  clean-streak requirement stays non-negotiable.
- **Broader queue** → add FindingSources (deps-outdated, TODO harvest) in
  Phase 3+; each must emit the same `Finding` shape with a citation or it
  doesn't ship (the trust story is the product).

## 10. Landscape notes (researched 2026-07-03, primary sources)

The four criteria: **(i)** cron/schedule-driven · **(ii)** forge-agnostic ·
**(iii)** agent-executor-pluggable · **(iv)** deterministic, cited work queue.

| Tool | (i) | (ii) | (iii) | (iv) | Note |
|---|---|---|---|---|---|
| Renovate | ✅ | ✅ | — | ✅ *deps only* | The proof the model works — for exactly one finding type |
| Dependabot | ✅ | GitHub only | — | deps only | |
| Copilot coding agent | reactive | GitHub only | Copilot only | human-assigned issues | GitHub's own billing team uses it for debt burndown |
| Claude Code Routines (Apr 2026) | ✅ managed cron | Anthropic cloud, GitHub-centric | Claude only | judgment-driven | headless `claude -p` is a building block, not a harness |
| Codex cloud/Automations · Cursor BG agents · Jules | partial | GitHub-centric | single-vendor each | judgment-driven | |
| OpenHands Resolver | event-driven | GitHub+GitLab | own harness | label→PR, no rule queue | |
| **Sortie** (nearest neighbor) | partial | GitHub/Linear/Jira trackers | ✅ Claude/Codex/OpenCode/… | ❌ consumes *human tickets* | Go binary + SQLite + adapters — validates Janitor's architecture shape |
| Sweep | pivoted to JetBrains assistant (dead as maintainer bot) | | | | |
| CodeRabbit / Korbit | review-only, not authors | | | | |

**Conclusion: the square is empty.** Sortie has the executor plumbing but no
finding intelligence; Renovate has the scheduled-deterministic model but only
for dependencies; the vendor agents have the intelligence but neither the
schedule discipline nor the agnosticism. Janitor = Renovate's operating model
generalized from "dependency updates" to "any doc-cited standard," with
Whetstone as the rule brain and any agent CLI as the hands.

**The timing argument — the 2026 PR-slop crisis.** AI-generated PR spam is now
an ecosystem-level problem: Jazzband shut down over it, curl ended its bug
bounty, GitHub shipped maintainer throttles for AI PRs, Godot banned AI
contributions. The industry's reaction is *distrust of open-ended agent PRs* —
which is precisely why a nightly bot whose every PR traces to a deterministic,
doc-cited, golden-verified finding (and which caps itself) is the right design
for this moment, not just a nice-to-have.

Build on, don't reinvent: `claude -p --output-format stream-json
--allowedTools --max-turns` · `codex exec` · `opencode run --format json` ·
git worktrees for isolation · `gh`/`glab` for forge plumbing · plain cron/CI
`schedule:` triggers.

Sources: docs.renovatebot.com · docs.github.com (Dependabot, Copilot plans) ·
github.blog (billing-team coding-agent) · code.claude.com/docs/en/scheduled-tasks ·
developers.openai.com/codex/cli · devin.ai/pricing · developers.google.com/jules ·
openhands.dev · docs.sortie-ai.com · thenewstack.io (AI-generated code crisis) ·
coderabbit.ai/blog (GitHub AI-PR throttle).
