# Whetstone Field-Test Playbook

> How to test Whetstone (v0.10.0) on your real repos and iterate it into a
> genuinely reliable tool. The machinery is built and validated synthetically;
> this playbook is the real-world test — adoption, friction, and measurement
> drive the next epic, not speculation.

**Time budget:** ~10 min setup · ~30 min smoke test · 1–2 weeks living with it ·
30 min weekly review.

---

## Phase 0 — Preflight (5 min, do this first)

**⚠️ Gotcha zero: a stale `wh` on your PATH silently breaks everything.**
The hooks and `.mcp.json` exec `wh`. If that binary is old (yours was 0.8.22),
the PostToolUse hook and MCP server no-op or error. Fix first:

```bash
wh update                 # self-update from GitHub Releases
wh --version              # MUST print: whetstone 0.10.0
```

If `wh update` misbehaves, reinstall:

```bash
curl -fsSL https://raw.githubusercontent.com/angusbezzina/whetstone/main/install.sh | sh -s -- --version v0.10.0
```

**Pick 1–2 real repos.** Ideal: one Python repo using FastAPI / Pydantic /
SQLAlchemy / httpx, and one TypeScript repo using React / Next / Zod / Express —
those have starter-pack coverage. A repo outside the corpus is *also* a good
test (it exercises the extraction flywheel instead).

---

## Phase 1 — Onboard a repo (10 min)

```bash
cd /path/to/your/repo
wh init --claude
```

Expected: a `wired` summary listing imported packs, generated context, MCP
registration, and hooks. Verify the artifacts:

```bash
ls whetstone/packs/                      # imported starter packs (if any matched)
cat whetstone/whetstone.yaml             # version + extends entries
cat .mcp.json                            # mcpServers.whetstone
cat .claude/settings.json                # hooks.SessionStart + hooks.PostToolUse
head -5 whetstone/context/AGENTS.md      # generated agent context
wh scan .                                # baseline: how much existing code violates?
```

**Restart your Claude Code session** so the hooks and MCP server load.
Then commit the generated files (except `.personal/`, which is gitignored).

Record in your log: how long it took, anything confusing, baseline
`wh scan` violation count, and `wh status` score.

---

## Phase 2 — Smoke-test all three loops (30 min)

### Loop A — In-session enforcement (the headline feature)

Plant a violation and ask your agent to touch that file. Python bait:

```python
# bait.py — two violations if fastapi pack is imported
from fastapi import Depends, FastAPI
app = FastAPI()

@app.on_event("startup")                      # fastapi.lifespan-over-on-event
async def startup(): ...

@app.get("/items")
async def items(commons: dict = Depends(dict)):   # fastapi.annotated-depends
    return commons
```

TypeScript bait: `ReactDOM.render(<App/>, el)` (react.no-reactdom-render) or
`req.param("id")` (express.no-req-param).

Ask the agent: *"add a docstring to bait.py"* (any edit works). **Expected:**
after its Edit, the hook feeds the violations back and the agent addresses them
in the same turn — without you pasting anything.

Debug path if nothing surfaces (test the hook directly):

```bash
printf '{"tool_input":{"file_path":"bait.py"},"cwd":"%s"}' "$PWD" | wh hook posttooluse
# expected: {"hookSpecificOutput":{"additionalContext":"Whetstone: ..."}}
```

Silent → check `wh --version` (gotcha zero), that `bait.py` violates an
*imported* pack, and that `.claude/settings.json` has the PostToolUse entry.

### Loop B — Agent lookup (MCP)

Ask the agent: *"What Whetstone rules apply to src/<some file>?"* — expected: it
calls the `whetstone` MCP `rules_query` tool (or `wh rules query --file ...`)
and cites rule ids + doc URLs, plus any guidance entries.

### Loop C — Taste capture (your preferences)

Mid-session, tell the agent something like: *"Never use bare `print()` in this
repo — use logging. Make that a standing rule."* **Expected:** the agent follows
SKILL.md's capture workflow — authors a rule with 3–5 goldens, verifies with
`wh eval`, asks scope (personal/project) + severity — or, for judgment-only
taste ("keep handlers thin"), writes a guidance entry instead. Then verify:

```bash
wh eval                                   # must stay green
wh scan .                                 # new rule fires on old sins?
wh rules query --lang python --json | head -30
```

Grade it honestly: did the agent capture unprompted? With a nudge? Not at all?

### CI gate (optional but recommended)

```bash
wh init --ci --schedule=weekly
git add .github && git commit -m "ci: whetstone gates" && git push
```

Expected: the `enforce` job fails a PR containing bait violations; the scheduled
`freshness` job gates drift.

---

## Phase 3 — Live with it (1–2 weeks)

Work normally. Keep a **friction log** — this is the whole point:

```markdown
| date | surface | what happened | expected | severity (blocker/annoy/nit) |
|------|---------|---------------|----------|------------------------------|
```

Watch for, specifically:

- **False positives** — a rule firing on legitimate code. *Worst failure mode*;
  log the exact snippet. (High-confidence-or-silence means FPs are bugs, full stop.)
- **Hook latency** — does the PostToolUse hook add noticeable lag on this repo
  size? (Budget: sub-second.)
- **Agent compliance** — when a violation is surfaced, does the agent actually
  fix it, or acknowledge-and-ignore? Would `--block` mode serve you better?
- **Missed captures** — you stated a preference and the agent didn't offer to
  codify it. Log the phrasing that failed.
- **Coverage gaps** — deps you use daily with no rules. For each, run the
  extraction flywheel: `wh init`, then have the agent work `wh extract` →
  bundle → `wh rules approve` → `wh actions all`. Log the experience — this
  loop is the least field-tested part of the product.
- **Noise** — anything Whetstone said that you ignored. Repeated ignores = the
  rule or message is wrong, not you.

Grow your **personal taste pack** as preferences come up: copy
`packs/templates/taste.yaml` to a shared location (e.g. dotfiles), import it in
each repo via `extends`, and land captured personal rules there. The test:
does one pack genuinely follow you across both repos?

---

## Phase 4 — Measure (end of each week)

```bash
wh status            # adherence + rule-system score; compare to baseline
wh report            # one-page narrative — is it accurate? useful?
wh scan . --json | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['violations_count'],'violations')"
wh eval              # all rules (incl. captured ones) still golden-clean
wh ci                # any doc drift since rules were authored?
```

Success criteria for "the tool is working":

| Metric | Target |
|---|---|
| False positives on must/should rules | 0 (each one is a bug to file) |
| Hook latency | imperceptible (<1s) |
| Agent fixes surfaced violations same-turn | ≥ 80% of the time |
| Preferences captured per week | ≥ 1 that later fires or guides |
| Adherence score | flat or rising while you work normally |
| Your verdict | you'd install it on the *next* repo unprompted |

---

## Phase 5 — Iterate (the improvement loop)

Weekly, turn the friction log into action — in this order:

1. **False positives** → fix the rule's `ast_query`, add the FP snippet as a
   *pass* golden (regression guard), re-run `wh eval`, bump the pack version.
2. **Real bugs / UX friction** → `bd create` in the whetstone repo, one bead per
   distinct issue, with the log entry pasted in. Batch-fix, gate, release.
3. **Coverage gaps** → run the corpus-research flow for your top uncovered deps
   (research current docs → rules with verbatim `source_quote` → `wh eval` gate
   → new pack). Zero-yield deps are a valid outcome.
4. **Capture misses** → tune the SKILL.md capture trigger with the phrasings
   that failed; consider whether the hook should nudge ("this looks like a
   standing preference — capture it?").
5. **Pack currency** → first real re-validation cycle: re-fetch docs for one
   pack, confirm each `source_quote` still holds, bump `last_validated`.
6. Cut a release when a batch lands (protocol in CLAUDE.md).

**Kill criteria — be honest.** If after two weeks a surface is pure noise (never
changed agent behavior, or you always ignore it), file a bead to fix-or-freeze
it rather than letting it rot. The boundary doc's discipline applies to the
product itself: high confidence or silence.

---

## Known rough edges (so they don't surprise you)

- In-session enforcement is **Claude Code only** (Cursor has no hook API — it
  gets context files + MCP + `wh scan`).
- The corpus covers **10 deps**; anything else needs the extraction flywheel.
- Guidance capture requires the agent to have the skill/context loaded —
  session start matters.
- The hook is **advisory** by default; switch the installed hook script to
  `wh hook posttooluse --block` if agents ignore advisories.
- Homebrew tap not yet published; use `install.sh` or `wh update`.
