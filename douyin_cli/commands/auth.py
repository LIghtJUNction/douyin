"""Official OAuth auth commands."""

from __future__ import annotations

import secrets
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

import click
import qrcode

from douyin_cli.commands.common import (
    DEFAULT_OPENAPI_SCOPES,
    echo_json,
    extract_openapi_token_fields,
    get_openapi_config,
    save_openapi_config,
)
from douyin_cli.cookies import CookieManager
from douyin_cli.douyin.openapi import DouyinOpenAPIClient, DouyinOpenAPIError
from douyin_cli.settings import SETTINGS_FILE, settings


@click.group()
def auth() -> None:
    """管理授权。

    \b
    网页端 Cookie 流程：
      douyin auth cookie-login
      douyin auth cookie-status
      douyin -u "搜索关键词" -t search -l 5 --no-download
      douyin auth cookie-logout

    \b
    官方 OpenAPI OAuth 流程：
      douyin auth login --client-key KEY --redirect-uri URI
      douyin auth code --code CODE --client-secret SECRET
      douyin auth status
    """


@auth.command("login")
@click.option("--client-key", envvar="DOUYIN_CLIENT_KEY", help="开放平台 client_key")
@click.option(
    "--client-secret",
    envvar="DOUYIN_CLIENT_SECRET",
    help="开放平台 client_secret",
)
@click.option("--redirect-uri", help="开放平台应用回调地址")
@click.option("--scope", "scopes", multiple=True, help="授权 scope，可多次传入")
@click.option("--code", help="授权回调得到的 code；传入后会直接换取 token")
@click.option(
    "--qr/--no-qr",
    default=True,
    show_default=True,
    help="在终端显示授权链接二维码",
)
@click.option(
    "--listen/--no-listen",
    default=False,
    help="在本机监听回调并自动捕获 code",
)
@click.option(
    "--callback-host",
    default="127.0.0.1",
    show_default=True,
    help="--listen 使用的本机监听地址",
)
@click.option(
    "--callback-port",
    default=8787,
    show_default=True,
    type=click.IntRange(1, 65535),
    help="--listen 使用的本机监听端口",
)
@click.option(
    "--timeout",
    default=300,
    show_default=True,
    type=click.IntRange(1, 3600),
    help="--listen 等待授权回调的秒数",
)
def login(
    client_key: str | None,
    client_secret: str | None,
    redirect_uri: str | None,
    scopes: tuple[str, ...],
    code: str | None,
    qr: bool,
    listen: bool,
    callback_host: str,
    callback_port: int,
    timeout: int,
) -> None:
    """通过官方 OAuth 授权接入账号."""
    config = get_openapi_config()
    client_key = client_key or config.get("clientKey")
    client_secret = client_secret or config.get("clientSecret")
    redirect_uri = redirect_uri or config.get("redirectUri")
    selected_scopes = list(scopes or config.get("scopes") or DEFAULT_OPENAPI_SCOPES)
    state = secrets.token_urlsafe(16) if listen and not code else None

    if listen:
        redirect_uri = f"http://{callback_host}:{callback_port}/callback"

    if not client_key:
        raise click.ClickException(_missing_client_key_message(qr=qr))
    if not redirect_uri:
        raise click.ClickException("缺少 redirect_uri，请传入 --redirect-uri")

    with DouyinOpenAPIClient() as client:
        auth_url = client.authorize_url(
            client_key, redirect_uri, selected_scopes, state
        )
        click.echo("请在浏览器打开以下官方授权链接：")
        click.echo(auth_url)
        if qr:
            _print_qr(auth_url)

        updates = {
            "clientKey": client_key,
            "clientSecret": client_secret or "",
            "redirectUri": redirect_uri,
            "scopes": selected_scopes,
        }
        if listen and not code:
            click.echo(f"正在等待授权回调: {redirect_uri}")
            code = _wait_for_oauth_code(callback_host, callback_port, state, timeout)
        if code:
            if not client_secret:
                raise click.ClickException("使用 code 换 token 需要 --client-secret")
            token_data = client.access_token(client_key, client_secret, code)
            updates.update(extract_openapi_token_fields(token_data))
            echo_json(token_data)
        save_openapi_config(updates)

    if not code:
        click.echo("授权完成后运行：douyin auth code --code 授权码")
    click.echo(f"官方授权配置已保存: {SETTINGS_FILE}")


@auth.command("code")
@click.option("--code", required=True, help="授权回调得到的 code")
@click.option(
    "--client-secret",
    envvar="DOUYIN_CLIENT_SECRET",
    help="开放平台 client_secret",
)
def code(code: str, client_secret: str | None) -> None:
    """用官方 OAuth code 换取并保存 token."""
    config = get_openapi_config()
    client_key = config.get("clientKey")
    client_secret = client_secret or config.get("clientSecret")
    if not client_key:
        raise click.ClickException("缺少 client_key，请先运行 douyin auth login")
    if not client_secret:
        raise click.ClickException("缺少 client_secret，请传入 --client-secret")

    with DouyinOpenAPIClient() as client:
        token_data = client.access_token(client_key, client_secret, code)
    save_openapi_config(
        {
            "clientSecret": client_secret,
            **extract_openapi_token_fields(token_data),
        },
    )
    echo_json(token_data)
    click.echo(f"官方 token 已保存: {SETTINGS_FILE}")


@auth.command("refresh")
def refresh() -> None:
    """刷新已保存的官方 access_token."""
    config = get_openapi_config()
    client_key = config.get("clientKey")
    refresh_token = config.get("refreshToken")
    if not client_key or not refresh_token:
        raise click.ClickException("缺少 client_key 或 refresh_token，请重新授权")

    with DouyinOpenAPIClient() as client:
        token_data = client.refresh_token(client_key, refresh_token)
    save_openapi_config(extract_openapi_token_fields(token_data))
    echo_json(token_data)
    click.echo("官方 token 已刷新")


@auth.command("status")
@click.option("--json", "json_output", is_flag=True, help="输出机器可读 JSON")
def status(json_output: bool) -> None:
    """检查官方授权状态."""
    config = get_openapi_config()
    access_token = config.get("accessToken")
    open_id = config.get("openId")
    status_data = {
        "authorized": bool(access_token and open_id),
        "connected": False,
        "configFile": str(SETTINGS_FILE),
        "openId": open_id or "",
        "scopes": config.get("scopes") or [],
    }
    if not access_token or not open_id:
        if json_output:
            echo_json(status_data)
            return
        click.echo("未完成官方授权")
        return

    if not json_output:
        click.echo(f"已保存官方授权: {SETTINGS_FILE}")
        click.echo(f"open_id: {open_id}")
    scopes = config.get("scopes") or []
    if scopes and not json_output:
        click.echo(f"scopes: {', '.join(scopes)}")
    if not json_output:
        click.echo("正在检查官方 OpenAPI 连通性...")
    try:
        with DouyinOpenAPIClient() as client:
            userinfo = client.userinfo(access_token, open_id)
            status_data["connected"] = True
            status_data["userinfo"] = userinfo
            echo_json(status_data if json_output else userinfo)
    except DouyinOpenAPIError as exc:
        if json_output:
            status_data["error"] = str(exc)
            echo_json(status_data)
            raise click.exceptions.Exit(1) from exc
        raise click.ClickException(f"官方 OpenAPI 连通性检查失败: {exc}") from exc


@auth.command("logout")
def logout() -> None:
    """删除已保存的官方 OAuth token."""
    save_openapi_config(
        {
            "accessToken": "",
            "refreshToken": "",
            "openId": "",
            "expiresIn": 0,
        },
    )
    click.echo("已清除官方授权 token")


@auth.command("cookie-login")
@click.option("--cookie", prompt=True, help="从浏览器复制的 Cookie")
def cookie_login(cookie: str) -> None:
    """保存网页端 Cookie，用于搜索、评论和下载等网页端采集."""
    cookie_value = cookie.strip()
    if not _validate_cookie(cookie_value):
        raise click.ClickException("Cookie 格式校验失败，未保存")
    settings.save({"cookie": cookie_value})
    click.echo(f"Cookie 已保存: {SETTINGS_FILE}")


@auth.command("cookie-status")
def cookie_status() -> None:
    """检查已保存 Cookie 是否可用于网页端请求."""
    cookie_value = settings.get("cookie", "").strip()
    if not cookie_value:
        click.echo("未保存 Cookie")
        return
    if not CookieManager.validate_cookie(cookie_value):
        raise click.ClickException(f"已保存 Cookie，但格式无效: {SETTINGS_FILE}")

    click.echo("正在检查网页端请求连通性...")
    if CookieManager.test_cookie_validity(cookie_value):
        click.echo(f"Cookie 可用于网页端请求: {SETTINGS_FILE}")
        return
    raise click.ClickException("Cookie 已保存，但网页端请求连通性检查失败")


@auth.command("cookie-logout")
def cookie_logout() -> None:
    """删除已保存的网页端 Cookie."""
    settings.save({"cookie": ""})
    click.echo("已清除 Cookie")


def _validate_cookie(cookie: str) -> bool:
    return CookieManager.validate_cookie(cookie)


def _missing_client_key_message(*, qr: bool) -> str:
    lines = [
        "当前命令是官方 OpenAPI OAuth 授权，需要开放平台 client_key。",
        "这不是网页端 Cookie 扫码登录，不能直接生成可保存 Cookie 的登录二维码。",
        "",
        "可选方案：",
        "  1. 官方 OpenAPI：传入 --client-key，或设置 DOUYIN_CLIENT_KEY。",
        "  2. 网页端采集：从浏览器复制 Cookie 后运行：",
        "     douyin auth cookie-login --cookie 'sessionid=...; ttwid=...'",
    ]
    if qr:
        lines.insert(
            1,
            "--qr 只会把官方 OAuth 授权链接渲染成二维码，仍然需要 client_key。",
        )
    return "\n".join(lines)


def _print_qr(value: str) -> None:
    qr = qrcode.QRCode(border=1)
    qr.add_data(value)
    qr.make(fit=True)
    matrix = qr.get_matrix()
    click.echo()
    for row_index in range(0, len(matrix), 2):
        top = matrix[row_index]
        bottom = (
            matrix[row_index + 1] if row_index + 1 < len(matrix) else [False] * len(top)
        )
        line = "".join(
            _qr_cell(top_cell, bottom_cell)
            for top_cell, bottom_cell in zip(top, bottom, strict=True)
        )
        click.echo(line)
    click.echo()


def _qr_cell(top: bool, bottom: bool) -> str:
    if top and bottom:
        return "█"
    if top:
        return "▀"
    if bottom:
        return "▄"
    return " "


def _wait_for_oauth_code(
    host: str,
    port: int,
    expected_state: str | None,
    timeout: int,
) -> str:
    try:
        server = _OAuthCallbackServer((host, port), _OAuthCallbackHandler)
    except OSError as exc:
        raise click.ClickException(
            f"无法监听 {host}:{port}，请换一个 --callback-port",
        ) from exc
    server.code = None
    server.error = None
    server.expected_state = expected_state
    server.timeout = timeout

    thread = threading.Thread(target=server.handle_request, daemon=True)
    thread.start()
    thread.join(timeout + 1)
    server.server_close()

    if server.error:
        raise click.ClickException(server.error)
    if not server.code:
        raise click.ClickException("等待授权回调超时，未获取到 code")
    return server.code


class _OAuthCallbackServer(ThreadingHTTPServer):
    code: str | None = None
    error: str | None = None
    expected_state: str | None = None


class _OAuthCallbackHandler(BaseHTTPRequestHandler):
    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_GET(self) -> None:
        server = self.server
        if not isinstance(server, _OAuthCallbackServer):
            self.send_error(500)
            return

        parsed = urlparse(self.path)
        params = parse_qs(parsed.query)
        state = _first(params.get("state"))
        code = _first(params.get("code"))
        error = _first(params.get("error")) or _first(params.get("error_description"))

        if parsed.path != "/callback":
            self._send_html(404, "Douyin CLI", "未找到回调路径")
            return
        if error:
            server.error = f"授权失败: {error}"
            self._send_html(400, "授权失败", "可以关闭此页面并返回终端。")
            return
        if server.expected_state and state != server.expected_state:
            server.error = "授权回调 state 不匹配，已拒绝"
            self._send_html(400, "授权失败", "state 不匹配。")
            return
        if not code:
            server.error = "授权回调缺少 code"
            self._send_html(400, "授权失败", "回调缺少 code。")
            return

        server.code = code
        self._send_html(200, "授权完成", "可以关闭此页面并返回终端。")

    def _send_html(self, status: int, title: str, body: str) -> None:
        content = (
            "<!doctype html><meta charset='utf-8'>"
            f"<title>{title}</title><h1>{title}</h1><p>{body}</p>"
        ).encode()
        self.send_response(status)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(content)))
        self.end_headers()
        self.wfile.write(content)


def _first(values: list[str] | None) -> str | None:
    if not values:
        return None
    return values[0]
