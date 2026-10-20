# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

This log starts with the rebuilt product. Release notes for the previous
product line (0.1.0 to 0.12.0) are in Git history: `git show 1b7fd8c:CHANGELOG.md`.

## [Unreleased]

### Added

- `exclude` on a rule and on a `sweep_unsure` entry: globs or prefixes,
  matched like `paths`, that the rule (or the doubt) never covers. An
  excluded path is out of scope in every mode: pre-commit, a base check, a
  plain check and the sweep. `wh explain`, the dashboard and the schema name
  it.
- `mode: change` on a rule: it runs only in a check against a base
  (pre-push, `wh check --ci --base`), and a sweep runs it to count its flags
  but files no finding. `wh check --sweep --format json` lists the counts as
  `data.counts` (`{rule, state, count}`), and `findings.json` is unchanged.
  `wh explain` says the mode. A plain check names the rules it left to a
  base in `data.selection.change_only`. Scripts still get `WH_MODE` and
  `WH_BASE`; the `[ "$WH_MODE" = base ] || exit 0` guard is no longer
  needed.
- `batch: by-file | by-crate` on a rule: a sweep files one finding per file,
  or per crate or package (the nearest directory above the file with a
  `Cargo.toml` or `package.json`). The finding's new `sites` field lists
  each flag as `{path, line, text}`; its fingerprint is the rule and the
  batch (`by-file:<path>` or `by-crate:<directory>`), and its
  `subjectDigest` covers every site's content. Doubtful sites are batched
  apart, each batch carrying its `unsure` reason. Unbatched rules keep their
  exact findings, and `wh explain` resolves a batch's fingerprint.

### Changed

- A rule without `exclude` no longer applies to `**/generated/**`. A rule
  that wants generated code says `"exclude": []`. Rules that name none of
  the new fields keep their digest.
- A lint or formatter rule's flags on files outside its scope (its `paths`
  and exclusions) are dropped; when every flag is outside it, the rule
  passes.

- The check summary labels a should rule's flag WARN instead of FAIL, so
  only a blocking failure reads FAIL.
- `--custom-principle` names the principle by its statement's first words
  (`principle.leave-the-campsite-cleaner`) instead of a hash, so it is easy
  to type into `--serves`. Different words that slug the same are numbered.

### Fixed

- `wh check --ci --base ... --format annotations` failed on a must flag
  outside the change without printing an annotation for it. Each such rule
  now gets one `::error` that names where its flags are.

## [0.15.0] - 2026-10-19

### Added

- Principle coverage: the chain from principle to rule is checked.
  `wh check` (but not `--staged` or `--sweep`) reports the principles no rule
  serves and the rules that serve none, in its summary and as
  `data.coverage`. The verdict and `findings.json` do not change.
  `wh dash --json` carries it as `data.current.principle_coverage`, and
  each principle carries `served_by`. The dashboard lists the rules that
  serve each principle as links, marks an unserved principle with a button
  that opens Add rule pre-filled, marks a rule that serves no principle,
  and adds a health line.
- `wh explain <principle id>` answers for a principle: its statement, the
  rules that serve it, or the `wh change ... --serves` command that would
  add one.
- `wh change --retire` refuses a principle that rules still serve and names
  them.
- Strict mode: an optional `require_served` field in the rulebook, drafted
  with `wh change --require-served true|false`. When it is on,
  `wh change --accept` refuses a rulebook with a gap and lists the gap. The
  dashboard shows it in the header, and the pending bar announces the
  refusal.

- A rule names the principle it serves. `wh change --kind rule ... --serves
  <principle id>` writes it as the rule's source (the id has to be a
  principle the rulebook holds), the dashboard's rule form has a Serves
  picker and each rule shows it, `wh explain` prints it, and every sweep
  finding carries it as `principle`, so a host can quote the why with the
  flag. The reusable Action adds a Serves line to what it opens.
- The agent contract tells an agent to read the rulebook's principles before
  planning and that `wh explain` names the principle a rule serves.
- `wh change --export-pack <file> --principles` writes the accepted
  rulebook's own principles as a principle pack
  (`whetstone.principle-pack.v1`, the shape pstack and `mattpocock` ship in)
  for another repository to install with `--import-pack`.

- A starting set of principles. A pack marks the principles it offers as a
  starting point (`starting`), `wh init --action agree --principle starting`
  drafts them, and the dashboard offers them with one button (Adopt the
  starting set) until the rulebook holds a principle. Each is an ordinary record, edited or retired with `wh change`;
  a retired one is not drafted again.
- A second built-in principle pack, `mattpocock`: the lessons of Matt
  Pocock's skills (deep modules, vertical slices, a feedback loop before a
  hypothesis, a shared domain language and more) in Whetstone's words, MIT,
  pinned to skills 1.3.1. The starting set takes six principles from pstack
  and four from this pack.

### Changed

- A rule serves a principle only when its source kind is `principle`.
- `wh change --accept` names a draft that was retired before it was ever
  committed, instead of writing an empty commit subject.
- A check without a rulebook points at `wh init --action agree` instead of a
  removed command.
- `wh dash --search`, `--as-of` and `--page-size` say in their help what
  they do: they narrow the changelog that `wh dash --json` returns.
- `wh dash` is one page: a one-line header, the pending changes with a
  field-level diff and the commit message accepting writes, one health line
  per problem, the mission, the principles (with the packs' principles to
  adopt), rules grouped Hard rules (must) and Soft rules (should) with each
  rule's own history, Add rule with starters and pack principles as
  suggestions, and a Changelog with a summary of what each commit changed.
  Setup shows only until a mission and a rule are committed, and every
  section but the mission folds.
- Values are edited where they stand: the mission, principles and every
  field of a rule; each edit writes a draft, and the changed text is
  redlined until it is accepted or withdrawn.
- The dashboard is built from `web/` (React, shadcn, Tailwind, Anime.js,
  Oswald, Vollkorn and JetBrains Mono) into `assets/dashboard/`, embedded
  in the binary; it still makes no third-party request.
- The `wh dash --json` view model changed shape: `current` holds `header`,
  `pending`, `health`, `mission`, `principles`, `groups`, `retired`,
  `suggestions`, `history`, `setup` and `coverage`. `rulebook`,
  `changelog`, `workflows` and `read_only` are unchanged.

## [0.14.0] - 2026-10-19

Whetstone becomes a repository's rulebook: enforced rules with a rationale
and an agent protocol, kept as `whetstone/rulebook.json` in the repository.
Five commands remain (`wh init`, `wh dash`, `wh change`, `wh check` and
the new `wh explain`).
Whetstone keeps no store and no host-specific code: hosts consume the
rulebook file, `findings.json` and the JSON envelope.

### Migration notes

- 0.14.0 reads only `whetstone/rulebook.json`. It does not read a Beads
  store, and records already in one stay there untouched.
- Private stores from 0.13 under `.git/whetstone/v1/<id>/private` are left
  on disk and nothing reads them. Once the export below is done, delete them
  with `rm -rf .git/whetstone` (that also clears old check evidence, which
  the next check writes again).
- To move a 0.13 store to the file, install the build at commit
  `20fb5fb06bb3e27103427af0ad62a0b7607770da`, tagged
  `rulebook-export-0.13` (`cargo install --git
  https://github.com/angusbezzina/whetstone --tag rulebook-export-0.13`),
  and run `whetstone init --action export` in the repository. It reads the shared Beads database and
  the private store and writes the latest accepted, shared revision of each
  principle and rule, with the mission, as `whetstone/rulebook.json`;
  records accepted only in that checkout, pending drafts and private
  retirements go to `whetstone/rulebook.drafts.json` for the owner to read.
  A rulebook that differs is replaced only with `--yes`. The details are in
  that commit's message. Then install 0.14.0, commit the file, and delete
  `whetstone/store.json` if it exists.
- `wh push` and `wh pull` are gone: share the rulebook the way you share any
  commit, and review rule changes in pull requests.
- The CI workflow is scaffolded once and then team-owned: to adopt the new
  one, delete `.github/workflows/whetstone.yml` and run `wh init --action
  wire --ci` again. It no longer sets up Node, and it adds the sweep on
  pushes to main.

### Added

- The rulebook is a file in the repository: `whetstone/rulebook.json`
  (`whetstone.rulebook.v1`, [schema](references/rulebook.schema.json)) holds
  the mission, the chosen principles and the rules, each rule with the
  sha256 digest of its canonical JSON. A revision is the commit that last
  touched the file, and the decision log is its Git history. `wh check`, the
  sweep, `wh change` and the dashboard read and write it; there is no record
  store.
- The CI scaffold (`wh init --action wire --ci`) runs the sweep on every
  push to main and uploads `findings.json` as the `scan-findings` artifact,
  next to the required `whetstone/policy` check on pull requests.
- Every rule `wh change` writes is probed first: its enforcer runs once on
  the repository, and a rule whose paths cover nothing, whose query does not
  parse, whose lint code is unknown or whose command cannot start is not
  written without `--allow-unvalidated`. Command enforcers get `WH_MODE`,
  `WH_RULE`, `WH_PATHS` and `WH_BASE`; scripts under `whetstone/checks/` run
  at pre-commit on the staged files.
- `wh init --action wire` writes the agent contract into `AGENTS.md`
  between markers (text outside them is kept) and scaffolds
  `whetstone/checks/` once: the script contract and a working example, which
  `wh init` then offers as a starter rule.
- `wh explain <fingerprint | rule id>`: the rule behind a finding, its
  enforcer, what it covers, how a sweep works it, the principle it serves
  and the commits that added or changed it, with their reasons. A
  fingerprint resolves from `findings.json` or by recomputing it.
- Principle packs (`whetstone.principle-pack.v1`,
  [schema](references/principle-pack-v1.schema.json)): pstack is the default
  pack in the binary, and a repository adds its own under
  `whetstone/principles/`. `--principle` takes `<id>` or `<pack>/<id>`; a
  principle from another pack is written with the pack's name and version.
- `wh change --from-text | --from-file` compiles prose (CONTRIBUTING, a
  style guide) into probed rule drafts where the text names its enforcer,
  and returns the rest as needing one; `--from-finding` drafts a must rule
  from a finding that keeps coming back. The dashboard's Rules view drafts
  from pasted text.
- A sweep finding of a formatter or lint rule whose tool can rewrite files
  carries the tool's own fix: an optional `fix` diff that applies with
  `git apply`, made on a throwaway copy (the working tree is never written).
  The findings shape is in [references](references/findings-v1.schema.json).
- `wh check --format annotations` prints each flag as a GitHub workflow
  command (`::error` for must, `::warning` for should); with `--base` only
  lines the change touched are annotated. The CI scaffold runs the required
  check this way.
- A reusable GitHub Action (`action.yml`) works a sweep's `findings.json` by
  class, as Messo does inside Virgil: a draft pull request per `draft_pr`
  finding (with its fix when it has one), an issue to approve per
  `confirm_first` and an issue to discuss per `discuss`, idempotent by
  fingerprint. The CI scaffold's `whetstone/work` job runs it on the
  sweep's findings when the repository variable `WHETSTONE_WORK` is on.
- Rule packs (`whetstone.rule-pack.v1`): `wh change --export-pack <file>`
  writes the committed rulebook's mission-independent rules and principles
  with a content digest (paths only with `--include-paths`); `wh change
  --import-pack <file|https URL|rust|typescript|python>` verifies it, probes
  each rule here and writes them as drafts. A principle pack imported the
  same way is installed under `whetstone/principles/`.
- The dashboard's Coverage view shows the parts of the repository no rule
  reads (by top-level directory and language) and the rules that read
  nothing. `wh dash --export <dir>` writes the accepted rulebook and its
  changelog as one static page with no script; the CI scaffold publishes it
  to GitHub Pages when `WHETSTONE_PAGES` is on.

### Changed

- `wh change` edits `whetstone/rulebook.json`. A change or a `--retire` is
  a draft, the edit in the working tree; `wh change --accept` commits the
  file, and only it, with the edits, the rules' new digests and the reasons
  in the message; `wh change --withdraw` restores the file's last commit. A
  retired rule or principle leaves the rulebook and is listed under its
  `retired` entries with the reason. The decision log is Git: `wh dash
  --trail` and the dashboard's changelog are the commits that touched the
  file, with author, date and message.
- `wh dash` shows `whetstone/rulebook.json` as the working tree holds it:
  each entry is accepted, edited or new against the file's last commit,
  and the dashboard lists the draft edits with Accept and Withdraw. Every
  dashboard write is a `wh change` (onboarding included): the rule form
  sets the sweep class, priority and doubts, accepted rules and principles
  retire with a reason, and the changelog is the file's Git log with the
  retired entries beneath it. `wh dash --json` names the rulebook's state
  and revision in place of the store, sync and progress sections, and
  lists no push or pull.
- Whetstone's own gates (pre-push, CI and the release job) check Whetstone
  with itself: `scripts/self-check.sh` runs `wh check --ci` over the whole
  tree against the must rules in Whetstone's committed
  `whetstone/rulebook.json` (clippy's `dead_code`, rustfmt, and an AST rule
  against `dbg!`, `todo!` and `unimplemented!`), and fails on any must
  finding or when no must rule ran. The pre-push hook and CI run three
  gates: clippy, the tests, and the self-check; no test needs `bd` or Python.
- `wh check --ci` refuses an uncommitted edit to the rulebook, and
  `wh check --rule <id>` with an id that is not in the rulebook is an error.
- The CI scaffold installs only the pinned `wh`: no Node, no secrets.
- `README.md`, `AGENTS.md`, `CLAUDE.md`, `PRODUCT.md`, `DESIGN.md`,
  `planning/direction.md` and `planning/skill-cli-boundary.md` describe the
  rulebook file, the five commands, the gates and the host contracts.
  Whetstone's own rulebook states its real mission.
- The crate ships only the manifest, the README, the changelog, `assets/`,
  `src/` and `references/`.
- `wh init --action agree` drafts the mission, principles and starter rules
  into `whetstone/rulebook.json` (`wh change --accept` commits them) and
  takes no `--expected-revision` or `--resume`; repeating it drafts only
  what is new. `wh init --action setup` installs the Git gates and scaffolds
  the CI workflow (`wire --hooks --ci`). Bare `wh` lists five workflows and
  names the rulebook file, and its tool report is gone.

### Fixed

- A checker's process group is signalled until it is empty, so a descendant
  forking as the group is killed can no longer hold its output open.
- Repeated sweep findings of one rule on one file are numbered in source
  order, so a flag added later in the file never changes an earlier
  finding's fingerprint (and never un-dismisses it in Messo).
- A check or sweep over a large repository with symlinks inside it succeeds:
  the report's code digest is Git's tree plus the bytes of the changed and
  untracked paths, a symlink counts as its link text, and there is no file
  cap. Paths that escape the project are still refused.

### Removed

- The record store, with `bd` and every Virgil-specific path: `wh push` and
  `wh pull` (and `--select`, `--include-ancestry`, `--canary` and
  `--confirm`), the private and shared Beads databases and the Virgil
  `rulebook` collection behind one `RecordStore` port,
  `whetstone/store.json`, `wh init --action store`, `wh init
  --store`, `--yes`, `--expected-revision` and `--resume`, `VIRGIL_VIA`, the
  minimum `bd` version and its install and upgrade commands, the response
  envelope's `expected_revision` and `resume_token`, the record envelope and
  its kinds (`references/agreement-record-v1.schema.json`, and
  `references/rule-v2.schema.json`, superseded by the rulebook schema),
  stored records read as retired history, `scripts/bd-timing.sh`, and the
  Python gates (ruff and the pytest black-box tests with the fake `virgil`).
  Records already in a Beads store stay there untouched; Whetstone no longer
  reads them.
- `wh change`'s proposal, local review and retirement records and the
  decision-history read model behind them: `wh change --accept <proposal>`
  and `--withdraw <proposal>` (both are now plain flags), `--expected-revision`,
  `--resume`, `--new-owner`, `--source`, `--expected-effect`, `--impact`,
  `--example` and `--conflict`, request-id idempotency, `wh dash --json`'s
  `history`, `history_state` and `history_detail`, the dashboard
  inspection's `expected_snapshot`, and the check report's
  `selection.skipped_drafts`.
- The legacy YAML rule scanner: `whetstone/rules`, the hidden `validate`,
  `eval` and `scan` commands, `wh check --path` and `--lang`, the
  `whetstone.native-scan` result and `raw_scan` data, the AST enforcer's
  fallback to a rule file by name, `wh change --migrate` (and the
  dashboard's Earlier records panel that ran it), and
  `references/rule-schema.yaml`. Whetstone no longer depends on `serde_yaml`
  or `toml`. A rule's `paths` scope a check or a sweep.
- The agent-host hook loop: `wh init --action wire --hooks` no longer writes
  Stop and SessionStart hooks into `.claude/settings.json`,
  `.codex/hooks.json` or `.cursor/hooks.json`, and `wh check --hook stop`,
  `wh dash --hook session-start`, `wh check --host` and the host checkpoint
  receipts are gone. The Git pre-commit and pre-push gates are the loop
  every tool passes through.
- Raised hands: `wh check --raise-hand`, `wh change --answer`, a rule's
  `hand_raise` triggers, the dashboard's Requests view and its Beads task
  queue, and the Virgil task route. A sweep finding's `unsure` is how a rule
  says it cannot decide.
- Review attestations and briefs: `wh check --attest`, `--brief` and their
  flags, the `review` and `brief` enforcers, and the `/interrogate` or named
  reviewer. The prove-it-works starter is offered only when a test command
  is detected.
- Jev: the question enforcer, judgment receipts, the driver's `ask` command
  and every TypeSafe call, redaction, local-only rules, shadow mode and the
  shadow badges, flag labels (`wh change --accept-flag` and
  `--dismiss-flag`), the tuning and hardening drafts they fed (`wh change
  --tune`, `--hardens`), per-rule flag statistics, the dashboard's Flags to
  label and Suggestions panels, and the Jev half of `wh eval`. The
  tests-only-when-asked starter is gone.
- The `design_tokens` and `public_surface` enforcers and their built-in
  scanners, and the ask-before-public-API starter: pre-commit runs AST rules
  only. A design token rule can be expressed as a validator rule.
- The `advisory` strength: a rule is must (blocks the gate) or should
  (flags, never blocks, and becomes a sweep finding). `wh change` refuses
  `advisory`.
- The rule-example half of `wh eval` (per-rule precision and recall over
  labelled examples): `wh eval` runs the scanner's golden examples and the
  pstack round trip.
- Lessons and the memory layer: `wh change --kind lesson`, `--admit`,
  `--supersede`, `--forget` and `--from-lesson`, the lesson anchor checks
  `wh check` recorded, the dashboard's Lessons panel and its Admit action,
  lesson sharing in `wh push`, the Virgil lessons backend and its import,
  `wh init --action memory`, `--group` and `whetstone/memory.json` (Virgil
  owns lessons itself).
- The verification skill and drive proofs: `wh init --action wire --host`
  and `--regenerate-driver` (wire now installs the Git gates and the CI
  workflow only), `wh init --action import`, the verification driver
  (`whetstone/verify/`, Node and Chrome), feature maps (`wh change --kind
  feature` and `--kind map`), map hygiene, the `drive` enforcer, flaky-proof
  quarantine, `wh check --feature`, `--step`, `--mutate` and
  `--maintain-outcome`, the dashboard's Features panel and feature editor,
  and the pstack round trip in `wh eval`. `wh check --sweep` prints its
  findings by default; `--format json` gives the envelope.
- Check receipts: `wh check` writes no record. Its report (now
  `whetstone.check-report.v1`, each finding's repair route in
  `recheck_command`) is the result, and a sweep's `findings.json` is the
  record a night sweep keeps. Gone with them: the dashboard's Checks view
  and its check runner, `/api/evidence`, the hosted dashboard mode and the
  receipt rows in the changelog and decision trail.
- Onboarding extras: `wh init --action exemplar` (`--from`, `--url`) and the
  dashboard's Exemplars step, the read-only repository discovery `wh init`
  and `wh dash` reported as `setup`, and the pstack skill detection in `wh
  init --action setup` with `--host`, its install commands and
  `whetstone/tools.lock.json`. The principle catalogue is read only from the
  copy compiled into `wh`.
- Project files that described or served the earlier direction: the
  dashboard design reference and its build and smoke scripts
  (`planning/direction-demo/`), the archived plans (`planning/archive/`),
  `planning/record-store.md`, the example clients (`examples/`), the root
  `SKILL.md`, `scripts/beads-repair.sh`, `.impeccable/` and
  `skills-lock.json`, and the legacy `whetstone/evals/`,
  `whetstone/context/`, `whetstone/whetstone.yaml` and
  `whetstone/.metrics.jsonl`.

## [0.13.0] - 2026-10-15

The first release of the rebuilt product. Whetstone is rebuilt around six
workflows and a verification skill that any agent can use. It replaces the
previous product line (0.1.0 to 0.12.0) and is not compatible with it.

**Direction change (2026-09-27).** The owner adopted the delegation plan
(`planning/direction.md`, Beads epic `whetstone-ppq`, which supersedes
`whetstone-k5r`). Rules get one strength and one enforcer: a mechanical check,
then a Jev question, then review. Checks move to git pre-commit, pre-push and
a required CI status, and onboarding becomes mission, pstack principles and
exemplar codebases. Several entries below describe code this plan removes:
the eight-decision agreement, key metrics, team activation through pull
requests, and gate exceptions. They stay here as a record of what was built.

### Added (memory as the whetstone, `whetstone-jpd`)

- Lessons: a `lesson` record (`whetstone.lesson.v1`) with subjects,
  audience, a store-stamped provenance, anchors, trust (inferred, validated,
  authoritative) and status (proposed, admitted, superseded, forgotten).
  `wh change --kind lesson` proposes one; `--admit`, `--supersede` and
  `--forget` are a person's (an agent's correction or forget is a proposal,
  an agent's admit is refused). Requests that supply provenance, and group
  lessons about something narrower than the group, are refused.
- Anchors: the paths a lesson names plus the writer's additions, hashed at
  write. `wh dash` and session start mark a lesson whose anchor changed as
  "may be stale" without a model call; `wh check` records each comparison as
  a `lesson_check` receipt.
- A pluggable lesson store behind `whetstone/memory.json`
  (`wh init --action memory`): Whetstone's Beads store standalone, or a
  Virgil group's lesson collection through `virgil lessons` only. Raised
  hands follow the switch: a Virgil task in the group's channel instead of
  `bd human`. Standalone, admitted group lessons travel with `wh push`;
  personal and unadmitted lessons never do.
- The Virgil backend speaks the shipped `virgil lessons` CLI (Virgil PRs
  #1021, #1036): proposals stay in Whetstone's store and a person's admit is
  their `virgil lessons remember` (validated; `--trust` is refused there);
  corrections rewrite in place with `--if-revision`, keeping `--about` and
  `--anchor`; Virgil's exit codes map to Whetstone's refusals (`read_again`,
  `unattached`, ...); staleness comes from Virgil's anchors and
  `list --changed`. An agent's writes carry `VIRGIL_VIA=agent:<host>`, and an
  agent's raised hand names the agent in the task.
- Flipping to Virgil migrates: `wh init --action memory --store virgil`
  checks the checkout's attachment, writes the admitted lessons outside the
  repository as the file `virgil lessons import-beads` reads (proposals
  beside it), and flips only when a person confirms with `--yes` that every
  key read back; anchored lessons are then re-anchored. `wh dash` counts the
  lessons left behind until then.
- The ladder: `wh change --kind rule --from-lesson <key>` drafts a shadow
  rule from an admitted lesson; accepting it cites the lesson, and `wh dash`
  walks rule, accepting review, lesson and anchors. Any rule can be in
  shadow (`shadow`), and shadow rules are promoted by `wh change --tune`.
- Sweep mode: a rule's `sweep` class (draft_pr, confirm_first, discuss) and
  `wh check --sweep --format findings`, a JSON array (`findings.json`, the
  shape Messo's `messo-ctl observe` reads) with one stable finding per flag
  of the rules below must. Each finding carries the rule's `sweep_priority`
  (0 to 4, default 2) and, where the rule's `sweep_unsure` paths or pattern
  cover the flag, an `unsure` reason; a doubt covering every flag is refused.

### Added (delegation plan, `whetstone-ppq`)

- Rule v2 (`references/rule-v2.schema.json`): one strength and exactly one
  enforcer (mechanical, Jev question or review), labelled examples, source,
  hand-raise triggers and privacy. Earlier standards and guidance are read as
  rules.
- Onboarding: `wh init --action setup` (Beads and pstack detection, one
  confirmed install, `whetstone/tools.lock.json`), mission plus pstack
  principles plus three starter rules in `--action agree`, and
  `--action exemplar` to draft rules from a codebase the owner admires.
- Jev questions through the driver's `ask` command with kernel-side
  redaction and local-only rules; shadow mode, confidence bars, and the rule
  that a Jev answer never passes a must rule.
- Gates: git pre-commit (`wh check --staged`), pre-push
  (`wh check --base <remote>`) and a required CI status (`--ci`), chaining any
  existing hook; Codex and Cursor stop hooks next to Claude's.
- Review attestations (`wh check --attest`), briefs (`--brief`), raised hands
  as `bd human` issues (`--raise-hand`, answered with `wh change --answer`),
  flag labels (`--accept-flag`, `--dismiss-flag`), false-flag rates, tuning
  drafts (`wh change --tune`), hardening candidates (`--hardens`), and
  `wh change --migrate` for earlier values, philosophy and YAML rules.
- Proof quality: flaky proofs are quarantined, and `wh check --mutate` runs
  declared mutations in an isolated worktree to catch hollow proofs.
- `wh eval` scores each rule's examples by enforcer and checks the pstack
  round trip.
- The generated verification skill carries `rules/*.md` and
  `whetstone.verify.json` (`references/verify-skill-v1.schema.json`).

### Removed

- The eight-decision onboarding, core values, implementation philosophy and
  key metrics as live kinds, policy exceptions, scopes, PR-manifest activation,
  repair-host sockets and transport, and mandates. Their stored records are
  still read byte-exactly as retired kinds and can never be written.

### Fixed

- Answering a raised hand works on bd 1.1.2, the minimum supported version,
  whose `bd human respond` cannot open the store. The answer is recorded the
  same way: a `Response:` comment and the close reason `Responded`.
- A Chrome profile the driver cannot remove (Chrome still writing it) no
  longer turns a real verdict into `unknown`.
- A second `wh change` (or `--accept`) without `--request-id` no longer
  collides with the first: the default request id is derived from the
  change's input.
- Pre-push checks exactly what is pushed: the hook passes the pushed commit,
  and the check refuses when that commit is not checked out or tracked files
  have uncommitted changes; untracked files are not part of the push.
- A review attestation binds to the tree it reviewed, so it stops holding as
  soon as the checked content changes (one more commit included).
- Committing Whetstone's own wiring, a change with no file an AST rule reads,
  a whitespace-only signature change, a new module with public items, or a
  change outside a path-scoped review no longer blocks a commit; neither
  does a repository whose rules all run at pre-push, or whose only rules are
  in shadow.
- Staged checks record no receipts, which brings pre-commit to about 1.5s.
- `wh init --action agree` replays exactly, resumes a half-written batch,
  accepts the mission already in force, and writes nothing when stale; batch
  writes work beside retired records.
- The Stop hook no longer claims a hand was raised when none could be filed.

### Documentation

- README, AGENTS, CLAUDE, PRODUCT, SKILL and the planning docs now describe
  the delegation plan, and separate what exists today from what is planned.
  The storage and trust-boundary ADRs and the R0 prune inventory moved to
  `planning/archive/` with superseded notes.

### Added

- **Six workflows** with one JSON envelope (`whetstone.command-response.v1`):
  `wh init`, `wh dash`, `wh change`, `wh check`, `wh pull` and `wh push`. Every
  workflow that writes accepts `--dry-run`.
- **A private project agreement.** `wh init` records the owner's eight
  decisions (mission, desired outcome, values, philosophy, owner, first
  safeguard and its scope, revision triggers, first gate) in a private store
  under `.git/whetstone/`. Nothing is committed or shared. Run by a person
  before any agreement exists, it opens the dashboard's guided onboarding and
  logs each recorded step to the terminal; `--no-open` prints the decisions
  and the exact command instead. Machine callers always get the JSON envelope.
- **A verification skill for any agent.** `wh init --action wire` generates a
  skill, feature map and driver from the accepted records and writes the same
  bytes into `.claude/skills`, `.cursor/skills` or `.agents/skills`. Feature
  files follow pstack's four-section format, so pstack's skills work on them
  unchanged. `wh init --action import` turns edited or pstack-generated skills
  back into drafts.
- **Proof, not claims.** `wh check` runs every gate and records receipts bound
  to their evidence. A pass without evidence is unknown, never green.
  - `--feature <id>` drives one feature; `--step` adds the change's own
    assertions to the receipt.
  - `--changed` proves what a change touched; `--base <revision>` includes
    committed work.
  - `--sweep` drives every feature and reports each as proven, failed,
    unreachable or skipped, with map hygiene findings.
- **A verification driver** (`whetstone/verify/drive.mjs`, Node 22, no
  dependencies) for web, CLI and HTTP apps: isolated launches, doctor checks,
  screenshots and cleanup that keeps evidence.
- **A dashboard** (`wh dash`) with four views: Dashboard (mission, what needs
  attention, metrics, gates), Foundations, Checks (every gate's last run and
  its evidence) and Changelog (searchable decision history). `wh dash --trail`
  exports the decision trail as TSV.
- **Agent hosts.** Claude Code gets hooks: session context at start and failed
  checks handed back to the same session at stop, at most three times. Other
  agents check in with `wh check --changed --host <name>`.
- **Team sync.** `wh push` shares an exact, confirmed package, and private
  drafts never leave the machine. `wh pull` never runs anything it receives.
- **Team activation.** A shared rule becomes binding only through a pull
  request approved by a different account on a protected branch. CI enforces
  activated rules with `wh check --required`, restores tampered checkers from
  their pinned versions, and reports reviewed, expiring exceptions as excepted,
  never passing.
- **Maintenance.** Map drift and hygiene show up as attention items, and
  `wh check --maintain-outcome` records the result of a maintain pass.
- **A design system.** The dashboard follows the Whetstone logo: black and
  orange, white and orange in light mode, with ShadCN token names
  (`--background`, `--foreground`, `--muted`, `--primary`, `--ring`, ...) in
  the dashboard's own CSS. Every option is present as a quiet ghost and the
  one live thing is struck in orange; states are marks, not hues (a lit disc
  for pass, an orange cross for fail, a ring for unknown or stale, a dashed
  ring for draft). No boxes or rules; the tokens and rules are in `DESIGN.md`.

### Changed

- Records are stored in Beads (`bd` 1.1.2 or later) instead of Whetstone's own
  Dolt repository. The `dolt` binary is no longer needed.

### Removed

- The previous product line: its terminal UI, packs, legacy commands, Python
  runtime and generated artifacts. See the Git history above for its notes.
