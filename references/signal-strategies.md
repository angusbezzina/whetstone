# Deterministic signal strategies

This is how a rule's mechanical enforcer is chosen. Every rule has exactly
one, and a concern that no mechanical enforcer can hold stays in prose
(`AGENTS.md`, review) rather than becoming a weak rule. The boundary is in
`planning/skill-cli-boundary.md`.

## Three-bucket audit

Every candidate safeguard belongs to one bucket:

1. A native linter or formatter already expresses it: bind and verify that tool.
2. It needs taste, semantics, or type resolution: leave it to prose and review, not a rule.
3. It is type-independent structure no native tool catches: use a tree-sitter query.

Do not turn bucket 2 into a fake deterministic rule. Do not duplicate bucket 1
with custom scanning. Prefer a few trusted, actionable safeguards.

Each bucket-1 or bucket-3 safeguard is a rule's enforcer in
`whetstone/rulebook.json` ([schema](rulebook.schema.json)), drafted with
`wh change` and run by `wh check`.

## `ast`

A real tree-sitter S-expression in `query`, with an optional `language`.
Every node captured as `@match` is a violation, and the same source and query
always produce the same findings. A query that compiles for no language, or a
whole-repository check with no file in scope, is unknown, never a pass; there
is no fallback to raw text matching.

```json
{"kind": "ast", "language": "rust",
 "query": "((macro_invocation macro: (identifier) @name) @match (#eq? @name \"dbg\"))"}
```

## `lint`

The native linter's command and the exact rule code. The check runs the
linter with its structured reporter (Ruff, Clippy or Cargo, Biome, ESLint)
and reads the code from its JSON, so a code is never matched by searching
text.

```json
{"kind": "lint", "tool": "cargo clippy --all-targets -- -D warnings", "code": "dead_code"}
```

## Formatter, test, and validator commands

Bind existing mechanical or executable safeguards directly:

- `formatter`: the formatter in check mode, for example `cargo fmt --check`.
- `test`: a test command.
- `validator`: any other checking command.

Commands run without a shell, with a bounded environment and a time limit;
a timeout, an unavailable tool or a signal is unknown rather than a pass.

## A check that needs an outside tool

Some concerns can only be checked by a tool the repository does not hold: a
desktop app driven through the operating system's accessibility tree, a
service, a device, a permission. Such a check can fail to run for reasons
that say nothing about the code. Keep the two apart:

- Exit 0 when the code holds, and 1 (with `path:line message` lines) only
  when the check ran and the code broke the rule.
- Exit 69 when the check could not reach a verdict: the tool is not
  installed, a permission is not granted, the app or service is not running,
  the platform is not supported. The rule is unknown, which is never a pass:
  a must rule blocks the gate and says why, a should rule is named and never
  blocks, and a sweep files no finding for it.
- Start it as `should`, with `"mode": "change"` and `paths` scoped to the
  code it judges, until it has shown on real changes that it flags only what
  it should. Raise it to `must` only where every gate that runs it (the
  hooks on each contributor's machine, and CI) can reach the tool.
- Write it on a machine that has the tool: every rule is probed before it is
  written, and a probe that reaches no verdict refuses the rule.
- Keep what the tool observed out of the output. Standard output and
  standard error are kept in the evidence log, and a snapshot of a desktop
  can hold private content; print findings, not screens, and delete any
  temporary capture.

Rule commands keep the graphical session's variables (`DISPLAY`,
`WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS`,
`XAUTHORITY`) when the caller has them, so such a tool can reach the session
on Linux.

### Worked example: agent-desktop

[agent-desktop](https://github.com/vercel-labs/agent-desktop) reads and
operates desktop apps through the accessibility tree and prints one JSON
envelope per command. Its exit status is 0 on success, 1 for a structured
error and 2 for a bad argument, and the error names a `code`
([docs/json-output.md](https://github.com/vercel-labs/agent-desktop/blob/main/docs/json-output.md)).
`PERM_DENIED` (no accessibility permission), `PLATFORM_NOT_SUPPORTED`,
`APP_NOT_FOUND` (not running, or no windows) and `APP_UNRESPONSIVE` say the
check could not run, so they map to 69. Whether a missing element or an
expired wait is a violation is the rule's own judgment.

```sh
#!/bin/sh
# whetstone/checks/desktop-buttons-named.sh: every button in the app's main
# window has an accessible name. Exit 69 when the check cannot run here.
set -u
for tool in agent-desktop jq; do
  command -v "$tool" >/dev/null 2>&1 || { echo "$tool is not installed"; exit 69; }
done
snapshot=$(mktemp)
trap 'rm -f "$snapshot"' EXIT
if ! agent-desktop snapshot --app "MyApp" > "$snapshot"; then
  code=$(jq -r '.error.code // "UNKNOWN"' "$snapshot")
  case "$code" in
    PERM_DENIED | PLATFORM_NOT_SUPPORTED | APP_NOT_FOUND | APP_UNRESPONSIVE)
      echo "agent-desktop could not observe the app: $code"; exit 69 ;;
  esac
  echo "agent-desktop failed: $code"; exit 69
fi
# The rule's own judgment over the snapshot: print one finding line per
# unnamed button (for example `src/ui/Toolbar.tsx: a button has no name`)
# and exit 1 when there is any.
node whetstone/checks/buttons-named.mjs "$snapshot"
```

```json
{"type": "rule", "strength": "should", "mode": "change", "paths": ["src/ui/**"],
 "enforcer": {"kind": "validator", "command": "sh whetstone/checks/desktop-buttons-named.sh"}}
```

An error the script does not recognise is also 69 here: the check did not
run to a result, and an unexplained failure must not read as a violation.
Whetstone holds no agent-desktop code; the tool is one command a
repository's own script calls.

## Raw patterns

Raw regex is not an enforcement strategy. Use a native-tool binding or an
AST query.

## Initial language support

| Language | AST grammar | Native lint verification |
| --- | --- | --- |
| Python | tree-sitter-python | Ruff |
| TypeScript / JavaScript | tree-sitter-typescript | Biome |
| Rust | tree-sitter-rust | Clippy |
