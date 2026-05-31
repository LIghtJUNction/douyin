"""Command line entry point for douyin-cli."""

from __future__ import annotations

import sys


def _prefer_utf8_stdio() -> None:
    for stream in (sys.stdout, sys.stderr):
        reconfigure = getattr(stream, "reconfigure", None)
        if reconfigure is None:
            continue
        try:
            reconfigure(encoding="utf-8")
        except (OSError, ValueError):
            continue


_prefer_utf8_stdio()

try:
    from douyin_cli.commands.root import main
except ImportError as exc:  # pragma: no cover
    if exc.name == "click":
        raise SystemExit(
            "douyin CLI dependencies are not installed. "
            "Install with: uv tool install douyin-cli"
        ) from exc
    raise

__all__ = ["main"]


if __name__ == "__main__":
    main()
