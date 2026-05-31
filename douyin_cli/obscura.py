"""Obscura integration helpers."""

from __future__ import annotations

import shutil
import subprocess
from pathlib import Path
from typing import Any


def build_obscura_manifest(
    *,
    version: str,
    config_file: Path,
) -> dict[str, Any]:
    """Return a machine-readable integration manifest for Obscura."""
    return {
        "name": "douyin-cli",
        "version": version,
        "homepage": "https://github.com/LIghtJUNction/douyin",
        "entrypoint": "douyin",
        "configFile": str(config_file),
        "auth": {
            "type": "oauth2",
            "login": ["douyin", "auth", "login"],
            "exchangeCode": ["douyin", "auth", "code", "--code", "<code>"],
            "status": ["douyin", "auth", "status", "--json"],
            "refresh": ["douyin", "auth", "refresh"],
            "logout": ["douyin", "auth", "logout"],
        },
        "openapi": {
            "output": "json",
            "tokenSource": "saved-oauth-config-or-env",
            "commands": {
                "userinfo": ["douyin", "api", "userinfo"],
                "commentList": [
                    "douyin",
                    "api",
                    "comment-list",
                    "--item-id",
                    "<item_id>",
                ],
                "commentReplies": [
                    "douyin",
                    "api",
                    "comment-replies",
                    "--item-id",
                    "<item_id>",
                    "--comment-id",
                    "<comment_id>",
                ],
                "commentReply": [
                    "douyin",
                    "api",
                    "comment-reply",
                    "--item-id",
                    "<item_id>",
                    "--comment-id",
                    "<comment_id>",
                    "--content",
                    "<content>",
                    "--yes",
                ],
                "imMessageSend": [
                    "douyin",
                    "api",
                    "im-message-send",
                    "--to-user-id",
                    "<to_user_id>",
                    "--text",
                    "<text>",
                    "--yes",
                ],
                "request": ["douyin", "api", "request", "<method>", "<path>"],
            },
        },
        "environment": {
            "clientKey": "DOUYIN_CLIENT_KEY",
            "clientSecret": "DOUYIN_CLIENT_SECRET",
            "accessToken": "DOUYIN_ACCESS_TOKEN",
            "home": "DOUYIN_HOME",
        },
    }


def detect_obscura(binary: str = "obscura") -> dict[str, Any]:
    """Detect a local Obscura executable without requiring it as a dependency."""
    executable = shutil.which(binary)
    if executable is None:
        return {
            "available": False,
            "binary": binary,
            "path": None,
            "version": None,
        }

    version = _read_obscura_version(executable)
    return {
        "available": True,
        "binary": binary,
        "path": executable,
        "version": version,
    }


def _read_obscura_version(executable: str) -> str | None:
    try:
        result = subprocess.run(
            [executable, "--version"],
            capture_output=True,
            check=False,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None

    text = (result.stdout or result.stderr).strip()
    return text or None
