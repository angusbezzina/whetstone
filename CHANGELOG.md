# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-04-12

### Added
- **Multi-tier content fetching** — 4-tier resolve pipeline: llms.txt →
  registry README (npm/PyPI/crates.io) → HTML docs conversion → GitHub
  changelog. All dependencies now get content (previously null for non-llms.txt).
- **Changelog discovery** — probes GitHub repos for CHANGELOG.md, filters
  to last 18 months, includes as a separate `sections` entry alongside README.
- **Sections array** — resolver output now includes labeled sections (readme,
  changelog, llms_txt) for per-section extraction.
- **Custom source support** — `sources.custom` in `whetstone.yaml` lets
  users add arbitrary URLs (blogs, team guides, any public page).
- **Built-in rules** — 5 Rust rules ship embedded in the binary
  (`whetstone:recommended`). Project rules override by ID. Deny list support.
- **`match` field on signals** — concrete regex patterns that enable real
  test generation instead of TODO stubs.
- **Real regex test generation** — generated tests scan source files with
  actual regex checks, reporting violations with file path and line number.
- **`wh refresh` command** — detect drift and re-resolve changed deps.
  `--check` flag for CI exits non-zero on drift.
- **Source attribution** — `content_origin` (how binary fetched it) and
  `source_kind` (official_docs, changelog, blog, social, etc.) fields.
- **`wh validate` checks real rules** — now validates `whetstone/rules/`
  in addition to test fixtures.

### Changed
- **SKILL.md rewritten** — teaches agents the sections/changelog/source_kind
  model, match patterns for signals, and the full extraction workflow.
- **README.md** — comparison table (vs Semgrep, Continue.dev, CodeRabbit),
  worked example showing full extraction flow, updated capabilities section.
- **Extraction prompt** — multi-section content guidance, source_kind
  attribution requirement, match pattern documentation.

### Fixed
- `wh validate` now checks `whetstone/rules/` (was only checking test fixtures).

## [0.1.2] - 2026-04-05

### Added
- **`wh update`** — self-update command that downloads the latest release
  binary from GitHub, verifies sha256 checksum, and replaces the running
  binary atomically. Flags: `--check` (just check), `--force` (reinstall).
- **`wh` binary alias** — short name for `whetstone`, installed alongside
  the main binary.
- **Progress bar** during dependency resolution via indicatif.
- **Human-friendly default output** — all commands now print readable text
  by default. Use `--json` (global flag) for machine-readable JSON.
  Auto-detects piped stdout.
- **Scoped package grouping** — `@radix-ui/*` and similar npm scopes shown
  as a single grouped line in human output; JSON gains a `scope` field.

### Changed
- Command renames (old names kept as hidden aliases):
  `detect-deps`→`init`, `resolve-sources`→`set-sources`,
  `generate-context`→`context`, `generate-tests`→`tests`,
  `validate-rules`→`validate`, `detect-patterns`→`patterns`,
  `ci-check`→`ci`. Doctor gains visible alias `start`.

### Fixed
- Box-drawing characters now consistent (no mixed ASCII `=` and Unicode `═`).
- `status` no longer prints "Monorepo detected" twice.

## [0.1.1] - 2026-04-05

### Fixed
- Switch reqwest from native-tls to rustls-tls so cross-compilation for
  aarch64-unknown-linux-gnu no longer requires a system OpenSSL. The binary
  is now fully self-contained on all targets.
- Update macOS x86_64 CI runner from deprecated macos-13 to macos-14.

## [0.1.0] - 2026-04-05

First public release. Whetstone is a single self-contained Rust binary with
no Python runtime dependency.

### Added
- **Dependency detection** (`detect-deps`) for Python, TypeScript, and Rust,
  including monorepo support and incremental fingerprinting.
- **Documentation resolution** (`resolve-sources`) via PyPI, npm, and
  crates.io registry APIs with llms.txt probing and content hashing.
- **One-command bootstrap** (`doctor`) from zero to working rules.
- **Health monitoring** (`status`) with drift detection, freshness scoring,
  dimensional breakdown, and append-only metric snapshots.
- **CI integration** (`ci-check`) with JSON output, PR comment generation,
  and configurable `--fail-on` thresholds.
- **Agent context generation** (`generate-context`) for CLAUDE.md, AGENTS.md,
  .cursorrules, and three other formats.
- **Test generation** (`generate-tests`) producing pytest, vitest, and cargo
  test scaffolds plus ruff, biome, and clippy lint overlays.
- **Pattern mining** (`detect-patterns`) from agent transcripts, git history,
  and GitHub PR review comments. Project-scoped by default for privacy.
- **Rule schema validation** (`validate-rules`) replacing the legacy Python
  validator with identical output.
- **GitHub Action** (`action.yml`) for CI freshness gating with PR comments.
- **Install script** (`install.sh`) with sha256 checksum verification,
  platform detection, and `--version` pinning.
- **Homebrew formula template** (`packaging/homebrew/whetstone.rb`).
- **Release workflow** building Linux and macOS binaries for x86_64 and
  aarch64 with cross-compilation support.

[0.1.2]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.2
[0.1.1]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.1
[0.1.0]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.0
