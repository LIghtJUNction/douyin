# 使用指南

## 账号授权

命令行安装：

```bash
uv tool install douyin-cli
```

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

服务器默认读取 `douyin auth` 保存的 `access_token` 和 `open_id`。如果工具调用参数里显式传入 `token` 或 `open_id`，会优先使用传入值。

当前 MCP 工具：

- `auth_status`：查看本机保存的授权状态
- `userinfo`：获取授权用户信息
- `comment_list`：获取视频评论列表
- `comment_replies`：获取评论回复列表
- `comment_reply`：回复视频或评论
- `im_message_send`：发送企业号私信消息
- `openapi_request`：调用任意官方 OpenAPI 路径

## 本地字幕

字幕功能默认基于 `Qwen/Qwen3-ASR-1.7B`，从本地视频或音频生成字幕文件。该依赖较重，不随默认安装启用。

```bash
uv tool install 'douyin-cli[subtitle]'
```

CUDA 版本：

```bash
uv tool install 'douyin-cli[subtitle-cuda]'
```

生成字幕：

```bash
douyin subtitle video.mp4 --language zh
douyin subtitle video.mp4 --format vtt
douyin subtitle *.mp4 --output subtitles/
```

首次使用模型时会自动从 Hugging Face 下载。网络受限时可设置 `HF_ENDPOINT` 或提前缓存模型：

```bash
douyin subtitle video.mp4 \
  --model Qwen/Qwen3-ASR-1.7B \
  --model-cache-dir ~/.cache/douyin-cli/models
```

如果需要继续使用 Whisper 后端，可显式指定：

```bash
douyin subtitle video.mp4 \
  --backend faster-whisper \
  --model Systran/faster-whisper-small
```

不用 CUDA 时：

```bash
douyin subtitle video.mp4 --device cpu --compute-type int8 --language zh
```

## 环境变量

- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`
