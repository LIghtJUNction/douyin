"""Douyin comment crawling helpers."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from time import sleep
from urllib.parse import unquote, urlparse

import ujson as json

from douyin_cli.douyin.client import DouyinClient
from douyin_cli.douyin.request import Request
from douyin_cli.text import url_redirect


@dataclass(frozen=True)
class CommentOptions:
    limit: int
    count: int
    with_replies: bool
    reply_limit: int
    sleep_seconds: float
    cookie: str
    user_agent: str


def crawl_comments(target: str, options: CommentOptions) -> dict:
    """Crawl comments for one aweme."""
    aweme_id = extract_aweme_id(target)
    if not aweme_id:
        msg = f"无法识别作品 ID: {target}"
        raise ValueError(msg)

    comments: list[dict] = []
    cursor = 0
    has_more = True

    with Request(options.cookie, options.user_agent) as request:
        client = DouyinClient(request)
        while has_more and not reached_limit(comments, options.limit):
            page, cursor, has_more = client.fetch_comments(
                aweme_id,
                cursor,
                options.count,
            )
            if not page:
                break

            for raw_comment in page:
                comment = parse_comment(raw_comment)
                if options.with_replies:
                    comment["replies"] = crawl_comment_replies(
                        client,
                        aweme_id,
                        comment["id"],
                        options,
                    )
                comments.append(comment)
                if reached_limit(comments, options.limit):
                    break

            if has_more and options.sleep_seconds > 0:
                sleep(options.sleep_seconds)

    return {"aweme_id": aweme_id, "comments": comments}


def crawl_comment_replies(
    client: DouyinClient,
    aweme_id: str,
    comment_id: str,
    options: CommentOptions,
) -> list[dict]:
    """Crawl replies for one top-level comment."""
    replies: list[dict] = []
    cursor = 0
    has_more = True

    while has_more and not reached_limit(replies, options.reply_limit):
        page, cursor, has_more = client.fetch_comment_replies(
            aweme_id,
            comment_id,
            cursor,
            options.count,
        )
        if not page:
            break

        for raw_reply in page:
            replies.append(parse_comment(raw_reply))
            if reached_limit(replies, options.reply_limit):
                break

        if has_more and options.sleep_seconds > 0:
            sleep(options.sleep_seconds)

    return replies


def reached_limit(items: list, limit: int) -> bool:
    """Return whether a positive limit has been reached."""
    return limit > 0 and len(items) >= limit


def extract_aweme_id(target: str) -> str:
    """Extract an aweme id from a raw id or Douyin URL."""
    value = target.strip()
    if value.isdigit():
        return value

    parsed = urlparse(value)
    if not parsed.hostname:
        return ""

    if parsed.hostname == "v.douyin.com":
        value = url_redirect(value)
        parsed = urlparse(value)

    path_parts = [part for part in unquote(parsed.path).split("/") if part]
    if not path_parts:
        return ""

    for marker in ("video", "note"):
        if marker in path_parts:
            index = path_parts.index(marker)
            if len(path_parts) > index + 1:
                aweme_id = path_parts[index + 1]
                return aweme_id if aweme_id.isdigit() else ""

    candidate = path_parts[-1]
    return candidate if candidate.isdigit() else ""


def parse_comment(comment: dict) -> dict:
    """Normalize a Douyin comment object."""
    user = comment.get("user") or {}
    return {
        "id": comment.get("cid") or comment.get("comment_id") or "",
        "text": comment.get("text") or "",
        "create_time": comment.get("create_time"),
        "digg_count": comment.get("digg_count", 0),
        "reply_comment_total": comment.get("reply_comment_total", 0),
        "ip_label": comment.get("ip_label") or "",
        "user": {
            "uid": user.get("uid") or "",
            "sec_uid": user.get("sec_uid") or "",
            "nickname": user.get("nickname") or "",
            "unique_id": user.get("unique_id") or "",
        },
    }


def write_comments(data: dict, output_path: Path | None) -> None:
    """Write comments to a file or stdout-ready JSON string."""
    text = json.dumps(data, ensure_ascii=False, indent=2)
    if output_path is None:
        print(text)
        return
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(text + "\n", encoding="utf-8")
