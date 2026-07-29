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
onboarding a second package never re-exposes the first. Two encodings keep that
true for real-world paths: glob metacharacters are escaped in the *entries*
(`.git/info/exclude` is gitignore syntax, so an unescaped `[` matches nothing),
and `[`/`]`/`%` are percent-encoded in the *label* (it is bracket-delimited, so
a raw bracket truncates on read — and a truncated label collides with a
sibling's block and deletes it).

**Writes are locked and atomic.** The exclude file is a single shared
read-modify-write, so `enable`/`publish` take a lock file beside it (stale locks
older than 30s are reclaimed) and write through a uniquely-named temp file.
Without both, two `wh` processes onboarding different packages lose one of the
blocks — leaving a package marked private with nothing hidden. The write follows
a symlinked `exclude` to its target and restores the original file mode.

**A torn block stops at the first foreign line.** Repairing a block with no
terminator drops only lines that match an entry we render, so user ignores
below a half-written fence survive. A *well-formed* block is handled differently:
our fences and entries are removed across the whole region while foreign lines
inside it are kept — stopping at the first foreign line there would orphan the
rest of the block, and `publish` must be the exact inverse of `enable`. (That
also matters for forward compatibility: removing an entry from `EXCLUDE_ENTRIES`
in a future release turns old lines "foreign" for everyone who upgrades.)

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

## Verification: the promise is checked, not assumed

Writing the right patterns is not the same as being hidden. After every step has
written its files, `wh init --private` asks **git** whether the promise holds —
`git status --porcelain --untracked-files=all -z`, filtered to this project's
artifact paths — and **fails loudly** (non-zero, naming the exposed paths) if
anything is visible.

Both flags are load-bearing. `--untracked-files=all`: the default collapses an
untracked directory to `?? .claude/`, hiding a single re-included file inside it.
`-z`: git C-quotes any path containing non-ASCII bytes (`core.quotePath` defaults
to true), a quote, or a backslash — a quoted path matches no entry, so a real
leak would be filtered out and reported as verified. `-z` output is never quoted.

The check **fails closed**: if `git status` cannot run at all, that is an error,
not a pass. "Could not verify" must never render as "nothing exposed". Same rule
for the exclude file itself — git treats it as *bytes*, so a non-UTF-8 or
unreadable file is an error, never an implicit empty file (treating it as empty
destroyed the user's personal ignores and made `publish` a silent no-op).

The verifier distinguishes **our** footprint from **the user's** by asking the
**file**, not git: does its content bear Whetstone's mark? Paths we exclusively
own (anything with `whetstone` in the path) are ours by name; the shared-name
files (`.mcp.json`, `.claude/settings*.json`, `.githooks/post-merge`,
`.gitignore`) are ours only if our content is actually in them. One exemption:
if the **committed** copy already bears our mark, our content is public history,
so a working-tree change to it is the user's edit rather than our leak.

**Do not reintroduce git-state inference here.** Four consecutive releases
shipped a hole trying to deduce authorship from the index, from HEAD, or from
porcelain status letters — each fix missed a state nobody enumerated:

| Attempt | Missed |
|---|---|
| "is it in the index?" | `A ` — an artifact `git add`ed after we wrote it (what `wh publish` itself instructs) |
| `??` or `A*` (index column) | ` A` — intent-to-add (`git add -N`), whose `A` sits in the *worktree* column |
| same | `AA`/`AU` — unmerged paths that **are** in HEAD, wrongly claimed as ours |
| `git ls-tree HEAD -- <path>` | a root-relative path handed to a **cwd-relative** pathspec — off-root, every check asked about `<prefix><prefix><artifact>` |

The deeper problem was the premise, not the encoding: *"present in HEAD ⇒ not
ours"* assumed every artifact has a tracked-file guard, and
`.claude/settings.local.json` — the file private mode deliberately redirects
writes into — had none. Content attribution has no such premise. It gives the
same answer in every git state, at any directory depth, on any filesystem.

Two mechanical details still matter. `git status --porcelain` paths are
**repo-root-relative**, so every path check must resolve against the git root,
not `project_dir`. And `-z` emits a rename or copy as `R  <new>\0<old>\0`, so
the bare original-path field must be consumed or it gets rescanned as a record
with its first three characters read as a status code.

`.gitignore` follows the same rule with one extra step: it is ours only when it
carries Whetstone's personal-layer marker (`personal::GITIGNORE_MARKER`), and if
**HEAD's copy already has that marker** the block is committed and public, so a
later modification is the user's edit rather than our leak. Matching the bare
substring `whetstone` here instead of the marker was a real defect: a
hand-written `whetstone/` ignore line — the most natural first move a cautious
solo adopter makes *before* running the tool — read as our leak and refused
onboarding with a false diagnosis.

**An exposed `.gitignore` is advisory, not blocking.** It holds ignore lines
only — never rules, config, or taste — and it is a legitimately shared file, so
it is *reported* (in `warnings`, and as `exposed_advisory`) but never fatal.
Treating it as fatal made `enable → publish → enable` impossible, because
publish writes those very entries. Every other exposure stays blocking.

**A blocking refusal leaves no half-private repo.** `enable` writes the block and
the marker before it can ask git anything, and the caller aborts on error — so
the artifacts never got written while the project sat flagged private. On a
blocking exposure, `enable` now reverts exactly what that call created (a repo
that was *already* private keeps its state; only the error is reported).

**Verification is not one-shot.** The repo moves underneath a verified enable: a
teammate commits `.claude/*` + `!.claude/settings.json`, it arrives on `git pull`,
and our artifacts are visible again with nothing having re-checked. `wh status`
— what the SessionStart hook runs every session — re-runs the same check and
reports any blocking exposure in `warnings`. It reports rather than gates
(status is a read-only health command), and it fails closed in the message: a
check that could not run says so instead of reading as "nothing exposed".

The verifier covers the hidden artifacts **and** the inherently-shared ones.
`.github/workflows/whetstone-check.yml` is never hidden — a workflow only means
anything if the team has it — but `wh init --ci` before going private left it
visible while `wh` reported "invisible to `git status`". It is therefore part of
the checked set: enabling fails and names it, so the user deletes it or accepts
that it is public.

Paths containing a **control character** are refused up front rather than
verified after the fact — a newline splits the fence line and every entry, so no
correct block can be written for such a path.

This is the backstop for the whole failure class where `wh` reports success while
artifacts are exposed. It catches what patterns alone cannot: an in-tree
`.gitignore` negation (`.claude/*` + `!.claude/settings.json`) outranks
`.git/info/exclude` entirely — git precedence puts the working-tree file above
`$GIT_DIR/info/exclude`, and nothing Whetstone writes there can override it.

## Invariants

- Nothing Whetstone writes in private mode is visible to `git status` on a fresh
  repo (the acceptance test drives `wh init --claude --private` then asserts
  `git status --porcelain` is empty, including after `wh scan` and a hook run).
- **Private mode never reports success while artifacts are exposed.** Any leak
  is an error with the offending paths named, never a silent pass.
- Enforcement is mode-independent: scan / hook / MCP / status behave identically
  in private and public mode — private changes *visibility*, never *function*.
- `enable` then `publish` restores `.git/info/exclude` byte-identical to its
  pre-private content. The repo itself is not byte-identical to never having
  used private mode: the `.gitignore` entries publish writes remain, and
  `whetstone/whetstone.yaml` exists (carrying `version: 1` after the marker is
  removed). Both are inert; neither is reverted.

**Scope limit, stated honestly:** the verifier inspects the artifact paths
Whetstone owns. A footprint created anywhere else is outside what it can check —
e.g. `.git/info/exclude` symlinked to a *tracked* worktree file (refused up
front for that reason), or `wh debt --beads` shelling out to `bd`, which writes
`.beads/` on purpose because filing an issue in the team tracker is a shared act.
- User content in `.git/info/exclude` survives both operations verbatim,
  including CRLF endings, with exactly one normalization: a file that had no
  final newline gains one (enable must separate its block from the last line,
  and publish cannot know whether that newline was originally there).
- **Publish is one-way in one respect:** it writes real `.gitignore` entries, and
  re-enabling private mode afterwards cannot hide `.gitignore` (a legitimately
  shared file). So `enable → publish → enable` **succeeds** and leaves that one
  visible change, reported as an advisory exposure. This is intended — publish is
  the decision to share — but it means "un-publish" is not a supported
  operation; revert the `.gitignore` hunk by hand if needed.
- **Attribution is content-based, and that cuts both ways.** A user who strips
  Whetstone's name out of an artifact owns that file as far as the verifier is
  concerned; it will no longer be recognised as ours. This is the deliberate
  trade for an attribution rule that holds in every git state (see the table
  above) rather than one that guesses from the index.
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
