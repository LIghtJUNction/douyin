---
name: douyin-hidden-commands
description: "This skill should be used when working with douyin hidden commands, undocumented command surfaces, cookie auth commands, root compatibility crawling, Obscura integration commands, or when the user asks about commands not shown in the normal CLI help."
---

# Douyin CLI Hidden Commands

Use this skill to discover and operate douyin command surfaces that are easy
to miss from the default help output. Treat "hidden" here as two categories:
Click-hidden commands and non-primary command surfaces that are intentionally
documented less prominently than the official OpenAPI flow.

## Discovery Workflow

Inspect command registration before answering. Start with:

```bash
rg -n "hidden=|@.*command|add_command|invoke_without_command|cookie-login|obscura" douyin_cli README.md tests
uv run douyin --help
uv run douyin auth --help
uv run douyin obscura --help
```

Use `DOUYIN_HOME=/tmp/douyin-hidden-check` when commands may read or write
auth/config state during verification.

## Hidden Or Easy-To-Miss Commands

### `douyin comment`

`douyin comment` is Click-hidden in `douyin_cli/commands/comment.py`, so it is
not listed in the root help. Use it for webpage-cookie comment collection from a
single aweme/note URL.

```bash
douyin comment "https://www.douyin.com/video/..."
douyin comment "https://www.douyin.com/video/..." --limit 100 --output comments.jsonl
```

It uses saved Cookie auth by default and also accepts `--cookie` for one-off
runs. Do not confuse this with official OpenAPI comment commands under
`douyin api`, which require `access_token`, `open_id`, and appropriate scopes.

### `douyin api im-message-send`

This is an official enterprise OpenAPI command, not a cookie/web crawler flow.
It requires the app to have `enterprise.im` permission and a `to_user_id` from
Douyin private-message event callbacks.

```bash
douyin api im-message-send --to-user-id "$DOUYIN_TO_USER_ID" --text "你好" --yes
```

### `douyin auth cookie-login`

Cookie auth commands live under `douyin auth` but are separate from the official
OpenAPI OAuth path. Use them for webpage-compatible crawling/search/comment
flows that need browser Cookie state.

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status
douyin auth cookie-logout
```

Cookie values are saved in the user config file, not the repository:
`$XDG_CONFIG_HOME/douyin-cli/config/settings.json`,
`~/.config/douyin-cli/config/settings.json`, or
`%APPDATA%\douyin-cli\config\settings.json`. Use `DOUYIN_HOME` to isolate tests.

### Root Compatibility Crawl

The root command can run the legacy webpage crawler directly when URL/search
options are provided. This is not a subcommand, so command discovery must inspect
`douyin_cli/commands/root.py` and `douyin_cli/commands/compat.py`.

```bash
douyin -u "搜索关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin -u "https://www.douyin.com/user/..." -t post -l 20
```

Use saved Cookie auth or pass `--cookie` for a single run. When no crawl options
are provided, the root command prints help instead of crawling.

### `douyin obscura`

Obscura commands are automation metadata helpers. Use them when another tool or
agent needs machine-readable command metadata or local Obscura detection.

```bash
douyin obscura manifest
douyin obscura status
douyin obscura status --binary obscura
```

Prefer JSON outputs from Obscura/status surfaces for automation rather than
parsing human help text.

## Verification

For command-surface changes, run targeted help and tests:

```bash
uv run pytest tests/test_cookie_status.py tests/test_obscura.py tests/test_comments.py
DOUYIN_HOME=/tmp/douyin-hidden-check uv run douyin auth --help
DOUYIN_HOME=/tmp/douyin-hidden-check uv run douyin obscura manifest
```

If full CLI help is involved, also verify Windows-style encoding behavior:

```bash
PYTHONIOENCODING=cp1252 uv run python -m douyin_cli.cli --help
```

Avoid running live Douyin network requests unless the user explicitly asks for
real endpoint validation or provides valid Cookie/OpenAPI credentials.
