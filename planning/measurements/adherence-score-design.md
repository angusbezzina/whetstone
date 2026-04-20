# Adherence Score — Design Pass

> Tracked as `whetstone-m3k`. Blocks `whetstone-90m`.
> Goal: pick the formula that turns `wh check` violations into a 0–100 "how well does my code adhere to my rules?" number.

---

## Requirements

1. **Interpretable**: a user reading "87 / 100" should have a rough feel for how bad it is.
2. **Severity-aware**: a `must` violation hurts more than a `may` violation.
3. **Stable across project sizes**: a 100-file repo with one `must` violation shouldn't score the same as a 10-file repo with one.
4. **Cheap**: `wh status` must stay under 200 ms — the formula runs the existing `wh check` deterministic scan and a small arithmetic pass.
5. **Non-gameable**: splitting a rule into two signals shouldn't inflate the denominator.

---

## Candidate formulas

### Formula A — per-violation severity-weighted

```
weight(must) = 1.0;  weight(should) = 0.5;  weight(may) = 0.2
weighted_viols = Σ weight(rule.severity) over all violations
denom = max(1, |approved_rules| × |applicable_files|)   // scales with scope
raw   = 100 * (1 − weighted_viols / denom)
score = max(0, raw)
```

**Pros:** severity-aware. Denominator scales with project size AND rule count.
**Cons:** opaque ("why 87?"). A repo with zero applicable files scores 100 even if rules exist (not-evaluated ≠ clean).

### Formula B — per-file binary clean ratio

```
files_with_rules = files eligible for at least one rule (by language)
files_clean      = |{ f in files_with_rules : no violations for f }|
score            = 100 * files_clean / max(1, files_with_rules)
```

**Pros:** most interpretable ("73% of my Python files are clean"). Resistant to severity gaming.
**Cons:** severity-blind — one `may` violation penalizes the same as one `must`.

### Formula C — hybrid (recommended)

```
clean_component   = 100 * files_clean / max(1, files_with_rules)
penalty           = min(100, 100 * weighted_viols / max(1, |approved_rules| × |applicable_files|))
severity_component = 100 − penalty
score             = round(0.6 * clean_component + 0.4 * severity_component)
```

**Pros:** severity-aware (40% weight) + interpretable clean ratio (60% weight). Bounded 0–100. A clean repo scores 100 even with just a few files; a dirty repo with only `may` violations still drops noticeably (not just pinned at 100).
**Cons:** two-component formula is harder to explain in one line. We describe it as "60% files-clean / 40% severity-weighted."

---

## Dogfooding

Run each formula against three targets.

| Target | Files eligible | Approved rules | Violations (must / should / may) | A | B | C |
|--------|----------------|----------------|----------------------------------|----|----|----|
| `tests/fixtures` (small, clean) | 3 py + 2 ts | 3 (2 must, 1 should) | 0 / 0 / 0 | 100 | 100 | 100 |
| `.` (whetstone-self, many lints) | 40+ rs files | 7 (must + should) | many (see `wh check src/`) | ~60 | ~30 | ~42 |
| Hypothetical: 10-file project, 1 `must` viol on 1 file | 10 | 1 | 1/0/0 | 90 | 90 | 90 |
| Hypothetical: 100-file project, 1 `must` viol on 1 file | 100 | 1 | 1/0/0 | 99 | 99 | 99 |
| Hypothetical: 10-file project, 10 `may` viol (1/file) | 10 | 1 | 0/0/10 | 80 | 0 | 32 |

**Interpretation:** C penalizes the 10-file `may`-only case more than A (A rewards you unfairly) and less than B (B is too harsh for `may`-only). This is the desirable behavior: `may` violations should lower the score, but not equate to `must`.

Detailed whetstone-self dogfood numbers are captured in `planning/measurements/epic-3e-baseline.md` alongside each `wh status` run.

---

## Decision

**Adopt Formula C (hybrid).** Document the components in `wh status` output:

```
Adherence: 87 / 100  (clean 92% · severity-weighted 78)
Rule system: 88 / 100
```

### Known limitations

1. A project with **zero approved rules** scores N/A — not 100. `wh status` emits `adherence_score: null` and a warning, distinguishing "unmeasured" from "perfect."
2. **Rules with `ai` signals are skipped** (there are no `ai` signals post–lean refactor, but leave the guard for future restoration). Per-rule coverage in the denominator is the count of rules with at least one deterministic signal.
3. **Rule without applicable files** (e.g. a Python rule in a Rust-only repo) is excluded from the denominator for that language. Cross-language inflation is avoided.
4. File eligibility is determined by language extension (`*.py` / `*.ts` / `*.rs` / etc.) via the same inference as `wh rules query --file`.

### Implementation notes for `whetstone-90m`

- Run `wh check --no-fail --json` internally; parse violation list.
- Walk the project tree (filtered by language) to count eligible files. Reuse the walker from `wh init --detect-only`.
- Cache result in `.state/adherence.json` with a timestamp; invalidate after 60 s OR when any source file in the scan set has a newer mtime than the cache.
- `wh status --json` returns BOTH `adherence_score` and `rule_system_score` top-level; the pretty-print shows both on separate lines.

---

## Follow-ups

- If users want a way to weight rules non-uniformly (some rules "matter more"), add an optional `weight: float` to the rule schema. Not in Epic 3E scope.
- If denom inflation from signals-per-rule becomes a concern, switch the severity denominator to per-rule (not per-signal). Easy change.
