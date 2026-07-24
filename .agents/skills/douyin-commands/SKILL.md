---
name: douyin-commands
description: "Use this skill for douyin CLI command discovery and operation, especially choosing Cookie versus OAuth login, running root crawls, collecting comments, calling OpenAPI, starting MCP, or checking Obscura integration."
---

# Douyin Command Guide

Discover commands from the Rust binary instead of relying on memory:

```bash
douyin --help
douyin auth --help
douyin api --help
douyin comment --help
```
All commands are public. If documentation and help disagree, inspect `src/cli.rs`
and the relevant Rust module, then update the documentation.

## Route Authentication

Use browser Cookie for webpage endpoints:

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status --offline
douyin auth cookie-status
```

Use official OAuth for OpenAPI and MCP:

```bash
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --scope user_info --listen
douyin auth status
```

Explain that `cookie-status --offline` checks format only, while ordinary
`cookie-status` asks the login-state endpoint to distinguish logged in, logged
out or expired, and unable to confirm. Captcha, risk-control, and unrecognized
upstream responses are unable-to-confirm results. A successful anonymous web
endpoint is not authentication proof. Keep Cookie values and response bodies
out of logs, chat, and repositories.

## Run Web Workflows

```bash
douyin -u "关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin -u "https://www.douyin.com/user/..." -t post -l 20
douyin comment "https://www.douyin.com/video/..." --limit 100
```

Use `post`, `favorite`, `music`, `hashtag`, `search`, `following`, `follower`,
`collection`, `mix`, or `aweme` as the root crawl type. Use saved Cookie auth,
`--cookie`, or `DOUYIN_COOKIE`.

## Run Official And Local Workflows

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
douyin api request GET /oauth/userinfo/ --param open_id="$DOUYIN_OPEN_ID"
douyin mcp
douyin obscura manifest
```

Treat OpenAPI write commands as confirmation-gated unless `--yes` is explicit.
Only use same-origin OpenAPI paths.

## Verify Command Changes

```bash
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
DOUYIN_HOME=/tmp/douyin-command-check cargo run --locked --offline -- auth --help
DOUYIN_HOME=/tmp/douyin-command-check cargo run --locked --offline -- obscura manifest
```
