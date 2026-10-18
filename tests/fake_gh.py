#!/usr/bin/env python3
"""A stand-in for the `gh` CLI with just enough of issues, pull requests and
labels for the runner's tests. State lives in the JSON file $FAKE_GH_STATE."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path


def option(args: list[str], name: str) -> list[str]:
    return [args[index + 1] for index, arg in enumerate(args[:-1]) if arg == name]


def main() -> int:
    path = Path(os.environ["FAKE_GH_STATE"])
    state = (
        json.loads(path.read_text())
        if path.exists()
        else {"issue": [], "pr": [], "labels": []}
    )
    args = sys.argv[1:]
    noun, verb, rest = args[0], args[1], args[2:]
    if noun == "label" and verb == "create":
        if rest[0] not in state["labels"]:
            state["labels"].append(rest[0])
    elif noun in ("issue", "pr") and verb == "list":
        wanted = option(rest, "--label")
        listed = [
            {"number": item["number"], "state": item["state"], "body": item["body"]}
            for item in state[noun]
            if all(label in item["labels"] for label in wanted)
        ]
        print(json.dumps(listed))
    elif noun in ("issue", "pr") and verb == "create":
        labels = option(rest, "--label")
        missing = [label for label in labels if label not in state["labels"]]
        if missing:
            print(f"could not add label: {missing[0]} not found", file=sys.stderr)
            return 1
        number = 1 + len(state["issue"]) + len(state["pr"])
        item = {
            "number": number,
            "state": "OPEN",
            "title": option(rest, "--title")[0],
            "body": Path(option(rest, "--body-file")[0]).read_text(),
            "labels": labels,
        }
        if noun == "pr":
            item["draft"] = "--draft" in rest
            item["head"] = option(rest, "--head")[0]
            item["base"] = option(rest, "--base")[0]
        state[noun].append(item)
        print(f"https://github.invalid/{noun}/{number}")
    elif noun in ("issue", "pr") and verb == "edit":
        number = int(rest[0])
        for item in state[noun]:
            if item["number"] == number:
                item["body"] = Path(option(rest, "--body-file")[0]).read_text()
                item["edits"] = item.get("edits", 0) + 1
    else:
        print(f"fake gh does not know {noun} {verb}", file=sys.stderr)
        return 1
    path.write_text(json.dumps(state, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
