"""Hidden comment compatibility command."""

from __future__ import annotations

from pathlib import Path

import click

from douyin_cli.commands.compat import load_cookie, validate_cookie
from douyin_cli.comment_formats import (
    ChatMLFormatOptions,
    format_chatml_records,
    write_chatml_json,
    write_chatml_jsonl,
)
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
    "--format",
    "output_format",
    type=click.Choice(["raw", "chatml-jsonl", "chatml-json"], case_sensitive=False),
    default="raw",
    show_default=True,
    help="输出格式",
)
@click.option("--comment-role", default="user", show_default=True, help="评论者角色名")
@click.option(
    "--reply-role",
    default="assistant",
    show_default=True,
    help="回复者角色名",
)
@click.option(
    "--min-comment-digg",
    default=0,
    show_default=True,
    help="一级评论最少点赞数",
)
@click.option(
    "--min-reply-digg",
    default=0,
    show_default=True,
    help="回复评论最少点赞数",
)
@click.option(
    "--include-single-comments",
    is_flag=True,
    help="保留没有回复的一级评论；默认跳过，避免 SFT 缺少 assistant 标签",
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
    output_format: str,
    comment_role: str,
    reply_role: str,
    min_comment_digg: int,
    min_reply_digg: int,
    include_single_comments: bool,
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
    if output_format == "raw":
        write_comments(data, output)
    else:
        format_options = ChatMLFormatOptions(
            comment_role=comment_role,
            reply_role=reply_role,
            min_comment_digg=min_comment_digg,
            min_reply_digg=min_reply_digg,
            include_single_comments=include_single_comments,
        )
        records = format_chatml_records(data, format_options)
        if output_format == "chatml-json":
            write_chatml_json(records, output)
        else:
            write_chatml_jsonl(records, output)
    if output is not None:
        click.echo(f"评论已保存: {output}")
