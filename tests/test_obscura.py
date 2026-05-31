import ujson as json
from click.testing import CliRunner

from douyin_cli.cli import main
from douyin_cli.commands import api, common
from douyin_cli.obscura import build_obscura_manifest
from douyin_cli.settings import SETTINGS_FILE


def test_obscura_manifest_describes_saved_auth_commands() -> None:
    manifest = build_obscura_manifest(
        version="1.2.3",
        config_file=SETTINGS_FILE,
    )

    assert manifest["entrypoint"] == "douyin"
    assert manifest["auth"]["status"] == ["douyin", "auth", "status", "--json"]
    assert manifest["openapi"]["output"] == "json"


def test_obscura_manifest_command_outputs_json() -> None:
    result = CliRunner().invoke(main, ["obscura", "manifest"])

    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["name"] == "douyin-cli"
    assert data["homepage"] == "https://github.com/LIghtJUNction/douyin"


def test_api_userinfo_uses_saved_openapi_auth(monkeypatch) -> None:
    monkeypatch.setattr(
        common.settings,
        "_settings",
        {
            "openapi": {
                "accessToken": "saved-token",
                "openId": "saved-open-id",
            },
        },
    )

    class DummyClient:
        def __enter__(self):
            return self

        def __exit__(self, exc_type, exc_val, exc_tb) -> bool:
            return False

        def userinfo(self, token: str, open_id: str) -> dict:
            assert token == "saved-token"
            assert open_id == "saved-open-id"
            return {"data": {"nickname": "tester"}}

    monkeypatch.setattr(api, "DouyinOpenAPIClient", DummyClient)

    result = CliRunner().invoke(main, ["api", "userinfo"])

    assert result.exit_code == 0
    assert json.loads(result.output) == {"data": {"nickname": "tester"}}


def test_auth_status_json_reports_missing_auth(monkeypatch) -> None:
    monkeypatch.setattr(common.settings, "_settings", {"openapi": {}})

    result = CliRunner().invoke(main, ["auth", "status", "--json"])

    assert result.exit_code == 0
    data = json.loads(result.output)
    assert data["authorized"] is False
    assert data["connected"] is False
