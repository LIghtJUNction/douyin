# 直播操作指南

`douyin live` 提供房间创建、续播、主播状态上报、有限次心跳和 OBS 推流控制。以下流程使用当前账号的 Android 登录态；执行带 `--yes` 的命令会修改直播或 OBS 状态。

当前自动化验证覆盖命令参数、账号绑定、私密会话保存、请求路由和手机表单编码。真实创建房间、持续推流、续播与结束仍需实机验收；开播权限以当前账号和服务端结果为准。

## 1. 登录并选择传输方式

首次导入需要已登录抖音、开启 USB 调试且具备 root 授权的 Android 手机：

```bash
adb devices
douyin device
douyin auth --adb
douyin auth status
```

多台设备同时连接时，用 `douyin device --serial "设备序列号"` 检查目标设备，再用 `douyin auth --adb --serial "设备序列号"` 明确选择。更换账号、设备或应用版本后重新导入；导入失败时先处理设备标识、安装标识或版本提示，再继续开播流程。

| 传输方式 | 运行条件 | 选择方法 |
| --- | --- | --- |
| `direct` | 已导入有效的 Android 会话；运行期间无需连接手机 | 默认，也可设置 `DOUYIN_NATIVE_TRANSPORT=direct` |
| `phone` | 手机保持连接，账号、设备和应用构建与导入信息一致 | 显式设置 `DOUYIN_NATIVE_TRANSPORT=phone` |

使用手机通道时，在执行直播命令的终端设置：

```bash
export DOUYIN_NATIVE_TRANSPORT=phone
```

PowerShell 使用 `$env:DOUYIN_NATIVE_TRANSPORT = "phone"`。手机通道仅接受已核对的应用构建，当前夹具覆盖 40.8.0 和 41.1.0 对应的固定版本组合；不支持的构建会明确报错。本轮 41.1.0 设备在 Frida 附加阶段失败，尚未通过手机通道的真实网络验收，详情见第 8 节。`direct` 与 `phone` 都会验证业务响应，错误不会自动触发传输切换。

## 2. 查询权限与开播配置

先查询当前账号的权限配置、预览和开播检查结果：

```bash
douyin live permissions
douyin live create-info
douyin live create-check
```

`create-info` 使用已核对的 `platform=2`。`create-check` 支持 `--live-room-mode`、`--chat-auth`、`--share-auth` 等参数，完整选项可用 `douyin live create-check --help` 查看。返回成功说明本次查询被接受；实名、账号权限及实际创建条件仍由服务端检查。

## 3. 创建房间并交给 OBS

创建参数使用 JSON 文件。下面是 CLI 接受的最小字段示例：

```json
{
  "title": "我的直播"
}
```

将其保存为 `live-params.json`。服务端需要额外字段时，填入当前账号与直播类型已经确认的配置，例如封面或分辨率键；`third_party`、`screen_shot` 等类型开关应来自实际直播配置。

创建房间时同时保存私密会话，供后续推流和状态命令使用：

```bash
douyin live create \
  --params-file live-params.json \
  --output live-session.json \
  --yes
```

成功摘要中的 `session_saved: true` 表示会话文件已保存，`has_push_url: true` 表示文件包含可交给 OBS 的推流地址。文件同时记录账号、房间 ID 和流 ID，并保留推流地址原始签名字节。省略 `--output` 时只返回摘要，因此完整流程应保留此选项。

在 OBS 中准备好场景和音视频源，并启用本机 WebSocket 服务，默认端口为 `4455`。需要密码时，通过 `DOUYIN_OBS_PASSWORD` 提供：

```bash
douyin live obs status
douyin live obs configure-push \
  --stream-file live-session.json \
  --start \
  --yes
douyin live obs status
```

`configure-push` 默认只更新推流配置，增加 `--start` 才启动推流。自定义端口可用 `douyin live obs --port 4456 status` 这样的形式指定。若 `has_push_url` 为 `false`，先核对账号的推流权限和开播响应；此会话仍可用于主播状态命令，OBS 则需要有效推流地址。

## 4. 发送心跳与结束直播

心跳需要明确指定间隔和次数。在另一个终端运行，并保持与开播时相同的传输配置：

```bash
douyin live heartbeat \
  --session-file live-session.json \
  --interval 10 \
  --count 60 \
  --yes
```

`--interval` 范围为 1–3600 秒，从上次成功响应后开始计时；`--count` 范围为 1–10000。示例的 10 秒、60 次只是用法演示，实际值应按已确认的客户端配置选择。每次请求上报 `play`，失败即退出；Ctrl-C 停止后续心跳，正在进行的同步请求会先结束或超时。

结束时先让心跳完成或按 Ctrl-C 停止，再依次停止 OBS 和上报主播结束状态：

```bash
douyin live obs stop --yes
douyin live finish --session-file live-session.json --yes
```

心跳达到次数上限只结束心跳命令，OBS 和直播房间仍需上述结束操作。`finish` 会先上报结束状态，再回读同一房间：

| 输出字段 | 含义 |
| --- | --- |
| `request_accepted: true` | 服务端接受了主播状态上报 |
| `status_reported: "finish"` | 本次上报的是结束状态 |
| `verification: "room_info_observed"` | 已取得同一房间的可识别状态 |
| `observed_room_status` | 服务端返回的房间状态数值 |

当前 CLI 保留房间状态数值，尚未定义其“已关闭”枚举映射；最终关闭状态需结合实际房间观察确认。回读失败会输出 `verification: "room_info_unavailable"` 并以非零退出码结束。若摘要显示上报已经被接受，下一步先用 `live info` 查询房间，避免直接重复 `finish`。

## 5. 续播与单次状态上报

`continue` 会请求恢复当前账号的直播间，并写入新的私密会话。分辨率键应使用当前已确认的值：

```bash
douyin live continue \
  --resolution-key "当前分辨率键" \
  --output live-session-resumed.json \
  --yes
```

恢复后，OBS 配置、心跳和结束命令均改用新会话文件。重新配置 OBS 推流目标前，应先停止已有推流；旧会话文件可保留供核对。

需要单次上报时使用 `anchor-status`：

```bash
douyin live anchor-status \
  --session-file live-session.json \
  --status pause \
  --yes
```

可选状态为 `prepare`、`play`、`pause`、`finish`。此命令只上报状态，OBS 推流需要单独控制；需要结束后的房间回读时使用 `live finish`。`--reason-no` 和 `--source` 可用于传入已经确认的上报参数。

## 6. 查询公开房间与观众离开

原生房间查询使用创建结果中的房间 ID：

```bash
douyin live info "房间ID"
```

网页查询使用直播页面中的 `web_rid`：

```bash
DOUYIN_NATIVE_TRANSPORT=web douyin live info "网页直播间编号"
```

网页摘要中的 `web_rid` 是输入的网页编号，`live_id` 是返回的真实房间 ID。使用主播状态命令时以创建或续播保存的会话为准。

观众离开使用以下命令；主播结束流程见第 4 节：

```bash
douyin live leave --room-id "房间ID" --yes
```

## 7. 私密会话与错误恢复

会话文件包含推流凭据，应保存在自己的私密目录中。Unix 系统将文件权限设为 `0600`；输出拒绝覆盖现有文件，也不接受标准输出 `-`。状态命令会检查会话所属账号，并在线核验当前登录账号；切换账号后应使用该账号自己的直播会话。

创建或续播已被接受、但文件写入或提交失败时，错误会给出房间 ID，并在能够保留文件时给出私密恢复路径。应先查询房间状态并处理恢复文件，避免再次创建。恢复提示会区分“写入可能不完整”和“JSON 已完整写入但同步未确认”；完整且可解析的恢复文件可直接作为 `--session-file` 或 `--stream-file` 使用。

会话格式、输出字段和错误处理以 [直播实现](../src/live.rs) 为准；手机请求范围见 [固定直播路径](../assets/phone/live-paths.json)，协议依据见 [安卓直播协议记录](android-live-protocol.md)。

## 8. 本轮验收记录（2026-11-17）

本轮在已连接、已授权的 PHK110 手机上导入抖音 41.1.0 登录态。应用升级后，XML 中的旧安装 ID 与当前 Keva 记录不同；导入逻辑现沿 `.chk` 的活跃键引用读取 `.blk` 中的值，并检查稳定快照、设备 ID、账号及导入前后的安装 ID。解析器及合成数据测试见 [src/keva.rs](../src/keva.rs)。

| 检查 | 本轮结果 |
| --- | --- |
| `cargo test --locked --offline` | 180 项通过，1 项独立真机测试按项目原配置跳过 |
| `cargo fmt --all -- --check` | 通过 |
| `cargo clippy --locked --offline --all-targets -- -D warnings` | 通过 |
| `node tools/test-phone-agent.mjs` | 通过 |
| release 编译与安装 | `cargo install --path . --locked --offline --force` 成功，版本为 `douyin-cli 2026.11.17` |
| 41.1.0 真机 `auth --adb` | 原生 Token、当前设备/安装标识及在线账号校验成功 |
| `direct` 的 `live permissions` | 成功，返回 17 个可识别配置栏目 |
| `direct` 的 `live create-info` | 成功，`status_code=0`，存在数据 |
| `direct` 的 `live create-check` | 服务端返回 `10001` 和 `Service Unavailable`，CLI 以非零状态退出；底层原因尚未证实 |
| `phone` 的只读查询 | 在 Frida 附加应用进程阶段失败，尚未发送直播网络请求 |
| 本机 OBS WebSocket | 本轮连接或认证未成功，实际推流前需检查 OBS 服务配置 |

以上三个 `direct` 直播查询在禁止通过 `PATH` 启动外部工具的环境下执行，确认本轮请求由 Rust CLI 直接完成。权限配置和预览成功不能推出当前账号可创建房间。真实创建、持续推流、续播及房间结束尚未执行；新增生命周期的验证范围是实现、协议依据与自动化测试。

开播预检的错误数据包含 `message` 与 `prompts`，没有顶层 `status_msg`。CLI 现在按顺序选择短的 `status_msg`、`data.prompts` 或 `data.message`，因此实际命令会显示“错误码 10001：服务器打瞌睡了, 请稍后刷新”；仍保留错误码与非零退出，且不输出整个数据对象。

手机通道诊断确认应用未处于 cgroup 冻结状态。清理一个经核实无客户端连接的旧 CLI 孤儿服务后，单次复验仍在附加阶段失败，不能据此归因于直播权限或服务端。测试创建的临时服务、文件和 ADB 转发均已清理。
