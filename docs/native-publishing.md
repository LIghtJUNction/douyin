# 原生创作者投稿

`douyin publish app` 和别名 `douyin publish native` 使用 Rust HTTP 客户端完成上传、创建作品和本人作品回查。视频检查调用本机 `ffprobe`；投稿过程不启动浏览器、ADB、手机 App、Python 或 Node。原有 `publish video`、`publish resume` 的官方 OAuth 流程保持独立。

## 当前验收状态

截至 2026-11-19，原生上传、请求签名、创建请求和防重复事务已实现，本地 HTTP fixture 与 CLI 回归通过。**真实账号上传、创建作品和审核回查尚未验收。**

开发机上保存的旧 Cookie 可以被解析，但使用实际编译后的原生命令读取创作者本人身份，服务端返回状态码 `8`。一秒合成视频的 `--dry-run` 因此在 `account_check` 阶段退出，未上传或投稿。离线 `creator-status` 同时报告未配置完整创作者凭据。状态码本身不能区分过期、登录上下文不足或平台校验，需要先完成正常创作者登录。

2026-11-19 的原生新登录实测中，按当时官方登录 UI 2.4.22 / SDK 3.4.20 补齐应用签名及请求参数后，取二维码成功；仅执行一次状态轮询，返回 HTTP 200、`error_code: 4031`。命令以非零状态退出并保存 `server_blocked` 终态，随后停止线上请求。未取得服务器票据与本次客户端公钥的绑定，未完成本人身份核验，`active` 和 `publishReady` 均为 `false`。该响应不足以确定拒绝原因。

本版本提供原生扫码身份验证、严格的会话包导入与签名。**扫码流程尚未实现从扫码结果收集完整 Creator 发布材料并激活投稿会话**；这既是实现缺口，也没有真实发布验收。即使通过两个本人账号接口，仍保存 `native-session.pending.json`，`publishReady` 和 `active` 为 `false`。重复扫码不能补齐当前流程没有采集的 `dtrait`、平台参数等材料。普通 Cookie 登录同样不等于投稿凭据完整。

## 命令

先离线查看真实能力，再检查本地配置：

```bash
douyin auth creator-login --capabilities
douyin auth creator-status
```

`--capabilities` 不读凭据、不生成二维码、不访问网络，明确报告 `qrActivationImplemented: false`。需要投稿时，有条件可用的 CLI 路线是导入已合法取得的完整同次 Creator 会话包，或使用已获审批和用户授权的官方 OAuth 流程；没有这些前提时可由账号所有者在正常 Creator 界面操作。当前 CLI 不会从浏览器已登录这一事实推断原生投稿材料完整。

仅需要验证原生扫码身份流程时，以下命令使用全新会话，不读取或覆盖当前活动账号：

```bash
# 生成二维码并轮询；验证码、平台拒绝、取消或过期都会停止
douyin auth creator-login

# 只生成二维码；保存本次私钥和 token，以便恢复同一次会话
douyin auth creator-login --qr-only
douyin auth creator-login --resume /path/to/pending-directory

# 只轮询一次；仍待扫码时保存会话，之后使用 --resume 继续
douyin auth creator-login --poll-once
```

默认等待 180 秒，`--timeout` 范围为 5–1800 秒；`--poll-interval` 范围为 5–60 秒，默认 5 秒。二维码和继续会话所需材料只保存在独立私有目录。服务器明确拒绝、取消、过期、密钥不匹配或响应无法解释后，不自动换码、不继续轮询，命令返回非零状态。`--qr-only` 正常返回的仍是未认证状态。 `--poll-once` 与 `--qr-only` 互斥；单次轮询后仍待确认时保留本次会话，不能把正常退出当作登录或投稿成功。

确认登录后还要核对服务器票据与本次客户端公钥，并通过两个 Creator 同源接口核对本人 UID。缺少完整投稿材料时，结果保持 `publishReady: false`、`active: false`，只保存待核验材料，不改动活动账号。当前不能把这一引导步骤当作已经打通的新账号投稿入口。

已经具有完整同次创作者会话包时，可从本机文件导入：

```bash
douyin auth creator-import /private/path/creator-session.json
```

导入只验证本地格式、私钥与 Cookie 公钥是否匹配，以及已有的签名绑定证据；`onlineVerified` 始终为 `false`。每次投稿仍向创作者服务重新核对本人 UID，不能仅凭导入包中的账号字段操作。

以下命令要求创作者本人身份和投稿凭据都通过检查：

```bash
# 检查视频、当前创作者身份、配套发布凭据；不上传、不创建投稿记录
douyin publish native ./video.mp4 --title "标题" --topic Rust --dry-run

# 仅上传到视频存储并生成预览；不会调用创建作品接口
douyin publish native ./video.mp4 --title "标题 #Rust" \
  --visibility private --prepare-only

# 明确提交；可额外绑定预期账号 UID
douyin publish app ./video.mp4 --title "标题 #Rust" \
  --visibility private --account YOUR_UID --yes

# 只读查看本地回执，不访问网络
douyin publish status /path/to/receipt.json
```

`app` 是兼容最初需求的命令名称，实际执行与 `native` 相同。该入口没有 `--backend`、`--serial` 或 `--browser-url` 参数，也不会根据 `ANDROID_SERIAL` 切换实现。

| 参数 | 行为 |
| --- | --- |
| `--title` / `-t` | 必填，完整文案原样发送；标题与追加话题合计最多 1000 个 Unicode 字符 |
| `--topic` | 可重复，追加字面 `#话题`，同一参数中的重复话题去重；不创建额外的富文本提及关系 |
| `--ai-generated` | 明确要求平台 AI 声明；当前缺少可核验字段，上传前拒绝，文案不替代标识 |
| `--cover` | 明确要求本地 JPG；当前原生上传协议不支持该已验证流程，上传前拒绝；官方 OAuth 路线支持 |
| `--short-title` | 明确要求独立短标题；非空协议未验证，上传前拒绝 |
| `--visibility` | `public`、`private` 或 `friends`，默认 `public` |
| `--account` | 可选预期 UID，与服务端当前身份不一致即停止 |
| `--timeout` | 每次 API/上传请求等待上限，默认 300 秒，范围 1–3600；别名 `--upload-timeout` |
| `--dry-run` | 只检查，不上传、不保存投稿记录；会访问账号接口 |
| `--prepare-only` | 上传并保存预览，停止在创建作品前；别名 `--upload-only` |
| `--yes` | 明确允许创建作品；普通投稿没有此参数会在读取文件、凭据和启动外部程序前退出 |

`--dry-run` 与 `--prepare-only` 互斥。两个预检模式不要求 `--yes`；其中 `--prepare-only` **确实会上传视频**。本地上限为 4 GiB，文件必须非空且可由 `ffprobe` 识别为有效视频。预检通过不保证服务端最终接受内容、账号权限或审核结果。

## 登录材料与保存方式

创作者 Cookie 表示登录会话；配套私钥用于证明某次请求属于同一客户端登录。只有公钥、签名摘要或 Cookie，不能反推出缺失的私钥。Android 登录态、普通网页 Cookie、官方 OAuth 与创作者投稿凭据分别处理。

会话包采用 `schema: 1`，`origin` 固定为 `https://creator.douyin.com`。字段分组如下；这些是导入格式说明，不是从旧 Cookie 自动生成新登录的步骤。

| 字段 | 来源与用途 |
| --- | --- |
| `account_id`、`cookie`、`user_agent` | 同一次正常创作者登录的账号、Cookie 和请求环境 |
| `ms_token`、`csrf_token` | 该登录环境中的真实查询令牌与 Creator 域 CSRF 令牌 |
| `ticket_guard` | `ticket`、`ts_sign`、`private_key`、`sign_mode`；按签名模式提供相应服务器证书 |
| `dtrait` | 正常登录设备产生的内部 `blob`、SDK RSA 公钥及版本信息 |
| `platform_params` | 同次登录的平台参数键值对 |

导入使用有长度限制的完整文件校验与原子替换；错误输入不会覆盖有效配置。Unix 上配置文件权限为 `0600`。输出、回执和错误信息均不包含 Cookie、票据、私钥、CSRF、动态签名或完整网络响应。清除该独立配置使用：

```bash
douyin auth creator-logout
```

## 上传与创建的边界

原生客户端先核对两个同源账号接口返回的 UID，随后检查完整签名材料。缺少材料时，第一次视频上传授权请求也不会发出。

| 步骤 | 实现与成功依据 |
| --- | --- |
| 文件检查 | 流式计算 SHA-256，创建私有临时快照，再检查实际将上传的快照 |
| 上传授权 | 通过创作者 `/web/api/media/upload/auth/v5/` 获取短期 STS |
| 视频上传 | 固定 VOD 网关执行 ApplyUploadInner，再向通过域名校验的存储节点发送字节 |
| 完成上传 | 重新获取 STS，执行 CommitUploadInner，要求真实视频 ID、封面和有效元信息 |
| 发布前核对 | 实际上传 SHA-256、字节数、账号、文案和可见范围必须与本次任务一致 |
| 创建作品 | 先落盘提交意图，再向 `/web/api/media/aweme/create_v2/` 发送唯一一次 POST |
| 本人回查 | 通过 `/janus/douyin/creator/pc/work_list` 按作品 ID 或已上传视频 ID 精确匹配 |

不超过 3 MiB 的文件直接上传。更大的文件以 5 MiB 分片，达到 500 MiB 后使用 10 MiB 分片；上传缓冲最大 10 MiB。分片检查 CRC32，上传流同时计算 MD5 和 SHA-256，完成上传时核对服务端提供的文件证据。账号 Cookie 不进入 VOD/存储客户端；STS 只进入固定 VOD 网关，存储授权仅发送给已验证的上传节点。客户端验证 TLS，拒绝重定向，创建请求禁用自动重试。

标题原样进入完整文案字段，短标题留空；封面采用服务端上传结果中的真实 `poster_uri`。这些字段在本地 fixture 中验证过，仍需要真实账号发布前验收。

`settings` 会明确列出 AI 声明、自定义封面和短标题是否请求、支持和编码；视频快照封面不表示用户给出的 JPG 已应用。不支持的选项会在账号访问、视频上传和创建作品之前退出，`--dry-run` 同时输出 `options_check` 报告。官方封面映射和证据见 [视频投稿](publishing.md#自定义封面和平台声明)。

## 重复执行与结果解释

默认记录位于配置目录的 `publish-local/`，键只由**账号 UID + 视频 SHA-256**决定。修改标题或后端名称不能绕过同一份记录；记录参数不一致时停止。记录包含内容摘要、文案、可见范围、视频 ID 和必要回执，不包含认证材料。

| 阶段 | 可以作出的结论 |
| --- | --- |
| `prepared` | 已有准备记录；可能完成了上传预览，未开始创建作品 |
| `submitting` | 创建意图已可靠落盘；请求是否被服务端接收尚需回查 |
| `unknown` | 本次创建结果或作品对应关系不明确，不能据此再次提交 |
| `submitted` | 作品 ID 或精确本人作品记录证明已提交；当前可见状态可能未确认 |
| `reviewing` | 本次回查存在明确审核中的证据 |
| `public` / `private` / `friends` | 本次回查同时满足审核与相应可见范围证据 |
| `rejected` | 本次回查得到拒绝或撤下等明确证据，命令返回非零退出状态 |

HTTP 成功或 `status_code: 0` 不足以证明作品公开。创建响应必须具有真实作品 ID 才标记已提交。回查只按 ID 匹配，不按标题相似或发布时间猜测。找不到、分页未完整覆盖、同一视频对应多个候选、验证码或登录异常都会保留不确定结果。

同一命令再次运行时，只要记录已进入 `submitting` 或之后阶段，就只回查，不上传、不再 POST。不要删除该记录以“重试”。当前没有绕过原生去重保护的重试开关。保留本地记录是这一保护的前提，它不是服务端的全局幂等承诺。

`prepared` 阶段再次运行会重新上传并生成新预览；它尚未发送创建请求，不会由此产生重复作品。当前未实现复用旧准备阶段的视频上传结果。原生记录也不由 OAuth 的 `publish resume` 继续执行。

写入创建回执失败时，程序仍尝试一次同账号只读回查，并再次保存结果；先前持久化的提交意图继续阻止重发。回查失败不会沿用历史的“当前公开”结论：仍可证明接受过的作品退回 `submitted`，上次状态和链接分别保留为 `last_known_stage`、`last_known_share_url`。

## 验证与证据

本地测试使用隔离配置、合成凭据和 loopback HTTP 服务，不访问真实账号的创建接口。覆盖签名字节、STS 刷新、分片内容、CRC、重定向隔离、账号变更、文件变更、进程退出、锁释放、写盘恢复、只回查不重发，以及原有 OAuth 兼容。

```bash
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo fmt --all -- --check
```

线上验收仍缺：正常新创作者登录/完整配套材料、真实视频上传预览、用户明确授权视频的创建与审核状态回查。本次没有把合成测试视频实际投稿。

协议字段依据当前官方 Creator 脚本与固定版本的公开实现核对后独立实现；不是平台公开稳定的 OpenAPI 合约。SDK 和私有接口变化时应重新核验，不能将本地 fixture 当成线上兼容性的证明。

- [官方 Creator 账号接口调用代码](https://lf-fe-creator.douyinstatic.com/obj/douyn-creator-scm-cdn/douyin-creator-master-new/static/js/main.a20c35af.js)，2026-11-19 核对。
- [官方内容管理状态与审核判断代码](https://lf-fe-creator.douyinstatic.com/obj/douyn-creator-scm-cdn/douyin-creator-content-new/micro/static/js/douyin_content_index_micro_new.ac16fbc2.js)，2026-11-19 核对。
- [官方原生扫码登录所依据的登录 UI](https://auth.zijieapi.com/ucenter_web/app/douyin-login-new/dist/index.umd.production.js)，2026-11-19 实际加载版本 2.4.22 / SDK 3.4.20。
- [官方安全消费者 SDK 引导](https://lf-fe-creator.douyinstatic.com/obj/passport-fe/ucenter_fe/byted/secure-consumer-sdk/creator/latest/bootstrap.umd.production.js)，同日核对。
- [创作者上传接口协议参考，固定提交 fc8b9c2](https://github.com/cv-cat/DouYin_Spider/blob/fc8b9c2490a9561265425c26058290f01c064eda/dy_apis/douyin_creator_api.py)。
- [VOD 签名协议参考，同一固定提交](https://github.com/cv-cat/DouYin_Spider/blob/fc8b9c2490a9561265425c26058290f01c064eda/utils/imagex_sign.py)。
- [原生客户端与 HTTP fixture](../src/publish_native_client.rs)、[上传模块](../src/publish_native_upload.rs)、[会话签名](../src/creator_auth.rs)、[事务记录](../src/publish_journal.rs)、[CLI 回归](../tests/publish_native_cli.rs)。
