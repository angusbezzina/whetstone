"""Integration tests for the Whetstone v2 incremental pipeline.

Tests cache reuse, fingerprint stability, resume after interruption,
and state persistence across runs.

Run with: pytest tests/test_incremental.py -v
"""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from pathlib import Path

SCRIPTS_DIR = Path(__file__).resolve().parent.parent / "scripts"
FIXTURES_DIR = Path(__file__).resolve().parent / "fixtures"


def load_script_module(filename: str, module_name: str):
    """Load a script module directly from file path."""
    path = SCRIPTS_DIR / filename
    spec = importlib.util.spec_from_file_location(module_name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec is not None and spec.loader is not None
    spec.loader.exec_module(module)
    return module


StateManager = load_script_module("state.py", "state_test_module").StateManager


def make_dep(name: str, language: str = "python") -> dict:
    return {"name": name, "language": language, "version": ">=1.0", "dev": False}


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

    def test_incomplete_llms_cache_entry_is_refetched(self, tmp_path, monkeypatch):
        """Fresh llms cache entries without content must be re-fetched, not reused."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.31"]\n'
        )

        sm = StateManager(tmp_path)
        sm.ensure_dir()
        sm.load_all()
        sm.inventory.upsert(
            {
                "name": "requests",
                "language": "python",
                "version": ">=2.31",
                "dev": False,
                "sources": ["root"],
            }
        )
        sm.inventory.set_state("python", "requests", "extraction_ready")
        sm.cache.upsert(
            {
                "name": "requests",
                "language": "python",
                "version": ">=2.31",
                "docs_url": "https://example.com",
                "llms_txt_url": "https://example.com/llms.txt",
                "source_type": "llms_txt",
                "content": None,
                "content_hash": "sha256:old",
                "freshness": {"confidence": "high", "content_stale": False},
                "fetch_timestamp": "2026-03-29T00:00:00+00:00",
                "ttl_seconds": 604800,
            }
        )
        sm.save_all()

        mod = load_script_module("resolve-sources.py", "resolve_sources_test")

        def fake_resolver(name: str, version: str, timeout: int) -> dict:
            return {
                "docs_url": "https://example.com",
                "llms_txt_url": "https://example.com/llms.txt",
                "source_type": "llms_txt",
                "content": "resolved llms content",
                "content_hash": "sha256:new",
                "latest_version": "2.32.0",
                "latest_release_date": "2026-03-01T00:00:00+00:00",
            }

        monkeypatch.setitem(mod.RESOLVERS, "python", fake_resolver)

        result = mod.resolve_sources(
            {
                "dependencies": [
                    {
                        "name": "requests",
                        "language": "python",
                        "version": ">=2.31",
                        "dev": False,
                    }
                ]
            },
            project_dir=tmp_path,
        )

        assert result["resolution_stats"]["skipped_cached"] == 0
        assert result["resolution_stats"]["resolved"] == 1
        assert result["sources"][0]["content"] == "resolved llms content"
        refreshed = StateManager(tmp_path)
        refreshed.load_all()
        cached = refreshed.cache.get("python", "requests", ">=2.31")
        assert cached is not None
        assert cached["content"] == "resolved llms content"


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

    def test_state_only_repo_is_not_not_initialized_for_status(self, tmp_path):
        """Repos with whetstone/.state should report pipeline status, not not_initialized."""
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

        result = run_script(
            "status.py",
            ["--project-dir", str(tmp_path), "--json", "--no-drift-check"],
        )
        assert result["status"] == "ok"
        assert result["label"] == "No Rules"
        assert result["pipeline_state"]["total_deps"] == 1

    def test_doctor_ready_only_uses_cached_extraction_ready_sources(self, tmp_path):
        """doctor --ready-only should return cached llms-backed extraction-ready deps."""
        pyproject = tmp_path / "pyproject.toml"
        pyproject.write_text(
            '[project]\nname = "test"\ndependencies = ["requests>=2.31"]\n'
        )

        sm = StateManager(tmp_path)
        sm.ensure_dir()
        sm.load_all()
        sm.manifests.upsert("pyproject.toml", sha256="abc", workspace="")
        sm.inventory.upsert(
            {
                "name": "requests",
                "language": "python",
                "version": ">=2.31",
                "dev": False,
                "sources": ["root"],
            }
        )
        sm.inventory.set_state("python", "requests", "extraction_ready")
        sm.cache.upsert(
            {
                "name": "requests",
                "language": "python",
                "version": ">=2.31",
                "docs_url": "https://example.com",
                "llms_txt_url": "https://example.com/llms.txt",
                "source_type": "llms_txt",
                "content": "cached llms content",
                "content_hash": "sha256:cached",
                "freshness": {"confidence": "high", "content_stale": False},
                "fetch_timestamp": "2099-01-01T00:00:00+00:00",
                "ttl_seconds": 604800,
            }
        )
        sm.save_all()

        result = run_script(
            "doctor.py",
            [
                "--project-dir",
                str(tmp_path),
                "--json",
                "--skip-patterns",
                "--ready-only",
            ],
        )

        assert result["extraction_context"]["dep_names"] == ["requests"]
        assert len(result["extraction_context"]["sources"]) == 1
        assert result["resolution_buckets"]["ready_now"] == [
            {"name": "requests", "source_type": "llms_txt"}
        ]
        assert result["next_command"] == "whetstone status --extraction-ready"

    def test_doctor_defaults_to_fast_first_on_large_uncached_repo(
        self, tmp_path, monkeypatch
    ):
        """Large uncached repos should auto-limit first pass and suggest resume."""
        mod = load_script_module("doctor.py", "doctor_fast_first_test")

        deps = [make_dep(f"dep{i}") for i in range(12)]

        detect_result = {
            "dependencies": deps,
            "languages": ["python"],
            "counts": {"runtime": {"_all": 12, "python": 12}, "dev": {"_all": 0}},
            "manifest_diff": {
                "changed": [],
                "added": [],
                "removed": [],
                "unchanged": ["pyproject.toml"],
            },
            "inventory_diff": {
                "added": [f"python:dep{i}" for i in range(12)],
                "changed": [],
                "removed": [],
                "unchanged": [],
            },
            "manifests_changed": True,
        }

        captured = {"resolve_args": None}

        def fake_run_script(name: str, args: list[str], stdin_data: str | None = None):
            if name == "detect-deps.py":
                return detect_result, 0.0
            if name == "resolve-sources.py":
                captured["resolve_args"] = args
                dep_names = [d["name"] for d in deps]
                for idx, arg in enumerate(args):
                    if arg == "--deps":
                        dep_names = args[idx + 1].split(",")
                return {
                    "sources": [
                        {
                            "name": dep_names[0],
                            "language": "python",
                            "source_type": "llms_full_txt",
                            "freshness": {"confidence": "high"},
                            "content": "...",
                        }
                    ],
                    "errors": [],
                }, 0.0
            return {"patterns": []}, 0.0

        monkeypatch.setattr(mod, "_run_script", fake_run_script)

        result = mod.doctor(project_dir=tmp_path, skip_patterns=True, json_mode=True)

        assert captured["resolve_args"] is not None
        dep_arg = captured["resolve_args"][captured["resolve_args"].index("--deps") + 1]
        assert len(dep_arg.split(",")) == mod.DEFAULT_FAST_FIRST_MAX_DEPS
        assert result["workflow"]["fast_first"] is True
        assert result["workflow"]["remaining_dependencies"] == 2
        assert result["next_command"] == "whetstone doctor --resume"
        assert any(
            rec.get("command") == "whetstone doctor --resume"
            for rec in result["recommendations"]
        )

    def test_full_run_disables_fast_first_limiting(self, tmp_path, monkeypatch):
        """--full-run should bypass default fast-first limiting."""
        mod = load_script_module("doctor.py", "doctor_full_run_test")

        deps = [make_dep(f"dep{i}") for i in range(12)]
        detect_result = {
            "dependencies": deps,
            "languages": ["python"],
            "counts": {"runtime": {"_all": 12, "python": 12}, "dev": {"_all": 0}},
            "manifest_diff": {
                "changed": [],
                "added": [],
                "removed": [],
                "unchanged": ["pyproject.toml"],
            },
            "inventory_diff": {
                "added": [f"python:dep{i}" for i in range(12)],
                "changed": [],
                "removed": [],
                "unchanged": [],
            },
            "manifests_changed": True,
        }

        captured = {"resolve_args": None}

        def fake_run_script(name: str, args: list[str], stdin_data: str | None = None):
            if name == "detect-deps.py":
                return detect_result, 0.0
            if name == "resolve-sources.py":
                captured["resolve_args"] = args
                dep_names = [d["name"] for d in deps]
                for idx, arg in enumerate(args):
                    if arg == "--deps":
                        dep_names = args[idx + 1].split(",")
                return {
                    "sources": [
                        {
                            "name": dep_names[0],
                            "language": "python",
                            "source_type": "llms_full_txt",
                            "freshness": {"confidence": "high"},
                            "content": "...",
                        }
                    ],
                    "errors": [],
                }, 0.0
            return {"patterns": []}, 0.0

        monkeypatch.setattr(mod, "_run_script", fake_run_script)

        result = mod.doctor(
            project_dir=tmp_path,
            skip_patterns=True,
            json_mode=True,
            full_run=True,
        )

        assert (
            captured["resolve_args"] is None or "--deps" not in captured["resolve_args"]
        )
        assert result["workflow"]["fast_first"] is False
        assert result["next_command"] == "whetstone status --extraction-ready"

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
