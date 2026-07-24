---
name: douyin
description: "Use this skill when installing, upgrading, developing, testing, packaging, or troubleshooting this repository's Rust douyin CLI, including OAuth/OpenAPI, Cookie web workflows, MCP, crawling, comments, and downloads."
---

# Douyin Rust CLI

Treat this repository as a Rust-only CLI. Keep `Cargo.toml` and committed
`Cargo.lock` as the build source. Do not reintroduce Python packaging.

## Inspect Before Acting

Confirm the current command surface and worktree:

```bash
git status -sb
cargo run --locked --offline -- --help
cargo run --locked --offline -- auth --help
cargo run --locked --offline -- api --help
```

Keep every command visible in the applicable Clap help output.

## Install Or Upgrade

Use crates.io for users and `--path .` for repository development:

```bash
cargo install douyin-cli --locked
cargo install douyin-cli --locked --force
cargo install --path . --locked
douyin --version
```

Require Rust 1.88 or newer. Require `node` only for webpage crawling and
comments because those flows use the bundled JavaScript signer.

## Select Authentication Correctly

- Use Cookie auth for search, webpage crawling, downloads, and `douyin comment`.
- Use official OAuth for `douyin api` and `douyin mcp`.
- Never claim Cookie and OAuth credentials are interchangeable.
- Never ask the user to paste a real Cookie or secret into chat.

Cookie flow:

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status --offline
douyin auth cookie-status
douyin auth cookie-logout
```

Treat `cookie-status --offline` as local format validation only. Ordinary
`cookie-status` uses the login-state endpoint and may report that the state
cannot be confirmed when Douyin returns a captcha, risk-control page, or an
unrecognized upstream response. Never use a successful anonymous web endpoint
as proof of authentication, and never expose Cookie values or response bodies.

OAuth flow:

```bash
douyin auth login --client-key "$DOUYIN_CLIENT_KEY" --client-secret "$DOUYIN_CLIENT_SECRET" --scope user_info --listen --callback-port 8787
douyin auth status
douyin auth refresh
douyin auth logout
```

Ensure the platform application allows the local callback URL before using
`--listen`. Use `douyin auth code --code <code>` for manual callbacks.

## Exercise Core Workflows

```bash
douyin -u "关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin comment "https://www.douyin.com/video/..." --with-replies --format chatml-jsonl --output comments.jsonl
douyin api userinfo
douyin mcp
```

Use saved auth by default. Use `DOUYIN_HOME` to isolate local auth/config state
during tests. Avoid live Douyin requests unless the user explicitly requests
endpoint validation or supplies credentials for that purpose.

## Verify Changes

After Rust, dependency, CLI, documentation, or packaging changes, run:

```bash
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --release --locked --offline
cargo package --locked --offline --allow-dirty
cargo run --locked --offline -- --help
```

Ensure `src/cookie.rs` appears in `cargo package --list`; the root `/cookie.*`
ignore rule must never match Rust source files.
