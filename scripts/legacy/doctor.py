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
    python3 scripts/doctor.py --project-dir . --verbose
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path

# State management — optional import for incremental mode
try:
    from state import StateManager
except ImportError:
    StateManager = None  # type: ignore[assignment,misc]


DEFAULT_FAST_FIRST_MAX_DEPS = 10


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


def _count_existing_rules(project_dir: Path) -> int:
    """Count existing approved rules in whetstone/rules/."""
    rules_dir = project_dir / "whetstone" / "rules"
    if not rules_dir.exists():
        return 0

    count = 0
    try:
        import re

        for yaml_file in rules_dir.rglob("*.yaml"):
            text = yaml_file.read_text()
            # Count approved: true entries
            count += len(re.findall(r"^\s*approved:\s*true\s*$", text, re.MULTILINE))
    except Exception:
        pass
    return count


def _rank_dependencies(
    deps: list[dict],
    sm: object | None = None,
) -> list[dict]:
    """Rank dependencies by resolution priority.

    Scoring model:
    - Runtime deps: +100 (dev: +0)
    - Source quality from prior cache: llms_full_txt +50, llms_txt +40, docs_url_only +10
    - Multi-workspace: +20 per additional workspace beyond the first
    - Stale cache entry: +30
    - Failed retry: +5

    Returns deps sorted by score descending, with score added to each dict.
    """
    scored = []
    for dep in deps:
        score = 0.0
        name = dep["name"]
        language = dep["language"]
        version = dep.get("version", "*")

        # Runtime vs dev
        if not dep.get("dev", False):
            score += 100

        # Prior source quality from cache
        if sm is not None and hasattr(sm, "cache"):
            cached = sm.cache.get(language, name, version)
            if cached:
                st = cached.get("source_type", "")
                if st == "llms_full_txt":
                    score += 50
                elif st == "llms_txt":
                    score += 40
                elif st == "docs_url_only":
                    score += 10

        # Multi-workspace
        sources_count = len(dep.get("sources", []))
        score += max(0, (sources_count - 1)) * 20

        # Staleness and failure from inventory
        if sm is not None and hasattr(sm, "inventory"):
            inv = sm.inventory.get(language, name)
            if inv:
                state = inv.get("state", "")
                if state == "stale":
                    score += 30
                elif state == "failed":
                    score += 5

        scored.append({**dep, "_score": score})

    scored.sort(key=lambda d: d["_score"], reverse=True)
    return scored


def _classify_deps(
    deps: list[dict],
    sm: object | None = None,
) -> dict:
    """Classify dependencies into cache buckets.

    Returns dict with cached, stale, missing, failed lists.
    """
    cached, stale, missing, failed = [], [], [], []

    for dep in deps:
        name = dep["name"]
        language = dep["language"]
        version = dep.get("version", "*")

        if sm is None or not hasattr(sm, "cache"):
            missing.append(dep)
            continue

        entry = sm.cache.get(language, name, version)
        if entry is None:
            missing.append(dep)
        elif entry.get("errors"):
            failed.append(dep)
        elif sm.cache.is_fresh(language, name, version):
            cached.append(dep)
        else:
            stale.append(dep)

    return {
        "cached": cached,
        "stale": stale,
        "missing": missing,
        "failed": failed,
    }


def _build_source_details(sources: list[dict], errors: list[dict]) -> list[dict]:
    """Build per-source detail list sorted by confidence."""
    details = []

    for s in sources:
        source_type = s.get("source_type", "unknown")
        if source_type in ("llms_txt", "llms_full_txt"):
            confidence = "high"
        elif source_type in ("docs_url", "readme"):
            confidence = "medium"
        else:
            confidence = "low"
        details.append(
            {
                "name": s.get("name", "unknown"),
                "source_type": source_type,
                "confidence": confidence,
                "status": "resolved",
            }
        )

    for e in errors:
        details.append(
            {
                "name": e.get("name", "unknown"),
                "source_type": None,
                "confidence": None,
                "status": "failed",
                "error": e.get("error", "unknown"),
            }
        )

    # Sort: resolved high first, then medium, then low, then failed
    confidence_order = {"high": 0, "medium": 1, "low": 2, None: 3}
    details.sort(key=lambda d: confidence_order.get(d.get("confidence"), 3))

    return details


def _build_recommendations(
    sources: list[dict],
    errors: list[dict],
    llms_txt_count: int,
    existing_rules: int,
    auto_limited: bool = False,
    remaining_count: int = 0,
) -> list[dict]:
    """Build structured recommendations based on doctor findings."""
    recs = []

    if sources:
        recs.append(
            {
                "priority": "high",
                "action": "extract",
                "message": f"Extract rules for {len(sources)} dependencies with resolved docs",
            }
        )

    if llms_txt_count > 0:
        recs.append(
            {
                "priority": "high",
                "action": "prioritize",
                "message": (
                    f"{llms_txt_count} deps have llms.txt — "
                    "these will produce highest quality rules"
                ),
            }
        )

    if errors:
        recs.append(
            {
                "priority": "medium",
                "action": "resolve",
                "message": (
                    f"Consider providing manual docs URLs for "
                    f"{len(errors)} unresolved dependencies"
                ),
            }
        )

    if existing_rules > 0:
        recs.append(
            {
                "priority": "low",
                "action": "review",
                "message": f"{existing_rules} existing rules found — doctor will update them",
            }
        )

    if not sources and not errors:
        recs.append(
            {
                "priority": "high",
                "action": "add-deps",
                "message": "No dependencies found. Add dependencies to your project first.",
            }
        )

    if auto_limited and remaining_count > 0:
        recs.append(
            {
                "priority": "high",
                "action": "continue",
                "message": (
                    f"Fast-first mode resolved the top {len(sources)} dependencies; "
                    f"resume to continue with {remaining_count} remaining"
                ),
                "command": "whetstone doctor --resume",
            }
        )

    return recs


def _format_report(
    result: dict,
    project_dir: Path,
    verbose: bool = False,
) -> str:
    """Build a structured box-drawing report for stderr output."""
    W = 62  # inner width (between ║ markers)

    def line(text: str = "") -> str:
        """Format a content line with box borders."""
        return f"\u2551  {text:<{W - 2}}\u2551"

    def section_header(title: str) -> str:
        """Format a section header."""
        padded = f"\u2550\u2550 {title} "
        return f"\u2560{padded}{'=' * (W - len(padded))}\u2563"

    lines = []

    # Top border
    lines.append(f"\u2554{'=' * W}\u2557")

    # Title
    summary = result.get("summary", {})
    existing_rules = result.get("_existing_rules", 0)
    if existing_rules > 0:
        lines.append(
            line(f"Whetstone Doctor Report (Update — {existing_rules} existing rules)")
        )
    else:
        lines.append(line("Whetstone Doctor Report"))

    # Project + date header
    lines.append(section_header(""))
    lines.append(line())
    lines.append(line(f"Project: {project_dir.resolve()}"))
    lines.append(line(f"Date:    {datetime.now(timezone.utc).strftime('%Y-%m-%d')}"))
    lines.append(line())

    # Dependencies section
    lines.append(section_header("Dependencies"))
    lines.append(line())

    deps_found = summary.get("dependencies_found", 0)
    dev_count = result.get("_dev_count", 0)
    languages = summary.get("languages", [])

    lines.append(line(f"Found {deps_found} runtime + {dev_count} dev dependencies"))
    lines.append(line(f"Languages: {', '.join(languages) if languages else 'none'}"))
    lines.append(line())

    # Per-language counts
    lang_counts = result.get("_lang_counts", {})
    if lang_counts:
        lines.append(line("Runtime deps by language:"))
        for lang, count in sorted(lang_counts.items()):
            lines.append(line(f"  {lang + ':':14s} {count} deps"))
        lines.append(line())

    # Scan Summary section (Phase A)
    scan = result.get("scan", {})
    cache_stats = scan.get("cache_stats", {})
    if any(cache_stats.values()):
        lines.append(section_header("Scan Summary"))
        lines.append(line())
        lines.append(
            line(
                f"Cached:  {cache_stats.get('cached', 0):>3}    "
                f"Stale:   {cache_stats.get('stale', 0):>3}"
            )
        )
        lines.append(
            line(
                f"Missing: {cache_stats.get('missing', 0):>3}    "
                f"Failed:  {cache_stats.get('failed', 0):>3}"
            )
        )
        lines.append(line())

    # Documentation Sources section (Phase B)
    lines.append(section_header("Documentation Sources"))
    lines.append(line())

    sources_resolved = summary.get("sources_resolved", 0)
    deps_targeted = summary.get("dependencies_targeted", 0)
    llms_count = summary.get("sources_with_llms_txt", 0)
    source_details = result.get("source_details", [])
    failed_count = sum(1 for d in source_details if d.get("status") == "failed")
    docs_only = sources_resolved - llms_count

    lines.append(line(f"Resolved: {sources_resolved}/{deps_targeted} dependencies"))
    lines.append(line(f"With llms.txt: {llms_count}"))
    lines.append(line(f"Docs URL only: {docs_only}"))
    if failed_count > 0:
        lines.append(line(f"Failed: {failed_count} (see warnings below)"))

    # Show resolution buckets
    buckets = result.get("resolution_buckets", {})
    ready_now = buckets.get("ready_now", [])
    if ready_now:
        lines.append(line(f"Extraction ready: {len(ready_now)} deps"))

    lines.append(line())

    # Top sources
    if source_details:
        lines.append(line("Top sources:"))
        show_count = len(source_details) if verbose else min(5, len(source_details))
        for detail in source_details[:show_count]:
            name = detail.get("name", "unknown")
            if detail.get("status") == "resolved":
                stype = detail.get("source_type", "unknown")
                conf = detail.get("confidence", "unknown")
                lines.append(line(f"  + {name:<16s} -- {stype} ({conf} confidence)"))
            else:
                lines.append(line(f"  x {name:<16s} -- no docs found"))
        if not verbose and len(source_details) > 5:
            lines.append(
                line(
                    f"  ... and {len(source_details) - 5} more (use --verbose to show all)"
                )
            )
        lines.append(line())

    # Style Patterns section
    patterns_count = summary.get("patterns_found", 0)
    lines.append(section_header("Style Patterns"))
    lines.append(line())
    if patterns_count > 0:
        lines.append(
            line(f"Found {patterns_count} recurring patterns from project history")
        )
    else:
        lines.append(line("No patterns detected (or pattern detection skipped)"))
    lines.append(line())

    # Recommendations section
    recommendations = result.get("recommendations", [])
    if recommendations:
        lines.append(section_header("Recommendations"))
        lines.append(line())
        for i, rec in enumerate(recommendations, 1):
            msg = rec.get("message", str(rec)) if isinstance(rec, dict) else str(rec)
            # Wrap long messages
            if len(msg) > W - 6:
                # Split into multiple lines
                words = msg.split()
                current = f"{i}. "
                for word in words:
                    if len(current) + len(word) + 1 > W - 4:
                        lines.append(line(current))
                        current = "   " + word
                    else:
                        current += (" " if len(current) > 3 else "") + word
                if current.strip():
                    lines.append(line(current))
            else:
                lines.append(line(f"{i}. {msg}"))
        lines.append(line())

        next_cmd = result.get("next_command", "")
        if next_cmd:
            lines.append(line(f"Next: {next_cmd}"))
            lines.append(line())

    # Warnings section
    warnings = result.get("warnings", [])
    if warnings:
        lines.append(section_header("Warnings"))
        lines.append(line())
        for w in warnings:
            # Wrap long warnings
            if len(w) > W - 6:
                words = w.split()
                current = "* "
                for word in words:
                    if len(current) + len(word) + 1 > W - 4:
                        lines.append(line(current))
                        current = "  " + word
                    else:
                        current += (" " if len(current) > 2 else "") + word
                if current.strip():
                    lines.append(line(current))
            else:
                lines.append(line(f"* {w}"))
        lines.append(line())

    # Timing
    elapsed = summary.get("elapsed_seconds", 0)
    steps = result.get("steps", [])
    lines.append(section_header("Timing"))
    lines.append(line())
    for step in steps:
        step_name = step.get("name", "unknown")
        step_time = step.get("elapsed_seconds", 0)
        step_status = step.get("status", "?")
        indicator = (
            "+" if step_status == "ok" else ("~" if step_status == "skipped" else "x")
        )
        lines.append(line(f"  {indicator} {step_name:<22s} {step_time:>5.1f}s"))
    lines.append(line(f"  {'Total:':<24s} {elapsed:>5.1f}s"))
    lines.append(line())

    # Bottom border
    lines.append(f"\u255a{'=' * W}\u255d")

    return "\n".join(lines)


def doctor(
    project_dir: Path,
    skip_patterns: bool = False,
    skip_dev: bool = True,
    json_mode: bool = False,
    deps_filter: str | None = None,
    verbose: bool = False,
    changed_only: bool = False,
    refresh: bool = False,
    resume: bool = False,
    max_deps: int | None = None,
    ready_only: bool = False,
    workers: int | None = None,
    full_run: bool = False,
) -> dict:
    """Run the full doctor flow and return a structured result.

    Phase A (fast scan): detect → fingerprint → classify → rank → report
    Phase B (incremental resolve): resolve changed/missing/stale → report
    """

    total_start = time.monotonic()
    steps: list[dict] = []
    warnings: list[str] = []

    # Initialize state manager for incremental support
    sm = None
    if StateManager is not None:
        sm = StateManager(project_dir)
        sm.ensure_dir()
        sm.load_all()

    # Check for existing rules (repeat run detection)
    existing_rules = _count_existing_rules(project_dir)

    # ══════════════════════════════════════════════════════════════════════
    # PHASE A: Fast Scan
    # ══════════════════════════════════════════════════════════════════════

    # ── Step 1: Detect dependencies ──────────────────────────────────────
    _log("Step 1/4: Detecting dependencies...", json_mode)

    detect_args = ["--project-dir", str(project_dir)]
    if sm is not None:
        detect_args.append("--incremental")

    deps_result, deps_time = _run_script(
        "detect-deps.py",
        detect_args,
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
            "recommendations": [],
            "source_details": [],
            "next_command": "Check project directory has manifest files (pyproject.toml, package.json, Cargo.toml)",
        }

    deps_count = deps_result.get("counts", {}).get("runtime", {}).get("_all", 0)
    dev_count = deps_result.get("counts", {}).get("dev", {}).get("_all", 0)
    languages = deps_result.get("languages", [])

    # Compute per-language runtime counts
    lang_counts: dict[str, int] = {}
    runtime_counts = deps_result.get("counts", {}).get("runtime", {})
    for lang in languages:
        lang_counts[lang] = runtime_counts.get(lang, 0)

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
            "recommendations": [],
            "source_details": [],
            "scan": {"cache_stats": {}, "ranked_queue": []},
            "next_command": "Add dependencies to your project, then run whetstone doctor again",
        }

    # ── Phase A: Classify and rank dependencies ──────────────────────────
    # Reload state in case detect-deps updated it
    if sm is not None:
        sm.cache.load()
        sm.inventory.load()

    cache_buckets = _classify_deps(target_deps, sm)
    ranked_queue = _rank_dependencies(target_deps, sm)

    scan_info = {
        "manifests_changed": deps_result.get("manifests_changed"),
        "manifest_diff": deps_result.get("manifest_diff"),
        "inventory_diff": deps_result.get("inventory_diff"),
        "cache_stats": {
            "cached": len(cache_buckets["cached"]),
            "stale": len(cache_buckets["stale"]),
            "missing": len(cache_buckets["missing"]),
            "failed": len(cache_buckets["failed"]),
        },
        "ranked_queue": [
            {"name": d["name"], "language": d["language"], "score": d.get("_score", 0)}
            for d in ranked_queue[:20]
        ],
    }

    _log(
        f"  Scan: {len(cache_buckets['cached'])} cached, "
        f"{len(cache_buckets['stale'])} stale, "
        f"{len(cache_buckets['missing'])} missing, "
        f"{len(cache_buckets['failed'])} failed",
        json_mode,
    )

    # ═════════════════════════════════════��════════════════════════════════
    # PHASE B: Incremental Resolution
    # ═���════════════════════════════════════════════════════════════════════

    # Build the resolve work list based on flags
    ranked_key_order = [f"{d['language']}:{d['name']}" for d in ranked_queue]

    if changed_only:
        resolve_deps = cache_buckets["stale"] + cache_buckets["missing"]
        # Also include deps whose manifests changed
        manifest_diff = deps_result.get("manifest_diff", {})
        changed_manifests = set(
            manifest_diff.get("changed", []) + manifest_diff.get("added", [])
        )
        if changed_manifests:
            inv_diff = deps_result.get("inventory_diff", {})
            changed_dep_keys = set(
                inv_diff.get("changed", []) + inv_diff.get("added", [])
            )
            for dep in cache_buckets["cached"]:
                key = f"{dep['language']}:{dep['name']}"
                if key in changed_dep_keys:
                    resolve_deps.append(dep)
    else:
        resolve_deps = target_deps

    resolve_dep_map = {f"{d['language']}:{d['name']}": d for d in resolve_deps}
    resolve_deps = [
        resolve_dep_map[k] for k in ranked_key_order if k in resolve_dep_map
    ]

    auto_limited = False
    auto_limit_target = DEFAULT_FAST_FIRST_MAX_DEPS
    if (
        max_deps is None
        and not full_run
        and not changed_only
        and not refresh
        and not resume
        and not ready_only
        and not deps_filter
        and len(cache_buckets["cached"]) == 0
        and len(resolve_deps) > auto_limit_target
    ):
        resolve_deps = resolve_deps[:auto_limit_target]
        auto_limited = True
        _log(
            f"  Fast-first: limiting initial resolution to top {auto_limit_target} deps; use --full-run or --resume to continue",
            json_mode,
        )

    if max_deps is not None:
        resolve_deps = resolve_deps[:max_deps]

    # ── Step 2: Resolve documentation sources ──────────��─────────────────
    _log(
        f"Step 2/4: Resolving documentation for {len(resolve_deps)} dependencies...",
        json_mode,
    )

    resolve_args = ["--project-dir", str(project_dir)]
    if deps_filter:
        resolve_args += ["--deps", deps_filter]
    elif resolve_deps and len(resolve_deps) < len(target_deps):
        # Only resolve the subset
        dep_names = ",".join(d["name"] for d in resolve_deps)
        resolve_args += ["--deps", dep_names]
    if refresh:
        resolve_args.append("--force-refresh")
    if resume:
        resolve_args.append("--resume")
    if workers is not None:
        resolve_args += ["--workers", str(workers)]

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
            "recommendations": [],
            "source_details": [],
            "next_command": "Check network connectivity and dependency names",
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

    # Classify resolution results into buckets
    ready_now = [
        s
        for s in sources
        if s.get("source_type") in ("llms_full_txt", "llms_txt")
        and s.get("freshness", {}).get("confidence") == "high"
    ]
    resolved_low = [s for s in sources if s not in ready_now]

    resolution_buckets = {
        "ready_now": [
            {"name": s["name"], "source_type": s.get("source_type")} for s in ready_now
        ],
        "resolved_low": [
            {"name": s["name"], "source_type": s.get("source_type")}
            for s in resolved_low
        ],
        "failed": [{"name": e["name"], "error": e.get("error")} for e in errors],
        "cached": [{"name": d["name"]} for d in cache_buckets["cached"]],
    }

    extraction_subsets = {
        "ready_now": [
            {"name": s["name"], "source_type": s.get("source_type")} for s in ready_now
        ],
        "resolved_not_ready": [
            {"name": s["name"], "reason": s.get("source_type", "unknown")}
            for s in resolved_low
        ],
        "pending": [{"name": d["name"]} for d in cache_buckets.get("missing", [])],
        "failed": [{"name": e["name"]} for e in errors],
    }

    # Build the extraction context for the agent
    extraction_sources = ready_now if ready_only else sources
    extraction_context = {
        "sources": extraction_sources,
        "patterns": (patterns_result or {}).get("patterns", []),
        "languages": languages,
        "dep_names": [s["name"] for s in extraction_sources],
    }

    steps.append(
        {
            "name": "extraction-ready",
            "status": "ok",
            "elapsed_seconds": 0,
            "summary": (
                f"Ready for extraction: {len(extraction_sources)} sources, "
                f"{patterns_count} patterns"
            ),
        }
    )

    total_elapsed = time.monotonic() - total_start

    # Build source details and recommendations
    source_details = _build_source_details(sources, errors)
    recommendations = _build_recommendations(
        sources,
        errors,
        llms_txt_count,
        existing_rules,
        auto_limited=auto_limited,
        remaining_count=max(0, len(target_deps) - len(resolve_deps)),
    )

    # Determine next command
    if auto_limited:
        next_command = "whetstone doctor --resume"
    elif extraction_sources:
        next_command = "whetstone status --extraction-ready"
    elif sources:
        next_command = "whetstone status"
    else:
        next_command = "whetstone doctor --refresh"

    result = {
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
        "source_details": source_details,
        "recommendations": recommendations,
        "extraction_context": extraction_context,
        "scan": scan_info,
        "resolution_buckets": resolution_buckets,
        "extraction_subsets": extraction_subsets,
        "warnings": warnings,
        "next_command": next_command,
        "workflow": {
            "fast_first": auto_limited,
            "remaining_dependencies": max(0, len(target_deps) - len(resolve_deps)),
            "resolved_this_run": len(resolve_deps),
        },
        # Private fields for report formatting (not part of public contract)
        "_existing_rules": existing_rules,
        "_dev_count": dev_count,
        "_lang_counts": lang_counts,
    }

    # Format and print human-readable report
    if not json_mode:
        report = _format_report(result, project_dir, verbose=verbose)
        print(report, file=sys.stderr)

    return result


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
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Show full source list instead of top N in report",
    )
    parser.add_argument(
        "--changed-only",
        action="store_true",
        help="Only resolve deps where manifests or versions changed",
    )
    parser.add_argument(
        "--refresh",
        action="store_true",
        help="Force re-resolve even cached dependencies",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="Resume incomplete resolution from last checkpoint",
    )
    parser.add_argument(
        "--max-deps",
        type=int,
        default=None,
        help="Maximum number of dependencies to resolve in this run",
    )
    parser.add_argument(
        "--ready-only",
        action="store_true",
        help="Only hand off extraction-ready deps for extraction",
    )
    parser.add_argument(
        "--workers",
        type=int,
        default=None,
        help="Number of parallel source-resolution workers (default: auto)",
    )
    parser.add_argument(
        "--full-run",
        action="store_true",
        help="Disable fast-first limiting and resolve the full dependency set now",
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
            verbose=args.verbose,
            changed_only=args.changed_only,
            refresh=args.refresh,
            resume=args.resume,
            max_deps=args.max_deps,
            ready_only=args.ready_only,
            workers=args.workers,
            full_run=args.full_run,
        )

        # Remove private fields before JSON output
        output = {k: v for k, v in result.items() if not k.startswith("_")}
        json.dump(output, sys.stdout, indent=2)
        sys.stdout.write("\n")

        if result.get("status") == "error":
            sys.exit(1)

    except Exception as e:
        json.dump(
            {
                "error": str(e),
                "recommendations": [],
                "source_details": [],
                "next_command": "Check project directory and script dependencies",
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
