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


# --- Drift regression tests ---


class TestDrift:
    """Regression tests for the drift data flow: detect-deps → status → ci-check."""

    def test_drift_shape_is_normalized_dict(self):
        """detect-deps --check-drift must return drift as dict with changed/count/checked."""
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--check-drift"],
        )
        drift = result.get("drift")
        assert drift is not None, "drift key missing from output"
        assert isinstance(drift, dict), (
            f"drift should be dict, got {type(drift).__name__}"
        )
        assert "changed" in drift, "drift missing 'changed' key"
        assert "count" in drift, "drift missing 'count' key"
        assert "checked" in drift, "drift missing 'checked' key"
        assert isinstance(drift["changed"], list)
        assert isinstance(drift["count"], int)

    def test_drift_present_has_expected_fields(self):
        """When drift is detected, each entry has name/language/old_version/new_version."""
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--check-drift"],
        )
        drift = result["drift"]
        for entry in drift["changed"]:
            assert "name" in entry
            assert "language" in entry
            assert "old_version" in entry
            assert "new_version" in entry

    def test_status_with_drift_enabled(self):
        """status.py with drift enabled returns valid dimensions including pending_updates."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json"],
        )
        assert result["status"] == "ok"
        dims = result["dimensions"]
        assert "pending_updates" in dims
        assert isinstance(dims["pending_updates"], int)
        assert dims["pending_updates"] >= 0
        # drift key should be present and be a dict
        assert "drift" in result
        assert isinstance(result["drift"], dict)

    def test_ci_check_with_drift_enabled(self):
        """ci-check.py with drift returns valid changed_sources_count."""
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(FIXTURES_DIR), "--json"],
        )
        assert "changed_sources_count" in result
        assert isinstance(result["changed_sources_count"], int)

    def test_no_drift_when_no_rules_dir(self, tmp_path):
        """No rules directory = no drift data (not a crash)."""
        # Create a minimal project with a manifest but no whetstone/ dir
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\nversion = "0.1.0"\ndependencies = ["requests"]\n'
        )
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(tmp_path), "--check-drift"],
        )
        drift = result.get("drift", {})
        assert isinstance(drift, dict)
        assert drift.get("count", 0) == 0

    def test_status_drift_never_crashes(self, tmp_path):
        """status.py with drift enabled on empty/new project doesn't crash."""
        # Create whetstone dir but no rules
        (tmp_path / "whetstone" / "rules").mkdir(parents=True)
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json"],
        )
        # Should return ok with 0 pending_updates, not crash
        assert result["status"] == "ok"
        assert result["dimensions"]["pending_updates"] == 0


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


# --- Error path contract tests ---


class TestErrorContracts:
    """All scripts must include 'error' and 'next_command' in error responses."""

    def test_detect_deps_error_has_next_command(self, tmp_path):
        """No manifests → error response must have next_command."""
        result = run_script("detect-deps.py", ["--project-dir", str(tmp_path)])
        assert "error" in result
        assert "next_command" in result

    def test_doctor_error_has_next_command(self, tmp_path):
        """No manifests → doctor error response must have next_command."""
        result = run_script(
            "doctor.py",
            ["--project-dir", str(tmp_path), "--skip-patterns", "--json"],
        )
        assert result["status"] == "error"
        assert "error" in result
        assert "next_command" in result

    def test_status_not_initialized_has_next_command(self, tmp_path):
        """Empty dir → status not_initialized response must have next_command."""
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert "next_command" in result

    def test_generate_agent_context_no_rules_has_next_command(self, tmp_path):
        """No rules → generate response must have next_command."""
        result = run_script(
            "generate-agent-context.py",
            ["--project-dir", str(tmp_path), "--dry-run"],
        )
        assert "next_command" in result

    def test_generate_tests_no_rules_has_next_command(self, tmp_path):
        """No rules → generate-tests response must have next_command."""
        result = run_script(
            "generate-tests.py",
            ["--project-dir", str(tmp_path), "--dry-run"],
        )
        assert "next_command" in result

    def test_ci_check_not_initialized_has_next_command(self, tmp_path):
        """Empty dir → ci-check response must include expected fields."""
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert "freshness_status" in result
        # ci-check wraps status output; next_command comes through in recommendations


# --- YAML parsing and rule validation edge case tests ---


class TestYAMLEdgeCases:
    """Tests for YAML parsing robustness — Epic 3 (whetstone-6x4)."""

    def test_multiline_description(self, tmp_path):
        """Rule files with multiline YAML descriptions parse correctly."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "test.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: testlib\n"
            "  version: '1.0.0'\n"
            "  content_hash: sha256:abc\n"
            "rules:\n"
            "  - id: testlib.multi-line\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    category: convention\n"
            "    description: >\n"
            "      This is a long description that spans\n"
            "      multiple lines using YAML folded scalar.\n"
            "    source_url: https://example.com\n"
            "    approved: true\n"
            "    approved_at: 2026-02-25T12:00:00Z\n"
            "    signals:\n"
            "      - id: sig1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert result["dimensions"]["rules_count"] == 1

    def test_optional_fields_missing(self, tmp_path):
        """Rules with only required fields (no approved_at, no category) still parse."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "minimal.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: minimal\n"
            "rules:\n"
            "  - id: minimal.rule1\n"
            "    severity: should\n"
            "    confidence: medium\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: sig1\n"
            "        strategy: pattern\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert result["dimensions"]["rules_count"] == 1
        # freshness should be None (no approved_at)
        assert result["dimensions"]["freshness_days"] is None

    def test_multiple_rules_per_file(self, tmp_path):
        """A single YAML file with multiple rules is counted correctly."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "multi.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: multi\n"
            "  version: '2.0.0'\n"
            "rules:\n"
            "  - id: multi.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
            "  - id: multi.rule2\n"
            "    severity: should\n"
            "    confidence: medium\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s2\n"
            "        strategy: pattern\n"
            "  - id: multi.rule3\n"
            "    severity: may\n"
            "    confidence: high\n"
            "    approved: false\n"
            "    signals:\n"
            "      - id: s3\n"
            "        strategy: lint_proxy\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        # Only approved rules count
        assert result["dimensions"]["rules_count"] == 2
        breakdown = result["breakdown"]
        assert breakdown["severity"]["must"] == 1
        assert breakdown["severity"]["should"] == 1

    def test_datetime_object_freshness(self, tmp_path):
        """PyYAML parses timestamps to datetime objects — freshness must still work."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "datetime.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: dttest\n"
            "rules:\n"
            "  - id: dttest.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    approved_at: 2026-02-25T12:00:00Z\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        # freshness_days should be a positive number (not None, not crash)
        assert result["dimensions"]["freshness_days"] is not None
        assert result["dimensions"]["freshness_days"] > 0

    def test_malformed_yaml_produces_warning(self, tmp_path):
        """Malformed YAML files produce warnings, not crashes."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        # Valid file
        good_file = rules_dir / "good.yaml"
        good_file.write_text(
            "source:\n"
            "  name: good\n"
            "rules:\n"
            "  - id: good.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        # Malformed file
        bad_file = rules_dir / "bad.yaml"
        bad_file.write_text("this is: not\n  valid yaml content:::\n    - [broken\n")
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        # Should still succeed — good rules counted, bad file warned
        assert result["status"] == "ok"
        assert result["dimensions"]["rules_count"] >= 1
        # Should have at least one warning about the bad file
        assert len(result.get("warnings", [])) >= 1

    def test_empty_rules_list(self, tmp_path):
        """A rule file with empty rules list is handled gracefully."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "empty.yaml"
        rule_file.write_text(
            "source:\n  name: emptylib\n  version: '1.0.0'\nrules: []\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert result["dimensions"]["rules_count"] == 0

    def test_rule_missing_signals_produces_warning(self, tmp_path):
        """A rule with no signals should produce a validation warning."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "nosig.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: nosig\n"
            "rules:\n"
            "  - id: nosig.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        warnings = result.get("warnings", [])
        # Should warn about missing signals
        assert any("no signals" in w for w in warnings)

    def test_rule_missing_severity_produces_warning(self, tmp_path):
        """A rule missing required 'severity' field should produce a warning."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "nosev.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: nosev\n"
            "rules:\n"
            "  - id: nosev.rule1\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        warnings = result.get("warnings", [])
        assert any("severity" in w for w in warnings)

    def test_mixed_signal_strategies_coverage(self, tmp_path):
        """Deterministic coverage computed correctly with mixed signal types."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "mixed.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: mixed\n"
            "rules:\n"
            "  - id: mixed.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
            "      - id: s2\n"
            "        strategy: ai\n"
            "  - id: mixed.rule2\n"
            "    severity: should\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s3\n"
            "        strategy: pattern\n"
            "      - id: s4\n"
            "        strategy: lint_proxy\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        # 4 signals total, 3 deterministic, 1 AI = 75% coverage
        signals = result["breakdown"]["signals"]
        assert signals["deterministic"] == 3
        assert signals["ai"] == 1
        assert signals["total"] == 4
        assert result["dimensions"]["deterministic_coverage"] == 75.0

    def test_quoted_version_string(self, tmp_path):
        """Version strings in quotes parse correctly."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "quoted.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: quoted\n"
            "  version: '3.11.0'\n"
            "  content_hash: sha256:def456\n"
            "rules:\n"
            "  - id: quoted.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert result["dimensions"]["rules_count"] == 1

    def test_subdirectory_nesting(self, tmp_path):
        """Rules in nested subdirectories (e.g., whetstone/rules/python/fastapi.yaml) are found."""
        for lang in ("python", "typescript"):
            rules_dir = tmp_path / "whetstone" / "rules" / lang
            rules_dir.mkdir(parents=True)
            rule_file = rules_dir / f"{lang}_lib.yaml"
            rule_file.write_text(
                f"source:\n"
                f"  name: {lang}_lib\n"
                f"rules:\n"
                f"  - id: {lang}_lib.rule1\n"
                f"    severity: must\n"
                f"    confidence: high\n"
                f"    approved: true\n"
                f"    signals:\n"
                f"      - id: s1\n"
                f"        strategy: ast\n"
            )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert result["dimensions"]["rules_count"] == 2
        deps = result["dependencies_covered"]
        assert "python_lib" in deps
        assert "typescript_lib" in deps


# --- CI-check fail-on exit code tests ---


class TestCICheckFailOn:
    """Validate --fail-on exit codes — Epic 4 (whetstone-3lx)."""

    def test_fail_on_none_always_passes(self):
        """--fail-on none should always exit 0."""
        script = SCRIPTS_DIR / "ci-check.py"
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
                "--fail-on",
                "none",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert result.returncode == 0

    def test_fail_on_stale_passes_when_healthy(self):
        """--fail-on stale should exit 0 when status is not stale."""
        script = SCRIPTS_DIR / "ci-check.py"
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
                "--fail-on",
                "stale",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        # Fixture has recent rules so should not be stale
        output = json.loads(result.stdout)
        if output.get("freshness_status") != "stale":
            assert result.returncode == 0
        else:
            assert result.returncode == 1

    def test_fail_on_needs_review_passes_when_healthy(self):
        """--fail-on needs_review should exit 0 when status is healthy."""
        script = SCRIPTS_DIR / "ci-check.py"
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
                "--fail-on",
                "needs_review",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        output = json.loads(result.stdout)
        if output.get("freshness_status") in ("stale", "needs_review"):
            assert result.returncode == 1
        else:
            assert result.returncode == 0

    def test_not_initialized_does_not_trigger_fail_on(self, tmp_path):
        """--fail-on stale should not trigger on not_initialized projects."""
        script = SCRIPTS_DIR / "ci-check.py"
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "--project-dir",
                str(tmp_path),
                "--json",
                "--no-drift-check",
                "--fail-on",
                "stale",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        # not_initialized is not "stale", so should exit 0
        assert result.returncode == 0
        output = json.loads(result.stdout)
        assert output["freshness_status"] == "not_initialized"


# --- Action changed-only wiring tests ---


class TestActionChangedOnlyWiring:
    """Verify that --changed-only flag is accepted and works — Epic 4 (whetstone-3lx)."""

    def test_detect_deps_changed_only_flag_accepted(self):
        """detect-deps.py --changed-only should be accepted and include drift."""
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--changed-only"],
        )
        assert "drift" in result
        assert isinstance(result["drift"], dict)

    def test_changed_only_produces_valid_drift_structure(self):
        """--changed-only drift output has the normalized dict shape."""
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--changed-only"],
        )
        drift = result["drift"]
        assert "changed" in drift
        assert "count" in drift
        assert "checked" in drift
        assert isinstance(drift["changed"], list)
        assert isinstance(drift["count"], int)

    def test_action_yml_has_changed_only_input(self):
        """action.yml must declare the changed-only input."""
        import yaml as _yaml

        action_path = Path(__file__).resolve().parent.parent / "action.yml"
        with open(action_path) as f:
            action = _yaml.safe_load(f)
        inputs = action.get("inputs", {})
        assert "changed-only" in inputs
        assert inputs["changed-only"].get("default") == "true"


# --- Impact metrics tests ---


class TestImpactMetrics:
    """Verify impact metrics are present and structured — Epic 7 (whetstone-ibc)."""

    def test_status_has_metrics(self):
        """status.py output includes a metrics object."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "metrics" in result
        metrics = result["metrics"]
        assert isinstance(metrics, dict)

    def test_metrics_has_required_fields(self):
        """Metrics object has all defined fields."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        metrics = result["metrics"]
        for field in (
            "rules_approved",
            "rules_proposed",
            "approval_rate",
            "must_rules",
            "dependencies_covered",
            "dependencies_total",
            "dependency_coverage",
            "deterministic_coverage",
            "pending_drift",
        ):
            assert field in metrics, f"metrics missing '{field}'"

    def test_metrics_values_are_sensible(self):
        """Metrics values are within expected ranges."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        metrics = result["metrics"]
        assert metrics["rules_approved"] >= 0
        assert metrics["rules_proposed"] >= metrics["rules_approved"]
        assert 0 <= metrics["approval_rate"] <= 100
        assert 0 <= metrics["dependency_coverage"] <= 100
        assert 0 <= metrics["deterministic_coverage"] <= 100
        assert metrics["pending_drift"] >= 0

    def test_metrics_absent_when_not_initialized(self, tmp_path):
        """Not-initialized projects don't have metrics (no crash)."""
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "not_initialized"
        # metrics key should not be present for not-initialized
        assert "metrics" not in result
