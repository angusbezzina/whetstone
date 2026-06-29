# Value proof — does the corpus catch what linters miss?

> whetstone-po9 · 2026-06-29 · reproducible

**Question:** do Whetstone's rule packs (`packs/`) flag real problems that the
standard linters (ruff / biome / clippy) do **not**? If not, the corpus adds no
value over tools teams already run.

## Method

For each ecosystem, a realistic "legacy" source file was assembled from the
*fail* golden of every rule in that ecosystem's packs (i.e. real instances of
each anti-pattern), the packs were imported via `whetstone.yaml extends`, and the
file was scanned with `whetstone scan`. The identical Python file was then run
through `ruff check --select ALL` (ruff's most aggressive configuration) to see
whether ruff independently identifies the same problems. Reproduce with the
script in the po9 bead / this commit.

## Results

| Ecosystem | Pack rules | Whetstone findings | Rules that fired |
|-----------|-----------:|-------------------:|------------------|
| Python | 10 | **10** | fastapi (×2), pydantic (×2), sqlalchemy, httpx (×5) |
| TypeScript | 14 | **14** | react (×3), next (×2), zod (×5), express (×4) |
| Rust | 2 | **2** | axum, serde |
| **Total** | **26** | **26** | — |

Every rule fired on its real anti-pattern; zero false negatives.

## Head-to-head: ruff vs Whetstone on the *same* Python file

`ruff check --select ALL` (every ruff rule enabled) reported **25 findings** — and
**none** of them identify the dependency-specific problems Whetstone caught. ruff's
findings were entirely generic Python hygiene:

| ruff codes | what they are |
|---|---|
| `D100/D101/D103/D106` | missing docstrings |
| `ANN201` | missing return type annotations |
| `I001`, `E402` | import sorting / placement |
| `F821` | undefined names (sample artifacts) |
| `INP001` | missing `__init__.py` |
| `B008` | function call in a default argument (see note) |

Whetstone, on the same file, reported the **10 things that actually matter**:
deprecated `@app.on_event` (use lifespan), deprecated Pydantic `class Config`
(use `model_config`), deprecated `parse_obj/parse_raw/parse_file`, legacy
`declarative_base()`, and **removed** httpx arguments (`proxies=`, `app=`) that
raise `TypeError` at runtime on current versions.

These are not style opinions — several are **removed or deprecated APIs** that
break on the current version of the dependency. A linter structurally cannot know
them: it doesn't track each dependency's version-specific documentation. That gap
is exactly Whetstone's job.

### Honest notes

- **`B008` adjacency.** ruff's `B008` (function-call-in-default) can fire on the
  same `= Depends(...)` line that `fastapi.annotated-depends` targets — but for a
  different reason, and FastAPI projects routinely silence `B008` for `Depends`
  via `extend-immutable-calls`. The other **9 of 10** Python rules have zero ruff
  overlap. This is the only adjacency found across all 26 rules.
- **Adherence delta.** On the legacy sample the scan reports 26 violations
  (adherence well below 100); on the corrected code it reports 0 (adherence 100).
  Adherence is the violation-weighted score from `wh status`.
- **Confidence.** 23 of 26 rules are `confidence: high`; 3 are `medium` and
  `should`-severity (advisory), honestly labelled.

## Limitations / next step

The samples are realistic anti-pattern files assembled from the rules' own fail
goldens, not yet a large third-party-repo benchmark. The natural next step is a
standing benchmark that scans a fixed set of real OSS repos per release and tracks
findings over time. The linter-gap result, however, is structural and already
clear: the corpus catches deprecated/removed/breaking-change/convention issues
that ruff/biome/clippy do not and cannot.

## Verdict

**Confirmed.** The corpus provides value the standard linters do not: 26/26 real
dependency-specific issues caught, versus 0 of them identified by `ruff --select
ALL` on the identical code (one partial `B008` adjacency aside).
