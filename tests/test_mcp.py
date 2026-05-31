import asyncio

import ujson as json
from click.testing import CliRunner

from douyin_cli import mcp_server
from douyin_cli.cli import main
from douyin_cli.commands import common
from douyin_cli.commands import mcp as mcp_command


def test_mcp_command_is_registered() -> None:
    result = CliRunner().invoke(main, ["--help"])

    assert result.exit_code == 0
    assert "mcp" in result.output
    assert "stdio" in result.output


def test_mcp_command_runs_stdio_server(monkeypatch) -> None:
    called = False

    def fake_run_stdio_server() -> None:
        nonlocal called
        called = True

    monkeypatch.setattr(mcp_command, "run_stdio_server", fake_run_stdio_server)

    result = CliRunner().invoke(main, ["mcp"])

    assert result.exit_code == 0
    assert called is True


def test_mcp_server_exposes_openapi_tools() -> None:
    server = mcp_server.create_mcp_server()

    async def list_tool_names() -> set[str]:
        tools = await server.list_tools()
        return {tool.name for tool in tools}

    assert {
        "auth_status",
        "userinfo",
        "comment_list",
        "comment_replies",
        "comment_reply",
        "im_message_send",
        "openapi_request",
    }.issubset(asyncio.run(list_tool_names()))


def test_mcp_userinfo_uses_saved_openapi_auth(monkeypatch) -> None:
    monkeypatch.setattr(
        common.settings,
        "_settings",
        {
            "openapi": {
                "accessToken": "saved-token",
                "openId": "saved-open-id",
            },
        },
    )

    class DummyClient:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
            return False

        def userinfo(self, token: str, open_id: str) -> dict:
            assert token == "saved-token"
            assert open_id == "saved-open-id"
            return {"data": {"nickname": "tester"}}

    monkeypatch.setattr(mcp_server, "DouyinOpenAPIClient", DummyClient)
    server = mcp_server.create_mcp_server()

    async def call_userinfo() -> dict:
        content, _structured = await server.call_tool("userinfo", {})
        return json.loads(content[0].text)

    assert asyncio.run(call_userinfo()) == {"data": {"nickname": "tester"}}
