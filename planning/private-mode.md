# Private mode — solo adoption with zero shared-repo footprint

> Bead: `whetstone-xdr` (epic) / `whetstone-ejx` (this design). Written 2026-07-28.
>
> The beads-style model: one team member adopts Whetstone and **nothing appears in
> `git status`** for their teammates to see or accidentally commit. When the team
> is ready, `wh publish` flips the same artifacts into normal, trackable files.

## The problem

`wh init --claude` writes `whetstone/**`, `.mcp.json`, `.claude/settings.json` +
hook scripts, and `.githooks/post-merge` at the repo root — untracked but NOT
ignored. On a team repo where only one person wants Whetstone, one careless
`git add .` ships all of it. There is no way to trial Whetstone on a shared repo
without a visible footprint.

## The mechanism: a managed block in `.git/info/exclude`

`.git/info/exclude` behaves exactly like `.gitignore` but is **per-clone and never
committed** — the canonical git feature for personal ignores. Private mode writes a
fenced, machine-managed block there; `wh publish` removes exactly that block.

```
# >>> whetstone private mode [.] (managed by `wh`; `wh publish` removes this block) >>>
/whetstone/
/.mcp.json
/.claude/settings.json
/.claude/settings.local.json
/.claude/whetstone-session-hook.sh
/.claude/whetstone-posttooluse-hook.sh
/.cursor/whetstone-session.md
/.githooks/post-merge
# <<< whetstone private mode [.] <<<
```

**Blocks are labelled with the project's path** (`[.]` at the repo root,
`[packages/api]` for a package). One repo can have several private packages at
once: `enable` and `publish` only ever read and write their own label, so
onboarding a second package never re-exposes the first. Glob metacharacters in
a real directory name (`pkg[1]`) are escaped in the entries — `.git/info/exclude`
is gitignore syntax, and an unescaped `[` matches nothing.

The path is resolved via `git rev-parse --git-path info/exclude` so worktrees and
non-standard git dirs work. Entries are static (excluding an already-tracked path
is a harmless no-op — exclude only affects untracked files), which keeps
enable/publish exactly inverse operations.

**The exclude file lives at the repo root**, so every entry is anchored under the
project's path relative to that root (`repo_prefix`, from
`git rev-parse --show-toplevel`). A package inside a monorepo gets
`/packages/api/whetstone/`; without the prefix the entries would match nothing
and expose every artifact while reporting success.

**The block is self-healing.** `enable` compares the block on disk to what it
would write now and replaces it on any mismatch — a torn write, or a block left
by a different project directory. Trusting the `>>>` marker alone would silently
leave artifacts exposed on a re-run. Both the exclude file and the
`whetstone.yaml` marker are written atomically (temp + rename).

## Decisions (locked)

1. **Small cut, not the root refactor.** Artifacts stay at their current paths;
   only their git *visibility* changes. The out-of-repo `WhetstoneRoots` design
   (`~/.whetstone/projects/<hash>/`) stays future work on the epic.
2. **Marker: `setup.private: true`** in `whetstone/whetstone.yaml`, written via the
   same round-trip helpers as `setup.dismissed`. The file itself is excluded, so
   the marker is invisible to teammates. Oracles read it with
   `private_mode::is_private()`.
3. **Tracked files are never modified in private mode.** `.git/info/exclude`
   cannot hide changes to tracked files — and for a committed hook script,
   overwriting it would destroy a teammate's content, which is data loss, not
   just a leak. Every artifact private mode can write is guarded via
   `private_mode::skip_tracked`:
   - `whetstone/` already tracked → private mode **refuses to enable** (the repo
     is already publicly onboarded; private mode is a pre-adoption state).
   - `.mcp.json` tracked → `register_mcp` skips it and reports the local-scope
     alternative (`claude mcp add whetstone -s local -- wh mcp --project-dir .`).
   - `.claude/settings.json` tracked → hooks go to `.claude/settings.local.json`
     (Claude Code's per-user overlay) instead; `wh publish` migrates them back
     and deletes the overlay if nothing of the user's remains in it.
   - `.githooks/post-merge`, `.cursor/whetstone-session.md`, and both
     `.claude/whetstone-*.sh` scripts tracked → skipped (already shared).

   Related, and not limited to private mode: **`core.hooksPath` is never set
   when the repo's hooks dir holds live executable hooks.** Redirecting it
   silently stops a `pre-commit install` / lefthook setup from firing, with no
   diff to notice. The dir is resolved with `git rev-parse --git-path hooks`,
   not `project_dir/.git/hooks` — in a linked worktree `.git` is a *file* and in
   a monorepo package it doesn't exist, and either way a naive probe reports "no
   hooks" and redirects the SHARED config. It is also left alone when the
   project isn't the git root (`core.hooksPath` is repo-wide) or is already set.
   Every skip is reported in `install_hooks`' `warnings`, and `wh init --claude`
   downgrades its "enforcement installed" line when any caveat is present —
   silence would claim a hook is running when it can never fire.
4. **`--ci` is refused in private mode.** A workflow file is inherently shared.
   `wh publish --ci` writes it at flip time.
5. **`wh publish` never runs `git add` or `git commit`.** It removes the exclude
   block, writes the real `.gitignore` entries (`.state`/`.personal`/metrics — the
   existing `personal::ensure_gitignore_entries`), *removes* the `setup.private`
   key (writing `false` would ship a private-mode artifact to the whole team in
   the very file publish makes trackable),
   completes any wiring skipped under decision 3 (now that sharing is intended,
   including migrating our hook entries out of `settings.local.json` into
   `settings.json`), and **prints** the file list + suggested `git add` command.
   Repo mutations stay the user's move. Idempotent: re-running reports `noop`.
6. **Private awareness lives inside the oracles**, not in callers: `register_mcp`
   and `install_session_hooks` consult `is_private()` themselves. The TUI wizard
   therefore inherits correct behavior with zero new TUI logic (boundary §11
   discipline), and `wh status --setup` gains a `private_mode` field for display.
7. **Requires a git repo.** Without `.git`, `--private` errors cleanly — there is
   nothing to hide from.

## Invariants

- Nothing Whetstone writes in private mode is visible to `git status` on a fresh
  repo (the acceptance test drives `wh init --claude --private` then asserts
  `git status --porcelain` is empty, including after `wh scan` and a hook run).
- Enforcement is mode-independent: scan / hook / MCP / status behave identically
  in private and public mode — private changes *visibility*, never *function*.
- `enable` then `publish` leaves the repo byte-identical to never having used
  private mode (modulo the `.gitignore` entries `init --personal` would add and
  `setup.private: false`).
- **Publish is one-way in one respect:** it writes real `.gitignore` entries, and
  re-enabling private mode afterwards cannot hide `.gitignore` (a legitimately
  shared file). So `enable → publish → enable` leaves that one visible change.
  This is intended — publish is the decision to share — but it means "un-publish"
  is not a supported operation; revert the `.gitignore` hunk by hand if needed.
- User content outside the managed block in `.git/info/exclude` is preserved
  verbatim by both operations.

## CLI surface

- `wh init --private` — enable private mode (composes: `wh init --claude --private`,
  `wh init --hooks --private`; `--private --ci` errors).
- `wh publish [--ci] [--schedule=…]` — the flip.
- `wh status --setup` → `"private_mode": true|false`.

## Out of scope (stays on the epic)

Out-of-repo artifact root; personal-layer `extends` (truly personal packs after
publish); `wh promote`; wizard step for choosing private at onboard time (the
wizard *displays* the mode; enabling it is `wh init --private`).
