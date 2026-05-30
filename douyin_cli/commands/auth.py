"""Official OAuth auth commands."""

from __future__ import annotations

import click

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
def login(
    client_key: str | None,
    client_secret: str | None,
    redirect_uri: str | None,
    scopes: tuple[str, ...],
    code: str | None,
) -> None:
    """通过官方 OAuth 授权接入账号."""
    config = get_openapi_config()
    client_key = client_key or config.get("clientKey")
    client_secret = client_secret or config.get("clientSecret")
    redirect_uri = redirect_uri or config.get("redirectUri")
    selected_scopes = list(scopes or config.get("scopes") or DEFAULT_OPENAPI_SCOPES)

    if not client_key:
        raise click.ClickException(
            "缺少 client_key，请传入 --client-key 或设置 DOUYIN_CLIENT_KEY",
        )
    if not redirect_uri:
        raise click.ClickException("缺少 redirect_uri，请传入 --redirect-uri")

    with DouyinOpenAPIClient() as client:
        auth_url = client.authorize_url(client_key, redirect_uri, selected_scopes)
        click.echo("请在浏览器打开以下官方授权链接：")
        click.echo(auth_url)

        updates = {
            "clientKey": client_key,
            "clientSecret": client_secret or "",
            "redirectUri": redirect_uri,
            "scopes": selected_scopes,
        }
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
