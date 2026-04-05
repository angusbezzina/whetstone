# Changelog

All notable changes to Whetstone are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.0]: https://github.com/angusbezzina/whetstone/releases/tag/v0.1.0
