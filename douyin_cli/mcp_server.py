"""MCP server for Douyin OpenAPI tools."""

from __future__ import annotations

from typing import Any

from mcp.server.fastmcp import FastMCP

from douyin_cli.commands.api import _build_im_message_content
from douyin_cli.commands.common import (
    get_openapi_config,
    resolve_openapi_auth,
)
from douyin_cli.douyin.openapi import DouyinOpenAPIClient


def create_mcp_server() -> FastMCP:
    """Create the stdio MCP server used by ``douyin mcp``."""
    server = FastMCP(
        "douyin",
        instructions=(
            "抖音开放平台 OpenAPI MCP 服务器。默认读取 douyin auth 保存的 "
            "access_token/open_id，也可以在工具参数中显式传入。"
        ),
    )

    @server.tool()
    def auth_status() -> dict[str, Any]:
        """查看本机是否已保存抖音开放平台授权信息。"""
        config = get_openapi_config()
        return {
            "authorized": bool(config.get("accessToken") and config.get("openId")),
            "client_key_saved": bool(config.get("clientKey")),
            "open_id": config.get("openId") or None,
            "scopes": config.get("scopes") or [],
            "expires_in": config.get("expiresIn") or 0,
        }

    @server.tool()
    def userinfo(
        token: str | None = None,
        open_id: str | None = None,
    ) -> dict[str, Any]:
        """获取官方授权用户信息。"""
        token, open_id = resolve_openapi_auth(token, open_id)
        with DouyinOpenAPIClient() as client:
            return client.userinfo(token, open_id)

    @server.tool()
    def comment_list(
        item_id: str,
        cursor: int = 0,
        count: int = 20,
        token: str | None = None,
        open_id: str | None = None,
    ) -> dict[str, Any]:
        """获取官方接口中的视频评论列表。"""
        token, open_id = resolve_openapi_auth(token, open_id)
        with DouyinOpenAPIClient() as client:
            return client.comment_list(token, open_id, item_id, cursor, count)

    @server.tool()
    def comment_replies(
        item_id: str,
        comment_id: str,
        cursor: int = 0,
        count: int = 20,
        token: str | None = None,
        open_id: str | None = None,
    ) -> dict[str, Any]:
        """获取官方接口中的评论回复列表。"""
        token, open_id = resolve_openapi_auth(token, open_id)
        with DouyinOpenAPIClient() as client:
            return client.comment_replies(
                token,
                open_id,
                item_id,
                comment_id,
                cursor,
                count,
            )

    @server.tool()
    def comment_reply(
        item_id: str,
        content: str,
        comment_id: str | None = None,
        token: str | None = None,
        open_id: str | None = None,
    ) -> dict[str, Any]:
        """通过官方 OpenAPI 回复视频或评论。"""
        token, open_id = resolve_openapi_auth(token, open_id)
        with DouyinOpenAPIClient() as client:
            return client.reply_comment(token, open_id, item_id, content, comment_id)

    @server.tool()
    def im_message_send(
        to_user_id: str,
        message_type: str = "text",
        text: str | None = None,
        media_id: str | None = None,
        item_id: str | None = None,
        card_id: str | None = None,
        persona_id: str | None = None,
        client_msg_id: str | None = None,
        token: str | None = None,
        open_id: str | None = None,
    ) -> dict[str, Any]:
        """通过企业号 OpenAPI 发送私信消息。"""
        token, open_id = resolve_openapi_auth(token, open_id)
        content = _build_im_message_content(
            message_type,
            text,
            media_id,
            item_id,
            card_id,
        )
        with DouyinOpenAPIClient() as client:
            return client.send_im_message(
                token,
                open_id,
                to_user_id,
                message_type,
                content,
                persona_id=persona_id,
                client_msg_id=client_msg_id,
            )

    @server.tool()
    def openapi_request(
        method: str,
        path: str,
        token: str | None = None,
        params: dict[str, str] | None = None,
        json_body: dict[str, Any] | list[Any] | None = None,
        form: dict[str, str] | None = None,
        headers: dict[str, str] | None = None,
    ) -> dict[str, Any]:
        """调用任意官方 OpenAPI 路径。"""
        token = token or get_openapi_config().get("accessToken") or None
        with DouyinOpenAPIClient() as client:
            return client.request(
                method,
                path,
                token=token,
                params=params,
                json_body=json_body,
                form=form,
                headers=headers,
            )

    return server


def run_stdio_server() -> None:
    """Run the Douyin MCP server over stdio."""
    create_mcp_server().run(transport="stdio")
