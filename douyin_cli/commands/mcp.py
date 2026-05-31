"""MCP server command."""

from __future__ import annotations

import click

from douyin_cli.mcp_server import run_stdio_server


@click.command("mcp")
def mcp() -> None:
    """通过 stdio 启动抖音 MCP 服务器."""
    run_stdio_server()
