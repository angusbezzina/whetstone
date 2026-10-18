"""The reusable runner acts on findings.json by class, once per fingerprint."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))

import whetstone_act  # noqa: E402

FIX = """diff --git a/src/app.py b/src/app.py
index 1111111..2222222 100644
--- a/src/app.py
+++ b/src/app.py
@@ -1 +1 @@
-x=1
+x = 1
"""


def git(cwd: Path, *args: str) -> str:
    return subprocess.run(
        ["git", *args], cwd=cwd, check=True, capture_output=True, text=True
    ).stdout


def setup(tmp_path: Path, monkeypatch) -> tuple[Path, Path, Path]:
    origin = tmp_path / "origin.git"
    git(tmp_path, "init", "--quiet", "--bare", "--initial-branch=main", str(origin))
    repo = tmp_path / "repo"
    repo.mkdir()
    git(repo, "init", "--quiet", "--initial-branch=main")
    git(repo, "config", "user.name", "Runner")
    git(repo, "config", "user.email", "runner@example.invalid")
    git(repo, "config", "commit.gpgsign", "false")
    (repo / "src").mkdir()
    (repo / "src" / "app.py").write_text("x=1\n")
    git(repo, "add", "-A")
    git(repo, "commit", "--quiet", "-m", "base")
    git(repo, "remote", "add", "origin", str(origin))
    git(repo, "push", "--quiet", "origin", "main")
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()
    gh = bin_dir / "gh"
    gh.write_text(
        f'#!/bin/sh\nexec {sys.executable} {ROOT / "tests" / "fake_gh.py"} "$@"\n'
    )
    gh.chmod(0o755)
    state = tmp_path / "gh-state.json"
    monkeypatch.setenv("PATH", f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    monkeypatch.setenv("FAKE_GH_STATE", str(state))
    return repo, origin, state


def finding(
    name: str, klass: str, subject: str = "1", **extra: object
) -> dict[str, object]:
    return {
        "fingerprint": "sha256:" + name * 64,
        "subjectDigest": "sha256:" + subject * 64,
        "class": klass,
        "priority": 2,
        "title": f"Rule {name} (src/app.py)",
        "responsibility": f"rule.{name}",
        **extra,
    }


def test_each_class_opens_its_work_once_and_dismissed_findings_stay_quiet(
    tmp_path: Path, monkeypatch
) -> None:
    repo, origin, state_path = setup(tmp_path, monkeypatch)
    findings = tmp_path / "findings.json"
    findings.write_text(
        json.dumps(
            [
                finding("a", "draft_pr", fix=FIX),
                finding("b", "confirm_first"),
                finding("c", "discuss"),
                finding("d", "draft_pr", unsure="the rule doubts generated files"),
            ]
        )
    )

    planned = whetstone_act.act(repo, findings, "main", dry_run=True)
    assert len(planned.planned) == 3
    assert not state_path.exists() or json.loads(state_path.read_text())["issue"] == []

    report = whetstone_act.act(repo, findings, "main")
    assert len(report.created) == 3
    assert report.parked == ["sha256:" + "d" * 64]
    state = json.loads(state_path.read_text())
    [pr] = state["pr"]
    assert pr["draft"] is True
    assert pr["head"] == "whetstone/" + "a" * 16
    assert "whetstone:draft-pr" in pr["labels"]
    issues = {issue["labels"][1]: issue for issue in state["issue"]}
    assert set(issues) == {"whetstone:confirm-first", "whetstone:discuss"}
    assert "approve" in issues["whetstone:confirm-first"]["body"]
    # The draft pull request's branch carries the tool's fix.
    shown = git(origin, "show", f"whetstone/{'a' * 16}:src/app.py")
    assert shown == "x = 1\n"
    assert git(repo, "rev-parse", "--abbrev-ref", "HEAD").strip() == "main"

    # A second run changes nothing.
    again = whetstone_act.act(repo, findings, "main")
    assert again.created == [] and again.updated == []
    assert len(again.unchanged) == 3
    assert json.loads(state_path.read_text()) == state

    # A dismissed (closed) finding stays quiet until its subject changes.
    state["issue"][0]["state"] = "CLOSED"
    state_path.write_text(json.dumps(state))
    quiet = whetstone_act.act(repo, findings, "main")
    assert quiet.quiet == [
        state["issue"][0]["body"].split("fingerprint=")[1].split()[0]
    ]
    changed = json.loads(findings.read_text())
    for item in changed:
        if item["class"] in ("confirm_first", "discuss"):
            item["subjectDigest"] = "sha256:" + "9" * 64
    findings.write_text(json.dumps(changed))
    moved = whetstone_act.act(repo, findings, "main")
    assert len(moved.created) == 1, moved.as_json()
    assert len(moved.updated) == 1, moved.as_json()


def test_a_file_that_is_not_findings_is_refused(tmp_path: Path) -> None:
    bad = tmp_path / "findings.json"
    bad.write_text(json.dumps([{"title": "no fingerprint"}]))
    assert whetstone_act.main(["--findings", str(bad), "--repo", str(tmp_path)]) == 1
