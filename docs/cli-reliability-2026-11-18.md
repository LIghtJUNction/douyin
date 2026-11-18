# douyin CLI 可靠性迭代

日期：2026-11-18（Asia/Taipei）

## 交付状态

本轮在官方视频投稿实现上继续迭代，修复下载恢复、CDN 地址回退、OAuth 在线状态和投稿记录锁。源码已分阶段推送到 `iteration/cli-reliability-20261118`，本地共 215 项测试通过，0 失败，1 项原有手机依赖测试保持忽略。

[PR #7：修复下载恢复、CDN 回退与账号/投稿状态核实](https://github.com/LIghtJUNction/douyin/pull/7) 以 [PR #6 的投稿分支](https://github.com/LIghtJUNction/douyin/pull/6) 为基底，当前保持打开。包版本为 `2026.11.18`；本轮没有创建新的发布标签或 crates.io 版本。

开发机已通过 `cargo install --path . --locked --offline --force` 更新 `/home/lightjunction/.cargo/bin/douyin`。实际可执行文件返回 `douyin-cli 2026.11.18`；发布命令帮助、隔离的已提交记录查询和恢复检查均通过，输出 `review_status: not_queried` 且记录内容保持。

## 用户可见变化

| 问题 | 当前行为 | 验证依据 |
| --- | --- | --- |
| 下载目标残留零字节文件，重试仍失败 | 先下载到临时文件，完整读取、验证并同步后原子替换空文件；失败保留原文件 | 本地 HTTP fixture：空文件恢复、全部候选失败、临时文件清理 |
| 下载目标是目录，却显示已完成 | 明确拒绝目录和其他非普通文件；已有非空普通文件继续复用 | 目录拒绝、完整文件无网络复用测试 |
| 第一条 CDN 地址过期后整项下载失败 | 保留候选顺序，去空、去重，最多尝试 8 个地址；每个候选最多 5 次含重定向的 HTTP 请求 | 403 后回退、合法与非法重定向、候选和请求上限测试 |
| 下载中断或返回未请求的部分内容 | 截断内容、HTTP 206、错误类型和空内容都不会作为完整文件提交 | 截断与部分响应后的回退测试 |
| OAuth 响应没有账号 ID，却显示已连接 | 官方响应必须提供非空字符串 open_id，并与本地保存的账号一致，才返回 connected=true | 真实本地 HTTP 请求测试：缺失、空值、错误类型、账号不一致、业务错误与匹配账号 |
| 本地投稿记录一直显示审核中 | 已提交记录返回 review_status=not_queried，表示未查询平台审核结果 | CLI status/resume 及创建响应测试 |
| 投稿操作结束后记录锁延迟释放 | Journal 结束时主动解锁；复制或临时继承的描述符不再拖延释放 | 保留复制描述符的操作系统锁回归，以及文件别名互斥测试 |

下载器继续使用独立客户端。每个候选与每次重定向都接受 HTTPS/CDN 校验；HTTP fixture 检查请求没有 Cookie、Authorization、access-token 或 x-tt-token。错误摘要只包含尝试次数和原因，不回显带签名的 URL 或响应正文。

## 投稿命令与本机授权

当前 CLI 已提供上传并投稿、流式分片、独立上传超时、本地预检、仅上传和记录恢复。当前开发机的官方 OAuth 离线检查结果为：

```json
{
  "authorized": false,
  "scopesVerified": false,
  "has_publish_scope": false
}
```

实际 OpenAPI 投稿需要应用具备发布能力，并由用户授予 `video.create.bind`。Android 登录态、网页 Cookie 和官方 OAuth 各自独立。应用凭据和回调地址准备好后，可使用：

```bash
export DOUYIN_CLIENT_KEY="你的应用 Client Key"
export DOUYIN_CLIENT_SECRET="你的应用 Client Secret"

douyin auth login --scope user_info,video.create.bind --listen
douyin auth status --oauth

douyin publish video ./video.mp4 --title "标题 #话题" --dry-run
douyin publish video ./video.mp4 --title "标题 #话题" --yes
```

分步投稿及恢复：

```bash
douyin publish video ./video.mp4 --title "标题" \
  --upload-only --receipt ./video-receipt.json --yes

douyin publish status ./video-receipt.json
douyin publish resume ./video-receipt.json --yes
```

`stage: submitted` 表示创建接口已经返回作品 ID。`review_status: not_queried` 表示没有查询服务端审核结果，实际审核及可见状态请在创作者中心核对。已有 submitted 记录恢复时直接读取记录，不会再次创建作品。creating、unknown 记录仍要求先核实账号中的作品，再显式决定是否重试。

详细上传限制、分片行为和恢复阶段见 [投稿指南](https://github.com/LIghtJUNction/douyin/blob/iteration/cli-reliability-20261118/docs/publishing.md)。

## 本地验证

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked --offline` | 215 passed，0 failed，1 ignored |
| `cargo clippy --locked --offline --all-targets -- -D warnings` | 通过 |
| `cargo fmt --all -- --check` | 通过 |
| `git diff --check` | 通过 |
| 独立代码复审 | 下载、状态和记录锁修复通过 |

本轮新增 10 项测试：下载 7 项、OAuth 2 项、锁释放 1 项。原有 CLI 投稿测试同步检查新的审核状态语义。唯一忽略项是项目原有的 rooted 手机 TicketGuard 测试，涉及真实设备写入。

HTTP 回归使用本地服务器和隔离目录，覆盖实际请求、响应和文件提交行为。本轮没有向真实抖音账号投稿；当前账号的 OpenAPI 投稿验收仍需要完成上述授权。本轮本地验证平台为 Linux；没有为日常源码修复重跑远程 CI 或自动审查。

## 代码提交

- [aa33eb9：核实 OAuth 账号并明确本地审核状态](https://github.com/LIghtJUNction/douyin/commit/aa33eb9c831543bdd7cb9f310d57e5d947176faf)
- [19e83d5：恢复空文件并支持有界 CDN 回退](https://github.com/LIghtJUNction/douyin/commit/19e83d5405d96124057329fe63748e4221f6993a)
- [f456b59：主动释放投稿记录锁](https://github.com/LIghtJUNction/douyin/commit/f456b598af207512559a9712e3b029754d69df55)

开发机工作树：`/home/lightjunction/Documents/GITHUB/douyin/.worktrees/cli-reliability-20261118`。

交付目录：[douyin-cli-投稿迭代-2026-11-18](https://drive.google.com/drive/folders/1RJQudADJhrQsZsQ90qM2-XET6Q2b6Opa)。
