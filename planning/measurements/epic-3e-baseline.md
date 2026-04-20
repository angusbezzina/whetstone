# Epic 3E — Measurement Log

> Tracked as `whetstone-piy`. Part of Epic 3E (`whetstone-n34`).
> Re-run `scripts/measure-epic-3e.sh <project-dir> "<label>"` after each theme lands and append a row to the results table.

---

## Methodology

Metrics captured per measurement run (via `scripts/measure-epic-3e.sh`):

| Metric | How | Why it matters |
|--------|-----|----------------|
| **AGENTS.md bytes** | `wc -c` on `whetstone/context/AGENTS.md` after `wh actions` | Primary proxy for per-session agent token cost. |
| **~tokens** | `bytes / 4` (GPT-style rule-of-thumb; tokenizer-independent) | Lower bound on context cost. Real cost scales with rule count too. |
| **wh status runtime (ms)** | Average of 3 runs, no-snapshot + no-drift-check so network is not a factor | Time-to-gauge-repo-health. Target: ≤200ms. |
| **wh rules query runtime (ms)** | Average of 3 runs of `wh rules query --file <dummy.py>` | Per-turn JIT lookup cost. Target: ≤50ms. |

Metrics captured by narrative (re-measure after each theme lands):

| Metric | Measured how |
|--------|--------------|
| **Time-to-add-a-personal-preference (sec)** | Wall-clock from "I want to add X" to "X is approved". Today: write YAML + `wh extract submit` + `wh approve`. Target after 9uh: one `wh rule add --personal` call. |
| **Time-to-gauge-repo-health (sec, perceived)** | From "is my code in good shape?" to a code-quality number. Today: `wh status` returns a rule-system number, not a code-quality one; effectively unsupported. Target after 0m0+90m: `wh status` returns `adherence_score` in one command. |

---

## Baseline — 2026-04-20

Captured before any Epic 3E theme landed (commit `324f1ad` — `wh rules query` had already shipped at this point; treat the `wh rules query` latency as a lower bound, not a "before" measurement).

| Label | Project | AGENTS.md bytes | ~tokens | wh status | wh rules query |
|-------|---------|-----------------|---------|-----------|----------------|
| 2026-04-20 baseline (fixtures) | tests/fixtures | 1434 | 358 | 11.4ms | 7.0ms |
| 2026-04-20 baseline (whetstone self) | . | 4058 | 1014 | 11.3ms | 7.3ms |

### Narrative baselines

| Metric | Baseline observation |
|--------|----------------------|
| Time-to-add-a-personal-preference | 3-step flow (hand-write YAML → `wh extract submit` → `wh approve`). Measured as ~3–5 minutes for a simple preference with an existing regex pattern in hand. Drastically longer if the user has to figure out the schema. |
| Time-to-gauge-repo-health | `wh status` returns a score, but it reflects rule-system health, not code quality. A user asking "is my code adhering to my rules?" today has to mentally combine `wh status` + `wh check` outputs. Effectively *not answerable* in one command. |

---

## Delta targets (epic-level acceptance gate)

Epic 3E (`whetstone-n34`) does not close until:

| Metric | Target | Why |
|--------|--------|-----|
| **Typical session token cost** | ↓ ≥40% on the whetstone-self project | Proven if agents adopt `wh rules query` mid-turn (SKILL.md teaches this). Concretely: either AGENTS.md halves in size via `--terse` (`whetstone-ydw`), or agents stop preloading it (qualitative, tracked via conversation transcripts). |
| **Time-to-add-personal-preference** | ↓ ≥60% | After `whetstone-9uh` (`wh rule add --personal`), a personal preference is one CLI call with no YAML authoring. |
| **Time-to-gauge-repo-health** | ≤1 command, code-quality number | After `whetstone-0m0` + `whetstone-90m`, `wh status` returns `adherence_score` alongside `rule_system_score`. |
| **wh status runtime** | ≤200ms on whetstone-self | Performance guardrail — folding `wh check` into `wh status` (`whetstone-0m0`) must not regress runtime. |

Formal revision of these targets is acceptable with rationale recorded here; abandoning them without rationale is not.

---

## Per-theme results (append as each theme lands)

| Date | Theme | Project | AGENTS.md bytes | ~tokens | wh status | wh rules query | Notes |
|------|-------|---------|-----------------|---------|-----------|----------------|-------|
| 2026-04-20 | (baseline) | tests/fixtures | 1434 | 358 | 11.4ms | 7.0ms | pre-epic |
| 2026-04-20 | (baseline) | . | 4058 | 1014 | 11.3ms | 7.3ms | pre-epic |
| 2026-04-20 | post-ydw+2gw (non-terse) | tests/fixtures | 1515 | 378 | 9.2ms | 5.6ms | includes sidecar-hint overhead |
| 2026-04-20 | post-ydw+2gw (non-terse) | . | 4058 | 1014 | 8.2ms | 5.6ms | unchanged; terse is opt-in |
| 2026-04-20 | post-ydw+2gw (`--terse`) | tests/fixtures | 926 | 231 | — | — | **−35%** vs baseline |
| 2026-04-20 | post-ydw+2gw (`--terse`) | . | 1967 | 491 | — | — | **−51.5%** vs baseline ✓ meets ≥40% target |

### How to append a row

```bash
# Rebuild the binary with the theme's changes.
cargo build --release

# Run the measurement script; append the stdout markdown row to the table above.
scripts/measure-epic-3e.sh . "2026-MM-DD post-<theme>"

# Also record any narrative changes (time-to-add-rule, time-to-gauge-health).
```

---

## Dogfood gate

Epic 3E additionally closes only after at least one external repo has been run through the full loop (`wh init` → `wh extract submit` → `wh approve --all` → `wh actions` → `wh check` → `wh reinit`) with no crash-path failures. Target repo TBD; pick something small with 3–5 deps across at least 2 languages. Record findings in this file.
