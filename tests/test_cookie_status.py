from click.testing import CliRunner

from douyin_cli.cli import main
from douyin_cli.commands import auth
from douyin_cli.cookies import CookieManager


def test_cookie_status_uses_web_request_validity(monkeypatch) -> None:
    monkeypatch.setattr(
        auth.settings,
        "_settings",
        {"cookie": "sessionid=abc; ttwid=def"},
    )
    monkeypatch.setattr(CookieManager, "validate_cookie", lambda _cookie: True)
    monkeypatch.setattr(CookieManager, "test_cookie_validity", lambda _cookie: True)

    result = CliRunner().invoke(main, ["auth", "cookie-status"])

    assert result.exit_code == 0
    assert "Cookie 可用于网页端请求" in result.output


def test_cookie_validity_prefers_web_probe_over_sso(monkeypatch) -> None:
    monkeypatch.setattr(
        CookieManager,
        "test_web_request_validity",
        lambda _cookie_dict: True,
    )
    monkeypatch.setattr(
        CookieManager,
        "test_sso_login_validity",
        lambda _cookie_dict: False,
    )

    assert CookieManager.test_cookie_validity("sessionid=abc; ttwid=def") is True
