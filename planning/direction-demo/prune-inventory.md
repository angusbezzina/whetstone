# Whetstone R0 prune inventory

This is implementation evidence for `whetstone-k5r.1`, not a second backlog. It fixes the exact prune boundary before the new product foundations begin.

## Baseline and recovery

- Original revision: `1b7fd8c341b8a5aaea742c564092fbcca26b51bb` (`docs: require prune-first Whetstone MVP conversion`).
- Original release: v0.12.0; restore old executable behavior by checking out the revision or installing that release, never by retaining an executable archive in the new tree.
- Baseline: 37,982 Rust lines across 109 tracked `src/` files, 10,266 test lines, 7,077 archived Python lines, 22 direct Rust dependencies, 3,748 KiB of tracked files, and a 12,777,600-byte local release binary.
- Preserve all Git history, all Beads/Dolt records, `.git/info/exclude`, and every existing file under `whetstone/` byte-for-byte during R0. Those are user/project data or recovery evidence, not justification for keeping their writers.
- The recorded pre-cut SHA-256 canaries for `whetstone/**` and `.git/info/exclude` are attached to the Beads evidence; R0 exit must reproduce them.

## Accepted minimum product boundary

The team MVP is one repository with mission, values, implementation philosophy, versioned standards and decisions, deterministic native checks, repair-and-recheck feedback, full history, a lightweight local dashboard, selected private-safe sharing, independent approval, protected CI, and two agent hosts. Its public workflow families are exactly `wh init`, `wh dash`, `wh change`, `wh check`, `wh pull`, and `wh push`; bare `wh` is read-only orientation and stable JSON is the agent interface. The R0 foundation is not that MVP.

Proactive mandates, telemetry, tracker bridges, context gardening, broad dependency discovery, starter-pack catalogs, a TUI, a task board, a graph editor/database, a model router, a fleet manager, a scheduler, broad connectors, org administration, billing, and functional legacy compatibility are outside the minimum boundary.

Owner and evidence roles:

- Product and policy owner: Angus Bezzina.
- Implementation operator: the agent executing `whetstone-k5r`.
- R0 exit and epic acceptance: an independent reviewer; final epic acceptance requires an explicitly spawned Astra extra-high reviewer.
- First dogfood repository: Whetstone itself.
- First safeguards: the retained AST rule/golden gate, actual Clippy configuration verification, a full CLI repair/recheck journey, and one attributed judgment-only preference once M1 is implemented.

Initial measurable budgets, to be measured rather than assumed:

- Clean checkout to first trusted local safeguard: at most 5 minutes at p95.
- Warm `wh check --json` on this repository: at most 2 seconds at p95; 10,000 supported source files: at most 10 seconds at p95.
- Single-file hook/check feedback: at most 2 seconds at p95.
- Local dashboard first usable content: at most 3 seconds at p95.
- R0 direct production dependencies: at most 12; R0 tracked legacy scripts and duplicate runtime tests: zero.
- R0 Rust source reduction: at least 70% from the 37,982-line baseline while preserving non-vacuous validate/eval/scan gates.
- Hard safety budgets: zero private content/ancestry leakage, zero unauthorized actions, zero stale/unknown/partial required evidence accepted as success, and zero duplicate work on replay.

Storage feasibility, remote topology, identity roots, and protected-CI authority are deliberately unresolved until `whetstone-k5r.2` and `.3`, after the lean baseline. Proposed default storage remains Dolt.

## Production source disposition

KEEP means the path survives R0 after unused symbols are carved away. REMOVE means delete the actual source/parser/dispatch path, not merely hide it.

| Disposition | Exact paths | Named consumer and retained evidence |
| --- | --- | --- |
| KEEP/CARVE | `src/main.rs`, `src/lib.rs`, `src/cli.rs`, `src/output.rs` | One release binary and a small R0-only developer gate surface: `validate`, `eval`, `scan`; bare invocation reports the honest foundation state. Replace the 3,700-line legacy parser/dispatcher and remove all old aliases. CLI contract tests prove JSON, failure exits, non-vacuous counts, and no retired commands. |
| KEEP/CARVE | `src/types.rs`, `src/rules.rs` | Minimum-MVP deterministic rule validation/loading plus the current validate/eval/self-scan gates. Remove dependency/lifecycle/catalog/status helpers and JSON projections not consumed by the kernel. Unit and integration tests cover schema rejection, approved-only loading, language scoping, and path safety. This legacy rule representation is replaced by M1 records, not treated as authenticated policy. |
| KEEP/CARVE | `src/ast/mod.rs` | Minimum-MVP AST enforcement for Python, TypeScript/JavaScript and Rust, shared by scan and golden eval. Retain parser/query primitives and parser tests only. |
| REMOVE | `src/ast/python.rs`, `src/ast/rust_lang.rs`, `src/ast/typescript.rs` | Convenience query catalogs are unused by the scanner and existed for retired debt discovery/tests. Scanner-owned arbitrary AST queries remain covered in `src/ast/mod.rs` and check tests. |
| KEEP/CARVE | `src/check/mod.rs`, `src/check/lint_proxy.rs` | Minimum-MVP deterministic scan, golden eval, native lint/formatter/test/validator binding verification, actionable JSON, and current gates. Remove preview packs, merged layers, legacy config/state writers, and dependency-discovery coupling. Read source-cache provenance through a narrow read-only parser only. Tests must assert nonzero files/rules/goldens and known-bad/good behavior; zero violations alone is insufficient. |
| REMOVE | `src/adherence.rs`, `src/report.rs`, `src/status.rs`, `src/worklist.rs` | Legacy scores, reports, status snapshots and extraction queues are outside the target product and would confuse observations with policy outcomes. Retire their tests rather than preserving writers. |
| REMOVE | `src/debt.rs` if present and `src/debt/**` | Legacy hotspot discovery/scoring and Beads task creation are outside the MVP; later proactive work uses an existing tracker boundary and cannot justify retention. |
| REMOVE | `src/tui/**` | The TUI and implicit launch path are explicitly excluded; dashboard work later supplies the human GUI. |
| REMOVE | `src/detect/**`, `src/resolve/**`, `src/source_mgmt.rs`, `src/corpus.rs`, `src/config_packs.rs` | Dependency-first discovery, online document resolution, starter/resource catalogs and pack imports are the old choreography. Documentation judgment belongs to the skill; M1 starts from explicit agreements. Carve only the source-directory skip list into `check`. |
| REMOVE | `src/config.rs`, `src/layers.rs`, `src/personal.rs`, `src/conflicts.rs` | The old YAML/layer/pack merge model is not the new accepted/required/installed/personal-overlay model. Direct rule reads support the temporary gate kernel; M1 implements the new model after R0. |
| REMOVE | `src/state/**`, `src/second_brain.rs` | The old JSON cache/inventory/refresh stores and knowledge graph are not transactional Dolt state. Preserve their data; a narrow read-only JSON cache reader may be carved into eval. Never retain the fixed `.tmp` writer or treat old approval as authenticated authority. |
| REMOVE | `src/onboard.rs`, `src/doctor.rs`, `src/extract.rs`, `src/handoff.rs` | Old dependency onboarding, diagnostics, extraction handoff and worklist are retired. The target `init` onboarding is implemented after R0 and must not reuse the old flow by alias. |
| REMOVE | `src/approve.rs`, `src/review.rs`, `src/rule_authoring.rs`, `src/rules_query.rs`, `src/guidance.rs` | Candidate/approved YAML mutation and old query/guidance surfaces do not satisfy versioned independent approval. User-authored files remain preserved as data. |
| REMOVE | `src/gen.rs`, `src/generate_context.rs`, `src/generate_lint.rs`, `src/generate_tests.rs`, `src/templates.rs`, `src/templates/**` | Old generated context/lint/test products and templates are replaced by shared M1 services and protected native wiring. Retaining them would create a second workflow. |
| REMOVE | `src/agent_hook.rs`, `src/mcp.rs`, `src/triggers.rs`, `src/ci_check.rs` | Old hooks, MCP, trigger installation and freshness CI are coupled to legacy state and commands. M1/M2 adapters are rebuilt against snapshot-bound receipts; preserving a facade is forbidden. |
| REMOVE | `src/private_mode.rs` | Preserve `.git/info/exclude` and private data exactly, but remove the old global visibility/publish writer. New private-safe record sharing is M2 and old `publish` must never become `push`. |
| REMOVE | `src/update.rs` | Self-update command/runtime is not part of the six workflows or minimum safeguards. Distribution hardening is M4. |
| REMOVE | `src/bin/wh.rs` | Duplicate binary target; `install.sh` already installs one `whetstone` artifact and exposes `wh` as a symlink. One compiled runtime avoids divergent surfaces. |

No other `src/` path is unclassified: the table names every top-level module and exhausts the `ast`, `check`, `debt`, `detect`, `resolve`, `state`, `templates`, and `tui` subtrees.

## Dependencies

| Disposition | Direct dependency |
| --- | --- |
| KEEP pending post-cut `cargo tree` proof | `anyhow`, `clap`, `serde`, `serde_json`, `serde_yaml`, `toml`, `tree-sitter`, `tree-sitter-python`, `tree-sitter-rust`, `tree-sitter-typescript`, `walkdir` |
| REMOVE | `indicatif`, `chrono`, `glob`, `rayon`, `regex`, `reqwest`, `scraper`, `sha2`, `tera`, `ratatui`, `crossterm` |
| KEEP dev-only | `tempfile` |

Every retained dependency has a direct kernel consumer: errors, parser, signal matching, record/receipt serialization, old rule input, native Cargo lint inspection, AST grammars, source traversal, or isolated tests. `cargo tree` and unused-dependency inspection at `.32` may remove more; no dependency is retained for later convenience.

## Tests, fixtures, scripts, packaging and instructions

| Disposition | Exact paths | Reason or replacement |
| --- | --- | --- |
| REMOVE | `tests/agent_hook.rs`, `tests/corpus_bundle.rs`, `tests/corpus_packs.rs`, `tests/mcp_server.rs`, `tests/private_mode.rs`, `tests/rust_integration.rs` | These suites primarily prove retired commands/products. Replace retained invariants with focused `tests/kernel_cli.rs`; do not port obsolete behavior. |
| REMOVE | `tests/test_script_contracts.py`, `tests/test_state.py`, `tests/test_incremental.py`, `scripts/legacy/**` | Duplicate Python runtime/parity/state behavior is removed together. Replace the non-empty Python gate with focused black-box R0 CLI/safety tests in `tests/test_kernel_contract.py`. |
| REMOVE | Existing `tests/fixtures/**` except any exact rule fixture copied into the focused kernel suite | Old dependency, generation, context and malformed-fixture trees belong to removed behavior. New tests create isolated explicit inputs. |
| REMOVE | `benchmarks/**`, `packs/**`, `assets/whetstone.yaml.template`, `scripts/measure-epic-3e.sh` | Legacy dependency/personal-layer benchmarks, bundled catalogs, config template and status benchmark have no minimum-MVP consumer. The retained Whetstone rule goldens keep eval real. |
| KEEP | `scripts/beads-repair.sh` | Repository issue-tracker recovery required by active project instructions; not product runtime. |
| KEEP | `.githooks/pre-push`, `.github/workflows/ci.yml` | Genuine quality/security gates. Rewrite CI to exercise only the meaningful eight local gates plus build/install checks for the lean runtime; remove legacy jobs and command assertions. |
| KEEP/CARVE | `Cargo.toml`, `.github/workflows/release.yml`, `install.sh` | One lean binary plus clean-checkout installation are direct consumers at `.16`. Update build/smoke assertions to the honest R0 surface; do not release or bump a version in R0. |
| REMOVE | `packaging/homebrew/whetstone.rb`, `packaging/homebrew/README.md` | Homebrew is broad M4 distribution and cannot justify surviving R0. Published tap/releases and Git history remain the recovery source. |
| REMOVE | `action.yml`, `.github/workflows/whetstone-example.yml` | Both expose the retired `wh ci` dependency-drift product. Protected target-policy CI is designed in `.3` and implemented in M2, not preserved by compatibility. |
| KEEP/CARVE | `references/rule-schema.yaml`, `references/signal-strategies.md` | Embedded schema and current deterministic-kernel explanation. Remove legacy workflow language during `.32`; M1 supersedes the schema. |
| REMOVE | `references/cli-vnext-migration.md`, `references/conflicts-schema.md`, `references/extraction-prompt.md`, `references/guidance-schema.yaml`, `references/handoff-schema.md`, `references/platform-registry-api.md`, `references/rust-python-differences.md`, `references/troubleshooting.md`, `references/workflow-matrix.md` | Old CLI, extraction, layer, handoff and platform contracts; Git history is the recovery source. |
| PRESERVE DATA | `whetstone/**`, `.git/info/exclude`, `.beads/**` | Never delete or rewrite during R0. Runtime readers may use only `whetstone/rules/**` and read-only source-cache evidence; other old records stay inert for later narrow import. |
| KEEP historical | `planning/archive/**` | Explicitly archived context is inert and useful only as history; it does not justify code retention. |
| KEEP current plan | `planning/direction-demo/**` | Canonical Direction 05 behavior and implementation evidence. |
| REMOVE | Other obsolete active `planning/*.md` files after exact review in `.32` | Old product roadmaps/designs must not remain active agent guidance. History remains in Git. |
| REMOVE | `pyproject.toml` | It declares an unrelated `demo` package and is not required by Ruff or Pytest. Python remains a development-only black-box gate. |
| REWRITE | `README.md`, `AGENTS.md`, `CLAUDE.md`, `SKILL.md`, `planning/skill-cli-boundary.md` | At `.32`, describe the lean foundation honestly, the six target workflows as not-yet-implemented, the R0 developer gates, data recovery, and the new skill/judgment boundary. Remove old command choreography and default-keep claims. |
| KEEP | `CHANGELOG.md`, `LICENSE`, `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md`, `.gitignore`, `.gitattributes`, `.github/ISSUE_TEMPLATE/**`, `.github/PULL_REQUEST_TEMPLATE.md` | Project governance, security and history; not legacy product runtime. Do not add a release entry until an authorized release is cut.

## Test retirement map and R0 exit proof

- Scanner/AST assertions in the old Rust integration suite become focused known-good/known-bad AST query, language scoping, skipped-directory, invalid-query, and config-issue tests in the retained modules and `tests/kernel_cli.rs`.
- Schema/rule loading assertions become valid/invalid/status/path tests in `rules.rs` and CLI tests; validation must report a positive checked-file count.
- Native lint/formatter/test/validator safety assertions remain in `check/lint_proxy.rs`, including repo-relative path and executable checks.
- Eval assertions remain in `check/mod.rs` and CLI tests; the gate must report positive `rules_evaluated`, `golden_checked`, and no mismatch.
- Private-mode suites are retired as behavior, while SHA-256 canaries prove user data and exclusion configuration were not touched.
- Install/release tests retain one binary plus the `wh` symlink and reject every retired command.
- Python parity/state/incremental suites are retired as duplicate behavior; focused Python black-box tests keep Ruff/format/Pytest non-empty and exercise real Rust output.
- CI must assert `files_scanned > 0`, `rules_applied > 0`, `golden_checked > 0`, and `config_issues_count == 0` where required. Green from zero work is a failure.

R0 exit compares the live tree against every row above, runs all eight gates without bypasses, proves one known-bad fixture fails and one known-good fixture passes, reproduces all user-data/privacy canaries, records post-cut dependency/LOC/file/binary measurements, and confirms retired command strings are absent from parser/dispatch/help/package surfaces. Any unclassified survivor blocks `.33`.
