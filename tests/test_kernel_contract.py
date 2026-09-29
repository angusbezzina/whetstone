"""Black-box checks that keep the Python quality gate meaningful after R0."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "release" / "whetstone"


@pytest.fixture(scope="session", autouse=True)
def release_binary() -> None:
    # Always build: cargo is a no-op when the binary is current, and a stale
    # binary would test a surface that no longer exists.
    subprocess.run(
        ["cargo", "build", "--quiet", "--release", "--bin", "whetstone"],
        cwd=ROOT,
        check=True,
        timeout=900,
    )


def run(*args: str, cwd: Path = ROOT) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(BIN), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
        env={**os.environ, "WHETSTONE_JEV_OFFLINE": "1"},
    )


def git_repo(path: Path) -> Path:
    subprocess.run(["git", "init", "-q"], cwd=path, check=True)
    subprocess.run(["git", "config", "user.name", "Test Owner"], cwd=path, check=True)
    subprocess.run(
        ["git", "config", "user.email", "owner@example.invalid"], cwd=path, check=True
    )
    return path


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


@pytest.mark.parametrize(
    "args",
    [
        ("init", "--action", "agree", "--desired-outcome", "x"),
        ("init", "--action", "agree", "--values", "x"),
        ("change", "--kind", "value"),
        ("change", "--kind", "exception"),
        ("change", "--activate", "proposal.x"),
        ("check", "--repair-session", "x"),
    ],
)
def test_removed_flags_are_rejected(args: tuple[str, ...], tmp_path: Path) -> None:
    result = run(*args, cwd=git_repo(tmp_path))
    assert result.returncode == 2, result.stdout
    assert not (tmp_path / ".git" / "whetstone").exists()


def test_init_inspects_read_only_then_agrees(tmp_path: Path) -> None:
    repo = git_repo(tmp_path)
    inspect = run("init", "--json", "--request-id", "py-1", cwd=repo)
    assert inspect.returncode == 6, inspect.stderr
    handoff = json.loads(inspect.stdout)
    assert handoff["state"] == "needs_input"
    assert handoff["data"]["inspection_writes"] == []
    assert handoff["data"]["progress"]["missing_decisions"] == ["mission", "rules"]
    assert not (repo / ".git" / "whetstone").exists()

    agreed = run(
        "init",
        "--json",
        "--action",
        "agree",
        "--request-id",
        "py-1",
        "--expected-revision",
        str(handoff["expected_revision"]),
        "--resume",
        handoff["resume_token"],
        "--mission",
        "Keep project work aligned.",
        "--principle",
        "prove-it-works",
        "--starter",
        "ask-before-public-api",
        cwd=repo,
    )
    assert agreed.returncode == 6, agreed.stderr
    response = json.loads(agreed.stdout)
    assert response["data"]["progress"]["agreement_complete"] is True
    assert sorted(record["id"] for record in response["data"]["records"]) == [
        "mission.project",
        "principle.prove-it-works",
        "rule.ask-before-public-api",
    ]
    assert response["data"]["shared"] is False


def test_sync_without_a_shared_database_changes_nothing(tmp_path: Path) -> None:
    # A throwaway repository: never this repository's real .beads.
    git_repo(tmp_path)
    pull = run("pull", "--json", "--request-id", "python-client", cwd=tmp_path)
    assert pull.returncode == 4
    response = json.loads(pull.stdout)
    assert response["state"] == "unavailable"
    assert response["workflow"] == "pull"
    push = run("push", "--json", "--request-id", "python-client", cwd=tmp_path)
    assert push.returncode == 6
    response = json.loads(push.stdout)
    assert response["state"] == "needs_input"
    assert "nothing was published" in response["summary"]


def test_validation_and_eval_do_real_work() -> None:
    validation = parsed(run("validate", "--project-dir", str(ROOT), "--json"))
    assert validation["ok"] is True
    assert "Checking " in validation["report"]
    assert "Checking 0 rule files" not in validation["report"]

    evaluation = parsed(run("eval", "--project-dir", str(ROOT), "--json"))
    assert evaluation["ok"] is True
    assert evaluation["rules_evaluated"] > 0
    assert sum(card["golden_checked"] for card in evaluation["scorecards"]) > 0
    assert evaluation["rule_eval"]["ok"] is True
    assert evaluation["rule_eval"]["pstack_roundtrip"]["ok"] is True


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
