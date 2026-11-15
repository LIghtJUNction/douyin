# 重建验收记录

当前为重建开发阶段，尚未发布。旧版以 Git HEAD `6fd6055` 核对功能，不恢复被用户刻意删除的实现。

## 必须完成

- `auth --adb`：明确设备选择；读取真实应用登录态、device_id、install_id、应用版本及设备信息；安全保存；在线验证账号；过期、无 root、多设备和账号切换给出明确结果。
- 导入后断开手机，全部功能仍需独立运行，包括开播；不得以连接手机代发请求替代独立实现。
- 直播必须以导入的安卓身份和安卓业务流程开播，并取得服务端按手机端直播处理的证据。OBS 两个方向都需要：OBS 编码推流到抖音、抖音直播画面进入 OBS。单独配置 RTMP 地址不证明安卓开播完成。
- 旧版网页功能：post、favorite、music、hashtag、search、following、follower、collection、mix、aweme；分页、增量采集、批量目标、媒体/封面/标题下载。
- 评论：主评论与回复分页，raw、chatml-jsonl、chatml-json。
- OAuth：授权地址、回调监听、code 交换、token 刷新与注销。
- OpenAPI：client token、userinfo、评论及回复、文字/图片/视频私信、同源通用请求。
- 离线 insights、stats；stdio MCP；原有命令兼容性（按用户最新要求移除 Obscura）。
- 扩展抖音功能：推荐、搜索、互动、发布与草稿、直播、消息、账号及设置等逐项建立能力与实机验收证据；通用请求入口本身不证明全部功能可用。
- 本地格式、测试、Clippy、release 构建、打包检查；实机验收；更新文档与技能；正式大版本发布与发布产物校验。

## 当前证据

- ADB 已连接 PHK110，抖音主包存在，`su -c id` 返回 root。
- ttnetCookieStore.xml 为空；应用 WebView Cookies 数据库存在 sessionid 等明文 Cookie，包括多个业务域。导入必须保留域、有效期和路径语义，不能拼接所有记录。
- 新实现的设备检查保持选定 serial，多个在线设备时拒绝自动选择。设备安装和 root 检查不代表账号已登录。
- 已新增设备登录导入、在线状态检查、离线状态检查及本地注销代码。Cookie 按域、路径和有效期筛选，配置原子写入并限制权限；10 项本地测试和 Clippy 通过。
- 直接探测账号信息接口成功；服务端设备 ID 为 0，不能用作当前设备身份。Cookie 安装 ID 与当前 Keva 记录一致，旧 XML 的安装 ID 已过期。
- 已修复 ADB 退出状态问题：使用无 PTY 的 shell v2，分离标准错误，命令失败不再当成成功读取；增加 30 秒超时和 16 MiB 输出限制。
- 已定位并修复设备 ID 校验误判：Keva 定长字符串后可能残留数字，按 tagged length 验证完整字符串，拒绝匹配其他 ID 的前缀。原始 XML 中的 device_id 得到当前记录交叉印证，install_id 使用 Cookie 中得到交叉印证的值。
- 手机曾因重启后未解锁而无法读取应用数据。解锁后 `auth --adb` 已成功读取登录态、验证账号并保存配置；`auth status` 联网验证成功，`auth status --offline` 明确返回未在线验证。配置文件权限 0600，目录 0700。新增未解锁提示及非主用户拒绝导入，防止使用其他 Android 用户的账号。
- 已全新实现 stats：JSON/JSONL、旧版扁平导出及原始接口嵌套数据，作者筛选、8 种排序、汇总/覆盖率/话题/作者/时长分组、JSON/Markdown 输出。整数排序避免浮点精度损失；缺失值保留为缺失。stdin Markdown 流程已实际运行。
- 已全新实现 insights：文本/JSON/JSONL/ChatML，带权热词、热梗候选、需求信号及 JSON/Markdown 输出。18 项本地测试和 Clippy 已通过；完整旧版输出契约对比仍需继续。
- 已增加绑定设备身份的 native API 请求层、业务错误和风控识别、同源地址限制、显式记录 ID 分页去重、游标循环检测，以及分页中途达到数量上限时的截断说明。
- 已接入旧式 `-u/-t/-l/-p` 采集入口和主要旧选项。实现批量目标、分享链接解析、元数据原子输出，以及视频/图文/封面/标题下载代码；尚不代表所有采集类型通过实机验收。增量采集与网页 Cookie 后端等旧版能力仍待补齐。
- `douyin me` 已联网读取资料并确认账号 ID 与导入登录态一致；`-t following -l 3 --no-download` 已实际导出 3 个关注用户，正确报告仍有后续记录。
- `download URL -o file` 已从实际抖音 CDN 下载 220995 字节 JPEG。媒体请求不携带 API Cookie，限制 HTTPS CDN、逐跳验证重定向，使用临时文件与长度/响应类型校验后提交。视频、图文与封面完整流程尚待验证。
- 搜索探测确认 `status_code=0` 但 `search_nil_info.search_nil_type=antispam_check`、`search_nil_item=hit_shark`；已明确报错，不把风控空列表当作无结果。资料显示有 145 个作品的公开账号也返回空作品列表，作品签名/可见性问题尚待解决，新增资料数量交叉检查以拒绝误报完整采集。
- OAuth 已重写：独立安全保存，兼容读取旧配置，授权 URL、二维码、随机 state、回调监听及路径/state/重复参数验证，code 交换和 token 刷新。回调监听器使用 tiny_http，拒绝账号意外切换，凭据不进入帮助输出。已做本地回调与真实 CLI 配置测试，尚无真实开放平台应用授权验收。
- OpenAPI 已补回旧版 11 个子命令，包括评论、文字/图片/视频私信和通用请求。固定官方同源、拒绝重定向和敏感头覆盖，有限响应读取，业务错误不包含私有响应。写操作需要确认。本地 TCP 伪服务和 CLI 拒绝用例已通过，未执行真实社交写操作。
- 评论已重写主评论和楼中楼分页、raw/ChatML JSON/JSONL、点赞阈值与角色配置。分页按 cid 去重，检查作品归属。当前使用设备会话，尚未通过真实作品评论的完整联网验收，也未接回网页 Cookie 评论后端。
- MCP 使用官方 rmcp 3.10.0：补回旧版 10 个工具并增加 native_me。真实 stdio 子进程测试分别覆盖 2025-11-25 initialize 握手和 2026-07-28 每请求元数据流程、工具枚举、离线工具执行及写确认。协议测试不代表线上业务接口已验收。
- 用户已明确不需要 Obscura，相关命令、模块、测试和使用说明已移除，不列入剩余验收范围。
- 作品增量归档已接入：本次数据更新相同作品，保留旧记录，兼容旧数组输入，按精确十进制 ID 排序；拒绝目标错配和损坏归档。保存事务使用文件锁和原子提交，默认目标文件按账号 ID 区分，避免账号切换混写；分页明确仅适用于本次采集。历史媒体兼容及完整联网流程仍需验收。
- Cookie 登录/检查/清除命令已重写：独立 0600 文件、兼容旧配置、文件或环境变量输入、格式及冲突校验。在线检查只接受官方 SSO has_login 明确证据，旧配置 Cookie 同时清除以防注销后恢复。本地测试通过；网页采集后端和真实网页会话验收仍待完成。
- 2026-11-13 再次实机运行 `auth --adb`、在线 `auth status` 和离线 `auth status --offline`：前两者确认登录，离线明确未在线验证；PHK110、抖音 40.8.0 的设备标识交叉核对通过。
- 补回旧扁平媒体字段兼容：视频 download_addr 字符串、图集 download_addr 数组、字符串封面及数字 ID；本地测试确认与 native 记录混合的增量归档可复用已有下载，并导出标题。完整联网视频/图集下载仍待验收。
- 推荐流验收请求发生响应读取失败，未导出成功数据，也未进入后续评论验收；单次传输失败不能用来断定签名问题或接口可用性。
- 进一步对比当前应用运行时，确认三种版本码不同：API version_code=400800、APK manifest_version_code=400801、update_version_code=40809900。此前把 APK 码用于 API 查询不正确，现已修复。
- 增加只读 Android PackageManager 辅助程序，源码与 3468 字节 DEX 均随包提供。`auth --adb` 临时上传、只读执行并清理，读取 API/更新版本和运行时 UA；独立交叉核对 dumpsys 的安装版本。无需 Frida、启动抖音或在电脑安装 Java。当前手机已成功运行完整新导入流程；保存的三种版本码与真实应用一致，配置仍为 0600，临时辅助程序清理已确认。开发侧 Android SDK 编译重现检查通过。
- 使用运行中应用的公共参数和签名辅助请求曾返回 6 条作品；尚未稳定复现，也未接入正式请求后端，不能视为作品采集验收。搜索同样参数下仍被风控；修正版本后的 CLI 再次导出 3 个关注用户，作品请求仍有非 JSON 响应。后续直接传输探测还出现业务状态 10000，已停止重复探测。
- ADB 期间发生断连与重新枚举，应用内通道验证未完成；当前转为离线核对安装包的网络调用链。临时手机调试服务与二进制已移除；离线安装包副本仅保留在本机临时研究目录，不随源码发布。
- 当前 54 项本地测试、Clippy、格式与 diff 检查通过。新增版本修正的 release 构建、cargo package 完整源码包编译验证也已通过；打包共 42 个文件，包含元数据程序的源码、DEX、构建脚本及 src/cookie.rs。USB 重新连接后，用 release 二进制再次执行 auth --adb，在线验证与设备标识交叉核对均成功。

未完成以上功能前不得宣称重建完成或发布最终大版本。

## 手机通道接入（继续开发）

- 离线检查当前 APK 的注册调用链，定位实际 GET 提供方。直接 GET 入口使用 NetworkUtils.LJII；未采用存在旧式重试逻辑的 LJFF/LJI。
- 使用隔离临时服务、独立 ADB 转发和当前应用公共参数的原型，成功读取关注列表、6 条作品、3 条评论及 3 条搜索结果；搜索返回状态 0，未出现 search_nil_info 风控空列表。观察到应用会自行补齐 ticket guard、token 等请求头，因此 frameSign 返回值本身不等于完整 HTTP 请求上下文。测试未记录这些敏感头的值。
- 原型的临时服务和端口转发已由监督程序清理。设备随后恢复，新的 Rust CLI 首次真实搜索测试退出失败，提示通道断开或超时；不能算 Rust 手机通道通过验收。
- Rust 正式接入新增 phone/phone_support 模块，使用 frida 0.17.2 官方绑定和匹配的 17.9.5 核心资产。支持文件按官方发布的 SHA-256、长度校验，XZ 解压限制内存和输出大小，缓存原子提交；四种 Android ABI 的资产清单固定随包提供。
- 应用内脚本在每次请求前核对当前账号 UID、device_id、iid、三种版本码以及 aid；拒绝同名参数、覆盖公共身份、外部路径和路径穿越。通过原生 GET 提供方发送，Cookie、签名、证书和安全密钥不传回电脑。
- SDK RPC 隔离在 CLI 子进程，父进程控制 45 秒响应期限、请求序号与大小限制，拒绝错序响应，并拥有手机服务与转发的清理责任。工作进程只接受本地 ADB 转发地址。新通道只实现 GET；不能据此宣称互动、发布或其他写功能已完成。
- 此阶段 Rust 测试 56 项与 Clippy 通过；Node 隔离测试覆盖账号/设备/版本变化、请求源与参数拒绝，确认无效请求未进入 GET 提供方。随后按用户明确要求改为默认 direct，仅显式 phone 允许协议调试，不自动调用已连接手机。17.9.5 与当前 Java bridge 的兼容性仍未证实，此前成功原型使用的是 17.26.2。
- Frida 核心开发包默认下载遇到网络错误，已通过官方发布 API 获取并校验 SHA-256 后缓存；开发机 release 构建使用只绑定本地地址、只提供该已校验资产的临时下载服务，未修改用户全局环境配置。

## 安卓直播和 OBS（继续开发）

- 当前 APK 40.8.0 的 Retrofit 定义确认 `RoomManagementRetrofitApi` 和 `RoomRetrofitApi` 中的 `POST /webcast/room/create/`（表单编码）及 `GET /webcast/room/info/`。这只是协议定义证据，尚未创建房间，也未证明安卓身份被服务端接受；不根据猜测拼接结束直播接口。
- `com.bytedance.android.live.broadcast.api.BroadcastRoomApi` 进一步确认：开播检查为 `GET /webcast/room/create_check/`（QueryMap），预览配置为 `GET /webcast/room/create_info/`（整型 platform），最近房间为 `POST /webcast/room/get_latest_room/`。同一接口另有 `getPCLivePermissionStatus(permission_name, need_apply_detail)` 和 OBS 审核查询；不能把这些 PC/OBS 相关方法当作安卓身份开播的证据。platform 的真实调用值仍需追踪，尚未猜填。
- 后续追踪发现实际预览调用是 `StartLiveApi.getPreviewRoomCreateInfo` 的多参数版本：platform、last_related_circle_id、enable_preview_stream、preview_resolutionkey、quick_live_source、quick_live_biz_params、scene。classes13.dex 的 `PreInitFragmentContext.OA(boolean)` 调用处明确将 platform 设为 2。该值只证明当前版本此调用路径，不等于服务器已认可独立请求的安卓身份。
- 开房调用进一步定位到 classes13.dex 的 `StartLiveViewModelHelper.EA(HashMap, …)`，调用 `StartLiveApi.createRoomWithPb(HashMap)`，参数通过 FieldMap 表单提交给 `/webcast/room/create/`。返回类型为包含 Room 和 RoomCreateBlockExtra 的 ExtraResponse；仅凭方法名不能断定实际 HTTP 响应编码。
- `live info ROOM_ID` 已接入固定 webcast.amemv.com 的 `/webcast/room/info/` 请求，共用导入的安卓身份，并按直播主机筛选 Cookie，不允许用户指定凭据接收主机。不支持该主机的手机调试通道明确报错。输出只保留指定类型的房间摘要，并核对房间 ID，丢弃流地址和密钥。本地目标绑定/密钥过滤测试通过；独立查询占位房间 1 的实际请求返回非 JSON，未冒充联网验收成功。当前 57 项测试与 Clippy 通过。
- 增加无敏感内容的响应诊断，区分空响应、未解码 gzip、明确 Protobuf 类型及其他非 JSON。再次同样查询确认是空响应，尚不能归因于压缩或 Protobuf，也不能证明不存在该房间；后续需要补全签名与服务端验证上下文。诊断不会输出响应正文或原始头值。

## 独立认证上下文调查

- 当前 APK 中 `com.ss.android.account.token.TTTokenInterceptor` 调用 `TTTokenUtils` 及 `TTTokenManager`，另有 Badger 报告头处理。`TTTokenManager.getXTTToken()` 只读取 token 字段；`addRequestHeader(url)` 则按 URL 进入 TokenFactory，包含 token guard、域/路径规则和 attestation 分支。因此复制单个 token 不等价于复现应用请求。
- 已定位 `TokenObjectProvider` 的 `get_token_object` 只读查询。手机 manifest authority 为 `com.ss.android.ugc.aweme.TokenObjectProvider`，未导出；此次仅通过用户已授权的 root ADB 查询。实际返回一行，包含 token、ts_sign、attest_ts_sign、token_encrypted、token_hash、token_tlb_tag、is_login。没有执行 set_token_object、更新、插入或清除操作。
- 查询结果在本机进程内存中处理，输出仅有列名和存在性，不记录凭据值。未向现有 session.json 保存未验证的 token，也未导出私钥。
- 使用单独 X-Tt-Token、不发送 Cookie 的只读验证：Passport 返回 HTTP 200，但没有证明与已导入账号匹配；profile/self 未返回可识别的账号响应。不能因此宣称 token 过期、当前账号不同或 token 已独立可用；已排除“直接复制 token 就能确认完整独立认证”的假设。下一步需要核对按 URL 生成的 guard 头和完整公共参数，之后再做账号绑定验证。
- 已沿实际混淆调用链定位 REE 实现：`X.1Ytd`（软件 key helper）、`X.1YuD`（软件 key pair）及 `net.bytedance.zdplib.DeltaSignerVerifier`。原始类名出现在 Kotlin 反射字符串中，不能直接据此把它当作 APK 中保留了原名的类。SDK 私钥是 32 字节标量的十六进制字符串，子进程公钥由未压缩点字节转 Base64；不是此前尝试的 PKCS#8 DER 导入格式。
- `TicketGuardProvider/query_subprocess_sign_data` 能提供软件签名快照，但初始化期间公钥可能为 NULL，即使其他字段已经存在也不能当作完整快照。启动已登录应用后重新取到完整快照，在本机内存中通过 OpenSSL 按 P-256 标量导出的公钥与 Provider 公钥匹配。未将私钥写入磁盘、仓库或日志，也没有改写手机密钥。
- 新增 `guard::ReeSigner`，使用现有 ring 依赖实现软件快照格式校验、公私钥一致性校验及 P-256/SHA-256 ASN.1 DER 签名/验签。拒绝缺失、NULL、非法格式、无效曲线点和不匹配密钥；错误不含输入凭据。此模块尚未启用为 HTTP 签名器，不代表账号绑定、请求规范化或服务端验收完成。
- 新增开发用 `examples/guard-probe.rs`，仅从有大小限制的 stdin 接收快照，只输出布尔验证结果。以固定公开消息调用手机 SDK 的 REE sign，再在电脑端验证：local_signature_verified=true、native_signature_verified=true。证实软件签名原语与该设备当前 SDK 的样本互通；未向抖音服务端提交直播或社交写请求。
- 当前 60 项本地测试通过，Clippy 和 diff 检查通过。仍需实现完整 guard 头结构、请求消息构造、token/key/账号绑定与刷新，并完成断开手机后的真实业务验收。TEE 相关要求不能因 REE 原语互通就认定已满足。
- 请求原文进一步定位到 `X.1ZVK.LIZIZ/LIZJ`：按 `ticket=…&path=…&timestamp=…` 直接拼接，不做 URL 编码。`ReeSigner` 已增加请求级签名/验签接口，拒绝无效路径/时间戳，测试覆盖原始百分号与加号保留、ticket/路径/时间戳分别被修改后的拒绝。开发 probe 使用固定公开 ticket/路径/时间戳，与手机原生 REE sign 对照均通过。
- `ClientDataTypeAdapter` 的序列化字段已核对：req_content、timestamp（整数）、ts_sign、ts_sign_ree、req_sign、req_sign_ree，extra_info 按空值分支处理。客户端声明 req_content 为 ticket,path,timestamp。完整 HTTP 头组装及各模式选择尚未接入，不能将已定位字段当作已实现全部 guard。
- 另确认软件 HMAC 路径 `X.1Yta.LJII` → `X.1XVl.LIZ` 使用 HmacSHA256，SDK 快照将原始 HMAC 密钥以 Base64 提供。新增 `HmacSigner`，使用 ring 实现原语和同样的请求级签名/验签，拒绝 NULL/null 哨兵、空值和超限输入；RFC 4231 公共测试向量通过。
- 手机 `ree_hmac` 对固定公开请求原文生成的样本，也已在电脑端验证成功：local_signature_verified=true、native_signature_verified=true。两种算法的样本对照均未发送抖音业务请求，真实密钥仅经进程内存及本机子进程 stdin 处理，未保存到本地会话、磁盘或仓库。当前 62 项测试和 Clippy 通过。
- 新增软件 client-data 封装：req_content、整数 timestamp、空的 TEE 字段、ts_sign_ree、Base64 req_sign_ree，整个 JSON 以 UTF-8 编码再转无换行标准 Base64。签名原文中的真实 ticket、私钥、HMAC 密钥和 SDK transient requestContentText 不进入 JSON。该类型仅表示软件字段，不是完整 HTTP guard 头组，也未接到线上请求。
- 使用独立临时 DEX，通过安装包的实际 `X.1YtB` 模型和 `ClientDataTypeAdapter` 在手机生成固定公开样本，确认 6 个字段、整数类型及 extra_info=null 时省略的行为。公共样本保存于 assets/guard-fixtures，工具源码为 tools/GuardWireProbe.java；没有真实账号、密钥或有效业务签名。Rust 测试比较解码 JSON 结构，不将键顺序差异误称为字节完全一致。手机临时 DEX 已清理。当前 64 项本地测试、Clippy 与 diff 检查通过。
- 已增加 `live obs status/configure-push/add-source/stop`，使用维护中的 obws 0.15.0，通过本机 OBS WebSocket 操作。推流目标与媒体地址从有限大小文件或 stdin 读取；密钥保持原始字节，不打印。配置/启动/媒体源/停止需要显式 `--yes`，默认配置不启动。
- 使用独立 Xvfb、临时 XDG 配置与 OBS 33.0.1 实际验证密码认证、状态、配置目标但不开始推流、创建指定场景的 ffmpeg_source，以及空闲停止。临时 OBS 与 Xvfb 进程已结束，测试目录已清理。测试没有真实推流，也没有验证直播画面解码。
- 当前源码 56 项本地测试、Clippy、Node 手机脚本校验及 diff 检查通过。归档锁增加显式释放，避免并发启动子进程时短暂继承文件描述符造成锁生命周期延长。
- 新增 `guard::TokenProofs`：按 APK 中的组合证明结构区分 ts_sign 与 ts_sign_ree，旧式不透明证明只归 TEE，缺少 REE 证明时明确拒绝软件证明选择。拒绝异常类型、控制字符、NULL 哨兵和过大输入，错误不带原值。这不是完整签名模式选择，也尚未接入 HTTP 认证链路。
- 手机探针验证了 CombineTsSign 的实际 SerializedName 注解、构造器及 getter，生成 6 个公开模型对照样本。输入 JSON 由 Android org.json 解析；独立进程直接初始化应用 Gson 因缺少 ApplicationHolder/Keva 上下文失败，未宣称原生 Gson 解析验收。临时 DEX 已删除，样本不含真实账号或证明。
- 时间戳调用链已进一步定位：TokenFactory 从 `X.1YZa.LIZIZ(context,true)` 取账号时间结果，再加 `X.1Yoy.LJIILLIIL` 配置偏移后传入签名上下文。账号时间方法同时读取 wall clock、elapsedRealtime、持久时间状态与配置分支；另有签名缓存可复用签名时间戳。尚未确认所有状态/配置语义，不能简单用当前 Unix 秒替代并宣称与 SDK 一致。
- 本轮验证：66 项本地测试、Clippy、格式和 diff 检查通过；release 构建和离线 cargo package 实际编译验证通过。未安装、推送或发布新版本，也未进行直播/社交写操作。
- 进一步确认时间单位：`X.1Ys5.LIZIZ()` 将正的上下文毫秒值整除 1000；非正时才读取当前系统毫秒并转秒。签名原文与 client-data 使用该秒值，签名缓存另有复用分支。
- `guard-probe` 新增有界 stdin 的 `request` 对象（ticket/path/timestamp），仍只输出布尔结果。实际应用关注列表 GET 返回 status_code=0；捕获后续网络处理阶段的真实 guard 数据，在内存中用该请求的 X-Tt-Token、路径、时间戳和 SDK HMAC 快照验签，local_signature_verified 与 native_signature_verified 均为 true。P-256 验签该 HMAC 样本为 false，未据此推断其他路径模式。
- 同次实验，单独 TTTokenManager.addRequestHeader 仅得到早期头集合，未含 client-data；实际网络请求另补入 client-data、版本、REE 公钥等头。证实不能只复制早期工厂输出。验证了请求上下文中的签名互通，尚未证明服务端账号/token 绑定、全部头生成、刷新或断开手机后的独立 HTTP 请求。追加搜索/电脑重放探测在 RPC 阶段失败，没有获得电脑端成功响应证据。
- 所有真实 token、签名、头值和密钥仅在进程内存流转，未写入样本或日志。临时 Frida 服务、文件与 ADB 转发已清理。动态探针公开样本的正确签名、改路径失配、非法时间戳拒绝均验证通过；66 项测试、Clippy、格式和 diff 检查通过。
- 新的分阶段探针已完成实际搜索对照：当前 Android 账号、device_id 与 iid 匹配导入记录时，手机返回 status_code=0、3 条搜索结果；该请求 client-data 的 req_sign_ree 再次通过本地 HMAC 验证。观察到 guard-version=3、iteration-version=3，client-data 内 TEE 请求签名/证明为空、REE 证明存在；部分请求仍附带 client-cert、sec-ts、static-sign、static-ts-sign，不能据此宣称完整请求不需要额外设备证明。
- 电脑端重放该搜索返回 HTTP 200、status_code=0，但 data 为空且存在 search_nil_info。移除误加的 frameSign 字段、补入运行中 CookieHandler 的真实 Cookie、保留原始 client-data 字节且核对本地 HMAC 一致后，仍得到同类风控空列表。这是失败验收，不是独立搜索成功；Cookie 缺失并非唯一差异，也没有证明 HMAC 本身被服务端拒绝。
- frameSign 实测返回 frameType/lid/signInfo/signValue/signVersion 结构字段。APK 中 SecApiImpl.LJIIIZ 使用 frameSign 参数 2；开发原型参数 0 的返回 map 不应直接作为 HTTP 头发送。追踪到实际网络签名/上下文之前，不应继续把这个 API 当完整 HTTP 签名器。
- 已从 classes48.dex 定位 CronetUrlRequest 与 CronetUrlRequestContext 的 addSecurityFactor：它们接收交替键值数组，转换为小写键 map，调用 onCallToAddSecurityFactor 再转回数组。本次在上下文方法的挂钩未观察到目标请求调用，不能据此认定其它安全字段或签名不存在。
- 实际请求调用栈确认进入 X.1SoD 与 X.1T5e（SsCronetHttpClient），后者通过 ICronetClient 创建 CronetHttpURLConnection。其 LJJIJIL 在 Retrofit Request.getHeaders 之后继续设置连接属性：若请求未指定 User-Agent，则尝试 X.1Ywf.LJFF 的全局值，并在非空时追加 cronet/版本；另有 ttUrlDispatch 分支。早期 Java Request.getHeaders 不能证明完整出站上下文已被复制。
- 对连接配置方法挂钩的追加探针遇到 InvalidOperationError；只读 UA 探针未取得可用的全局 UA/客户端组合，未取得最终出站 UA。现有元数据助手读取的是独立 app_process 的 http.agent，尚未证明等同于应用最终 TTNet UA，不应将这一点标记为完成。
- 身份检查曾暂时阻止探针；后续检查再次匹配导入的账号与设备标识，没有覆盖已有身份。新的研究探针区分标识未就绪、有效 device_id 变化和有效 iid 变化；尚未据此证明早先失败的具体原因。真实业务探测仍限 GET，没有开播、发帖、发送消息或其它社交写操作。
- 剩余核心：独立签名和设备验证、安卓创建/心跳/结束直播流程、账号与房间绑定、从实际响应安全提取推拉流信息、OBS 端到端画面及推流验收，以及服务端手机端开播证据。OBS-only 成功不替代这些要求。

- 使用 CraftAnything 的 APK 方法和引用追踪，定位 `X.1Zev.LIZ(Context)` 默认 Cronet UA 生成器。独立进程直接调用受包管理埋点初始化影响；在临时 VM 内用实际 PackageManager versionCode 初始化其缓存后，真实手机成功读取 SDK 默认值。它仍不能证明应用全局覆盖后的最终出站 UA。
- 元数据助手对 40.8.0 / 400801 导入该默认值，保存于私有 nativeTransport，明确标记 finalRequestVerified=false；有效请求 UA 保持独立。已重新构建 bundled DEX，重复构建检查通过。
- 实机发现 Cookie 数据库已迁至 `app_webview/Default/Cookies`。导入支持固定路径识别，多个候选同时存在时拒绝混合。当前私有会话已包含新 Cookie 来源及 SDK 默认 UA，权限为 0600；这些私有字段不进入公开摘要。
- 后续隔离 UA 对照验收：复制私有会话到 0700 临时目录、0600 文件，显式 direct，分别使用原 http.agent 与导入的 Cronet 默认 UA。两者 `me` 均返回匹配账号；两者限定 3 条的 rust 搜索均失败、输出记录为 0，并报告风控/验证类错误。没有覆盖原会话，没有使用手机通道；临时配置和结果已清理。默认 UA 本身不足以解除当前搜索失败，不能据此宣称完整独立客户端已验收。

- 当前 Rust 手机诊断通道实机 `me` 成功，身份核对通过；搜索两次超时，包括将实际 count 从固定 20 改为 3 后的复测。不能据此宣称 Rust 手机搜索已验收，也没有把手机通道改为默认回退。临时查询结果已清理，转发已由所属服务释放。
- 修正分页请求预算：回调接收去重后的剩余条数；作品、评论和回复请求 count 不再超过剩余额度，limit=0 保持按配置页大小请求。测试核对了有边界重复项时剩余额度由 3 变为 1，以及不限量模式。68 项本地测试通过。手机错误增加初始化/接口请求阶段区分，仍只显示脱敏错误。
- 新增受限初始化阶段帧（SDK、服务连接、附加、创建脚本、加载脚本、身份核对）。监督进程验证阶段顺序和请求序号，全部阶段共享原始总截止时间；未知阶段只返回静态错误，不回显工作进程内容。新增协议测试后共 69 项测试通过，Clippy 通过。
- 实机后续诊断约 6.4 秒在创建应用脚本阶段断开，搜索尚未发出；此前初始化失败不能归因于搜索接口超时。一次 me 成功仍不足以证明通道稳定，脚本创建失败需要继续调查。

- 隔离脚本创建探针：临时启动独立本地端口服务，使用官方 Python 绑定只尝试 attach 与创建公开脚本，不 load、不初始化账号、不调用业务请求。现有 17.9.5 服务和缓存中的较新服务都在 attach 阶段报告 unexpectedly timed out while waiting for stop；最小脚本尚未进入编译，不能据此认定完整 JS 有语法错误，也没有证据支持仅升级核心即可修复。Rust SDK 的 create_script 会丢弃 GError 细节并统一返回 ScriptCreationError。
- 探针后主进程状态为 S，TracerPid 为 0。崩溃缓冲中存在较早的 signal 11 记录，尚无时间关联证据，不能归因为本次创建失败。未改变 SDK 依赖、未重启/强停应用，所属临时服务、文件和转发已清理。
- 读取 cgroup.events 发现目标应用处于有效冻结状态（即使该节点 cgroup.freeze=0，也可能继承父级冻结）。手机诊断通道现会在下载/启动支持服务前核对有效冻结状态，提示解锁并将抖音置于前台；不写 freezer 控制、不重启/强停应用。路径和事件解析拒绝异常值、重复字段及路径穿越，不回显原值。70 项本地测试、Clippy 通过。
- 用户解锁并将抖音置于前台后，冻结预检通过；现有 Rust 通道仍在 attach 阶段失败。官方较新 Python/服务组合能创建最小和完整公开脚本，但现有旧 Rust 客户端连接新服务仍不能 attach，因此不能只改服务端资产版本。正在隔离验证官方手动 devkit 构建路线，尚未改变项目的 SDK 依赖或默认支持服务。
- 新 devkit 隔离实验：已从官方 GitHub 17.26.2 发布资产下载 linux-x86_64 devkit，按官方资产大小及 SHA-256 验证。原 Rust 包装层可在不启用 auto-download 的临时清单中编译链接该核心，运行确认 core=17.26.2；但附加仍未通过。底层 C API 保留的错误为 Connection closed / waiting for stop 超时，显式创建 SessionOptions 也未解决。因此没有把单次较新 Python 探针成功推广为 Rust 通道已修复，也没有修改项目依赖。隔离源码、SDK 和构建缓存保留用于后续研究，不包含真实凭据。
- 本轮产品代码：有效冻结前置检测、严格路径/事件解析。70 项测试、Clippy、格式/diff、release 构建和打包验证通过；真实手机附加问题仍未解决。

- 无 Frida 的 Binder 入口已实测：root content query 可读取非导出的 TokenObjectProvider/get_token_object 与 TicketGuardProvider/query_subprocess_sign_data。早期读取尚未稳定，后续获得前后 Token 一致、is_login 有效的快照；导出材料在既有 Rust guard-probe 中分别通过 HMAC 与 P-256 本地签名校验。真实材料只在内存和子进程 stdin 中流转，没有写入会话、日志或测试样本。
- Token 与 token_encrypted 不同；ts_sign 为含 ts_sign/ts_sign_ree 的组合对象。无 Cookie 独立认证试验未通过：不带 Token 和携带 Provider Token 的 self/profile 均返回 status_code=8；补软件证明和 HMAC client-data 后仍为 8，Passport account/info/v2 返回 error_code=13。完整模式选择、校准时间及最终请求上下文仍未复现，不能归因为某个单独字段，也不能把本地原语通过当作服务端账号绑定通过。Provider 导入路径尚未接入生产登录态。

- 新增显式开发探针 `cargo run --locked --offline --example guard-adb-probe -- [serial]`，通过 Binder 的 query_subprocess_sign_data / ree_hmac 对照公开固定消息，不需要 Frida 附加。限定主用户及已检查 APK 版本，签名前后快照必须一致，严格解析单行固定字段，不回显真实值及原生异常。实机 hmac_matches_native=true、ree_pair_valid=true、snapshot_stable=true；账号绑定与独立认证标志仍为 false。
- Cursor 解析新增离线测试，核对 Base64 等号保留、异常字段顺序/重复、多行、控制字符、超大输入及错误脱敏。71 项本地测试、Clippy、格式检查通过。
- WebView Cookie 当前在线校验仍返回保存的同一账号；ttnetCookieStore.xml 实际为空，未将它视为可用原生 Cookie 来源。Smartisan Provider 只有会话同步入口，未调用其同步方法。
- HTTP 协议隔离对照：当前 reqwest 构建未启用 http2。另用同一个 curl/OpenSSL 客户端、同一保存 Cookie/UA/查询参数，分别协商 HTTP/1.1 与 HTTP/2，实际协商版本均已确认；self/profile 两组均 HTTP 200、status_code=0、账号匹配，搜索 rust/count=3 两组均 HTTP 200、status_code=0、零记录并含 search_nil_info。当前进程没有设置 HTTP_PROXY/HTTPS_PROXY/ALL_PROXY 及其小写环境变量；这不能排除系统透明代理。此对照表明仅切换 HTTP 协议不足以恢复该搜索请求，不证明原生签名、TLS 指纹或出口网络已正确复现。未因此修改产品网络依赖；真实 Cookie 仅通过内存及 curl stdin 配置传递，响应正文未写盘或回显。
- Binder 软件签名快照再次可读，subprocess_sign_strategy 当前可解析为 JSON 整数，尚未核对枚举语义；不能据字段存在就决定生产请求采用何种签名模式。
- 后续静态追踪纠正了上述策略字段的解释：`X.1Yth.LIZIZ` 中 0 仅 IPC，1 先 IPC 后内存，2 先内存后 IPC，其他值与 1 同支；回退条件是 null，而非空字符串。`X.1Yth.LIZJ` 用另一个布尔参数选择 HMAC 或 REE/ECDSA，`X.1YtZ.LJII` 传 true、`LJIIL` 传 false。该策略描述子进程执行位置和尝试顺序，不是 HTTP 签名算法/TEE 选择。Provider 无快照也默认输出 1，须独立核对密钥可用性。Craft Anything 证据：`ev_4dc37b05036c9df762824fd7645e503c56726679832052fe82042092b54e1ff1`、`ev_8eecceff6b0fb4e74b27b03742ec101c6d90faac86e13f519a0f04ab5ddcf82c`、`ev_7afe31fb184be63d5583eba6f8f79e14c278d0263b65c5dbcf1e61c4f52529b3`。
- 应用此前主进程已退出，未据此认定崩溃原因。重新正常打开应用后，有效冻结为 false、设备为 Awake。隔离 Python/Frida 17.26.2 探针成功附加、加载当前完整 agent、校验导入身份；self/profile 为同一账号，搜索 rust/count=3 返回 3 条且无 search_nil_info。随后第二个探针再次通过相同请求，并在 `com.ttnet.org.chromium.net.impl.CronetUrlRequest.start()` 捕获目标搜索的 Java 请求快照。此处初始主机仍为 aweme.snssdk.com；含 guard、TLB、Token 补充、Cookie 等请求头，但没有显式 User-Agent。该位置仍不证明原生网络层最终请求头、UA 或调度后的 URL 已捕获。仅输出主机、参数名、头名称和校验布尔/计数；真实 URL 参数及头值只留在临时进程内存，未落盘。探针所属服务、远端文件及转发均在 finally 清理。生产 Rust 通道、独立认证和开播尚未因此通过。
- 后续指标探针再次取得原生搜索 3 条；`CronetUrlRequest.mFinalUrl` 在 `onMetricsCollected` 才被写入，实际搜索主机为 search3-search.amemv.com，与初始 aweme.snssdk.com 不同，参数名集合一致。证据 `ev_4282e396c5ea0b9d5f448f1400c6956e18e7d03a9fd5088d439696feac5914f6`。但独立端仅将同一普通 Cookie 请求切换到该主机仍为 HTTP/2、HTTP 200、status_code=0、零记录并含 search_nil_info，故未硬编码替换产品接口主机。完整原生上下文的双主机重放尚未执行成功：先遇到冻结导致 attach 超时，用户重新置于前台后另一次探针在查询主进程时已无目标 PID。系统退出历史及 crash buffer 有本轮时段的新近 SIGSEGV，无充分堆栈确定触发点。已暂停附加重试；临时开发探针增加显式还原 Java hook、释放 retain、卸载脚本及上传前冻结预检，仅做离线语法检查，不宣称崩溃已修复。
- 无附加的 Token 元数据检查取得 11 个 SDK 跟踪字段，包含 reePub/teePub 和来源、更新时间等，没有直接 UID 字段；前后稳定 Token 所记录的 REE 公钥与软件快照一致。新增 `guard-adb-probe --token-context [serial]`：严格解析已登录单行 Token 的 JSON 元数据，拒绝重复 reePub、异常边界、多行及过大响应，前后分别核对 Token 与软件快照未变，再比较 Token 公钥与已校验的密钥对；HMAC 材料只检查格式。实机 token_snapshot_stable、software_snapshot_stable、token_ree_key_matches 均为 true，账号绑定及独立认证仍为 false。该关联仅证明 SDK 快照一致性，不证明 HMAC 与账号的服务端绑定。没有将真实材料写入会话或夹具。
- 本轮新增公钥关联和元数据边界回归测试，73 项本地测试及 Clippy 通过；serde 从既有传递依赖显式声明为直接依赖，以使用拒绝重复字段的结构化反序列化。
- 同轮 release 构建、61 文件的 cargo package 完整编译验证、CLI help、格式和 diff 检查通过，未发布。随后准备比较 Token 与 token_encrypted 的无 Cookie 请求时，快照准备未通过，因此没有发出这两组 HTTP 请求；复核显示软件快照可读，但 Token 行暂不可稳定读取，不能把它解释成账号已退出或某种 Token 表示被服务端拒绝。
- 继续检查 Token Provider 的初始化门槛：`LIZIZ` 在 `TTTokenManager.LIZIZ` 尚未置位时返回 false，`query` 随之返回 null；查询异常也会被捕获后返回 null。因此 content 工具的空结果不能等同于显式 `is_login=0`，也不能把所有空结果都归因于初始化。固定字段顺序仍为已解析的八列，后续实测又能读到单行。探针现将明确的 `No result found.` 单独提示为 SDK 暂未提供快照，不判定退出登录；没有强设 SDK 初始化标志或改写应用状态。相关两项解析测试通过。静态证据：`ev_c5a6cfa53ebc94331b8ba2fd50b94a86148e446c8591e644c3a1c28d76e14c3d`、`ev_7a4c4e305fbbab648f64e2cfd5cf5e6390cc242e22c32f1e9d11b680b1a468ea`。


## 2026-11-14 原生 Token 基础认证验收

此前不带完整 SDK 上下文的 Token 试验失败，不能据此判断 Token 本身无效。APK Header factory 选择原始 Token，并同时写入 sdk-version=2、passport-sdk-version=60760；证据 `ev_3ff1e67a2d2234717282bf06e0a9c10ddbd2ac4026e724e0631ba629fe38a665`。受控 HTTP/1.1 对照仅改变这两个 SDK 头时，self/profile 从 status_code=8 变为 0 并返回预期账号；纯 Rust 请求另行通过。这不证明服务端验证了 HMAC，也不证明完整请求签名已经复现。

当前支持 40.8.0 / 400800 / 400801 / 40809900：从稳定 SDK Token 与 Token-only self/profile 响应确定账号，再次核对同一账号；安装 ID 从 XML 读取并在当前 Keva 记录中佐证。Token-only Passport 启动验证本轮返回 HTTP 403，已保留为接口差异证据，未添加拒绝响应重试或 Cookie 降级。Cookie 改为可选兼容凭据：快照一致、安装 ID 一致且 Passport 独立确认同一账号才保存；失败不阻断已经验证的原生 Token 登录。保存前再次核对手机 Token 未变化。

真实手机隔离验收通过：临时 ADB 包装器仅屏蔽 Cookie 数据库发现，其余操作仍访问实际手机；`auth --adb` 成功，Cookie 数组为空，账号及安装 ID 与原有已验证导入一致，私有配置权限为 0600。随后将 PATH 设为不可用，状态验证与 me 均成功且账号一致。没有删除或修改手机 Cookie 数据库，也没有更新默认用户配置。同轮普通导入也成功，无 ADB、无 Cookie 的关注列表复测通过；该次 nativeCookiesImported=false，不能据此宣称可选 Cookie 兼容分支已在线验收。

80 项本地测试和 Clippy 通过。原生 Token 仅发送到已验证的 aweme.snssdk.com，不转发给媒体、OpenAPI 或未验收的直播主机；TEE/REE/HMAC 私钥未加入会话。搜索、完整签名和安卓身份开播仍未验收，OBS 双向媒体链路也不能由这些基础认证结果推定完成。


- 保存前新增设备与安装标识复核：重新读取 XML/Keva，原生 Token 路径要求 XML 两项标识与待保存值相同；旧 Cookie 路径保留其安装 ID 来源规则。标识中途变化会拒绝保存。实机隔离测试只在 ADB 包装器第二次读取 XML 时替换为公开虚拟标识，导入退出 1、提示标识变化，原有测试配置逐字节保持不变；没有修改手机数据。正常无 Cookie 数据库导入及无 ADB 状态/me 复测仍通过。81 项测试、Clippy、release 构建及打包验证通过。


- 已接入独立 `live permissions` 与 MCP `native_live_permissions`。客户端固定 GET 直播权限路径单独使用经验证的 Token/SDK 头及已接受的请求 UA，不扩大其它直播路径的认证白名单，不使用 Cookie 回退或手机通道。请求先用 me 确认同一账号，再取本次 sec_uid；输出认证类型、已知开关与栏目名称，不输出历史房间或任意响应字段值。Cookie-only 会话在发请求前拒绝。
- 实机隔离导入后，清空 Cookie、禁用 ADB：实际 CLI 和 MCP 2025-11-25 工具调用均成功，Token 未进入输出，last_room 被排除。83 项测试及严格 Clippy 通过，release 构建通过；创建/续播/推流/心跳/结束仍未调用或验收，未安装或发布新版本。


- 根据 APK PermissionResult 和 broadcastConfig 字段注解扩展查询摘要：补全录制/回放/商城/群聊等 11 项已知设置，以及 7 项 scheduled_* 预约设置；保留 int32 负值哨兵，过滤越界设置数字、未知值和历史房间。broadcast_config 的实际含义是预约配置，不当作 canBroadcast。实机隔离复测 CLI 与 MCP 均在无 ADB、无 Cookie 下成功，返回 11 项开关及 7 项预约字段，Token 未进入输出。84 项测试和严格 Clippy 通过。
- 创建表单的 VIDEO/THIRD_PARTY 元数据、外部摄像头条件和三处后续合并均已核对，尚未发出创建/续播/结束请求。完整表单、手机身份开播与 OBS 媒体验收仍未完成。


- 创建提交调用链继续核对到 ExtraResponse<Room,RoomCreateBlockExtra> 成功回调及 SDK Room.getStreamType。`live info` 摘要新增 sdk_stream_type，按已核对的媒体布局、录屏、第三方、音频、视频优先级分类；必要字段缺失时保持 null，不把不完整 JSON 当作手机视频房间。新增优先级、字段缺失和错误类型测试，共 85 项测试与 Clippy 通过；release 构建通过。该改动未新增真实房间联网验收，不能视为安卓开播证明。


- 完整软件 guard 的后续对照仍未验收：补齐已核对 SDK 头和候选 UA，Passport 无 guard 返回 HTTP 403/空响应，正确 HMAC 与故意改错 HMAC 均返回 HTTP 200、error_code=13。搜索在原主机与此前观察到的最终主机返回 status_code=2483；补 Build.DISPLAY 和匹配 Token 公钥的缓存静态证明仍未成功。这些结果不能证明服务端接受 HMAC，也不应将 2483 擅自解释为特定失败原因。没有把实验头设为默认，也没有持久化私钥。
- APK 静态证明路径已核对：get_tee_static_sign_result 读取缓存；响应 Cursor 还有 exception_message 列，不能把整列尾部当成 JSON。静态证明头与主请求头使用独立列表，仅在 X.1Yoy.LJJJLIIL 开关开启时合并。证据：ev_d3d0fd8d6b580e82f4f8cf465b29ab21febb293f1cecd378fe3ec37929274e41、ev_20f4323d918abef100b9ce6d3e01867e0b66fdd3e7a463c3298b97aa8291579d、ev_0bf2957c3a0a464b6375e6812f4d98d74597b91af2bfb455bd93b2ba764dbbe8。源码和实验均不支持无条件启用这些头。
- Token 头工厂还会在 Token 对象 LJFF 非空时添加 token-tlb-tag，并按上下文添加 bd-ticket-guard-request-tlb-group（ev_f539c383ab1ad9d0f741d2eba5af197011719b47c6f81e78ee5ce97b8698e9f9）。该轮生产导入尚未包含这些字段；当时尚未完成独立请求对照，不能宣称已完整复现工厂或最终网络上下文。
- 后续 Token Provider 明确返回 is_login=0，用户确认手机已退出并重新登录。没有调用退出登录接口；退出原因未确定，也不能排除诊断请求触发服务端安全校验的可能。暂停 guard 请求实验，先验证普通导入；本轮未将真实退出当作初始化延迟修改导入逻辑。真实凭据和完整响应均未写入公开记录。

- 用户重新登录后，使用自动清理的隔离目录再次执行 auth --adb 成功；随后 PATH=/nonexistent 的 auth status 也成功。没有覆盖默认配置，没有恢复 guard 实验；该结果只证明重新登录后的普通导入和独立状态验证。

- 修正原生导入顺序：受支持版本在读取 XML/Keva 设备及安装标识之前正常打开应用，避免先取快照、再启动 SDK 导致初始化更新后混用标识。Token 导入不再重复打开应用；保存前的标识与 Token 复核保持不变。此调整不把 is_login=0 当成有效登录，也不增加签名试验。
- 实机顺序验收：临时 ADB 包装器在正常启动成功前拒绝任何 applog_stats.xml 读取；新导入成功，随后 PATH=/nonexistent 的 auth status 成功。临时配置与包装器自动清理，默认配置未改变。85 项本地测试、严格 Clippy 和打包编译通过。

- 离线继续核对 TLB 上下文：X.1Ypq.LJJJJI 在响应头分支读取 token-tlb-tag，将值放入新 Token 对象的第五个构造参数；更新 attest 证明或登录状态的分支保留旧对象 LJFF，而不是生成新标签（ev_5aabe3a787f0e78ab2bbf6b14ef3c3397205583ea06fb6629618a32287515ff4）。X.1Ypr.LJIIIZ 从 Provider 的 token_tlb_tag 列恢复该参数，空值按空字符串处理（ev_790ed74bd022e10a34541f790970b2f3c58db7a28a94dd3cfdea8193e7df83f8）。不能用设备标识、签名摘要或自行生成字符串代替服务端标签。
- request-tlb-group 是另一项配置：X.1Yoy.LJJJJJ 默认空字符串（ev_151738749874b6a838eee93b97edd981742e27a35faa4a78b066355ee471fb25），加载键 bd_ticket_guard_request_tlb_group（ev_f7833db7a4f98274b452205d3e3bdd2771d8b26e63db54bf0c65666c46ddda46）；请求头工厂及设置更新请求均仅在配置非空时发送（ev_2ea423e08044f580347dac7c2f9ed868c811c35019635c19473ba9e0e2a49f01）。它与服务端 Token 标签的来源和生命周期不同，不应相互推导或固定为猜测值。本轮只读本地 APK，没有访问手机、导出材料或发出联网实验请求；生产头与会话格式未改变。

- OBS 新增只读 source-status --name，返回媒体源状态、duration_ms 和 cursor_ms，不读取源设置或拉流地址，不要求 --yes。隔离 OBS 33.0.1 使用本机 HTTP 测试视频，实际 add-source 成功；source-status 返回 OBS_MEDIA_STATE_PLAYING、duration_ms=30000、cursor_ms=400，缺失媒体源查询被拒绝。未启动推流，未访问手机或抖音直播接口；隔离 OBS、HTTP 服务和临时配置已清理。此验收不证明真实抖音直播画面或安卓身份开播。

- OBS configure-push 增加完整 rtmp_push_url 输入，避免手动拆分破坏签名。边界测试覆盖编码斜杠、查询串内斜杠、空流名称、空值和混用配置；86 项测试通过。隔离 OBS 33.0.1 实际配置后，直接读取其隔离 service.json 核对 server+key 与公开测试地址逐字节一致，且推流保持空闲。本地媒体播放与 source-status 复测通过；没有连接真实推流主机、访问手机或创建直播间。

- 原生 Token 导入现解析配套 token_tlb_tag：通过 JSON 解码器确认 track_data 结束边界，仅保留 Token 和非空标签，不保存跟踪对象或证明。两次稳定快照及保存前复核同时比较 Token 与标签；NULL/空标签不发送，旧版无标签会话仍兼容。标签请求头标记 sensitive，沿用 Token 的主机范围及固定直播权限端点例外，没有启用 request-tlb-group 或软件 guard 默认头。
- 新增标签轮换、元数据嵌套/边界、异常尾部、敏感标记和主机范围测试，共 88 项测试通过。实机隔离导入在发现设备阶段报告没有已授权 ADB 设备，尚未发出新导入的账号请求；新增非空标签的在线验收待手机恢复，不将以前无标签认证结果当作本轮证明。默认配置未修改。

- 新增 guard::policy 路径规则组件，依据 X.1Yoy.LIZIZ/LJI/LJII 及 TokenFactory 调用链，独立输出 TEE、REE、HMAC 规则请求，不根据本机可用密钥做降级。use_tee_ree_config 开启时分开匹配 TEE/REE，关闭时旧路径命中同时请求二者；HMAC 另受 enable_symmetric_encryption 控制，并与 REE 共用 ree_force_sign_paths。强制精确路径优先，其次排除精确/前缀，最后包含精确/前缀；匹配保持大小写和原始编码。组件要求调用方明确提供有效配置，没有接入生产请求默认签名。
- 路径类型选择器证据 ev_f63e8306902db9655492d4121efa14fdeb0637ca7c0c39a79ab9601299771456；规则优先级 ev_66f86ba082851bc26700f420200278a47807b1f78caa6b22efc8225cfc042962；调用及字段映射 ev_e59b420fcc56f52addd34e8ca61fb2ece3ab515c146d30c73fe08a04813231d9、ev_3dcb44b5fe14b9eeee73cd0a768e41e1dee32e4f8c38a2b4c2f3cee73489e700、ev_fff108084a439c1040e421fb348ee9e821b70723d6f7d312df2bae48dc814e4c。新增矩阵、强制/排除冲突、HMAC 开关和字面匹配测试。有效 SDK 配置取得、主机/账号门槛、校准时间与最终签名执行仍未接通；不是完整模式选择或服务端验收。
- 配置加载入口还读取账号 SDK 设置中的 token_guard_config，并处理 overrides（ev_f7833db7a4f98274b452205d3e3bdd2771d8b26e63db54bf0c65666c46ddda46）。不能直接把缺省 APK 常量或未处理覆盖的配置当作当前生效规则。新增组件后 91 项本地测试、严格 Clippy 通过。

- 继续核对有效配置的组合：X.1YZm.LJII 在缺少下发数组时恢复构造器缺省集合；显式 [] 则保持空集合。LIZLLL 在没有覆盖数组时合并附加集合与下发/缺省集合，有覆盖数组时只使用覆盖集合；显式空覆盖可清空全部。证据 ev_ffc57e30600673a23399928d258634c1d82c1777efeb7cefbf6a72037a6bd62f、ev_a9fba0f71c91676c659d12612a603067e86a52639811ecb61e22de6983bdea9b、ev_150da478cf456f8b31031f698e5243b2c0db2b0bfe45872bf7a623412077ad68。数组项通过 JSONArray.optString 读取并丢弃空字符串；尚未在 Rust 中假定其非字符串转换语义。
- overrides 不能统一递归合并到 token_guard_config：当前加载器对 TEE/REE 的普通包含和排除数组直接调用 LJII；对 symmetric_encryption_prefix_path 使用支持覆盖的 LJI；ree_force_sign_paths 的 LJI 调用甚至把普通下发对象参数设为 null，仅传覆盖对象。标量包装器另有两级显式值及构造器缺省值的选择顺序，仍需继续核对各级赋值。路径组件只接收已经解析的有效集合，不擅自实现通用 JSON 覆盖器。
- 配置存储入口已定位：X.1Yov.LIZJ 优先返回进程中的 JSONObject 缓存；缺少缓存时，通过 KevaSpAopHook 打开 account_sdk_settings_sp，读取 account_sdk_settings 字符串并解析（ev_d0241054a88571a51a030e079c59fedcf691ff8ffda6aceed47cd8df09f13242）。磁盘旧 XML 不足以证明当前内存设置，手机恢复后仍须校验实际存储/缓存的一致性。此轮没有访问应用数据或发请求。
- 布尔层级随后核对完成：LJ 把存在且非 null 的 overrides 值放入最高优先级 LIZJ；否则清除该覆盖。基础 JSON 经 optBoolean(key, 构造器缺省值) 更新 LIZ；LIZ() 在无最高覆盖时先取进程内 LIZIZ 显式值，再取基础 LIZ。不能简单写成 override.as_bool().or(base)，因为 SDK 对无效但非 null 的覆盖会采用构造器缺省值，且仍保留覆盖优先级。证据 ev_fddd5c1811fbc0a5bfbd660fd3359915d7990431247e9513a3aaf899ee2749c0、ev_ac05fa5f0c880b66c7359cb4e53e54d181184d1b513782948d3d408c93929ef3。尚未声称已复现 Android JSON 类型转换。

- 新增 guard::settings::BooleanSetting，显式传入已核对的 SDK 构造器缺省值，保留覆盖值 > 进程内显式值 > 基础值的优先级。基础对象缺失保留旧基础值；对象存在但键缺失恢复缺省；覆盖缺失/null 清除旧覆盖，无效但非 null 的覆盖仍以最高优先级采用构造器缺省。构造器对应关系由 ev_f761dff7c7d10c44d395e4aac72c87b3a17d23c66d064f4ec01deb661eabb0e0 补证。
- 布尔转换对照 Android 16 libcore JSON.toBoolean：布尔值或 String.equalsIgnoreCase 的 true/false，既不 trim，也不把数字当真值。使用公开固定输入在主机 Java 核对大小写、空格、数字字符串及 Unicode long-s 边界；不是手机端执行验收。来源 https://android.googlesource.com/platform/libcore/+/refs/heads/android16-release/json/src/main/java/org/json/JSON.java 。新增转换、覆盖优先级、对象/键缺失和清除旧覆盖测试，共 94 项本地测试通过。有效 SDK 配置取得、路径数组解析和最终请求签名仍未接通，未扩大生产签名行为。

- 新增 ArraySetting，对应已核对的 X.1YZm 配置层：缺少下发数组恢复构造器缺省；显式 [] 清空基础层但保留附加集合；覆盖数组替代所有层，空覆盖清空全部，移除覆盖恢复当前基础/附加集合。只在 SDK 支持覆盖的加载位置调用覆盖更新；不通用合并所有 JSON 字段。输入是已经完成 SDK optString 转换的字符串数组，原始 JSON 非字符串转换仍待接通。
- 数组层去重并保留非空规则的空白、大小写与百分号编码，输出排序仅用于确定性；路径判断仍是集合成员/字面前缀匹配。新增缺省/下发/空覆盖/撤销覆盖、去重与编码保留测试，并将布尔和数组结果送入 GuardPolicy 做组合验证：HMAC 开关和路径变化不会抹去已有 TEE 请求。98 项本地测试通过。这不是实时配置导入或完整请求签名的验收。

- 离线定位到 Cronet addSecurityFactor → 应用提供者 → metasec 原生桥的独立链路，输入为 URL 与处理后的请求头，并非 frameSign 的调用契约。两个桥的原生命令号、头名称归一化及首值提取已核对，具体原生输出和算法尚未验收；见 [安卓网络安全因子](android-network-signing.md)。本轮没有连接手机或调用原生桥对真实账号签名，ADB 当前没有已授权设备。

- 网络安全因子初始化继续定位到 b3 Java 分发入口的 0x2000001 → a3.LIZIZ → h5.LIZ(long)，后者保存上下文并注册 g5；另一分支操作 l2。已识别公开 APK 中的 AArch64 metasec 库及 JNI_OnLoad，但未复现上下文初始化、注册方法和原生算法，也未保存或复用真实句柄。证据已补入安卓网络安全因子记录；没有访问手机或发签名请求。

- JNI_OnLoad 原生静态调查补齐一处间接调用参数计算：XOR 辅助函数解出 5256，首参数地址算得 0x2a8eec，传给 0x29e764。尚未找到 JNI 注册表，不把该地址当作已识别的签名函数或虚拟机入口；未执行或修改 APK 原生库。细节及证据见安卓网络安全因子记录。

- 有界离线 AArch64 仿真已在虚构单线程/Java 环境中通过 17 项库初始化，并捕获 RegisterNatives 参数：ms/bd/c/k3.a，描述符 (IIJLjava/lang/String;Ljava/lang/Object;)Ljava/lang/Object;，原生 RVA 0x29b2b4。捕获后立即停止；尚未完成 JNI_OnLoad、执行具体签名命令或验证服务端。原库未打补丁，探针不使用账号材料、不执行宿主系统调用；见安卓网络安全因子记录中的输入哈希与边界。

- 恢复后重新拉取安装包，SHA-256 与原先固定 40.8.0 APK 一致。/tmp/re 下的历史研究临时文件已消失；后续公开 APK/DEX/ELF 和新研究账本放入 Git 忽略的 target/research/metasec，不将临时原型存在视为可复现证据。历史离线仿真记录保留为历史结果，当前未恢复其脚本。
- 新增非空 Token 标签的实机验收：首次隔离导入因不支持的 Provider 行格式失败，随后仅检查输出结构确认一行及预期字段，未打印 Token/签名/跟踪内容；第二次导入通过，保存会话包含非空 tokenTlbTag，权限 0600。随后将 adb 替换为会记录调用并失败的临时 shim，显式 direct 传输的 auth status 验证成功且 shim 未被调用。隔离会话自动删除，默认配置未改。首次格式失败原因仍待定位；不据此次成功宣称冷启动稳定，或证明搜索、直播签名已通过。

- 新增 Rust metasec::decode_constant，按公开 APK 的常量掩码及 7 个历史原生样本重建零整数/long 参数的常量 payload 变换；测试覆盖跨十字节周期与非法输入，100 项本地测试通过。此模块未接入在线请求，也不提供上下文初始化或签名。配置 builder 的实际引用传递、appID、动态 Map、tk_key 及模式选择已继续核对；SDK 版本字符串仅为静态推导，原生复核待完成。

- 新增 Metasec 配置编码器及公开假数据实机 Java 序列化对照。默认 17 项数组与配置样本 JSON 字节均匹配，覆盖 Java trim、键/值类型、整型边界和转义；102 项测试通过。fixture 使用独立 app_process、固定 APK 哈希及版本占位类，只执行实际 APK 的纯 Java 编码器，没有调用原生 SDK 初始化、账号接口或直播写操作，临时 DEX 已清理。该编码器不代表已获得运行时签名上下文。

- 按用户要求通过 gh 检索同版本开源研究，定位到 ShuaiLeiLu/DY_luckybag 中与本项目相同 SHA-256 的 Metasec 库及已恢复部件。新增 Rust metasec::digest、speck、helios：Medusa 64 轮压缩、非标准填充、增量摘要与尾部两分支，Speck-128/256 密钥扩展、分组加密和 PKCS#7，以及显式输入 SDK 上下文/秘密材料的 Helios 值封装。摘要、尾部和 Helios 固定值由公开实现用合成输入生成；Speck 另通过算法作者公开向量。参考仓库未声明许可证，未复制其源码文件，只移植算法事实和公开参数。新增 RustCrypto md-5 用于 Helios 的标准 MD5。108 项本地测试通过；尚未恢复完整 SDK 上下文、Medusa 状态位、其余安全头或服务端验收，未接入生产请求。

- 使用同一公开仓库的提取器对本地固定库重新静态提取，10 个模块全部恢复且无未解析 IR；仅在 target/research 隔离环境安装 Capstone/pyelftools，未加载原生库或访问手机。随后按同版本官方抓帧和 SsInterceptor 证据，为 aweme.snssdk.com 的独立 Token 验证与通用 GET 增加 x-ss-req-ticket 毫秒时间戳，每次传输重试重新取时；Webcast 固定权限端点不变。109 项本地测试通过。这只是已核对传输头，不是完整签名或新增实机验收。

- 继续通过 gh 找到许可兼容的 sunnysky123/hongguo-web pure-sign.js（MIT 仓库，算法源头标注 GPL-3.0），以其公开实现生成黄金向量，未采用另一份 PolyForm 非商业许可代码。新增 metasec::field13，完整实现 SM3(query)、MD5(body)/零摘要、分支选择、重量级 112 轮路径、36 字节直通路径及变种 MD5 路径。9 组合成向量覆盖全部三个分支，输出逐字节一致；新增 sm3 依赖，112 项本地测试通过。该字段尚未组装成完整 Medusa，也未接入生产请求。真实 auth status 显示当前保存登录未获服务端确认，本轮未进行已认证搜索验收。

- 继续从同一许可兼容实现提取 Gorgon 与 Medusa 外层向量。新增 metasec::gorgon，query/body 摘要、显式 SDK 版本字、RC4 变体和位处理均匹配 GET/POST 固定输出；新增签名 Protobuf 编码器，覆盖字段顺序、ZigZag、浮点和嵌套消息；新增 metasec::envelope 的 xmxor、SM3 keyHash 与短输入错误。115 项本地测试通过。目标 40.8 的 SDK 版本字、消息字段和 AesV3 封装仍须分别核对，未接入生产请求。

- 新增 metasec::aes_v3 与 seal_v3，完成四种 Khronos 变体的密钥扩展、分组加密、CBC、稀疏位回填，以及消息 xmxor、逆序掩码、校验字段和 Base64 外层。公开 S-box 仅作为算法常量 fixture 保存。四个变体均通过扩展密钥、单分组及完整封装向量；对拍定位并修复了列掩码索引方向。117 项本地测试通过。通用 v3 外层已完成，但目标 40.8 的消息字段、许可和 SDK 会话仍未确定，未接入生产请求。

- 通过 gh 找到 djskncxm/DuckDroid2 的抖音 40.8 dy7_case：同版本原生仿真已完成配置注册和 `libmetasec_ml.so + 0x2e01c0` 七头入口，公开记录已跑通搜索、详情、评论和作品。该项目为 GPL-2.0-only，不并入源码，改走独立进程的公开 Unix socket 协议。新增显式 `DOUYIN_NATIVE_TRANSPORT=duckemu` 与 `DOUYIN_DUCKEMU_SOCKET`，Rust 客户端提交最终 URL、CRLF 请求头及同一毫秒票据，校验七组返回头并固定实例；只允许原生 API 主机，不回退手机，默认 direct 不变。模拟 socket 验证请求帧与实例保持，119 项本地测试通过。真实仿真服务和业务接口尚待本项目验收。

- 真实 DuckDroid2 验收已完成。下载并校验官方 LineageOS 21 包（SHA-256 `996d4fc28d495ac0fb831d65e9de98f7fd2749d78c72c9664808623b605f7ee6`），提取系统库后独立 dy7_case 返回全部七组头。通过本地 dy7.sock 与项目 duckemu 传输，实际搜索保存 1 条、评论 2 条、单作品 1 条、用户作品 2 条；使用 target 下隔离公开参考身份，默认配置未改，临时服务关闭。过程中允许匿名公开 Cookie，搜索改为 POST `/aweme/v2/search_feed/` 并计算精确正文 MD5，加入动态 ts/_rticket 和设备可选 channel/ac/resolution/dpi/cdid，启用 gzip，严格处理 JSON 尾附数据。122 项本地测试通过。此结果不证明纯 Rust 签名、其他接口或直播已完成。

- 继续真实 DuckDroid2 验收，新增 feed、music、hashtag、mix 各保存 1 条。话题实际响应使用 `mix_list[*].aweme_info`，已接入该列表与展平；分页允许最多连续 10 个空页但有新游标，重复游标仍拒绝，123 项本地测试通过。收藏、关注和粉丝在当前公开参考身份下分别返回 3002279、8 或空响应，按需要有效登录态处理。公开 live info 在当前网络对多个房间均返回空响应，未记为通过；仅允许该固定路径的仿真签名和空 Cookie 查询，直播权限仍保持 direct。

- 继续验收评论 `--with-replies --reply-limit 1 --format chatml-jsonl`，实际导出主评论和一条真实回复及用户元数据。使用搜索结果的公开封面 URL 完成 CDN 下载，文件为 ISO Media；`stats` 和 `insights` 离线命令均可执行。临时签名服务关闭，未修改默认登录配置。

- 新增 `tools/prepare-duckemu.sh`，自动克隆 DuckDroid2、构建、下载并校验官方 ROM、提取系统库，已有文件和构建会复用。已在现有 DuckDroid2 与 ROM 上实际执行成功，配合 start-duckemu.sh 完成一键准备和启动。

- 新增 MCP 原生只读工具 `native_search`、`native_aweme`、`native_comments`、`native_comment_replies`、`native_user_posts`，复用 NativeClient 且单页数量限制为 1..=20。通过真实 stdio 分别调用成功，返回搜索结果、作品详情、评论、回复和用户作品；124 项本地测试通过。

- 新增网页 Cookie 后端 `DOUYIN_NATIVE_TRANSPORT=web`，实现主站 a_bogus（page_id=6241、aid=6383）、msToken、verifyFp、正文 MD5 和 cookie.json 独立加载；没有原生 session.json 时也可发起公开采集。主站固定 Python 向量逐字节一致，搜索和评论各保存 1 条，作品详情仍返回空响应，用户作品仍需要原生账号上下文。126 项本地测试通过。`DOUYIN_WEB_HOST` 仅允许 www.douyin.com 或 aweme.snssdk.com。

- 网页模式评论 `--with-replies --reply-limit 1 --format chatml-jsonl` 也已实测通过，主评论与回复均可导出。作品详情与 X-Bogus 对照仍为空响应，未把多余签名参数保留在生产路径。
- 新增网页传输本地 HTTP 端到端测试，核对最终 query 中的 a_bogus/msToken/verifyFp、表单原文和 JSON 业务状态解析，128 项测试通过。
- 修复网页 GET 查询参数丢失，并按公开 secsdk-strategy v1.0.40 实现作品详情的 `/aweme/v1/web/aweme/detail/` 路由、浏览器公共参数、URLSearchParams 规范化及 `uifid`/秒级 `timestamp`/`x-secsdk-web-signature`；WebSign 固定公开向量与本地端到端请求通过。当前旧 Cookie 不含有效 UIFID，真实详情验收明确提示更新 Cookie，未再发送必然 403 的请求；搜索回归仍保存 1 条。相同 WebSign 路由已覆盖 CLI 现有的用户作品、喜欢、收藏、合集作品、音乐作品和推荐 Feed 浏览器端点；这些接口的在线验收同样等待有效 UIFID。网页公开用户作品不再读取原生账号或强制请求公开资料；无资料交叉验证的空作品列表会明确失败。真实命令已到达作品列表的 UIFID 校验，不再提前失败。网页 `live info` 已接入 `live.douyin.com/webcast/room/web/enter/` 的短参数集，区分 web_rid 与场次 live_id，并解析 `data.data[0]`/`data.user`；真实请求仍为空响应，在线验收未通过。132 项本地测试通过。
- 根据公开匿名访客流程实现纯 HTTP GET 直播首页获取 `ttwid`/`UIFID_TEMP`，只合并匿名 Cookie 并安全保存；登录 Cookie 缺少 UIFID 时拒绝混入另一游客身份。受保护请求补齐 `x-tt-argus`，`s_v_web_id` 存在时同时绑定 verifyFp/fp，不存在时不伪造配对指纹。真实网页作品详情和用户作品各保存 1 条，公开已结束直播间返回 status=4、独立 web_rid/live_id 与主播公开资料，搜索回归保存 1 条。133 项本地测试通过。
