# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
