#!/usr/bin/env python3
"""Whetstone CI Check — lightweight freshness check for CI/CD pipelines.

Runs dependency drift detection and source freshness checks, outputs
machine-readable JSON with status fields suitable for GitHub Action outputs.

Usage:
    python3 scripts/ci-check.py --project-dir .
    python3 scripts/ci-check.py --project-dir . --json
    python3 scripts/ci-check.py --project-dir . --fail-on stale
    python3 scripts/ci-check.py --project-dir . --pr-comment
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path


def _script_dir() -> Path:
    """Return the directory containing this script."""
    return Path(__file__).resolve().parent


def _run_script(name: str, args: list[str]) -> dict | None:
    """Run a sibling script and return parsed JSON output."""
    script = _script_dir() / name
    try:
        result = subprocess.run(
            [sys.executable, str(script)] + args,
            capture_output=True,
            text=True,
            timeout=60,
        )
        if result.returncode == 0:
            return json.loads(result.stdout)
        try:
            return json.loads(result.stdout)
        except (json.JSONDecodeError, ValueError):
            return {"error": result.stderr.strip() or f"{name} failed"}
    except Exception as e:
        return {"error": str(e)}


def ci_check(
    project_dir: Path,
    check_drift: bool = True,
) -> dict:
    """Run CI freshness checks and return structured results."""
    start = time.monotonic()

    # Get status
    status_result = _run_script(
        "status.py",
        [
            "--project-dir",
            str(project_dir),
            "--json",
            *(["--no-drift-check"] if not check_drift else []),
        ],
    )

    if status_result is None:
        return {
            "freshness_status": "error",
            "error": "Failed to run status check",
            "requires_review": True,
        }

    if status_result.get("status") == "not_initialized":
        return {
            "freshness_status": "not_initialized",
            "changed_sources_count": 0,
            "recommended_rules_count": 0,
            "requires_review": False,
            "score": 0,
            "label": "Not Initialized",
            "message": "Whetstone not initialized in this project.",
            "elapsed_seconds": round(time.monotonic() - start, 1),
        }

    # Extract key fields
    label = status_result.get("label", "Unknown")
    score = status_result.get("score", 0)
    dims = status_result.get("dimensions", {})
    drift = status_result.get("drift", {})
    recommendations = status_result.get("recommendations", [])

    # Determine freshness status
    freshness_days = dims.get("freshness_days")
    pending_updates = dims.get("pending_updates", 0)

    if label == "Healthy":
        freshness_status = "healthy"
    elif label == "Needs Review":
        freshness_status = "needs_review"
    elif label == "Stale":
        freshness_status = "stale"
    elif label == "No Rules":
        freshness_status = "no_rules"
    else:
        freshness_status = "unknown"

    # Determine if review is required
    requires_review = freshness_status in ("stale", "needs_review")

    elapsed = round(time.monotonic() - start, 1)

    return {
        "freshness_status": freshness_status,
        "changed_sources_count": pending_updates,
        "recommended_rules_count": len(recommendations),
        "requires_review": requires_review,
        "score": score,
        "label": label,
        "dimensions": dims,
        "recommendations": recommendations,
        "elapsed_seconds": elapsed,
        "next_command": status_result.get("next_command", ""),
    }


def format_pr_comment(result: dict) -> str:
    """Format result as a GitHub PR comment body."""
    marker = "<!-- whetstone-ci-check -->"

    status_emoji = {
        "healthy": "OK",
        "needs_review": "!!",
        "stale": "XX",
        "no_rules": "--",
        "not_initialized": "--",
    }.get(result.get("freshness_status", ""), "??")

    lines = [
        marker,
        "## Whetstone Status",
        "",
        f"**[{status_emoji}] {result.get('label', 'Unknown')}** (score: {result.get('score', 0)}/100)",
        "",
    ]

    dims = result.get("dimensions", {})
    if dims:
        lines.append("| Metric | Value |")
        lines.append("|--------|-------|")
        freshness = dims.get("freshness_days")
        if freshness is not None:
            lines.append(f"| Freshness | {freshness:.0f} days |")
        lines.append(f"| Rules | {dims.get('rules_count', 0)} approved |")
        lines.append(
            f"| High confidence | {dims.get('high_confidence_ratio', 0):.0f}% |"
        )
        lines.append(
            f"| Deterministic coverage | {dims.get('deterministic_coverage', 0):.0f}% |"
        )
        lines.append(f"| Pending updates | {dims.get('pending_updates', 0)} deps |")
        lines.append("")

    recs = result.get("recommendations", [])
    if recs and recs != ["Everything looks good. No action needed."]:
        lines.append("### Recommendations")
        lines.append("")
        for rec in recs:
            lines.append(f"- {rec}")
        lines.append("")

    next_cmd = result.get("next_command", "")
    if next_cmd:
        lines.append(f"**Next:** `{next_cmd}`")
        lines.append("")

    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Whetstone CI Check — freshness check for CI/CD pipelines."
    )
    parser.add_argument(
        "--project-dir",
        type=Path,
        default=Path("."),
        help="Project root directory (default: .)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        dest="json_mode",
        help="Output JSON only",
    )
    parser.add_argument(
        "--pr-comment",
        action="store_true",
        help="Output as GitHub PR comment markdown",
    )
    parser.add_argument(
        "--fail-on",
        choices=["stale", "needs_review", "none"],
        default="none",
        help="Exit with error code on specified status (default: none)",
    )
    parser.add_argument(
        "--no-drift-check",
        action="store_true",
        help="Skip dependency drift check (faster)",
    )
    args = parser.parse_args()

    try:
        result = ci_check(
            project_dir=args.project_dir,
            check_drift=not args.no_drift_check,
        )

        if args.pr_comment:
            print(format_pr_comment(result))
        elif args.json_mode:
            json.dump(result, sys.stdout, indent=2)
            sys.stdout.write("\n")
        else:
            # Human-readable summary to stderr, JSON to stdout
            status = result.get("freshness_status", "unknown")
            label = result.get("label", "Unknown")
            score = result.get("score", 0)
            print(
                f"Whetstone: [{status.upper()}] {label} (score: {score}/100)",
                file=sys.stderr,
            )

            recs = result.get("recommendations", [])
            for rec in recs:
                print(f"  - {rec}", file=sys.stderr)

            next_cmd = result.get("next_command", "")
            if next_cmd:
                print(f"  Next: {next_cmd}", file=sys.stderr)

            json.dump(result, sys.stdout, indent=2)
            sys.stdout.write("\n")

        # Exit code based on --fail-on
        freshness = result.get("freshness_status", "unknown")
        if args.fail_on == "stale" and freshness == "stale":
            sys.exit(1)
        elif args.fail_on == "needs_review" and freshness in ("stale", "needs_review"):
            sys.exit(1)

    except Exception as e:
        json.dump({"error": str(e)}, sys.stdout, indent=2)
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
