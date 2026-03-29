#!/usr/bin/env python3
"""Resolve documentation URLs and fetch content for dependencies.

Takes dependency list (JSON from detect-deps.py), resolves docs URLs via
package registry APIs (PyPI, npm, crates.io), probes for llms.txt, fetches
content, and outputs structured JSON with source content and content hashes.

Usage:
    python3 scripts/detect-deps.py | python3 scripts/resolve-sources.py
    python3 scripts/resolve-sources.py --input deps.json
    python3 scripts/resolve-sources.py --input deps.json --deps fastapi,pydantic
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import ssl
import sys
import threading
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path

# State management — optional import for caching/checkpointing
try:
    from state import StateManager
except ImportError:
    StateManager = None  # type: ignore[assignment,misc]

USER_AGENT = "whetstone/0.1.0 (https://github.com/whetstone)"
DEFAULT_TIMEOUT = 15


def _http_get(
    url: str,
    timeout: int = DEFAULT_TIMEOUT,
    expect_plain_text: bool = False,
) -> str | None:
    """Fetch URL content. Returns None on any error.

    If expect_plain_text is True, rejects responses with HTML content-type
    or content that looks like HTML (starts with <!DOCTYPE or <html).
    """
    try:
        req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
        ctx = ssl.create_default_context()
        with urllib.request.urlopen(req, timeout=timeout, context=ctx) as resp:
            if resp.status != 200:
                return None

            # Check Content-Type header for HTML when we expect plain text
            if expect_plain_text:
                content_type = resp.headers.get("Content-Type", "")
                if "text/html" in content_type or "application/xhtml" in content_type:
                    return None

            body = resp.read().decode("utf-8", errors="replace")

            # Secondary check: reject content that looks like HTML
            if expect_plain_text and body:
                stripped = body.lstrip()[:100].lower()
                if stripped.startswith("<!doctype") or stripped.startswith("<html"):
                    return None

            return body
    except Exception:
        pass
    return None


def _http_get_json(url: str, timeout: int = DEFAULT_TIMEOUT) -> dict | None:
    """Fetch URL and parse as JSON. Returns None on error."""
    body = _http_get(url, timeout)
    if body:
        try:
            return json.loads(body)
        except json.JSONDecodeError:
            pass
    return None


def _content_hash(content: str) -> str:
    """SHA-256 hash of content, prefixed with sha256:."""
    h = hashlib.sha256(content.encode("utf-8")).hexdigest()
    return f"sha256:{h}"


def _normalize_url(url: str) -> str:
    """Ensure URL has no trailing slash."""
    return url.rstrip("/")


def _probe_llms_txt(base_url: str, timeout: int) -> tuple[str | None, str | None, str]:
    """Probe for llms-full.txt and llms.txt at a base URL.

    Returns (content, url, source_type) or (None, None, "none").
    """
    base = _normalize_url(base_url)

    # Build candidate URLs — try multiple common path patterns
    candidates: list[tuple[str, str]] = []
    for suffix in ("", "/latest", "/stable", "/en/latest", "/en/stable"):
        root = base + suffix if suffix else base
        candidates.append((f"{root}/llms-full.txt", "llms_full_txt"))
        candidates.append((f"{root}/llms.txt", "llms_txt"))

    for url, source_type in candidates:
        content = _http_get(url, timeout, expect_plain_text=True)
        if content and len(content) > 50:  # Sanity check
            return content, url, source_type

    return None, None, "none"


# --- Registry resolvers ---


def _extract_release_metadata(
    registry: str, data: dict, name: str, version: str
) -> dict:
    """Extract latest version and release date from registry API response.

    Returns dict with latest_version, latest_release_date (ISO), and
    version_released_date (ISO) if available.
    """
    meta: dict[str, str | None] = {
        "latest_version": None,
        "latest_release_date": None,
    }

    try:
        if registry == "pypi":
            info = data.get("info", {})
            meta["latest_version"] = info.get("version")
            # Release date: look up the latest version's upload time
            releases = data.get("releases", {})
            latest_ver = meta["latest_version"]
            if latest_ver and latest_ver in releases:
                files = releases[latest_ver]
                if files:
                    # Take the earliest upload_time for this version
                    upload_time = files[0].get("upload_time")
                    if upload_time:
                        meta["latest_release_date"] = upload_time

        elif registry == "npm":
            dist_tags = data.get("dist-tags", {})
            meta["latest_version"] = dist_tags.get("latest")
            time_data = data.get("time", {})
            latest_ver = meta["latest_version"]
            if latest_ver and latest_ver in time_data:
                meta["latest_release_date"] = time_data[latest_ver]
            elif "modified" in time_data:
                meta["latest_release_date"] = time_data["modified"]

        elif registry == "crates_io":
            versions = data.get("versions", [])
            if versions:
                # First version in the list is the latest
                meta["latest_version"] = versions[0].get("num")
                meta["latest_release_date"] = versions[0].get("created_at")
    except Exception:
        pass  # Non-critical — metadata is best-effort

    return {k: v for k, v in meta.items() if v is not None}


def resolve_python(name: str, version: str, timeout: int) -> dict:
    """Resolve documentation for a Python package via PyPI."""
    api_url = f"https://pypi.org/pypi/{name}/json"
    data = _http_get_json(api_url, timeout)

    if not data:
        return {"error": f"PyPI lookup failed for {name}"}

    info = data.get("info", {})
    release_meta = _extract_release_metadata("pypi", data, name, version)

    # Extract docs URL from project_urls or home_page
    docs_url = None
    project_urls = info.get("project_urls") or {}
    for key in (
        "Documentation",
        "Docs",
        "documentation",
        "docs",
        "Homepage",
        "homepage",
        "Home",
        "home",
    ):
        if key in project_urls and project_urls[key]:
            docs_url = project_urls[key]
            break

    if not docs_url:
        docs_url = info.get("home_page")

    if not docs_url:
        # Try project URL
        docs_url = info.get("project_url")

    if not docs_url:
        return {"error": f"No documentation URL found for {name}", **release_meta}

    # Probe for llms.txt
    content, llms_url, source_type = _probe_llms_txt(docs_url, timeout)

    if content:
        return {
            "docs_url": docs_url,
            "llms_txt_url": llms_url,
            "source_type": source_type,
            "content": content,
            "content_hash": _content_hash(content),
            **release_meta,
        }

    # Fallback: just record the docs URL
    return {
        "docs_url": docs_url,
        "llms_txt_url": None,
        "source_type": "docs_url_only",
        "content": None,
        "content_hash": None,
        **release_meta,
    }


def resolve_typescript(name: str, version: str, timeout: int) -> dict:
    """Resolve documentation for an npm package."""
    api_url = f"https://registry.npmjs.org/{name}"
    data = _http_get_json(api_url, timeout)

    if not data:
        return {"error": f"npm lookup failed for {name}"}

    release_meta = _extract_release_metadata("npm", data, name, version)

    # Extract homepage
    docs_url = data.get("homepage")

    if not docs_url:
        # Try repository URL
        repo = data.get("repository")
        if isinstance(repo, dict):
            docs_url = repo.get("url", "")
            # Clean up git URLs
            docs_url = (
                docs_url.replace("git+", "")
                .replace("git://", "https://")
                .rstrip(".git")
            )
        elif isinstance(repo, str):
            docs_url = repo

    if not docs_url:
        return {"error": f"No documentation URL found for {name}", **release_meta}

    # Probe for llms.txt
    content, llms_url, source_type = _probe_llms_txt(docs_url, timeout)

    if content:
        return {
            "docs_url": docs_url,
            "llms_txt_url": llms_url,
            "source_type": source_type,
            "content": content,
            "content_hash": _content_hash(content),
            **release_meta,
        }

    return {
        "docs_url": docs_url,
        "llms_txt_url": None,
        "source_type": "docs_url_only",
        "content": None,
        "content_hash": None,
        **release_meta,
    }


def resolve_rust(name: str, version: str, timeout: int) -> dict:
    """Resolve documentation for a Rust crate via crates.io."""
    api_url = f"https://crates.io/api/v1/crates/{name}"
    data = _http_get_json(api_url, timeout)

    if not data:
        return {"error": f"crates.io lookup failed for {name}"}

    release_meta = _extract_release_metadata("crates_io", data, name, version)
    crate = data.get("crate", {})

    # Try multiple URL fields
    docs_url = (
        crate.get("documentation") or crate.get("homepage") or f"https://docs.rs/{name}"
    )

    # Probe for llms.txt at docs.rs
    docsrs_url = f"https://docs.rs/{name}/latest"
    content, llms_url, source_type = _probe_llms_txt(docsrs_url, timeout)

    if not content and docs_url:
        # Try the actual docs URL too
        content, llms_url, source_type = _probe_llms_txt(docs_url, timeout)

    if content:
        return {
            "docs_url": docs_url,
            "llms_txt_url": llms_url,
            "source_type": source_type,
            "content": content,
            "content_hash": _content_hash(content),
            **release_meta,
        }

    return {
        "docs_url": docs_url,
        "llms_txt_url": None,
        "source_type": "docs_url_only",
        "content": None,
        "content_hash": None,
        **release_meta,
    }


RESOLVERS = {
    "python": resolve_python,
    "typescript": resolve_typescript,
    "rust": resolve_rust,
}


def _compute_freshness(
    result: dict,
    stored_hash: str | None = None,
) -> dict:
    """Compute freshness metadata for a resolved source entry.

    Returns a dict with source_age_days, content_stale, and confidence.
    """
    freshness: dict[str, int | bool | str | None] = {
        "source_age_days": None,
        "content_stale": False,
        "confidence": "low",
    }

    # Compute source_age_days from latest_release_date
    release_date_str = result.get("latest_release_date")
    if release_date_str:
        try:
            # Handle multiple ISO formats (with/without timezone, with T or space)
            cleaned = release_date_str.replace("Z", "+00:00")
            # Try parsing with timezone
            try:
                release_dt = datetime.fromisoformat(cleaned)
            except ValueError:
                # Fallback: strip to date portion
                release_dt = datetime.fromisoformat(cleaned[:10])
            if release_dt.tzinfo is None:
                release_dt = release_dt.replace(tzinfo=timezone.utc)
            now = datetime.now(timezone.utc)
            freshness["source_age_days"] = (now - release_dt).days
        except Exception:
            pass

    # Determine confidence based on source_type
    source_type = result.get("source_type", "")
    if source_type in ("llms_full_txt", "llms_txt"):
        freshness["confidence"] = "high"
    elif source_type == "docs_url_only":
        freshness["confidence"] = "low"
    elif result.get("content"):
        freshness["confidence"] = "medium"
    else:
        freshness["confidence"] = "low"

    # Content staleness: compare current hash against stored hash
    current_hash = result.get("content_hash")
    if stored_hash and current_hash:
        freshness["content_stale"] = stored_hash != current_hash

    return freshness


def load_stored_hashes(project_dir: Path) -> dict[str, str]:
    """Load content hashes from existing rule YAML files."""
    hashes: dict[str, str] = {}
    rules_dir = project_dir / "whetstone" / "rules"
    if not rules_dir.exists():
        return hashes

    for yaml_file in rules_dir.rglob("*.yaml"):
        text = yaml_file.read_text()
        name_match = re.search(r"^\s*name:\s*(.+)$", text, re.MULTILINE)
        hash_match = re.search(r"^\s*content_hash:\s*(.+)$", text, re.MULTILINE)
        if name_match and hash_match:
            hashes[name_match.group(1).strip()] = hash_match.group(1).strip()

    return hashes


def _resolve_single_dep(
    dep: dict,
    stored_hash: str | None,
    timeout: int,
) -> dict:
    """Resolve a single dependency. Thread-safe.

    Returns a dict with either resolved source data or an error key.
    """
    name = dep["name"]
    language = dep["language"]
    version = dep.get("version", "*")

    resolver = RESOLVERS.get(language)
    if not resolver:
        return {
            "name": name,
            "language": language,
            "version": version,
            "error": f"Unsupported language: {language}",
        }

    try:
        result = resolver(name, version, timeout)
    except Exception as e:
        return {
            "name": name,
            "language": language,
            "version": version,
            "error": str(e),
        }

    if "error" in result:
        return {
            "name": name,
            "language": language,
            "version": version,
            "error": result["error"],
        }

    freshness = _compute_freshness(result, stored_hash=stored_hash)

    return {
        "name": name,
        "language": language,
        "version": version,
        **result,
        "freshness": freshness,
    }


def resolve_sources(
    deps_data: dict,
    filter_deps: set[str] | None = None,
    changed_only: bool = False,
    project_dir: Path = Path("."),
    timeout: int = DEFAULT_TIMEOUT,
    ttl: int = 604800,
    force_refresh: bool = False,
    resume: bool = False,
    retry_failed: bool = False,
    workers: int = 4,
) -> dict:
    """Resolve documentation sources for all dependencies.

    Supports caching, checkpointing, parallel resolution, and resume.
    """
    sources: list[dict] = []
    errors: list[dict] = []
    cache_counts = {"hit": 0, "miss": 0, "stale": 0}
    skipped_cached = 0

    # Always load stored hashes — needed for freshness.content_stale
    stored_hashes = load_stored_hashes(project_dir)

    # Initialize state manager for caching/checkpointing
    sm = None
    if StateManager is not None:
        sm = StateManager(project_dir)
        sm.ensure_dir()
        sm.cache.load()
        sm.inventory.load()
        sm.refresh_log.load()

    # Build work list: filter deps, check cache
    work_list: list[dict] = []

    def _cache_entry_reusable(entry: dict | None) -> bool:
        """Return True when a cached entry is complete enough to reuse.

        llms-backed entries must retain content so doctor --ready-only can hand
        extraction-ready sources back to the agent on cached reruns.
        """
        if not entry or entry.get("errors"):
            return False
        source_type = entry.get("source_type")
        if source_type in ("llms_full_txt", "llms_txt"):
            return bool(entry.get("content"))
        return True

    for dep in deps_data.get("dependencies", []):
        name = dep["name"]
        language = dep["language"]
        version = dep.get("version", "*")

        if filter_deps and name not in filter_deps:
            continue
        if dep.get("dev", False):
            continue

        # Resume: skip deps already in resolved/extraction_ready state
        if resume and sm:
            inv_entry = sm.inventory.get(language, name)
            if inv_entry and inv_entry.get("state") in (
                "resolved",
                "extraction_ready",
                "extracted",
                "approved",
            ):
                # Use cached source if available
                cached = sm.cache.get(language, name, version)
                if _cache_entry_reusable(cached):
                    sources.append(cached)
                    skipped_cached += 1
                    cache_counts["hit"] += 1
                    continue

        # Retry-failed: only process deps in failed state
        if retry_failed and sm:
            inv_entry = sm.inventory.get(language, name)
            if inv_entry and inv_entry.get("state") != "failed":
                continue

        # Check source cache (unless force-refresh)
        if not force_refresh and sm:
            cached = sm.cache.get(language, name, version)
            if _cache_entry_reusable(cached):
                if sm.cache.is_fresh(language, name, version, ttl_seconds=ttl):
                    # Cache hit
                    cache_counts["hit"] += 1
                    sources.append(cached)
                    skipped_cached += 1
                    continue
                else:
                    cache_counts["stale"] += 1
            elif cached and cached.get("errors"):
                cache_counts["stale"] += 1
            elif cached:
                # Incomplete cached entry (e.g. llms source without content)
                cache_counts["stale"] += 1
            else:
                cache_counts["miss"] += 1
        else:
            cache_counts["miss"] += 1

        work_list.append(dep)

    # Mark deps as resolving in inventory
    if sm:
        for dep in work_list:
            try:
                sm.inventory.set_state(dep["language"], dep["name"], "resolving")
            except KeyError:
                # Dep not in inventory yet — upsert it first
                sm.inventory.upsert(dep)
                sm.inventory.set_state(dep["language"], dep["name"], "resolving")
        sm.inventory.save()

    # Lock for thread-safe state writes
    state_lock = threading.Lock()
    completed = 0
    total = len(work_list)

    def _resolve_and_checkpoint(dep: dict) -> dict:
        nonlocal completed
        name = dep["name"]
        language = dep["language"]
        version = dep.get("version", "*")
        stored_hash = stored_hashes.get(name)

        result = _resolve_single_dep(dep, stored_hash, timeout)

        # Checkpoint to state
        if sm:
            with state_lock:
                if "error" in result:
                    try:
                        sm.inventory.set_state(language, name, "failed")
                    except KeyError:
                        pass
                else:
                    # Check content hash change
                    old_cached = sm.cache.get(language, name, version)
                    if (
                        old_cached
                        and old_cached.get("content_hash")
                        and result.get("content_hash")
                        and old_cached["content_hash"] != result["content_hash"]
                    ):
                        sm.refresh_log.record(
                            "content_hash_changed",
                            f"{language}:{name}",
                            "content changed on re-resolve",
                        )

                    # Cache the result
                    cache_entry = dict(result)
                    cache_entry["fetch_timestamp"] = datetime.now(
                        timezone.utc
                    ).isoformat()
                    cache_entry["ttl_seconds"] = ttl
                    sm.cache.upsert(cache_entry)

                    # Determine extraction readiness
                    source_type = result.get("source_type", "")
                    confidence = result.get("freshness", {}).get("confidence", "low")
                    has_content = bool(result.get("content"))

                    if (
                        source_type in ("llms_full_txt", "llms_txt")
                        and has_content
                        and confidence == "high"
                    ):
                        try:
                            sm.inventory.set_state(language, name, "extraction_ready")
                        except KeyError:
                            pass
                    else:
                        try:
                            sm.inventory.set_state(language, name, "resolved")
                        except KeyError:
                            pass

                sm.cache.save()
                sm.inventory.save()

        completed += 1
        print(
            f"  [{completed}/{total}] {dep['name']}: "
            f"{'error' if 'error' in result else result.get('source_type', 'ok')}",
            file=sys.stderr,
        )
        return result

    # Parallel resolution
    if work_list:
        print(f"Resolving {total} dependencies ({workers} workers)...", file=sys.stderr)
        effective_workers = min(workers, total)
        with ThreadPoolExecutor(max_workers=effective_workers) as executor:
            futures = {
                executor.submit(_resolve_and_checkpoint, dep): dep for dep in work_list
            }
            for future in as_completed(futures):
                result = future.result()

                # Changed-only filter
                if changed_only and result.get("content_hash"):
                    stored_hash = stored_hashes.get(result["name"])
                    if stored_hash and stored_hash == result["content_hash"]:
                        continue

                if "error" in result:
                    errors.append(
                        {
                            "name": result["name"],
                            "language": result["language"],
                            "error": result["error"],
                        }
                    )
                else:
                    sources.append(result)

    # Save final state
    if sm:
        sm.refresh_log.save()

    if sources:
        next_command = "Extract rules: agent applies extraction prompt to each source"
    else:
        next_command = (
            "No sources resolved. Provide manual docs URLs or check errors above."
        )

    return {
        "sources": sources,
        "errors": errors,
        "cache": cache_counts,
        "resolution_stats": {
            "total": total + skipped_cached,
            "resolved": len(sources),
            "failed": len(errors),
            "skipped_cached": skipped_cached,
            "workers": workers,
        },
        "next_command": next_command,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Resolve documentation URLs and fetch content for dependencies."
    )
    parser.add_argument(
        "--input",
        type=Path,
        help="JSON input file from detect-deps.py (default: read from stdin)",
    )
    parser.add_argument(
        "--deps",
        type=str,
        help="Comma-separated list of dependency names to resolve",
    )
    parser.add_argument(
        "--changed-only",
        action="store_true",
        help="Only resolve deps whose content has changed since last extraction",
    )
    parser.add_argument(
        "--project-dir",
        type=Path,
        default=Path("."),
        help="Project root directory (default: .)",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=DEFAULT_TIMEOUT,
        help=f"HTTP request timeout in seconds (default: {DEFAULT_TIMEOUT})",
    )
    parser.add_argument(
        "--ttl",
        type=int,
        default=604800,
        help="Cache TTL in seconds (default: 7 days / 604800s)",
    )
    parser.add_argument(
        "--force-refresh",
        action="store_true",
        help="Ignore cache and re-resolve all dependencies",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="Skip deps already resolved in state (resume interrupted run)",
    )
    parser.add_argument(
        "--retry-failed",
        action="store_true",
        help="Re-resolve only dependencies in failed state",
    )
    parser.add_argument(
        "--workers",
        type=int,
        default=4,
        help="Number of parallel resolution workers (default: 4)",
    )
    args = parser.parse_args()

    try:
        # Read input
        if args.input:
            with open(args.input) as f:
                deps_data = json.load(f)
        else:
            deps_data = json.load(sys.stdin)

        filter_deps = set(args.deps.split(",")) if args.deps else None

        result = resolve_sources(
            deps_data,
            filter_deps=filter_deps,
            changed_only=args.changed_only,
            project_dir=args.project_dir,
            timeout=args.timeout,
            ttl=args.ttl,
            force_refresh=args.force_refresh,
            resume=args.resume,
            retry_failed=args.retry_failed,
            workers=args.workers,
        )

        json.dump(result, sys.stdout, indent=2)
        sys.stdout.write("\n")

    except Exception as e:
        json.dump(
            {
                "error": str(e),
                "next_command": "Check input JSON format and network connectivity",
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
