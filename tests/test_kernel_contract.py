"""Black-box checks that keep the Python quality gate meaningful after R0."""

from __future__ import annotations

import json
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "target" / "debug" / "whetstone"


@pytest.fixture(scope="session", autouse=True)
def debug_binary() -> None:
    # Always build: cargo is a no-op when the binary is current, and a stale
    # binary would test a surface that no longer exists.
    subprocess.run(
        ["cargo", "build", "--quiet", "--bin", "whetstone"],
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


@pytest.mark.parametrize("command", ["publish", "validate", "eval", "scan"])
def test_legacy_command_is_not_dispatchable(command: str) -> None:
    result = run(command)
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
        ("check", "--path", "src"),
        ("check", "--lang", "rust"),
        ("change", "--migrate"),
    ],
)
def test_removed_flags_are_rejected(args: tuple[str, ...], tmp_path: Path) -> None:
    result = run(*args, cwd=git_repo(tmp_path))
    assert result.returncode == 2, result.stdout
    assert not (tmp_path / ".git" / "whetstone").exists()


def test_init_inspects_read_only_then_agrees(tmp_path: Path) -> None:
    repo = git_repo(tmp_path)
    # A test command, so the prove-it-works starter is offered.
    (repo / "package.json").write_text('{"scripts":{"test":"true"}}')
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
        "prove-it-works",
        cwd=repo,
    )
    assert agreed.returncode == 6, agreed.stderr
    response = json.loads(agreed.stdout)
    assert response["data"]["progress"]["agreement_complete"] is True
    assert sorted(record["id"] for record in response["data"]["records"]) == [
        "mission.project",
        "principle.prove-it-works",
        "rule.prove-it-works",
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
