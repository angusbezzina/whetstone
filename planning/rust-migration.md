# Rust Migration Architecture

> Covers beads 0hq.1.1 through 0hq.1.4

## Crate Layout

Single binary crate at the repo root (`Cargo.toml`). Modules mirror the Python
script responsibilities:

```
src/
  main.rs              # entry point
  cli.rs               # clap command definitions
  types.rs             # Language, Dependency, shared enums
  config.rs            # whetstone.yaml loading
  output.rs            # JSON envelope + human report helpers
  state/
    mod.rs             # StateManager facade
    manifest.rs        # ManifestStore
    inventory.rs       # InventoryStore
    cache.rs           # SourceCacheStore
    refresh.rs         # RefreshLog
  detect/
    mod.rs             # detect_deps() orchestrator
    walk.rs            # manifest file discovery
    python.rs          # pyproject.toml + requirements.txt
    typescript.rs      # package.json
    rust_lang.rs       # Cargo.toml
  resolve/
    mod.rs             # resolve_sources() orchestrator
    http.rs            # HTTP client helpers
    pypi.rs            # PyPI resolver
    npm.rs             # npm resolver
    crates_io.rs       # crates.io resolver
  doctor.rs            # doctor orchestration
  status.rs            # status computation + reporting
  ci_check.rs          # CI freshness checks
```

A workspace split (e.g. `whetstone-core` library) is deferred until the binary
stabilises. The module boundaries above are designed so a future split is
mechanical.

## Command Contract Compatibility

| Command          | JSON output | Flags        | Compatibility   |
|------------------|-------------|--------------|-----------------|
| `detect-deps`    | identical   | identical    | strict parity   |
| `resolve-sources`| identical   | identical    | strict parity   |
| `doctor`         | identical   | identical    | strict parity   |
| `status`         | identical   | identical    | strict parity   |
| `ci-check`       | identical   | identical    | strict parity   |

All commands produce the same JSON shape to stdout and progress messages to
stderr. The Rust binary is a drop-in replacement for the Python scripts.

## State Schema Migration Policy

- Rust reads the same `whetstone/.state/*.json` files as Python.
- Schema version field (`"version": 1`) is checked on load; mismatches reset
  the store (same as current Python behavior).
- No automatic migration is needed for v1→v1. Future schema bumps will use a
  version check + migration function pattern.
- State files written by Rust are readable by Python and vice versa.

## Deterministic vs Agent-Mediated Ownership

| Responsibility                | Owner         | Notes                           |
|-------------------------------|---------------|---------------------------------|
| Manifest discovery + parsing  | Rust binary   | Deterministic                   |
| Source resolution + caching   | Rust binary   | Deterministic (HTTP + cache)    |
| State persistence             | Rust binary   | Deterministic                   |
| Doctor orchestration          | Rust binary   | Deterministic pipeline          |
| Status computation            | Rust binary   | Deterministic                   |
| Pattern detection             | Rust binary   | Deterministic heuristics        |
| Rule extraction               | Agent (LLM)   | Requires judgment               |
| Rule approval                 | Human         | Interactive                     |
| Test generation (scaffolding) | Rust binary   | Deterministic templates         |
| Agent context generation      | Rust binary   | Deterministic templates         |
