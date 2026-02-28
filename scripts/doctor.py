#!/usr/bin/env python3
"""Whetstone Doctor — single command to go from zero to working rules.

Orchestrates the full init flow: detect deps → resolve sources → (optional)
detect patterns → hand off to agent for extraction → generate outputs.

This is the "first value" command: one invocation, sensible defaults,
useful output in ~5 minutes.

Usage:
    python3 scripts/doctor.py --project-dir .
    python3 scripts/doctor.py --project-dir . --skip-patterns --skip-dev
    python3 scripts/doctor.py --project-dir . --json
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path


def _script_dir() -> Path:
    """Return the directory containing this script."""
    return Path(__file__).resolve().parent


def _run_script(
    name: str,
    args: list[str],
    stdin_data: str | None = None,
) -> tuple[dict | None, float]:
    """Run a sibling script and return parsed JSON output + elapsed seconds."""
    script = _script_dir() / name
    cmd = [sys.executable, str(script)] + args
    start = time.monotonic()

    try:
        result = subprocess.run(
            cmd,
            input=stdin_data,
            capture_output=True,
            text=True,
            timeout=120,
        )
        elapsed = time.monotonic() - start

        if result.returncode != 0:
            # Try to parse error JSON from stdout
            try:
                return json.loads(result.stdout), elapsed
            except (json.JSONDecodeError, ValueError):
                return {
                    "error": result.stderr.strip()
                    or f"{name} exited with code {result.returncode}"
                }, elapsed

        try:
            return json.loads(result.stdout), elapsed
        except (json.JSONDecodeError, ValueError):
            return {
                "error": f"Invalid JSON from {name}",
                "raw": result.stdout[:500],
            }, elapsed

    except subprocess.TimeoutExpired:
        elapsed = time.monotonic() - start
        return {"error": f"{name} timed out after 120s"}, elapsed
    except Exception as e:
        elapsed = time.monotonic() - start
        return {"error": f"Failed to run {name}: {e}"}, elapsed


def _log(msg: str, json_mode: bool = False) -> None:
    """Print a progress message to stderr (so stdout stays clean for JSON)."""
    if not json_mode:
        print(msg, file=sys.stderr)


def doctor(
    project_dir: Path,
    skip_patterns: bool = False,
    skip_dev: bool = True,
    json_mode: bool = False,
    deps_filter: str | None = None,
) -> dict:
    """Run the full doctor flow and return a structured result."""

    total_start = time.monotonic()
    steps: list[dict] = []
    warnings: list[str] = []

    # ── Step 1: Detect dependencies ──────────────────────────────────────
    _log("Step 1/4: Detecting dependencies...", json_mode)

    deps_result, deps_time = _run_script(
        "detect-deps.py",
        ["--project-dir", str(project_dir)],
    )

    if deps_result is None or "error" in deps_result:
        error_msg = (deps_result or {}).get(
            "error", "Unknown error detecting dependencies"
        )
        return {
            "status": "error",
            "error": error_msg,
            "step": "detect-deps",
            "steps": steps,
        }

    deps_count = deps_result.get("counts", {}).get("runtime", {}).get("_all", 0)
    dev_count = deps_result.get("counts", {}).get("dev", {}).get("_all", 0)
    languages = deps_result.get("languages", [])

    steps.append(
        {
            "name": "detect-deps",
            "status": "ok",
            "elapsed_seconds": round(deps_time, 1),
            "summary": f"Found {deps_count} runtime deps (+{dev_count} dev) across {', '.join(languages) or 'no languages'}",
        }
    )

    _log(
        f"  Found {deps_count} runtime dependencies (+{dev_count} dev) "
        f"across {', '.join(languages)}  [{deps_time:.1f}s]",
        json_mode,
    )

    # Filter to non-dev deps by default
    all_deps = deps_result.get("dependencies", [])
    if skip_dev:
        target_deps = [d for d in all_deps if not d.get("dev", False)]
    else:
        target_deps = all_deps

    if deps_filter:
        filter_set = set(deps_filter.split(","))
        target_deps = [d for d in target_deps if d["name"] in filter_set]

    if not target_deps:
        return {
            "status": "ok",
            "warning": "No dependencies to extract rules for",
            "steps": steps,
            "summary": {
                "dependencies_found": deps_count,
                "dependencies_targeted": 0,
                "sources_resolved": 0,
                "patterns_found": 0,
                "languages": languages,
            },
            "next_command": "Add dependencies to your project, then run whetstone doctor again",
        }

    dep_names = [d["name"] for d in target_deps]

    # ── Step 2: Resolve documentation sources ────────────────────────────
    _log(
        f"Step 2/4: Resolving documentation for {len(target_deps)} dependencies...",
        json_mode,
    )

    resolve_args = ["--project-dir", str(project_dir)]
    if deps_filter:
        resolve_args += ["--deps", deps_filter]

    resolve_result, resolve_time = _run_script(
        "resolve-sources.py",
        resolve_args,
        stdin_data=json.dumps(deps_result),
    )

    if resolve_result is None or "error" in resolve_result:
        error_msg = (resolve_result or {}).get(
            "error", "Unknown error resolving sources"
        )
        steps.append(
            {
                "name": "resolve-sources",
                "status": "error",
                "elapsed_seconds": round(resolve_time, 1),
                "error": error_msg,
            }
        )
        return {
            "status": "error",
            "error": error_msg,
            "step": "resolve-sources",
            "steps": steps,
        }

    sources = resolve_result.get("sources", [])
    errors = resolve_result.get("errors", [])
    llms_txt_count = sum(
        1 for s in sources if s.get("source_type") in ("llms_txt", "llms_full_txt")
    )

    steps.append(
        {
            "name": "resolve-sources",
            "status": "ok",
            "elapsed_seconds": round(resolve_time, 1),
            "summary": (
                f"Resolved docs for {len(sources)}/{len(target_deps)} deps "
                f"({llms_txt_count} with llms.txt)"
            ),
        }
    )

    if errors:
        for err in errors:
            warnings.append(
                f"Could not resolve docs for {err['name']}: {err.get('error', 'unknown')}"
            )

    _log(
        f"  Resolved {len(sources)}/{len(target_deps)} deps "
        f"({llms_txt_count} with llms.txt)  [{resolve_time:.1f}s]",
        json_mode,
    )

    # ── Step 3: Detect patterns (optional) ───────────────────────────────
    patterns_result = None
    patterns_count = 0

    if not skip_patterns:
        _log("Step 3/4: Mining style patterns from history...", json_mode)

        patterns_result, patterns_time = _run_script(
            "detect-patterns.py",
            ["--project-dir", str(project_dir)],
        )

        if patterns_result and "error" not in patterns_result:
            patterns_count = len(patterns_result.get("patterns", []))
            steps.append(
                {
                    "name": "detect-patterns",
                    "status": "ok",
                    "elapsed_seconds": round(patterns_time, 1),
                    "summary": f"Found {patterns_count} recurring style patterns",
                }
            )
            _log(
                f"  Found {patterns_count} recurring style patterns  [{patterns_time:.1f}s]",
                json_mode,
            )
        else:
            # Pattern detection is optional — don't fail
            pattern_error = (patterns_result or {}).get("error", "unknown")
            warnings.append(f"Pattern detection skipped: {pattern_error}")
            steps.append(
                {
                    "name": "detect-patterns",
                    "status": "skipped",
                    "elapsed_seconds": round(patterns_time, 1),
                    "warning": pattern_error,
                }
            )
            _log(
                f"  Pattern detection skipped (non-fatal)  [{patterns_time:.1f}s]",
                json_mode,
            )
    else:
        _log("Step 3/4: Skipping pattern detection (--skip-patterns)", json_mode)
        steps.append(
            {
                "name": "detect-patterns",
                "status": "skipped",
                "elapsed_seconds": 0,
                "summary": "Skipped by user request",
            }
        )

    # ── Step 4: Prepare extraction handoff ───────────────────────────────
    _log("Step 4/4: Preparing extraction handoff...", json_mode)

    # Build the extraction context for the agent
    extraction_context = {
        "sources": sources,
        "patterns": (patterns_result or {}).get("patterns", []),
        "languages": languages,
        "dep_names": [s["name"] for s in sources],
    }

    steps.append(
        {
            "name": "extraction-ready",
            "status": "ok",
            "elapsed_seconds": 0,
            "summary": (
                f"Ready for extraction: {len(sources)} sources, "
                f"{patterns_count} patterns"
            ),
        }
    )

    total_elapsed = time.monotonic() - total_start

    _log("", json_mode)
    _log("=" * 60, json_mode)
    _log("  Whetstone Doctor — Summary", json_mode)
    _log("=" * 60, json_mode)
    _log(f"  Dependencies found:  {deps_count} runtime, {dev_count} dev", json_mode)
    _log(f"  Languages:           {', '.join(languages)}", json_mode)
    _log(
        f"  Sources resolved:    {len(sources)}/{len(target_deps)} ({llms_txt_count} llms.txt)",
        json_mode,
    )
    _log(f"  Patterns found:      {patterns_count}", json_mode)
    _log(f"  Total time:          {total_elapsed:.1f}s", json_mode)
    _log("=" * 60, json_mode)
    _log("", json_mode)

    if sources:
        _log(
            "  Ready for extraction. The agent will now read each source,",
            json_mode,
        )
        _log(
            "  propose high-confidence rules, and ask you to approve them.",
            json_mode,
        )
        _log("", json_mode)
        _log(
            "  Next: Agent applies extraction prompt to each source",
            json_mode,
        )
    else:
        _log("  No sources resolved. Check warnings for details.", json_mode)
        _log("", json_mode)
        _log(
            "  Next: Add --deps to target specific dependencies, or "
            "provide manual docs URLs",
            json_mode,
        )

    if warnings:
        _log("", json_mode)
        _log("  Warnings:", json_mode)
        for w in warnings:
            _log(f"    - {w}", json_mode)

    # Determine next command
    if sources:
        next_command = (
            "Agent: apply extraction prompt to sources, then run generate scripts"
        )
    else:
        next_command = "Resolve source issues above, then re-run whetstone doctor"

    return {
        "status": "ok",
        "steps": steps,
        "summary": {
            "dependencies_found": deps_count,
            "dependencies_targeted": len(target_deps),
            "sources_resolved": len(sources),
            "sources_with_llms_txt": llms_txt_count,
            "patterns_found": patterns_count,
            "languages": languages,
            "elapsed_seconds": round(total_elapsed, 1),
        },
        "extraction_context": extraction_context,
        "warnings": warnings,
        "next_command": next_command,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Whetstone Doctor — single command from zero to rules.",
    )
    parser.add_argument(
        "--project-dir",
        type=Path,
        default=Path("."),
        help="Project root directory (default: .)",
    )
    parser.add_argument(
        "--skip-patterns",
        action="store_true",
        help="Skip pattern detection from transcripts/git/PRs",
    )
    parser.add_argument(
        "--skip-dev",
        action="store_true",
        default=True,
        help="Skip dev dependencies (default: true)",
    )
    parser.add_argument(
        "--include-dev",
        action="store_true",
        help="Include dev dependencies in extraction",
    )
    parser.add_argument(
        "--deps",
        type=str,
        help="Comma-separated list of dependency names to target",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        dest="json_mode",
        help="Output only JSON (suppress progress messages)",
    )
    args = parser.parse_args()

    skip_dev = not args.include_dev

    try:
        result = doctor(
            project_dir=args.project_dir,
            skip_patterns=args.skip_patterns,
            skip_dev=skip_dev,
            json_mode=args.json_mode,
            deps_filter=args.deps,
        )

        json.dump(result, sys.stdout, indent=2)
        sys.stdout.write("\n")

        if result.get("status") == "error":
            sys.exit(1)

    except Exception as e:
        json.dump({"error": str(e)}, sys.stdout, indent=2)
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
