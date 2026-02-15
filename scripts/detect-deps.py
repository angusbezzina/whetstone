#!/usr/bin/env python3
"""Detect project dependencies from manifest files.

Parses pyproject.toml, requirements.txt, package.json, and Cargo.toml.
Outputs structured JSON to stdout with dependency name, version, language,
dev flag, and source manifest.

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
    import ast as _ast

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


def parse_pyproject_toml(filepath: Path) -> list[dict]:
    """Parse dependencies from pyproject.toml (PEP 621 + Poetry)."""
    data = _load_toml(filepath)

    deps: list[dict] = []

    # PEP 621: [project].dependencies
    project = data.get("project", {})
    for dep_str in project.get("dependencies", []):
        name, version = _parse_pep508(dep_str)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "python",
                "dev": False,
                "manifest": "pyproject.toml",
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
                    "manifest": "pyproject.toml",
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
                "manifest": "pyproject.toml",
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
                "manifest": "pyproject.toml",
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
                    "manifest": "pyproject.toml",
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


def parse_requirements_txt(filepath: Path) -> list[dict]:
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
                        "manifest": "requirements.txt",
                    }
                )

    return deps


def parse_package_json(filepath: Path) -> list[dict]:
    """Parse dependencies from package.json."""
    with open(filepath) as f:
        data = json.load(f)

    deps: list[dict] = []

    for name, version in data.get("dependencies", {}).items():
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "typescript",
                "dev": False,
                "manifest": "package.json",
            }
        )

    for name, version in data.get("devDependencies", {}).items():
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "typescript",
                "dev": True,
                "manifest": "package.json",
            }
        )

    return deps


def parse_cargo_toml(filepath: Path) -> list[dict]:
    """Parse dependencies from Cargo.toml."""
    data = _load_toml(filepath)

    deps: list[dict] = []

    for name, spec in data.get("dependencies", {}).items():
        version = _cargo_version(spec)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "rust",
                "dev": False,
                "manifest": "Cargo.toml",
            }
        )

    for name, spec in data.get("dev-dependencies", {}).items():
        version = _cargo_version(spec)
        deps.append(
            {
                "name": name,
                "version": version,
                "language": "rust",
                "dev": True,
                "manifest": "Cargo.toml",
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


def check_drift(deps: list[dict], project_dir: Path) -> list[dict]:
    """Compare current deps against stored versions in whetstone.yaml."""
    config_path = project_dir / "whetstone" / "whetstone.yaml"
    if not config_path.exists():
        return []

    try:
        # Simple YAML parsing for the stored_versions section
        # We avoid requiring PyYAML for this script
        content = config_path.read_text()
        # Look for stored dependency versions in rule files instead
        rules_dir = project_dir / "whetstone" / "rules"
        if not rules_dir.exists():
            return []

        stored_versions: dict[str, str] = {}
        for yaml_file in rules_dir.rglob("*.yaml"):
            text = yaml_file.read_text()
            # Simple extraction of source name and version
            name_match = re.search(r"^\s*name:\s*(.+)$", text, re.MULTILINE)
            ver_match = re.search(
                r"^\s*version:\s*[\"']?(.+?)[\"']?\s*$", text, re.MULTILINE
            )
            if name_match and ver_match:
                stored_versions[name_match.group(1).strip()] = ver_match.group(
                    1
                ).strip()

        drifted = []
        for dep in deps:
            stored_ver = stored_versions.get(dep["name"])
            if stored_ver and stored_ver != dep["version"]:
                drifted.append(
                    {
                        "name": dep["name"],
                        "language": dep["language"],
                        "old_version": stored_ver,
                        "new_version": dep["version"],
                    }
                )

        return drifted
    except Exception:
        return []


def detect_deps(project_dir: Path, do_check_drift: bool = False) -> dict:
    """Main detection logic. Returns structured JSON-ready dict."""
    all_deps: list[dict] = []
    warnings: list[str] = []

    # Detect manifests
    manifests = {
        "pyproject.toml": parse_pyproject_toml,
        "requirements.txt": parse_requirements_txt,
        "package.json": parse_package_json,
        "Cargo.toml": parse_cargo_toml,
    }

    found_any = False
    for filename, parser in manifests.items():
        filepath = project_dir / filename
        if filepath.exists():
            found_any = True
            try:
                deps = parser(filepath)
                all_deps.extend(deps)
            except Exception as e:
                warnings.append(f"Error parsing {filename}: {e}")

    if not found_any:
        result: dict = {
            "languages": [],
            "dependencies": [],
            "error": "No manifest files found",
        }
        return result

    # Deduplicate by (name, language, manifest)
    seen = set()
    unique_deps = []
    for dep in all_deps:
        key = (dep["name"], dep["language"], dep["manifest"])
        if key not in seen:
            seen.add(key)
            unique_deps.append(dep)

    languages = sorted(set(d["language"] for d in unique_deps))

    result = {
        "languages": languages,
        "dependencies": unique_deps,
    }

    if warnings:
        result["warnings"] = warnings

    if do_check_drift:
        drift = check_drift(unique_deps, project_dir)
        result["drift"] = drift

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
    args = parser.parse_args()

    try:
        result = detect_deps(args.project_dir, args.check_drift)
        json.dump(result, sys.stdout, indent=2)
        sys.stdout.write("\n")
    except Exception as e:
        json.dump({"error": str(e)}, sys.stdout, indent=2)
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
