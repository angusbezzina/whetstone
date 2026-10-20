# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

A Rust binary (`wh`) serves the dashboard from files embedded at build
time. The page is built from `web/` (Vite, React, TypeScript, Tailwind and
shadcn components, Oswald and Vollkorn self-hosted, Anime.js) into
`assets/dashboard/`, and that build is committed so no gate and no
`cargo install` needs Node. The server sends nothing else, and the CSP
allows only its own origin: no third-party request, font or script.

## What Whetstone is

Whetstone is a repository's rulebook: enforced rules with a rationale and an
agent protocol. It sits between `AGENTS.md` (prose an agent may read, which
nothing enforces) and a linter (enforced, with no rationale and nothing an
agent can act on).

The rulebook is a file, `whetstone/rulebook.json`, committed in the
repository. It holds the mission, the chosen principles and the rules. Every
rule has one strength (must blocks the gate, should reports and never
blocks), one mechanical enforcer (ast, lint, formatter, test, validator) and
sweep fields (class, priority, unsure). A revision is a commit, the decision
log is Git history, and a rule change is a pull request. The rules are
enforced at pre-commit, pre-push and as the required CI status
`whetstone/policy`.

Hosts consume Whetstone through three things: the rulebook file,
`findings.json` (`wh check --sweep`: fingerprint, subjectDigest, class,
priority, title, responsibility, unsure) and the JSON envelope every command
returns. A host may be Virgil's Messo, a GitHub Action or an agent session.
Whetstone keeps no store and no host-specific code; hosts own memory, tasks
and the agent.

What makes it different:
- Only a mechanical pass passes a must rule.
- Unknown, stale, skipped or unavailable evidence is never reported as
  success.
- Every rule carries its rationale, and every flag names the rule
  responsible and how to recheck it.
- A rule changes only by an edit a person commits.

## Users and job

Primary user: a software engineer or small engineering team lead who lives in
the terminal and an editor, and opens `wh dash` in a browser tab beside them.
They are also the accountable project owner. In under a minute they want to
know:
- what the rules are and why each exists;
- which drafts are waiting to be accepted;
- what changed in the rulebook, when and by whom.

Coding agents read the same file and the same services through `--json`; the
dashboard is the human view of the same rulebook. The dashboard is not a task
tracker, a fleet console or a CI replacement.

## Surfaces

The dashboard is one page, `wh dash`, built around one job: a person
reviewing or editing the rulebook understands what they are looking at
within five seconds.

1. **Header** (one line): project, branch, the rulebook's revision, a
   summary ("14 rules: 7 block push, 7 report"), the last change and its
   author, and a mode badge (Editing or Read-only, with how to get edit
   access).
2. **Pending changes** (only when the working tree's rulebook differs from
   its last commit): the count, a field-level before and after per entry,
   the commit message accepting would use, Accept (commits the file and only
   it) and Withdraw (restores it).
3. **Health** (one line per problem): gates not installed with the fix
   command, the file unreadable, rules that read nothing, source files no
   rule reads; one healthy line when nothing is wrong, and nothing at all
   before there is a rulebook.
4. **The rulebook**: the mission as a sentence, the principles (with the
   pack's principles to adopt), then rules grouped Hard rules (must) and
   Soft rules (should), sorted by id. A row says the rule, how it is
   checked, where it runs and what happens when it breaks; opened, it shows
   the rationale, the exact command, the paths, examples, source, sweep
   fields and the rule's own history. Every value is edited where it stands
   and each edit writes a draft. Retired rules sit at the end. Principles,
   both groups, Retired and the Changelog fold to one line as they grow.
5. **Add rule**: starters detected for the repository, principles from the
   loaded packs (as suggestions; principles are not enforced), a blank rule,
   or rules compiled from pasted guidance.
6. **Changelog**: every commit that touched the rulebook, newest first,
   with a summary computed by diffing successive versions and the
   field-level diff on demand; TSV export of the decision trail.
7. **Setup**: only before a mission and a rule are committed, a short
   guided page (mission, rules, gates) that ends in one commit.

## Durable constraints

- Honest states: missing, unknown, stale and not-run are explicit and never
  green. Read-only inspection is not success.
- No blended health score, no gamification, no tracker or fleet-console
  behaviour.
- Keyboard navigation, focus management, live-region feedback, safe rendering
  of user content, and widths of 320, 390, 768 and 1024 px.
- The page's HTML, CSS and JavaScript stay under 600 KiB and its fonts
  under 200 KiB, all served by `wh` itself.
- CLI and dashboard read the same typed projections; the UI invents no state.
- The five public commands (`wh init`, `dash`, `change`, `check`, `explain`) support the
  UI and may be progressively disclosed; they are not the human information
  architecture.

## Terminology

Mission, principle (a catalogue principle or the owner's own), rule (one
strength must/should and one mechanical enforcer), rulebook, draft (an
uncommitted edit to the rulebook), accepted (committed), retired, check,
finding, sweep, owner.

## Brand commitments

Owner-pinned 2026-09-14 and extended 2026-10-19: the Whetstone logo (a flat
isometric orange stone on a black base; the wordmark set wide and tracked
in caps, WHET in ink and STONE in orange). Core colours are black and orange
#e66c3a; light mode is white and orange. Oswald sets titles, labels, buttons
and numbers; Vollkorn sets body text; JetBrains Mono sets code, and any
command or path in a sentence is wrapped as code. Components are shadcn's, restyled to
the visual world; motion is Anime.js. The visual world chosen on 2026-10-19
is The Standards Sheet, recorded in DESIGN.md.
