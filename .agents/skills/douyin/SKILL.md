---
name: douyin
description: "This skill should be used when working with this repository's douyin-cli package and douyin command-line tool: installing it with uv tool install, running official Douyin OpenAPI commands, managing OAuth auth, validating packaging with uv_build, or troubleshooting CLI-only behavior."
---

# Douyin CLI

This repository is a CLI-only Python package for Douyin official OpenAPI workflows. Do not add frontend, Docker, FastAPI server, PyInstaller, Nuitka, or setuptools workflows unless the user explicitly reintroduces them.

## Install And Verify

For end-user installs, install the published CLI package by name:

```bash
uv add douyin-cli
uv tool install douyin-cli
douyin --help
```

Install optional dependency groups only when the requested workflow needs them:

```bash
uv tool install 'douyin-cli[subtitle]'
uv tool install 'douyin-cli[subtitle-cuda]'
uv tool install 'douyin-cli[subtitle-mac]'
```

For local repository development, install from the checkout:

```bash
uv tool install -e .
uv tool install -e '.[subtitle-mac]'
douyin --help
uv run douyin api --help
uv build --wheel
```

## Runtime Notes

- Console entry point: `douyin = "douyin_cli.cli:main"`.
- Build backend: `uv_build`; do not add setuptools config.
- CLI dependencies are included in the default package so `uv tool install douyin-cli` works directly.
- CLI state uses a user config directory, not the repo or `site-packages`: `$XDG_CONFIG_HOME/douyin-cli/config/settings.json`, `~/.config/douyin-cli/config/settings.json`, or `%APPDATA%\douyin-cli\config\settings.json`.
- `DOUYIN_HOME` can be used to override the writable app directory for tests.
- `douyin auth` is the official OAuth flow by default: `login`, `code`, `refresh`, `status`, `logout`.
- Official write operations stay under `douyin api`, require access token/open_id/scope, and ask for confirmation unless `--yes` is passed.
- Local subtitle generation lives under `douyin subtitle` and uses optional extras: `subtitle`, `subtitle-cuda`, or `subtitle-mac`.

## Common Commands

```bash
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --redirect-uri "https://example.com/callback" --scope user_info --scope item.comment
douyin auth code --code "$DOUYIN_AUTH_CODE"
douyin auth status
douyin api client-token --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET"
douyin api userinfo --token "$DOUYIN_ACCESS_TOKEN" --open-id "$DOUYIN_OPEN_ID"
douyin api comment-reply --token "$DOUYIN_ACCESS_TOKEN" --open-id "$DOUYIN_OPEN_ID" --item-id "$DOUYIN_ITEM_ID" --comment-id "$DOUYIN_COMMENT_ID" --content "谢谢反馈"
douyin api im-message-send --token "$DOUYIN_ACCESS_TOKEN" --open-id "$DOUYIN_OPEN_ID" --to-user-id "$DOUYIN_TO_USER_ID" --text "你好" --yes
douyin api request GET /oauth/userinfo/ --token "$DOUYIN_ACCESS_TOKEN" --param open_id="$DOUYIN_OPEN_ID"
douyin subtitle video.mp4 --language zh
douyin subtitle video.mp4 --backend mlx-whisper --language zh
```

## Maintenance Rules

- Keep `pyproject.toml` as the single uv configuration source; do not recreate `uv.toml`.
- Keep default docs and help focused on official OpenAPI integration.
- If adding dependencies, use `uv add`.
- After dependency or build config edits, run `uv lock`, `uv build --wheel`, and `uv run douyin --help`.
