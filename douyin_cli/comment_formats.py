"""Format normalized Douyin comments for training datasets."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import ujson as json


@dataclass(frozen=True)
class ChatMLFormatOptions:
    comment_role: str = "user"
    reply_role: str = "assistant"
    min_comment_digg: int = 0
    min_reply_digg: int = 0
    include_single_comments: bool = False


def format_chatml_records(data: dict, options: ChatMLFormatOptions) -> list[dict]:
    """Convert normalized comment trees into ChatML-style JSONL records."""
    aweme_id = str(data.get("aweme_id") or "")
    records: list[dict] = []
    for comment in data.get("comments") or []:
        comment_text = clean_content(comment.get("text"))
        if not comment_text or digg_count(comment) < options.min_comment_digg:
            continue

        replies = comment.get("replies") or []
        if replies:
            records.extend(
                build_reply_records(aweme_id, comment, comment_text, replies, options),
            )
        elif options.include_single_comments:
            records.append(
                build_single_comment_record(aweme_id, comment, comment_text, options),
            )

    return records


def build_reply_records(
    aweme_id: str,
    comment: dict,
    comment_text: str,
    replies: list[dict],
    options: ChatMLFormatOptions,
) -> list[dict]:
    records: list[dict] = []
    for reply in replies:
        reply_text = clean_content(reply.get("text"))
        if not reply_text or digg_count(reply) < options.min_reply_digg:
            continue

        records.append(
            {
                "messages": [
                    {"role": options.comment_role, "content": comment_text},
                    {"role": options.reply_role, "content": reply_text},
                ],
                "metadata": build_metadata(
                    aweme_id,
                    comment,
                    reply,
                    source="douyin_comment_reply",
                ),
            },
        )
    return records


def build_single_comment_record(
    aweme_id: str,
    comment: dict,
    comment_text: str,
    options: ChatMLFormatOptions,
) -> dict:
    return {
        "messages": [{"role": options.comment_role, "content": comment_text}],
        "metadata": build_metadata(
            aweme_id,
            comment,
            None,
            source="douyin_comment",
        ),
    }


def build_metadata(
    aweme_id: str,
    comment: dict,
    reply: dict | None,
    *,
    source: str,
) -> dict:
    comment_digg = digg_count(comment)
    reply_digg = digg_count(reply) if reply is not None else 0
    metadata = {
        "source": source,
        "aweme_id": aweme_id,
        "comment_id": comment.get("id") or "",
        "comment_digg_count": comment_digg,
        "comment_create_time": comment.get("create_time"),
        "comment_user": user_metadata(comment.get("user")),
        "quality_score": comment_digg + reply_digg,
    }
    if reply is not None:
        metadata.update(
            {
                "reply_id": reply.get("id") or "",
                "reply_digg_count": reply_digg,
                "reply_create_time": reply.get("create_time"),
                "reply_user": user_metadata(reply.get("user")),
            },
        )
    return metadata


def user_metadata(user: dict | None) -> dict:
    user = user or {}
    return {
        "uid": user.get("uid") or "",
        "sec_uid": user.get("sec_uid") or "",
        "nickname": user.get("nickname") or "",
        "unique_id": user.get("unique_id") or "",
    }


def digg_count(item: dict | None) -> int:
    if not item:
        return 0
    value = item.get("digg_count", 0)
    if isinstance(value, int):
        return value
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def clean_content(value: object) -> str:
    if value is None:
        return ""
    return str(value).strip()


def write_chatml_jsonl(records: list[dict], output_path: Path | None) -> None:
    text = "\n".join(json.dumps(record, ensure_ascii=False) for record in records)
    write_text(text, output_path)


def write_chatml_json(records: list[dict], output_path: Path | None) -> None:
    text = json.dumps(records, ensure_ascii=False, indent=2)
    write_text(text, output_path)


def write_text(text: str, output_path: Path | None) -> None:
    if text:
        text += "\n"
    if output_path is None:
        print(text, end="")
        return
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(text, encoding="utf-8")
