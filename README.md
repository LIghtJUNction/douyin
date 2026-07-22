![douyin](https://socialify.git.ci/LIghtJUNction/douyin/image?description=1&font=Source%20Code%20Pro&forks=1&issues=1&language=1&owner=1&pattern=Circuit%20Board&stargazers=1&theme=Auto)

# douyin-cli

面向抖音开放平台与网页工作流的 Rust 命令行工具，支持网页采集与下载、评论抓取、官方 OAuth/OpenAPI、stdio MCP 和本地字幕。

## 安装

需要 Rust 1.88 或更高版本。网页采集和评论命令还需要系统中存在 `node`；其他命令不依赖 Node.js。

Arch Linux 可通过 [AUR 的 douyin-cli 软件包](https://aur.archlinux.org/packages/douyin-cli) 安装：

```bash
paru -S douyin-cli
# 或
yay -S douyin-cli
```

从 crates.io 安装稳定版：

```bash
cargo install douyin-cli --locked
douyin --version
douyin --help
```

更新到最新版本：

```bash
cargo install douyin-cli --locked --force
```

从源码安装开发版：

```bash
git clone https://github.com/LIghtJUNction/douyin.git
cd douyin
cargo install --path . --locked
```

安装时会编译 whisper.cpp。若系统提示原生构建工具缺失，请安装 C/C++ 工具链、CMake，以及 Clang（Windows 可使用 MSVC Build Tools）。macOS 默认启用 Metal；Linux/Windows 默认使用 CPU。CUDA 构建方式见[本地字幕](#本地字幕)。

## 选择登录方式

本项目有两套相互独立的认证方式：

| 用途 | 登录方式 | 是否需要开放平台应用 |
| --- | --- | --- |
| 搜索、主页作品、单作品下载、网页评论 | 浏览器 Cookie | 否 |
| 官方用户信息、官方评论接口、企业号私信、MCP | 官方 OAuth | 是 |

Cookie 不能代替 OpenAPI token，OAuth token 也不能代替网页 Cookie。

### 使用浏览器 Cookie

先在浏览器登录抖音网页端，从开发者工具的网络请求头复制完整 Cookie 值。不要包含 `Cookie:` 前缀，也不要把真实 Cookie 发到聊天、日志或 Git 仓库。

```bash
douyin auth cookie-login --cookie "sessionid=...; ttwid=...; 其他字段=..."
douyin auth cookie-status --offline
douyin auth cookie-status
```

`--offline` 只检查本地格式；不带该参数时会请求抖音登录态接口，尝试确认当前网页登录态。匿名网页接口即使返回成功也不能证明 Cookie 有效，因此不会作为登录依据。若遇到验证码、风控或上游接口变化，命令会明确报告“无法确认”，而不会误判为已登录。检查过程不会输出 Cookie 或响应正文。

也可以通过环境变量传入，避免每次添加 `--cookie`：

```bash
export DOUYIN_COOKIE="完整 Cookie 字符串"
douyin auth cookie-login
unset DOUYIN_COOKIE
```

退出并删除保存的 Cookie：

```bash
douyin auth cookie-logout
```

### 使用官方 OAuth

先在抖音开放平台创建应用，准备 `client_key`、`client_secret`、允许的回调地址和所需 scope。

推荐使用本机回调监听。开放平台应用中需要允许 `http://127.0.0.1:8787/callback`：

```bash
export DOUYIN_CLIENT_KEY="你的 client_key"
export DOUYIN_CLIENT_SECRET="你的 client_secret"

douyin auth login \
  --scope user_info \
  --scope item.comment \
  --listen \
  --callback-port 8787
```

命令会输出授权链接和二维码，并在授权完成后自动保存 token。不能监听回调时，可以手动完成 code 交换：

```bash
douyin auth login \
  --redirect-uri "https://example.com/callback" \
  --scope user_info \
  --scope item.comment

douyin auth code --code "回调中的授权码"
```

检查、刷新或清除授权：

```bash
douyin auth status
douyin auth status --json
douyin auth refresh
douyin auth logout
```

## 基础用法

所有命令均公开显示：

```bash
douyin --help
douyin auth --help
douyin api --help
douyin comment --help
douyin subtitle --help
```

### 搜索和采集

仅采集元数据，不下载媒体：

```bash
douyin -u "搜索关键词" -t search -l 5 --no-download
```

下载单个作品或账号主页作品：

```bash
douyin -u "https://www.douyin.com/video/作品ID" -t aweme
douyin -u "https://www.douyin.com/user/用户ID" -t post -l 20
```

批量读取目标文件，并保存标题和封面：

```bash
douyin -u targets.txt -p ./downloads --download-title --download-cover
```

`-t` 支持 `post`、`favorite`、`music`、`hashtag`、`search`、`following`、`follower`、`collection`、`mix` 和 `aweme`。网页命令默认读取已保存 Cookie，也可以用 `--cookie` 或 `DOUYIN_COOKIE` 为单次运行传入。

### 抓取评论

```bash
douyin comment "https://www.douyin.com/video/作品ID" --limit 100

douyin comment "https://www.douyin.com/video/作品ID" \
  --with-replies \
  --reply-limit 50 \
  --format chatml-jsonl \
  --output comments.jsonl
```

输出格式包括 `raw`、`chatml-jsonl` 和 `chatml-json`。

### 调用官方 OpenAPI

完成 OAuth 后，命令会自动读取已保存的 `access_token` 和 `open_id`：

```bash
douyin api userinfo
douyin api comment-list --item-id "$DOUYIN_ITEM_ID"
douyin api comment-replies \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID"
```

回复评论属于写操作，默认要求确认：

```bash
douyin api comment-reply \
  --item-id "$DOUYIN_ITEM_ID" \
  --comment-id "$DOUYIN_COMMENT_ID" \
  --content "谢谢反馈"
```

通用请求仅接受当前 OpenAPI 基地址的同源路径：

```bash
douyin api request GET /oauth/userinfo/ \
  --param open_id="$DOUYIN_OPEN_ID"
```

## MCP 服务器

`douyin mcp` 通过 stdio 提供官方 OpenAPI 工具，并复用已保存的 OAuth 授权。

```bash
douyin mcp
```

MCP 客户端配置：

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

命令行配置示例：

```bash
claude mcp add douyin -- douyin mcp
codex mcp add douyin -- douyin mcp
```

## 本地字幕

输入支持本地视频或音频，输出支持 SRT、VTT、TXT 和 JSON：

```bash
douyin subtitle video.mp4 --language zh
douyin subtitle voice.mp3 --format txt
douyin subtitle *.mp4 --output subtitles/ --format vtt
```

首次使用模型别名时会下载 whisper.cpp GGML 模型。也可以只使用本地模型：

```bash
douyin subtitle video.mp4 \
  --model ./ggml-small.bin \
  --local-files-only
```

Linux/Windows 可选择 CUDA 构建：

```bash
cargo install douyin-cli --locked --features cuda --force
douyin subtitle video.mp4 --device cuda --language zh
```

## 配置和环境变量

默认配置文件位置：

- Linux/macOS：`~/.config/douyin-cli/config/settings.json`
- Windows：`%APPDATA%\douyin-cli\config\settings.json`
- 测试隔离：设置 `DOUYIN_HOME`

常用环境变量：

- `DOUYIN_COOKIE`
- `DOUYIN_CLIENT_KEY`
- `DOUYIN_CLIENT_SECRET`
- `DOUYIN_ACCESS_TOKEN`
- `DOUYIN_HOME`

完整参数和进阶示例见 [USAGE.md](USAGE.md)。

## Agent Skill

安装本仓库配套 skill：

```bash
bunx skills add LIghtJUNction/douyin -g
# 或
npx skills add LIghtJUNction/douyin -g
```

## 开发验证

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
cargo package --locked
```
