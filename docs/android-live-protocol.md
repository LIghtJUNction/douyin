# 安卓开播协议调查

这份记录区分 APK 静态事实与服务端验收。当前没有调用创建、续播或结束直播接口，也未验证服务端将 CLI 识别为安卓主播。

依据：本机安装的 40.8.0 / APK versionCode 400801，APK SHA-256 `a4e0682ff174d73dbc00d77622037090ee88f164fc0f9b62c324a1234e64ba57`。CraftAnything 定位 classes13.dex 中的 `com.bytedance.android.live.broadcast.preview.api.StartLiveApi`；Android SDK apkanalyzer 进一步核对 Retrofit 参数注解。没有将反编译代码或真实账号数据加入仓库。

| 方法 | HTTP / 路径 | 已确认参数形式 |
| --- | --- | --- |
| getUserPermissionWithId | GET `/webcast/user/permission/` | query: sec_anchor_id, micro_app_entrance_visibility |
| getPrePreviewRoomCreateInfo / Pb | GET `/webcast/room/pre_create_info/` | query: quick_live_source, quick_live_biz_params |
| getPreviewRoomCreateInfo | GET `/webcast/room/create_info/` | query: platform, last_related_circle_id, enable_preview_stream, preview_resolutionkey, quick_live_source, quick_live_biz_params, scene |
| createRoom / createRoomWithPb | POST `/webcast/room/create/` | FormUrlEncoded + FieldMap，HashMap<String,String> |
| continueRoom / continueRoomWithPb | GET `/webcast/room/continue/` | query: resolution_key；一个 Pb 重载另有 is_anchor_exit_new_connection |

`continue` 即使使用 GET，也可能恢复直播状态，不能归入只读验收。预览接口的参数名不证明具体值、默认值或权限条件；尤其不能猜测 platform 的安卓取值。接口路径也不证明最终主机、完整请求签名或调用顺序。

下一步从 createRoomWithPb 的调用者追踪表单构建，再核对房间所属账号、设备绑定、响应解码、推流信息与续期。需要继续定位心跳和正常结束流程。不得以人工填写 RTMP 地址、OBS 配置成功或静态接口存在替代安卓开播验收。

本地研究证据：StartLiveApi 类清单 `ev_63b21563c5ee980c8c329967dc14cf64b27b4e1c2b6242288735de2181982ae7`；classes13.dex SHA-256 `8bf6f8bc53df350946b1439b4904638f19dab8f948f1754d6ed330114434c083`。完整研究账本只保存公开 APK 证据。

## 创建表单调用链

直接字节码引用已定位到：

`X.0VaJ.LIZ(0VaU)` → `X.0VaJ.LIZLLL(0VaU)` 生成 HashMap → `StartLiveViewModel.EA` → `StartLiveViewModelHelper.EA` → `StartLiveApi.createRoomWithPb`。

`X.0VaJ.LIZLLL` 共 2154 条指令，包含按直播类型、设置和预览状态选择的字段。已看到 title、cover_uri、game、tags、gen_replay、record_screen、gift_auth、orientation、live_audio、screen_shot、third_party、live_room_mode 等；这份列表不代表所有字段必填，也没有证明这些开关可互相替代。标题、封面等先进入 `X.0Vaj` 参数构造器，还需核对其空值处理。OBS 媒体接入不能仅凭 third_party 字段名推断手机身份。

调用链证据分别为 `ev_327eb05145a9d344baf6ff3c14792fa8470a232d153bbee27fc2edfcd9433c0f`、`ev_ff6e1335e00dea3f60408014d8395fffd8c666b37f313558326290eb9cef310a`、`ev_43165a51e4fd1d875801d9c429370368fec0689a2085b1a2c6b7dc1c0e1cf743`、`ev_afc0b9fe7f1b63a636a14c64bf81e53a06e4055c955f5e479a8b432f8813ad4d`、`ev_bb97eb56904056059b402d366c90be5a94cb4ccbcf098080322f2b2901c13182`。直接引用搜索不覆盖反射、JNI 和运行时加载代码。

## 参数空值与类型分支

`X.0Vaj.LIZIZ(key,value)` 先执行 Android `TextUtils.isEmpty(value)`，为空则直接返回，非空才写入 HashMap。因此经过这个辅助方法的 null 和空字符串会被省略；没有看到 trim 操作，不能将纯空格等同于空值。后续直接 HashMap.put 的字段不自动具有这个规则。证据：`ev_0fb87e087d8e439a4995983d8cda2260c3d69d033a7379e7abeb76fb2c0d5b7a`。

创建表单中的直播类型比较链已确认：ACQUAINTANCE 添加 acquaintance=1，AUDIO 添加 live_audio=1，THIRD_PARTY 添加 third_party=1，SCREEN_RECORD 添加 screen_shot=1。这些是分别匹配的分支，不能为了 OBS 联动同时盲目添加全部标志。THIRD_PARTY 分支还依据一个 Long 状态决定是否写 auto_cover=1 或 2；其状态来源与产品含义尚未确定。以上只是本版本静态分支事实，不是对应账号有权限开播的证明，也不代表普通摄像头直播需要第三方标志。

## 主播状态上报

已通过 DEX 注解反查并用 apkanalyzer 核对 `com.bytedance.android.live.pushstream.StatusApi`：sendStatus 与 sendStatusWithExtra 都使用 GET `/webcast/room/ping/anchor/`。明确的 query 参数为 room_id、status、stream_id、reason_no、source、prop_id；WithExtra 另带 QueryMap，并返回 PingAnchorExtra。该接口具有状态上报语义，不能当作只读探测调用。

在当前 classes13.dex 中找到 7 个直接调用方法，集中于 X.0VXL、X.0VXM、X.0VXD。X.0VXL.LJ 固定传 status=3、reason_no=0、source 为空；LJII 固定传 status=4、source=anchor_ping，reason_no 来自调用参数，两者使用当前 room/stream 标识及 current_sticker_id。尚未根据上层调用与服务端结果确认这些数值分别代表结束、暂停或其它状态，因此不能直接写成 CLI stop/heartbeat 常量。上报周期、失败重试与正常结束链路仍待确认。

引用清单证据 `ev_9223fc9ae6789c2bb4ce96e03c565212c26e9b74c749769ff36a98d981e404a6`；常量追踪证据 `ev_ec15c6d90e5752b3a4132abe693e524ec9c2bbc1a2a082945f35382b769bd0ad`、`ev_ac68d8d49d04eb54b3df30f746f967e711969f4615d931f3e362da679e225acb`。未请求任何真实房间。

## 区分两套状态上报器

进一步追踪发现，`X.0VXL` 的构造器明确命名 Handler 为 PushStreamReporterForAnchorInAudienceRoom；其无效周期参数回退为 5000 ms。不能把这个实现的默认周期当作主直播间协议常量。该实现 onStart 设置内部状态 2，onStop 移除定时消息、清除运行标志并上报状态 4；仍只证明客户端状态机行为，不证明房间已在服务端关闭。

另一个实现 `X.0VXM` 持有 IRoom，Handler 命名为 PushStreamReporter。它的构造器分别保存状态常量 1、2、3、4，周期由 Lazy 配置取得。onStart 将当前状态改为 2 后立即上报；onStop 移除定时消息、清除运行标志并将当前状态改为 4 后上报。定时回调重新发送当前状态，只有状态非 4 才继续安排下次回调。具体重试、配置来源和房间结束响应还需继续核对。

构造器证据 `ev_bfa90a50e9e5f11fe3c1efe636b1be048c1b2db31eb01c970a50b29dc88666d3`、`ev_fda02679c349e88f879461ffd52ac0e56530865890d86e1e4f7af00499d2a717`；主上报器调用证据 `ev_72ee5dba5ea20de4340cc4dbdcb51d602242da87a0ea264dc1e04822e0f61a4a`、`ev_af916a122c300176a1c31fe2f3f6f5229149ee18e1a314c8f4c66e268b1a01c4`、`ev_7d5b2918959fb1dc4bc0621aa6ba342f01123fcc10dd8ff477bc65f764d7d400`、`ev_6a3478961dfb34b1813762306c2ae6f2c44e48a33dd48e2fb9d27dbffcf85cd5`。此前的 3/4 常量观察应结合所属上报器理解，不能单凭相同接口路径推断其业务角色。

## 已确认状态名与结束条件

主上报器的成功回调 `X.0VXN.accept` 明确将状态常量映射为 prepare=1、play=2、pause=3、finish=4；这是 SDK 自身监控标签的直接证据，替代此前仅凭数值的猜测。finish 成功分支调用回调 wC(status,true,0,null)；仍需在实际服务端响应中确认房间已经结束。

`X.0VXM.LJFF` 的 finish 分支存在跳过请求条件：reason_no=112 直接返回；特定功能开关启用且 reason_no=115 时，也返回并记录 temporary close。普通 finish 请求实际调用的仍是 StatusApi.sendStatusWithExtra，监控字符串 /room/close/ 不能误当成其 HTTP 路径。重连的首次 play 在特定 lastPingTime/createTime 条件下还会添加 last_ping_time。

pause/finish 的 retryWhen 受配置开关控制；重试次数、间隔和错误筛选来自 X.0VXS 与 X.0VXR，不能给所有心跳统一添加无限重试。当前构造器包含次数 2、间隔 200 的缺省值，但参数单位和实际配置覆盖仍需结合重试执行器核对。

状态名证据 `ev_a4fab3f6b77e840e58a49e626e9a58ff02df075e67296d8afdf65fbab624cb17`；请求条件证据 `ev_227700a77764416f3523fe8234dbe505611999a6415e1a13c7d635c391b1544d`；重试配置构造器证据 `ev_be9bcad50b5318417076ceff42bdc1d85cfb9c40bc72e179e723912bf269f120`。


## 直播域名配置来源

40.8.0 中 `NetWorkService.getHostDomain()` 委托 `IMiddlewareHostNetwork.getHostDomain()`；抖音的 `LiveHostNetwork.getHostDomain()` 通过 `ILiveInnerService.getLiveDomain()` 取值。已检查 `LiveInnerService.getLiveDomain()`，其方法直接返回 `webcast.amemv.com`。这证明该实现的配置值，不证明某次 Retrofit 请求使用了它，也不证明 URL dispatch 后的最终主机，更不证明安卓开播身份。没有修改运行时 Token 主机白名单。

上述方法证据依次为 `ev_a9044103cef42f4ad996568f28cad6445f545b7a6293150fad654a7b7126ff9d`、`ev_1cc8a75c9de79c52cbb97811f7c792c0741e5edecdeaa7094e6ae1cf9f9afed4`、`ev_15edebb1fbb69dd4f859222d8947f4a1852901709fb0642a6e819408d3ca3c52`。后续仍需核对网络客户端创建、运行时分支和只读服务端响应。


只读接口受控对照：在真实手机导入原生 Token 后，独立主机请求上述主机的 `/webcast/user/permission/`，携带已导入的设备/安装标识、应用版本、自己的 sec_anchor_id 与 SDK 头，不带 Cookie。带 Token 时 HTTP 200、status_code=0，返回配置对象；保持其它参数及请求头不变，仅去掉 Token 后 HTTP 200、status_code=20003。curl 使用 `-q` 禁用用户配置、限制 HTTPS/响应大小/超时且不跟随重定向，凭据只通过 stdin 传入，未记录真实请求或响应字段值。这支持该接口接受原生 Token，不证明某项开播权限已获准，更不证明移动端开播分类、房间生命周期或推流已验收。随后已接入 `live permissions` 与 MCP `native_live_permissions`；只为这个固定只读路径选择 Token 认证，通用 Token 主机白名单未扩大。


产品命令实机验收：隔离导入后清空测试配置的 Cookie，将 PATH 设为不可用，实际 Rust CLI `live permissions` 退出 0，返回可识别的认证类型、3 个设置开关和配置栏目；原始 Token 未出现在输出，last_room 被排除。默认用户配置及手机 Cookie 数据库未修改。该查询先调用 me 确认保存的同一账号，再使用当前返回的 sec_uid，未调用创建、续播或结束接口。


## VIDEO 类型与最终表单

已核对 X.0Vgc 构造器和静态初始化：VIDEO 的日志类型 general、场景名 video，background=false、camera=true；THIRD_PARTY 则为 thirdparty / third_party，background=true、camera=false。这些是 SDK 元数据，不能直接当作服务端 wire 枚举。构造器与初始化证据 `ev_2a675cd3613b2ff76177906a42d7d4363350998af1e64b3af8f0a0dee1dde432`、`ev_24116fec659e81cf1d9caa1b4b96381e287b5423e4c14882b3ac939c3e8ec13d`。

创建表单 VIDEO 分支可选写 category；只有外部摄像头设置启用且 ILiveExternalCameraServiceForOther.isConnected() 为 true 等条件满足，才向预览来源业务 payload 写 webcast/broadcast_biz_type=external_camera。该分支不自行写 third_party、live_audio 或 screen_shot。不能因为接入 OBS 就模拟已连接外部摄像头。

X.0VaJ.LIZLLL 存在三处后续 putAll：调用者附加表、PreviewSourceParam.LJLLJ()、最终创建数据表，均可覆盖先前值。PreviewSourceParam 会把 JSON 来源值转成字符串，补入 payload，并保留部分未反序列化的额外字段。因此未来创建请求必须在全部合并完成后核对类型字段，不能仅验证早期 VIDEO 分支。证据 `ev_826d1a4716e85974115f6fdcb806416af738aac7e0012e4b661e2342694e48a6`、`ev_0def4c22abf6673af2960e6e5a67a884cbe04fa4e021cc2716439bf13fc5385e`。未发出创建请求。

## 权限模型中的预约设置

官方 apkanalyzer 的 Retrofit Signature 注解确认 getUserPermissionWithId 返回 PermissionResult；其 broadcast_config 对应 X.0Vws。该模型的 SerializedName 注解只有 scheduled_master_switch、scheduled_profile_switch、scheduled_notice_switch、scheduled_time、scheduled_days、scheduled_notice_switch_display、scheduled_time_text，并非是否可开播。模型清单证据 `ev_6b6c685cc1213cf1867b2903d745a07596c41c0285be16d3362777a05a10cbd8`、`ev_e0c551a33451cd9859ef74bbe157ed7fb0911c3689e18332e7c2df3132e4a23a`。CLI/MCP 已补全已知的回放、录制、商城、群聊等 scalar 设置及预约设置；保留 SDK int32 负值哨兵，不输出任意新增字段值或历史房间，不推导 canBroadcast。


## 创建提交、附加阻断与房间类型

StartLiveViewModel.EA 是短转交方法，创建 helper 最终把传入 HashMap 交给 StartLiveApi.createRoomWithPb；没有在这两个转交点看到第三方/平台字段再写入。这不是对 Retrofit 或原生网络拦截器的完整证明。证据 `ev_610516a6d941fc6c16374c1de619e1702400c06561f4588a62455117b1fef241`、`ev_b28bb49a3482a91121e463583bc28958605f0309e928e4238af7ae0109f95388`。

官方 apkanalyzer 注解确认 createRoom 返回 Observable<0b14<Room>>，而 createRoomWithPb 返回 Observable<ExtraResponse<Room,RoomCreateBlockExtra>>，两者都使用同一路径和 FormUrlEncoded FieldMap；不能从方法名字推定响应线上编码。RoomCreateBlockExtra 含实名验证、手机号绑定、直播协议/答题、风险级别等字段，也含 ticket、uniqueId 与阻断详情；这些不是房间成功数据，不得原样公开。成功回调读取 data.Room 的 roomId 和 streamIdStr，后者 getter 直接读取 stream_id_str 对应字段。证据 `ev_8ec3cc1098555e0db2720fa8354debca0133eda9321a469fa0a1e2c2afab6ddc`、`ev_38e7e39ff1a8411d851835fb5793dea42f4a11f326376ade798bf0d5055d8645`、`ev_39f5bbf350b6375772f05e809c7463dad40c58b8c6a01615a975d77f7629b414`。尚未收到真实创建响应。

Room.getStreamType 先判断 room_layout=1，再判断 layout=ROOM_TYPE_OFFICIAL_ACTIVITY（静态初始化值为 1），两者都返回 MEDIA；随后依次判断 live_type_screenshot、live_type_third_party、live_type_audio，最后返回 VIDEO。字段名已用 SerializedName 注解核对。证据 `ev_387d8a005ea295898ee846c2c3b880cf6ea6b23033c548c9bf043ffe53b0e3e8`、`ev_47f58dd83dabf63a1bae3c3a6d21c0e31143bbd3b3f391dc6797de786d8b13a4`。房间摘要已加入 sdk_stream_type；对于字段不全或类型不明的原始 JSON 保留 null，不照搬 Java 默认字段值把未知房间误判为视频。此分类仅代表客户端 SDK 的房间类型规则，不证明服务端把发送端认作安卓手机。


## 推流地址与 OBS 输入格式

classes53.dex 的 StreamUrl 字段注解分别包含 rtmp_push_url、rtmps_push_url、push_urls、rtmp_push_url_params、rtmp_pull_url 和 flv_pull_url。它们是不同字段，不能把推流参数或拉流地址当作推流主地址。类定位证据 ev_1df43a08a88da0029e9c2501da12b410694d491ae1e82756fda2f217b5ec09ab；字段注解由 apkanalyzer 对原 APK 核对。

OBS configure-push 输入现允许单独的 rtmp_push_url 字符串，限定 RTMP/RTMPS，并保持原始地址字节：仅在查询串之前寻找最后一个路径分隔符；不解码 %2F，不按签名内的斜杠拆分。空流名称、首尾空白、非法协议及与 server/key 混用均拒绝。只适配明确提供的地址，不自动选择多地址列表，不接受整个未经验证的房间响应，不声明已经取得安卓开播权限。
