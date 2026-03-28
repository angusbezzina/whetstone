#!/usr/bin/env python3
"""Whetstone Status — compact project health summary.

Reads whetstone/rules/*.yaml and whetstone.yaml, computes health dimensions:
  - freshness: days since last extraction
  - rules_count: total approved rules
  - high_confidence_ratio: % of rules with confidence=high
  - deterministic_coverage: % of signals that are ast/pattern/lint_proxy (not ai)
  - pending_updates: number of deps with version drift

Outputs a status label: Healthy, Needs Review, or Stale.

Usage:
    python3 scripts/status.py --project-dir .
    python3 scripts/status.py --project-dir . --json
    python3 scripts/status.py --project-dir . --score
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None  # type: ignore[assignment]


# --- YAML loading ---


def _load_yaml(filepath: Path) -> dict | None:
    """Load a YAML file using PyYAML (preferred) or regex fallback."""
    try:
        text = filepath.read_text()
        if yaml:
            return yaml.safe_load(text)
        # Fallback: minimal regex extraction for environments without PyYAML
        return _regex_parse_rule_file(text)
    except Exception:
        return None


def _regex_parse_rule_file(text: str) -> dict | None:
    """Regex-based fallback for parsing rule YAML when PyYAML unavailable."""
    source: dict = {}
    for field in ("name", "version", "content_hash"):
        match = re.search(rf"^\s*{field}:\s*(.+)$", text, re.MULTILINE)
        if match:
            source[field] = match.group(1).strip().strip("'\"")

    rules = []
    rule_blocks = re.split(r"^  - id:", text, flags=re.MULTILINE)
    for block in rule_blocks[1:]:
        rule_id = block.split("\n")[0].strip()
        rule: dict = {"id": rule_id}
        for field in ("severity", "confidence", "category", "approved_at"):
            match = re.search(rf"^\s*{field}:\s*(.+)$", block, re.MULTILINE)
            if match:
                rule[field] = match.group(1).strip().strip("'\"")
        approved_match = re.search(r"^\s*approved:\s*(.+)$", block, re.MULTILINE)
        rule["approved"] = (
            approved_match.group(1).strip().lower() == "true"
            if approved_match
            else False
        )
        rule["signals"] = [
            {"strategy": s}
            for s in re.findall(r"^\s+strategy:\s+(\w+)", block, re.MULTILINE)
        ]
        rules.append(rule)

    return {"source": source, "rules": rules}


VALID_SEVERITIES = frozenset({"must", "should", "may"})
VALID_CONFIDENCES = frozenset({"high", "medium"})
VALID_CATEGORIES = frozenset(
    {"migration", "default", "convention", "breaking-change", "semantic"}
)
VALID_STRATEGIES = frozenset({"ast", "pattern", "lint_proxy", "ai"})


def _validate_rule(rule: dict, filepath: str) -> list[str]:
    """Validate required fields and enum values on a rule. Returns list of warnings."""
    warnings = []
    rule_id = rule.get("id", "unknown")

    # Required fields
    for field in ("id", "severity", "confidence"):
        if not rule.get(field):
            warnings.append(
                f"{filepath}: rule '{rule_id}' missing required field '{field}'"
            )

    # Enum validation
    severity = rule.get("severity")
    if severity and severity not in VALID_SEVERITIES:
        warnings.append(
            f"{filepath}: rule '{rule_id}' has invalid severity '{severity}' "
            f"(expected one of: {', '.join(sorted(VALID_SEVERITIES))})"
        )

    confidence = rule.get("confidence")
    if confidence and confidence not in VALID_CONFIDENCES:
        warnings.append(
            f"{filepath}: rule '{rule_id}' has invalid confidence '{confidence}' "
            f"(expected one of: {', '.join(sorted(VALID_CONFIDENCES))})"
        )

    category = rule.get("category")
    if category and category not in VALID_CATEGORIES:
        warnings.append(
            f"{filepath}: rule '{rule_id}' has invalid category '{category}' "
            f"(expected one of: {', '.join(sorted(VALID_CATEGORIES))})"
        )

    # Signals validation
    signals = rule.get("signals")
    if not signals:
        warnings.append(f"{filepath}: rule '{rule_id}' has no signals")
    elif isinstance(signals, list):
        for i, sig in enumerate(signals):
            if isinstance(sig, dict):
                strategy = sig.get("strategy")
                if strategy and strategy not in VALID_STRATEGIES:
                    warnings.append(
                        f"{filepath}: rule '{rule_id}' signal {i} has invalid "
                        f"strategy '{strategy}' "
                        f"(expected one of: {', '.join(sorted(VALID_STRATEGIES))})"
                    )

    return warnings


def load_rule_files(rules_dir: Path) -> tuple[list[dict], list[str]]:
    """Load metadata from all rule YAML files.

    Returns (rule_files, warnings).
    """
    rule_files = []
    warnings: list[str] = []

    if not rules_dir.exists():
        return rule_files, warnings

    for yaml_file in sorted(rules_dir.rglob("*.yaml")):
        data = _load_yaml(yaml_file)
        if data is None:
            warnings.append(f"Failed to parse: {yaml_file}")
            continue

        source = data.get("source", {})
        source_name = source.get("name", yaml_file.stem)
        source_version = source.get("version")
        content_hash = source.get("content_hash")

        rules = []
        for rule_data in data.get("rules", []):
            # Validate
            rule_warnings = _validate_rule(rule_data, str(yaml_file))
            warnings.extend(rule_warnings)

            # Normalize signals to list of strategy strings
            signals = rule_data.get("signals", [])
            signal_strategies = []
            for sig in signals:
                if isinstance(sig, dict):
                    signal_strategies.append(sig.get("strategy", "unknown"))
                elif isinstance(sig, str):
                    signal_strategies.append(sig)

            rules.append(
                {
                    "id": rule_data.get("id", "unknown"),
                    "severity": rule_data.get("severity"),
                    "confidence": rule_data.get("confidence"),
                    "category": rule_data.get("category"),
                    "approved": bool(rule_data.get("approved", False)),
                    "approved_at": rule_data.get("approved_at"),
                    "signals": signal_strategies,
                }
            )

        rule_files.append(
            {
                "file": str(yaml_file),
                "source_name": source_name,
                "source_version": source_version,
                "content_hash": content_hash,
                "rules": rules,
            }
        )

    return rule_files, warnings


def compute_freshness_days(rule_files: list[dict]) -> float | None:
    """Compute days since the most recent rule approval."""
    latest_approval: datetime | None = None

    for rf in rule_files:
        for rule in rf["rules"]:
            if rule.get("approved_at"):
                try:
                    ts = rule["approved_at"]
                    # PyYAML may parse timestamps to datetime objects directly
                    if isinstance(ts, datetime):
                        dt = ts
                    else:
                        # Parse ISO 8601 string
                        ts_str = str(ts)
                        if ts_str.endswith("Z"):
                            ts_str = ts_str[:-1] + "+00:00"
                        dt = datetime.fromisoformat(ts_str)
                    if latest_approval is None or dt > latest_approval:
                        latest_approval = dt
                except (ValueError, TypeError, AttributeError):
                    continue

    if latest_approval is None:
        return None

    now = datetime.now(timezone.utc)
    if latest_approval.tzinfo is None:
        latest_approval = latest_approval.replace(tzinfo=timezone.utc)

    return (now - latest_approval).total_seconds() / 86400


def _compute_freshness_label(freshness_days: float | None) -> str:
    """Compute a human-readable freshness label."""
    if freshness_days is None:
        return "Unknown"
    if freshness_days < 7:
        return "Fresh"
    if freshness_days < 30:
        return "Current"
    if freshness_days < 60:
        return "Aging"
    return "Stale"


def _compute_last_extraction_date(rule_files: list[dict]) -> str | None:
    """Compute the ISO 8601 date string of the most recent extraction."""
    latest: datetime | None = None

    for rf in rule_files:
        for rule in rf["rules"]:
            if rule.get("approved_at"):
                try:
                    ts = rule["approved_at"]
                    if isinstance(ts, datetime):
                        dt = ts
                    else:
                        ts_str = str(ts)
                        if ts_str.endswith("Z"):
                            ts_str = ts_str[:-1] + "+00:00"
                        dt = datetime.fromisoformat(ts_str)
                    if latest is None or dt > latest:
                        latest = dt
                except (ValueError, TypeError, AttributeError):
                    continue

    if latest is None:
        return None

    if latest.tzinfo is None:
        latest = latest.replace(tzinfo=timezone.utc)

    return latest.isoformat()


def _run_detect_deps(
    project_dir: Path, extra_args: list[str] | None = None
) -> dict | None:
    """Run detect-deps.py and return parsed JSON output, or None on failure."""
    script = Path(__file__).resolve().parent / "detect-deps.py"
    cmd = [sys.executable, str(script), "--project-dir", str(project_dir)]
    if extra_args:
        cmd.extend(extra_args)
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=30)
        if result.returncode == 0:
            return json.loads(result.stdout)
    except Exception:
        pass
    return None


def check_drift(project_dir: Path) -> dict:
    """Run detect-deps --check-drift and return normalized drift info.

    Returns: {"changed": [...], "count": N, "checked": M}
    """
    empty: dict = {"changed": [], "count": 0, "checked": 0}
    data = _run_detect_deps(project_dir, ["--check-drift"])
    if data is not None:
        drift = data.get("drift", empty)
        # Defensive: handle legacy list format
        if isinstance(drift, list):
            drift = {"changed": drift, "count": len(drift), "checked": 0}
        return drift
    return empty


def _count_project_deps(project_dir: Path) -> int:
    """Get the total number of runtime dependencies from detect-deps.

    Returns 0 if detection fails — callers should treat 0 as "unknown".
    """
    data = _run_detect_deps(project_dir)
    if data is None:
        return 0
    deps = data.get("dependencies", [])
    # Count only runtime (non-dev) deps
    return sum(1 for d in deps if not d.get("dev", False))


def compute_status(
    project_dir: Path,
    check_dep_drift: bool = True,
    changed_only: bool = False,
) -> dict:
    """Compute the full project health status.

    Args:
        project_dir: Root directory of the project.
        check_dep_drift: Whether to check for dependency version drift.
        changed_only: If True, scope evaluation to only dependencies with
            version drift. Implies drift checking.
    """
    rules_dir = project_dir / "whetstone" / "rules"
    config_path = project_dir / "whetstone" / "whetstone.yaml"

    # changed-only implies drift checking
    if changed_only:
        check_dep_drift = True

    # Check if whetstone is initialized
    if not rules_dir.exists() and not config_path.exists():
        return {
            "status": "not_initialized",
            "label": "Not Initialized",
            "message": "No whetstone directory found. Run 'whetstone doctor' to get started.",
            "next_command": "whetstone doctor",
        }

    # Load rule files
    rule_files, load_warnings = load_rule_files(rules_dir)

    # Initialize drift_info (may be populated below)
    drift_info: dict = {}

    # If changed-only, get drifted dep names and filter rule_files
    if changed_only:
        drift_info = check_drift(project_dir)
        drifted_names = {
            c.get("name", "").lower() for c in drift_info.get("changed", [])
        }
        if drifted_names:
            rule_files = [
                rf for rf in rule_files if rf["source_name"].lower() in drifted_names
            ]
        # If no drift found, return early with healthy status
        if not drifted_names:
            return {
                "status": "ok",
                "label": "Healthy",
                "score": 100,
                "changed_only": True,
                "freshness_label": "Unknown",
                "last_extraction_date": None,
                "dimensions": {
                    "freshness_days": None,
                    "rules_count": 0,
                    "high_confidence_ratio": 0,
                    "deterministic_coverage": 0,
                    "pending_updates": 0,
                },
                "breakdown": {
                    "confidence": {"high": 0, "medium": 0},
                    "severity": {"must": 0, "should": 0, "may": 0},
                    "categories": {},
                    "signals": {"deterministic": 0, "ai": 0, "total": 0},
                },
                "dependencies_covered": [],
                "drift": {"changed": [], "count": 0, "checked": 0},
                "metrics": {
                    "rules_approved": 0,
                    "rules_proposed": 0,
                    "approval_rate": 0,
                    "must_rules": 0,
                    "dependencies_covered": 0,
                    "dependencies_total": 0,
                    "dependency_coverage": 0,
                    "deterministic_coverage": 0,
                    "pending_drift": 0,
                },
                "recommendations": [
                    {
                        "priority": "low",
                        "action": "info",
                        "message": "No dependency drift detected. Everything is current.",
                        "command": None,
                    }
                ],
                "warnings": [],
                "next_command": "whetstone status",
                "message": "No dependency drift detected.",
            }

    # Aggregate rule data
    all_rules = []
    dep_names = set()
    for rf in rule_files:
        dep_names.add(rf["source_name"])
        all_rules.extend(rf["rules"])

    approved_rules = [r for r in all_rules if r.get("approved")]
    total_rules = len(approved_rules)

    # Unapproved (proposed) rules
    unapproved_rules = [r for r in all_rules if not r.get("approved")]

    # Confidence breakdown
    high_confidence = sum(1 for r in approved_rules if r.get("confidence") == "high")
    medium_confidence = sum(
        1 for r in approved_rules if r.get("confidence") == "medium"
    )
    high_confidence_ratio = (
        (high_confidence / total_rules * 100) if total_rules > 0 else 0
    )

    # Signal coverage
    all_signals = []
    for r in approved_rules:
        all_signals.extend(r.get("signals", []))

    deterministic_signals = [
        s for s in all_signals if s in ("ast", "pattern", "lint_proxy")
    ]
    ai_signals = [s for s in all_signals if s == "ai"]
    total_signals = len(all_signals)
    deterministic_coverage = (
        (len(deterministic_signals) / total_signals * 100) if total_signals > 0 else 0
    )

    # Severity breakdown
    must_count = sum(1 for r in approved_rules if r.get("severity") == "must")
    should_count = sum(1 for r in approved_rules if r.get("severity") == "should")
    may_count = sum(1 for r in approved_rules if r.get("severity") == "may")

    # Category breakdown
    categories: dict[str, int] = {}
    for r in approved_rules:
        cat = r.get("category", "unknown")
        categories[cat] = categories.get(cat, 0) + 1

    # Freshness
    freshness_days = compute_freshness_days(rule_files)
    freshness_label = _compute_freshness_label(freshness_days)
    last_extraction_date = _compute_last_extraction_date(rule_files)

    # Drift check
    # When changed_only is True, drift_info was already computed above
    # during the filtering step. Otherwise compute or skip.
    if not changed_only:
        if check_dep_drift:
            drift_info = check_drift(project_dir)
        else:
            drift_info = {}
    drifted_count = len(drift_info.get("changed", []))

    # Compute status label
    label = _compute_label(
        total_rules=total_rules,
        freshness_days=freshness_days,
        deterministic_coverage=deterministic_coverage,
        drifted_count=drifted_count,
    )

    # Compute optional score
    score = _compute_score(
        total_rules=total_rules,
        freshness_days=freshness_days,
        deterministic_coverage=deterministic_coverage,
        high_confidence_ratio=high_confidence_ratio,
        drifted_count=drifted_count,
        dep_count=len(dep_names),
    )

    # Build recommendations
    recommendations = _build_recommendations(
        total_rules=total_rules,
        freshness_days=freshness_days,
        freshness_label=freshness_label,
        last_extraction_date=last_extraction_date,
        deterministic_coverage=deterministic_coverage,
        drifted_count=drifted_count,
        drift_info=drift_info,
        ai_signal_count=len(ai_signals),
        unapproved_count=len(unapproved_rules),
        score=score,
    )

    # Get the real project dependency count from manifests
    project_dep_count = _count_project_deps(project_dir)

    # Impact metrics — lightweight indicators of value over time
    metrics = _compute_impact_metrics(
        total_rules=total_rules,
        approved_rules=approved_rules,
        all_rules=all_rules,
        dep_names=dep_names,
        rule_files=rule_files,
        deterministic_coverage=deterministic_coverage,
        drifted_count=drifted_count,
        project_dep_count=project_dep_count,
    )

    # Next command
    if drifted_count > 0:
        next_command = "whetstone doctor"
    elif total_rules == 0:
        next_command = "whetstone doctor"
    elif freshness_days and freshness_days > 30:
        next_command = "whetstone doctor"
    else:
        next_command = "whetstone generate-tests && whetstone generate-context"

    return {
        "status": "ok",
        "label": label,
        "score": score,
        "freshness_label": freshness_label,
        "last_extraction_date": last_extraction_date,
        "dimensions": {
            "freshness_days": round(freshness_days, 1)
            if freshness_days is not None
            else None,
            "rules_count": total_rules,
            "high_confidence_ratio": round(high_confidence_ratio, 1),
            "deterministic_coverage": round(deterministic_coverage, 1),
            "pending_updates": drifted_count,
        },
        "breakdown": {
            "confidence": {"high": high_confidence, "medium": medium_confidence},
            "severity": {"must": must_count, "should": should_count, "may": may_count},
            "categories": categories,
            "signals": {
                "deterministic": len(deterministic_signals),
                "ai": len(ai_signals),
                "total": total_signals,
            },
        },
        "dependencies_covered": sorted(dep_names),
        "drift": drift_info,
        "metrics": metrics,
        "recommendations": recommendations,
        "warnings": load_warnings,
        "next_command": next_command,
    }


def _compute_impact_metrics(
    total_rules: int,
    approved_rules: list[dict],
    all_rules: list[dict],
    dep_names: set,
    rule_files: list[dict],
    deterministic_coverage: float,
    drifted_count: int,
    project_dep_count: int = 0,
) -> dict:
    """Compute lightweight impact metrics for value tracking.

    These metrics help teams quantify the value of maintaining rules over time.
    They are purely derived from current state — no persistent storage needed.

    Args:
        project_dep_count: Total runtime deps from manifest scanning.
            When > 0, used as the denominator for dependency coverage.
            When 0 (unknown), falls back to deps with rule files.
    """
    # Approval rate: approved / total proposed (including unapproved)
    total_proposed = len(all_rules)
    approval_rate = (total_rules / total_proposed * 100) if total_proposed > 0 else 0

    # Must-severity rules count — these are the highest-impact rules
    must_rules = sum(1 for r in approved_rules if r.get("severity") == "must")

    # Dependencies with at least one approved rule
    deps_with_rules = set()
    for rf in rule_files:
        if any(r.get("approved") for r in rf.get("rules", [])):
            deps_with_rules.add(rf.get("source_name", ""))

    # Coverage ratio: deps with rules / total project deps
    # Use actual manifest dep count as denominator when available,
    # falling back to deps-with-rule-files when detection fails.
    deps_covered = len(deps_with_rules)
    deps_total = project_dep_count if project_dep_count > 0 else len(dep_names)
    dep_coverage = (deps_covered / deps_total * 100) if deps_total > 0 else 0

    return {
        "rules_approved": total_rules,
        "rules_proposed": total_proposed,
        "approval_rate": round(approval_rate, 1),
        "must_rules": must_rules,
        "dependencies_covered": deps_covered,
        "dependencies_total": deps_total,
        "dependency_coverage": round(dep_coverage, 1),
        "deterministic_coverage": round(deterministic_coverage, 1),
        "pending_drift": drifted_count,
    }


def _compute_label(
    total_rules: int,
    freshness_days: float | None,
    deterministic_coverage: float,
    drifted_count: int,
) -> str:
    """Compute the human-readable status label."""
    if total_rules == 0:
        return "No Rules"

    # Stale: no extraction in 60+ days, or significant drift
    if (freshness_days and freshness_days > 60) or drifted_count >= 3:
        return "Stale"

    # Needs Review: some drift, or moderately old, or low deterministic coverage
    if drifted_count > 0:
        return "Needs Review"
    if freshness_days and freshness_days > 30:
        return "Needs Review"
    if deterministic_coverage < 50:
        return "Needs Review"

    return "Healthy"


def _compute_score(
    total_rules: int,
    freshness_days: float | None,
    deterministic_coverage: float,
    high_confidence_ratio: float,
    drifted_count: int,
    dep_count: int,
) -> int:
    """Compute a 0-100 health score. Secondary to the label and dimensions."""
    if total_rules == 0:
        return 0

    # Freshness component (0-30 points)
    # Full marks if <7 days, zero if >90 days
    if freshness_days is None:
        freshness_score = 15  # Unknown = middle
    elif freshness_days <= 7:
        freshness_score = 30
    elif freshness_days <= 30:
        freshness_score = 25
    elif freshness_days <= 60:
        freshness_score = 15
    elif freshness_days <= 90:
        freshness_score = 5
    else:
        freshness_score = 0

    # Deterministic coverage component (0-30 points)
    det_score = min(30, int(deterministic_coverage * 0.3))

    # Confidence component (0-20 points)
    conf_score = min(20, int(high_confidence_ratio * 0.2))

    # Drift component (0-20 points)
    # Full marks if no drift, decreasing with more drifted deps
    if drifted_count == 0:
        drift_score = 20
    elif drifted_count <= 2:
        drift_score = 10
    elif drifted_count <= 5:
        drift_score = 5
    else:
        drift_score = 0

    return min(100, freshness_score + det_score + conf_score + drift_score)


def _build_recommendations(
    total_rules: int,
    freshness_days: float | None,
    freshness_label: str,
    last_extraction_date: str | None,
    deterministic_coverage: float,
    drifted_count: int,
    drift_info: dict,
    ai_signal_count: int,
    unapproved_count: int = 0,
    score: int = 0,
) -> list[dict]:
    """Build actionable structured recommendations based on status."""
    recs: list[dict] = []

    if total_rules == 0:
        recs.append(
            {
                "priority": "high",
                "action": "init",
                "message": "No rules found. Run 'whetstone doctor' to get started.",
                "command": "whetstone doctor",
            }
        )
        return recs

    if drifted_count > 0:
        changed = drift_info.get("changed", [])
        dep_list = ", ".join(c.get("name", "?") for c in changed[:3])
        suffix = f" (+{drifted_count - 3} more)" if drifted_count > 3 else ""
        recs.append(
            {
                "priority": "high",
                "action": "refresh",
                "message": (
                    f"{drifted_count} deps have version drift: {dep_list}{suffix}. "
                    f"Re-run doctor to resolve updated sources, then re-extract."
                ),
                "command": "whetstone doctor",
            }
        )

    if freshness_days and freshness_days > 30:
        date_str = (
            f" (last: {last_extraction_date[:10]})" if last_extraction_date else ""
        )
        recs.append(
            {
                "priority": "high" if freshness_days > 60 else "medium",
                "action": "refresh",
                "message": (
                    f"Last extraction was {freshness_days:.0f} days ago{date_str}. "
                    f"Re-run doctor to check for documentation changes."
                ),
                "command": "whetstone doctor",
            }
        )

    if deterministic_coverage < 70 and total_rules > 0:
        recs.append(
            {
                "priority": "medium",
                "action": "review",
                "message": (
                    f"Deterministic signal coverage is {deterministic_coverage:.0f}%. "
                    f"Consider adding AST/pattern signals."
                ),
                "command": None,
            }
        )

    if ai_signal_count > 0 and deterministic_coverage < 50:
        recs.append(
            {
                "priority": "medium",
                "action": "decompose",
                "message": (
                    f"{ai_signal_count} signals use AI — "
                    f"consider decomposing into deterministic checks."
                ),
                "command": None,
            }
        )

    if unapproved_count > 0:
        recs.append(
            {
                "priority": "medium",
                "action": "approve",
                "message": (
                    f"{unapproved_count} proposed rules await approval. "
                    f"Run extraction to review."
                ),
                "command": None,
            }
        )

    if not recs:
        # Compute recommended next check
        if freshness_days is not None:
            days_until_check = max(1, int(30 - freshness_days))
            recs.append(
                {
                    "priority": "low",
                    "action": "none",
                    "message": f"Everything looks good. Next check recommended in {days_until_check} days.",
                    "command": None,
                }
            )
        else:
            recs.append(
                {
                    "priority": "low",
                    "action": "none",
                    "message": "Everything looks good. No action needed.",
                    "command": None,
                }
            )

    return recs


def _supports_color() -> bool:
    """Check if the terminal likely supports ANSI color codes."""
    if os.environ.get("NO_COLOR"):
        return False
    if os.environ.get("FORCE_COLOR"):
        return True
    return hasattr(sys.stderr, "isatty") and sys.stderr.isatty()


def format_human_output(result: dict) -> str:
    """Format status as human-readable text with box-drawing characters."""
    W = 62  # inner width between borders
    use_color = _supports_color()

    def _color(text: str, code: str) -> str:
        if not use_color:
            return text
        return f"\033[{code}m{text}\033[0m"

    def line(text: str = "") -> str:
        return f"\u2551  {text:<{W - 2}}\u2551"

    def section_header(title: str) -> str:
        padded = f"\u2550\u2550 {title} "
        return f"\u2560{padded}{'=' * (W - len(padded))}\u2563"

    lines = []

    if result.get("status") == "not_initialized":
        lines.append(f"\u2554{'=' * W}\u2557")
        lines.append(line("Whetstone Status"))
        lines.append(section_header(""))
        lines.append(line())
        lines.append(line(result["message"]))
        lines.append(line(f"Next: {result['next_command']}"))
        lines.append(line())
        lines.append(f"\u255a{'=' * W}\u255d")
        return "\n".join(lines)

    label = result.get("label", "Unknown")
    score = result.get("score", 0)
    dims = result.get("dimensions", {})
    freshness_label = result.get("freshness_label", "Unknown")

    # Status indicator
    label_indicator = {
        "Healthy": "OK",
        "Needs Review": "!!",
        "Stale": "XX",
        "No Rules": "--",
    }.get(label, "??")

    # Color the indicator
    indicator_colors = {
        "OK": "32",  # green
        "!!": "33",  # yellow
        "XX": "31",  # red
        "--": "90",  # gray
    }
    colored_indicator = _color(
        label_indicator, indicator_colors.get(label_indicator, "0")
    )

    # Top border
    lines.append(f"\u2554{'=' * W}\u2557")
    lines.append(line("Whetstone Status"))
    lines.append(section_header(""))
    lines.append(line())

    # Status bar
    rules_count = dims.get("rules_count", 0)
    drifted = dims.get("pending_updates", 0)
    deps = result.get("dependencies_covered", [])

    status_parts = [f"[{colored_indicator}] {label}"]
    status_parts.append(f"Score: {score}/100")
    status_parts.append(f"{rules_count} rules")
    if drifted > 0:
        status_parts.append(f"{drifted} deps drifted")
    status_line = " \u00b7 ".join(status_parts)
    # For box alignment, use the non-colored version for padding
    raw_status = f"[{label_indicator}] {label} \u00b7 Score: {score}/100 \u00b7 {rules_count} rules"
    if drifted > 0:
        raw_status += f" \u00b7 {drifted} deps drifted"

    if use_color:
        # Manually pad because ANSI codes mess up len()
        padding = W - 2 - len(raw_status)
        lines.append(f"\u2551  {status_line}{' ' * max(0, padding)}\u2551")
    else:
        lines.append(line(raw_status))

    lines.append(line())

    # Dimensions
    lines.append(section_header("Health Dimensions"))
    lines.append(line())

    freshness = dims.get("freshness_days")
    if freshness is not None:
        lines.append(
            line(f"Freshness:              {freshness:.0f} days ({freshness_label})")
        )
    else:
        lines.append(
            line(f"Freshness:              No timestamps found ({freshness_label})")
        )

    lines.append(line(f"Rules:                  {rules_count} approved"))

    # Dependency coverage
    if deps:
        # Try to figure out total deps from metrics
        metrics = result.get("metrics", {})
        total_deps = metrics.get("dependencies_total", len(deps))
        if total_deps > 0:
            coverage_pct = len(deps) / total_deps * 100
            lines.append(
                line(
                    f"Dep coverage:           {len(deps)} of {total_deps} deps have rules ({coverage_pct:.0f}%)"
                )
            )
        else:
            lines.append(line(f"Dependencies:           {', '.join(deps)}"))

    lines.append(
        line(f"High confidence:        {dims.get('high_confidence_ratio', 0):.0f}%")
    )
    lines.append(
        line(f"Deterministic coverage: {dims.get('deterministic_coverage', 0):.0f}%")
    )
    lines.append(line(f"Pending updates:        {drifted} deps with drift"))

    # Breakdown
    breakdown = result.get("breakdown", {})
    severity = breakdown.get("severity", {})
    if any(severity.values()):
        lines.append(line())
        lines.append(
            line(
                f"Severity: {severity.get('must', 0)} must, {severity.get('should', 0)} should, {severity.get('may', 0)} may"
            )
        )

    categories = breakdown.get("categories", {})
    if categories:
        cat_parts = [
            f"{v} {k}" for k, v in sorted(categories.items(), key=lambda x: -x[1])
        ]
        lines.append(line(f"Categories: {', '.join(cat_parts)}"))

    signals = breakdown.get("signals", {})
    if signals.get("total", 0) > 0:
        lines.append(
            line(
                f"Signals: {signals['deterministic']} deterministic, {signals['ai']} ai ({signals['total']} total)"
            )
        )

    # Recommendations
    recs = result.get("recommendations", [])
    if recs:
        lines.append(line())
        lines.append(section_header("Recommendations"))
        lines.append(line())
        for rec in recs:
            if isinstance(rec, dict):
                priority = rec.get("priority", "medium")
                msg = rec.get("message", "")
                cmd = rec.get("command")

                # Priority marker
                if use_color:
                    priority_markers = {
                        "high": _color("[HIGH]", "31"),
                        "medium": _color("[MED]", "33"),
                        "low": _color("[LOW]", "32"),
                    }
                else:
                    priority_markers = {
                        "high": "[HIGH]",
                        "medium": "[MED]",
                        "low": "[LOW]",
                    }
                marker = priority_markers.get(priority, f"[{priority.upper()}]")
                raw_marker = f"[{priority.upper()[:4]}]"

                # Wrap long messages
                prefix = f"{marker} "
                raw_prefix = f"{raw_marker} "
                full_msg = f"{raw_prefix}{msg}"
                if len(full_msg) > W - 4:
                    # First line with marker
                    lines.append(line(f"{prefix}{msg[: W - 4 - len(raw_prefix)]}"))
                    remaining = msg[W - 4 - len(raw_prefix) :]
                    while remaining:
                        chunk = remaining[: W - 8]
                        lines.append(line(f"       {chunk}"))
                        remaining = remaining[W - 8 :]
                else:
                    if use_color:
                        padding = W - 2 - len(full_msg)
                        lines.append(
                            f"\u2551  {prefix}{msg}{' ' * max(0, padding)}\u2551"
                        )
                    else:
                        lines.append(line(f"{prefix}{msg}"))

                if cmd:
                    lines.append(line(f"       -> {cmd}"))
            else:
                # Legacy string recommendation
                lines.append(line(f"  - {rec}"))
        lines.append(line())

    # Next command
    next_cmd = result.get("next_command")
    if next_cmd:
        lines.append(line(f"Next: {next_cmd}"))
        lines.append(line())

    # Bottom border
    lines.append(f"\u255a{'=' * W}\u255d")

    return "\n".join(lines)


def _snapshot_metrics(project_dir: Path, result: dict) -> None:
    """Append a metric snapshot to whetstone/.metrics.jsonl.

    Each line is a timestamped JSON object with the metrics and score.
    The file is append-only — old entries are never modified or deleted.
    """
    metrics = result.get("metrics")
    if not metrics:
        return

    snapshot = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "score": result.get("score", 0),
        "label": result.get("label", "Unknown"),
        **metrics,
    }

    metrics_file = project_dir / "whetstone" / ".metrics.jsonl"
    try:
        metrics_file.parent.mkdir(parents=True, exist_ok=True)
        with open(metrics_file, "a") as f:
            f.write(json.dumps(snapshot) + "\n")
    except OSError:
        pass  # Non-fatal — metrics are optional


def _load_metrics_history(project_dir: Path, limit: int = 20) -> list[dict]:
    """Load the most recent metric snapshots from .metrics.jsonl.

    Returns up to `limit` entries, most recent last.
    """
    metrics_file = project_dir / "whetstone" / ".metrics.jsonl"
    if not metrics_file.exists():
        return []

    entries: list[dict] = []
    try:
        with open(metrics_file) as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    entries.append(json.loads(line))
                except json.JSONDecodeError:
                    continue
    except OSError:
        return []

    return entries[-limit:]


def format_history(entries: list[dict]) -> str:
    """Format metric history as a human-readable trend table."""
    if not entries:
        return "No metric history found. Run 'whetstone status' to record snapshots."

    lines = []
    lines.append("")
    lines.append("=" * 72)
    lines.append("  Whetstone Metric History")
    lines.append("=" * 72)
    lines.append(
        f"  {'Date':<12s} {'Score':>5s}  {'Label':<14s} {'Rules':>5s} "
        f"{'Must':>4s} {'Det%':>5s} {'Drift':>5s}"
    )
    lines.append("  " + "-" * 64)

    for entry in entries:
        ts = entry.get("timestamp", "")[:10]  # YYYY-MM-DD
        score = entry.get("score", 0)
        label = entry.get("label", "?")
        rules = entry.get("rules_approved", 0)
        must = entry.get("must_rules", 0)
        det = entry.get("deterministic_coverage", 0)
        drift = entry.get("pending_drift", 0)
        lines.append(
            f"  {ts:<12s} {score:>5d}  {label:<14s} {rules:>5d} "
            f"{must:>4d} {det:>4.0f}% {drift:>5d}"
        )

    # Trend summary
    if len(entries) >= 2:
        first = entries[0]
        last = entries[-1]
        score_delta = last.get("score", 0) - first.get("score", 0)
        rules_delta = last.get("rules_approved", 0) - first.get("rules_approved", 0)
        direction = "+" if score_delta >= 0 else ""
        lines.append("  " + "-" * 64)
        lines.append(
            f"  Trend: score {direction}{score_delta}, "
            f"rules {'+' if rules_delta >= 0 else ''}{rules_delta} "
            f"over {len(entries)} snapshots"
        )

    lines.append("=" * 72)
    lines.append("")
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Whetstone Status — compact project health summary."
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
        help="Output only JSON",
    )
    parser.add_argument(
        "--score",
        action="store_true",
        help="Output only the numeric score and label",
    )
    parser.add_argument(
        "--no-drift-check",
        action="store_true",
        help="Skip dependency drift check (faster)",
    )
    parser.add_argument(
        "--changed-only",
        action="store_true",
        help="Only evaluate rules for dependencies with version drift",
    )
    parser.add_argument(
        "--history",
        action="store_true",
        help="Show metric trend history instead of current status",
    )
    parser.add_argument(
        "--no-snapshot",
        action="store_true",
        help="Skip recording a metric snapshot (default: record on every run)",
    )
    args = parser.parse_args()

    try:
        # History mode — show trends and exit
        if args.history:
            entries = _load_metrics_history(args.project_dir)
            if args.json_mode:
                json.dump({"history": entries}, sys.stdout, indent=2)
                sys.stdout.write("\n")
            else:
                print(format_history(entries), file=sys.stderr)
                json.dump({"history": entries}, sys.stdout, indent=2)
                sys.stdout.write("\n")
            return

        result = compute_status(
            project_dir=args.project_dir,
            check_dep_drift=not args.no_drift_check,
            changed_only=args.changed_only,
        )

        # Record metric snapshot (unless disabled or not initialized)
        if not args.no_snapshot and result.get("status") != "not_initialized":
            _snapshot_metrics(args.project_dir, result)

        if args.score:
            score = result.get("score", 0)
            label = result.get("label", "Unknown")
            print(f"{score} {label}")
            return

        if args.json_mode:
            json.dump(result, sys.stdout, indent=2)
            sys.stdout.write("\n")
        else:
            # Human-readable to stderr, JSON to stdout
            print(format_human_output(result), file=sys.stderr)
            json.dump(result, sys.stdout, indent=2)
            sys.stdout.write("\n")

    except Exception as e:
        json.dump(
            {
                "error": str(e),
                "next_command": "Check project directory and whetstone configuration",
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
