import pytest
from click.testing import CliRunner

from douyin_cli.cli import main
from douyin_cli.commands import api, auth, common
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


def test_login_prints_qr_and_keeps_manual_code_flow(monkeypatch) -> None:
    saved: dict = {}

    def save_config(updates: dict) -> None:
        saved.update(updates)

    monkeypatch.setattr(auth, "save_openapi_config", save_config)

    result = CliRunner().invoke(
        main,
        [
            "auth",
            "login",
            "--client-key",
            "client",
            "--client-secret",
            "secret",
            "--redirect-uri",
            "https://example.com/callback",
            "--scope",
            "user_info",
        ],
    )

    assert result.exit_code == 0
    assert "请在浏览器打开以下官方授权链接" in result.output
    assert "█" in result.output or "▀" in result.output or "▄" in result.output
    assert "授权完成后运行：douyin auth code --code 授权码" in result.output
    assert saved["clientKey"] == "client"
    assert saved["redirectUri"] == "https://example.com/callback"


def test_login_qr_without_client_key_explains_openapi_requirement() -> None:
    result = CliRunner().invoke(main, ["auth", "login", "--qr"])

    assert result.exit_code != 0
    assert "官方 OpenAPI OAuth 授权" in result.output
    assert "--qr 只会把官方 OAuth 授权链接渲染成二维码" in result.output
    assert "douyin auth cookie-login --cookie" in result.output


def test_login_listen_uses_local_callback_and_exchanges_code(monkeypatch) -> None:
    saved: dict = {}

    def save_config(updates: dict) -> None:
        saved.update(updates)

    def wait_for_code(*_args: object) -> str:
        return "callback-code"

    monkeypatch.setattr(auth, "save_openapi_config", save_config)
    monkeypatch.setattr(auth, "_wait_for_oauth_code", wait_for_code)

    class DummyClient:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
            return False

        def authorize_url(
            self,
            client_key: str,
            redirect_uri: str,
            scopes: list[str],
            state: str | None = None,
        ) -> str:
            assert client_key == "client"
            assert redirect_uri == "http://127.0.0.1:8787/callback"
            assert scopes == ["user_info"]
            assert state
            return "https://open.douyin.com/oauth"

        def access_token(
            self,
            client_key: str,
            client_secret: str,
            code: str,
        ) -> dict:
            assert (client_key, client_secret, code) == (
                "client",
                "secret",
                "callback-code",
            )
            return {
                "data": {
                    "access_token": "access",
                    "refresh_token": "refresh",
                    "open_id": "open",
                    "expires_in": 86400,
                },
            }

    monkeypatch.setattr(auth, "DouyinOpenAPIClient", DummyClient)

    result = CliRunner().invoke(
        main,
        [
            "auth",
            "login",
            "--client-key",
            "client",
            "--client-secret",
            "secret",
            "--scope",
            "user_info",
            "--listen",
            "--no-qr",
        ],
    )

    assert result.exit_code == 0
    assert "正在等待授权回调: http://127.0.0.1:8787/callback" in result.output
    assert saved["accessToken"] == "access"
    assert saved["refreshToken"] == "refresh"
    assert saved["openId"] == "open"


def test_send_im_message_posts_enterprise_payload(monkeypatch) -> None:
    client = DouyinOpenAPIClient()
    captured: dict = {}

    def fake_request(method: str, path: str, **kwargs: object) -> dict:
        captured.update({"method": method, "path": path, **kwargs})
        return {"data": {"message_id": "msg-1"}}

    monkeypatch.setattr(client, "request", fake_request)

    result = client.send_im_message(
        "access",
        "enterprise-open-id",
        "user-open-id",
        "text",
        {"text": "你好"},
        persona_id="persona",
        client_msg_id="client-msg",
    )

    assert result == {"data": {"message_id": "msg-1"}}
    assert captured["method"] == "POST"
    assert captured["path"] == "/enterprise/im/message/send/"
    assert captured["token"] == "access"
    assert captured["params"] == {"open_id": "enterprise-open-id"}
    assert captured["json_body"] == {
        "to_user_id": "user-open-id",
        "message_type": "text",
        "content": '{"text":"你好"}',
        "persona_id": "persona",
        "client_msg_id": "client-msg",
    }


def test_api_im_message_send_uses_saved_auth(monkeypatch) -> None:
    monkeypatch.setattr(
        common.settings,
        "_settings",
        {
            "openapi": {
                "accessToken": "saved-token",
                "openId": "enterprise-open-id",
            },
        },
    )

    class DummyClient:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
            return False

        def send_im_message(
            self,
            token: str,
            open_id: str,
            to_user_id: str,
            message_type: str,
            content: dict,
            *,
            persona_id: str | None = None,
            client_msg_id: str | None = None,
        ) -> dict:
            assert token == "saved-token"
            assert open_id == "enterprise-open-id"
            assert to_user_id == "user-open-id"
            assert message_type == "text"
            assert content == {"text": "你好"}
            assert persona_id is None
            assert client_msg_id == "client-msg"
            return {"data": {"message_id": "msg-1"}}

    monkeypatch.setattr(api, "DouyinOpenAPIClient", DummyClient)

    result = CliRunner().invoke(
        main,
        [
            "api",
            "im-message-send",
            "--to-user-id",
            "user-open-id",
            "--text",
            "你好",
            "--client-msg-id",
            "client-msg",
            "--yes",
        ],
    )

    assert result.exit_code == 0
    assert "msg-1" in result.output


def test_api_im_message_send_requires_text_for_text_message() -> None:
    result = CliRunner().invoke(
        main,
        [
            "api",
            "im-message-send",
            "--token",
            "token",
            "--open-id",
            "open",
            "--to-user-id",
            "user",
            "--yes",
        ],
    )

    assert result.exit_code != 0
    assert "message-type=text 需要 --text" in result.output
