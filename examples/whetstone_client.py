"""Minimal client for the versioned Whetstone command envelope."""

from __future__ import annotations

import json
import subprocess
import sys


result = subprocess.run(
    ["wh", "check", "--json", *sys.argv[1:]],
    check=False,
    capture_output=True,
    text=True,
)
response = json.loads(result.stdout)
state = response["state"]

if state == "success":
    raise SystemExit(0)
if state == "violated":
    print(json.dumps(response, indent=2))
    raise SystemExit(1)
if state in {
    "unknown",
    "unavailable",
    "needs_decision",
    "needs_input",
    "stale",
    "conflict",
}:
    print(json.dumps(response, indent=2))
    raise SystemExit(2)
raise RuntimeError(f"unsupported Whetstone state: {state}")
