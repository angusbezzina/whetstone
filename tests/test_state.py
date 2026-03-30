"""Unit tests for the Whetstone state management layer.

Tests StateManager, ManifestStore, InventoryStore, SourceCacheStore,
and RefreshLog — CRUD operations, atomic writes, TTL, invalidation.

Run with: pytest tests/test_state.py -v
"""

from __future__ import annotations

import json

# Add archived legacy scripts to path for import
import sys
from datetime import datetime, timedelta, timezone
from pathlib import Path

import pytest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "scripts" / "legacy"))

from state import (
    InventoryStore,
    ManifestStore,
    RefreshLog,
    SourceCacheStore,
    StateManager,
    _atomic_write,
    _load_json,
)


@pytest.fixture
def tmp_project(tmp_path):
    """Create a temporary project directory with whetstone/.state/."""
    state_dir = tmp_path / "whetstone" / ".state"
    state_dir.mkdir(parents=True)
    return tmp_path


@pytest.fixture
def state_mgr(tmp_project):
    """Create a StateManager pointed at tmp_project."""
    return StateManager(tmp_project)


# --- Atomic write ---


class TestAtomicWrite:
    def test_creates_file(self, tmp_path):
        path = tmp_path / "test.json"
        _atomic_write(path, {"key": "value"})
        assert path.exists()
        data = json.loads(path.read_text())
        assert data["key"] == "value"

    def test_creates_parent_dirs(self, tmp_path):
        path = tmp_path / "nested" / "deep" / "test.json"
        _atomic_write(path, {"a": 1})
        assert path.exists()

    def test_overwrites_existing(self, tmp_path):
        path = tmp_path / "test.json"
        _atomic_write(path, {"v": 1})
        _atomic_write(path, {"v": 2})
        data = json.loads(path.read_text())
        assert data["v"] == 2

    def test_no_temp_files_left(self, tmp_path):
        path = tmp_path / "test.json"
        _atomic_write(path, {"a": 1})
        files = list(tmp_path.iterdir())
        assert len(files) == 1
        assert files[0].name == "test.json"


class TestLoadJson:
    def test_missing_file(self, tmp_path):
        assert _load_json(tmp_path / "nope.json") == {}

    def test_corrupt_file(self, tmp_path):
        bad = tmp_path / "bad.json"
        bad.write_text("not json {{{")
        assert _load_json(bad) == {}

    def test_valid_file(self, tmp_path):
        good = tmp_path / "good.json"
        good.write_text('{"key": "val"}')
        assert _load_json(good) == {"key": "val"}


# --- ManifestStore ---


class TestManifestStore:
    def test_upsert_and_get(self, tmp_project):
        store = ManifestStore(tmp_project / "whetstone" / ".state" / "manifests.json")
        store.load()
        store.upsert("pyproject.toml", sha256="abc123", workspace="root")
        entry = store.get("pyproject.toml")
        assert entry is not None
        assert entry["sha256"] == "abc123"
        assert entry["workspace"] == "root"
        assert entry["path"] == "pyproject.toml"

    def test_upsert_preserves_first_seen(self, tmp_project):
        store = ManifestStore(tmp_project / "whetstone" / ".state" / "manifests.json")
        store.load()
        store.upsert("pyproject.toml", sha256="v1", workspace="root")
        first = store.get("pyproject.toml")["first_seen"]
        store.upsert("pyproject.toml", sha256="v2", workspace="root")
        assert store.get("pyproject.toml")["first_seen"] == first
        assert store.get("pyproject.toml")["sha256"] == "v2"

    def test_compare_detects_changes(self, tmp_project):
        store = ManifestStore(tmp_project / "whetstone" / ".state" / "manifests.json")
        store.load()
        store.upsert("a.toml", sha256="aaa", workspace="root")
        store.upsert("b.toml", sha256="bbb", workspace="root")
        store.upsert("c.toml", sha256="ccc", workspace="root")

        diff = store.compare(
            {
                "a.toml": "aaa",  # unchanged
                "b.toml": "XXX",  # changed
                "d.toml": "ddd",  # added
                # c.toml removed
            }
        )
        assert diff.unchanged == ["a.toml"]
        assert diff.changed == ["b.toml"]
        assert diff.added == ["d.toml"]
        assert diff.removed == ["c.toml"]

    def test_compare_empty_store(self, tmp_project):
        store = ManifestStore(tmp_project / "whetstone" / ".state" / "manifests.json")
        store.load()
        diff = store.compare({"a.toml": "aaa"})
        assert diff.added == ["a.toml"]
        assert diff.changed == []
        assert diff.removed == []

    def test_fingerprint_file(self, tmp_path):
        f = tmp_path / "test.txt"
        f.write_text("hello world")
        sha = ManifestStore.fingerprint_file(f)
        assert len(sha) == 64  # SHA256 hex
        # Same content = same hash
        f2 = tmp_path / "test2.txt"
        f2.write_text("hello world")
        assert ManifestStore.fingerprint_file(f2) == sha

    def test_save_and_reload(self, tmp_project):
        path = tmp_project / "whetstone" / ".state" / "manifests.json"
        store = ManifestStore(path)
        store.load()
        store.upsert("pyproject.toml", sha256="abc", workspace="root")
        store.save()

        # Reload from disk
        store2 = ManifestStore(path)
        store2.load()
        entry = store2.get("pyproject.toml")
        assert entry is not None
        assert entry["sha256"] == "abc"


# --- InventoryStore ---


class TestInventoryStore:
    def _dep(
        self,
        name="fastapi",
        language="python",
        version="0.115.0",
        dev=False,
        sources=None,
    ):
        return {
            "name": name,
            "language": language,
            "version": version,
            "dev": dev,
            "sources": sources or ["root"],
        }

    def test_upsert_and_get(self, tmp_project):
        store = InventoryStore(tmp_project / "whetstone" / ".state" / "inventory.json")
        store.load()
        store.upsert(self._dep())
        dep = store.get("python", "fastapi")
        assert dep is not None
        assert dep["name"] == "fastapi"
        assert dep["state"] == "discovered"

    def test_upsert_preserves_state(self, tmp_project):
        store = InventoryStore(tmp_project / "whetstone" / ".state" / "inventory.json")
        store.load()
        store.upsert(self._dep())
        store.set_state("python", "fastapi", "resolved")
        store.upsert(self._dep(version="0.116.0"))
        dep = store.get("python", "fastapi")
        assert dep["state"] == "resolved"
        assert dep["version"] == "0.116.0"

    def test_set_state_validates(self, tmp_project):
        store = InventoryStore(tmp_project / "whetstone" / ".state" / "inventory.json")
        store.load()
        store.upsert(self._dep())
        with pytest.raises(ValueError, match="Invalid state"):
            store.set_state("python", "fastapi", "bogus")

    def test_set_state_missing_dep(self, tmp_project):
        store = InventoryStore(tmp_project / "whetstone" / ".state" / "inventory.json")
        store.load()
        with pytest.raises(KeyError, match="not found"):
            store.set_state("python", "nope", "resolved")

    def test_bulk_upsert_diff(self, tmp_project):
        store = InventoryStore(tmp_project / "whetstone" / ".state" / "inventory.json")
        store.load()
        store.upsert(self._dep("fastapi", "python", "0.115.0"))
        store.upsert(self._dep("flask", "python", "3.0.0"))

        diff = store.bulk_upsert(
            [
                self._dep("fastapi", "python", "0.116.0"),  # changed version
                self._dep("pydantic", "python", "2.6.0"),  # added
                # flask removed
            ]
        )
        assert "python:pydantic" in diff.added
        assert "python:fastapi" in diff.changed
        assert "python:flask" in diff.removed

    def test_by_state(self, tmp_project):
        store = InventoryStore(tmp_project / "whetstone" / ".state" / "inventory.json")
        store.load()
        store.upsert(self._dep("fastapi", "python"))
        store.upsert(self._dep("pydantic", "python"))
        store.set_state("python", "fastapi", "resolved")

        resolved = store.by_state("resolved")
        assert len(resolved) == 1
        assert resolved[0]["name"] == "fastapi"

        discovered = store.by_state("discovered")
        assert len(discovered) == 1
        assert discovered[0]["name"] == "pydantic"

    def test_save_and_reload(self, tmp_project):
        path = tmp_project / "whetstone" / ".state" / "inventory.json"
        store = InventoryStore(path)
        store.load()
        store.upsert(self._dep())
        store.set_state("python", "fastapi", "resolved")
        store.save()

        store2 = InventoryStore(path)
        store2.load()
        dep = store2.get("python", "fastapi")
        assert dep["state"] == "resolved"


# --- SourceCacheStore ---


class TestSourceCacheStore:
    def _entry(
        self,
        name="fastapi",
        language="python",
        version="0.115.0",
        source_type="llms_txt",
        confidence="high",
        errors=None,
    ):
        return {
            "name": name,
            "language": language,
            "version": version,
            "docs_url": f"https://{name}.example.com",
            "llms_txt_url": f"https://{name}.example.com/llms.txt",
            "source_type": source_type,
            "content_hash": "sha256:abc123",
            "fetch_timestamp": datetime.now(timezone.utc).isoformat(),
            "ttl_seconds": 604800,
            "confidence": confidence,
            "latest_version": version,
            "latest_release_date": "2026-03-15T00:00:00Z",
            "errors": errors,
        }

    def test_upsert_and_get(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        store.upsert(self._entry())
        entry = store.get("python", "fastapi", "0.115.0")
        assert entry is not None
        assert entry["source_type"] == "llms_txt"

    def test_is_fresh_within_ttl(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        store.upsert(self._entry())
        assert store.is_fresh("python", "fastapi", "0.115.0") is True

    def test_is_fresh_expired(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        entry = self._entry()
        entry["fetch_timestamp"] = (
            datetime.now(timezone.utc) - timedelta(days=30)
        ).isoformat()
        store.upsert(entry)
        assert store.is_fresh("python", "fastapi", "0.115.0") is False

    def test_is_fresh_with_errors(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        store.upsert(self._entry(errors="connection timeout"))
        assert store.is_fresh("python", "fastapi", "0.115.0") is False

    def test_is_fresh_missing(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        assert store.is_fresh("python", "nope", "1.0") is False

    def test_invalidate_by_version(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        store.upsert(self._entry(version="0.115.0"))
        assert store.get("python", "fastapi", "0.115.0") is not None
        removed = store.invalidate_by_version("python", "fastapi", "0.115.0", "0.116.0")
        assert removed is True
        assert store.get("python", "fastapi", "0.115.0") is None

    def test_invalidate_missing_entry(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        removed = store.invalidate_by_version("python", "nope", "1.0", "2.0")
        assert removed is False

    def test_stats(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        store.upsert(self._entry("fastapi", "python", "0.115.0"))
        expired = self._entry("flask", "python", "3.0.0")
        expired["fetch_timestamp"] = (
            datetime.now(timezone.utc) - timedelta(days=30)
        ).isoformat()
        store.upsert(expired)
        store.upsert(self._entry("broken", "python", "1.0", errors="timeout"))

        stats = store.stats()
        assert stats.hits == 1
        assert stats.stale == 2
        assert stats.total == 3

    def test_custom_ttl(self, tmp_project):
        store = SourceCacheStore(
            tmp_project / "whetstone" / ".state" / "source-cache.json"
        )
        store.load()
        store.upsert(self._entry())
        # With a 0-second TTL, everything is stale
        assert store.is_fresh("python", "fastapi", "0.115.0", ttl_seconds=0) is False

    def test_save_and_reload(self, tmp_project):
        path = tmp_project / "whetstone" / ".state" / "source-cache.json"
        store = SourceCacheStore(path)
        store.load()
        store.upsert(self._entry())
        store.save()

        store2 = SourceCacheStore(path)
        store2.load()
        entry = store2.get("python", "fastapi", "0.115.0")
        assert entry is not None


# --- RefreshLog ---


class TestRefreshLog:
    def test_record_and_recent(self, tmp_project):
        log = RefreshLog(tmp_project / "whetstone" / ".state" / "refresh-log.json")
        log.load()
        log.record("version_changed", "python:fastapi", "0.115 -> 0.116")
        log.record("ttl_expired", "python:flask", "cache expired")

        recent = log.recent(10)
        assert len(recent) == 2
        assert recent[0]["type"] == "version_changed"
        assert recent[1]["type"] == "ttl_expired"

    def test_trim_to_max(self, tmp_project):
        log = RefreshLog(tmp_project / "whetstone" / ".state" / "refresh-log.json")
        log.load()
        for i in range(250):
            log.record("test", f"target-{i}", f"detail-{i}")
        signals = log._signals
        assert len(signals) == 200

    def test_save_and_reload(self, tmp_project):
        path = tmp_project / "whetstone" / ".state" / "refresh-log.json"
        log = RefreshLog(path)
        log.load()
        log.record("test", "target", "detail")
        log.save()

        log2 = RefreshLog(path)
        log2.load()
        assert len(log2.recent()) == 1


# --- StateManager facade ---


class TestStateManager:
    def test_ensure_dir_creates_state_dir(self, tmp_path):
        sm = StateManager(tmp_path)
        sm.ensure_dir()
        assert (tmp_path / "whetstone" / ".state").is_dir()

    def test_load_all_empty(self, tmp_project):
        sm = StateManager(tmp_project)
        sm.load_all()  # Should not raise
        assert sm.manifests._loaded
        assert sm.inventory._loaded
        assert sm.cache._loaded
        assert sm.refresh_log._loaded

    def test_save_all_creates_files(self, tmp_project):
        sm = StateManager(tmp_project)
        sm.load_all()
        sm.save_all()
        state_dir = tmp_project / "whetstone" / ".state"
        assert (state_dir / "manifests.json").exists()
        assert (state_dir / "inventory.json").exists()
        assert (state_dir / "source-cache.json").exists()
        assert (state_dir / "refresh-log.json").exists()

    def test_full_workflow(self, tmp_project):
        """End-to-end: fingerprint → inventory → cache → log → reload."""
        sm = StateManager(tmp_project)
        sm.load_all()

        # Create a manifest to fingerprint
        manifest = tmp_project / "pyproject.toml"
        manifest.write_text('[project]\nname = "test"\n')

        sha = sm.manifests.fingerprint_file(manifest)
        sm.manifests.upsert("pyproject.toml", sha256=sha, workspace="root")

        sm.inventory.upsert(
            {
                "name": "fastapi",
                "language": "python",
                "version": "0.115.0",
                "dev": False,
                "sources": ["root"],
            }
        )
        sm.inventory.set_state("python", "fastapi", "resolved")

        sm.cache.upsert(
            {
                "name": "fastapi",
                "language": "python",
                "version": "0.115.0",
                "docs_url": "https://fastapi.tiangolo.com",
                "llms_txt_url": None,
                "source_type": "docs_url_only",
                "content_hash": "sha256:test",
                "fetch_timestamp": datetime.now(timezone.utc).isoformat(),
                "ttl_seconds": 604800,
                "confidence": "medium",
                "latest_version": "0.115.0",
                "latest_release_date": None,
                "errors": None,
            }
        )

        sm.refresh_log.record("version_changed", "python:fastapi", "new version")

        sm.save_all()

        # Reload and verify
        sm2 = StateManager(tmp_project)
        sm2.load_all()

        assert sm2.manifests.get("pyproject.toml")["sha256"] == sha
        assert sm2.inventory.get("python", "fastapi")["state"] == "resolved"
        assert sm2.cache.is_fresh("python", "fastapi", "0.115.0")
        assert len(sm2.refresh_log.recent()) == 1

    def test_schema_version_mismatch_resets(self, tmp_project):
        """If schema version changes, store resets to empty."""
        path = tmp_project / "whetstone" / ".state" / "manifests.json"
        path.write_text('{"version": 999, "manifests": {"a": "b"}}')

        store = ManifestStore(path)
        store.load()
        # Should have reset since version doesn't match
        assert store.get("a") is None
