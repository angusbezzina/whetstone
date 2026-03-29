#!/usr/bin/env python3
"""Whetstone persistent state layer.

Provides atomic read/write for manifest fingerprints, dependency
inventory, and source resolution cache. All state lives under
whetstone/.state/ within the project directory.

Usage:
    from state import StateManager
    sm = StateManager(Path("."))
    sm.manifests.load()
    sm.manifests.upsert("pyproject.toml", sha256="abc", workspace="root")
    sm.manifests.save()
"""

from __future__ import annotations

import hashlib
import json
import os
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import NamedTuple

# --- Data types ---


class ManifestDiff(NamedTuple):
    """Result of comparing current manifest fingerprints against stored."""

    changed: list[str]
    added: list[str]
    removed: list[str]
    unchanged: list[str]


class InventoryDiff(NamedTuple):
    """Result of merging new dependency detection into stored inventory."""

    added: list[str]
    changed: list[str]
    removed: list[str]
    unchanged: list[str]


class CacheStats(NamedTuple):
    """Source cache hit/miss/stale statistics."""

    hits: int
    misses: int
    stale: int
    total: int


# --- Lifecycle states ---

LIFECYCLE_STATES = (
    "discovered",
    "queued",
    "resolving",
    "resolved",
    "extraction_ready",
    "extracted",
    "approved",
    "stale",
    "failed",
)


# --- Atomic write helper ---


def _atomic_write(path: Path, data: dict) -> None:
    """Write JSON atomically via tempfile + os.replace."""
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp = tempfile.mkstemp(dir=str(path.parent), suffix=".tmp", prefix=".state-")
    try:
        with os.fdopen(fd, "w") as f:
            json.dump(data, f, indent=2)
            f.write("\n")
        os.replace(tmp, str(path))
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def _load_json(path: Path) -> dict:
    """Load JSON file, returning empty dict if missing or corrupt."""
    if not path.exists():
        return {}
    try:
        with open(path) as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return {}


def _now_iso() -> str:
    """Return current UTC timestamp as ISO 8601 string."""
    return datetime.now(timezone.utc).isoformat()


# --- Store base ---


class _BaseStore:
    """Base class for JSON-backed stores with load/save/dirty tracking."""

    def __init__(self, path: Path, schema_version: int = 1):
        self.path = path
        self.schema_version = schema_version
        self._data: dict = {}
        self._loaded = False

    def load(self) -> None:
        raw = _load_json(self.path)
        if raw.get("version") == self.schema_version:
            self._data = raw
        else:
            self._data = {"version": self.schema_version}
        self._loaded = True

    def save(self) -> None:
        self._data["version"] = self.schema_version
        self._data["updated_at"] = _now_iso()
        _atomic_write(self.path, self._data)

    def _ensure_loaded(self) -> None:
        if not self._loaded:
            self.load()


# --- ManifestStore ---


class ManifestStore(_BaseStore):
    """Tracks manifest file fingerprints for change detection."""

    def __init__(self, path: Path):
        super().__init__(path, schema_version=1)

    @property
    def _manifests(self) -> dict:
        return self._data.setdefault("manifests", {})

    def get(self, rel_path: str) -> dict | None:
        """Lookup a manifest fingerprint by relative path."""
        self._ensure_loaded()
        return self._manifests.get(rel_path)

    def upsert(self, rel_path: str, *, sha256: str, workspace: str) -> None:
        """Add or update a manifest fingerprint."""
        self._ensure_loaded()
        now = _now_iso()
        existing = self._manifests.get(rel_path)
        self._manifests[rel_path] = {
            "path": rel_path,
            "sha256": sha256,
            "last_seen": now,
            "workspace": workspace,
            "first_seen": existing["first_seen"] if existing else now,
        }

    def compare(self, current: dict[str, str]) -> ManifestDiff:
        """Compare current fingerprints {rel_path: sha256} against stored.

        Returns ManifestDiff with changed, added, removed, unchanged lists.
        """
        self._ensure_loaded()
        stored = self._manifests
        changed, added, removed, unchanged = [], [], [], []

        for path, sha in current.items():
            if path not in stored:
                added.append(path)
            elif stored[path]["sha256"] != sha:
                changed.append(path)
            else:
                unchanged.append(path)

        for path in stored:
            if path not in current:
                removed.append(path)

        return ManifestDiff(
            changed=changed, added=added, removed=removed, unchanged=unchanged
        )

    @staticmethod
    def fingerprint_file(filepath: Path) -> str:
        """Compute SHA256 hex digest of a file's contents."""
        h = hashlib.sha256()
        with open(filepath, "rb") as f:
            for chunk in iter(lambda: f.read(8192), b""):
                h.update(chunk)
        return h.hexdigest()


# --- InventoryStore ---


class InventoryStore(_BaseStore):
    """Tracks normalized dependency inventory with lifecycle states."""

    def __init__(self, path: Path):
        super().__init__(path, schema_version=1)

    @property
    def _deps(self) -> dict:
        return self._data.setdefault("dependencies", {})

    @staticmethod
    def _key(language: str, name: str) -> str:
        return f"{language}:{name}"

    def get(self, language: str, name: str) -> dict | None:
        """Lookup a dependency by language and name."""
        self._ensure_loaded()
        return self._deps.get(self._key(language, name))

    def upsert(self, dep: dict) -> None:
        """Add or update a single dependency from detect-deps output."""
        self._ensure_loaded()
        key = self._key(dep["language"], dep["name"])
        now = _now_iso()
        existing = self._deps.get(key)
        self._deps[key] = {
            "name": dep["name"],
            "language": dep["language"],
            "version": dep.get("version", ""),
            "dev": dep.get("dev", False),
            "sources": dep.get("sources", []),
            "state": existing["state"] if existing else "discovered",
            "first_seen": existing["first_seen"] if existing else now,
            "last_seen": now,
            "state_changed_at": existing.get("state_changed_at", now)
            if existing
            else now,
        }

    def set_state(self, language: str, name: str, state: str) -> None:
        """Transition a dependency to a new lifecycle state."""
        if state not in LIFECYCLE_STATES:
            raise ValueError(
                f"Invalid state: {state!r}. Must be one of {LIFECYCLE_STATES}"
            )
        self._ensure_loaded()
        key = self._key(language, name)
        dep = self._deps.get(key)
        if dep is None:
            raise KeyError(f"Dependency not found: {key}")
        dep["state"] = state
        dep["state_changed_at"] = _now_iso()

    def bulk_upsert(self, deps: list[dict]) -> InventoryDiff:
        """Merge a full detection result into stored inventory.

        Deps not in the new list are marked removed (state unchanged but
        tracked in diff). Returns InventoryDiff.
        """
        self._ensure_loaded()
        current_keys = set()
        added, changed, unchanged = [], [], []

        for dep in deps:
            key = self._key(dep["language"], dep["name"])
            current_keys.add(key)
            existing = self._deps.get(key)

            if existing is None:
                added.append(key)
            elif existing.get("version") != dep.get("version", ""):
                changed.append(key)
            else:
                unchanged.append(key)

            self.upsert(dep)

        removed = [k for k in self._deps if k not in current_keys]

        return InventoryDiff(
            added=added, changed=changed, removed=removed, unchanged=unchanged
        )

    def by_state(self, state: str) -> list[dict]:
        """Return all dependencies in a given lifecycle state."""
        self._ensure_loaded()
        return [d for d in self._deps.values() if d.get("state") == state]

    def all_deps(self) -> list[dict]:
        """Return all dependencies."""
        self._ensure_loaded()
        return list(self._deps.values())


# --- SourceCacheStore ---


class SourceCacheStore(_BaseStore):
    """Caches source resolution results per dependency/version."""

    DEFAULT_TTL = 604800  # 7 days

    def __init__(self, path: Path):
        super().__init__(path, schema_version=1)

    @property
    def _entries(self) -> dict:
        return self._data.setdefault("entries", {})

    @staticmethod
    def _key(language: str, name: str, version: str) -> str:
        return f"{language}:{name}:{version}"

    def get(self, language: str, name: str, version: str) -> dict | None:
        """Lookup a cached source resolution result."""
        self._ensure_loaded()
        return self._entries.get(self._key(language, name, version))

    def is_fresh(
        self,
        language: str,
        name: str,
        version: str,
        ttl_seconds: int | None = None,
    ) -> bool:
        """Check if a cached entry exists and is within TTL."""
        self._ensure_loaded()
        entry = self.get(language, name, version)
        if entry is None:
            return False
        if entry.get("errors"):
            return False
        fetch_ts = entry.get("fetch_timestamp")
        if not fetch_ts:
            return False
        ttl = ttl_seconds if ttl_seconds is not None else self.DEFAULT_TTL
        try:
            fetched = datetime.fromisoformat(fetch_ts)
            age = (datetime.now(timezone.utc) - fetched).total_seconds()
            return age < ttl
        except (ValueError, TypeError):
            return False

    def upsert(self, entry: dict) -> None:
        """Add or update a source cache entry."""
        self._ensure_loaded()
        key = self._key(entry["language"], entry["name"], entry["version"])
        self._entries[key] = entry

    def invalidate_by_version(
        self,
        language: str,
        name: str,
        old_version: str,
        new_version: str,
    ) -> bool:
        """Invalidate cache for a dep whose version changed.

        Removes the old-version entry. Returns True if an entry was removed.
        """
        self._ensure_loaded()
        old_key = self._key(language, name, old_version)
        if old_key in self._entries:
            del self._entries[old_key]
            return True
        return False

    def stats(self, ttl_seconds: int | None = None) -> CacheStats:
        """Compute cache statistics: hits (fresh), stale (expired), misses not counted here."""
        self._ensure_loaded()
        ttl = ttl_seconds if ttl_seconds is not None else self.DEFAULT_TTL
        hits = 0
        stale = 0
        for entry in self._entries.values():
            if entry.get("errors"):
                stale += 1
                continue
            fetch_ts = entry.get("fetch_timestamp")
            if not fetch_ts:
                stale += 1
                continue
            try:
                fetched = datetime.fromisoformat(fetch_ts)
                age = (datetime.now(timezone.utc) - fetched).total_seconds()
                if age < ttl:
                    hits += 1
                else:
                    stale += 1
            except (ValueError, TypeError):
                stale += 1

        return CacheStats(hits=hits, misses=0, stale=stale, total=len(self._entries))

    def all_entries(self) -> list[dict]:
        """Return all cache entries."""
        self._ensure_loaded()
        return list(self._entries.values())


# --- RefreshLog ---


class RefreshLog(_BaseStore):
    """Append-only log of cache invalidation signals."""

    MAX_ENTRIES = 200

    def __init__(self, path: Path):
        super().__init__(path, schema_version=1)

    @property
    def _signals(self) -> list:
        return self._data.setdefault("signals", [])

    def record(self, signal_type: str, target: str, detail: str) -> None:
        """Record an invalidation signal."""
        self._ensure_loaded()
        self._signals.append(
            {
                "timestamp": _now_iso(),
                "type": signal_type,
                "target": target,
                "detail": detail,
            }
        )
        # Trim to max entries
        if len(self._signals) > self.MAX_ENTRIES:
            self._data["signals"] = self._signals[-self.MAX_ENTRIES :]

    def recent(self, n: int = 20) -> list[dict]:
        """Return the N most recent signals."""
        self._ensure_loaded()
        return self._signals[-n:]


# --- StateManager facade ---


class StateManager:
    """Facade for all Whetstone state operations.

    Usage:
        sm = StateManager(Path("/path/to/project"))
        sm.manifests.load()
        sm.manifests.upsert("pyproject.toml", sha256="abc", workspace="root")
        sm.manifests.save()
    """

    def __init__(self, project_dir: Path):
        self.project_dir = Path(project_dir).resolve()
        self.state_dir = self.project_dir / "whetstone" / ".state"
        self.manifests = ManifestStore(self.state_dir / "manifests.json")
        self.inventory = InventoryStore(self.state_dir / "inventory.json")
        self.cache = SourceCacheStore(self.state_dir / "source-cache.json")
        self.refresh_log = RefreshLog(self.state_dir / "refresh-log.json")

    def ensure_dir(self) -> None:
        """Create .state/ directory if needed."""
        self.state_dir.mkdir(parents=True, exist_ok=True)

    def load_all(self) -> None:
        """Load all stores from disk."""
        self.manifests.load()
        self.inventory.load()
        self.cache.load()
        self.refresh_log.load()

    def save_all(self) -> None:
        """Save all stores to disk."""
        self.ensure_dir()
        self.manifests.save()
        self.inventory.save()
        self.cache.save()
        self.refresh_log.save()


if __name__ == "__main__":
    import sys

    # Quick smoke test / info dump
    if len(sys.argv) > 1:
        project_dir = Path(sys.argv[1])
    else:
        project_dir = Path(".")

    sm = StateManager(project_dir)
    sm.load_all()

    info = {
        "state_dir": str(sm.state_dir),
        "manifests_count": len(sm.manifests._manifests),
        "inventory_count": len(sm.inventory._deps),
        "cache_entries": len(sm.cache._entries),
        "refresh_signals": len(sm.refresh_log._signals),
        "cache_stats": sm.cache.stats()._asdict(),
    }
    json.dump(info, sys.stdout, indent=2)
    print()
