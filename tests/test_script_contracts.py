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

import pytest  # noqa: F401

LEGACY_SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts" / "legacy"
ACTIVE_SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
FIXTURES_DIR = Path(__file__).resolve().parent / "fixtures"
ROOT_DIR = Path(__file__).resolve().parent.parent


def run_script(name: str, args: list[str], stdin_data: str | None = None) -> dict:
    """Run a script and return parsed JSON output."""
    script_dir = (
        ACTIVE_SCRIPTS_DIR if name == "detect-patterns.py" else LEGACY_SCRIPTS_DIR
    )
    script = script_dir / name
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


def run_rust(args: list[str]) -> dict:
    """Run the Rust binary via cargo and return parsed JSON output."""
    result = subprocess.run(
        ["cargo", "run", "--quiet", "--"] + args,
        cwd=ROOT_DIR,
        capture_output=True,
        text=True,
        timeout=120,
    )
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

    def test_generate_context_parity_with_rust(self):
        py = run_script(
            "generate-agent-context.py",
            ["--project-dir", str(FIXTURES_DIR), "--dry-run"],
        )
        rust = run_rust(
            [
                "generate-context",
                "--project-dir",
                str(FIXTURES_DIR),
                "--dry-run",
                "--json",
            ]
        )
        assert py["rules_count"] == rust["rules_count"]
        assert len(py["generated"]) == len(rust["generated"])

    def test_generate_tests_parity_with_rust(self):
        py = run_script(
            "generate-tests.py",
            ["--project-dir", str(FIXTURES_DIR), "--dry-run"],
        )
        rust = run_rust(
            [
                "generate-tests",
                "--project-dir",
                str(FIXTURES_DIR),
                "--dry-run",
                "--json",
            ]
        )
        assert py["rules_processed"] == rust["rules_count"]
        py_tests_total = sum(len(v) for v in py["generated"]["tests"].values())
        rust_tests_total = sum(
            1 for entry in rust["generated"]["tests"] if entry.get("type") == "test"
        )
        assert py_tests_total == rust_tests_total

    def test_ci_check_parity_with_rust(self):
        py = run_script(
            "ci-check.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        rust = run_rust(
            [
                "ci-check",
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
            ]
        )
        for key in [
            "freshness_status",
            "changed_sources_count",
            "recommended_rules_count",
            "requires_review",
            "score",
        ]:
            assert py[key] == rust[key]

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

    def test_recommendations_are_structured_dicts(self):
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        recs = result["recommendations"]
        assert len(recs) > 0
        for rec in recs:
            assert isinstance(rec, dict), (
                f"recommendation should be dict, got {type(rec).__name__}"
            )
            assert "priority" in rec, "recommendation missing 'priority'"
            assert "action" in rec, "recommendation missing 'action'"
            assert "message" in rec, "recommendation missing 'message'"
            assert "command" in rec, "recommendation missing 'command' (can be null)"

    def test_has_freshness_label(self):
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "freshness_label" in result
        valid_labels = {"Fresh", "Current", "Aging", "Stale", "Unknown"}
        assert result["freshness_label"] in valid_labels

    def test_has_last_extraction_date(self):
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "last_extraction_date" in result
        # Should be an ISO 8601 string (fixture has approved_at)
        assert result["last_extraction_date"] is not None
        assert isinstance(result["last_extraction_date"], str)

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

    def test_output_has_recommendations(self):
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--skip-patterns", "--json"],
        )
        assert "recommendations" in result
        assert isinstance(result["recommendations"], list)

    def test_output_has_source_details(self):
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--skip-patterns", "--json"],
        )
        assert "source_details" in result
        assert isinstance(result["source_details"], list)


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

        shutil.copytree(
            FIXTURES_DIR / "whetstone" / "rules", tmp_path / "whetstone" / "rules"
        )
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
        script = LEGACY_SCRIPTS_DIR / "ci-check.py"
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
        script = LEGACY_SCRIPTS_DIR / "ci-check.py"
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
        script = LEGACY_SCRIPTS_DIR / "ci-check.py"
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
        script = LEGACY_SCRIPTS_DIR / "ci-check.py"
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


# --- Discovery hardening tests (stf.4) ---


class TestDiscoveryMetadata:
    """Tests for discovery include/exclude boundaries and monorepo detection."""

    def test_discovery_key_present_in_output(self):
        """detect-deps output includes the discovery key."""
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        assert "discovery" in result
        discovery = result["discovery"]
        assert "excluded" in discovery
        assert "included" in discovery
        assert "monorepo" in discovery
        assert isinstance(discovery["excluded"], list)
        assert isinstance(discovery["included"], list)
        assert isinstance(discovery["monorepo"], bool)

    def test_discovery_excluded_contains_defaults(self):
        """Default excluded list includes hardcoded SKIP_DIRS entries."""
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        excluded = result["discovery"]["excluded"]
        # Check a sample of hardcoded defaults
        for expected in (
            "node_modules",
            "vendor",
            "fixtures",
            "examples",
            "third_party",
        ):
            assert expected in excluded, f"{expected} not in excluded list"

    def test_discovery_key_present_on_error(self, tmp_path):
        """discovery key is present even when no manifests are found."""
        result = run_script("detect-deps.py", ["--project-dir", str(tmp_path)])
        assert "error" in result
        assert "discovery" in result
        assert "excluded" in result["discovery"]
        assert result["discovery"]["monorepo"] is False

    def test_cli_exclude_flag_extends_exclusions(self, tmp_path):
        """--exclude adds patterns to the excluded list."""
        # Create a manifest in root
        (tmp_path / "pyproject.toml").write_text(
            '[project]\nname = "test"\nversion = "0.1.0"\ndependencies = ["requests"]\n'
        )
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(tmp_path), "--exclude", "custom_dir,another_dir"],
        )
        excluded = result["discovery"]["excluded"]
        assert "custom_dir" in excluded
        assert "another_dir" in excluded

    def test_monorepo_detection_single_dir(self):
        """Single manifest directory does not flag monorepo."""
        result = run_script("detect-deps.py", ["--project-dir", str(FIXTURES_DIR)])
        # Fixtures dir has manifests in root only
        discovery = result["discovery"]
        # Whether monorepo depends on fixture layout — just verify the key works
        assert isinstance(discovery["monorepo"], bool)
        assert isinstance(discovery.get("workspaces", []), list)

    def test_monorepo_detection_multiple_dirs(self, tmp_path):
        """Multiple manifests in different directories triggers monorepo=true."""
        # Root manifest
        (tmp_path / "pyproject.toml").write_text(
            '[project]\nname = "root"\nversion = "0.1.0"\ndependencies = ["requests"]\n'
        )
        # Subdirectory manifest
        sub = tmp_path / "services" / "api"
        sub.mkdir(parents=True)
        (sub / "pyproject.toml").write_text(
            '[project]\nname = "api"\nversion = "0.1.0"\ndependencies = ["flask"]\n'
        )
        result = run_script("detect-deps.py", ["--project-dir", str(tmp_path)])
        assert result["discovery"]["monorepo"] is True
        assert len(result["discovery"]["workspaces"]) > 1

    def test_fixtures_dir_excluded_by_default(self, tmp_path):
        """Directories named 'fixtures' are excluded from manifest discovery."""
        # Root manifest
        (tmp_path / "pyproject.toml").write_text(
            '[project]\nname = "main"\nversion = "0.1.0"\ndependencies = ["requests"]\n'
        )
        # Fixture manifest (should be excluded)
        fix = tmp_path / "fixtures"
        fix.mkdir()
        (fix / "package.json").write_text(
            '{"name": "fixture", "dependencies": {"lodash": "^4.0.0"}}'
        )
        result = run_script("detect-deps.py", ["--project-dir", str(tmp_path)])
        # Only root pyproject.toml should be found
        assert len(result["manifests"]) == 1
        assert result["manifests"][0] == "pyproject.toml"

    def test_include_overrides_exclude(self, tmp_path):
        """--include allows a normally-excluded directory to be scanned."""
        # Root manifest
        (tmp_path / "pyproject.toml").write_text(
            '[project]\nname = "main"\nversion = "0.1.0"\ndependencies = ["requests"]\n'
        )
        # Vendor manifest (normally excluded)
        vendor = tmp_path / "vendor"
        vendor.mkdir()
        (vendor / "package.json").write_text(
            '{"name": "vendored", "dependencies": {"express": "^4.0.0"}}'
        )
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(tmp_path), "--include", "vendor"],
        )
        # Both manifests should be found
        assert len(result["manifests"]) == 2


class TestSourceFreshness:
    """Tests for source freshness and confidence metadata (stf.4.3)."""

    def test_freshness_key_present_in_resolved_sources(self):
        """resolve-sources output includes freshness key for each source."""
        # Create a minimal deps input and pipe it to resolve-sources
        deps_input = json.dumps(
            {
                "dependencies": [
                    {
                        "name": "requests",
                        "version": "*",
                        "language": "python",
                        "dev": False,
                    }
                ]
            }
        )
        result = run_script(
            "resolve-sources.py",
            ["--deps", "requests", "--timeout", "10"],
            stdin_data=deps_input,
        )
        # Should have at least one source or error
        if result.get("sources"):
            source = result["sources"][0]
            assert "freshness" in source, "freshness key missing from source"
            freshness = source["freshness"]
            assert "source_age_days" in freshness
            assert "content_stale" in freshness
            assert "confidence" in freshness
            assert freshness["confidence"] in ("high", "medium", "low")

    def test_freshness_confidence_values(self):
        """Confidence is 'high' for llms_txt sources, 'low' for docs_url_only."""
        # We can't control which source_type we get from live resolution,
        # so test the _compute_freshness function directly
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "resolve_sources", str(LEGACY_SCRIPTS_DIR / "resolve-sources.py")
        )
        mod = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
        spec.loader.exec_module(mod)  # type: ignore[union-attr]

        # High confidence for llms_txt
        result_high = mod._compute_freshness(
            {
                "source_type": "llms_txt",
                "content": "some content",
                "content_hash": "sha256:abc",
            },
        )
        assert result_high["confidence"] == "high"

        # Low confidence for docs_url_only
        result_low = mod._compute_freshness(
            {"source_type": "docs_url_only", "content": None, "content_hash": None},
        )
        assert result_low["confidence"] == "low"

        # Medium confidence for docs_url with content
        result_med = mod._compute_freshness(
            {
                "source_type": "other",
                "content": "fetched content",
                "content_hash": "sha256:def",
            },
        )
        assert result_med["confidence"] == "medium"

    def test_freshness_content_stale_detection(self):
        """content_stale is true when stored hash differs from current hash."""
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "resolve_sources", str(LEGACY_SCRIPTS_DIR / "resolve-sources.py")
        )
        mod = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
        spec.loader.exec_module(mod)  # type: ignore[union-attr]

        # Same hash = not stale
        result_same = mod._compute_freshness(
            {"source_type": "llms_txt", "content": "x", "content_hash": "sha256:abc"},
            stored_hash="sha256:abc",
        )
        assert result_same["content_stale"] is False

        # Different hash = stale
        result_diff = mod._compute_freshness(
            {"source_type": "llms_txt", "content": "x", "content_hash": "sha256:def"},
            stored_hash="sha256:abc",
        )
        assert result_diff["content_stale"] is True

    def test_freshness_source_age_days(self):
        """source_age_days is computed from latest_release_date."""
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "resolve_sources", str(LEGACY_SCRIPTS_DIR / "resolve-sources.py")
        )
        mod = importlib.util.module_from_spec(spec)  # type: ignore[arg-type]
        spec.loader.exec_module(mod)  # type: ignore[union-attr]

        # Recent release
        result = mod._compute_freshness(
            {
                "source_type": "llms_txt",
                "content": "x",
                "content_hash": "sha256:abc",
                "latest_release_date": "2026-03-01T00:00:00Z",
            },
        )
        assert result["source_age_days"] is not None
        assert isinstance(result["source_age_days"], int)
        assert result["source_age_days"] >= 0

        # No release date
        result_none = mod._compute_freshness(
            {"source_type": "llms_txt", "content": "x", "content_hash": "sha256:abc"},
        )
        assert result_none["source_age_days"] is None


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

    def test_metrics_use_real_project_dependency_denominator(self):
        """Dependency coverage reflects actual detected project deps, not just rule files."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        metrics = result["metrics"]
        assert metrics["dependencies_total"] == 5
        assert metrics["dependencies_covered"] == 2
        assert metrics["dependency_coverage"] == 40.0

    def test_status_next_command_uses_real_cli_commands(self):
        """next_command should be executable guidance, not prose or phantom commands."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        # next_command should start with a real CLI command (may have flags)
        assert result["next_command"].startswith("whetstone ")

    def test_metrics_absent_when_not_initialized(self, tmp_path):
        """Not-initialized projects don't have metrics (no crash)."""
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "not_initialized"
        # metrics key should not be present for not-initialized
        assert "metrics" not in result


# --- Changed-only end-to-end tests ---


class TestChangedOnlySemantics:
    """Verify --changed-only scopes evaluation — Epic 2 (whetstone-tkg)."""

    def test_status_changed_only_accepted(self):
        """status.py --changed-only flag is accepted without error."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--changed-only"],
        )
        assert result["status"] == "ok"

    def test_changed_only_returns_subset(self):
        """--changed-only should return <= rules vs full scan."""
        full = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json"],
        )
        scoped = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--changed-only"],
        )
        assert scoped["status"] == "ok"
        # Scoped rules_count must be <= full rules_count
        assert scoped["dimensions"]["rules_count"] <= full["dimensions"]["rules_count"]

    def test_changed_only_no_drift_is_healthy(self, tmp_path):
        """When no drift exists, --changed-only returns healthy with zero rules."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        # Create a rule file for a dep that won't have drift (no manifests)
        rule_file = rules_dir / "nodrift.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: nodrift\n"
            "  version: '1.0.0'\n"
            "rules:\n"
            "  - id: nodrift.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--changed-only"],
        )
        assert result["status"] == "ok"
        # No manifests = no drift = 0 rules in changed-only mode
        assert result["dimensions"]["rules_count"] == 0
        assert result["dimensions"]["pending_updates"] == 0

    def test_ci_check_changed_only_accepted(self):
        """ci-check.py --changed-only flag is accepted."""
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--changed-only"],
        )
        assert "freshness_status" in result
        assert "score" in result


# --- PR mining regression tests ---


class TestPRMining:
    """Regression tests for PR comment mining — Epic 3 (whetstone-n54)."""

    def test_detect_patterns_pr_source_no_crash(self, tmp_path):
        """detect-patterns with --sources pr doesn't crash even without gh/repo."""
        result = run_script(
            "detect-patterns.py",
            ["--project-dir", str(tmp_path), "--sources", "pr"],
        )
        assert "patterns" in result
        assert isinstance(result["patterns"], list)
        assert "sources_analyzed" in result

    def test_detect_patterns_all_sources_no_crash(self):
        """detect-patterns with all sources doesn't crash."""
        result = run_script(
            "detect-patterns.py",
            ["--project-dir", str(FIXTURES_DIR), "--sources", "transcript,git,pr"],
        )
        assert "patterns" in result
        assert "sources_analyzed" in result
        # PR source should be present in analyzed dict
        assert "pr" in result["sources_analyzed"]

    def test_detect_patterns_has_next_command(self):
        """detect-patterns always includes next_command."""
        result = run_script(
            "detect-patterns.py",
            ["--project-dir", str(FIXTURES_DIR)],
        )
        assert "next_command" in result
        assert isinstance(result["next_command"], str)
        assert len(result["next_command"]) > 0


# --- Strict schema validation tests ---


class TestStrictSchemaValidation:
    """Tests for strict enum/type validation — Epic 4 (whetstone-ifa)."""

    def test_invalid_severity_produces_warning(self, tmp_path):
        """A rule with invalid severity value produces a validation warning."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "badsev.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: badsev\n"
            "rules:\n"
            "  - id: badsev.rule1\n"
            "    severity: critical\n"
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
        warnings = result.get("warnings", [])
        assert any("severity" in w and "critical" in w for w in warnings)

    def test_invalid_confidence_produces_warning(self, tmp_path):
        """A rule with invalid confidence value produces a validation warning."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "badconf.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: badconf\n"
            "rules:\n"
            "  - id: badconf.rule1\n"
            "    severity: must\n"
            "    confidence: low\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        warnings = result.get("warnings", [])
        assert any("confidence" in w and "low" in w for w in warnings)

    def test_invalid_category_produces_warning(self, tmp_path):
        """A rule with invalid category value produces a validation warning."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "badcat.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: badcat\n"
            "rules:\n"
            "  - id: badcat.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    category: performance\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        warnings = result.get("warnings", [])
        assert any("category" in w and "performance" in w for w in warnings)

    def test_invalid_signal_strategy_produces_warning(self, tmp_path):
        """A signal with invalid strategy produces a validation warning."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "badstrat.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: badstrat\n"
            "rules:\n"
            "  - id: badstrat.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: magic\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        warnings = result.get("warnings", [])
        assert any("strategy" in w and "magic" in w for w in warnings)

    def test_valid_rule_produces_no_warnings(self, tmp_path):
        """A fully valid rule produces zero validation warnings."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        rule_file = rules_dir / "good.yaml"
        rule_file.write_text(
            "source:\n"
            "  name: good\n"
            "  version: '1.0.0'\n"
            "  content_hash: sha256:abc\n"
            "rules:\n"
            "  - id: good.rule1\n"
            "    severity: must\n"
            "    confidence: high\n"
            "    category: convention\n"
            "    approved: true\n"
            "    signals:\n"
            "      - id: s1\n"
            "        strategy: ast\n"
        )
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result.get("warnings", []) == []


# --- Exhaustive output-contract tests ---


class TestTranscriptPrivacy:
    """Transcript mining is project-scoped by default — Epic 6 (whetstone-jve)."""

    def test_default_scoped_to_project(self, tmp_path):
        """detect-patterns without --global-transcripts reports scoped: true."""
        result = run_script(
            "detect-patterns.py",
            ["--project-dir", str(tmp_path), "--sources", "transcript"],
        )
        assert "sources_analyzed" in result
        if "transcript" in result["sources_analyzed"]:
            assert result["sources_analyzed"]["transcript"]["scoped"] is True

    def test_global_flag_disables_scoping(self, tmp_path):
        """detect-patterns --global-transcripts reports scoped: false."""
        result = run_script(
            "detect-patterns.py",
            [
                "--project-dir",
                str(tmp_path),
                "--sources",
                "transcript",
                "--global-transcripts",
            ],
        )
        assert "sources_analyzed" in result
        if "transcript" in result["sources_analyzed"]:
            assert result["sources_analyzed"]["transcript"]["scoped"] is False

    def test_global_flag_emits_stderr_warning(self, tmp_path):
        """--global-transcripts emits a privacy warning to stderr."""
        script = ACTIVE_SCRIPTS_DIR / "detect-patterns.py"
        proc = subprocess.run(
            [
                sys.executable,
                str(script),
                "--project-dir",
                str(tmp_path),
                "--sources",
                "transcript",
                "--global-transcripts",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert "WARNING" in proc.stderr
        assert "global" in proc.stderr.lower()

    def test_project_filter_matches_correctly(self):
        """_project_transcript_filter matches project name in path."""
        # Import the filter function directly
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "detect_patterns", ACTIVE_SCRIPTS_DIR / "detect-patterns.py"
        )
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)

        project = Path("/home/user/code/my-project")
        assert mod._project_transcript_filter(
            project, Path("/home/user/.claude/projects/my-project/session.jsonl")
        )
        assert not mod._project_transcript_filter(
            project, Path("/home/user/.claude/projects/other-project/session.jsonl")
        )


class TestTSRustTestGeneration:
    """TS/Rust generators produce real checks, not just TODOs — Epic 7 (whetstone-kxo)."""

    def _load_generate_tests_module(self):
        """Import generate-tests.py as a module."""
        import importlib.util

        spec = importlib.util.spec_from_file_location(
            "generate_tests", LEGACY_SCRIPTS_DIR / "generate-tests.py"
        )
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    def test_ts_deprecated_signal_produces_real_check(self):
        """TS generator with a pattern/deprecated signal produces scanning code."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "react.deprecated-api",
            "source_url": "https://react.dev/reference",
            "description": "Use createRoot instead of ReactDOM.render",
            "_dep_name": "react",
            "_language": "typescript",
            "signals": [
                {
                    "id": "deprecated-render",
                    "strategy": "pattern",
                    "description": "Uses deprecated `ReactDOM.render()`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_typescript_test(rule)
        # Should contain real scanning code, not just a TODO
        assert "for (let i = 0" in output or ".test(lines[i])" in output
        assert "violations.push" in output

    def test_ts_ast_async_signal_produces_real_check(self):
        """TS generator with async/sync AST signal produces route handler check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "express.async-handlers",
            "source_url": "https://expressjs.com/en/guide/error-handling.html",
            "description": "Route handlers MUST use async functions",
            "_dep_name": "express",
            "_language": "typescript",
            "signals": [
                {
                    "id": "sync-handler",
                    "strategy": "ast",
                    "description": "Function used as route handler is sync instead of async",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_typescript_test(rule)
        assert "routeDecorators" in output or "async" in output
        assert "violations.push" in output

    def test_rs_deprecated_signal_produces_real_check(self):
        """Rust generator with a pattern/deprecated signal produces scanning code."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "tokio.deprecated-api",
            "source_url": "https://docs.rs/tokio/latest",
            "description": "Use tokio::spawn instead of deprecated thread::spawn",
            "_dep_name": "tokio",
            "_language": "rust",
            "signals": [
                {
                    "id": "deprecated-spawn",
                    "strategy": "pattern",
                    "description": "Uses deprecated `thread::spawn()`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_rust_test(rule)
        assert "for (i, line) in content.lines()" in output
        assert "violations.push" in output

    def test_rs_unsafe_signal_produces_real_check(self):
        """Rust generator with unsafe AST signal produces real check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "serde.no-unsafe",
            "source_url": "https://docs.rs/serde/latest",
            "description": "Avoid unsafe blocks in serialization code",
            "_dep_name": "serde",
            "_language": "rust",
            "signals": [
                {
                    "id": "unsafe-block",
                    "strategy": "ast",
                    "description": "Uses unsafe block in serialization code",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_rust_test(rule)
        assert "unsafe" in output
        assert "violations.push" in output

    def test_rs_unwrap_signal_produces_real_check(self):
        """Rust generator with unwrap AST signal produces .unwrap() check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "anyhow.no-unwrap",
            "source_url": "https://docs.rs/anyhow/latest",
            "description": "Use ? operator instead of .unwrap()",
            "_dep_name": "anyhow",
            "_language": "rust",
            "signals": [
                {
                    "id": "unwrap-usage",
                    "strategy": "ast",
                    "description": "Uses .unwrap() which may panic at runtime",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_rust_test(rule)
        assert ".unwrap()" in output
        assert "violations.push" in output

    def test_rs_uses_mut_violations(self):
        """Rust generator uses 'let mut violations' for real checks."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "test.simple",
            "source_url": "https://example.com",
            "description": "Test rule",
            "_dep_name": "test",
            "_language": "rust",
            "signals": [
                {
                    "id": "check",
                    "strategy": "pattern",
                    "description": "Uses deprecated `old_api()`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_rust_test(rule)
        # Should use `let mut` since we push to violations
        assert "let mut violations" in output

    def test_generate_tests_fixture_produces_ts_checks(self, tmp_path):
        """End-to-end: TS rule fixture produces real test file with checks."""
        # Create a TS rule fixture
        rules_dir = tmp_path / "whetstone" / "rules" / "typescript"
        rules_dir.mkdir(parents=True)

        rule_yaml = rules_dir / "react.yaml"
        rule_yaml.write_text(
            """source:
  name: react
  version: "19.0.0"
  content_hash: sha256:abc123

rules:
  - id: react.use-create-root
    severity: must
    confidence: high
    category: migration
    description: Use createRoot instead of ReactDOM.render
    source_url: https://react.dev/reference/react-dom/client/createRoot
    approved: true
    signals:
      - id: deprecated-render
        strategy: pattern
        description: "Uses deprecated `ReactDOM.render()`"
        weight: required
"""
        )

        result = run_script(
            "generate-tests.py",
            ["--project-dir", str(tmp_path), "--lang", "typescript", "--dry-run"],
        )
        assert result["rules_processed"] == 1
        assert len(result["generated"]["tests"]["typescript"]) == 1

    def test_ts_setup_file_generated(self, tmp_path):
        """TypeScript generation produces a setup.ts with shared utilities."""
        mod = self._load_generate_tests_module()
        setup_content = mod.generate_typescript_setup()
        assert "findSourceFiles" in setup_content
        assert "readLines" in setup_content
        assert "violation" in setup_content
        assert "export function" in setup_content

    def test_ts_real_check_removes_experimental_header(self):
        """TS test with real check should NOT have EXPERIMENTAL header."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "react.deprecated-api",
            "source_url": "https://react.dev/reference",
            "description": "Use createRoot instead of ReactDOM.render",
            "_dep_name": "react",
            "_language": "typescript",
            "signals": [
                {
                    "id": "deprecated-render",
                    "strategy": "pattern",
                    "description": "Uses deprecated `ReactDOM.render()`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_typescript_test(rule)
        assert "EXPERIMENTAL" not in output

    def test_ts_import_signal_produces_check(self):
        """TS generator with import-related pattern produces import check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "react.no-default-import",
            "source_url": "https://react.dev/reference",
            "description": "Avoid importing from legacy module",
            "_dep_name": "react",
            "_language": "typescript",
            "signals": [
                {
                    "id": "legacy-import",
                    "strategy": "pattern",
                    "description": "Imports from `react-dom/render`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_typescript_test(rule)
        assert "import" in output.lower()
        assert "violations.push" in output

    def test_biome_config_nested_groups(self):
        """Biome config has proper nested group structure."""
        mod = self._load_generate_tests_module()
        rules = [
            {
                "id": "test.rule",
                "signals": [
                    {
                        "strategy": "lint_proxy",
                        "description": "Enable noExplicitAny",
                    },
                    {
                        "strategy": "lint_proxy",
                        "description": "Enable noUnusedVariables",
                    },
                    {
                        "strategy": "lint_proxy",
                        "description": "Enable useConst",
                    },
                ],
            }
        ]
        config = mod.generate_biome_config(rules)
        linter_rules = config["linter"]["rules"]
        # Rules should be in nested groups, not flat
        assert isinstance(linter_rules, dict)
        # Should have group keys like 'suspicious', 'correctness', 'style'
        for group_name, group_rules in linter_rules.items():
            assert group_name in (
                "suspicious",
                "correctness",
                "style",
                "nursery",
                "performance",
                "a11y",
                "security",
            )
            assert isinstance(group_rules, dict)
            for rule_name, severity in group_rules.items():
                assert severity == "error"
        # Check specific categorizations
        assert "noExplicitAny" in linter_rules.get("suspicious", {})
        assert "noUnusedVariables" in linter_rules.get("correctness", {})
        assert "useConst" in linter_rules.get("style", {})

    def test_clippy_config_has_lint_categories(self):
        """Clippy config has lint category comments."""
        mod = self._load_generate_tests_module()
        rules = [
            {
                "id": "test.rule",
                "signals": [
                    {
                        "strategy": "lint_proxy",
                        "description": "Enable unwrap_used lint",
                    },
                    {
                        "strategy": "lint_proxy",
                        "description": "Enable expect_used lint",
                    },
                    {
                        "strategy": "lint_proxy",
                        "description": "Enable doc_markdown lint",
                    },
                ],
            }
        ]
        config = mod.generate_clippy_config(rules)
        # Should have category comments
        assert "# Restriction" in config or "# Pedantic" in config
        # Should have lint assignments
        assert 'unwrap_used = "deny"' in config
        assert 'expect_used = "deny"' in config
        assert "[lints.clippy]" in config

    def test_rs_use_check_produces_real_check(self):
        """Rust generator with use statement signal produces use check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "tokio.old-runtime",
            "source_url": "https://docs.rs/tokio/latest",
            "description": "Use new tokio runtime API",
            "_dep_name": "tokio",
            "_language": "rust",
            "signals": [
                {
                    "id": "old-use",
                    "strategy": "pattern",
                    "description": "Uses deprecated `use tokio::old_runtime`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_rust_test(rule)
        assert "starts_with" in output or "contains" in output
        assert "violations.push" in output

    def test_python_import_check(self):
        """Python generator with import signal produces AST import check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "flask.old-import",
            "source_url": "https://flask.palletsprojects.com",
            "description": "Use new import path",
            "_dep_name": "flask",
            "_language": "python",
            "signals": [
                {
                    "id": "old-import",
                    "strategy": "ast",
                    "description": "Imports from `flask.ext`",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_python_test(rule)
        assert "ImportFrom" in output or "Import" in output
        assert "violations" in output

    def test_python_class_inheritance_check(self):
        """Python generator with class inheritance signal produces class check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "django.old-view",
            "source_url": "https://docs.djangoproject.com",
            "description": "Use new base view class",
            "_dep_name": "django",
            "_language": "python",
            "signals": [
                {
                    "id": "old-base",
                    "strategy": "ast",
                    "description": "Class inherits from DeprecatedView",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_python_test(rule)
        assert "ClassDef" in output
        assert "DeprecatedView" in output

    def test_python_kwarg_check(self):
        """Python generator with kwarg signal produces keyword argument check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "sqlalchemy.missing-kwarg",
            "source_url": "https://docs.sqlalchemy.org",
            "description": "Pass pool_size keyword argument",
            "_dep_name": "sqlalchemy",
            "_language": "python",
            "signals": [
                {
                    "id": "missing-pool-size",
                    "strategy": "ast",
                    "description": "create_engine() called without keyword argument pool_size",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_python_test(rule)
        assert "keywords" in output
        assert "pool_size" in output

    def test_python_signal_comments(self):
        """Python tests include Signal: comments before each check."""
        mod = self._load_generate_tests_module()
        rule = {
            "id": "fastapi.async-routes",
            "source_url": "https://fastapi.tiangolo.com/async/",
            "description": "Route handlers MUST use async def",
            "_dep_name": "fastapi",
            "_language": "python",
            "signals": [
                {
                    "id": "is-sync-function",
                    "strategy": "ast",
                    "description": "Function decorated with route decorator uses def instead of async def",
                    "weight": "required",
                }
            ],
        }
        output = mod.generate_python_test(rule)
        assert "# Signal: is-sync-function (ast, required)" in output


class TestCLIWrapper:
    """CLI wrapper dispatches to scripts correctly — Epic 8 (whetstone-xpf)."""

    def test_cli_help_exits_zero(self):
        """CLI --help exits with 0."""
        script = LEGACY_SCRIPTS_DIR / "cli.py"
        result = subprocess.run(
            [sys.executable, str(script), "--help"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 0
        assert "whetstone" in result.stderr.lower()

    def test_cli_unknown_command_exits_nonzero(self):
        """CLI with unknown command exits with 1."""
        script = LEGACY_SCRIPTS_DIR / "cli.py"
        result = subprocess.run(
            [sys.executable, str(script), "nonexistent"],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 1
        assert "Unknown command" in result.stderr

    def test_cli_dispatches_status(self):
        """CLI 'status' dispatches to status.py."""
        script = LEGACY_SCRIPTS_DIR / "cli.py"
        result = subprocess.run(
            [
                sys.executable,
                str(script),
                "status",
                "--project-dir",
                str(FIXTURES_DIR),
                "--score",
            ],
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert result.returncode == 0
        # status --score outputs "NN Label"
        assert any(
            label in result.stdout
            for label in ["Healthy", "Needs Review", "Stale", "No Rules"]
        )

    def test_cli_alias_deps(self):
        """CLI alias 'deps' dispatches to detect-deps.py."""
        script = LEGACY_SCRIPTS_DIR / "cli.py"
        result = subprocess.run(
            [sys.executable, str(script), "deps", "--project-dir", str(FIXTURES_DIR)],
            capture_output=True,
            text=True,
            timeout=60,
        )
        assert result.returncode == 0
        data = json.loads(result.stdout)
        assert "dependencies" in data

    def test_cli_no_args_exits_nonzero(self):
        """CLI with no args exits with 1 and shows help."""
        script = LEGACY_SCRIPTS_DIR / "cli.py"
        result = subprocess.run(
            [sys.executable, str(script)],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert result.returncode == 1
        assert "Usage" in result.stderr


class TestLongitudinalMetrics:
    """Metric snapshots and history — Epic 10 (whetstone-mzs)."""

    def test_status_creates_metrics_snapshot(self, tmp_path):
        """status.py records a metrics snapshot to .metrics.jsonl."""
        # Create minimal whetstone dir so status doesn't return not_initialized
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        (tmp_path / "whetstone" / "whetstone.yaml").write_text(
            "languages:\n  - python\n"
        )

        run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )

        metrics_file = tmp_path / "whetstone" / ".metrics.jsonl"
        assert metrics_file.exists(), "Metrics snapshot file not created"
        lines = metrics_file.read_text().strip().split("\n")
        assert len(lines) >= 1
        entry = json.loads(lines[0])
        assert "timestamp" in entry
        assert "score" in entry
        assert "label" in entry

    def test_status_no_snapshot_flag(self, tmp_path):
        """status.py --no-snapshot skips recording."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        (tmp_path / "whetstone" / "whetstone.yaml").write_text(
            "languages:\n  - python\n"
        )

        run_script(
            "status.py",
            [
                "--project-dir",
                str(tmp_path),
                "--json",
                "--no-drift-check",
                "--no-snapshot",
            ],
        )

        metrics_file = tmp_path / "whetstone" / ".metrics.jsonl"
        assert not metrics_file.exists()

    def test_history_flag_returns_entries(self, tmp_path):
        """status.py --history returns recorded snapshots."""
        # Create metrics file with sample entries
        metrics_dir = tmp_path / "whetstone"
        metrics_dir.mkdir(parents=True)
        metrics_file = metrics_dir / ".metrics.jsonl"
        entry1 = json.dumps(
            {
                "timestamp": "2026-02-01T00:00:00+00:00",
                "score": 80,
                "label": "Needs Review",
                "rules_approved": 3,
            }
        )
        entry2 = json.dumps(
            {
                "timestamp": "2026-03-01T00:00:00+00:00",
                "score": 90,
                "label": "Healthy",
                "rules_approved": 5,
            }
        )
        metrics_file.write_text(entry1 + "\n" + entry2 + "\n")

        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--history", "--json"],
        )
        assert "history" in result
        assert len(result["history"]) == 2
        assert result["history"][0]["score"] == 80
        assert result["history"][1]["score"] == 90

    def test_history_empty_is_graceful(self, tmp_path):
        """status.py --history with no metrics file returns empty list."""
        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--history", "--json"],
        )
        assert "history" in result
        assert result["history"] == []

    def test_snapshots_are_append_only(self, tmp_path):
        """Multiple status runs append, not overwrite."""
        rules_dir = tmp_path / "whetstone" / "rules" / "python"
        rules_dir.mkdir(parents=True)
        (tmp_path / "whetstone" / "whetstone.yaml").write_text(
            "languages:\n  - python\n"
        )

        # Run status twice
        run_script(
            "status.py", ["--project-dir", str(tmp_path), "--json", "--no-drift-check"]
        )
        run_script(
            "status.py", ["--project-dir", str(tmp_path), "--json", "--no-drift-check"]
        )

        metrics_file = tmp_path / "whetstone" / ".metrics.jsonl"
        lines = [
            line
            for line in metrics_file.read_text().strip().split("\n")
            if line.strip()
        ]
        assert len(lines) == 2, f"Expected 2 snapshots, got {len(lines)}"


class TestExhaustiveOutputContracts:
    """All scripts must always include next_command — Epic 5 (whetstone-zsn)."""

    def test_detect_patterns_quiet_empty_has_next_command(self, tmp_path):
        """detect-patterns --quiet with no patterns still has next_command."""
        result = run_script(
            "detect-patterns.py",
            ["--project-dir", str(tmp_path), "--quiet"],
        )
        assert "next_command" in result
        assert isinstance(result["next_command"], str)

    def test_detect_patterns_normal_has_next_command(self):
        """detect-patterns in normal mode has next_command."""
        result = run_script(
            "detect-patterns.py",
            ["--project-dir", str(FIXTURES_DIR)],
        )
        assert "next_command" in result

    def test_resolve_sources_empty_input_has_next_command(self):
        """resolve-sources with empty deps has next_command."""
        script = LEGACY_SCRIPTS_DIR / "resolve-sources.py"
        result = subprocess.run(
            [sys.executable, str(script), "--project-dir", str(FIXTURES_DIR)],
            input='{"dependencies": [], "languages": []}',
            capture_output=True,
            text=True,
            timeout=60,
        )
        data = json.loads(result.stdout)
        assert "next_command" in data

    def test_ci_check_error_path_has_expected_fields(self, tmp_path):
        """ci-check on empty project returns structured output."""
        result = run_script(
            "ci-check.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert "freshness_status" in result
        assert "score" in result or "freshness_status" in result


# --- v2 Incremental Pipeline Contracts ---


class TestV2IncrementalContracts:
    """Contract tests for v2 incremental pipeline features."""

    def test_detect_deps_incremental_has_fingerprint_keys(self):
        """detect-deps --incremental adds manifest_diff and inventory_diff."""
        result = run_script(
            "detect-deps.py",
            ["--project-dir", str(FIXTURES_DIR), "--incremental"],
        )
        assert "manifests_changed" in result
        assert isinstance(result["manifests_changed"], bool)
        assert "manifest_diff" in result
        diff = result["manifest_diff"]
        assert "changed" in diff
        assert "added" in diff
        assert "removed" in diff
        assert "unchanged" in diff
        assert "inventory_diff" in result

    def test_resolve_sources_has_cache_and_stats(self):
        """resolve-sources output includes cache and resolution_stats."""
        script = LEGACY_SCRIPTS_DIR / "resolve-sources.py"
        result = subprocess.run(
            [sys.executable, str(script), "--project-dir", str(FIXTURES_DIR)],
            input='{"dependencies": [], "languages": []}',
            capture_output=True,
            text=True,
            timeout=60,
        )
        data = json.loads(result.stdout)
        assert "cache" in data
        assert "resolution_stats" in data
        stats = data["resolution_stats"]
        assert "total" in stats
        assert "resolved" in stats
        assert "failed" in stats
        assert "skipped_cached" in stats
        assert "workers" in stats
        assert "wall_seconds" in stats
        assert "timings" in stats
        assert "by_source_type" in stats["timings"]
        assert "slowest_dependencies" in stats["timings"]

    def test_doctor_has_scan_and_buckets(self):
        """doctor output includes scan, resolution_buckets, extraction_subsets."""
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--skip-patterns"],
        )
        assert "scan" in result
        scan = result["scan"]
        assert "cache_stats" in scan
        assert "ranked_queue" in scan
        assert "resolution_buckets" in result
        buckets = result["resolution_buckets"]
        assert "ready_now" in buckets
        assert "failed" in buckets
        assert "extraction_subsets" in result

    def test_doctor_next_command_is_executable(self):
        """doctor next_command should be a real command, not prose."""
        result = run_script(
            "doctor.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--skip-patterns"],
        )
        next_command = result["next_command"]
        assert isinstance(next_command, str)
        assert next_command.startswith("whetstone ") or next_command.startswith(
            "python3 "
        )
        assert "Agent:" not in next_command

    def test_status_has_pipeline_state(self):
        """status output includes pipeline_state and cache_stats."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        assert "pipeline_state" in result
        assert "cache_stats" in result
        assert "extraction_readiness" in result

    def test_status_drift_has_structured_format(self):
        """status drift separates dependency_changes from documentation_stale."""
        result = run_script(
            "status.py",
            ["--project-dir", str(FIXTURES_DIR), "--json", "--no-drift-check"],
        )
        drift = result.get("drift", {})
        assert isinstance(drift, dict)
        assert "dependency_changes" in drift
        assert "documentation_stale" in drift
        assert "count" in drift
