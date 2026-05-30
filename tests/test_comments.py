from douyin_cli.comments import extract_aweme_id, parse_comment


def test_extract_aweme_id_from_raw_id() -> None:
    assert extract_aweme_id("7380000000000000000") == "7380000000000000000"


def test_extract_aweme_id_from_video_url() -> None:
    url = "https://www.douyin.com/video/7380000000000000000?previous_page=webapp"

    assert extract_aweme_id(url) == "7380000000000000000"


def test_extract_aweme_id_from_note_url() -> None:
    url = "https://www.douyin.com/note/7380000000000000000"

    assert extract_aweme_id(url) == "7380000000000000000"


def test_parse_comment_normalizes_user_fields() -> None:
    comment = parse_comment(
        {
            "cid": "1",
            "text": "你好",
            "create_time": 1710000000,
            "digg_count": 3,
            "reply_comment_total": 2,
            "ip_label": "上海",
            "user": {
                "uid": "u1",
                "sec_uid": "sec",
                "nickname": "用户",
                "unique_id": "unique",
            },
        },
    )

    assert comment == {
        "id": "1",
        "text": "你好",
        "create_time": 1710000000,
        "digg_count": 3,
        "reply_comment_total": 2,
        "ip_label": "上海",
        "user": {
            "uid": "u1",
            "sec_uid": "sec",
            "nickname": "用户",
            "unique_id": "unique",
        },
    }
