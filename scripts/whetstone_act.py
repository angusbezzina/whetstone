#!/usr/bin/env python3
"""Act on a Whetstone sweep's findings.json in any GitHub repository.

This is what Messo does inside Virgil, for a repository without Virgil. Each
finding is worked by its class:

- ``draft_pr``: one draft pull request per fingerprint. When the finding
  carries the tool's own ``fix``, the pull request applies it; otherwise it
  is an empty commit whose description holds the rule and ``wh explain``
  output, for an agent to complete.
- ``confirm_first``: one issue asking a person to approve the change.
- ``discuss``: one issue labelled for discussion.

Fingerprints make every run idempotent. An open item for a fingerprint is
updated only when the finding's ``subjectDigest`` changed; a closed item (the
finding was dismissed or fixed) keeps it quiet until the subject changes. A
finding marked ``unsure`` is parked until a person raises it.

It shells out to ``gh`` and ``git`` and reads nothing but the findings file
and the repository it runs in.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

MARKER = re.compile(r"<!-- whetstone:finding fingerprint=(\S+) subject=(\S+) -->")
LABEL = "whetstone"
CLASS_LABELS = {
    "draft_pr": "whetstone:draft-pr",
    "confirm_first": "whetstone:confirm-first",
    "discuss": "whetstone:discuss",
}
LABEL_COLORS = {
    LABEL: "5319e7",
    "whetstone:draft-pr": "1d76db",
    "whetstone:confirm-first": "fbca04",
    "whetstone:discuss": "c5def5",
}


@dataclass
class Item:
    kind: str  # "issue" or "pr"
    number: int
    state: str  # "OPEN", "CLOSED" or "MERGED"
    fingerprint: str
    subject: str


@dataclass
class Report:
    created: list[str] = field(default_factory=list)
    updated: list[str] = field(default_factory=list)
    unchanged: list[str] = field(default_factory=list)
    quiet: list[str] = field(default_factory=list)
    parked: list[str] = field(default_factory=list)
    planned: list[dict[str, str]] = field(default_factory=list)

    def as_json(self) -> dict[str, object]:
        return {
            "created": self.created,
            "updated": self.updated,
            "unchanged": self.unchanged,
            "quiet": self.quiet,
            "parked": self.parked,
            "planned": self.planned,
        }


def run(args: list[str], cwd: Path, stdin: str | None = None) -> str:
    result = subprocess.run(
        args,
        cwd=cwd,
        input=stdin,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"{' '.join(args[:3])} failed ({result.returncode}): {result.stderr.strip()}"
        )
    return result.stdout


def load_findings(path: Path) -> list[dict[str, object]]:
    value = json.loads(path.read_text(encoding="utf-8"))
    items = value.get("findings", []) if isinstance(value, dict) else value
    if not isinstance(items, list):
        raise ValueError(f"{path} is not a findings file")
    for item in items:
        for key in ("fingerprint", "subjectDigest", "class", "title", "responsibility"):
            if key not in item:
                raise ValueError(f"a finding in {path} has no {key}")
    return items


def existing_items(repo: Path) -> dict[str, list[Item]]:
    """Every Whetstone issue and pull request, keyed by fingerprint."""
    found: dict[str, list[Item]] = {}
    for kind, command in (
        ("issue", ["gh", "issue", "list"]),
        ("pr", ["gh", "pr", "list"]),
    ):
        listed = json.loads(
            run(
                [
                    *command,
                    "--state",
                    "all",
                    "--label",
                    LABEL,
                    "--limit",
                    "1000",
                    "--json",
                    "number,state,body",
                ],
                repo,
            )
            or "[]"
        )
        for entry in listed:
            match = MARKER.search(entry.get("body") or "")
            if not match:
                continue
            item = Item(
                kind=kind,
                number=int(entry["number"]),
                state=str(entry["state"]).upper(),
                fingerprint=match.group(1),
                subject=match.group(2),
            )
            found.setdefault(item.fingerprint, []).append(item)
    return found


def ensure_labels(repo: Path) -> None:
    for name, color in LABEL_COLORS.items():
        run(["gh", "label", "create", name, "--color", color, "--force"], repo)


def explain(repo: Path, finding: dict[str, object], findings_path: Path) -> str:
    """`wh explain` for the finding, when wh is installed."""
    try:
        return run(
            [
                "wh",
                "explain",
                str(finding["fingerprint"]),
                "--findings",
                str(findings_path),
            ],
            repo,
        ).strip()
    except (OSError, RuntimeError):
        return ""


def body(finding: dict[str, object], explained: str, intro: str) -> str:
    lines = [
        intro,
        "",
        f"**Rule:** `{finding['responsibility']}`",
        f"**Finding:** {finding['title']}",
        f"**Priority:** {finding.get('priority', 2)} (0 is most urgent)",
        "",
    ]
    if explained:
        lines += [
            "<details><summary>wh explain</summary>",
            "",
            "```",
            explained,
            "```",
            "",
            "</details>",
            "",
        ]
    lines += [
        f"Explain it locally with `wh explain {finding['fingerprint']}`.",
        "",
        f"<!-- whetstone:finding fingerprint={finding['fingerprint']} subject={finding['subjectDigest']} -->",
    ]
    return "\n".join(lines)


def issue_intro(finding: dict[str, object]) -> str:
    if finding["class"] == "confirm_first":
        return (
            "Whetstone's sweep found a breach of an accepted rule. The rule says to ask a "
            "person first: approve a fix by assigning this issue to an agent, or close it to "
            "dismiss the finding (it stays quiet until the flagged code changes)."
        )
    return (
        "Whetstone's sweep found a breach of an accepted rule that is for discussion. "
        "Decide here whether to fix the code, change the rule (`wh change`), or close this "
        "issue to dismiss the finding until the flagged code changes."
    )


def pr_intro(finding: dict[str, object]) -> str:
    if finding.get("fix"):
        return (
            "Whetstone's sweep found a breach of an accepted rule, and the rule's own tool "
            "produced this fix. Review it, mark it ready, or close it to dismiss the finding."
        )
    return (
        "Whetstone's sweep found a breach of an accepted rule that an agent may fix. This "
        "draft has no change yet: an agent completes it from the rule and the explanation "
        "below, runs `wh check`, and marks it ready."
    )


def branch_name(finding: dict[str, object]) -> str:
    digest = str(finding["fingerprint"]).removeprefix("sha256:")
    return f"whetstone/{digest[:16]}"


def push_branch(repo: Path, finding: dict[str, object], base: str, remote: str) -> str:
    """Build the finding's branch from the base: the fix, or an empty commit."""
    branch = branch_name(finding)
    original = run(["git", "rev-parse", "--abbrev-ref", "HEAD"], repo).strip()
    run(["git", "fetch", "--quiet", remote, base], repo)
    run(
        ["git", "switch", "--quiet", "--force-create", branch, f"{remote}/{base}"], repo
    )
    try:
        message = f"whetstone: {finding['title']}"[:200]
        fix = finding.get("fix")
        if fix:
            with tempfile.NamedTemporaryFile(
                "w", suffix=".patch", delete=False
            ) as patch:
                patch.write(str(fix))
            try:
                run(["git", "apply", "--index", patch.name], repo)
            finally:
                os.unlink(patch.name)
            run(["git", "commit", "--quiet", "--no-verify", "-m", message], repo)
        else:
            run(
                [
                    "git",
                    "commit",
                    "--quiet",
                    "--no-verify",
                    "--allow-empty",
                    "-m",
                    message,
                ],
                repo,
            )
        run(["git", "push", "--quiet", "--force", remote, f"{branch}:{branch}"], repo)
    finally:
        run(["git", "switch", "--quiet", "--force", original], repo)
    return branch


def write_body(text: str) -> str:
    with tempfile.NamedTemporaryFile("w", suffix=".md", delete=False) as handle:
        handle.write(text)
        return handle.name


def act(
    repo: Path,
    findings_path: Path,
    base: str,
    remote: str = "origin",
    dry_run: bool = False,
) -> Report:
    report = Report()
    findings = load_findings(findings_path)
    known = existing_items(repo)
    if not dry_run and findings:
        ensure_labels(repo)
    for finding in findings:
        fingerprint = str(finding["fingerprint"])
        subject = str(finding["subjectDigest"])
        klass = str(finding["class"])
        if finding.get("unsure"):
            report.parked.append(fingerprint)
            continue
        if klass not in CLASS_LABELS:
            report.parked.append(fingerprint)
            continue
        items = known.get(fingerprint, [])
        open_items = [item for item in items if item.state == "OPEN"]
        if open_items:
            item = open_items[0]
            if item.subject == subject:
                report.unchanged.append(fingerprint)
                continue
            action = "update"
        elif any(item.subject == subject for item in items):
            # Dismissed or fixed: quiet until the flagged code changes.
            report.quiet.append(fingerprint)
            continue
        else:
            item = None
            action = "create"
        report.planned.append(
            {"fingerprint": fingerprint, "class": klass, "action": action}
        )
        if dry_run:
            continue
        explained = explain(repo, finding, findings_path)
        labels = ["--label", LABEL, "--label", CLASS_LABELS[klass]]
        title = f"whetstone: {finding['title']}"[:250]
        if klass == "draft_pr":
            text = write_body(body(finding, explained, pr_intro(finding)))
            try:
                branch = push_branch(repo, finding, base, remote)
                if item is None:
                    run(
                        [
                            "gh",
                            "pr",
                            "create",
                            "--draft",
                            "--base",
                            base,
                            "--head",
                            branch,
                            "--title",
                            title,
                            "--body-file",
                            text,
                            *labels,
                        ],
                        repo,
                    )
                else:
                    run(
                        ["gh", "pr", "edit", str(item.number), "--body-file", text],
                        repo,
                    )
            finally:
                os.unlink(text)
        else:
            text = write_body(body(finding, explained, issue_intro(finding)))
            try:
                if item is None:
                    run(
                        [
                            "gh",
                            "issue",
                            "create",
                            "--title",
                            title,
                            "--body-file",
                            text,
                            *labels,
                        ],
                        repo,
                    )
                else:
                    run(
                        ["gh", "issue", "edit", str(item.number), "--body-file", text],
                        repo,
                    )
            finally:
                os.unlink(text)
        (report.created if action == "create" else report.updated).append(fingerprint)
    return report


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--findings", default="findings.json", type=Path)
    parser.add_argument("--repo", default=".", type=Path)
    parser.add_argument("--base", default=os.environ.get("GITHUB_REF_NAME", "main"))
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    try:
        report = act(
            args.repo.resolve(),
            args.findings.resolve(),
            args.base,
            args.remote,
            args.dry_run,
        )
    except (OSError, RuntimeError, ValueError) as error:
        print(json.dumps({"error": str(error)}), file=sys.stderr)
        return 1
    print(json.dumps(report.as_json(), indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
