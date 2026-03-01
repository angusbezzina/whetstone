#!/usr/bin/env python3
"""Whetstone CLI — unified command dispatcher.

Provides a single `whetstone` entry point that dispatches to individual scripts.

Usage:
    python3 scripts/cli.py doctor --project-dir .
    python3 scripts/cli.py status --score
    python3 scripts/cli.py detect-deps --changed-only

Subcommands map directly to scripts:
    doctor          → scripts/doctor.py
    status          → scripts/status.py
    ci-check        → scripts/ci-check.py
    detect-deps     → scripts/detect-deps.py
    resolve-sources → scripts/resolve-sources.py
    detect-patterns → scripts/detect-patterns.py
    generate-tests  → scripts/generate-tests.py
    generate-context → scripts/generate-agent-context.py
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

# Map subcommand names to script filenames
COMMANDS: dict[str, str] = {
    "doctor": "doctor.py",
    "status": "status.py",
    "ci-check": "ci-check.py",
    "detect-deps": "detect-deps.py",
    "resolve-sources": "resolve-sources.py",
    "detect-patterns": "detect-patterns.py",
    "generate-tests": "generate-tests.py",
    "generate-context": "generate-agent-context.py",
}

# Short aliases
ALIASES: dict[str, str] = {
    "check": "ci-check",
    "deps": "detect-deps",
    "patterns": "detect-patterns",
    "tests": "generate-tests",
    "context": "generate-context",
}


def _scripts_dir() -> Path:
    """Return the scripts directory (same directory as this file)."""
    return Path(__file__).resolve().parent


def _print_usage() -> None:
    """Print usage help to stderr."""
    print("Whetstone — sharpen the tools that write your code.\n", file=sys.stderr)
    print("Usage: whetstone <command> [options]\n", file=sys.stderr)
    print("Commands:", file=sys.stderr)
    for name, script in COMMANDS.items():
        print(f"  {name:<20s} → {script}", file=sys.stderr)
    print(
        "\nAliases: check → ci-check, deps → detect-deps, patterns → detect-patterns,",
        file=sys.stderr,
    )
    print(
        "         tests → generate-tests, context → generate-context", file=sys.stderr
    )
    print(
        "\nAll remaining arguments are passed through to the script.",
        file=sys.stderr,
    )
    print(
        "\nExamples:",
        file=sys.stderr,
    )
    print("  whetstone doctor", file=sys.stderr)
    print("  whetstone status --score", file=sys.stderr)
    print("  whetstone deps --changed-only", file=sys.stderr)
    print("  whetstone patterns --sources git,pr", file=sys.stderr)


def main() -> int:
    args = sys.argv[1:]

    if not args or args[0] in ("--help", "-h", "help"):
        _print_usage()
        return 0 if args else 1

    subcommand = args[0]
    rest = args[1:]

    # Resolve aliases
    if subcommand in ALIASES:
        subcommand = ALIASES[subcommand]

    if subcommand not in COMMANDS:
        print(f"Error: Unknown command '{subcommand}'", file=sys.stderr)
        print(f"Available: {', '.join(sorted(COMMANDS.keys()))}", file=sys.stderr)
        print(
            f"Aliases: {', '.join(f'{k} → {v}' for k, v in sorted(ALIASES.items()))}",
            file=sys.stderr,
        )
        return 1

    script = _scripts_dir() / COMMANDS[subcommand]
    if not script.exists():
        print(f"Error: Script not found: {script}", file=sys.stderr)
        return 1

    # Execute the script, passing through all remaining args
    result = subprocess.run(
        [sys.executable, str(script)] + rest,
        stdin=sys.stdin,
    )
    return result.returncode


if __name__ == "__main__":
    sys.exit(main())
