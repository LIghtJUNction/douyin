![douyin](https://socialify.git.ci/LIghtJUNction/douyin/image?description=1&font=Source%20Code%20Pro&forks=1&issues=1&language=1&owner=1&pattern=Circuit%20Board&stargazers=1&theme=Auto)

# douyin-cli

面向抖音开放平台官方 OpenAPI 的命令行工具，提供 OAuth 授权、token 管理、常用官方接口封装和通用 OpenAPI 请求能力。

## 功能

- 官方 OAuth 授权链接生成、code 换 token、token 刷新
- 官方 `client_token`、`access_token` 管理
- 授权用户信息查询
- 官方评论列表、评论回复列表、评论回复
- 企业号私信消息发送
- 任意官方 OpenAPI 路径请求
- stdio MCP 服务器，供 MCP 客户端调用抖音 OpenAPI 工具
- 可选本地字幕生成
- Obscura/自动化运行时集成

## 安装

作为 Python 库安装：

```bash
uv add douyin-cli
```

命令行安装：

```bash
uv tool install douyin-cli
```

开发安装：

```bash
uv tool install -e .
uv tool install -e '.[subtitle-mac]'
```

字幕可选依赖：

```bash
uv tool install 'douyin-cli[subtitle]'
uv tool install 'douyin-cli[subtitle-cuda]'
uv tool install 'douyin-cli[subtitle-mac]'
```

Python 库调用示例：

```python
from douyin_cli.douyin import Douyin
from douyin_cli.douyin.openapi import DouyinOpenAPIClient

with DouyinOpenAPIClient() as client:
    token_data = client.client_token("client_key", "client_secret")

douyin = Douyin(target="搜索关键词", type="search", limit=5, cookie="sessionid=...")
douyin.run()
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

## Obscura 集成

`douyin` 提供稳定的 JSON 输出和集成 manifest，Obscura 可以直接发现命令能力并调用官方 OpenAPI。

```bash
douyin obscura manifest
douyin obscura status
douyin auth status --json
```

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
douyin subtitle video.mp4 --model Qwen/Qwen3-ASR-1.7B --format srt
```

首次使用模型时会自动从 Hugging Face 下载。CUDA 模式需要 CUDA 12 运行库；如果系统只提供 CUDA 13，可安装 `douyin-cli[subtitle-cuda]`，或使用 CPU 模式：

```bash
douyin subtitle video.mp4 --device cpu --compute-type int8 --language zh
```

macOS Apple Silicon 可安装 MLX 后端使用本机 GPU：

```bash
uv tool install 'douyin-cli[subtitle-mac]'
douyin subtitle video.mp4 --backend mlx-whisper --language zh
```

`--backend auto` 会在 macOS arm64 上优先使用 `mlx-whisper`，其他平台默认使用 `qwen-asr` 和 `Qwen/Qwen3-ASR-1.7B`。如需旧的 Whisper 路径，可显式使用 `--backend faster-whisper --model Systran/faster-whisper-small`。`--compute-type` 只影响 `faster-whisper` 后端。

## 环境变量

- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`

## 技术栈

- Python 3.13
- Click
- MCP Python SDK
- niquests
- uv / uv-build
