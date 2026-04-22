# `wh debt` — AI-Code Debt Triage

> **Status:** v1 design · 2026-04-22
> **Owner:** whetstone-8hm
> **Related:** `planning/whetstone-overview.md` · `SKILL.md`

---

## Why this command exists

AI coding agents amplify code volume. The result inside most repos is consistent:
- Dead helpers the model generated but never wired up.
- Near-duplicate utilities across files because the model didn't know one existed.
- Manifest drift — imports the manifest doesn't declare, and declared packages nothing imports.
- A handful of files where every agent lands, every commit, and every bug — the hotspots.

Existing tools sprawl. Agents given open-ended "find tech debt" prompts produce noise, inflated to-do lists, and subjective judgments. Whetstone's advantage is **deterministic oracles**: we already walk manifests, parse AST, and run lint proxies. `wh debt` reuses that plumbing to produce a tight, ranked, AI-triaged debt report that another agent can act on with minimal token cost.

**Guiding principle** (from project memory): *lean over comprehensive*. Cut any detector that doesn't pull its weight. High confidence or silence.

---

## v1 detector slate (8hm.1.1)

Each detector is deterministic, backed by AST/manifest evidence, and cheap enough for routine use. Signal quality governs inclusion — dogfood (8hm.4) may trim further.

### 1. Dead code

- **`dead.unused_declared_deps`** — packages in the manifest that no source file imports.
  *Evidence:* manifest entry + zero matching imports across scanned files.
- **`dead.unreferenced_private_symbols`** — private/non-exported functions and types with zero references in the same package.
  *Languages:* Rust (pub(self) / non-`pub`), Python (leading `_`, module-private), TS (non-exported declarations).
  *Evidence:* AST definition site + reference count across project.
- **`dead.orphaned_module`** — a source file whose module/path is never imported by any sibling module.
  *Evidence:* file path + zero import hits across scanned files.

### 2. Duplicates

- **`dup.exact_block`** — normalized token-hash matching across files for blocks ≥ `MIN_LINES` (default 8).
  *Normalization:* strip comments and identifier-preserving whitespace; preserve token order.
  *Evidence:* cluster with representative snippet + file:line list.

### 3. Dependency hygiene

- **`deps.undeclared_import`** — source imports a package name that no manifest declares.
  *Evidence:* import statement + absent manifest entry.
- **`deps.unused_declared`** — same as `dead.unused_declared_deps` from the dep-hygiene lens (emitted once, routed under the dead-code bucket by default; this entry captures the hygiene framing in the schema).

### 4. Hotspots

- **`hotspots.churn_x_violations`** — files ranked by `git log --name-only` change count within a window × outstanding rule/lint violations touching that file.
  *Window:* last 90 days (configurable).
  *Evidence:* changes-in-window count + violation ids intersecting the file.

### Non-goals (explicit rejection)

These categories are either subjective, high-noise, or duplicate existing tools:

- **Cyclomatic complexity / "code smell" scoring.** Too subjective; linters already target the actionable subset.
- **LOC-based ranking.** Size is not debt.
- **Time estimates ("~4h to fix").** Unreliable; creates false precision.
- **Coupling / cohesion indexes.** High measurement cost, low actionability at this stage.
- **Test coverage gaps.** Coverage tooling already does this well.
- **Security / CVE scan.** Out of scope for v1 — dedicated tools handle this.
- **"Style" debt.** ruff / biome / clippy catch it.
- **Issue-per-violation spam.** We emit clusters, not raw violations.

---

## JSON schema — evidence envelope (8hm.1.2)

The command emits a single top-level object. Field names are stable across versions.

```json
{
  "schema_version": 1,
  "generated_at": "2026-04-22T10:15:30Z",
  "project_dir": "/abs/path",
  "summary": {
    "debt_label": "low | moderate | elevated | high",
    "hotspot_count": 12,
    "finding_count": 47,
    "by_category": { "dead": 18, "dup": 9, "deps": 7, "hotspots": 13 }
  },
  "hotspots": [
    {
      "id": "h1",
      "category": "dead | dup | deps | hotspots",
      "rule_id": "dead.unused_declared_deps",
      "title": "Unused declared dependency: requests-mock",
      "confidence": "high | medium",
      "rank": 1,
      "score": 8.4,
      "files": ["pyproject.toml"],
      "evidence": {
        "kind": "manifest_entry",
        "snippet": "requests-mock = \"^1.11\"",
        "references": 0,
        "locations": [
          {"file": "pyproject.toml", "line": 23}
        ]
      },
      "next_action": "Remove from pyproject.toml if truly unused; otherwise add the import it guards."
    }
  ],
  "notes": ["optional human-readable caveats"]
}
```

- `rule_id` uses the `<category>.<name>` namespace (mirrors rule YAML convention).
- `evidence.kind` is one of `manifest_entry`, `symbol_def`, `duplicate_cluster`, `orphaned_file`, `churn_violation_intersection`. Kind determines the other fields inside `evidence`.
- `next_action` is a **single** concrete sentence, human-readable. No numbered lists, no "consider" hedging.
- The same schema backs the prompt (8hm.3.2) and Beads (8hm.3.3) outputs — both modes project subsets of this object.

### Beads mode projection

A Beads run emits one epic (summary scope) and one child task per ranked hotspot cluster, *not* one per raw finding. Each child bundles up to N evidence items from the same cluster.

### Prompt mode projection

The `--prompt` output is a single markdown block, capped at ~2k tokens, containing top-K hotspots with terse evidence and next-action lines. No preamble, no file tree, no restating of the task.

---

## Ranking, confidence, and debt label (8hm.1.3)

### Per-finding score

```
score = base_weight[category] * evidence_strength * confidence_factor
```

- `base_weight`: `dead=1.0`, `dup=0.8`, `deps=1.0`, `hotspots=1.2`.
- `evidence_strength`:
  - `dead.unused_declared_deps`: `1.0` if zero import matches anywhere, else not emitted.
  - `dead.unreferenced_private_symbols`: `1.0` if ref count is zero; if one self-reference inside the same file, `0.6`.
  - `dup.exact_block`: `min(cluster_size / 2, 1.5)` — more duplicates = higher score, capped.
  - `deps.undeclared_import`: `1.0`.
  - `hotspots.churn_x_violations`: `min(changes * violations / 20, 2.0)`.
- `confidence_factor`: `high=1.0`, `medium=0.6`. Only `high` and `medium` are valid (no `low`).

### Ranking

Findings are ranked by descending `score`, then ascending `category` alphabetical for ties. Ranks are 1-indexed and stable across runs given the same inputs.

### Confidence assignment

- **`high`**: deterministic AST + exact evidence (zero references, zero imports, identical token hash, explicit manifest fact). Most detectors default here.
- **`medium`**: heuristic (duplicate threshold near the minimum, churn window near the edge, package name ambiguity due to import aliasing).
- No `low` tier — per the "high confidence or silence" principle, anything weaker is dropped.

### Repo-level debt label

A single label, not a number. Labels:

- **`low`** — fewer than 5 high-confidence findings, zero hotspots with score ≥ 1.5.
- **`moderate`** — 5–20 high-confidence findings, or 1–2 hotspots with score ≥ 1.5.
- **`elevated`** — 20–60 high-confidence findings, or 3–5 hotspots with score ≥ 1.5.
- **`high`** — more than 60 high-confidence findings, or more than 5 hotspots with score ≥ 1.5.

Rationale: fixed bands are interpretable and resistant to gaming. No hour estimates, no percentage "debt ratio." Dogfood (8hm.4) may retune bands.

### Gaming resistance

- `dead.unused_declared_deps` counts raw import matches across all files regardless of `#[cfg]` or conditional import — no easy way to suppress one path.
- Duplicate detection normalizes whitespace + comments, so reformatting doesn't mask duplication.
- Hotspots use `git log` over a fixed window, not recent edits alone.

---

## CLI surface

```
wh debt                            # human report, top 20 hotspots
wh debt --json                     # full JSON envelope
wh debt --prompt                   # compact remediation prompt (stdout)
wh debt --beads                    # emit an epic + child tasks into the local bd store
wh debt --top=N                    # cap hotspot list
wh debt --min-confidence=high      # drop medium-confidence findings
wh debt --since=90d                # churn window for hotspots
wh debt --project-dir=.            # standard flag
```

Exit codes:
- `0` — command ran.
- `1` — command failed (IO, git, manifest parse).
- No "dirty on debt" exit code. Debt is informational, not gating.

---

## Integration with existing commands

- `wh debt` reuses `src/detect/` for manifest parsing and `src/ast/` for symbol extraction.
- `wh debt` reuses `src/check/lint_proxy.rs` for violation intersections in hotspots.
- `wh debt` does not write state under `.state/` — it's read-only unless `--beads` is passed.
- `wh status` / `wh report` link to the latest debt summary if one has been generated this session, but do not run debt themselves.

---

## Out of scope for v1

- Cross-repo debt aggregation.
- Dashboard history (debt-over-time graphs).
- Per-PR debt diff (`wh debt --base=main`).
- Inline agent suggestions on each finding.
- Integration with external issue trackers other than Beads.

Tracked as follow-ups once v1 dogfood (8hm.4) lands.
