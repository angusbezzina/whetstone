#!/usr/bin/env python3
"""Mine style patterns from conversation transcripts, git history, and PRs.

Analyzes three data sources for recurring style/convention patterns:
1. Agent conversation transcripts (JSONL from Claude Code, Cursor, Cline, etc.)
2. Git commit history and diffs
3. GitHub PR review comments (optional, requires gh CLI)

Usage:
    python3 scripts/detect-patterns.py
    python3 scripts/detect-patterns.py --since-last-run --quiet
    python3 scripts/detect-patterns.py --sources transcript,git --since "7 days ago"
"""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone
from pathlib import Path

# --- Style signal patterns ---

DIRECTIVE_PATTERNS = [
    re.compile(r"\b(always\s+use)\b", re.IGNORECASE),
    re.compile(r"\b(never\s+use)\b", re.IGNORECASE),
    re.compile(r"\b(prefer\s+\w+\s+over)\b", re.IGNORECASE),
    re.compile(r"\b(don'?t\s+use)\b", re.IGNORECASE),
    re.compile(r"\b(make\s+sure\s+you)\b", re.IGNORECASE),
    re.compile(r"\b(never\s+do)\b", re.IGNORECASE),
    re.compile(r"\b(always\s+prefer)\b", re.IGNORECASE),
    re.compile(r"\b(should\s+always)\b", re.IGNORECASE),
    re.compile(r"\b(must\s+use)\b", re.IGNORECASE),
    re.compile(r"\b(do\s+not\s+use)\b", re.IGNORECASE),
]

STYLE_KEYWORDS = re.compile(
    r"\b(format|style|convention|naming|pattern|approach|standard|consistent|"
    r"refactor|rename|rewrite|restructure)\b",
    re.IGNORECASE,
)

CORRECTION_PATTERNS = [
    re.compile(r"that'?s\s+not\s+how\s+we", re.IGNORECASE),
    re.compile(r"we\s+use\s+\w+\s+here", re.IGNORECASE),
    re.compile(r"change\s+this\s+to", re.IGNORECASE),
    re.compile(r"should\s+be\s+\w+\s+instead", re.IGNORECASE),
    re.compile(r"let'?s\s+use\s+\w+\s+instead", re.IGNORECASE),
]

VERSION_PATTERNS = [
    re.compile(r"use\s+the\s+new", re.IGNORECASE),
    re.compile(r"v\d+\s+way", re.IGNORECASE),
    re.compile(r"latest\s+api", re.IGNORECASE),
    re.compile(r"\bdeprecated\b", re.IGNORECASE),
]

# Git commit patterns indicating style work
GIT_STYLE_PATTERNS = re.compile(
    r"\b(fix\s*style|format|lint|convention|refactor:\s*rename|refactor:\s*style|"
    r"code\s*style|formatting|clean\s*up|standardize)\b",
    re.IGNORECASE,
)

# Config files that indicate style preferences
STYLE_CONFIG_FILES = {
    ".eslintrc",
    ".eslintrc.js",
    ".eslintrc.json",
    ".eslintrc.yml",
    "ruff.toml",
    "pyproject.toml",
    "rustfmt.toml",
    "biome.json",
    "biome.jsonc",
    ".prettierrc",
    ".prettierrc.json",
    ".editorconfig",
    "deno.json",
    "deno.jsonc",
}


def _run_cmd(cmd: list[str], cwd: str | None = None) -> str | None:
    """Run a command and return stdout, or None on failure."""
    try:
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30,
            cwd=cwd,
        )
        if result.returncode == 0:
            return result.stdout
        return None
    except (subprocess.TimeoutExpired, FileNotFoundError, OSError):
        return None


def _has_command(name: str) -> bool:
    """Check if a command is available."""
    return _run_cmd(["which", name]) is not None


# --- Source 1: Conversation Transcripts ---

# Known agent transcript directories (relative to home)
# Each agent stores conversation history in JSONL format at different paths.
TRANSCRIPT_DIRS = [
    ".claude/projects",  # Claude Code
    ".cursor/projects",  # Cursor
    ".cline/projects",  # Cline
    ".continue/sessions",  # Continue
    ".codex/sessions",  # Codex
    ".goose/sessions",  # Goose
    ".roo/projects",  # Roo Code
    ".agents/sessions",  # Amp, Gemini CLI, GitHub Copilot (shared path)
    ".config/opencode/sessions",  # OpenCode
    ".windsurf/sessions",  # Windsurf
]


def _project_transcript_filter(project_dir: Path, transcript_path: Path) -> bool:
    """Check if a transcript file is likely for the given project.

    Matches on project directory name appearing in the transcript path.
    This is a heuristic — agents typically store transcripts under
    paths like ~/.claude/projects/<project-name>/...
    """
    project_name = project_dir.resolve().name.lower()
    # Check if the project name appears in the transcript path
    return project_name in str(transcript_path).lower()


def mine_transcripts(
    project_dir: Path,
    since: datetime | None = None,
    global_transcripts: bool = False,
) -> tuple[list[dict], dict]:
    """Mine agent conversation transcripts for style patterns.

    By default, only scans transcripts matching the current project directory
    name for privacy. Use global_transcripts=True to scan all projects.
    """
    patterns: list[dict] = []
    stats = {"files": 0, "messages": 0, "scoped": not global_transcripts}

    # Collect JSONL files from all known agent transcript locations
    jsonl_files: list[Path] = []
    home = Path.home()
    for rel_dir in TRANSCRIPT_DIRS:
        transcript_dir = home / rel_dir
        if transcript_dir.exists():
            for f in transcript_dir.rglob("*.jsonl"):
                if global_transcripts or _project_transcript_filter(project_dir, f):
                    jsonl_files.append(f)

    if not jsonl_files:
        return patterns, stats

    stats["files"] = len(jsonl_files)

    # Group style signals by rough description
    signal_groups: dict[str, list[dict]] = defaultdict(list)

    for jsonl_file in jsonl_files:
        session_id = jsonl_file.stem
        try:
            with open(jsonl_file) as f:
                for line in f:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        msg = json.loads(line)
                    except json.JSONDecodeError:
                        continue

                    # Only look at user/human messages
                    role = msg.get("role", msg.get("type", ""))
                    if role not in ("user", "human"):
                        continue

                    # Extract text content
                    content = ""
                    if isinstance(msg.get("content"), str):
                        content = msg["content"]
                    elif isinstance(msg.get("content"), list):
                        for block in msg["content"]:
                            if isinstance(block, dict) and block.get("type") == "text":
                                content += block.get("text", "") + " "
                            elif isinstance(block, str):
                                content += block + " "

                    if not content.strip():
                        continue

                    stats["messages"] += 1

                    # Check timestamp filter
                    if since:
                        ts = msg.get("timestamp", msg.get("created_at", ""))
                        if ts:
                            try:
                                msg_time = datetime.fromisoformat(
                                    ts.replace("Z", "+00:00")
                                )
                                if msg_time < since:
                                    continue
                            except (ValueError, TypeError):
                                pass

                    # Check for style signals
                    matched = False
                    for pattern in DIRECTIVE_PATTERNS:
                        if pattern.search(content):
                            matched = True
                            break

                    if not matched:
                        for pattern in CORRECTION_PATTERNS:
                            if pattern.search(content):
                                matched = True
                                break

                    if not matched and STYLE_KEYWORDS.search(content):
                        # Only count keyword matches if they're in a directive context
                        if any(p.search(content) for p in VERSION_PATTERNS):
                            matched = True

                    if matched:
                        # Create a simplified description from the message
                        desc = content[:200].strip()
                        # Group by first significant phrase
                        key = _extract_pattern_key(content)
                        signal_groups[key].append(
                            {
                                "text": desc,
                                "session": session_id,
                                "source_file": str(jsonl_file),
                            }
                        )

        except (OSError, UnicodeDecodeError):
            continue

    # Convert groups to patterns
    for key, signals in signal_groups.items():
        sessions = list(set(s["session"] for s in signals))
        patterns.append(
            {
                "description": key,
                "source": "transcript",
                "occurrences": len(signals),
                "confidence": "high" if len(sessions) >= 3 else "medium",
                "sessions": sessions[:10],
                "example_quotes": [s["text"] for s in signals[:3]],
                "last_seen": datetime.now(timezone.utc).isoformat(),
            }
        )

    return patterns, stats


def _extract_pattern_key(text: str) -> str:
    """Extract a short key phrase from a style-related message."""
    text = text.strip()
    # Try to extract "always use X", "prefer X over Y", etc.
    for pat in [
        r"(always\s+use\s+[\w\s]+)",
        r"(never\s+use\s+[\w\s]+)",
        r"(prefer\s+\w+\s+over\s+\w+)",
        r"(don'?t\s+use\s+[\w\s]+)",
        r"(use\s+\w+\s+instead\s+of\s+\w+)",
    ]:
        m = re.search(pat, text, re.IGNORECASE)
        if m:
            return m.group(1).strip()[:100]

    # Fallback: first 80 chars
    return text[:80].strip()


# --- Source 2: Git History ---


def mine_git_history(
    project_dir: Path,
    since: str | None = None,
) -> tuple[list[dict], dict]:
    """Analyze git commit history for style-related changes."""
    patterns: list[dict] = []
    stats = {"commits": 0}

    # Check if we're in a git repo
    if not (project_dir / ".git").exists():
        return patterns, stats

    cwd = str(project_dir)

    # Get style-related commits
    cmd = ["git", "log", "--oneline", "--no-merges", "-500"]
    if since:
        cmd.extend(["--since", since])

    output = _run_cmd(cmd, cwd=cwd)
    if not output:
        return patterns, stats

    style_commits: list[dict] = []
    for line in output.strip().split("\n"):
        if not line.strip():
            continue
        stats["commits"] += 1
        parts = line.split(" ", 1)
        if len(parts) < 2:
            continue
        sha, message = parts
        if GIT_STYLE_PATTERNS.search(message):
            style_commits.append({"sha": sha, "message": message})

    # Group by type of style change
    commit_groups: dict[str, list[str]] = defaultdict(list)
    for commit in style_commits:
        msg = commit["message"].lower()
        if "format" in msg or "formatting" in msg:
            commit_groups["Code formatting standardization"].append(commit["message"])
        elif "lint" in msg:
            commit_groups["Linting fixes"].append(commit["message"])
        elif "rename" in msg:
            commit_groups["Naming convention changes"].append(commit["message"])
        elif "style" in msg:
            commit_groups["Style fixes"].append(commit["message"])
        elif "refactor" in msg:
            commit_groups["Refactoring patterns"].append(commit["message"])
        else:
            commit_groups["Other style changes"].append(commit["message"])

    for desc, commits in commit_groups.items():
        if len(commits) >= 2:
            patterns.append(
                {
                    "description": desc,
                    "source": "git",
                    "occurrences": len(commits),
                    "confidence": "high" if len(commits) >= 5 else "medium",
                    "sessions": [],
                    "example_quotes": commits[:3],
                    "last_seen": datetime.now(timezone.utc).isoformat(),
                }
            )

    # Check for config file changes (style preference evolution)
    for config_file in STYLE_CONFIG_FILES:
        cmd = ["git", "log", "--oneline", "-10", "--", config_file]
        if since:
            cmd.insert(4, f"--since={since}")
        output = _run_cmd(cmd, cwd=cwd)
        if output and output.strip():
            changes = [
                line_text
                for line_text in output.strip().split("\n")
                if line_text.strip()
            ]
            if len(changes) >= 2:
                patterns.append(
                    {
                        "description": f"Frequent changes to {config_file} (style preference evolution)",
                        "source": "git",
                        "occurrences": len(changes),
                        "confidence": "medium",
                        "sessions": [],
                        "example_quotes": changes[:3],
                        "last_seen": datetime.now(timezone.utc).isoformat(),
                    }
                )

    return patterns, stats


# --- Source 3: GitHub PR Comments ---


def _resolve_gh_repo(project_dir: Path) -> str | None:
    """Resolve the GitHub owner/repo from git remote.

    Returns 'owner/repo' string or None if not a GitHub repo.
    """
    output = _run_cmd(
        ["gh", "repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner"],
        cwd=str(project_dir),
    )
    if output:
        return output.strip()
    return None


def mine_pr_comments(
    project_dir: Path,
    since: str | None = None,
) -> tuple[list[dict], dict]:
    """Fetch GitHub PR review comments for convention feedback."""
    patterns: list[dict] = []
    stats = {"comments": 0}

    if not _has_command("gh"):
        return patterns, stats

    cwd = str(project_dir)

    # Resolve repo identity
    repo_slug = _resolve_gh_repo(project_dir)
    if not repo_slug:
        return patterns, stats

    # Get recent closed PRs
    cmd = [
        "gh",
        "pr",
        "list",
        "--state",
        "closed",
        "--limit",
        "20",
        "--json",
        "number,title,closedAt",
    ]
    output = _run_cmd(cmd, cwd=cwd)
    if not output:
        return patterns, stats

    try:
        prs = json.loads(output)
    except json.JSONDecodeError:
        return patterns, stats

    # Collect review comments
    style_comments: dict[str, list[str]] = defaultdict(list)

    for pr in prs:
        pr_num = pr.get("number")
        if not pr_num:
            continue

        cmd = [
            "gh",
            "api",
            f"repos/{repo_slug}/pulls/{pr_num}/comments",
            "--jq",
            ".[].body",
        ]
        output = _run_cmd(cmd, cwd=cwd)
        if not output:
            continue

        for comment in output.strip().split("\n"):
            if not comment.strip():
                continue
            stats["comments"] += 1

            # Check for style/convention language
            if STYLE_KEYWORDS.search(comment) or any(
                p.search(comment) for p in DIRECTIVE_PATTERNS + CORRECTION_PATTERNS
            ):
                key = _extract_pattern_key(comment)
                style_comments[key].append(comment[:200])

    for desc, comments in style_comments.items():
        if len(comments) >= 2:
            patterns.append(
                {
                    "description": desc,
                    "source": "pr",
                    "occurrences": len(comments),
                    "confidence": "high" if len(comments) >= 3 else "medium",
                    "sessions": [],
                    "example_quotes": comments[:3],
                    "last_seen": datetime.now(timezone.utc).isoformat(),
                }
            )

    return patterns, stats


# --- Pattern processing ---


def apply_strictness_filters(
    patterns: list[dict],
    min_occurrences: int = 2,
) -> list[dict]:
    """Apply strictness filters: frequency floor, recency bias, dedup."""
    filtered = []

    for p in patterns:
        # Frequency floor
        if p["occurrences"] < min_occurrences:
            continue

        # Recency bias scoring
        score = p["occurrences"]
        try:
            last_seen = datetime.fromisoformat(p["last_seen"])
            days_ago = (datetime.now(timezone.utc) - last_seen).days
            if days_ago <= 30:
                score *= 3
            elif days_ago <= 90:
                score *= 1.5
        except (ValueError, TypeError):
            pass

        p["score"] = score
        filtered.append(p)

    # Sort by score descending
    filtered.sort(key=lambda x: x.get("score", 0), reverse=True)

    # Deduplicate by rough description similarity
    seen_keys: set[str] = set()
    deduped: list[dict] = []
    for p in filtered:
        key = p["description"].lower()[:50]
        if key not in seen_keys:
            seen_keys.add(key)
            deduped.append(p)

    return deduped


def add_suggested_rules(patterns: list[dict]) -> list[dict]:
    """Add suggested_rule to patterns that are actionable."""
    for p in patterns:
        desc = p["description"]
        p["suggested_rule"] = {
            "description": f"Code SHOULD follow the convention: {desc}",
            "severity": "should",
            "category": "convention",
            "signals": [
                {
                    "strategy": "pattern",
                    "description": f"Check for adherence to: {desc}",
                }
            ],
        }
    return patterns


# --- Main ---


def detect_patterns(
    project_dir: Path,
    sources: set[str],
    since: str | None = None,
    since_last_run: bool = False,
    quiet: bool = False,
    min_occurrences: int = 2,
    global_transcripts: bool = False,
) -> dict:
    """Main pattern detection logic."""
    # Resolve --since-last-run
    effective_since = since
    last_run_file = project_dir / "whetstone" / ".last-run"
    if since_last_run and last_run_file.exists():
        try:
            ts = last_run_file.read_text().strip()
            # Use it as git --since argument
            effective_since = ts
        except OSError:
            pass

    # Convert since string to datetime for transcript filtering
    since_dt: datetime | None = None
    if effective_since:
        try:
            since_dt = datetime.fromisoformat(effective_since)
        except ValueError:
            # Git-style relative dates ("7 days ago") — approximate
            m = re.match(
                r"(\d+)\s+(day|week|month)s?\s+ago", effective_since, re.IGNORECASE
            )
            if m:
                n = int(m.group(1))
                unit = m.group(2).lower()
                if unit == "day":
                    since_dt = datetime.now(timezone.utc) - timedelta(days=n)
                elif unit == "week":
                    since_dt = datetime.now(timezone.utc) - timedelta(weeks=n)
                elif unit == "month":
                    since_dt = datetime.now(timezone.utc) - timedelta(days=n * 30)

    all_patterns: list[dict] = []
    sources_analyzed: dict[str, dict] = {}

    if "transcript" in sources:
        if global_transcripts:
            print(
                "WARNING: --global-transcripts scans ALL agent transcripts across "
                "your home directory. This may include conversations from other "
                "projects. Use with care.",
                file=sys.stderr,
            )
        pats, stats = mine_transcripts(
            project_dir, since=since_dt, global_transcripts=global_transcripts
        )
        all_patterns.extend(pats)
        sources_analyzed["transcript"] = stats

    if "git" in sources:
        pats, stats = mine_git_history(project_dir, since=effective_since)
        all_patterns.extend(pats)
        sources_analyzed["git"] = stats

    if "pr" in sources:
        pats, stats = mine_pr_comments(project_dir, since=effective_since)
        all_patterns.extend(pats)
        sources_analyzed["pr"] = stats

    # Apply filters
    filtered = apply_strictness_filters(all_patterns, min_occurrences)
    filtered = add_suggested_rules(filtered)

    # Quiet mode: only output if patterns found
    if quiet and not filtered:
        return {
            "patterns": [],
            "sources_analyzed": sources_analyzed,
            "next_command": "No patterns found. Proceed to extraction.",
        }

    # Update .last-run timestamp
    try:
        whetstone_dir = project_dir / "whetstone"
        whetstone_dir.mkdir(parents=True, exist_ok=True)
        last_run_file.write_text(datetime.now(timezone.utc).isoformat())
    except OSError:
        pass

    if filtered:
        next_command = "Review patterns and approve as rules during extraction"
    else:
        next_command = "No patterns found. Proceed to extraction."

    return {
        "patterns": filtered,
        "sources_analyzed": sources_analyzed,
        "next_command": next_command,
    }


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Mine style patterns from transcripts, git, and PRs."
    )
    parser.add_argument(
        "--project-dir",
        type=Path,
        default=Path("."),
        help="Project root directory (default: .)",
    )
    parser.add_argument(
        "--since-last-run",
        action="store_true",
        help="Only analyze data since last execution",
    )
    parser.add_argument(
        "--since",
        type=str,
        help='Time-bounded analysis (e.g., "7 days ago", ISO date)',
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="Only output if new patterns found",
    )
    parser.add_argument(
        "--sources",
        type=str,
        default="transcript,git,pr",
        help="Comma-separated sources to mine (default: transcript,git,pr)",
    )
    parser.add_argument(
        "--min-occurrences",
        type=int,
        default=2,
        help="Minimum occurrences to report (default: 2)",
    )
    parser.add_argument(
        "--global-transcripts",
        action="store_true",
        help="Scan all agent transcripts, not just those matching this project "
        "(privacy: by default only project-scoped transcripts are read)",
    )
    args = parser.parse_args()

    try:
        source_set = set(args.sources.split(","))
        valid_sources = {"transcript", "git", "pr"}
        source_set &= valid_sources

        if not source_set:
            json.dump(
                {"error": "No valid sources specified. Use: transcript, git, pr"},
                sys.stdout,
                indent=2,
            )
            sys.stdout.write("\n")
            sys.exit(1)

        result = detect_patterns(
            project_dir=args.project_dir,
            sources=source_set,
            since=args.since,
            since_last_run=args.since_last_run,
            quiet=args.quiet,
            min_occurrences=args.min_occurrences,
            global_transcripts=args.global_transcripts,
        )

        json.dump(result, sys.stdout, indent=2)
        sys.stdout.write("\n")

    except Exception as e:
        json.dump(
            {
                "error": str(e),
                "next_command": "Check project directory and source availability",
            },
            sys.stdout,
            indent=2,
        )
        sys.stdout.write("\n")
        sys.exit(1)


if __name__ == "__main__":
    main()
