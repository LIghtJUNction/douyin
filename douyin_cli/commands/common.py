"""Shared CLI helpers."""

from __future__ import annotations

import sys
from contextlib import suppress
from importlib.metadata import PackageNotFoundError, version
from json import JSONDecodeError

import click
import ujson as json
from loguru import logger

from douyin_cli.settings import settings

DEFAULT_OPENAPI_SCOPES = ("user_info",)
APP_VERSION = "0.0.0"
with suppress(PackageNotFoundError):
    APP_VERSION = version("douyin-cli")

_LOGGING_CONFIGURED = False


def configure_logging() -> None:
    """Configure loguru for human-facing CLI output."""
    global _LOGGING_CONFIGURED  # noqa: PLW0603

    if _LOGGING_CONFIGURED:
        return
    logger.remove()
    logger.add(sys.stderr, level="INFO", format="{message}")
    _LOGGING_CONFIGURED = True


def echo_json(data: dict | list) -> None:
    """Print a JSON response."""
    click.echo(json.dumps(data, ensure_ascii=False, indent=2))


def get_openapi_config() -> dict:
    """Return saved official OpenAPI auth config."""
    config = settings.get("openapi", {})
    if isinstance(config, dict):
        return config.copy()
    return {}


def save_openapi_config(updates: dict) -> None:
    """Update official OpenAPI auth config."""
    config = get_openapi_config()
    config.update(updates)
    settings.save({"openapi": config})


def resolve_openapi_auth(
    token: str | None,
    open_id: str | None,
) -> tuple[str, str]:
    """Resolve token and open_id from CLI options, env vars, or saved config."""
    config = get_openapi_config()
    resolved_token = token or config.get("accessToken")
    resolved_open_id = open_id or config.get("openId")
    if not resolved_token:
        raise click.ClickException("缺少 access_token，请先运行 douyin auth login")
    if not resolved_open_id:
        raise click.ClickException("缺少 open_id，请先运行 douyin auth login")
    return resolved_token, resolved_open_id


def extract_openapi_token_fields(data: dict) -> dict:
    """Extract token fields from a Douyin OpenAPI token response."""
    token_data = data.get("data") if isinstance(data.get("data"), dict) else data
    result = {}
    mapping = {
        "access_token": "accessToken",
        "refresh_token": "refreshToken",
        "open_id": "openId",
        "expires_in": "expiresIn",
    }
    for source, target in mapping.items():
        value = token_data.get(source)
        if value is not None:
            result[target] = value
    return result


def parse_json_body(text: str | None) -> dict | list | None:
    """Parse a JSON body option."""
    if text is None:
        return None
    try:
        data = json.loads(text)
    except JSONDecodeError as exc:
        msg = f"--json 不是合法 JSON: {exc}"
        raise ValueError(msg) from exc
    if not isinstance(data, (dict, list)):
        msg = "--json 必须是 JSON object 或 array"
        raise ValueError(msg)
    return data


def parse_key_values(values: tuple[str, ...]) -> dict[str, str] | None:
    """Parse repeated key=value CLI options."""
    if not values:
        return None
    parsed = {}
    for value in values:
        if "=" not in value:
            msg = f"参数必须是 key=value 格式: {value}"
            raise ValueError(msg)
        key, item_value = value.split("=", 1)
        if not key:
            msg = f"参数 key 不能为空: {value}"
            raise ValueError(msg)
        parsed[key] = item_value
    return parsed
