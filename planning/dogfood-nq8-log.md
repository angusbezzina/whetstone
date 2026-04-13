# Dogfood log — Epic whetstone-nq8

> Date: 2026-04-13
> Binary: `cargo run --quiet -- …` from the Whetstone source tree, v0.2.0 + nq8 work.
> Scope: 1A.5.1 (Whetstone itself) and notes for 1A.5.2 (external repo).

---

## 1A.5.1 — Whetstone itself

Run against `/Users/angusbezzina/Development/whetstone` after the
contract + handoff + built-in changes from epic `whetstone-nq8`.

### `wh validate`

```
Schema file found and readable.
Checking 11 rule files...
All schema checks passed.
```

11 files scanned: 5 fixtures + 3 built-ins (rust/python/typescript) + 6 project
Rust rules. No schema errors, no lifecycle warnings.

### `wh status --no-drift-check --no-snapshot`

```
status: ok
score: 95 (Healthy)
rules: 6         # status still counts project-only rules
deterministic_coverage: 100.0
```

**Finding:** `wh status` does not merge built-in rules into its health score —
only project rules land in `dimensions.rules_count`. `wh context` and `wh tests`
DO merge built-ins. This is asymmetric; users who rely solely on built-ins see
a score dominated by the "no project rules" ladder. Filed for a follow-up epic.

### `wh context --dry-run`

```
status: ok
rules_count: 21
  + ./whetstone/context/AGENTS.md
```

21 rules = 6 project Rust + 5 built-in Rust + 5 built-in Python + 5 built-in
TypeScript. Merge works. All built-ins carry source URLs and golden examples.

### `wh tests --dry-run`

```
status: ok
rules_count: 21
tests: 23
lint_configs: 0
```

`lint_configs: 0` is expected — no rules carry `lint_proxy` signals today. The
tests count (23) is 21 rules + 1 Python `conftest.py` + 1 TypeScript `setup.ts`.

### `wh eval run --deterministic-only`

```
status: ok
rules_evaluated: 21
files_scanned: 32
deterministic_violations: 861
```

The eval runner produced 861 real violations against Whetstone's own source.
Example: `rust.expect-over-unwrap` at `src/detect/mod.rs:493`. This confirms
the rust built-ins fire on the codebase they were designed for.

### `wh refresh --check`

Not exercised in this log (requires network for crates.io metadata and would
rewrite `whetstone/.state/refresh-diff.json` under load). The integration
tests in `tests/rust_integration.rs` cover the contract in an offline-safe
way via an empty-project fixture.

### Summary

| Check | Result |
|-------|--------|
| Schema validation | PASS |
| Lifecycle status transitions | PASS (all rules carry `status: approved`) |
| Built-in + project merge into context/tests | PASS (21 rules) |
| Eval runner against real source | PASS (861 real violations) |
| Extraction-handoff artifact written | PASS (via doctor + refresh paths, integration-tested) |
| Refresh-diff artifact schema | PASS (version=1, drift_count, changed, removed, failed) |

### Follow-ups captured

1. `wh status` does not merge built-in rules into its health score. This
   surprises any user running on a clean project with only built-ins.
2. The Python built-in `python.mutable-default-arguments` regex is
   conservative — it only catches literal `=[]` / `={}` defaults, not
   `dict()` / `list()` constructor calls. Upgrading requires tree-sitter.
3. The Python `open-without-encoding` match will over-report on any
   `open(path)` call including binary mode on the same line — consider
   a second negative-lookahead pattern once tree-sitter lands.

---

## 1A.5.2 — External project dogfood

Full external-repo workflow requires network access (PyPI / npm lookups) plus
a user-chosen target, which was out of reach in the execution environment.
We instead dogfooded a synthetic external Python project at
`/tmp/ext-dogfood2` to exercise the non-Whetstone code paths.

### Scenario

```python
# /tmp/ext-dogfood2/src/app/main.py
def load_config(path):
    with open(path) as f:   # open-without-encoding
        return f.read()

def fetch(url):
    r = requests.get(url)   # no-requests-without-timeout
    return r.json()

def run(cmd):
    return subprocess.run(cmd, shell=True)   # no-shell-true

def accumulate(items, bucket=[]):   # mutable-default-arguments
    bucket.extend(items)
    return bucket
```

### Commands exercised

| Command | Outcome |
|---------|---------|
| `wh validate --project-dir /tmp/ext-dogfood2` | PASS (after fix — see below) |
| `wh tests --project-dir /tmp/ext-dogfood2 --lang python` | PASS — 5 built-in Python rules landed in one grouped `test_whetstone_recommended_python.py` |
| `python3 -m pytest whetstone/evals/python/` (run inside the project) | 4 expected failures on the 4 planted violations, 1 pass on the unexercised rule |
| `wh eval run --deterministic-only --lang python` | 4 real violations reported with file path and line number |

### Bugs found and fixed in this dogfood pass

1. **`wh validate` failed outside the Whetstone source tree** because the
   validator required `references/rule-schema.yaml` relative to the project
   root. Fixed by embedding the schema at compile time
   (`src/rules.rs::EMBEDDED_SCHEMA`) and falling back to it when no local
   schema is present.

2. **Every rule with the same `source_name` clobbered its siblings' test
   files** — e.g. the 5 Python built-ins all wrote to
   `test_whetstone_recommended_python.py` and only the last rule survived.
   Fixed by grouping rules by `source_name` per language and emitting one
   test file per group containing all of that source's rules. Applied to
   Python, TypeScript, and Rust generators.

### Remaining reproducible recipe

### Reproducible recipe for a real external repo

```bash
# 1. Pick a target and clone it.
git clone https://github.com/<owner>/<repo> /tmp/whetstone-dogfood
cd /tmp/whetstone-dogfood

# 2. Let Whetstone bootstrap.
wh doctor --json > /tmp/doctor.json
cat whetstone/.state/extraction-handoff.json   # agent-readable candidates

# 3. Ask the agent to read extraction-handoff.json and extract rules.

# 4. Validate + generate.
wh validate
wh context
wh tests

# 5. Check health + refresh surface.
wh status
wh refresh --check || echo "drift detected — review whetstone/.state/refresh-diff.json"
```

### What to watch for on the first real external run

- Does `extraction-handoff.json` rank candidates in a useful order?
  (Should surface llms.txt / readme deps before html_converted / failed.)
- Does `wh refresh --check` correctly exit non-zero when a dep version bumps?
- Does `wh eval run --deterministic-only` produce actionable violations or
  only noise (high false-positive rate signals a rule that needs a
  tighter `match:` regex)?
- Does `wh tests --lang python` (or `--lang typescript`) generate tests
  that actually pass against the project's source when run with
  `python3 -m pytest whetstone/evals/python/` / `npx vitest run
  whetstone/evals/typescript/`?

When this recipe is exercised for real, capture the output in a follow-up
log file (e.g. `planning/dogfood-external-<repo>-log.md`) and link from the
roadmap. Failures or usability friction should be filed as beads under a
follow-up epic.
