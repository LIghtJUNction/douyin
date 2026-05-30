![douyin-cli](https://socialify.git.ci/LIghtJUNction/douyin-cli/image?description=1&font=Source%20Code%20Pro&forks=1&issues=1&language=1&owner=1&pattern=Circuit%20Board&stargazers=1&theme=Auto)

# douyin-cli

面向抖音开放平台官方 OpenAPI 的命令行工具，提供 OAuth 授权、token 管理、常用官方接口封装和通用 OpenAPI 请求能力。

## 功能

- 官方 OAuth 授权链接生成、code 换 token、token 刷新
- 官方 `client_token`、`access_token` 管理
- 授权用户信息查询
- 官方评论列表、评论回复列表、评论回复
- 任意官方 OpenAPI 路径请求
- 可选本地字幕生成
- Obscura/自动化运行时集成

## 安装

```bash
uv tool install douyin-cli
```

开发安装：

```bash
uv tool install -e .
```

字幕可选依赖：

```bash
uv tool install 'douyin-cli[subtitle]'
uv tool install 'douyin-cli[subtitle-cuda]'
```

## Agent Skill

安装本仓库配套 skill：

```bash
bunx skills add LIghtJUNction/douyin-cli -g
npx skills add LIghtJUNction/douyin-cli -g
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

`douyin-cli` 提供稳定的 JSON 输出和集成 manifest，Obscura 可以直接发现命令能力并调用官方 OpenAPI。

```bash
douyin obscura manifest
douyin obscura status
douyin auth status --json
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

通用请求：

```bash
douyin api request GET /oauth/userinfo/ \
  --token "$DOUYIN_ACCESS_TOKEN" \
  --param open_id="$DOUYIN_OPEN_ID"
```

## 本地字幕

```bash
douyin subtitle video.mp4 --language zh
douyin subtitle video.mp4 --model small --format srt
```

首次使用模型时会自动从 Hugging Face 下载。CUDA 模式需要 CUDA 12 运行库；如果系统只提供 CUDA 13，可安装 `douyin-cli[subtitle-cuda]`，或使用 CPU 模式：

```bash
douyin subtitle video.mp4 --device cpu --compute-type int8 --language zh
```

## 环境变量

- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`

## 技术栈

- Python 3.13
- Click
- niquests
- uv / uv-build
