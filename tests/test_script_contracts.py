"""Output contract tests for Whetstone scripts.

Verifies that scripts produce consistent JSON output structures.
These tests are NOT snapshot tests — they verify structural contracts
(required keys, types, non-empty fields) rather than exact values.

Run with: pytest tests/test_script_contracts.py -v
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
FIXTURES_DIR = Path(__file__).resolve().parent / "fixtures"


def run_script(name: str, args: list[str], stdin_data: str | None = None) -> dict:
    """Run a script and return parsed JSON output."""
    script = SCRIPTS_DIR / name
    cmd = [sys.executable, str(script)] + args
    result = subprocess.run(
        cmd,
        input=stdin_data,
        capture_output=True,
        text=True,
        timeout=60,
    )
    # Parse JSON from stdout (ignore stderr which may have progress messages)
    return json.loads(result.stdout)


# --- detect-deps.py ---


class TestDetectDeps:
    def test_output_has_required_keys(self):
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        assert "languages" in result
        assert "dependencies" in result
        assert "manifests" in result
        assert "counts" in result
        assert "next_command" in result

    def test_languages_is_list(self):
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        assert isinstance(result["languages"], list)
        assert "python" in result["languages"]
        assert "typescript" in result["languages"]

    def test_dependencies_have_required_fields(self):
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        for dep in result["dependencies"]:
            assert "name" in dep
            assert "version" in dep
            assert "language" in dep
            assert "dev" in dep

    def test_counts_structure(self):
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        counts = result["counts"]
        assert "total" in counts
        assert "runtime" in counts
        assert "dev" in counts
        assert "_all" in counts["total"]

    def test_check_drift_adds_drift_key(self):
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--check-drift"],
        )
        assert "drift" in result

    def test_changed_only_adds_drift_key(self):
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--changed-only"],
        )
        assert "drift" in result

    def test_no_manifests_returns_error(self, tmp_path):
        result = run_script("detect-deps.py", ["--project-dir", str(tmp_path)])
        assert "error" in result


# --- status.py ---


class TestStatus:
    def test_not_initialized_output(self, tmp_path):
        result = run_script(
            "status.py", ["--project-dir", str(tmp_path), "--json", "--no-drift-check"]
        )
        assert result["status"] == "not_initialized"
        assert "label" in result
        assert "next_command" in result

    def test_initialized_output_has_dimensions(self):
        # FIXTURES_DIR has whetstone/rules/ for status detection
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert "dimensions" in result
        dims = result["dimensions"]
        assert "freshness_days" in dims
        assert "rules_count" in dims
        assert "high_confidence_ratio" in dims
        assert "deterministic_coverage" in dims
        assert "pending_updates" in dims

    def test_has_breakdown(self):
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "breakdown" in result
        breakdown = result["breakdown"]
        assert "confidence" in breakdown
        assert "severity" in breakdown
        assert "categories" in breakdown
        assert "signals" in breakdown

    def test_has_recommendations(self):
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "recommendations" in result
        assert isinstance(result["recommendations"], list)

    def test_has_score(self):
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "score" in result
        assert isinstance(result["score"], int)
        assert 0 <= result["score"] <= 100


# --- ci-check.py ---


class TestCICheck:
    def test_output_has_ci_fields(self):
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "freshness_status" in result
        assert "changed_sources_count" in result
        assert "recommended_rules_count" in result
        assert "requires_review" in result
        assert "score" in result

    def test_freshness_status_is_valid(self):
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        valid_statuses = {
            "healthy",
            "needs_review",
            "stale",
            "no_rules",
            "not_initialized",
            "error",
            "unknown",
        }
        assert result["freshness_status"] in valid_statuses

    def test_not_initialized(self, tmp_path):
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["freshness_status"] == "not_initialized"


# --- doctor.py ---


class TestDoctor:
    def test_output_has_required_keys(self):
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--skip-patterns", "--json"],
        )
        assert "status" in result
        assert "steps" in result
        assert "summary" in result

    def test_summary_has_required_fields(self):
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--skip-patterns", "--json"],
        )
        summary = result["summary"]
        assert "dependencies_found" in summary
        assert "sources_resolved" in summary
        assert "languages" in summary

    def test_steps_have_names(self):
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--skip-patterns", "--json"],
        )
        for step in result["steps"]:
            assert "name" in step
            assert "status" in step

    def test_no_manifests_returns_error(self, tmp_path):
        result = run_script(
            "doctor.py",
            ["--project-dir", str(tmp_path), "--skip-patterns", "--json"],
        )
        assert result["status"] == "error"


# --- generate-agent-context.py ---


class TestGenerateAgentContext:
    def test_output_has_required_keys(self):
        result = run_script(
            "generate-agent-context.py",
            ["--project-dir", str(FIXTURES_DIR), "--dry-run"],
        )
        assert "generated" in result
        assert "rules_count" in result
        assert "dependencies" in result
        assert "next_command" in result

    def test_dry_run_does_not_write(self, tmp_path):
        # Copy fixtures to tmp
        import shutil

        shutil.copytree(FIXTURES_DIR / "rules", tmp_path / "whetstone" / "rules")
        result = run_script(
            "generate-agent-context.py",
            ["--project-dir", str(tmp_path), "--dry-run"],
        )
        # Should not create actual files in dry-run
        assert result["rules_count"] >= 0


# --- generate-tests.py ---


class TestGenerateTests:
    def test_output_has_required_keys(self):
        result = run_script(
            "generate-tests.py",
            ["--project-dir", str(FIXTURES_DIR), "--dry-run"],
        )
        assert "generated" in result
        assert "rules_processed" in result
        assert "next_command" in result

    def test_generated_has_tests_and_lints(self):
        result = run_script(
            "generate-tests.py",
            ["--project-dir", str(FIXTURES_DIR), "--dry-run"],
        )
        generated = result["generated"]
        assert "tests" in generated
        assert "lint_configs" in generated
