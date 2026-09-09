"""Black-box checks that keep the Python quality gate meaningful after R0."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "whetstone"


@pytest.fixture(scope="session", autouse=True)
def release_binary() -> None:
    if not BIN.exists():
        subprocess.run(
            ["cargo", "build", "--quiet", "--release"],
            cwd=ROOT,
            check=True,
            timeout=180,
        )


def run(*args: str, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(BIN), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=60,
        check=False,
    )


def parsed(result: subprocess.CompletedProcess[str]) -> dict:
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def test_orientation_names_only_the_target_workflows() -> None:
    result = parsed(run("--json"))
    assert result["schema"] == "whetstone.command-response.v1"
    assert result["state"] == "success"
    assert result["data"]["workflows"] == [
        "init",
        "dash",
        "change",
        "check",
        "pull",
        "push",
    ]
    assert result["data"]["read_only"] is True


def test_legacy_command_is_not_dispatchable() -> None:
    result = run("publish")
    assert result.returncode == 2
    assert "unrecognized subcommand" in result.stderr


def test_sync_is_explicitly_unavailable() -> None:
    for workflow in ("pull", "push"):
        result = run(workflow, "--json", "--request-id", "python-client")
        assert result.returncode == 4
        response = json.loads(result.stdout)
        assert response["state"] == "unavailable"
        assert response["workflow"] == workflow


def test_validation_and_eval_do_real_work() -> None:
    validation = parsed(run("validate", "--project-dir", str(ROOT), "--json"))
    assert validation["ok"] is True
    assert "Checking " in validation["report"]
    assert "Checking 0 rule files" not in validation["report"]

    evaluation = parsed(run("eval", "--project-dir", str(ROOT), "--json"))
    assert evaluation["ok"] is True
    assert evaluation["rules_evaluated"] > 0
    assert sum(card["golden_checked"] for card in evaluation["scorecards"]) > 0


def test_scan_reports_a_known_bad_file(tmp_path: Path) -> None:
    rules = tmp_path / "whetstone" / "rules" / "python"
    rules.mkdir(parents=True)
    source = tmp_path / "src"
    source.mkdir()
    (rules / "names.yaml").write_text(
        """source:
  name: team
rules:
  - id: team.lowercase-functions
    severity: must
    confidence: high
    category: convention
    description: Function names must begin lowercase.
    source_url: https://example.com/functions
    approved: true
    status: approved
    signals:
      - id: uppercase-function
        strategy: ast
        description: Finds uppercase function names.
        weight: required
        ast_query: '((function_definition name: (identifier) @match) (#match? @match "^[A-Z]"))'
    golden_examples:
      - code: "def read_config():\n    pass\n"
        verdict: pass
        reason: Lowercase names comply with the rule.
      - code: "def ReadConfig():\n    pass\n"
        verdict: fail
        reason: Uppercase names violate the rule.
"""
    )
    (source / "app.py").write_text("def ReadConfig():\n    pass\n")

    result = parsed(
        run(
            "scan",
            "src",
            "--project-dir",
            str(tmp_path),
            "--lang",
            "python",
            "--json",
            "--no-fail",
            cwd=tmp_path,
        )
    )
    assert result["files_scanned"] == 1
    assert result["rules_applied"] == 1
    assert result["violations_count"] == 1
