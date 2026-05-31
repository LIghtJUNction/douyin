"""Obscura integration commands."""

from __future__ import annotations

import click

from douyin_cli.commands.common import APP_VERSION, echo_json, get_openapi_config
from douyin_cli.obscura import build_obscura_manifest, detect_obscura
from douyin_cli.settings import SETTINGS_FILE


@click.group()
def obscura() -> None:
    """Obscura 集成辅助命令."""


@obscura.command("manifest")
def manifest() -> None:
    """输出 Obscura 集成 manifest."""
    echo_json(
        build_obscura_manifest(
            version=APP_VERSION,
            config_file=SETTINGS_FILE,
        ),
    )


@obscura.command("status")
@click.option(
    "--binary",
    default="obscura",
    show_default=True,
    help="Obscura 可执行文件名",
)
def status(binary: str) -> None:
    """检查本地 Obscura 集成状态."""
    config = get_openapi_config()
    echo_json(
        {
            "douyin": {
                "version": APP_VERSION,
                "entrypoint": "douyin",
                "configFile": str(SETTINGS_FILE),
                "authorized": bool(
                    config.get("accessToken") and config.get("openId"),
                ),
            },
            "obscura": detect_obscura(binary),
            "next": {
                "auth": ["douyin", "auth", "login"],
                "machineStatus": ["douyin", "auth", "status", "--json"],
                "manifest": ["douyin", "obscura", "manifest"],
            },
        },
    )
