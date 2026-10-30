<div align="center">
  <h1>douyin-cli</h1>
  <p><strong>把抖音网页工作流、官方 OpenAPI 和离线内容分析放进一个 Rust CLI。</strong></p>
  <p>采集与下载 · 评论 · OAuth · 私信 · 统计 · stdio MCP</p>

  [![Crates.io](https://img.shields.io/crates/v/douyin-cli?style=flat-square&logo=rust)](https://crates.io/crates/douyin-cli)
  [![AUR](https://img.shields.io/badge/AUR-douyin--cli-111111?style=flat-square&logo=archlinux)](https://aur.archlinux.org/packages/douyin-cli)
  [![Rust](https://img.shields.io/badge/Rust-1.88%2B-111111?style=flat-square&logo=rust)](Cargo.toml)
  [![License](https://img.shields.io/badge/license-AGPL--3.0--or--later-111111?style=flat-square)](LICENSE)

  [安装](#安装) · [认证模型](#先理解认证模型) · [快速使用](#快速使用) · [MCP](#mcp-服务器) · [完整手册](USAGE.md)
</div>

## 三条能力线

```text
浏览器 Cookie ─→ 搜索 / 主页 / 单作品 / 网页评论 / 下载

官方 OAuth ───→ 用户信息 / 官方评论 / 评论回复 / 私信 / MCP

本地文件 ─────→ 热词 / 热梗 / 需求线索 / 作品元数据统计
```

三条线可以独立使用。离线分析不访问网络；网页 Cookie 不能替代 OpenAPI token，OAuth token 也不能替代网页 Cookie。

## 安装

需要 Rust 1.88 或更高版本。网页采集与评论功能还需要系统中存在 `node`；官方 OpenAPI 与离线统计不依赖 Node.js。

### Arch Linux

```bash
paru -S douyin-cli
# 或：yay -S douyin-cli
```

### crates.io

```bash
cargo install douyin-cli --locked
douyin --version
douyin --help
```

更新：

```bash
cargo install douyin-cli --locked --force
```

### 源码

```bash
git clone https://github.com/LIghtJUNction/douyin.git
cd douyin
cargo install --path . --locked
```

## 先理解认证模型

| 能力 | 凭据 | 是否需要开放平台应用 |
| --- | --- | --- |
| 网页搜索、主页作品、单作品、收藏、网页评论 | 浏览器 Cookie | 否 |
| 官方用户信息、评论接口、私信、授权型 MCP 工具 | OAuth access token | 是 |
| 本地洞察和统计 | 无 | 否 |

### 浏览器 Cookie

在浏览器登录抖音网页端，从开发者工具的请求头复制完整 Cookie 值，不要包含 `Cookie:` 前缀：

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=..."
douyin auth cookie-status --offline
douyin auth cookie-status
```

`--offline` 只验证本地格式；联网状态检查会请求登录态接口。若平台要求验证码、触发风控或接口发生变化，工具会报告“无法确认”，不会把匿名接口成功误判成已登录。

也可只在当前 shell 注入：

```bash
export DOUYIN_COOKIE="完整 Cookie 字符串"
douyin auth cookie-login
unset DOUYIN_COOKIE
```

清除本地 Cookie：

```bash
douyin auth cookie-logout
```

> [!CAUTION]
> Cookie、`client_secret` 和 token 都是凭据。不要发到聊天记录、命令日志、Issue 或 Git 仓库；不要采集、下载或处理你无权访问的数据。

### 官方 OAuth

在抖音开放平台创建应用，配置允许的回调地址和 scope。推荐本机监听：

```bash
export DOUYIN_CLIENT_KEY="你的 client_key"
export DOUYIN_CLIENT_SECRET="你的 client_secret"

douyin auth login \
  --scope user_info \
  --scope item.comment \
  --listen \
  --callback-port 8787
```

开放平台应用需允许 `http://127.0.0.1:8787/callback`。无法本地监听时，先生成授权链接，再手动交换回调 code：

```bash
douyin auth login \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --scope item.comment

douyin auth code --code "回调中的授权码"
```

```bash
douyin auth status --json
douyin auth refresh
douyin auth logout
```

评论接口使用 `item.comment`；私信使用 `im.direct_message`；图片发送还需 `tool.image.upload`。实际可用接口取决于应用审核、授权 scope 与平台规则。

## 快速使用

### 搜索与下载

只采集元数据：

```bash
douyin -u "搜索关键词" -t search -l 5 --no-download
```

下载单作品或主页作品：

```bash
douyin -u "https://www.douyin.com/video/作品ID" -t aweme
douyin -u "https://www.douyin.com/user/用户ID" -t post -l 20
```

批量目标：

```bash
douyin -u targets.txt -p ./downloads \
  --download-title \
  --download-cover
```

`-t` 支持 `post`、`favorite`、`music`、`hashtag`、`search`、`following`、`follower`、`collection`、`mix` 和 `aweme`。`collection` 读取当前 Cookie 账号收藏，不传 `-u`。

### 评论

```bash
douyin comment "https://www.douyin.com/video/作品ID" --limit 100

douyin comment "https://www.douyin.com/video/作品ID" \
  --with-replies \
  --reply-limit 50 \
  --format chatml-jsonl \
  --output comments.jsonl
```

输出支持 `raw`、`chatml-jsonl` 与 `chatml-json`。

### 离线热词与需求线索

```bash
douyin insights comments.json --top 20 --min-count 2
cat comments.jsonl | douyin insights - --format markdown -o insights.md
```

分析过程是确定性的：停用词过滤、重复频次、互动权重和意图关键词匹配。它不会调用模型，也不代表工具真正理解语义。输出中的“需求”是可解释线索，不是市场结论。

### 离线作品统计

```bash
douyin stats search_陈震同学.json \
  --author 陈震同学 \
  --sort score \
  --top 10
```

综合分只在当前输入集合内比较。它对点赞、评论、收藏、分享分别做 `ln(1+x)` 归一化，再按 35%、20%、20%、25% 加权。由于采集元数据不含播放量，这不是互动率。

### 官方 OpenAPI

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
douyin api comment-replies \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID"
```

写操作默认要求确认：

```bash
douyin api comment-reply \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID" \
  --content "谢谢反馈"
```

私信回复：

```bash
douyin api im-message-send \
  --to-user-id "$DOUYIN_TO_USER_ID" \
  --msg-id "$DOUYIN_MSG_ID" \
  --conversation-id "$DOUYIN_CONVERSATION_ID" \
  --text "你好，已收到" \
  --yes
```

通用请求只接受当前 OpenAPI 基地址的同源路径，避免把 token 发往任意主机：

```bash
douyin api request GET /oauth/userinfo/ \
  --param open_id="$DOUYIN_OPEN_ID"
```

## MCP 服务器

`douyin mcp` 通过 stdio 暴露官方 OpenAPI 工具，并复用已保存的 OAuth。`hot_words`、`hot_memes` 和 `demand_discovery` 是纯离线工具，不需要授权。

```bash
douyin mcp
```

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

```bash
claude mcp add douyin -- douyin mcp
codex mcp add douyin -- douyin mcp
```

MCP 只是把已有命令变成结构化工具调用，不会绕过 OAuth、scope、确认或平台权限。

## 配置

| 平台 | 默认路径 |
| --- | --- |
| Linux / macOS | `~/.config/douyin-cli/config/settings.json` |
| Windows | `%APPDATA%\douyin-cli\config\settings.json` |
| 测试隔离 | 通过 `DOUYIN_HOME` 指定 |

常用环境变量：`DOUYIN_COOKIE`、`DOUYIN_CLIENT_KEY`、`DOUYIN_CLIENT_SECRET`、`DOUYIN_ACCESS_TOKEN`、`DOUYIN_HOME`。

所有参数以 CLI 自带帮助为准：

```bash
douyin --help
douyin auth --help
douyin api --help
douyin comment --help
douyin insights --help
douyin stats --help
```

更多示例见 [USAGE.md](USAGE.md)。

## Agent Skill

```bash
bunx skills add LIghtJUNction/douyin -g
# 或：npx skills add LIghtJUNction/douyin -g
```

## 开发验证

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
cargo package --locked
```

## License

[AGPL-3.0-or-later](LICENSE)
