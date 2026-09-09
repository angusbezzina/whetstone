import os
import shutil
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "test-dashboard-browser.mjs"


def test_dashboard_in_a_real_browser():
    if shutil.which("node") is None:
        pytest.skip("Node.js is unavailable for the dependency-free CDP harness")
    chrome = os.environ.get("CHROME_BIN") or next(
        (
            path
            for path in (
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/usr/bin/google-chrome",
                "/usr/bin/google-chrome-stable",
                "/usr/bin/chromium",
                "/usr/bin/chromium-browser",
            )
            if Path(path).exists()
        ),
        None,
    )
    if chrome is None:
        pytest.skip("Chrome/Chromium is unavailable")
    result = subprocess.run(
        ["node", str(SCRIPT)],
        cwd=ROOT,
        env={**os.environ, "CHROME_BIN": chrome},
        check=False,
        capture_output=True,
        text=True,
        timeout=30,
    )
    assert result.returncode == 0, result.stdout + result.stderr
