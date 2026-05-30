"""Hidden comment compatibility command."""

from __future__ import annotations

from pathlib import Path

import click

from douyin_cli.commands.compat import load_cookie, validate_cookie
from douyin_cli.comments import CommentOptions, crawl_comments, write_comments
from douyin_cli.settings import settings


@click.command("comment", hidden=True)
@click.argument("target")
@click.option(
    "-l",
    "--limit",
    default=100,
    show_default=True,
    help="最多抓取一级评论数",
)
@click.option("--count", default=20, show_default=True, help="每页请求数量")
@click.option("--with-replies", is_flag=True, help="同时抓取评论楼中楼回复")
@click.option(
    "--reply-limit",
    default=20,
    show_default=True,
    help="每条评论最多抓取回复数",
)
@click.option(
    "--sleep",
    "sleep_seconds",
    default=0.8,
    show_default=True,
    help="分页请求间隔秒数",
)
@click.option(
    "-o",
    "--output",
    type=click.Path(dir_okay=False, path_type=Path),
    help="输出 JSON 文件；不传则输出到 stdout",
)
@click.option(
    "-c",
    "--cookie",
    type=click.STRING,
    help="本次运行使用的 Cookie；长期保存请用 douyin auth login",
)
def comment(
    target: str,
    limit: int,
    count: int,
    with_replies: bool,
    reply_limit: int,
    sleep_seconds: float,
    output: Path | None,
    cookie: str | None,
) -> None:
    """抓取作品评论区."""
    cookie_value = load_cookie(cookie)
    if not cookie_value:
        raise click.ClickException("未登录。请先运行: douyin auth login")
    if not validate_cookie(cookie_value, quiet=True):
        raise click.ClickException("Cookie 格式校验失败")

    options = CommentOptions(
        limit=limit,
        count=count,
        with_replies=with_replies,
        reply_limit=reply_limit,
        sleep_seconds=sleep_seconds,
        cookie=cookie_value,
        user_agent=settings.get("userAgent", ""),
    )
    try:
        data = crawl_comments(target, options)
    except ValueError as exc:
        raise click.ClickException(str(exc)) from exc
    write_comments(data, output)
    if output is not None:
        click.echo(f"评论已保存: {output}")
