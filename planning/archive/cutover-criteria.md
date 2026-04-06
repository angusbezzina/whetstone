# Python to Rust Cutover Criteria

## Summary

All core commands have full Rust parity. detect-patterns is the only unported feature.

## Per-Command Parity Matrix

| Command | Rust Parity | Test Proof | Python Blockers | Cutover Status |
|---------|-------------|------------|-----------------|----------------|
| detect-deps | Full | rust_integration.rs: 4 tests | None | Ready |
| resolve-sources | Full | rust_integration.rs (via doctor) | None | Ready |
| doctor | Full (minus patterns) | rust_integration.rs (via status) | None | Ready |
| status | Full | rust_integration.rs: 6 tests | None | Ready |
| ci-check | Full | rust_integration.rs: 1 test | None | Ready |
| generate-context | Full | rust_integration.rs: 1 test | None | Ready |
| generate-tests | Full | rust_integration.rs: 1 test | None | Ready |
| detect-patterns | NOT ported | N/A | See crd.7 analysis | Deferred |

## Cutover Gate Checklist

For any command to be considered "cut over":
1. Rust integration tests pass for all fixture scenarios
2. JSON output contract matches (same required keys, types, shapes)
3. All CLI flags implemented
4. SKILL.md references binary command, not script
5. No user-facing docs mention the Python path for that command
