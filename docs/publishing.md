# 视频投稿

本文说明 `publish video` 的官方 OAuth 流程。独立的 `publish app` / `publish native` 入口见 [原生创作者投稿](native-publishing.md)，其中明确记录登录前提和线上验收边界。

投稿将本地视频上传到抖音开放平台，再为当前 OAuth 授权账号创建作品。新接口要求应用已取得内容发布能力，且用户实际授予 `video.create.bind`；Android Token、网页 Cookie 和官方 OAuth 分别保存。

## 检查和投稿

先完成开放平台应用设置与用户授权：

```bash
douyin auth login --scope user_info,video.create.bind
douyin auth status --oauth --offline
```

`requestedScopes` 表示请求的权限；只有 `scopesVerified: true` 时，`scopes` 才表示 token 响应确认的权限。发起重新授权会保留当前账号已有权限，直到新的授权成功。旧版配置的权限需要刷新 token 或重新授权后确认。

先检查文件、标题、封面时间、账号和本地授权状态：

```bash
douyin publish video ./video.mp4 --title "标题 #话题" --dry-run
```

`--dry-run` 不访问网络，不上传视频，也不创建投稿记录。检查通过表示本地条件满足；服务端还会检查应用能力、当前 token、配额和内容。标题最多 1000 个 Unicode 字符；封面秒数须有限且非负。显式 `--open-id` 必须与保存的 OAuth 账号一致。

```bash
# 上传并创建作品
douyin publish video ./video.mp4 --title "标题 #话题" --yes

# 仅自己可见，并从指定时间点截取封面
douyin publish video ./video.mp4 --visibility private --cover-tsp 2.3 --yes

# 调整每个上传请求的超时；普通 API 继续使用 30 秒
douyin publish video ./video.mp4 --upload-timeout 600 --yes
```

上传请求默认 300 秒。50 MiB 以内使用单次上传，更大的文件自动分片；默认每片 20 MiB，不足 5 MiB 的尾部并入前一片。文件和分片均流式读取，总文件上限为 4 GiB，额外内存不随整个视频大小增长。任意上传步骤失败都会停止，不自动重试。

## 继续已经上传的投稿

每次真实投稿都会创建私有的本地 JSON 记录，默认位于配置目录的 `publish/` 下。记录包含视频标识、所属 OAuth 应用与账号、标题、可见性及当前阶段，不保存 access token、refresh token 或 client secret。命令会在标准错误输出中显示记录路径。

```bash
# 先上传，确认内容后再创建作品
douyin publish video ./video.mp4 --title "标题" \
  --upload-only --receipt ./video-receipt.json --yes

# 查看本地记录
douyin publish status ./video-receipt.json

# 复用已上传的 video_id，不再次发送视频
douyin publish resume ./video-receipt.json --yes
```

自定义记录路径的父目录必须已存在。已有记录不会被覆盖；恢复时会校验账号和应用，文件别名也使用同一把进程锁。

| 阶段 | 含义 | 下一步 |
| --- | --- | --- |
| `uploading` | 文件还未完整上传 | 重新运行 `publish video` |
| `uploaded` | 已取得视频标识，尚未创建作品 | `publish resume --yes` |
| `creating` | 创建请求已经准备发送或正在处理 | 先检查创作者中心 |
| `unknown` | 创建请求返回错误或结果不完整 | 先检查创作者中心 |
| `submitted` | 服务器已返回作品 ID | 再次 resume 只读取记录，不重复创建 |

创建请求发出前会先持久保存 `creating`。网络中断时，服务器可能已收到投稿，因此 `creating` 和 `unknown` 默认停止恢复。核实账号中没有对应作品后，可明确重试：

```bash
douyin publish resume ./video-receipt.json --retry-create --yes
```

成功输出包含 `item_id`、`video_id`、记录路径及 `review_status: "not_queried"`。`stage: "submitted"` 表示创建接口已返回作品 ID；本地记录未查询服务端审核结果，实际审核及可见状态以创作者中心为准。

## 验证范围

本地回归通过真实 HTTP fixture 验证 multipart 字段、分片顺序和字节、尾片合并、错误中止、超时、禁止重定向、创建请求及恢复记录。CLI 测试使用隔离配置目录，覆盖预检、参数错误、授权状态、已有记录和重复恢复。测试不向真实抖音账号投稿。

```bash
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --all -- --check
```

当前分片接口的真实账号验收需要有对应能力的开放平台应用及 OAuth。浏览器创作者中心投稿单独验收，不作为这条 OpenAPI 链路的成功证据。

## 官方依据

- [上传视频](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/upload-video)
- [分片上传初始化](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/video-part-upload-init)
- [分片上传](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/video-part-upload)
- [分片上传完成](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/video-part-upload-complete)
- [创建视频](https://developer.open-douyin.com/docs/resource/zh-CN/dop/develop/openapi/video-management/douyin/create-video/video-create)
