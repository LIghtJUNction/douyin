from __future__ import annotations

import os
import subprocess
import sys


def test_help_works_when_stdio_starts_as_cp1252() -> None:
    env = {**os.environ, "PYTHONIOENCODING": "cp1252"}

    result = subprocess.run(
        [sys.executable, "-m", "douyin_cli.cli", "--help"],
        check=False,
        capture_output=True,
        env=env,
    )

    assert result.returncode == 0
    assert "抖音 CLI".encode() in result.stdout
