# douyin-cli

用 Rust 编写的抖音命令行工具。一个 CLI 即可完成公开内容采集、评论导出、媒体下载、账号互动、文本私信、OpenAPI 调用、MCP 接入和离线数据分析，并支持直播房间管理、主播状态上报与 OBS 推流。

[![crates.io](https://img.shields.io/crates/v/douyin-cli.svg)](https://crates.io/crates/douyin-cli)
[![docs.rs](https://docs.rs/douyin-cli/badge.svg)](https://docs.rs/douyin-cli)
[![Rust 1.100+](https://img.shields.io/badge/rust-1.100%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue.svg)](LICENSE)

- 项目地址：https://github.com/LIghtJUNction/douyin
- 安装包：https://crates.io/crates/douyin-cli
- 源码版本：`2026.11.18`
- 开发记录：[docs/rebuild.md](docs/rebuild.md)

> 本项目并非抖音官方产品，与抖音及字节跳动无任何隶属关系。请仅操作自己的账号和已获授权的数据，并遵守相关法律、平台规则和内容版权要求。接口与风控策略可能随时变化，命令存在并不代表所有线上场景都已验收，具体边界见下文。

## 项目特点

- **单一 Rust 可执行文件**：日常运行不依赖 Python、Node.js 或 Java。
- **三种独立认证方式**：Android 原生登录态、网页 Cookie 和官方 OAuth，各自保存，互不混用。
- **多种采集后端**：网页接口、需显式启用的 DuckDroid2 签名后端，以及仅用于协议研究的手机通道。
- **不止于只读采集**：支持作品点赞、收藏、分享链接、顶级评论、楼中楼回复和评论赞踩。
- **原生文本私信**：对接 Aweme 辅助接口与 ByteDance IM protobuf 协议，而不是用 REST JSON 请求冒充私信发送。
- **便于自动化和接入 AI 工具**：提供官方 OpenAPI 客户端、stdio/HTTP MCP 服务以及 JSON/JSONL 导出。
- **下载与分析闭环**：支持下载视频、图文、封面和标题，并可离线统计作品与评论数据。
- **最小化敏感信息暴露**：登录文件隔离保存，写操作须确认，CDN 下载不转发登录凭据。

## 功能与当前状态

| 模块 | 主要能力 | 当前情况 |
| --- | --- | --- |
| Android 登录 | ADB 导入原生 Token、账号和设备标识核对 | 支持已核对的 40.8.0 和 41.1.0 构建；基础请求可脱离手机运行 |
| 公开采集 | 搜索、作品、用户作品、评论、Feed、音乐、话题、合集 | 网页和 DuckDroid2 的主要只读流程已有真实验收 |
| 评论导出 | 主评论、楼中楼、raw、ChatML JSON/JSONL | 主评论已实测跨页；网页楼中楼新路由仍可能空响应或超时 |
| 社交互动 | 作品赞/藏、评论/回复、评论赞/踩、分享链接 | 上述写接口已实测成功；关注尚未完成真实验收 |
| 原生私信 | 会话创建、票据、消息拉取、文本发送 | 文本发送已实际跑通；其他 IM 流程仍需逐项验收 |
| 媒体下载 | 视频、图集、封面、标题、直接 CDN URL | 真实 MP4 下载已验收，不向 CDN 发送登录凭据 |
| OpenAPI | OAuth、资料、评论、官方私信、同源请求 | 本地协议与安全测试通过；依赖真实开放平台应用和权限 |
| 投稿 | 流式分片上传、预检、仅上传与进度恢复 | 本地 HTTP/CLI 回归通过；真实 OpenAPI 投稿需应用权限及 OAuth |
| 原生创作者投稿 | Rust HTTP 上传、签名、一次提交与本人回查 | 本地回归通过；新登录及真实账号上传/投稿待验收 |
| MCP | stdio、回环 Streamable HTTP、事件 Webhook | 真实进程端到端测试通过，含原生只读工具 |
| 直播 | 权限、预览、创建、续播、私密会话、心跳和结束状态 | 41.1.0 权限/预览查询通过；预检返回 10001，真实开播尚未验收 |
| OBS | 推流配置、媒体源、状态和停止 | 已有本地联动验收；本轮 OBS 未连接，真实抖音推拉流尚未验收 |
| 离线分析 | 热词、热梗候选、需求线索、作品统计 | 可完全离线运行，不需要登录 |

完整的成功记录、失败记录和研究边界以 [重建验收记录](docs/rebuild.md) 为准。

## 安装

### 从 crates.io 安装

需要 Rust **1.100 或更高版本**：

```bash
cargo install douyin-cli --locked
douyin --version
douyin --help
```

升级已安装版本：

```bash
cargo install douyin-cli --locked --force
```

### 从源码安装

```bash
git clone https://github.com/LIghtJUNction/douyin.git
cd douyin
cargo install --path . --locked
```

源码构建需要 C 编译器和 libclang；首次构建 Frida 依赖时会下载对应开发包。Windows 构建通过 vcpkg 使用动态 SQLite，避免与 Frida 静态库中的 SQLite 重复链接：

```powershell
$vcpkgExe = (Get-Command vcpkg -CommandType Application -ErrorAction Stop).Source
$env:VCPKG_ROOT = Split-Path -Parent $vcpkgExe
& $vcpkgExe install sqlite3:x64-windows "--vcpkg-root=$env:VCPKG_ROOT"
if ($LASTEXITCODE -ne 0) { throw "vcpkg install failed" }
$env:VCPKGRS_DYNAMIC = "1"
$env:SQLITE3_LIB_DIR = "$env:VCPKG_ROOT\installed\x64-windows\lib"
$env:SQLITE3_INCLUDE_DIR = "$env:VCPKG_ROOT\installed\x64-windows\include"
$env:PATH = "$env:VCPKG_ROOT\installed\x64-windows\bin;$env:PATH"
```

普通 CLI 使用不需要 Android SDK；只有重新编译随包附带的 Android 元数据辅助程序时才需要 JDK 和 Android SDK。

### 按使用场景准备环境

- 只分析已有 JSON/JSONL：安装后直接使用，不需要手机或登录。
- 网页公开采集：通常使用网页 Cookie；部分公开接口可以建立匿名访客状态。
- Android 登录导入：需要 ADB、已登录抖音的 rooted Android 手机；原生 Token 导入支持已核对的抖音 40.8.0、41.1.0 构建。
- 发布评论或回复：除导入登录态外，还需要显式 `--adb-guard` 和已连接 rooted 手机。
- DuckDroid2 后端：需要另行准备 DuckDroid2、匹配 ROM 和本地签名服务。
- OBS 联动：需要 OBS 28+ 并启用 OBS WebSocket。

## 快速上手

### 1. 选择登录方式

#### 方式 A：从 Android 手机导入原生登录态

手机先登录抖音并开启 USB 调试：

```bash
adb devices
douyin device
douyin auth --adb
```

连接多台设备时明确指定序列号：

```bash
douyin auth --adb --serial "设备序列号"
```

检查状态：

```bash
douyin auth status           # 联网确认账号
douyin auth status --offline # 只检查本地文件，不代表服务端确认
```

导入过程会核对账号、设备 ID、安装 ID、API 版本、APK 版本和更新版本；标识在导入过程中变化会拒绝保存。支持的版本优先使用原生 Token，不要求 WebView Cookie 才能确认账号。导入后，`auth status`、`me`、关注列表和直播配置等基础请求默认直接在电脑上运行，不会因为手机仍连接着就自动改走手机。

删除本机保存的 Android 登录态：

```bash
douyin auth logout
```

这不会退出手机上的抖音账号。

#### 方式 B：使用网页 Cookie

推荐从本地私密文件读取，避免 Cookie 出现在 shell 历史中：

```bash
douyin auth cookie-login --cookie-file ./private-cookie.txt
douyin auth cookie-status --offline
douyin auth cookie-status
```

也支持 `--cookie` 和 `DOUYIN_COOKIE`。保存及 `--offline` 检查只验证格式；只有在线检查返回明确登录证据，才会确认网页登录态。

```bash
douyin auth cookie-logout
```

#### 方式 C：使用官方开放平台 OAuth

先在抖音开放平台创建应用、申请权限并配置回调地址，然后通过环境变量提供应用凭据：

```bash
export DOUYIN_CLIENT_KEY="你的 client key"
export DOUYIN_CLIENT_SECRET="你的 client secret"

douyin auth login --scope user_info --listen --callback-port 8787
douyin auth status --oauth
douyin auth refresh
douyin api userinfo
```

也可以用手动回调 code：

```bash
douyin auth code --code "授权回调中的 code"
```

原生登录、网页 Cookie 和官方 OAuth 分别保存；OpenAPI 使用 OAuth，不使用 ADB 导入的 Token 代替。 `auth status --oauth` 只有在官方响应返回非空且匹配的账号 ID 后才确认连接；`--offline` 只展示本地授权记录。

### 2. 采集公开内容

采集入口沿用 `-u`、`-t`、`-l`、`-p`：

```bash
# 搜索 5 条作品，只保存元数据
douyin -u "关键词" -t search -l 5 --no-download -p ./downloads

# 下载单个作品
douyin -u "https://www.douyin.com/video/作品ID" -t aweme -l 1 -p ./downloads

# 采集用户作品
douyin -u "https://www.douyin.com/user/用户ID" -t post -l 20 -p ./downloads

# 音乐、话题和合集
douyin -u "音乐 ID 或链接" -t music -l 20 -p ./downloads
douyin -u "话题 ID 或链接" -t hashtag -l 20 -p ./downloads
douyin -u "合集 ID 或链接" -t mix -l 20 -p ./downloads

# 推荐 Feed 必须使用有限数量
DOUYIN_NATIVE_TRANSPORT=web douyin -t feed -l 20 -p ./downloads
```

支持的 `-t` 类型：

| 类型 | 用途 |
| --- | --- |
| `post` | 用户作品 |
| `favorite` | 用户喜欢的作品，需要合适权限 |
| `music` | 音乐下的作品 |
| `hashtag` | 话题作品 |
| `search` | 搜索作品 |
| `following` | 当前账号关注列表 |
| `follower` | 粉丝列表，仍受接口验收状态限制 |
| `collection` | 收藏列表，需要有效登录态 |
| `mix` | 合集作品 |
| `aweme` | 单作品详情 |
| `feed` | 推荐 Feed，必须设置有限 `-l` |

默认保存 JSON 元数据；不加 `--no-download` 时按作品内容下载媒体。可选：

```bash
--download-cover  # 下载封面
--download-title  # 导出标题
```

用户作品采用增量归档：本次采集更新相同作品并保留历史作品。`-l` 限制的是**本次请求数量**，不是合并后的归档总数。匿名网页话题分页目前可能反复返回零游标，此时工具会明确报错，而不会生成看似完整的虚假结果。

### 3. 选择采集传输

| 传输 | 设置 | 用途与限制 |
| --- | --- | --- |
| `direct` | 默认 | 直接使用导入的 Android 会话；基础账号流程已验证，不代表完整签名已复现 |
| `web` | `DOUYIN_NATIVE_TRANSPORT=web` | 使用网页 Cookie、主站签名与 WebSign；适合主要公开采集 |
| `duckemu` | `DOUYIN_NATIVE_TRANSPORT=duckemu` | 显式使用本机 DuckDroid2 服务；主要公开原生只读接口已验收 |
| `phone` | `DOUYIN_NATIVE_TRANSPORT=phone` | 已接入固定直播 GET/POST；按已核对构建匹配，本轮设备仍有 Frida 附加故障 |

网页模式示例：

```bash
DOUYIN_NATIVE_TRANSPORT=web douyin -u "关键词" -t search -l 5 --no-download
DOUYIN_NATIVE_TRANSPORT=web douyin comment "作品链接" --limit 100 -o comments.json
```

`DOUYIN_WEB_HOST` 只允许 `www.douyin.com` 或 `aweme.snssdk.com`。

DuckDroid2 模式需要另外安装外部服务，项目不会静默启动它或回退到手机：

```bash
tools/prepare-duckemu.sh
DUCKEMU_ROOT=/path/to/DuckDroid2 \
DUCKEMU_ROM_SYMBOLS=/path/to/ROM/symbols \
  tools/start-duckemu.sh

DOUYIN_NATIVE_TRANSPORT=duckemu douyin -u "关键词" -t search -l 3 --no-download
```

默认 Unix socket 是 `/tmp/duckemu-dy7/dy7.sock`，可用 `DOUYIN_DUCKEMU_SOCKET` 修改。该后端证明的是显式外部原生仿真链路，不等于纯 Rust 已独立完成全部签名。

## 评论与 ChatML 导出

```bash
# 导出原始评论 JSON
douyin comment "作品 ID 或分享链接" \
  --limit 100 --format raw -o comments.json

# 同时采集楼中楼，导出逐行 ChatML
douyin comment "作品 ID 或分享链接" \
  --limit 100 --with-replies --reply-limit 20 \
  --format chatml-jsonl -o comments.jsonl

# 导出 ChatML JSON 数组
douyin comment "作品 ID 或分享链接" \
  --with-replies --format chatml-json -o comments.chatml.json
```

可调整分页、间隔、角色和筛选条件：

```bash
douyin comment "作品 ID 或分享链接" \
  --count 20 --sleep 1.0 \
  --comment-role user --reply-role assistant \
  --min-comment-digg 10 --min-reply-digg 2 \
  --include-single-comments \
  --format chatml-jsonl -o comments.jsonl
```

主评论与回复分别分页并按评论 ID 去重。`--include-single-comments` 保留没有回复的评论。主评论已实测跨 4 页保存 25 条；网页回复新路由仍可能遇到空响应或超时，失败不会被当作空数据成功导出。

## 社交互动

以下命令使用 Android 原生登录态。**所有写操作必须显式加 `--yes`**，请先确认目标和内容。

```bash
# 作品点赞 / 取消点赞
douyin social digg "作品ID" --action 1 --yes
douyin social digg "作品ID" --action 0 --yes

# 收藏 / 取消收藏
douyin social collect "作品ID" --action 1 --yes
douyin social collect "作品ID" --action 0 --yes

# 发布顶级评论
douyin social comment "作品ID" \
  --text "评论内容" --adb-guard --yes

# 回复指定评论
douyin social comment "作品ID" \
  --text "回复内容" --reply-id "评论ID" --adb-guard --yes

# 评论点赞 / 取消点赞
douyin social comment-digg "评论ID" "作品ID" --action 1 --yes
douyin social comment-digg "评论ID" "作品ID" --action 0 --yes

# 评论点踩 / 取消点踩
douyin social comment-dislike "评论ID" "作品ID" --action 1 --yes
douyin social comment-dislike "评论ID" "作品ID" --action 0 --yes

# 获取服务端分享链接
douyin social share "作品ID"
```

评论赞踩的用户动作统一使用 `--action 1/0`，内部映射到 APK 的实际类型：点赞 `1`、取消点赞 `2`、点踩 `4`、取消点踩 `3`。

发布顶级评论和回复时必须显式加上 `--adb-guard`，由已连接的 rooted 手机生成当前请求所需的 TicketGuard 材料。工具不会静默依赖手机，也不会把签名材料写入仓库。`share` 只返回链接，不会自动发送给任何联系人。

作品赞/藏、评论/回复、评论赞/踩和分享链接均已取得真实成功响应。关注命令虽已接线，但尚未完成真实验收：

```bash
douyin social follow --sec-user-id "SEC_UID" --action 1 --yes
douyin social follow --sec-user-id "SEC_UID" --action 0 --yes
```

## 原生文本私信

原生 IM 由两部分组成：Aweme REST 辅助接口，以及 `imapi.snssdk.com` 的 protobuf 消息协议。它不同于官方 OpenAPI 的私信接口。

```bash
# 查询 IM 配置、陌生人权限和能否向目标用户发私信
douyin im config
douyin im stranger-priv
douyin im can-send --sec-user-id "SEC_UID"

# 创建会话；返回的会话 ID 和短 ID 用于后续命令
douyin im create-conversation --user-id "用户数字ID" --yes

# 查询会话、进入会话
douyin im conversation-info \
  --conversation-id "会话ID" --conversation-short-id "会话短ID" --yes
douyin im enter-chat \
  --conversation-id "会话ID" --conversation-short-id "会话短ID"

# 获取发送票据
douyin im ticket --to-user-id "用户数字ID" --yes

# 发送文本私信
douyin im send \
  --conversation-id "会话ID" \
  --conversation-short-id "会话短ID" \
  --ticket "会话 ticket" \
  --text "消息内容" --yes

# 拉取消息增量
douyin im pull --cursor 0 --limit 20 --yes
```

文本私信发送已实际跑通。`ticket`、会话 ID 等仍属于敏感会话数据，不要提交到仓库或贴到公开 Issue。接口返回 HTTP 成功并不必然代表业务发送成功，客户端还会检查 protobuf 业务响应。

## 媒体下载

下载单个作品：

```bash
douyin -u "作品 ID 或分享链接" -t aweme -l 1 -p ./downloads
```

直接下载已有 CDN 地址：

```bash
douyin download "https://抖音CDN域名/媒体路径" -o ./media.mp4
```

下载器会校验 HTTPS CDN 主机和每次重定向，按顺序尝试最多 8 个去重后的候选地址。文件完整读取并通过状态、类型和长度检查后才提交；已有非空普通文件直接复用，零字节文件会在下载成功后原子替换，目录和特殊文件会报错。中断或全部候选失败时保留原有文件。

登录 Token、Cookie 和 OAuth 凭据不会转发给 CDN。最终错误只显示尝试次数与原因，不回显带签名的媒体 URL。

## 官方 OpenAPI

```bash
# 应用级 token 与授权信息
douyin api client-token
douyin api authorize-url
douyin api access-token --code "授权 code"
douyin api refresh-token
douyin api renew-refresh-token

# 用户资料与评论
douyin api userinfo
douyin api comment-list --item-id "作品ID"
douyin api comment-replies --item-id "作品ID" --comment-id "评论ID"
douyin api comment-reply --item-id "作品ID" --content "回复内容" --yes

# 官方文字、图片或视频私信
douyin api im-message-send --help

# 同源通用请求；仅允许官方 OpenAPI 路径
douyin api request GET /oauth/userinfo/ --param open_id="OPEN_ID"
```

通用请求拒绝跨域地址、重定向和敏感请求头覆盖。写操作需要 `--yes` 或交互确认。真实可用范围取决于开放平台应用的审核、scope、账号权限和接口配额。

## 投稿

原生创作者投稿入口为 `douyin publish app`（别名 `publish native`），由 Rust 直接上传、创建并回查，保留账号与视频摘要的防重复记录。真实账号链路尚未验收。当前原生扫码流程只能保存待完成身份材料，尚无激活完整投稿会话的实现；可先运行 `auth creator-login --capabilities` 离线查看边界。已有完整同次创作者会话材料时可使用 `auth creator-import`；仅有普通 Cookie 不足以投稿。用法和准确边界见 [原生创作者投稿](docs/native-publishing.md)。

以下为独立的官方 OAuth 投稿流程。

投稿使用官方 OAuth 内容发布能力，需要用户实际授予 `video.create.bind`。支持自动流式分片、独立上传超时、本地预检和投稿记录恢复：

```bash
douyin auth login --scope user_info,video.create.bind

# 本地检查文件和权限，不上传、不创建作品
douyin publish video ./video.mp4 --title "标题 #话题" --dry-run

# 上传并创建作品，自动保存进度
douyin publish video ./video.mp4 --title "标题 #话题" --yes

# 先上传，再复用已上传的视频创建作品
douyin publish video ./video.mp4 --upload-only --receipt ./video-receipt.json --yes
douyin publish status ./video-receipt.json
douyin publish resume ./video-receipt.json --yes
```

超过 50 MiB 自动分片，每个上传请求默认超时 300 秒，可用 `--upload-timeout` 修改。支持 `--visibility private|friends|public`，以及互斥的 `--cover-tsp 2.3` 和官方 OAuth `--cover ./cover.jpg`。`--ai-generated` 和 `--short-title` 明确表达用户要求，但当前缺少已验证协议，会在上传前拒绝；不会静默忽略或把文案当作平台标识。创建结果未知时先检查创作者中心，再决定是否显式重试；成功创建后进入平台审核。完整命令、恢复阶段和验证范围见 [视频投稿](docs/publishing.md)。

## MCP：接入 AI 与自动化工具

启动 stdio MCP：

```bash
douyin mcp
```

启动本机 Streamable HTTP MCP：

```bash
douyin mcp --listen 127.0.0.1:8788
```

端点：

- `POST /mcp`：MCP Streamable HTTP。
- `POST /events`：接收 JSON 事件，可同步转发到 Webhook。

事件转发示例：

```bash
douyin mcp \
  --listen 127.0.0.1:8788 \
  --webhook https://example.com/events \
  --event-token "入口令牌" \
  --webhook-token "转发令牌"
```

HTTP 监听仅允许回环地址；远程 Webhook 必须使用 HTTPS，回环 HTTP 可用于本地开发。若要对公网提供 MCP，请自行在前面部署带认证的反向代理，不要直接暴露无认证服务。

主要工具包括：

- 离线：`hot_words`、`hot_memes`、`demand_discovery`。
- 原生只读：`native_search`、`native_aweme`、`native_comments`、`native_comment_replies`、`native_user_posts`、`native_me`、`native_live_permissions`、`native_live_create_info`。
- 原生写操作：`native_digg`、`native_collect`。
- OpenAPI：`userinfo`、`comment_list`、`comment_replies`、`comment_reply`、`im_message_send`、`openapi_request`。

MCP 写操作需要参数 `confirmed=true`，不会仅凭 schema 描述就执行。

## 直播与 OBS

直播命令支持创建房间、保存推流会话、续播、主播状态上报和有限次心跳。完整操作、结果字段与实机验收范围见 [直播使用指南](docs/live.md)。

先导入当前手机账号并检查开播配置：

```bash
douyin auth --adb
douyin live permissions
douyin live create-info
douyin live create-check
```

将当前直播配置写入 `create-room.json`；最小输入包含 `title`，其余字段取决于账号权限与直播类型。创建成功后，私密会话文件可以直接交给 OBS：

```bash
douyin live create --params-file ./create-room.json --output ./live-session.json --yes

douyin live obs configure-push --stream-file ./live-session.json --start --yes
douyin live obs status

# 间隔与次数由使用者明确指定；此例每次响应后等 5 秒，共发送 120 次。
douyin live heartbeat --session-file ./live-session.json --interval 5 --count 120 --yes

# 结束时先停止 OBS 推流，再上报主播结束状态并读取房间状态。
douyin live obs stop --yes
douyin live finish --session-file ./live-session.json --yes
```

OBS WebSocket 默认连接本机 `4455`，密码通过 `DOUYIN_OBS_PASSWORD` 提供。`configure-push` 默认仅配置，`--start` 明确启动推流。会话包含签名推流地址，使用私密新文件保存，Unix 权限为 `0600`；终端只输出房间/流标识和保存结果。已有文件会被拒绝覆盖，创建请求成功后的保存失败会给出恢复信息。

续播和单次状态上报：

```bash
douyin live continue --resolution-key "当前分辨率配置键" --output ./continued-session.json --yes
douyin live anchor-status --session-file ./live-session.json --status pause --yes
douyin live info "房间ID"
douyin live leave --room-id "房间ID" --yes
```

`anchor-status` 支持 `prepare`、`play`、`pause`、`finish`。`leave` 是观众离开操作。`finish` 分别输出状态请求是否被接受及房间状态的读回结果，读回失败返回非零退出码。`heartbeat` 遇到失败停止，Ctrl-C 会打断等待并阻止后续心跳。

使用手机内的已核对网络接口时显式设置 `DOUYIN_NATIVE_TRANSPORT=phone`；默认 `direct` 在电脑上发送请求。手机通道按 40.8.0、41.1.0 的具体构建匹配 GET/POST 入口，官方主机与路径名单之外的请求会被拒绝。

已有拉流地址也可加入 OBS 场景：

```bash
douyin live obs add-source --url-file ./private-pull-url.txt --scene "场景名" --yes
douyin live obs source-status --name "douyin-live"
```

## 离线数据分析

无需登录或联网，直接处理已有数据：

```bash
# 评论热词、重复短句和需求线索
douyin insights comments.json --top 20 --min-count 2
douyin insights comments.jsonl --format markdown -o insights.md

# 作品统计、排序和筛选
douyin stats works.json --sort score --top 10
douyin stats works.jsonl --author "作者名" --format markdown
```

`insights` 支持文本、JSON、JSONL 和 ChatML，输出热词、热梗候选及包含需求关键词的原句。`stats` 支持旧版扁平导出和原始嵌套数据，统计点赞、评论、收藏、分享、作者和时长等信息。

这些是确定性规则分析，不是 AI 内容理解；综合分也不等于互动率。缺失指标会标为缺失，不会伪装成真实的零。

## 配置与凭据安全

默认配置目录：

- Linux / macOS：`~/.config/douyin-cli/`；设置 `XDG_CONFIG_HOME` 时遵循该目录。
- Windows：`%APPDATA%\douyin-cli\`。
- 可用 `DOUYIN_HOME` 指定独立目录，测试时尤其推荐。

主要文件：

| 文件 | 内容 |
| --- | --- |
| `session.json` | Android 原生登录态 |
| `cookie.json` | 网页 Cookie 与访客状态 |
| `oauth.json` | 官方 OAuth 授权 |

隔离测试示例：

```bash
DOUYIN_HOME=/tmp/douyin-test douyin auth status --offline
```

这些文件以及 TicketGuard 材料、IM ticket、推拉流地址和密钥都属于敏感信息。不要把它们写入源码、测试夹具、日志、Issue 或公开帖子。媒体下载与 OpenAPI 请求均有独立凭据作用域，不会任意转发认证头。

## 开发与验证

```bash
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --release --locked --offline
cargo package --locked --offline
cargo run --locked --offline -- --help
```

直播生命周期变更已通过 Rust 测试、严格 Clippy 和手机桥接夹具测试。具体测试数量与设备验收结果记录在 [直播使用指南](docs/live.md)。

修改手机通道脚本时还需运行：

```bash
node tools/test-phone-agent.mjs
```

Node.js 只用于这项开发测试，不是 CLI 运行依赖。

修改 Android 元数据辅助程序后，需要 JDK、Android platform 36 和 build-tools 35.0.0，并检查构建可重现性：

```bash
bash tools/build-device-helper.sh
bash tools/build-device-helper.sh --check
```

## 目前尚未完成

项目仍在持续开发，以下边界需要明确：

- 默认 direct 后端尚未复现完整 Android 网络签名；基础 Token 认证成功不代表所有接口都能独立运行。
- 关注、粉丝、部分收藏和喜欢列表等接口仍需有效账号与进一步真实验收。
- 评论写入已成功，但后续评论列表回读可能受空响应影响，不能据此承诺所有读取链路稳定。
- 原生 IM 目前重点是文本发送，不代表已覆盖全部消息类型、实时推送和所有会话场景。
- 真实 Android 身份开播、完整房间生命周期和 OBS 双向真实媒体链路尚未闭环。
- 风控、验证码、接口变化和账号权限仍可能导致请求失败；工具会尽量报错，而不是把风控空响应当作成功。

详细进度见 [docs/rebuild.md](docs/rebuild.md)，实际参数始终以 CLI 帮助为准：

```bash
douyin --help
douyin auth --help
douyin social --help
douyin im --help
douyin live --help
```

## 参与项目

欢迎提交可复现的问题报告、协议证据、测试和文档改进。报告线上问题时请提供命令、版本和脱敏错误信息，不要附带 Cookie、Token、签名材料、私信 ticket 或完整私密响应。

## License

[AGPL-3.0-or-later](LICENSE)
