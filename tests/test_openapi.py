import pytest

from douyin_cli.douyin.openapi import DouyinOpenAPIClient, DouyinOpenAPIError


def test_authorize_url_encodes_scope_and_redirect_uri() -> None:
    client = DouyinOpenAPIClient()

    url = client.authorize_url(
        "client",
        "https://example.com/callback",
        ["user_info", "item.comment"],
        "state value",
    )

    assert url.startswith("https://open.douyin.com/platform/oauth/connect/?")
    assert "client_key=client" in url
    assert "scope=user_info%2Citem.comment" in url
    assert "redirect_uri=https%3A%2F%2Fexample.com%2Fcallback" in url
    assert "state=state+value" in url


def test_request_rejects_missing_token() -> None:
    client = DouyinOpenAPIClient()

    with pytest.raises(DouyinOpenAPIError, match="access-token"):
        client.request("GET", "/oauth/userinfo/")
