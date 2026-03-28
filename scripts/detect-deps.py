#!/usr/bin/env python3
"""Detect project dependencies from manifest files.

Recursively discovers manifest files (pyproject.toml, requirements.txt,
package.json, Cargo.toml) across the project tree, including monorepo
workspaces. Deduplicates by (name, language) and tracks which subdirectories
each dependency appears in.

Outputs structured JSON to stdout with dependency name, version, language,
dev flag, and source locations.

Usage:
    python3 scripts/detect-deps.py [--project-dir DIR] [--check-drift]
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

# Directories to skip when recursively searching for manifests.
# These are never project source — they're build artifacts, caches, or VCS internals.
SKIP_DIRS = frozenset(
    {
        "node_modules",
        ".git",
        ".hg",
        ".svn",
        "__pycache__",
        ".mypy_cache",
        ".ruff_cache",
        ".pytest_cache",
        ".tox",
        ".nox",
        ".venv",
        "venv",
        "env",
        ".env",
        "target",  # Rust build output
        "dist",
        "build",
        ".next",
        ".nuxt",
        ".turbo",
        ".vercel",
        ".output",
        "coverage",
        ".whetstone",
        "whetstone",
        # Fixture / vendored / example directories — not project source
        "tests/fixtures",
        "fixtures",
        "examples",
        "vendor",
        "third_party",
        "third-party",
    }
)

# Try tomllib (3.11+), then tomli, then fall back to a minimal parser
try:
    import tomllib
except ModuleNotFoundError:
    try:
        import tomli as tomllib  # type: ignore[no-redef]
    except ModuleNotFoundError:
        tomllib = None  # type: ignore[assignment]


def _minimal_toml_load(filepath: Path) -> dict:
    """Minimal TOML parser for pyproject.toml and Cargo.toml.

    Handles the subset of TOML used in dependency manifests:
    - String values, arrays, inline tables
    - Nested table headers [section.subsection]

    Not a full TOML parser — just enough for dependency extraction.
    Falls back to this only when tomllib/tomli are unavailable.
    """
    import ast as _ast  # noqa: F401

    text = filepath.read_text()
    result: dict = {}
    current_section: list[str] = []

    for line in text.split("\n"):
        stripped = line.strip()

        # Skip empty lines and comments
        if not stripped or stripped.startswith("#"):
            continue

        # Table headers: [section] or [section.subsection]
        header_match = re.match(r"^\[([^\]]+)\]$", stripped)
        if header_match:
            current_section = header_match.group(1).split(".")
            # Ensure nested dict exists
            d = result
            for key in current_section:
                d = d.setdefault(key.strip(), {})
            continue

        # Key-value pairs
        kv_match = re.match(r"^([a-zA-Z0-9_-]+)\s*=\s*(.+)$", stripped)
        if kv_match:
            key = kv_match.group(1).strip()
            value_str = kv_match.group(2).strip()

            # Navigate to current section
            d = result
            for section_key in current_section:
                d = d.setdefault(section_key, {})

            # Parse value
            d[key] = _parse_toml_value(value_str, text, stripped)

    return result


def _parse_toml_value(value_str: str, full_text: str = "", current_line: str = ""):
    """Parse a TOML value string into a Python object."""
    value_str = value_str.strip()

    # String (quoted)
    if value_str.startswith('"') and value_str.endswith('"'):
        return value_str[1:-1]
    if value_str.startswith("'") and value_str.endswith("'"):
        return value_str[1:-1]

    # Boolean
    if value_str == "true":
        return True
    if value_str == "false":
        return False

    # Integer
    try:
        return int(value_str)
    except ValueError:
        pass

    # Float
    try:
        return float(value_str)
    except ValueError:
        pass

    # Array (simple single-line)
    if value_str.startswith("["):
        # Find the matching bracket, handling multiline
        bracket_content = value_str
        if value_str.count("[") > value_str.count("]"):
            # Multiline array — find in full text
            idx = full_text.find(current_line)
            if idx >= 0:
                rest = full_text[idx:]
                depth = 0
                end = 0
                for i, ch in enumerate(rest):
                    if ch == "[":
                        depth += 1
                    elif ch == "]":
                        depth -= 1
                        if depth == 0:
                            end = i + 1
                            break
                if end > 0:
                    bracket_content = rest[:end]
                    # Extract just the array part
                    eq_idx = bracket_content.find("=")
                    if eq_idx >= 0:
                        bracket_content = bracket_content[eq_idx + 1 :].strip()

        # Parse simple arrays
        inner = (
            bracket_content[1:-1].strip()
            if bracket_content.endswith("]")
            else bracket_content[1:].strip()
        )
        if not inner:
            return []
        items = []
        for item in _split_array_items(inner):
            item = item.strip().strip(",").strip()
            if item:
                items.append(_parse_toml_value(item))
        return items

    # Inline table { key = value, ... }
    if value_str.startswith("{") and value_str.endswith("}"):
        inner = value_str[1:-1].strip()
        result = {}
        for pair in inner.split(","):
            pair = pair.strip()
            if "=" in pair:
                k, v = pair.split("=", 1)
                result[k.strip()] = _parse_toml_value(v.strip())
        return result

    # Bare string (shouldn't happen in valid TOML, but handle gracefully)
    return value_str


def _split_array_items(s: str) -> list[str]:
    """Split array items respecting quoted strings."""
    items = []
    current = ""
    in_string = False
    quote_char = ""

    for ch in s:
        if ch in ('"', "'") and not in_string:
            in_string = True
            quote_char = ch
            current += ch
        elif ch == quote_char and in_string:
            in_string = False
            current += ch
        elif ch == "," and not in_string:
            if current.strip():
                items.append(current.strip())
            current = ""
        elif ch == "\n" and not in_string:
            # Skip newlines in arrays
            continue
        elif ch == "#" and not in_string:
            # Skip comments
            break
        else:
            current += ch

    if current.strip():
        items.append(current.strip())
    return items


def _load_toml(filepath: Path) -> dict:
    """Load a TOML file using the best available parser."""
    if tomllib is not None:
        with open(filepath, "rb") as f:
            return tomllib.load(f)
    return _minimal_toml_load(filepath)


def _load_discovery_config(project_dir: Path) -> dict:
    """Load discovery include/exclude from whetstone.yaml if present."""
    config_path = project_dir / "whetstone" / "whetstone.yaml"
    if not config_path.exists():
        config_path = project_dir / "whetstone.yaml"
    if not config_path.exists():
        return {}

    try:
        import yaml  # type: ignore[import-untyped]

        with open(config_path) as f:
            data = yaml.safe_load(f) or {}
        return data.get("discovery", {}) or {}
    except Exception:
        return {}


def _should_skip_dir(
    dir_name: str,
    rel_path: str,
    skip_dirs: frozenset[str],
    extra_excludes: list[str],
    extra_includes: list[str],
) -> bool:
    """Determine whether a directory should be pruned during manifest walk.

    Checks dir name against SKIP_DIRS, then rel_path against multi-segment
    exclude patterns (e.g. 'tests/fixtures'). Include patterns override both.
    """
    # Check if explicitly included — include overrides exclude
    for inc in extra_includes:
        if rel_path == inc or rel_path.startswith(inc + "/") or dir_name == inc:
            return False

    # Check single-segment SKIP_DIRS
    if dir_name in skip_dirs:
        return True

    # Check multi-segment exclude patterns against relative path
    for excl in extra_excludes:
        if "/" in excl:
            # Multi-segment: match as prefix or exact
            if rel_path == excl or rel_path.startswith(excl + "/"):
                return True
        else:
            if dir_name == excl:
                return True

    return False


def find_manifests(
    project_dir: Path,
    extra_excludes: list[str] | None = None,
    extra_includes: list[str] | None = None,
) -> list[tuple[Path, str]]:
    """Recursively find all manifest files under project_dir.

    Returns list of (absolute_path, relative_source_dir) tuples.
    Skips directories in SKIP_DIRS (node_modules, .git, target, etc.)
    plus any extra_excludes patterns. extra_includes override excludes.

    The relative_source_dir is the directory containing the manifest,
    relative to project_dir. For the root it's ".".
    """
    manifest_names = {
        "pyproject.toml",
        "requirements.txt",
        "package.json",
        "Cargo.toml",
    }
    results: list[tuple[Path, str]] = []
    excludes = extra_excludes or []
    includes = extra_includes or []

    # Split SKIP_DIRS into single-segment (for fast name check) and
    # multi-segment (for rel-path check)
    single_seg_skips = frozenset(s for s in SKIP_DIRS if "/" not in s)
    multi_seg_skips = [s for s in SKIP_DIRS if "/" in s]
    all_excludes = multi_seg_skips + [e for e in excludes if "/" in e]
    single_excludes = frozenset(e for e in excludes if "/" not in e)
    combined_singles = single_seg_skips | single_excludes

    for dirpath, dirnames, filenames in os.walk(project_dir):
        rel_dir = os.path.relpath(dirpath, project_dir)
        if rel_dir == ".":
            rel_dir_str = ""
        else:
            rel_dir_str = rel_dir

        # Prune skipped directories in-place
        new_dirnames = []
        for d in dirnames:
            child_rel = f"{rel_dir_str}/{d}" if rel_dir_str else d
            if d.startswith(".") and d not in (".whetstone", ".env"):
                continue  # skip hidden dirs
            if _should_skip_dir(d, child_rel, combined_singles, all_excludes, includes):
                continue
            new_dirnames.append(d)
        dirnames[:] = sorted(new_dirnames)

        for fname in filenames:
            if fname in manifest_names:
                full_path = Path(dirpath) / fname
                if rel_dir_str == "" or rel_dir_str == ".":
                    source = "root"
                else:
                    source = rel_dir_str
                results.append((full_path, source))

    return results


def parse_pyproject_toml(filepath: Path, source: str = "root") -> list[dict]:
    """Parse dependencies from pyproject.toml (PEP 621 + Poetry)."""
    data = _load_toml(filepath)

    deps: list[dict] = []

    # Collect workspace-internal deps to filter out.
    # uv: [tool.uv.sources] with { workspace = true }
    # Poetry: { path = "..." } deps
    workspace_deps: set[str] = set()
    uv_sources = data.get("tool", {}).get("uv", {}).get("sources", {})
    for dep_name, spec in uv_sources.items():
        if isinstance(spec, dict) and spec.get("workspace"):
            workspace_deps.add(dep_name.lower())

    # PEP 621: [project].dependencies
    project = data.get("project", {})
    for dep_str in project.get("dependencies", []):
        name, version = _parse_pep508(dep_str)
        if name.lower() in workspace_deps:
            continue
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "python",
                "dev": False,
                "source": source,
            }
        )

    # PEP 621: [project.optional-dependencies]
    for group_deps in project.get("optional-dependencies", {}).values():
        for dep_str in group_deps:
            name, version = _parse_pep508(dep_str)
            deps.append(
                {
                    "name": name,
                    "version": version,
                    "language": "python",
                    "dev": True,
                    "source": source,
                }
            )

    # Poetry: [tool.poetry.dependencies]
    poetry = data.get("tool", {}).get("poetry", {})
    for name, spec in poetry.get("dependencies", {}).items():
        if name.lower() == "python":
            continue
        version = _poetry_version(spec)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "python",
                "dev": False,
                "source": source,
            }
        )

    # Poetry: [tool.poetry.dev-dependencies]
    for name, spec in poetry.get("dev-dependencies", {}).items():
        version = _poetry_version(spec)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "python",
                "dev": True,
                "source": source,
            }
        )

    # Poetry: [tool.poetry.group.*.dependencies]
    for group_name, group_data in poetry.get("group", {}).items():
        for name, spec in group_data.get("dependencies", {}).items():
            version = _poetry_version(spec)
            deps.append(
                {
                    "name": name,
                    "version": version,
                    "language": "python",
                    "dev": group_name != "main",
                    "source": source,
                }
            )

    return deps


def _parse_pep508(dep_str: str) -> tuple[str, str]:
    """Parse a PEP 508 dependency string into (name, version_spec)."""
    # Handle extras: package[extra1,extra2]>=1.0
    match = re.match(r"^([a-zA-Z0-9_.-]+)(?:\[[^\]]*\])?\s*(.*)", dep_str.strip())
    if match:
        name = match.group(1).strip()
        version = match.group(2).strip().rstrip(";").strip()
        # Remove environment markers after semicolon
        if ";" in version:
            version = version.split(";")[0].strip()
        return name, version or "*"
    return dep_str.strip(), "*"


def _poetry_version(spec: str | dict) -> str:
    """Extract version from Poetry dependency spec."""
    if isinstance(spec, str):
        return spec
    if isinstance(spec, dict):
        return spec.get("version", "*")
    return "*"


def parse_requirements_txt(filepath: Path, source: str = "root") -> list[dict]:
    """Parse dependencies from requirements.txt."""
    deps: list[dict] = []

    with open(filepath) as f:
        for line in f:
            line = line.strip()
            # Skip empty lines, comments, flags
            if not line or line.startswith("#") or line.startswith("-"):
                continue

            # Parse name and version spec
            match = re.match(r"^([a-zA-Z0-9_.-]+)\s*([><=!~]+.+)?", line)
            if match:
                name = match.group(1)
                version = match.group(2) or "*"
                # Remove environment markers
                if ";" in version:
                    version = version.split(";")[0].strip()
                deps.append(
                    {
                        "name": name,
                        "version": version.strip(),
                        "language": "python",
                        "dev": False,
                        "source": source,
                    }
                )

    return deps


def parse_package_json(filepath: Path, source: str = "root") -> list[dict]:
    """Parse dependencies from package.json."""
    with open(filepath) as f:
        data = json.load(f)

    deps: list[dict] = []

    for name, version in data.get("dependencies", {}).items():
        # Skip workspace references (internal packages, not real external deps)
        if _is_workspace_ref(version):
            continue
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "typescript",
                "dev": False,
                "source": source,
            }
        )

    for name, version in data.get("devDependencies", {}).items():
        if _is_workspace_ref(version):
            continue
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "typescript",
                "dev": True,
                "source": source,
            }
        )

    return deps


def _is_workspace_ref(version: str) -> bool:
    """Check if a version string is an internal workspace reference."""
    if not isinstance(version, str):
        return False
    v = version.strip().lower()
    return v.startswith("workspace:") or v.startswith("link:") or v.startswith("file:")


def parse_cargo_toml(filepath: Path, source: str = "root") -> list[dict]:
    """Parse dependencies from Cargo.toml."""
    data = _load_toml(filepath)

    deps: list[dict] = []

    for name, spec in data.get("dependencies", {}).items():
        # Skip path-only dependencies (workspace-internal)
        if isinstance(spec, dict) and "path" in spec and "version" not in spec:
            continue
        version = _cargo_version(spec)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "rust",
                "dev": False,
                "source": source,
            }
        )

    for name, spec in data.get("dev-dependencies", {}).items():
        if isinstance(spec, dict) and "path" in spec and "version" not in spec:
            continue
        version = _cargo_version(spec)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "rust",
                "dev": True,
                "source": source,
            }
        )

    return deps


def _cargo_version(spec: str | dict) -> str:
    """Extract version from Cargo dependency spec."""
    if isinstance(spec, str):
        return spec
    if isinstance(spec, dict):
        return spec.get("version", "*")
    return "*"


def check_drift(deps: list[dict], project_dir: Path) -> dict:
    """Compare current deps against stored versions in whetstone rules.

    Returns a normalized dict:
        {"changed": [...], "count": N, "checked": M}
    """
    empty: dict = {"changed": [], "count": 0, "checked": 0}
    rules_dir = project_dir / "whetstone" / "rules"
    if not rules_dir.exists():
        return empty

    try:
        stored_versions: dict[str, str] = {}
        for yaml_file in rules_dir.rglob("*.yaml"):
            text = yaml_file.read_text()
            name_match = re.search(r"^\s*name:\s*(.+)$", text, re.MULTILINE)
            ver_match = re.search(
                r"^\s*version:\s*[\"']?(.+?)[\"']?\s*$", text, re.MULTILINE
            )
            if name_match and ver_match:
                stored_versions[name_match.group(1).strip()] = ver_match.group(
                    1
                ).strip()

        changed = []
        for dep in deps:
            stored_ver = stored_versions.get(dep["name"])
            if stored_ver and stored_ver != dep["version"]:
                changed.append(
                    {
                        "name": dep["name"],
                        "language": dep["language"],
                        "old_version": stored_ver,
                        "new_version": dep["version"],
                    }
                )

        return {
            "changed": changed,
            "count": len(changed),
            "checked": len(stored_versions),
        }
    except Exception:
        return empty


def detect_deps(
    project_dir: Path,
    do_check_drift: bool = False,
    cli_excludes: list[str] | None = None,
    cli_includes: list[str] | None = None,
) -> dict:
    """Main detection logic. Returns structured JSON-ready dict.

    Recursively discovers all manifest files under project_dir,
    parses each, deduplicates by (name, language, dev), and merges
    source locations.
    """
    all_deps: list[dict] = []
    warnings: list[str] = []
    manifests_found: list[str] = []

    # Load discovery config from whetstone.yaml
    discovery_cfg = _load_discovery_config(project_dir)
    cfg_excludes = discovery_cfg.get("exclude", []) or []
    cfg_includes = discovery_cfg.get("include", []) or []

    # Merge CLI and config: CLI values extend config values
    merged_excludes = list(cfg_excludes) + (cli_excludes or [])
    merged_includes = list(cfg_includes) + (cli_includes or [])

    # Map filename to parser
    parsers = {
        "pyproject.toml": parse_pyproject_toml,
        "requirements.txt": parse_requirements_txt,
        "package.json": parse_package_json,
        "Cargo.toml": parse_cargo_toml,
    }

    # Recursively find all manifest files
    manifest_files = find_manifests(
        project_dir,
        extra_excludes=merged_excludes,
        extra_includes=merged_includes,
    )

    if not manifest_files:
        effective_excluded = sorted(set(list(SKIP_DIRS) + merged_excludes))
        result: dict = {
            "languages": [],
            "dependencies": [],
            "manifests": [],
            "discovery": {
                "excluded": effective_excluded,
                "included": merged_includes,
                "monorepo": False,
                "workspaces": [],
            },
            "error": "No manifest files found",
            "next_command": "Ensure project has pyproject.toml, package.json, or Cargo.toml",
        }
        return result

    for filepath, source in manifest_files:
        filename = filepath.name
        parser = parsers.get(filename)
        if parser is None:
            continue
        rel_path = os.path.relpath(filepath, project_dir)
        manifests_found.append(rel_path)
        try:
            deps = parser(filepath, source)
            all_deps.extend(deps)
        except Exception as e:
            warnings.append(f"Error parsing {rel_path}: {e}")

    # Deduplicate by (name, language, dev).
    # When the same dep appears in multiple workspaces, merge sources and
    # keep the most specific (non-wildcard) version.
    merged: dict[tuple[str, str, bool], dict] = {}
    for dep in all_deps:
        key = (dep["name"], dep["language"], dep["dev"])
        if key not in merged:
            merged[key] = {
                "name": dep["name"],
                "version": dep["version"],
                "language": dep["language"],
                "dev": dep["dev"],
                "sources": [dep["source"]],
            }
        else:
            entry = merged[key]
            if dep["source"] not in entry["sources"]:
                entry["sources"].append(dep["source"])
            # Prefer a more specific version over "*"
            if entry["version"] == "*" and dep["version"] != "*":
                entry["version"] = dep["version"]

    unique_deps = sorted(merged.values(), key=lambda d: (d["dev"], d["name"]))
    languages = sorted(set(d["language"] for d in unique_deps))

    # Build per-language counts so agents can present accurate summaries
    runtime_deps = [d for d in unique_deps if not d["dev"]]
    dev_deps = [d for d in unique_deps if d["dev"]]
    counts: dict[str, dict[str, int]] = {"total": {}, "runtime": {}, "dev": {}}
    for lang in languages:
        counts["total"][lang] = sum(1 for d in unique_deps if d["language"] == lang)
        counts["runtime"][lang] = sum(1 for d in runtime_deps if d["language"] == lang)
        counts["dev"][lang] = sum(1 for d in dev_deps if d["language"] == lang)
    counts["total"]["_all"] = len(unique_deps)
    counts["runtime"]["_all"] = len(runtime_deps)
    counts["dev"]["_all"] = len(dev_deps)

    # Monorepo detection: >1 manifest in different directories
    manifest_dirs = sorted(set(os.path.dirname(m) for m in manifests_found))
    is_monorepo = len(manifest_dirs) > 1
    if is_monorepo:
        print(
            f"Monorepo detected: {len(manifest_dirs)} workspaces found",
            file=sys.stderr,
        )

    # Build effective excluded list (deduped, sorted)
    effective_excluded = sorted(set(list(SKIP_DIRS) + merged_excludes))

    result = {
        "languages": languages,
        "counts": counts,
        "dependencies": unique_deps,
        "manifests": manifests_found,
        "discovery": {
            "excluded": effective_excluded,
            "included": merged_includes,
            "monorepo": is_monorepo,
            "workspaces": manifest_dirs if is_monorepo else [],
        },
    }

    if warnings:
        result["warnings"] = warnings

    if do_check_drift:
        drift = check_drift(unique_deps, project_dir)
        result["drift"] = drift
        if drift.get("count", 0) > 0:
            result["next_command"] = (
                "Resolve changed sources: python3 scripts/resolve-sources.py --changed-only"
            )
        else:
            result["next_command"] = "No drift detected. Rules are current."
    else:
        dep_names = ",".join(d["name"] for d in runtime_deps[:10])
        result["next_command"] = (
            f"Resolve docs: python3 scripts/resolve-sources.py --deps {dep_names}"
        )

    return result


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Detect project dependencies from manifest files."
    )
    parser.add_argument(
        "--project-dir",
        type=Path,
        default=Path("."),
        help="Root directory to search for manifest files (default: .)",
    )
    parser.add_argument(
        "--check-drift",
        action="store_true",
        help="Compare current deps against stored versions in whetstone rules",
    )
    parser.add_argument(
        "--changed-only",
        action="store_true",
        help="Only output dependencies that have drifted from stored versions",
    )
    parser.add_argument(
        "--exclude",
        type=str,
        default=None,
        help="Comma-separated directory patterns to exclude from discovery",
    )
    parser.add_argument(
        "--include",
        type=str,
        default=None,
        help="Comma-separated directory patterns to include even if normally skipped",
    )
    args = parser.parse_args()

    cli_excludes = [e.strip() for e in args.exclude.split(",") if e.strip()] if args.exclude else None
    cli_includes = [i.strip() for i in args.include.split(",") if i.strip()] if args.include else None

    try:
        do_drift = args.check_drift or args.changed_only
        result = detect_deps(
            args.project_dir,
            do_drift,
            cli_excludes=cli_excludes,
            cli_includes=cli_includes,
        )

        # --changed-only: filter to only drifted deps
        if args.changed_only and "drift" in result:
            changed = result["drift"].get("changed", [])
            drifted_names = {d["name"] for d in changed}
            if drifted_names:
                result["dependencies"] = [
                    d for d in result["dependencies"] if d["name"] in drifted_names
                ]
                result["next_command"] = (
                    "Resolve changed sources: python3 scripts/resolve-sources.py --changed-only"
                )
            else:
                result["dependencies"] = []
                result["next_command"] = "No changes detected. Rules are current."

        json.dump(result, sys.stdout, indent=2)
        sys.stdout.write("\n")
    except Exception as e:
        json.dump(
            {
                "error": str(e),
                "next_command": "Check project directory and manifest files",
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
