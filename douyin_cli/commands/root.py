"""Root CLI command."""

from __future__ import annotations

import click

from douyin_cli.commands.api import api
from douyin_cli.commands.auth import auth
from douyin_cli.commands.comment import comment
from douyin_cli.commands.common import APP_VERSION, configure_logging
from douyin_cli.commands.compat import DEFAULT_DOWNLOAD_DIR, run_crawl
from douyin_cli.commands.compat import should_run_crawl as should_run_compat
from douyin_cli.commands.mcp import mcp
from douyin_cli.commands.obscura import obscura
from douyin_cli.commands.subtitle import subtitle

BANNER = rf"""
  ____                    _          ____                    _
 |  _ \  ___  _   _ _   _(_)_ __    / ___|_ __ __ ___      _| | ___ _ __
 | | | |/ _ \| | | | | | | | '_ \  | |   | '__/ _` \ \ /\ / / |/ _ \ '__|
 | |_| | (_) | |_| | |_| | | | | | | |___| | | (_| |\ V  V /| |  __/ |
 |____/ \___/ \__,_|\__, |_|_| |_|  \____|_|  \__,_| \_/\_/ |_|\___|_|
                    |___/
                              V{APP_VERSION}
                Github: https://github.com/LIghtJUNction/douyin
"""


@click.group(invoke_without_command=True)
@click.pass_context
@click.option(
    "-u",
    "--urls",
    type=click.STRING,
    multiple=True,
    help="作品/账号/话题/音乐等类型的URL链接/ID或搜索关键词，也可输入文件路径（文件内一行一个），可多次输入。",
)
@click.option(
    "-l",
    "--limit",
    type=click.INT,
    default=0,
    help="限制最大采集数量，默认不限制（0表示不限制）",
)
@click.option(
    "--no-download",
    is_flag=True,
    help="不下载文件，仅采集数据",
)
@click.option(
    "-t",
    "--type",
    "crawl_type",
    type=click.Choice(
        [
            "post",
            "favorite",
            "music",
            "hashtag",
            "search",
            "following",
            "follower",
            "collection",
            "mix",
            "aweme",
        ],
        case_sensitive=False,
    ),
    default="post",
    help="采集类型，默认为post（主页作品）。支持：post/favorite/music/hashtag/search/following/follower/collection/mix/aweme",
)
@click.option(
    "-p",
    "--path",
    "output_path",
    type=click.STRING,
    default=DEFAULT_DOWNLOAD_DIR,
    show_default=True,
    help="下载文件夹路径",
)
@click.option(
    "-c",
    "--cookie",
    type=click.STRING,
    help="本次运行使用的 Cookie；长期保存请用 douyin auth cookie-login",
)
@click.option(
    "--sort-type",
    type=click.Choice(["0", "1", "2"], case_sensitive=False),
    help="搜索排序（仅search类型）：0=综合，1=最多点赞，2=最新",
)
@click.option(
    "--publish-time",
    type=click.Choice(["0", "1", "7", "180"], case_sensitive=False),
    help="发布时间（仅search类型）：0=不限，1=一天内，7=一周内，180=半年内",
)
@click.option(
    "--filter-duration",
    type=click.Choice(["", "0-1", "1-5", "5-10000"], case_sensitive=False),
    help="视频时长（仅search类型）：空=不限，0-1=1分钟以下，1-5=1-5分钟，5-10000=5分钟以上",
)
@click.option(
    "--download-title",
    is_flag=True,
    help="下载标题文本文件",
)
@click.option(
    "--download-cover",
    is_flag=True,
    help="下载封面图片",
)
def main(
    ctx: click.Context,
    urls: tuple[str, ...],
    limit: int,
    no_download: bool,
    crawl_type: str,
    output_path: str,
    cookie: str | None,
    sort_type: str | None,
    publish_time: str | None,
    filter_duration: str | None,
    download_title: bool,
    download_cover: bool,
) -> None:
    """抖音 CLI。

    \b
    通用抖音命令行工具。更多命令见：douyin COMMAND --help
    """
    configure_logging()

    if ctx.invoked_subcommand is not None:
        return

    if not should_run_compat(
        urls,
        limit,
        no_download,
        crawl_type,
        output_path,
        sort_type,
        publish_time,
        filter_duration,
        download_title,
        download_cover,
    ):
        click.echo(ctx.get_help())
        return

    click.echo(BANNER)
    run_crawl(
        urls,
        limit,
        no_download,
        crawl_type,
        output_path,
        cookie,
        sort_type,
        publish_time,
        filter_duration,
        download_title,
        download_cover,
    )


main.add_command(api)
main.add_command(auth)
main.add_command(obscura)
main.add_command(subtitle)
main.add_command(comment)
main.add_command(mcp)
