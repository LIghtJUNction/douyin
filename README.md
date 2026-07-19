![douyin](https://socialify.git.ci/LIghtJUNction/douyin/image?description=1&font=Source%20Code%20Pro&forks=1&issues=1&language=1&owner=1&pattern=Circuit%20Board&stargazers=1&theme=Auto)

# douyin-cli

面向抖音开放平台与网页工作流的 Rust 命令行工具，提供 OAuth、OpenAPI、Cookie 抓取、媒体下载、本地字幕和自动化集成。

## 功能

- 官方 OAuth 授权链接生成、code 换 token、token 刷新
- 官方 `client_token`、`access_token` 管理
- 授权用户信息查询
- 官方评论列表、评论回复列表、评论回复
- 企业号私信消息发送
- 任意官方 OpenAPI 路径请求
- 网页作品、账号、话题、音乐、合集与搜索结果采集/下载
- 网页评论与回复采集
- stdio MCP 服务器，供 MCP 客户端调用抖音 OpenAPI 工具
- whisper.cpp 本地字幕生成
- Obscura/自动化运行时集成

## 安装

从源码安装 Rust CLI：

```bash
cargo install --path .
```

开发构建和验证：

```bash
cargo build --release --locked
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
./target/release/douyin --help
```

## Agent Skill

安装本仓库配套 skill：

```bash
bunx skills add LIghtJUNction/douyin -g
npx skills add LIghtJUNction/douyin -g
```

## 官方 OAuth 接入

准备抖音开放平台应用的 `client_key`、`client_secret`、回调地址和所需 scope。

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --scope item.comment
```

命令会输出官方授权链接。用户授权后，将回调中的 `code` 保存为 token：

```bash
douyin auth code --code "授权码"
```

也可以一步完成：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --code "授权码"
```

本地开发或内网回调调试时，也可以让 CLI 临时监听回调并自动捕获 `code`。此模式会把回调地址设置为本机监听地址，需确保开放平台应用允许对应回调地址：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --scope user_info \
  --listen \
  --callback-port 8787
```

检查和刷新授权：

```bash
douyin auth status
douyin auth refresh
douyin auth logout
```

授权后，官方 OpenAPI 命令会自动读取已保存的 token 和 `open_id`，自动化调用不需要重复传参：

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
```

## 网页端 Cookie 与兼容采集

除官方 OpenAPI 外，`douyin` 还保留网页端 Cookie 流程，用于搜索、主页作品、单作品和评论等兼容采集场景。Cookie 会保存到用户配置目录；测试或自动化时可用 `DOUYIN_HOME` 隔离状态。

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status
douyin -u "搜索关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin auth cookie-logout
```

`douyin comment` 是隐藏兼容命令，不会出现在根命令帮助中，适合从单个作品 URL 抓取评论，也可以输出适合对话微调的数据格式：

```bash
douyin comment "https://www.douyin.com/video/..." --limit 100 --output comments.json
douyin comment "https://www.douyin.com/video/..." \
  --with-replies \
  --format chatml-jsonl \
  --output comments.jsonl
```

## Obscura 集成

`douyin` 提供稳定的 JSON 输出和集成 manifest，Obscura 可以直接发现命令能力并调用官方 OpenAPI。

```bash
douyin obscura manifest
douyin obscura status
douyin auth status --json
```

## 网页评论抓取

评论抓取继续使用网页 Cookie，并复用仓库原有的 Node.js 签名脚本：

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin comment "https://www.douyin.com/video/..." --limit 100
douyin comment "https://www.douyin.com/video/..." --with-replies --format chatml-jsonl --output comments.jsonl
```

## 网页采集与下载

根命令兼容 URL、ID、搜索关键词和目标文件，默认读取 `douyin auth cookie-login` 保存的 Cookie：

```bash
douyin -u "搜索关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin -u "https://www.douyin.com/user/..." -t post -l 20
douyin -u targets.txt -p ./downloads --download-title --download-cover
```

支持 `post`、`favorite`、`music`、`hashtag`、`search`、`following`、`follower`、`collection`、`mix` 与 `aweme` 类型。采集结果保存为 JSON 和 aria2 兼容下载清单；未指定 `--no-download` 时由 Rust 下载器写入媒体文件。

## MCP 服务器

`douyin mcp` 会通过 stdio 启动 MCP 服务器，默认复用 `douyin auth` 保存的官方 OpenAPI 授权信息。

```bash
douyin mcp
```

MCP 客户端配置示例：

```json
{
  "mcpServers": {
    "douyin": {
      "command": "douyin",
      "args": ["mcp"]
    }
  }
}
```

Claude Code 命令行配置：

```bash
claude mcp add douyin -- douyin mcp
claude mcp list
claude mcp get douyin
```

Codex CLI 命令行配置：

```bash
codex mcp add douyin -- douyin mcp
codex mcp list
codex mcp get douyin
```

可用工具包括授权状态、用户信息、评论列表、评论回复、企业号私信发送和通用 OpenAPI 请求。首次使用前先完成授权：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info

douyin auth code --code "授权码"
douyin mcp
```

推荐接入顺序：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info

douyin auth code --code "授权码"
douyin auth status --json
```

## 官方 OpenAPI

```bash
douyin api client-token \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET"

douyin api userinfo \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID"

douyin api comment-list \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID" \
  --item-id "$DOUYIN_ITEM_ID"

douyin api comment-reply \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID" \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID" \
  --content "谢谢反馈"
```

企业号私信发送需要应用已开通 `enterprise.im` 权限，并从私信事件回调中拿到接收方 `to_user_id`：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope enterprise.im

douyin api im-message-send \
  --to-user-id "$DOUYIN_TO_USER_ID" \
  --text "你好，已收到" \
  --yes
```

通用请求：

```bash
douyin api request GET /oauth/userinfo/ \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --param open_id="$DOUYIN_OPEN_ID"
```

## 本地字幕

```bash
douyin subtitle video.mp4 --language zh
douyin subtitle voice.mp3 --language zh
douyin subtitle meeting.wav --format txt
douyin subtitle video.mp4 --model small --format srt
```

输入可以是本地视频或音频文件，输出支持 SRT、VTT、TXT 和 JSON。默认写到同名字幕文件，例如 `voice.mp3` 会生成 `voice.srt`。首次使用模型别名时会从 whisper.cpp 的 Hugging Face 仓库下载 GGML 模型；也可以直接传入本地 `.bin` 模型：

```bash
douyin subtitle video.mp4 --model ./ggml-small.bin --local-files-only
douyin subtitle *.mp4 --output subtitles/ --format vtt
```

macOS 构建默认启用 Metal。其他平台默认使用 CPU；需要 CUDA 时以对应特性构建：

```bash
cargo install --path . --features cuda
douyin subtitle video.mp4 --device cuda --language zh
```

`--backend auto` 与 `--backend whisper-cpp` 使用同一 Rust 原生后端。`--compute-type` 仅为旧命令兼容参数；实际量化精度由所选 GGML 模型决定。

## 环境变量

- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`

## 技术栈

- Rust 2024 edition
- Clap
- Reqwest + rustls
- Serde JSON
- Symphonia + whisper.cpp
- Cargo
