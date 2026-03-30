#!/usr/bin/env python3
"""Generate agent context files from approved Whetstone rules.

Reads approved rules from whetstone/rules/**/*.yaml and generates agent context
files in multiple formats: CLAUDE.md, AGENTS.md, .cursorrules, copilot-instructions.md,
.windsurfrules, codex.md.

Usage:
    python3 scripts/generate-agent-context.py
    python3 scripts/generate-agent-context.py --formats claude.md,agents.md
    python3 scripts/generate-agent-context.py --dry-run
"""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None  # type: ignore[assignment]


# --- Simple YAML parser fallback ---


def _simple_yaml_load(text: str) -> dict:
    """Minimal YAML-like loader for rule files when PyYAML is unavailable.

    This handles the subset of YAML used in Whetstone rule files.
    For full YAML support, install PyYAML.
    """
    if yaml:
        return yaml.safe_load(text)

    # Very basic parser — enough for rule files
    # In production, PyYAML should be available
    raise ImportError(
        "PyYAML is required for generate-agent-context.py. "
        "Install it with: pip install pyyaml"
    )


def load_yaml_file(filepath: Path) -> dict | None:
    """Load and parse a YAML file, returning None on error."""
    try:
        text = filepath.read_text()
        if yaml:
            return yaml.safe_load(text)
        return _simple_yaml_load(text)
    except Exception:
        return None


# --- Rule loading ---


def load_rules(rules_dir: Path) -> tuple[list[dict], list[str], int]:
    """Load all approved rules from YAML files.

    Returns (approved_rules, warnings, skipped_count).
    Each rule dict includes extra fields: _language, _dep_name, _source_url_base.
    """
    approved_rules: list[dict] = []
    warnings: list[str] = []
    skipped = 0

    if not rules_dir.exists():
        return approved_rules, ["Rules directory not found: " + str(rules_dir)], 0

    for yaml_file in sorted(rules_dir.rglob("*.yaml")):
        data = load_yaml_file(yaml_file)
        if not data:
            warnings.append(f"Failed to parse: {yaml_file}")
            continue

        source = data.get("source", {})
        dep_name = source.get("name", yaml_file.stem)
        language = _infer_language(yaml_file, source)

        for rule in data.get("rules", []):
            if not rule.get("approved", False):
                skipped += 1
                continue

            rule["_language"] = language
            rule["_dep_name"] = dep_name
            approved_rules.append(rule)

    return approved_rules, warnings, skipped


def _infer_language(filepath: Path, source: dict) -> str:
    """Infer language from file path or source metadata."""
    path_str = str(filepath)
    if "/python/" in path_str:
        return "python"
    if "/typescript/" in path_str:
        return "typescript"
    if "/rust/" in path_str:
        return "rust"
    if "/patterns/" in path_str:
        return "generic"
    return source.get("language", "generic")


# --- Content generation ---


def _get_pass_examples(rule: dict) -> list[str]:
    """Extract pass code examples from golden_examples."""
    examples = []
    for ex in rule.get("golden_examples", []):
        if ex.get("verdict") == "pass":
            examples.append(ex.get("code", "").strip())
    return examples


def _get_fail_examples(rule: dict) -> list[str]:
    """Extract fail code examples from golden_examples."""
    examples = []
    for ex in rule.get("golden_examples", []):
        if ex.get("verdict") == "fail":
            examples.append(ex.get("code", "").strip())
    return examples


def _lang_tag(language: str) -> str:
    """Map language to markdown code fence tag."""
    return {"python": "python", "typescript": "typescript", "rust": "rust"}.get(
        language, ""
    )


def _severity_text(severity: str) -> str:
    """Map severity to display text."""
    return {"must": "MUST", "should": "SHOULD", "may": "MAY"}.get(
        severity, severity.upper()
    )


def generate_rules_content(rules: list[dict]) -> str:
    """Generate the shared rules content (used by all formats)."""
    # Group rules by section
    use_rules: list[dict] = []
    avoid_rules: list[dict] = []
    convention_rules: list[dict] = []

    for rule in rules:
        category = rule.get("category", "convention")
        if category in ("migration", "breaking-change"):
            avoid_rules.append(rule)
        elif category in ("convention", "semantic"):
            convention_rules.append(rule)
        else:  # default and others go to "use" section
            use_rules.append(rule)

    lines: list[str] = []

    # Patterns to USE
    if use_rules or convention_rules:
        lines.append("## Patterns to USE")
        lines.append("")
        for rule in use_rules + convention_rules:
            lines.extend(_format_rule_section(rule))

    # Patterns to AVOID
    if avoid_rules:
        lines.append("## Patterns to AVOID")
        lines.append("")
        for rule in avoid_rules:
            lines.extend(_format_rule_section(rule))

    return "\n".join(lines)


def _format_rule_section(rule: dict) -> list[str]:
    """Format a single rule as a markdown section."""
    lines: list[str] = []
    dep = rule.get("_dep_name", "Unknown")
    desc = rule.get("description", "").strip()
    source_url = rule.get("source_url", "")
    language = rule.get("_language", "")
    lang_tag = _lang_tag(language)

    # Short title from description
    title = desc.split(".")[0].strip() if "." in desc else desc[:80]
    lines.append(f"### {dep}: {title}")
    lines.append(f"{desc}")
    if source_url:
        lines.append(f"Source: {source_url}")
    lines.append("")

    # Pass examples
    pass_examples = _get_pass_examples(rule)
    if pass_examples:
        lines.append("Do:")
        lines.append(f"```{lang_tag}")
        lines.append(pass_examples[0])
        lines.append("```")
        lines.append("")

    # Fail examples
    fail_examples = _get_fail_examples(rule)
    if fail_examples:
        lines.append("Don't:")
        lines.append(f"```{lang_tag}")
        lines.append(fail_examples[0])
        lines.append("```")
        lines.append("")

    return lines


# --- Format-specific generators ---


def _header(title: str, date: str) -> str:
    """Generate a file header."""
    return f"""# {title} (Auto-generated by Whetstone)
# Last updated: {date}
# Source: whetstone/rules/*.yaml
# Do not edit manually — regenerate with: python3 scripts/generate-agent-context.py

"""


FORMAT_CONFIG = {
    "claude.md": {
        "filename": "CLAUDE.md",
        "title": "Claude Code Instructions",
        "preamble": "These coding standards are auto-generated by Whetstone from dependency documentation.\nSee AGENTS.md for universal agent context.\n\n",
    },
    "agents.md": {
        "filename": "AGENTS.md",
        "title": "Agent Instructions",
        "preamble": "These coding standards are auto-generated by Whetstone from dependency documentation.\n\n",
    },
    "cursorrules": {
        "filename": ".cursorrules",
        "title": "Cursor Rules",
        "preamble": "",
    },
    "copilot-instructions.md": {
        "filename": ".github/copilot-instructions.md",
        "title": "GitHub Copilot Instructions",
        "preamble": "These coding standards are auto-generated by Whetstone from dependency documentation.\n\n",
    },
    "windsurfrules": {
        "filename": ".windsurfrules",
        "title": "Windsurf Rules",
        "preamble": "",
    },
    "codex.md": {
        "filename": "codex.md",
        "title": "Codex Instructions",
        "preamble": "These coding standards are auto-generated by Whetstone from dependency documentation.\n\n",
    },
}


def generate_format(
    fmt: str,
    rules: list[dict],
    project_dir: Path,
    dry_run: bool = False,
) -> str | None:
    """Generate a single agent context file. Returns the output path or None."""
    config = FORMAT_CONFIG.get(fmt)
    if not config:
        return None

    date = datetime.now(timezone.utc).strftime("%Y-%m-%d")
    content = _header(config["title"], date)
    content += config["preamble"]
    content += generate_rules_content(rules)

    output_path = project_dir / config["filename"]

    if dry_run:
        return str(output_path)

    # Ensure parent directory exists
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(content)
    return str(output_path)


# --- Config loading ---


def load_config(project_dir: Path) -> tuple[dict, str]:
    """Load whetstone.yaml config. Returns (config, provenance) tuple."""
    config_path = project_dir / "whetstone" / "whetstone.yaml"
    if config_path.exists():
        data = load_yaml_file(config_path)
        if data:
            return data, f"Config: {config_path}"
    return {"agents": ["agents.md"]}, "Config: defaults (no whetstone.yaml found)"


# --- Main ---


def generate_agent_context(
    project_dir: Path,
    formats: list[str] | None = None,
    rules_dir: Path | None = None,
    dry_run: bool = False,
) -> dict:
    """Main generation logic."""
    if rules_dir is None:
        rules_dir = project_dir / "whetstone" / "rules"

    # Load rules
    rules, warnings, skipped = load_rules(rules_dir)

    if not rules:
        return {
            "generated": [],
            "rules_count": 0,
            "dependencies": [],
            "skipped_unapproved": skipped,
            "warnings": warnings or ["No approved rules found"],
            "next_command": "Extract rules first: run whetstone doctor",
        }

    # Determine formats
    config_provenance = ""
    if formats is None:
        config, config_provenance = load_config(project_dir)
        formats = config.get("agents") or ["agents.md"]
    if config_provenance:
        print(config_provenance, file=sys.stderr)

    # Generate each format
    generated: list[str] = []
    for fmt in formats:
        output = generate_format(fmt, rules, project_dir, dry_run)
        if output:
            generated.append(output)

    # Collect dependency names
    dep_names = sorted(set(r.get("_dep_name", "") for r in rules))

    result: dict = {
        "generated": generated,
        "rules_count": len(rules),
        "dependencies": dep_names,
        "skipped_unapproved": skipped,
    }
    if warnings:
        result["warnings"] = warnings

    result["next_command"] = (
        "Generate tests: python3 scripts/generate-tests.py --project-dir ."
    )

    return result


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Generate agent context files from approved Whetstone rules."
    )
    parser.add_argument(
        "--project-dir",
        type=Path,
        default=Path("."),
        help="Project root directory (default: .)",
    )
    parser.add_argument(
        "--formats",
        type=str,
        help="Comma-separated formats to generate (default: from whetstone.yaml)",
    )
    parser.add_argument(
        "--rules-dir",
        type=Path,
        help="Directory containing rule YAML files (default: whetstone/rules)",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Print what would be generated without writing files",
    )
    args = parser.parse_args()

    try:
        formats = args.formats.split(",") if args.formats else None

        result = generate_agent_context(
            project_dir=args.project_dir,
            formats=formats,
            rules_dir=args.rules_dir,
            dry_run=args.dry_run,
        )

        json.dump(result, sys.stdout, indent=2)
        sys.stdout.write("\n")

    except Exception as e:
        json.dump(
            {
                "error": str(e),
                "next_command": "Check rules directory and PyYAML installation",
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
