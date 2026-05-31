"""Hidden root compatibility flow."""

from __future__ import annotations

import traceback
from collections.abc import Iterator
from dataclasses import dataclass
from pathlib import Path

import click
from loguru import logger

from douyin_cli.cookies import CookieManager
from douyin_cli.douyin import Douyin
from douyin_cli.download import download
from douyin_cli.paths import get_download_root
from douyin_cli.settings import settings

ACCOUNT_ONLY_TYPES = {"favorite", "collection", "following", "follower"}
NO_DOWNLOAD_TYPES = {"following", "follower"}
DEFAULT_DOWNLOAD_DIR = str(get_download_root())


@dataclass(frozen=True)
class CrawlOptions:
    """Normalized options used by one or more compatibility tasks."""

    limit: int
    no_download: bool
    crawl_type: str
    output_path: str
    cookie: str
    filters: dict[str, str]
    download_title: bool
    download_cover: bool


def should_run_crawl(
    urls: tuple[str, ...],
    limit: int,
    no_download: bool,
    crawl_type: str,
    output_path: str,
    sort_type: str | None,
    publish_time: str | None,
    filter_duration: str | None,
    download_title: bool,
    download_cover: bool,
) -> bool:
    """Return whether the root command should run the compatibility flow."""
    return any(
        [
            bool(urls),
            limit != 0,
            no_download,
            crawl_type != "post",
            output_path != DEFAULT_DOWNLOAD_DIR,
            sort_type is not None,
            publish_time is not None,
            filter_duration is not None,
            download_title,
            download_cover,
        ],
    )


def run_crawl(
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
    """Run the hidden root compatibility flow."""
    cookie_value = load_cookie(cookie)
    if not cookie_value:
        raise click.ClickException("未登录。请先运行: douyin auth login")
    if not validate_cookie(cookie_value):
        return

    targets = resolve_targets(urls, crawl_type)
    if targets is None:
        return

    options = CrawlOptions(
        limit=limit,
        no_download=no_download,
        crawl_type=crawl_type,
        output_path=output_path,
        cookie=cookie_value,
        filters=build_filters(sort_type, publish_time, filter_duration),
        download_title=download_title,
        download_cover=download_cover,
    )
    run_targets(targets, options)


def build_filters(
    sort_type: str | None,
    publish_time: str | None,
    filter_duration: str | None,
) -> dict[str, str]:
    """Build search filter arguments."""
    filters = {}
    if sort_type:
        filters["sort_type"] = sort_type
    if publish_time:
        filters["publish_time"] = publish_time
    if filter_duration is not None:
        filters["filter_duration"] = filter_duration
    return filters


def load_cookie(cookie: str | None) -> str | None:
    """Load cookie from CLI or saved auth config."""
    if cookie is not None:
        logger.info("正在加载命令行指定的Cookie...")
        cookie_value = cookie.strip()
        if cookie_value:
            return cookie_value
        logger.error("无法加载指定的Cookie")
        return None

    cookie_value = settings.get("cookie", "").strip()
    if cookie_value:
        logger.info("✓ 已从配置文件加载Cookie")
        return cookie_value

    return None


def validate_cookie(cookie: str, *, quiet: bool = False) -> bool:
    """Validate cookie before compatibility tasks."""
    if CookieManager.validate_cookie(cookie):
        if not quiet:
            logger.success("✓ Cookie验证通过")
        return True

    if quiet:
        return False

    logger.error("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.error("✗ Cookie验证失败")
    logger.info("可能原因：")
    logger.info("  1. Cookie已过期，请重新获取")
    logger.info("  2. Cookie格式不正确")
    logger.info("  3. 账号已退出登录")
    logger.error("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    return False


def resolve_targets(urls: tuple[str, ...], crawl_type: str) -> tuple[str, ...] | None:
    """Resolve CLI targets or prompt for one when needed."""
    if urls:
        return urls

    if crawl_type in ACCOUNT_ONLY_TYPES:
        logger.info(f"采集本账号的 {crawl_type} 数据")
        return ("",)

    url_input = click.prompt(
        f"采集类型：{crawl_type}，请输入目标关键词/URL链接/ID或文件路径",
        default="",
        show_default=False,
    ).strip()
    if url_input:
        return (url_input,)

    logger.error("未输入目标，退出程序")
    return None


def run_targets(targets: tuple[str, ...], options: CrawlOptions) -> None:
    """Run all targets and print a summary."""
    success_count = 0
    fail_count = 0

    for target in iter_targets(targets):
        if run_task(target, options):
            success_count += 1
        else:
            fail_count += 1

    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.success(f"✓ 任务完成：成功 {success_count} 个，失败 {fail_count} 个")
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")


def iter_targets(targets: tuple[str, ...]) -> Iterator[str | None]:
    """Yield individual targets, expanding files when an argument is a path."""
    for raw_target in targets:
        target = raw_target.strip()
        if not target:
            continue

        path = Path(target)
        if not path.exists():
            yield target
            continue

        logger.info(f"从文件读取目标：{target}")
        lines = read_target_file(path)
        if not lines:
            logger.error(f"文件 [{target}] 中没有发现目标URL")
            yield None
            continue

        logger.info(f"文件中共有 {len(lines)} 个目标")
        for index, line in enumerate(lines, 1):
            logger.info(f"处理第 {index}/{len(lines)} 个目标")
            yield line


def read_target_file(path: Path) -> list[str]:
    """Read one target per non-empty line."""
    try:
        return [
            line.strip()
            for line in path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
    except OSError as exc:
        logger.error(f"读取文件失败: {exc}")
        return []


def run_task(target: str | None, options: CrawlOptions) -> bool:
    """Run one compatibility task."""
    if target is None:
        return False

    try:
        log_task_start(target, options)
        douyin = create_client(target, options)
        douyin.run()
        maybe_download(douyin, options)
        return True
    except KeyboardInterrupt:
        logger.warning("用户中断任务")
        return False
    except Exception as exc:
        logger.error(f"任务执行失败: {exc}")
        logger.debug(traceback.format_exc())
        return False


def log_task_start(target: str, options: CrawlOptions) -> None:
    """Log the task configuration."""
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")
    logger.info("开始采集任务")
    logger.info(f"  目标: {target or '本账号'}")
    logger.info(f"  类型: {options.crawl_type}")
    logger.info(f"  数量限制: {'不限' if options.limit == 0 else f'{options.limit}条'}")
    if options.filters:
        logger.info(f"  筛选条件: {options.filters}")
    if options.download_title:
        logger.info("  下载标题: ✓ 是")
    if options.download_cover:
        logger.info("  下载封面: ✓ 是")
    logger.info("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━")


def create_client(target: str, options: CrawlOptions) -> Douyin:
    """Create the client for one compatibility task."""
    return Douyin(
        target=target,
        limit=options.limit,
        type=options.crawl_type,
        down_path=options.output_path,
        cookie=options.cookie,
        user_agent=settings.get("userAgent", ""),
        filters=options.filters,
        enable_download_title=options.download_title
        or settings.get("enableDownloadTitle", False),
        enable_download_cover=options.download_cover
        or settings.get("enableDownloadCover", False),
    )


def maybe_download(douyin: Douyin, options: CrawlOptions) -> None:
    """Download files when requested and supported."""
    if options.no_download:
        logger.info("已跳过下载（--no-download）")
        return

    if douyin.type in NO_DOWNLOAD_TYPES:
        logger.info("此类型不需要下载文件")
        return

    logger.info("开始下载文件...")
    download(douyin.down_path, douyin.aria2_conf)
