# Layer System & Trigger Modes

> Planning document for a future epic. Not in scope for the current product-loop epic.
> Created: 2026-04-05

---

## Overview

The layer system gives Whetstone a cascading rule model — like git config or ESLint — where more specific scopes override broader ones. Combined with trigger modes, it enables Whetstone to scale from solo developer to org-wide policy without changing the core rule format.

---

## Layer Model

### Cascade Order

```
personal > project > team > built-in
```

A personal deny overrides a team MUST. A project rule overrides a built-in. Resolution follows the most-specific-wins pattern.

### Layer Definitions

| Layer | Location | Committed | CI Visible | Purpose |
|-------|----------|-----------|------------|---------|
| **Built-in** | Ships with the binary | — | Via generated tests | Curated language-level best practices (see built-in rules in core epic) |
| **Team / Org** | Standalone repo or package | In its own repo | Via `extends` | Org-wide standards shared across projects |
| **Project** | `<repo>/whetstone/` | Yes | Yes | Project-specific rules derived from project deps |
| **Personal** | `<repo>/whetstone/.personal/` | No (gitignored) | No | Individual preferences, local-only |

### Layer Behaviour

**Built-in layer:**
- Ships as embedded YAML in the binary (compiled in via `include_str!` or similar)
- Versioned with Whetstone releases
- Users can override severity or deny any built-in rule at the project or personal layer
- Think `extends: whetstone:recommended` — a strong baseline that earns its place
- Built-in rules are created as part of the core epic; layer *integration* is this epic's concern

**Project layer (current default):**
- Rules in `whetstone/rules/**/*.yaml` — this is what exists today
- All generated outputs (tests, lint configs, agent context) derive from this layer + built-in
- Committed to the repo, enforced in CI

**Personal layer:**
- Rules in `whetstone/.personal/rules/**/*.yaml`
- `.personal/` directory automatically added to `.gitignore` by `whetstone init --personal`
- Generated personal tests go to `whetstone/.personal/evals/`
- Generated personal lint configs go to `whetstone/.personal/lint/`
- `pytest whetstone/evals/` locally runs both project + personal tests (directory traversal); CI only sees committed project tests
- Agent context files are generated from **committed rules only** (project + team + built-in). Personal rules do NOT appear in AGENTS.md, CLAUDE.md, .cursorrules that get committed
- Personal agent context can optionally generate to `whetstone/.personal/claude.md` etc. for local agent use
- Personal sources in `whetstone/.personal/sources.yaml`

**Team layer:**
- A standalone repo or published package containing shared rules, sources, and settings
- Projects reference it via `extends` in `whetstone.yaml`:
  ```yaml
  extends:
    - whetstone:recommended        # built-in baseline
    - my-org/whetstone-config      # team standards
    - @someuser/fastapi-strict     # community config (future registry)
  ```
- Multiple extends supported, later entries override earlier
- Team configs can define rules, source lists, severity overrides, and deny lists
- Team rules are fetched and cached locally during `whetstone init` or `whetstone update`

### Merge Logic

```
final_rules = {}

# 1. Start with built-in rules
for rule in built_in_rules:
    final_rules[rule.id] = rule

# 2. Apply team layers (in extends order)
for team_config in extends:
    for rule in team_config.rules:
        final_rules[rule.id] = merge(final_rules.get(rule.id), rule)
    for deny in team_config.deny:
        final_rules.remove(deny)

# 3. Apply project rules
for rule in project_rules:
    final_rules[rule.id] = merge(final_rules.get(rule.id), rule)
for deny in project_deny:
    final_rules.remove(deny)

# 4. Apply personal rules (local only, not for committed output)
for rule in personal_rules:
    final_rules[rule.id] = merge(final_rules.get(rule.id), rule)
for deny in personal_deny:
    final_rules.remove(deny)
```

The `merge()` function takes the more-specific layer's values for any field that's explicitly set. Unset fields inherit from the broader layer. This allows a team to set severity=must and a project to override to severity=should without redefining the entire rule.

### Deny Lists

Each layer can deny rules by ID:

```yaml
# whetstone.yaml (project level)
deny:
  - whetstone.python.generic-exceptions    # too noisy for this codebase
  - my-org.require-docstrings              # team rule we disagree with here
```

A denied rule is fully removed from the merged set. It doesn't generate tests, lint configs, or agent context.

---

## Promote Command

`whetstone promote <rule-id> --to <layer>` moves a rule between layers.

### Use Cases

- A personal preference proves valuable → promote to project standard
- A project rule should be org-wide → promote to team config
- A built-in rule needs local customisation → copy to project layer (override, not promote)

### Mechanics

1. Read the rule from source layer
2. Write to destination layer's rules directory
3. Optionally remove from source layer (or leave as override)
4. Regenerate affected outputs in both layers

### Constraints

- Can only promote "up" (personal → project → team). Copying "down" (team → project) is an override, not a promotion.
- Promoting to team requires write access to the team config repo
- Agent context only regenerates from committed layers

---

## Trigger Modes

Configured in `whetstone.yaml`:

```yaml
trigger:
  mode: manual          # manual | session | post-merge | scheduled
  schedule: "weekly"    # Only for scheduled mode
  auto_patterns: true   # Run detect-patterns in background on session triggers
```

### Manual (Default — exists today)

User explicitly invokes `whetstone doctor`, `whetstone update`, etc. No automation.

### Session Start Hook

A Claude Code hook that runs a lightweight check on every session start:

```json
{
  "hooks": {
    "SessionStart": [
      {
        "matcher": "startup",
        "hooks": [
          {
            "type": "command",
            "command": "whetstone status --json --quiet",
            "timeout": 10000,
            "async": true,
            "statusMessage": "Whetstone: checking rule freshness..."
          }
        ]
      }
    ]
  }
}
```

**Behaviour:**
- Runs `whetstone status --json --quiet` asynchronously
- If status is `stale` or `needs_review`, surfaces a brief summary to the agent context
- If `auto_patterns: true`, also runs `whetstone patterns --since-last-run --quiet`
- Does NOT auto-extract or auto-update — just surfaces awareness

**Setup:** `whetstone init` writes the hook config to `.claude/settings.json` (or equivalent for other agents). User confirms.

### Post-Merge Hook

A git hook that checks for dependency drift after pulling changes:

```bash
#!/bin/bash
# .githooks/post-merge
# Installed by: whetstone init --hooks
whetstone init --json --quiet | jq -e '.drift.count > 0' > /dev/null 2>&1 && \
  echo "⚠ Whetstone: $(whetstone init --json --quiet | jq -r '.drift.count') dependencies have drifted since rules were last extracted. Run 'whetstone update' to review."
```

**Behaviour:**
- Runs after `git pull` or `git merge`
- Checks if manifest versions have changed relative to approved rule versions
- Prints a one-line warning if drift detected
- Does NOT block the merge — advisory only

**Setup:** `whetstone init --hooks` installs to `.githooks/post-merge` and sets `core.hooksPath`.

### Scheduled (CI)

A GitHub Actions workflow (or equivalent) that runs periodic freshness checks:

```yaml
# .github/workflows/whetstone-check.yml
name: Whetstone Freshness
on:
  schedule:
    - cron: '0 9 * * 1'  # Every Monday at 9am
  workflow_dispatch:

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Install Whetstone
        run: curl -fsSL https://raw.githubusercontent.com/.../install.sh | sh
      - name: Check freshness
        run: whetstone status --json
      - name: Check drift
        run: whetstone init --check-drift --json
      - name: Post summary
        if: always()
        run: whetstone ci --pr-comment >> $GITHUB_STEP_SUMMARY
```

**Behaviour:**
- Runs on a schedule (configurable: daily, weekly, biweekly, monthly)
- Reports freshness status and drift
- Posts summary to GitHub Actions step summary
- Optionally opens an issue if status is `stale`

**Setup:** `whetstone init --ci` generates the workflow file. User commits it.

---

## Config Implications

The layer system expands `whetstone.yaml`:

```yaml
# Current (v0.1.1)
discovery:
  exclude: [node_modules, target]
  include: []
generate:
  formats: [agents.md, claude.md, .cursorrules]

# Layer system additions
extends:
  - whetstone:recommended
  - my-org/whetstone-config

deny:
  - rule.id.to.deny

personal:
  enabled: true                    # Create .personal/ on init
  generate_local_context: false    # Generate personal agent context files

trigger:
  mode: manual
  schedule: weekly
  auto_patterns: true
```

### Global Personal Config

`~/.whetstone/config.yaml` — applies to all projects:

```yaml
default_languages: [python, typescript]
default_formats: [claude.md, agents.md]
sources:
  - url: https://my-team-style-guide.internal/llms.txt
    name: "Team style guide"
```

This file is read during `whetstone init` and merged as defaults (project config overrides).

---

## Implementation Considerations

### Team Config Resolution

Team configs referenced via `extends` need to be fetched. Options:
1. **Git clone** — `extends: github.com/my-org/whetstone-config` clones the repo, reads rules
2. **HTTP fetch** — `extends: https://my-org.com/whetstone-config.yaml` fetches a single file
3. **Package reference** — `extends: @my-org/whetstone-config` resolves via a registry (future)

For the initial implementation, git clone is simplest and most flexible. Cache the clone locally in `whetstone/.cache/teams/`.

### Personal Layer Isolation

The key invariant: **personal rules never leak into committed outputs.**

- `generate-context` reads only from project + team + built-in layers for committed output
- `generate-tests` writes project tests to `whetstone/evals/`, personal tests to `whetstone/.personal/evals/`
- `generate-context --personal` writes to `whetstone/.personal/context/` (opt-in)
- `.gitignore` management is automatic

### Migration Path

Projects using Whetstone v0.1.x (no layers) should seamlessly upgrade:
- Existing `whetstone/rules/` becomes the project layer (no file moves)
- `whetstone:recommended` built-in layer is additive (new rules, not breaking)
- Personal layer is opt-in (`whetstone init --personal`)
- Team layer is opt-in (`extends:` in config)

---

## Dependencies on Core Epic

The layer system depends on several items from the current product-loop epic:

| Dependency | Why |
|-----------|-----|
| Candidate management system | Layers need to know where to stage and approve rules |
| Built-in rule system | Built-in is the base layer in the cascade |
| whetstone.yaml config expansion | `extends`, `deny`, `personal`, `trigger` fields |
| Generate pipeline validation | Must work correctly before adding layer complexity |

---

## Estimated Scope

| Item | Complexity | Notes |
|------|-----------|-------|
| Personal layer | Medium | Mostly directory management + gitignore + output routing |
| Layer merge logic | Medium | Rule merge, deny lists, override semantics |
| Promote command | Low | File move + regenerate |
| Team config resolution | High | Git clone, caching, versioning, auth |
| Trigger: session hook | Low | Config file generation |
| Trigger: post-merge hook | Low | Shell script generation |
| Trigger: scheduled CI | Medium | Workflow file generation + status reporting |
| Global personal config | Low | File reading + merge with project config |

**Recommended build order:**
1. Personal layer (immediate value for solo devs)
2. Layer merge logic (enables team layer)
3. Promote command
4. Trigger: session hook (quick win)
5. Trigger: post-merge hook (quick win)
6. Team config resolution
7. Trigger: scheduled CI
8. Global personal config
