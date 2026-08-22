---
name: douyin
description: Develop, install, test, package, troubleshoot, or operate this Rust douyin CLI, including Cookie web crawls, OAuth/OpenAPI, comments, downloads, MCP, and Obscura.
---

# Douyin Rust CLI

Treat this repository as a Rust-only CLI. `Cargo.toml` and committed `Cargo.lock` are the build source; do not reintroduce Python packaging.

## Discover Before Acting

Use current Clap help rather than memory:

```bash
git status -sb
cargo run --locked --offline -- --help
cargo run --locked --offline -- auth --help
cargo run --locked --offline -- api --help
cargo run --locked --offline -- comment --help
cargo run --locked --offline -- obscura --help
```

If docs and help disagree, inspect `src/cli.rs` and the owning Rust module, then update the docs. Keep every public command visible in applicable help.

## Install or Upgrade

```bash
cargo install douyin-cli --locked
cargo install douyin-cli --locked --force
cargo install --path . --locked
douyin --version
```

Require Rust 1.88+. Node is needed only by webpage crawling/comment flows that use the bundled JavaScript signer.

## Route Authentication

Use Cookie auth for search, webpage crawls, comments, and downloads. Use official OAuth for `douyin api` and `douyin mcp`. Credentials are not interchangeable.

Never ask the user to paste a real Cookie, client secret, or token into chat; keep secrets and response bodies out of logs and repositories.

### Cookie

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status --offline
douyin auth cookie-status
douyin auth cookie-logout
```

`--offline` validates local format only. Online status uses Douyin's login-state endpoint and may be unable to confirm under captcha, risk control, or an unrecognized response. An anonymous endpoint succeeding is not authentication proof.

Use saved Cookie state, `--cookie`, or `DOUYIN_COOKIE` for web workflows.

### OAuth

```bash
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --scope user_info --listen --callback-port 8787
douyin auth status
douyin auth refresh
douyin auth logout
```

Ensure the platform app allows the callback URL. Use `douyin auth code --code <code>` for a manual callback.

## Web Workflows

```bash
douyin -u "关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin -u "https://www.douyin.com/user/..." -t post -l 20
douyin comment "https://www.douyin.com/video/..." --limit 100 --with-replies --format chatml-jsonl --output comments.jsonl
```

Root crawl types: `post`, `favorite`, `music`, `hashtag`, `search`, `following`, `follower`, `collection`, `mix`, and `aweme`.

Avoid live Douyin requests unless the user explicitly requests endpoint validation or provides credentials for that purpose.

## OpenAPI, MCP, and Obscura

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
douyin api request GET /oauth/userinfo/ --param open_id="$DOUYIN_OPEN_ID"
douyin mcp
douyin obscura manifest
```

Only send same-origin OpenAPI paths. Treat write operations as confirmation-gated unless the user explicitly supplies `--yes`.

Use `DOUYIN_HOME` to isolate auth/config state during tests:

```bash
DOUYIN_HOME=/tmp/douyin-command-check cargo run --locked --offline -- auth --help
DOUYIN_HOME=/tmp/douyin-command-check cargo run --locked --offline -- obscura manifest
```

## Verify Changes

After Rust, dependency, CLI, docs, or packaging changes, run:

```bash
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --release --locked --offline
cargo package --locked --offline --allow-dirty
cargo run --locked --offline -- --help
```

Ensure `src/cookie.rs` appears in `cargo package --list`; the root `/cookie.*` ignore rule must never match Rust source files.
