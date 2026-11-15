# douyin-cli

用命令行操作抖音，Rust 写的。

这个项目正在重写。我要的是模拟安卓端：手机上能干什么，命令行也能干什么。手机已经登录了，连上 ADB，执行 `auth --adb` 导入，之后断开手机也能用。登录态和设备标识一起处理，不能随便拼一套参数冒充手机。

直播也要支持，用导入的手机身份开播。OBS 两个方向都要有：OBS 往抖音推流，抖音直播画面送进 OBS。这里说的是开发目标，还没全部做完。全部做完、实机验证通过，再发一个大版本。

**重写还没完成，尚未发布新版。当前源码已经跑通这些：**

- `auth --adb` 导入原生 Token，核对账号、设备 ID、安装 ID 和应用版本；导入过程中标识变化就拒绝保存。
- 导入后断开手机、不带 WebView Cookie，账号验证、个人资料、关注列表和直播配置查询都已实测通过；直播配置查询也跑过实际 MCP 调用。
- 本地 123 项 Rust 测试、Clippy、release 构建和打包校验通过。OBS 目前只验过本地联动，真实推流和直播画面还没验收。

搜索、完整签名、手机身份开播，以及手机端其余功能还在做。下面写的是当前源码的用法，不能把它当成已发布版本的功能清单。

## 先跑起来

需要 Rust 1.100+、ADB，以及一台已经登录抖音的 Android 手机。目前导入登录态需要手机 root。源码构建还需要 C 编译器和 libclang，首次构建会下载 Frida SDK 开发包。

```bash
git clone https://github.com/LIghtJUNction/douyin.git
cd douyin
cargo install --path . --locked
```

看看设备有没有连上：

```bash
adb devices
douyin device
```

## 从手机登录

```bash
douyin auth --adb
```

读取手机里的抖音登录态，同时核对应用的设备 ID、安装 ID 和版本信息。API、APK 和更新版本码分别从已安装应用读取。服务端确认账号后才保存。

抖音 40.8.0 优先导入原生 Token，同时保留服务端给出的非空 Token 标签，不依赖 WebView Cookie 确认账号。带标签导入和屏蔽 ADB 后的账号验证已实测通过。导入后，`auth status` 和基础请求可以脱离手机运行。其他版本保留已有的 Cookie 登录方式。

连了多台手机就指定一台：

```bash
douyin auth --adb --serial 你的设备序列号
```

检查登录状态：

```bash
douyin auth status
```

只检查本地配置，不联网：

```bash
douyin auth status --offline
```

`--offline` 不代表服务端确认了登录。手机上切换了账号，就重新执行 `douyin auth --adb`。

删除电脑上保存的登录态：

```bash
douyin auth logout
```

这个命令不会把手机上的抖音退出登录。

## 读取资料和关注列表

```bash
douyin me
douyin -t following -l 20 --no-download -p ./downloads
```

这两个流程已验证导入后不带 Cookie、禁用 ADB 也能直接请求。采集入口沿用 `-u`、`-t`、`-l`，但其他类型还在验证，帮助里列出了命令不代表每个接口都已经跑通。

默认直接使用电脑上保存的会话请求，不会因为手机连接着就自动调用手机。独立签名还没补全，部分接口仍会触发风控；默认独立请求不代表全功能独立运行已经完成。

保留了一个 `phone` 协议调试通道，通过应用自己的网络客户端验证请求。它需要手机连接，会打开抖音并核对账号和设备。首次使用会下载并校验支持文件，电脑端不需要 Python 或 Node。

```bash
DOUYIN_NATIVE_TRANSPORT=phone douyin -u "关键词" -t search -l 3 --no-download
DOUYIN_NATIVE_TRANSPORT=direct douyin me
```

另外提供显式的 `duckemu` 传输，通过本机 DuckDroid2 的 `dy7.sock` 让官方 40.8 ARM64 库在电脑仿真环境生成七组签名头，不需要手机参与请求：

```bash
DOUYIN_NATIVE_TRANSPORT=duckemu douyin -u "关键词" -t search -l 3 --no-download
```

该后端需要另外安装 DuckDroid2、ROM 和匹配的本地签名服务，可用 `DOUYIN_DUCKEMU_SOCKET` 覆盖默认 `/tmp/duckemu-dy7/dy7.sock`。它不会自动启动或回退到手机，也只允许抖音原生 API 主机。构建 DuckDroid2 后，用 `DUCKEMU_ROOT=/path/to/DuckDroid2 DUCKEMU_ROM_SYMBOLS=/path/to/ROM/symbols tools/start-duckemu.sh` 启动；脚本会拒绝哈希不匹配的目标库。模拟 socket 的帧协议和实例保持测试已通过；真实仿真服务已验收搜索、评论及楼中楼回复、单作品详情、用户作品、feed、音乐、话题和合集；CDN 下载与离线统计也已跑通。

手机通道原型读取过作品、评论和搜索结果。Rust CLI 的 `me` 在实机跑通过，但后续附加和搜索仍不稳定。它只用于协议研究，不能算完整独立运行已经完成。

作品采集会合并已有归档：新采到的作品更新数据，历史作品保留。文件按目标账号区分；`-l` 限制的是本次采集数量，合并后的归档可能更多。分页信息也只描述本次采集。

## 评论和回复

```bash
douyin comment "作品 ID 或分享链接" --limit 100 --with-replies --format raw -o comments.json
douyin comment "作品 ID 或分享链接" --with-replies --format chatml-jsonl -o comments.jsonl
```

主评论和楼中楼分别分页，回复数量用 `--reply-limit` 控制。ChatML 导出可以筛选点赞数，也能保留没有回复的评论。这部分代码和导出测试已完成，完整联网采集还在验收。

## 官方授权和 MCP

开放平台应用配置通过 `DOUYIN_CLIENT_KEY`、`DOUYIN_CLIENT_SECRET` 提供。回调地址需要先在开放平台配置好。

```bash
douyin auth login --scope user_info --listen --callback-port 8787
douyin auth status --oauth
douyin auth refresh
douyin api userinfo
douyin mcp
```

OpenAPI 使用应用授权，ADB 导入使用手机登录态。评论回复、私信发送等写操作需要 `--yes`；MCP 调用需要 `confirmed=true`。

OAuth、OpenAPI 已重写并通过本地验证。MCP 接入了 [官方 Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)，新旧协议都跑过真实 stdio 进程测试。开放平台的实际授权、权限和业务接口还要继续验证。

## 网页 Cookie

```bash
douyin auth cookie-login --cookie-file ./private-cookie.txt
douyin auth cookie-status --offline
douyin auth cookie-status
douyin auth cookie-logout
```

也支持 `--cookie` 和 `DOUYIN_COOKIE`。保存时只检查格式，`--offline` 也只检查格式；联网检查成功才确认网页登录态。Cookie 管理已经补回，网页采集后端还在接。

## 直播配置和 OBS 联动

`douyin live permissions` 查询当前账号的直播配置。CLI 和 MCP 都已实测导入后独立运行，不需要手机连接或 WebView Cookie；会先核对当前账号，只输出认证类型、回放/录制/商城等设置开关、预约直播设置和配置栏目。这个命令不创建直播间，也不把接口返回的配置当作开播成功。MCP 对应工具是 `native_live_permissions`。

`douyin live info "房间 ID"` 已接入安卓房间查询代码，只输出房间摘要，不打印推拉流地址。摘要中的 `sdk_stream_type` 按已核对的 SDK 规则区分视频、音频、录屏、第三方和媒体房间；字段不完整时为 `null`，这个分类不等同于手机身份开播证明。当前独立请求仍受签名和风控限制，尚未通过真实房间验收。

OBS 28+ 自带 WebSocket，在 OBS 的工具菜单里启用。默认连接本机 `4455` 端口，密码用 `DOUYIN_OBS_PASSWORD` 环境变量提供。

```bash
douyin live obs status
douyin live obs configure-push --stream-file ./private-stream.json --yes
douyin live obs add-source --url-file ./private-pull-url.txt --scene "场景名" --yes
douyin live obs source-status --name "douyin-live"
douyin live obs stop --yes
```

推流配置文件是 `{"server":"rtmps://推流主机/路径/","key":"推流密钥"}`。也支持 `{"rtmp_push_url":"完整 RTMP 或 RTMPS 推流地址"}`，会按原始字节拆分，保留签名参数；两种格式不能混用。拉流文件只放完整的直播播放地址。文件里有私密地址和密钥，自己保管，不要提交到仓库。

`configure-push` 默认只配置，增加 `--start` 才启动 OBS 推流；已经推流时拒绝替换目标。`add-source` 在指定场景新建媒体源，重名会报错。`source-status` 只读查询媒体源的播放状态和进度，创建成功不代表已经解码出画面。`stop` 停止 OBS 推流，抖音直播间的结束流程还要单独实现。

这部分已用隔离的 OBS 33.0.1 验证连接、密码认证、配置目标但不启动、创建媒体源和空闲停止。新增 `source-status` 已用本地测试视频确认返回 PLAYING、时长和播放进度；查询不存在的源会报错。尚未验证真实抖音直播画面和推流。安卓身份开播、独立签名、自动取得推拉流地址仍在开发；目前需要已有地址，不能算手机端开播已经实现。

## 下载媒体链接

```bash
douyin download "抖音 CDN 的 HTTPS 媒体链接" -o ./media.jpg
```

下载不会把登录 Cookie 发给媒体主机。文件完整写入后才提交，已有的非空文件会跳过。

## 离线分析

已有的评论、作品数据可以直接拿来分析，不用联网。

```bash
douyin insights comments.json --top 20 --min-count 2
douyin insights comments.jsonl --format markdown -o insights.md
douyin stats works.json --sort score --top 10
douyin stats works.jsonl --author 作者名 --format markdown
```

`insights` 看词频、热梗候选和需求关键词。`stats` 统计点赞、评论、收藏、分享，支持旧版导出和原始接口数据。缺失的数据会标出来，不会当成真的零。

这些是规则分析，综合分也不是互动率，别拿它代替实际判断。

## 接下来补什么

搜索、作品列表和评论的联网流程还在验证，网页 Cookie 采集后端也要补齐。推荐、互动、发布、草稿、直播、消息和账号设置都在范围内，不能只做到命令能解析。

这些还没有全部实现，不能拿一个通用请求命令就说“所有功能都支持”。具体进度和验收要求放在 [重建记录](docs/rebuild.md)。

如果你要用已发布的旧版：

```bash
cargo install douyin-cli --locked
```

也可以在 Arch Linux 上安装：

```bash
paru -S douyin-cli
```

发布渠道里的版本和当前开发分支有区别。旧版命令看 [旧版使用说明](https://github.com/LIghtJUNction/douyin/blob/6fd6055/USAGE.md)，新版以实际帮助为准：

```bash
douyin --help
douyin auth --help
```

## 配置放哪

登录态保存在 `douyin-cli` 配置目录下的 `session.json`。默认位置：

- Linux / macOS：`~/.config/douyin-cli/session.json`，设置了 `XDG_CONFIG_HOME` 时使用该目录。
- Windows：`%APPDATA%\douyin-cli\session.json`。
- 设置 `DOUYIN_HOME` 可以单独指定目录，测试时也用这个隔离配置。

同一目录下，`oauth.json` 保存开放平台授权，`cookie.json` 保存网页 Cookie。旧版 `config/settings.json` 中的 OAuth 和 Cookie 可以兼容读取。`auth logout` 清除设备登录与 OAuth，网页 Cookie 用 `auth cookie-logout` 单独清除。

里面有登录凭据，别上传到仓库，也别贴到 Issue 里。

## 本地验证

```bash
cargo fmt --check
cargo test --locked --offline
cargo clippy --locked --offline --all-targets -- -D warnings
cargo build --release --locked --offline
cargo package --locked --offline
```

测试通过和手机上能用是两回事，都要验。

修改应用内通道脚本时，还要用开发机上的 Node 跑隔离测试，检查账号切换、设备变化、路径和参数是否会在请求前被拦下：

```bash
node tools/test-phone-agent.mjs
```

手机元数据辅助程序的源码和编译产物都在 `assets/device-metadata/`。正常安装和运行直接使用包内产物。修改这部分源码时，需要 JDK、Android SDK platform 36 和 build-tools 35.0.0：

```bash
bash tools/build-device-helper.sh
bash tools/build-device-helper.sh --check
```

## License

[AGPL-3.0-or-later](LICENSE)
