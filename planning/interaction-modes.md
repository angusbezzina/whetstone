# Human vs Agent Interaction Modes

> **Audit date:** 2026-04-26
> **Tracking:** whetstone-32yj

## Recommendation

Whetstone should be **TUI-first for humans** and **JSON-first for agents**.

- **Humans**: when running on an interactive TTY and not passing `--json`, enter
  the TUI by default.
- **Agents / scripts**: pass `--json` to get a stable machine contract.

## Why `--json`, not `--agents`

The dominant 2025–2026 CLI pattern is to expose a standard structured-output
flag, not an agent-specific mode name:

- **GitHub CLI**: `--json`
- **Docker**: `--format json`
- **kubectl**: `-o json`
- **uv**: TTY-aware human output plus structured/scripting-friendly flags

`--agents` would describe *who* is consuming the output rather than *what*
format is being requested. `--json` is the clearer contract and is already what
 agents and automation tools expect.

## Routing model

### Interactive TTY, no `--json`

- Launch the TUI for every command.
- Commands with a dedicated screen land on that screen (`status`, `check`,
  `extract`, `report`, `debt`, etc.).
- Commands without a dedicated screen land on a shared **Result** screen after
  the underlying action runs.

### `--json`

- Never launch the TUI.
- Return the machine-readable contract.

### Non-TTY without `--json`

- Avoid the TUI.
- Keep piped/redirected compatibility behavior intact.

## Shared-flow principle

Rather than inventing bespoke UIs per subcommand, reuse the same TUI shell:

- shared header/footer/navigation
- dedicated domain screens where they already exist
- shared Result screen for everything else

That keeps the human experience consistent while leaving `--json` as the agent
contract.
