# Beads - AI-Native Issue Tracking

Welcome to Beads! This repository uses **Beads** for issue tracking - a modern, AI-native tool designed to live directly in your codebase alongside your code.

## What is Beads?

Beads is issue tracking that lives in your repo, making it perfect for AI coding agents and developers who want their issues close to their code. No web UI required - everything works through the CLI and integrates seamlessly with git.

**Learn more:** [github.com/steveyegge/beads](https://github.com/steveyegge/beads)

## Quick Start

### Essential Commands

```bash
# Create new issues
bd create "Add user authentication"

# View all issues
bd list

# View issue details
bd show <issue-id>

# Update issue status
bd update <issue-id> --status in_progress
bd update <issue-id> --status done

# Sync with the remote Beads store (Dolt-native workflow)
bd dolt push
bd dolt pull
```

### Canonical Local State For This Repo

This repository expects local Beads state to use:

- backend: `dolt`
- database name: `beads`
- git remote name: `origin`

Healthy local checks:

```bash
bd context --json
bd count
bd ready
bd dolt pull
```

If these fail because the local `.beads` directory is stale, corrupted, or points
at the wrong Dolt database, use the repair helper below.

### Working with Issues

Issues in Beads are:
- **Dolt-native**: Stored in the Beads database under `.beads/` and synced via Beads/Dolt remotes
- **AI-friendly**: CLI-first design works perfectly with AI coding agents
- **Branch-aware**: Issues can follow your branch workflow
- **Portable**: Can be bootstrapped on new machines with `bd bootstrap`

## Important Note For This Repo

This repository is migrating away from the historical `bd sync` / `beads-sync`
workflow. If you see older references to `.beads/issues.jsonl` or sync branches,
consider them legacy compatibility artifacts rather than the supported workflow.

The intended collaboration model is the current upstream Beads guidance:

- initialize with `bd init` / `bd init --team`
- bootstrap new clones with `bd bootstrap`
- sync shared Beads state with `bd dolt push` / `bd dolt pull`

In practice, this repo currently relies on a **deterministic repair/hydration
flow** for new or broken clones because `bd bootstrap` has been inconsistent
across machines.

### Supported onboarding on another device

From a fresh clone of the repo:

```bash
./scripts/beads-repair.sh --role contributor
bd ready
```

This recreates the local Dolt database from the remote Beads data and avoids
common local-state mismatches.

### Recovering a broken local Beads setup

If you see errors like:

- `database not found: beads`
- `bd bootstrap` says the database already exists but `bd doctor` still fails
- `bd dolt push` / `bd dolt pull` fail because the local database points at the wrong state

run:

```bash
./scripts/beads-repair.sh --role maintainer
```

The script:

1. stops the local Dolt server
2. backs up the existing local Dolt database under `.beads/backup/local-db-repair/`
3. rewrites `.beads/metadata.json` to the canonical `beads` database name
4. clones the remote Dolt data into `.beads/dolt/beads`
5. restarts the Dolt server and prints verification commands

### Normal day-to-day sync flow

```bash
bd dolt pull
# work with beads
bd dolt push
```

If `bd dolt push` reports a non-fast-forward or merge conflict, do **not** force
push. Pull first and resolve from a healthy canonical clone.

## Why Beads?

✨ **AI-Native Design**
- Built specifically for AI-assisted development workflows
- CLI-first interface works seamlessly with AI coding agents
- No context switching to web UIs

🚀 **Developer Focused**
- Issues live in your repo, right next to your code
- Works offline, syncs when you push
- Fast, lightweight, and stays out of your way

🔧 **Git Integration**
- Automatic sync with git commits
- Branch-aware issue tracking
- Intelligent JSONL merge resolution

## Get Started with Beads

Try Beads in your own projects:

```bash
# Install Beads
curl -sSL https://raw.githubusercontent.com/steveyegge/beads/main/scripts/install.sh | bash

# Initialize in your repo
bd init

# Create your first issue
bd create "Try out Beads"
```

## Learn More

- **Documentation**: [github.com/steveyegge/beads/docs](https://github.com/steveyegge/beads/tree/main/docs)
- **Quick Start Guide**: Run `bd quickstart`
- **Examples**: [github.com/steveyegge/beads/examples](https://github.com/steveyegge/beads/tree/main/examples)

---

*Beads: Issue tracking that moves at the speed of thought* ⚡
