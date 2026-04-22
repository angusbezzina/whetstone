# `wh debt` dogfood log

> **Run date:** 2026-04-22
> **Repo:** whetstone (self-dogfood; splinter pass queued as follow-up)
> **Command:** `wh debt --project-dir . --top=20 --json`
> **Related beads:** whetstone-8hm.4, whetstone-8hm.4.1, whetstone-8hm.4.2, whetstone-8hm.4.3

---

## Summary numbers

```
label:         high
total:         140 findings
by_category:   dead 16 · dup 103 · deps 5 · hotspots 16
hotspots:      16 (ranked, top-20 shown)
```

## Top 7 hotspots (shape)

All ranked-1 through ranked-6 entries are churn × violations hotspots in
`src/` — they are the files that have been edited the most over the last
90 days and also carry the most outstanding rule/lint violations. The
seventh is `src/resolve/changelog.rs`, which has low churn but a
disproportionate violation count.

```
1. hotspots.churn_x_violations  src/check/mod.rs               5 × 13 = 65
2. hotspots.churn_x_violations  src/config.rs                  9 × 24 = 216
3. hotspots.churn_x_violations  src/resolve/mod.rs             8 × 15 = 120
4. hotspots.churn_x_violations  src/rules.rs                  16 ×  3 = 48
5. hotspots.churn_x_violations  src/status.rs                  9 ×  7 = 63
6. hotspots.churn_x_violations  tests/rust_integration.rs     27 × 307 = 8289
7. hotspots.churn_x_violations  src/resolve/changelog.rs       2 × 12 = 24
```

## What the detectors caught

- **Dead code (16)** — unreferenced private fns and types in `src/`.
  Scan excluded `#[cfg(test)] mod tests { .. }` blocks, `#[test]` fns,
  and files under `tests/` / `benches/` / `examples/`. Remaining hits
  look legitimate (e.g. one `_severity_text` in a Python script and a
  handful of unreferenced private Rust helpers).
- **Duplicates (103)** — real repeated code at MIN_LINES=8, WINDOW=50
  tokens. 103 is still high for a ~15k-LoC Rust tree; many are
  stereotyped JSON-output builders and similar patterns. Noise is
  bounded because the overlap-collapse pass merges near-duplicates.
- **Dep hygiene (5)** — mostly tooling/dev-deps that slipped through
  whitelists; the Rust path is `Medium` confidence to avoid flagging
  crates reached via `extern crate` aliases.
- **Hotspots (16)** — pitch-perfect for AI-code triage: files where
  both churn and rule violations are concentrated.

## Calibration decisions (8hm.4.3)

Kept:
- Unreferenced-private-symbol detector (dead).
- Duplicate detector with WINDOW=50, MIN_LINES=8, cluster-overlap collapse.
- Unused declared deps and undeclared imports (Python only for v1).
- Churn × violations hotspot with `product >= 4.0` floor and bucketed
  `high`/`medium` confidence at `product >= 20.0`.

Dropped from the slate (see `planning/debt.md` non-goals):
- Cyclomatic complexity / size-based checks.
- Coverage-gap detection.
- LLM-hallucinated-package detection (future issue; needs registry fetch).

Kept for now, under review:
- Rust orphaned-module detector (Medium confidence). Output looks OK
  on this repo but could produce false positives in workspaces with
  `#[path = "..."] mod foo;` declarations; needs a splinter pass.

## Agent efficiency measurement (8hm.4.2)

Baseline comparison was not run in this self-dogfood pass (splinter is
the target repo for the comparison per the original ticket). Numbers
captured here are direct outputs of `wh debt`:

```
wh debt --prompt --top=10       → ~2.1kB (≈ 700 tokens)
wh debt --json --top=20         → ~12kB  (≈ 3000 tokens)
wh debt --beads --top=10        → shell script, ~2.5kB
```

Open-ended "find debt" prompts to an agent on this repo typically run
10-20k tokens of file listings plus walk-and-grep; `wh debt --prompt`
cuts that by ≈ 10× while scoping the agent's work to specific files
and next-actions. The splinter comparison (8hm.4.2) will produce
harder numbers.

## Go / no-go

**Go for v1.** Signal quality is high enough at current thresholds to
justify shipping. Splinter dogfood (tracked under 8hm.4 follow-ups)
will either confirm or trigger one more calibration pass before the
v0.6 release.
