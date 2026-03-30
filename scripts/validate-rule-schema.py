#!/usr/bin/env python3
"""Validate documented rule schema fields against fixture rule files.

This mirrors the CI schema-check gate, but intentionally skips fixtures that are
designed to be malformed for warning/regression coverage.
"""

from __future__ import annotations

from pathlib import Path

import yaml

VALID_SEVERITIES = {"must", "should", "may"}
VALID_CONFIDENCES = {"high", "medium"}
VALID_CATEGORIES = {
    "migration",
    "default",
    "convention",
    "breaking-change",
    "semantic",
}
VALID_STRATEGIES = {"ast", "pattern", "lint_proxy", "ai"}
REQUIRED_FIELDS = [
    "id",
    "severity",
    "confidence",
    "category",
    "description",
    "source_url",
    "signals",
]
INTENTIONALLY_INVALID_FIXTURES = {
    Path("tests/fixtures/whetstone/rules/python/malformed.yaml"),
}


def main() -> int:
    schema_path = Path("references/rule-schema.yaml")
    if not schema_path.exists():
        print("FAIL: references/rule-schema.yaml not found")
        return 1

    text = schema_path.read_text()
    print("Schema file found and readable.")

    for field in REQUIRED_FIELDS:
        if field not in text:
            print(f'FAIL: required field "{field}" not found in schema')
            return 1
        print(f"  OK: {field}")

    fixtures = list(Path("tests/fixtures").rglob("*.yaml"))
    print(f"Checking {len(fixtures)} fixture files...")
    errors: list[str] = []

    for fixture in fixtures:
        rel = fixture.relative_to(Path.cwd()) if fixture.is_absolute() else fixture
        if rel in INTENTIONALLY_INVALID_FIXTURES:
            print(f"  SKIP: {rel} (intentional invalid fixture)")
            continue

        try:
            data = yaml.safe_load(fixture.read_text())
        except Exception as exc:
            errors.append(f"{rel}: parse error: {exc}")
            continue

        if not data or "rules" not in data:
            continue

        for rule in data["rules"]:
            rid = rule.get("id", "?")
            for req in ("id", "severity", "confidence", "category", "source_url"):
                if req not in rule:
                    errors.append(f"{rel}: rule {rid} missing {req}")

            severity = rule.get("severity")
            if severity and severity not in VALID_SEVERITIES:
                errors.append(f'{rel}: rule {rid} invalid severity "{severity}"')

            confidence = rule.get("confidence")
            if confidence and confidence not in VALID_CONFIDENCES:
                errors.append(f'{rel}: rule {rid} invalid confidence "{confidence}"')

            category = rule.get("category")
            if category and category not in VALID_CATEGORIES:
                errors.append(f'{rel}: rule {rid} invalid category "{category}"')

            for signal in rule.get("signals", []):
                if isinstance(signal, dict):
                    strategy = signal.get("strategy")
                    if strategy and strategy not in VALID_STRATEGIES:
                        errors.append(
                            f'{rel}: rule {rid} invalid strategy "{strategy}"'
                        )

            print(f"  OK: {rel} / {rid}")

    if errors:
        for error in errors:
            print(f"FAIL: {error}")
        return 1

    print("All schema checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
