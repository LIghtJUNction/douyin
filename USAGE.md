# 使用指南

## 账号授权

命令行安装：

```bash
cargo install --path .
```

当前命令面均由 Rust 实现，包括 OAuth、OpenAPI、Cookie 网页采集/下载、网页评论、本地字幕、stdio MCP 与 Obscura。

`douyin auth` 默认使用抖音开放平台官方 OAuth。

生成授权链接并保存应用配置：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --scope item.comment
```

用户完成授权后，用回调得到的 `code` 换取并保存 token：

```bash
douyin auth code --code "授权码"
```

如需本机自动捕获回调，可使用 `--listen`，并在开放平台应用中允许对应本机回调地址：

```bash
douyin auth login \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --scope user_info \
  --listen \
  --callback-port 8787
```

刷新和检查：

```bash
douyin auth refresh
douyin auth status
douyin auth logout
```

授权配置保存在系统用户配置目录，例如：

```text
~/.config/douyin-cli/config/settings.json
```

## 官方 OpenAPI 命令

获取 `client_token`：

```bash
douyin api client-token \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET"
```

生成 OAuth 授权链接：

```bash
douyin api authorize-url \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --scope item.comment
```

用 OAuth code 换取 `access_token`：

```bash
douyin api access-token \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --client-secret "$DOUYIN_CLIENT_SECRET" \
  --code "授权码"
```

刷新和续期：

```bash
douyin api refresh-token \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --refresh-token "$DOUYIN_REFRESH_TOKEN"

douyin api renew-refresh-token \
  --client-key "$DOUYIN_CLIENT_KEY" \
  --refresh-token "$DOUYIN_REFRESH_TOKEN"
```

授权用户信息：

```bash
douyin api userinfo \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID"
```

官方评论接口：

```bash
douyin api comment-list \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID" \
  --item-id "$DOUYIN_ITEM_ID"

douyin api comment-replies \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID" \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID"

douyin api comment-reply \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --open-id "$DOUYIN_OPEN_ID" \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID" \
  --content "谢谢反馈"
```

写操作默认会二次确认，自动化场景可加 `--yes`。

企业号私信发送需要应用已开通 `enterprise.im` 权限，并从私信事件回调中拿到接收方 `to_user_id`：

```bash
douyin api im-message-send \
  --to-user-id "$DOUYIN_TO_USER_ID" \
  --text "你好，已收到" \
  --yes
```

通用官方 OpenAPI 请求：

```bash
douyin api request GET /oauth/userinfo/ \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --param open_id="$DOUYIN_OPEN_ID"

douyin api request POST /item/comment/reply/ \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --param open_id="$DOUYIN_OPEN_ID" \
  --json '{"item_id":"xxx","comment_id":"xxx","content":"谢谢反馈"}'
```

## 网页端 Cookie 与兼容采集

网页端 Cookie 流程独立于官方 OpenAPI OAuth，适合搜索、主页作品、单作品、评论抓取等兼容采集场景。长期保存 Cookie 请使用 `cookie-login`；一次性运行可传 `--cookie`。

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status
douyin -u "搜索关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/user/..." -t post -l 20
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin auth cookie-logout
```

隐藏命令 `douyin comment` 用于从单个作品抓取评论区，不在根命令帮助中展示：

```bash
douyin comment "https://www.douyin.com/video/..." --limit 100 --output comments.json
douyin comment "https://www.douyin.com/video/..." \
  --with-replies \
  --reply-limit 50 \
  --format chatml-jsonl \
  --output comments.jsonl
```

常用输出格式：

- `raw`：原始评论 JSON
- `chatml-jsonl`：每行一条 ChatML 训练样本
- `chatml-json`：JSON 数组格式的 ChatML 训练样本

## MCP 服务器

`douyin mcp` 通过 stdio 启动抖音 MCP 服务器，适合 Claude Desktop、Codex、Claude Code 等 MCP 客户端直接调用抖音开放平台工具。

```bash
douyin mcp
```

客户端配置示例：

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

服务器默认读取 `douyin auth` 保存的 `access_token` 和 `open_id`。如果工具调用参数里显式传入 `token` 或 `open_id`，会优先使用传入值。

当前 MCP 工具：

- `auth_status`：查看本机保存的授权状态
- `userinfo`：获取授权用户信息
- `comment_list`：获取视频评论列表
- `comment_replies`：获取评论回复列表
- `comment_reply`：回复视频或评论
- `im_message_send`：发送企业号私信消息
- `openapi_request`：调用任意官方 OpenAPI 路径

## 网页采集与下载

根命令直接运行网页采集器。先保存 Cookie，或用 `--cookie` 仅为本次运行传入：

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin -u "搜索关键词" -t search -l 5 --no-download
douyin -u "https://www.douyin.com/video/..." -t aweme
douyin -u "https://www.douyin.com/user/..." -t post -l 20
douyin -u targets.txt -p ./downloads --download-title --download-cover
```

`-t` 支持 `post`、`favorite`、`music`、`hashtag`、`search`、`following`、`follower`、`collection`、`mix` 和 `aweme`。搜索还可以传入 `--sort-type`、`--publish-time` 与 `--filter-duration`。`--no-download` 仅写 JSON 数据与 aria2 兼容下载清单；默认同时下载媒体文件。

## 本地字幕

字幕功能使用 Rust 音频解码与 whisper.cpp，从本地视频或音频生成 SRT、VTT、TXT 或 JSON。macOS 构建默认启用 Metal，Linux/Windows 默认使用 CPU。

生成字幕：

```bash
douyin subtitle video.mp4 --language zh
douyin subtitle voice.mp3 --language zh
douyin subtitle meeting.wav --format txt
douyin subtitle video.mp4 --format vtt
douyin subtitle *.mp4 --output subtitles/
```

输入可以是本地视频或音频文件。默认输出路径会沿用输入文件名，只替换字幕后缀，例如 `voice.mp3` 生成 `voice.srt`。

首次使用 `tiny`、`base`、`small`、`medium`、`large-v3` 或 `turbo` 等模型别名时，会自动从 whisper.cpp 的 Hugging Face 仓库下载 GGML 模型。也可以直接指定本地模型：

```bash
douyin subtitle video.mp4 \
  --model ./ggml-small.bin \
  --local-files-only
```

可用 `--model-cache-dir` 更改模型缓存目录。需要 CUDA 时重新构建：

```bash
cargo install --path . --features cuda
douyin subtitle video.mp4 --device cuda --language zh
```

强制 CPU：

```bash
douyin subtitle video.mp4 --device cpu --language zh
```

`--compute-type` 是旧版兼容参数，whisper.cpp 的精度由 GGML 模型量化格式决定。

## 环境变量

- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`
