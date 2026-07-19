---
name: douyin
description: "Use this skill when working with this repository's Rust douyin CLI: building with Cargo, running official Douyin OpenAPI or web commands, managing auth, generating subtitles, or validating the native binary."
---

# Douyin CLI

This repository is a Rust CLI. Do not add a frontend, web server, Docker packaging, or Python packager unless the user explicitly requests it.

## Install And Verify

For local installs and development:

```bash
cargo install --path .
douyin --help
```

Build and verify with Cargo:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
cargo run --locked -- api --help
```

## Runtime Notes

- Binary entry point: `src/main.rs`, producing `douyin`.
- Build and dependency source: `Cargo.toml` plus committed `Cargo.lock`.
- CLI state uses a user config directory, not the repo or `site-packages`: `$XDG_CONFIG_HOME/douyin-cli/config/settings.json`, `~/.config/douyin-cli/config/settings.json`, or `%APPDATA%\douyin-cli\config\settings.json`.
- `DOUYIN_HOME` can be used to override the writable app directory for tests.
- `douyin auth` is the official OAuth flow by default: `login`, `code`, `refresh`, `status`, `logout`; `login` can print a QR code and can use `--listen` to capture a local callback.
- Official write operations stay under `douyin api`, require access token/open_id/scope, and ask for confirmation unless `--yes` is passed.
- OAuth, OpenAPI, Cookie auth, root crawling/downloads, webpage comments, subtitles, stdio MCP, and Obscura are native Rust modules.
- The webpage comment command uses the bundled signing JavaScript through `node`; Rust handles HTTP, pagination, normalization, formats, and output.
- The root crawler also reuses the bundled signing JavaScript through `node`; data parsing, manifests, retries, and atomic downloads are implemented in Rust.
- Subtitle decoding uses Symphonia and inference uses whisper.cpp through `whisper-rs`; model aliases resolve to GGML files.

## Common Commands

```bash
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --redirect-uri "https://example.com/callback" --scope user_info --scope item.comment
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --scope user_info --listen --callback-port 8787
douyin auth code --code "$DOUYIN_AUTH_CODE"
douyin auth status
douyin api client-token --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET"
douyin api userinfo --token "$DOUYIN_ACCESS_TOKEN" --open-id "$DOUYIN_OPEN_ID"
douyin api comment-reply --token "$DOUYIN_ACCESS_TOKEN" --open-id "$DOUYIN_OPEN_ID" --item-id "$DOUYIN_ITEM_ID" --comment-id "$DOUYIN_COMMENT_ID" --content "谢谢反馈"
douyin api im-message-send --token "$DOUYIN_ACCESS_TOKEN" --open-id "$DOUYIN_OPEN_ID" --to-user-id "$DOUYIN_TO_USER_ID" --text "你好" --yes
douyin api request GET /oauth/userinfo/ --token "$DOUYIN_ACCESS_TOKEN" --param open_id="$DOUYIN_OPEN_ID"
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin -u "搜索关键词" -t search -l 5 --no-download
douyin comment "https://www.douyin.com/video/..." --with-replies --format chatml-jsonl --output comments.jsonl
douyin subtitle video.mp4 --language zh
douyin subtitle video.mp4 --backend whisper-cpp --device cpu --language zh
```

## Maintenance Rules

- Keep `Cargo.toml` as the Rust dependency and build source.
- Keep default docs and help focused on official OpenAPI integration.
- After dependency or build config edits, update `Cargo.lock`, then run formatting, tests, Clippy, a release build, and CLI help smoke tests.
