from douyin_cli.comment_formats import ChatMLFormatOptions, format_chatml_records


def sample_comments() -> dict:
    return {
        "aweme_id": "7380000000000000000",
        "comments": [
            {
                "id": "c1",
                "text": "这车能买吗？",
                "create_time": 1710000000,
                "digg_count": 8,
                "user": {"uid": "u1", "nickname": "买车人"},
                "replies": [
                    {
                        "id": "r1",
                        "text": "先查维保和事故，再看价格。",
                        "create_time": 1710000001,
                        "digg_count": 12,
                        "user": {"uid": "seller", "nickname": "懂车"},
                    },
                    {
                        "id": "r2",
                        "text": " ",
                        "digg_count": 99,
                        "user": {"uid": "empty"},
                    },
                ],
            },
            {
                "id": "c2",
                "text": "路过",
                "digg_count": 1,
                "user": {"uid": "u2"},
                "replies": [],
            },
        ],
    }


def test_format_chatml_pairs_comment_and_each_reply() -> None:
    records = format_chatml_records(sample_comments(), ChatMLFormatOptions())

    assert records == [
        {
            "messages": [
                {"role": "user", "content": "这车能买吗？"},
                {"role": "assistant", "content": "先查维保和事故，再看价格。"},
            ],
            "metadata": {
                "source": "douyin_comment_reply",
                "aweme_id": "7380000000000000000",
                "comment_id": "c1",
                "comment_digg_count": 8,
                "comment_create_time": 1710000000,
                "comment_user": {
                    "uid": "u1",
                    "sec_uid": "",
                    "nickname": "买车人",
                    "unique_id": "",
                },
                "quality_score": 20,
                "reply_id": "r1",
                "reply_digg_count": 12,
                "reply_create_time": 1710000001,
                "reply_user": {
                    "uid": "seller",
                    "sec_uid": "",
                    "nickname": "懂车",
                    "unique_id": "",
                },
            },
        },
    ]


def test_format_chatml_uses_configured_roles_and_digg_thresholds() -> None:
    records = format_chatml_records(
        sample_comments(),
        ChatMLFormatOptions(
            comment_role="human",
            reply_role="gpt",
            min_comment_digg=5,
            min_reply_digg=10,
        ),
    )

    assert records[0]["messages"] == [
        {"role": "human", "content": "这车能买吗？"},
        {"role": "gpt", "content": "先查维保和事故，再看价格。"},
    ]


def test_format_chatml_skips_low_quality_comments() -> None:
    records = format_chatml_records(
        sample_comments(),
        ChatMLFormatOptions(min_comment_digg=10),
    )

    assert records == []


def test_format_chatml_can_include_single_comments() -> None:
    records = format_chatml_records(
        sample_comments(),
        ChatMLFormatOptions(include_single_comments=True),
    )

    assert records[-1] == {
        "messages": [{"role": "user", "content": "路过"}],
        "metadata": {
            "source": "douyin_comment",
            "aweme_id": "7380000000000000000",
            "comment_id": "c2",
            "comment_digg_count": 1,
            "comment_create_time": None,
            "comment_user": {
                "uid": "u2",
                "sec_uid": "",
                "nickname": "",
                "unique_id": "",
            },
            "quality_score": 1,
        },
    }
