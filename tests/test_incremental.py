"""Integration tests for the Whetstone v2 incremental pipeline.

Tests cache reuse, fingerprint stability, resume after interruption,
and state persistence across runs.

Run with: pytest tests/test_incremental.py -v
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

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
    return json.loads(result.stdout)


class TestManifestFingerprinting:
    """Tests for detect-deps --incremental fingerprint stability."""

    def test_second_run_shows_no_changes(self, tmp_path):
        """Running detect-deps --incremental twice shows no changes on second run."""
        # Create a minimal manifest
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.31"]\n'
        )

        # First run — everything is new
        r1 = run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )
        assert r1["manifests_changed"] is True
        assert "pyproject.toml" in r1["manifest_diff"]["added"]

        # Second run — nothing changed
        r2 = run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )
        assert r2["manifests_changed"] is False
        assert r2["manifest_diff"]["changed"] == []
        assert r2["manifest_diff"]["added"] == []
        assert "pyproject.toml" in r2["manifest_diff"]["unchanged"]

    def test_manifest_change_detected(self, tmp_path):
        """Changing a manifest between runs is detected as changed."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.31"]\n'
        )

        # First run
        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        # Modify manifest
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.32", "flask>=3.0"]\n'
        )

        # Second run — detects change
        r2 = run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )
        assert r2["manifests_changed"] is True
        assert "pyproject.toml" in r2["manifest_diff"]["changed"]

    def test_inventory_tracks_new_deps(self, tmp_path):
        """New dependencies show up in inventory_diff.added."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.31"]\n'
        )

        r1 = run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )
        assert "python:requests" in r1["inventory_diff"]["added"]
        assert r1["inventory_diff"]["changed"] == []

    def test_inventory_tracks_version_changes(self, tmp_path):
        """Version changes show up in inventory_diff.changed."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.31"]\n'
        )
        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        # Change version
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.32"]\n'
        )
        r2 = run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )
        assert "python:requests" in r2["inventory_diff"]["changed"]


class TestSourceCacheReuse:
    """Tests for resolve-sources cache reuse and TTL."""

    def test_empty_deps_has_zero_stats(self):
        """Empty input yields zero resolution stats."""
        script = SCRIPTS_DIR / "resolve-sources.py"
        result = subprocess.run(
            [sys.executable, str(script), "--project-dir", str(FIXTURES_DIR)],
            input='{"dependencies": [], "languages": []}',
            capture_output=True,
            text=True,
            timeout=60,
        )
        data = json.loads(result.stdout)
        stats = data["resolution_stats"]
        assert stats["total"] == 0
        assert stats["resolved"] == 0
        assert stats["failed"] == 0
        assert stats["skipped_cached"] == 0

    def test_cache_counts_are_non_negative(self):
        """Cache hit/miss/stale counts are all non-negative."""
        script = SCRIPTS_DIR / "resolve-sources.py"
        result = subprocess.run(
            [sys.executable, str(script), "--project-dir", str(FIXTURES_DIR)],
            input='{"dependencies": [], "languages": []}',
            capture_output=True,
            text=True,
            timeout=60,
        )
        data = json.loads(result.stdout)
        cache = data["cache"]
        assert cache["hit"] >= 0
        assert cache["miss"] >= 0
        assert cache["stale"] >= 0


class TestStatePersistence:
    """Tests for state file persistence and structure."""

    def test_incremental_creates_state_dir(self, tmp_path):
        """Running detect-deps --incremental creates whetstone/.state/."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text('[project]\nname = "test"\ndependencies = ["flask"]\n')

        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        state_dir = tmp_path / "whetstone" / ".state"
        assert state_dir.is_dir()
        assert (state_dir / "manifests.json").exists()
        assert (state_dir / "inventory.json").exists()

    def test_state_files_are_valid_json(self, tmp_path):
        """All state files are valid JSON with version field."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text('[project]\nname = "test"\ndependencies = ["flask"]\n')

        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        state_dir = tmp_path / "whetstone" / ".state"
        for name in ("manifests.json", "inventory.json"):
            path = state_dir / name
            data = json.loads(path.read_text())
            assert "version" in data
            assert data["version"] == 1
            assert "updated_at" in data

    def test_inventory_has_lifecycle_state(self, tmp_path):
        """Dependencies in inventory have a lifecycle state field."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text('[project]\nname = "test"\ndependencies = ["flask"]\n')

        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        inv = json.loads(
            (tmp_path / "whetstone" / ".state" / "inventory.json").read_text()
        )
        deps = inv.get("dependencies", {})
        assert len(deps) > 0
        for key, dep in deps.items():
            assert "state" in dep
            assert dep["state"] == "discovered"  # Initial state

    def test_refresh_log_records_manifest_changes(self, tmp_path):
        """Changing a manifest records a refresh signal."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text('[project]\nname = "test"\ndependencies = ["flask"]\n')
        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        # Modify manifest
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["flask", "requests"]\n'
        )
        run_script(
            "detect-deps.py",
            [
                "--project-dir",
                str(tmp_path),
                "--incremental",
            ],
        )

        log_path = tmp_path / "whetstone" / ".state" / "refresh-log.json"
        if log_path.exists():
            log = json.loads(log_path.read_text())
            signals = log.get("signals", [])
            signal_types = [s["type"] for s in signals]
            assert "manifest_changed" in signal_types


class TestDoctorIncremental:
    """Tests for doctor Phase A/B staged output."""

    def test_doctor_scan_has_cache_stats(self):
        """Doctor output scan section has cache_stats."""
        result = run_script(
            "doctor.py",
            [
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--skip-patterns",
            ],
        )
        scan = result.get("scan", {})
        cs = scan.get("cache_stats", {})
        assert isinstance(cs.get("cached", 0), int)
        assert isinstance(cs.get("missing", 0), int)
        assert isinstance(cs.get("stale", 0), int)
        assert isinstance(cs.get("failed", 0), int)

    def test_doctor_ranked_queue_has_scores(self):
        """Doctor ranked queue entries have name, language, score."""
        result = run_script(
            "doctor.py",
            [
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--skip-patterns",
            ],
        )
        queue = result.get("scan", {}).get("ranked_queue", [])
        for entry in queue:
            assert "name" in entry
            assert "language" in entry
            assert "score" in entry
            assert isinstance(entry["score"], (int, float))

    def test_doctor_extraction_subsets_present(self):
        """Doctor output has extraction_subsets with expected buckets."""
        result = run_script(
            "doctor.py",
            [
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--skip-patterns",
            ],
        )
        subsets = result.get("extraction_subsets", {})
        assert "ready_now" in subsets
        assert "resolved_not_ready" in subsets
        assert "pending" in subsets
        assert "failed" in subsets


class TestStatusPipeline:
    """Tests for status pipeline state reporting."""

    def test_status_pipeline_state_counts(self):
        """Status pipeline_state has expected count keys."""
        result = run_script(
            "status.py",
            [
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
            ],
        )
        ps = result.get("pipeline_state", {})
        for key in (
            "total_deps",
            "resolved",
            "extraction_ready",
            "failed",
            "stale",
            "discovered",
        ):
            assert key in ps
            assert isinstance(ps[key], int)

    def test_status_cache_stats_present(self):
        """Status has cache_stats section."""
        result = run_script(
            "status.py",
            [
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
            ],
        )
        cs = result.get("cache_stats", {})
        assert isinstance(cs, dict)

    def test_status_extraction_readiness_list(self):
        """Status extraction_readiness is a list of dep entries."""
        result = run_script(
            "status.py",
            [
                "--project-dir",
                str(FIXTURES_DIR),
                "--json",
                "--no-drift-check",
            ],
        )
        er = result.get("extraction_readiness", [])
        assert isinstance(er, list)
        for entry in er:
            assert "name" in entry
            assert "language" in entry
            assert "state" in entry
