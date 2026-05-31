"""Official OpenAPI commands."""

from __future__ import annotations

import click

from douyin_cli.commands.common import (
    echo_json,
    get_openapi_config,
    parse_json_body,
    parse_key_values,
    resolve_openapi_auth,
)
from douyin_cli.douyin.openapi import DouyinOpenAPIClient, DouyinOpenAPIError


@click.group()
def api() -> None:
    """调用抖音开放平台官方 OpenAPI."""


@api.command("client-token")
@click.option("--client-key", envvar="DOUYIN_CLIENT_KEY", required=True)
@click.option("--client-secret", envvar="DOUYIN_CLIENT_SECRET", required=True)
def client_token(client_key: str, client_secret: str) -> None:
    """获取 client_token."""
    with DouyinOpenAPIClient() as client:
        echo_json(client.client_token(client_key, client_secret))


@api.command("authorize-url")
@click.option("--client-key", envvar="DOUYIN_CLIENT_KEY", required=True)
@click.option("--redirect-uri", required=True, help="OAuth 回调地址")
@click.option(
    "--scope",
    "scopes",
    multiple=True,
    required=True,
    help="授权 scope，可多次传入",
)
@click.option("--state", help="OAuth state")
def authorize_url(
    client_key: str,
    redirect_uri: str,
    scopes: tuple[str, ...],
    state: str | None,
) -> None:
    """生成官方 OAuth 授权链接."""
    with DouyinOpenAPIClient() as client:
        click.echo(client.authorize_url(client_key, redirect_uri, list(scopes), state))


@api.command("access-token")
@click.option("--client-key", envvar="DOUYIN_CLIENT_KEY", required=True)
@click.option("--client-secret", envvar="DOUYIN_CLIENT_SECRET", required=True)
@click.option("--code", required=True, help="OAuth 授权码")
def access_token(client_key: str, client_secret: str, code: str) -> None:
    """用 OAuth code 换取 access_token."""
    with DouyinOpenAPIClient() as client:
        echo_json(client.access_token(client_key, client_secret, code))


@api.command("refresh-token")
@click.option("--client-key", envvar="DOUYIN_CLIENT_KEY", required=True)
@click.option("--refresh-token", required=True)
def refresh_token(client_key: str, refresh_token: str) -> None:
    """刷新官方 access_token."""
    with DouyinOpenAPIClient() as client:
        echo_json(client.refresh_token(client_key, refresh_token))


@api.command("renew-refresh-token")
@click.option("--client-key", envvar="DOUYIN_CLIENT_KEY", required=True)
@click.option("--refresh-token", required=True)
def renew_refresh_token(client_key: str, refresh_token: str) -> None:
    """续期官方 refresh_token."""
    with DouyinOpenAPIClient() as client:
        echo_json(client.renew_refresh_token(client_key, refresh_token))


@api.command("userinfo")
@click.option("--token", envvar="DOUYIN_ACCESS_TOKEN", help="默认读取已保存 token")
@click.option("--open-id", help="默认读取已保存 open_id")
def userinfo(token: str | None, open_id: str | None) -> None:
    """获取官方授权用户信息."""
    token, open_id = resolve_openapi_auth(token, open_id)
    with DouyinOpenAPIClient() as client:
        echo_json(client.userinfo(token, open_id))


@api.command("comment-list")
@click.option("--token", envvar="DOUYIN_ACCESS_TOKEN", help="默认读取已保存 token")
@click.option("--open-id", help="默认读取已保存 open_id")
@click.option("--item-id", required=True, help="官方接口返回的视频 item_id")
@click.option("--cursor", default=0, show_default=True)
@click.option("--count", default=20, show_default=True)
def comment_list(
    token: str | None,
    open_id: str | None,
    item_id: str,
    cursor: int,
    count: int,
) -> None:
    """调用官方接口获取视频评论列表."""
    token, open_id = resolve_openapi_auth(token, open_id)
    with DouyinOpenAPIClient() as client:
        echo_json(client.comment_list(token, open_id, item_id, cursor, count))


@api.command("comment-replies")
@click.option("--token", envvar="DOUYIN_ACCESS_TOKEN", help="默认读取已保存 token")
@click.option("--open-id", help="默认读取已保存 open_id")
@click.option("--item-id", required=True, help="官方接口返回的视频 item_id")
@click.option("--comment-id", required=True)
@click.option("--cursor", default=0, show_default=True)
@click.option("--count", default=20, show_default=True)
def comment_replies(
    token: str | None,
    open_id: str | None,
    item_id: str,
    comment_id: str,
    cursor: int,
    count: int,
) -> None:
    """调用官方接口获取评论回复列表."""
    token, open_id = resolve_openapi_auth(token, open_id)
    with DouyinOpenAPIClient() as client:
        echo_json(
            client.comment_replies(token, open_id, item_id, comment_id, cursor, count),
        )


@api.command("comment-reply")
@click.option("--token", envvar="DOUYIN_ACCESS_TOKEN", help="默认读取已保存 token")
@click.option("--open-id", help="默认读取已保存 open_id")
@click.option("--item-id", required=True, help="官方接口返回的视频 item_id")
@click.option("--comment-id", help="被回复的评论 ID；不传则回复视频")
@click.option("--content", required=True, help="回复内容")
@click.option("--yes", is_flag=True, help="跳过写操作确认")
def comment_reply(
    token: str | None,
    open_id: str | None,
    item_id: str,
    comment_id: str | None,
    content: str,
    yes: bool,
) -> None:
    """调用官方接口回复视频评论."""
    token, open_id = resolve_openapi_auth(token, open_id)
    if not yes:
        click.confirm("将通过官方 OpenAPI 发送评论回复，是否继续？", abort=True)
    with DouyinOpenAPIClient() as client:
        echo_json(client.reply_comment(token, open_id, item_id, content, comment_id))


@api.command("im-message-send")
@click.option("--token", envvar="DOUYIN_ACCESS_TOKEN", help="默认读取已保存 token")
@click.option("--open-id", help="默认读取已保存 open_id")
@click.option("--to-user-id", required=True, help="私信接收方 open_id/to_user_id")
@click.option(
    "--message-type",
    type=click.Choice(["text", "image", "video", "card"]),
    default="text",
    show_default=True,
)
@click.option("--text", help="文本消息内容；message-type=text 时使用")
@click.option("--media-id", help="图片素材 media_id；message-type=image 时使用")
@click.option("--item-id", help="已审核通过的视频 item_id；message-type=video 时使用")
@click.option("--card-id", help="企业消息卡片 card_id；message-type=card 时使用")
@click.option("--persona-id", help="客服 persona_id；不传则走普通会话")
@click.option("--client-msg-id", help="调用方自定义消息 ID")
@click.option("--yes", is_flag=True, help="跳过写操作确认")
def im_message_send(
    token: str | None,
    open_id: str | None,
    to_user_id: str,
    message_type: str,
    text: str | None,
    media_id: str | None,
    item_id: str | None,
    card_id: str | None,
    persona_id: str | None,
    client_msg_id: str | None,
    yes: bool,
) -> None:
    """调用企业号 OpenAPI 发送私信消息."""
    token, open_id = resolve_openapi_auth(token, open_id)
    content = _build_im_message_content(message_type, text, media_id, item_id, card_id)
    if not yes:
        click.confirm("将通过企业号 OpenAPI 发送私信消息，是否继续？", abort=True)
    with DouyinOpenAPIClient() as client:
        echo_json(
            client.send_im_message(
                token,
                open_id,
                to_user_id,
                message_type,
                content,
                persona_id=persona_id,
                client_msg_id=client_msg_id,
            ),
        )


def _build_im_message_content(
    message_type: str,
    text: str | None,
    media_id: str | None,
    item_id: str | None,
    card_id: str | None,
) -> dict:
    if message_type == "text":
        if not text:
            raise click.ClickException("message-type=text 需要 --text")
        return {"text": text}
    if message_type == "image":
        if not media_id:
            raise click.ClickException("message-type=image 需要 --media-id")
        return {"media_id": media_id}
    if message_type == "video":
        if not item_id:
            raise click.ClickException("message-type=video 需要 --item-id")
        return {"item_id": item_id}
    if message_type == "card":
        if not card_id:
            raise click.ClickException("message-type=card 需要 --card-id")
        return {"card_id": card_id}
    raise click.ClickException(f"不支持的私信类型: {message_type}")


@api.command("request")
@click.argument("method")
@click.argument("path")
@click.option("--token", envvar="DOUYIN_ACCESS_TOKEN", help="access_token/client_token")
@click.option("--param", "params", multiple=True, help="查询参数，格式 key=value")
@click.option("--json", "json_text", help="JSON 请求体")
@click.option("--form", "forms", multiple=True, help="表单参数，格式 key=value")
@click.option("--header", "headers", multiple=True, help="额外请求头，格式 key=value")
def request(
    method: str,
    path: str,
    token: str | None,
    params: tuple[str, ...],
    json_text: str | None,
    forms: tuple[str, ...],
    headers: tuple[str, ...],
) -> None:
    """调用任意官方 OpenAPI 路径."""
    try:
        token = token or get_openapi_config().get("accessToken") or None
        json_body = parse_json_body(json_text)
        with DouyinOpenAPIClient() as client:
            echo_json(
                client.request(
                    method,
                    path,
                    token=token,
                    params=parse_key_values(params),
                    json_body=json_body,
                    form=parse_key_values(forms),
                    headers=parse_key_values(headers),
                ),
            )
    except (DouyinOpenAPIError, ValueError) as exc:
        raise click.ClickException(str(exc)) from exc
