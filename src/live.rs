//! Android room lifecycle and OBS transport with private, account-bound sessions.
use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Value, json};
use std::{
    future::Future,
    io::{Read, Write},
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

#[derive(Args)]
pub struct LiveArgs {
    #[command(subcommand)]
    command: LiveCommand,
}

#[derive(Subcommand)]
enum LiveCommand {
    /// 独立查询当前安卓账号的直播权限配置，不创建直播间
    Permissions,
    /// 查询开播预览配置（GET /webcast/room/create_info/，platform 固定为 APK 证实值 2）
    CreateInfo {
        #[arg(long, default_value_t = 2)]
        platform: i32,
    },
    /// 开播前权限检查（GET /webcast/room/create_check/）
    CreateCheck {
        #[arg(long, default_value = "0")]
        live_room_mode: String,
        #[arg(long, default_value = "2")]
        check_scene: String,
        #[arg(long, default_value = "1")]
        chat_auth: String,
        #[arg(long, default_value = "1")]
        share_auth: String,
        #[arg(long, default_value = "1")]
        digg_auth: String,
        #[arg(long, default_value = "douyin,xigua")]
        deliver: String,
        #[arg(long, default_value = "0")]
        introduction: String,
    },
    /// 使用当前采集传输查询公开直播间
    Info { room_id: String },
    /// 用 APK 证实的 FieldMap 键创建直播间（POST /webcast/room/create/）；值来自 JSON 文件，不猜填
    Create {
        #[arg(long)]
        params_file: String,
        /// 保存账号绑定的私密直播会话和 OBS 推流配置
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        yes: bool,
    },
    /// 观众离开直播间（POST /webcast/room/leave/）
    Leave {
        #[arg(long)]
        room_id: String,
        #[arg(long)]
        yes: bool,
    },
    /// 恢复当前账号直播间，并保存新的私密会话
    Continue {
        #[arg(long)]
        resolution_key: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        yes: bool,
    },
    /// 上报主播 prepare/play/pause/finish 状态
    AnchorStatus {
        #[arg(long)]
        session_file: String,
        #[arg(long, value_enum)]
        status: AnchorStatus,
        #[arg(long, default_value_t = 0)]
        reason_no: u32,
        #[arg(long, default_value = "anchor_ping")]
        source: String,
        #[arg(long)]
        yes: bool,
    },
    /// 上报结束状态并回读房间状态
    Finish {
        #[arg(long)]
        session_file: String,
        #[arg(long, default_value_t = 0)]
        reason_no: u32,
        #[arg(long, default_value = "anchor_ping")]
        source: String,
        #[arg(long)]
        yes: bool,
    },
    /// 有限次上报 play；Ctrl-C 停止后续心跳，失败立即停止
    Heartbeat {
        #[arg(long)]
        session_file: String,
        /// 每次成功响应到下次请求的秒数，必须明确指定
        #[arg(long, value_parser = clap::value_parser!(u64).range(1..=3600))]
        interval: u64,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=10000))]
        count: u32,
        #[arg(long)]
        yes: bool,
    },
    /// 通过本机 OBS WebSocket 联动直播
    Obs(ObsArgs),
}

#[derive(Args)]
struct ObsArgs {
    #[arg(long, default_value_t = 4455, global = true)]
    port: u16,
    #[arg(
        long,
        env = "DOUYIN_OBS_PASSWORD",
        hide_env_values = true,
        global = true
    )]
    password: Option<String>,
    #[command(subcommand)]
    command: ObsCommand,
}

#[derive(Subcommand)]
enum ObsCommand {
    /// 查询 OBS 推流状态，不读取推流密钥
    Status,
    /// 查询媒体源播放状态，不读取拉流地址
    SourceStatus {
        #[arg(long, default_value = "douyin-live")]
        name: String,
    },
    /// 将已有直播间的推流 JSON 配置到 OBS；默认不开始推流
    ConfigurePush {
        #[arg(long)]
        stream_file: String,
        /// 配置后启动 OBS 推流
        #[arg(long)]
        start: bool,
        #[arg(long)]
        yes: bool,
    },
    /// 将 URL 文件中的直播拉流地址作为媒体源加入指定场景
    AddSource {
        #[arg(long)]
        url_file: String,
        #[arg(long)]
        scene: String,
        #[arg(long, default_value = "douyin-live")]
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// 停止 OBS 推流；抖音直播间生命周期另行处理
    Stop {
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(args: LiveArgs) -> Result<(), String> {
    let args = match args.command {
        LiveCommand::Permissions => {
            let response = crate::net::NativeClient::saved()?.live_permissions()?;
            println!("{}", permission_summary(&response)?);
            return Ok(());
        }
        LiveCommand::CreateInfo { platform } => {
            let response = crate::net::NativeClient::saved()?.live_create_info(platform)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&create_info_summary(&response)?)
                    .map_err(|_| "开播预览结果序列化失败")?
            );
            return Ok(());
        }
        LiveCommand::CreateCheck {
            live_room_mode,
            check_scene,
            chat_auth,
            share_auth,
            digg_auth,
            deliver,
            introduction,
        } => {
            let response = crate::net::NativeClient::saved()?.live_create_check(&[
                ("live_room_mode".into(), live_room_mode),
                ("check_scene".into(), check_scene),
                ("chat_auth".into(), chat_auth),
                ("share_auth".into(), share_auth),
                ("digg_auth".into(), digg_auth),
                ("deliver".into(), deliver),
                ("introduction".into(), introduction),
            ])?;
            println!(
                "{}",
                serde_json::to_string_pretty(&create_check_summary(&response)?)
                    .map_err(|_| "开播检查结果序列化失败")?
            );
            return Ok(());
        }
        LiveCommand::Info { room_id } => {
            let response = crate::net::NativeClient::saved()?.live_room_info(&room_id)?;
            println!("{}", room_summary(&response, &room_id)?);
            return Ok(());
        }
        LiveCommand::Create {
            params_file,
            output,
            yes,
        } => {
            require_yes(yes, "创建直播间")?;
            let form = create_room_form(&read_private(&params_file)?)?;
            let pending = output.as_deref().map(SessionOutput::prepare).transpose()?;
            let client = crate::net::NativeClient::saved()?;
            let account_id = confirmed_account(&client)?;
            let response = client.live_create_room(&form)?;
            let summary = save_created_session(&response, &account_id, pending)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&summary).map_err(|_| "开播结果序列化失败")?
            );
            return Ok(());
        }
        LiveCommand::Continue {
            resolution_key,
            output,
            yes,
        } => {
            require_yes(yes, "恢复直播间")?;
            let pending = SessionOutput::prepare(&output)?;
            let client = crate::net::NativeClient::saved()?;
            let account_id = confirmed_account(&client)?;
            let response = client.live_continue_room(&resolution_key)?;
            let summary = save_created_session(&response, &account_id, Some(pending))?;
            println!(
                "{}",
                serde_json::to_string_pretty(&summary).map_err(|_| "续播结果序列化失败")?
            );
            return Ok(());
        }
        LiveCommand::AnchorStatus {
            session_file,
            status,
            reason_no,
            source,
            yes,
        } => {
            require_yes(yes, "上报主播状态")?;
            let (client, session) = confirmed_session(&session_file)?;
            println!(
                "{}",
                report_anchor(&client, &session, status, reason_no, &source)?
            );
            return Ok(());
        }
        LiveCommand::Finish {
            session_file,
            reason_no,
            source,
            yes,
        } => {
            require_yes(yes, "结束主播推流状态")?;
            let (client, session) = confirmed_session(&session_file)?;
            let mut summary =
                report_anchor(&client, &session, AnchorStatus::Finish, reason_no, &source)?;
            let verification = finish_verification(
                &mut summary,
                client.live_room_info(&session.room_id),
                &session.room_id,
            );
            println!("{summary}");
            verification?;
            return Ok(());
        }
        LiveCommand::Heartbeat {
            session_file,
            interval,
            count,
            yes,
        } => {
            require_yes(yes, "发送直播心跳")?;
            let (client, session) = confirmed_session(&session_file)?;
            return heartbeat(&client, &session, interval, count);
        }
        LiveCommand::Leave { room_id, yes } => {
            if !yes {
                return Err("离开直播间需要 --yes".into());
            }
            if room_id.parse::<u64>().ok().filter(|id| *id > 0).is_none()
                || !room_id.bytes().all(|b| b.is_ascii_digit())
            {
                return Err("直播间 ID 必须是正整数".into());
            }
            let response = crate::net::NativeClient::saved()?.live_leave_room(&room_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&create_check_summary(&response)?)
                    .map_err(|_| "离开直播间结果序列化失败")?
            );
            return Ok(());
        }
        LiveCommand::Obs(args) => args,
    };
    let confirmed = match &args.command {
        ObsCommand::Status | ObsCommand::SourceStatus { .. } => true,
        ObsCommand::ConfigurePush { yes, .. }
        | ObsCommand::AddSource { yes, .. }
        | ObsCommand::Stop { yes } => *yes,
    };
    if !confirmed {
        return Err("修改 OBS 配置或推流状态需要 --yes".into());
    }
    if args.port == 0 {
        return Err("OBS 端口不能为 0".into());
    }
    // Parse inputs before connecting, so malformed private data cannot change OBS.
    let settings = match &args.command {
        ObsCommand::ConfigurePush { stream_file, .. } => {
            push_settings(&read_private(stream_file)?)?
        }
        ObsCommand::AddSource {
            url_file,
            scene,
            name,
            ..
        } => {
            if scene.trim().is_empty() || name.trim().is_empty() {
                return Err("OBS 场景和媒体源名称不能为空".into());
            }
            let text = read_private(url_file)?;
            validate_url(text.trim(), &["https", "http", "rtmp", "rtmps"])?;
            json!({"is_local_file":false,"input":text.trim(),"restart_on_activate":true,
                "close_when_inactive":true,"reconnect_delay_sec":2})
        }
        ObsCommand::SourceStatus { name } if name.trim().is_empty() => {
            return Err("OBS 媒体源名称不能为空".into());
        }
        _ => Value::Null,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法创建 OBS 运行时")?;
    let value = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(30), execute(args, settings))
            .await
            .map_err(|_| "OBS 操作超时，修改操作可能已生效，请查询 status 确认".to_owned())?
    })?;
    println!("{value}");
    Ok(())
}

async fn execute(args: ObsArgs, settings: Value) -> Result<Value, String> {
    let client = obws::Client::connect("127.0.0.1", args.port, args.password)
        .await
        .map_err(|_| "无法连接或认证本机 OBS，请检查 WebSocket 端口和密码")?;
    let status = client
        .streaming()
        .status()
        .await
        .map_err(|_| "无法查询 OBS 推流状态")?;
    match args.command {
        ObsCommand::Status => Ok(
            json!({"active":status.active,"reconnecting":status.reconnecting,
            "bytes":status.bytes,"frames":status.total_frames,"skipped_frames":status.skipped_frames}),
        ),
        ObsCommand::SourceStatus { name } => {
            let media = client
                .media_inputs()
                .status(obws::requests::inputs::InputId::Name(&name))
                .await
                .map_err(|_| "无法查询 OBS 媒体源播放状态，请检查名称和媒体源类型")?;
            Ok(json!({"name":name,"state":media.state,
                "duration_ms":media.duration.map(|v| v.whole_milliseconds()),
                "cursor_ms":media.cursor.map(|v| v.whole_milliseconds())}))
        }
        ObsCommand::ConfigurePush { start, .. } => {
            if status.active {
                return Err("OBS 正在推流，不能替换推流目标".into());
            }
            client
                .config()
                .set_stream_service_settings("rtmp_custom", &settings)
                .await
                .map_err(|_| "OBS 未确认推流配置成功，请检查 OBS 配置")?;
            if start {
                client
                    .streaming()
                    .start()
                    .await
                    .map_err(|_| "OBS 配置已更新，但启动推流失败，请查询 status 确认")?;
            }
            Ok(json!({"configured":true,"start_requested":start}))
        }
        ObsCommand::AddSource { scene, name, .. } => {
            client
                .inputs()
                .create(obws::requests::inputs::Create {
                    scene: obws::requests::scenes::SceneId::Name(&scene),
                    input: &name,
                    kind: "ffmpeg_source",
                    settings: Some(settings),
                    enabled: Some(true),
                })
                .await
                .map_err(|_| "OBS 未确认媒体源创建成功，请检查场景、名称冲突和媒体源插件")?;
            Ok(json!({"created":true,"scene":scene,"name":name}))
        }
        ObsCommand::Stop { .. } => {
            if status.active {
                client
                    .streaming()
                    .stop()
                    .await
                    .map_err(|_| "OBS 停止推流请求失败，请查询 status 确认")?;
            }
            Ok(json!({"stop_requested":status.active}))
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AnchorStatus {
    Prepare,
    Play,
    Pause,
    Finish,
}

impl AnchorStatus {
    fn code(self) -> u8 {
        match self {
            Self::Prepare => 1,
            Self::Play => 2,
            Self::Pause => 3,
            Self::Finish => 4,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Play => "play",
            Self::Pause => "pause",
            Self::Finish => "finish",
        }
    }
}

#[derive(Debug)]
struct LiveSession {
    account_id: String,
    room_id: String,
    stream_id: String,
    push_url: Option<String>,
}

impl LiveSession {
    fn from_value(value: &Value) -> Result<Self, String> {
        if value["schema"].as_u64() != Some(1) {
            return Err("直播会话文件 schema 必须为 1".into());
        }
        let push_url = value
            .get("rtmp_push_url")
            .map(|value| {
                let raw = value.as_str().ok_or("直播会话推流地址必须是字符串")?;
                push_settings(&json!({"rtmp_push_url":raw}).to_string())?;
                Ok::<_, String>(raw.to_owned())
            })
            .transpose()?;
        Ok(Self {
            account_id: object_id(value, &["account_id"], "账号")?,
            room_id: object_id(value, &["room_id"], "直播间")?,
            stream_id: object_id(value, &["stream_id"], "直播流")?,
            push_url,
        })
    }

    fn from_response(response: &Value, account_id: &str) -> Result<Self, String> {
        response_success(response, "直播房间")?;
        let room = response_room(response)?;
        if let Some(owner) = room.get("owner").filter(|value| value.is_object())
            && (owner.get("id_str").is_some() || owner.get("id").is_some())
            && object_id(owner, &["id_str", "id"], "主播")? != account_id
        {
            return Err("返回直播间的主播与当前账号不一致，未保存推流信息".into());
        }
        let push_url = ["rtmps_push_url", "rtmp_push_url"]
            .into_iter()
            .find_map(|field| {
                room.get("stream_url")?
                    .get(field)?
                    .as_str()
                    .filter(|raw| !raw.is_empty())
            })
            .map(str::to_owned);
        let session = Self {
            account_id: positive_id(account_id, "账号")?,
            room_id: object_id(room, &["id_str", "id"], "直播间")?,
            stream_id: object_id(room, &["stream_id_str", "stream_id"], "直播流")?,
            push_url,
        };
        // Reuse exactly the same validation as files loaded by lifecycle/OBS commands.
        Self::from_value(&session.to_value())
    }

    fn to_value(&self) -> Value {
        let mut value = json!({
            "schema":1,
            "account_id":self.account_id,
            "room_id":self.room_id,
            "stream_id":self.stream_id,
        });
        if let Some(url) = &self.push_url {
            // The existing OBS configure-push command accepts this field directly.
            value["rtmp_push_url"] = json!(url);
        }
        value
    }

    fn require_account(&self, account_id: &str) -> Result<(), String> {
        if self.account_id != positive_id(account_id, "账号")? {
            return Err("直播会话属于其它账号，拒绝发送主播状态".into());
        }
        Ok(())
    }
}

struct SessionOutput {
    pending: tempfile::NamedTempFile,
    destination: PathBuf,
}

impl SessionOutput {
    fn prepare(path: &Path) -> Result<Self, String> {
        if path.as_os_str().is_empty() || path == Path::new("-") || path.file_name().is_none() {
            return Err("直播会话必须保存到新文件，不能写入标准输出".into());
        }
        match std::fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Ok(_) => return Err("直播会话输出文件已存在，拒绝覆盖".into()),
            Err(_) => return Err("无法检查直播会话输出位置".into()),
        }
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let pending = tempfile::Builder::new()
            .prefix(".douyin-live-")
            .tempfile_in(parent)
            .map_err(|_| "无法在目标目录创建私密直播会话文件")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            pending
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(|_| "无法限制直播会话文件权限")?;
        }
        Ok(Self {
            pending,
            destination: path.to_owned(),
        })
    }

    fn save(self, session: &LiveSession) -> Result<(), String> {
        self.save_with_io(
            session,
            |file, bytes| file.write_all(bytes),
            |file| file.sync_all(),
        )
    }

    fn save_with_io(
        mut self,
        session: &LiveSession,
        write: impl FnOnce(&mut std::fs::File, &[u8]) -> std::io::Result<()>,
        sync: impl FnOnce(&std::fs::File) -> std::io::Result<()>,
    ) -> Result<(), String> {
        let mut text =
            serde_json::to_vec_pretty(&session.to_value()).map_err(|_| "直播会话序列化失败")?;
        text.push(b'\n');
        if text.len() > 16384 {
            return Err("直播请求已接受，但会话超过 16 KiB，未写入文件".into());
        }
        if write(self.pending.as_file_mut(), &text).is_err() {
            return Err(Self::recovery_error(
                self.pending,
                "会话写入失败，恢复文件可能不完整",
            ));
        }
        if sync(self.pending.as_file()).is_err() {
            return Err(Self::recovery_error(
                self.pending,
                "会话同步失败，JSON 已完整写入但尚未确认持久化",
            ));
        }
        match self.pending.persist_noclobber(&self.destination) {
            Ok(_) => Ok(()),
            Err(error) => Err(Self::recovery_error(error.file, "目标文件提交失败")),
        }
    }

    fn recovery_error(pending: tempfile::NamedTempFile, reason: &str) -> String {
        match pending.keep() {
            Ok((_, path)) => format!(
                "直播请求已接受，但{reason}；私密恢复文件保留在 {}，请勿重新创建直播间",
                path.display()
            ),
            Err(_) => {
                format!("直播请求已接受，但{reason}且恢复文件保留失败，请检查房间状态后再操作")
            }
        }
    }
}

fn positive_id(value: &str, label: &str) -> Result<String, String> {
    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!("{label} ID 必须是正整数"));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .map(|id| id.to_string())
        .ok_or_else(|| format!("{label} ID 必须是正整数"))
}

fn object_id(object: &Value, fields: &[&str], label: &str) -> Result<String, String> {
    for field in fields {
        if let Some(value) = object.get(*field) {
            return match value {
                Value::Null => continue,
                Value::String(value) if value.is_empty() => continue,
                Value::String(value) => positive_id(value, label),
                Value::Number(value) => value
                    .as_u64()
                    .filter(|id| *id > 0)
                    .map(|id| id.to_string())
                    .ok_or_else(|| format!("{label} ID 必须是正整数")),
                _ => Err(format!("{label} ID 格式无效")),
            };
        }
    }
    Err(format!("直播响应或会话缺少{label} ID"))
}

fn response_success(response: &Value, label: &str) -> Result<(), String> {
    match crate::net::integer(&response["status_code"]) {
        Some(0) => Ok(()),
        Some(code) => Err(format!("{label}接口未接受请求（status_code={code}）")),
        None => Err(format!("{label}响应缺少 status_code，无法确认请求成功")),
    }
}

fn response_room(response: &Value) -> Result<&Value, String> {
    response
        .pointer("/data/room")
        .filter(|room| room.is_object())
        .or_else(|| response.get("data").filter(|room| room.is_object()))
        .ok_or_else(|| "直播响应缺少房间对象".into())
}

fn confirmed_account(client: &crate::net::NativeClient) -> Result<String, String> {
    let account_id = positive_id(client.account_id()?, "账号")?;
    client.me()?;
    Ok(account_id)
}

fn confirmed_session(path: &str) -> Result<(crate::net::NativeClient, LiveSession), String> {
    let value: Value =
        serde_json::from_str(&read_private(path)?).map_err(|_| "直播会话不是合法 JSON")?;
    let session = LiveSession::from_value(&value)?;
    let client = crate::net::NativeClient::saved()?;
    // Reject account mismatches before making any network request.
    session.require_account(client.account_id()?)?;
    confirmed_account(&client)?;
    Ok((client, session))
}

fn save_created_session(
    response: &Value,
    account_id: &str,
    output: Option<SessionOutput>,
) -> Result<Value, String> {
    let mut summary = create_room_summary(response)?;
    summary["session_saved"] = json!(false);
    if let Some(output) = output {
        let room_id = summary["room_id"].as_str().ok_or("开播结果缺少房间 ID")?;
        let session = LiveSession::from_response(response, account_id).map_err(|error| {
            format!("直播间 {room_id} 请求已接受，但会话保存失败：{error}；请先查询房间状态，勿直接重试创建")
        })?;
        summary["stream_id"] = json!(session.stream_id);
        summary["has_push_url"] = json!(session.push_url.is_some());
        output
            .save(&session)
            .map_err(|error| format!("直播间 {}：{error}", session.room_id))?;
        summary["session_saved"] = json!(true);
    }
    Ok(summary)
}

fn require_yes(yes: bool, action: &str) -> Result<(), String> {
    if yes {
        Ok(())
    } else {
        Err(format!("{action}需要 --yes"))
    }
}

fn report_anchor(
    client: &crate::net::NativeClient,
    session: &LiveSession,
    status: AnchorStatus,
    reason_no: u32,
    source: &str,
) -> Result<Value, String> {
    let response = client.live_anchor_status(
        &session.room_id,
        &session.stream_id,
        status.code(),
        reason_no,
        source,
    )?;
    response_success(&response, "主播状态")?;
    Ok(json!({
        "room_id":session.room_id,
        "stream_id":session.stream_id,
        "status_reported":status.name(),
        "request_accepted":true,
    }))
}

fn finish_verification(
    summary: &mut Value,
    response: Result<Value, String>,
    room_id: &str,
) -> Result<(), String> {
    match response.and_then(|response| room_summary(&response, room_id)) {
        Ok(room) if crate::net::integer(&room["status"]).is_some() => {
            summary["verification"] = json!("room_info_observed");
            summary["observed_room_status"] = room["status"].clone();
            // Ping status=4 is NOT evidence that Room.status=4 means ended.
            // Preserve the observed value until that separate enum is verified.
            Ok(())
        }
        _ => {
            summary["verification"] = json!("room_info_unavailable");
            summary["observed_room_status"] = Value::Null;
            Err(format!(
                "finish 已接受，房间状态回读失败，请运行 douyin live info {room_id} 查询，勿重复 finish"
            ))
        }
    }
}

type InterruptSignal = Pin<Box<dyn Future<Output = std::io::Result<()>>>>;

fn wait_for_interrupt(
    runtime: &tokio::runtime::Runtime,
    signal: &mut InterruptSignal,
    duration: Duration,
) -> Result<bool, String> {
    runtime.block_on(async {
        let mut timer = Box::pin(tokio::time::sleep(duration));
        std::future::poll_fn(|context| {
            if let std::task::Poll::Ready(result) = signal.as_mut().poll(context) {
                return std::task::Poll::Ready(
                    result
                        .map(|_| true)
                        .map_err(|_| "无法监听 Ctrl-C".to_owned()),
                );
            }
            timer.as_mut().poll(context).map(|_| Ok(false))
        })
        .await
    })
}

fn heartbeat(
    client: &crate::net::NativeClient,
    session: &LiveSession,
    interval: u64,
    count: u32,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法创建直播心跳运行时")?;
    let mut signal: InterruptSignal = Box::pin(tokio::signal::ctrl_c());
    let mut sent = 0;
    loop {
        // This first poll registers Ctrl-C before the first HTTP request. Blocking
        // requests execute outside Tokio; no further request follows cancellation.
        if wait_for_interrupt(&runtime, &mut signal, Duration::ZERO)? {
            println!(
                "{}",
                json!({"event":"heartbeat_stopped","sent":sent,"interrupted":true})
            );
            return Ok(());
        }
        let mut summary = report_anchor(client, session, AnchorStatus::Play, 0, "anchor_ping")?;
        sent += 1;
        summary["event"] = json!("heartbeat");
        summary["sent"] = json!(sent);
        summary["count"] = json!(count);
        println!("{summary}");
        std::io::stdout()
            .flush()
            .map_err(|_| "写入直播心跳结果失败")?;
        if sent == count {
            println!(
                "{}",
                json!({"event":"heartbeat_complete","sent":sent,"interrupted":false})
            );
            return Ok(());
        }
        if wait_for_interrupt(&runtime, &mut signal, Duration::from_secs(interval))? {
            println!(
                "{}",
                json!({"event":"heartbeat_stopped","sent":sent,"interrupted":true})
            );
            return Ok(());
        }
    }
}

fn read_private(path: &str) -> Result<String, String> {
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path).map_err(|_| "无法读取直播输入文件")?)
    };
    let mut text = String::new();
    reader
        .take(16385)
        .read_to_string(&mut text)
        .map_err(|_| "直播输入不是有效 UTF-8")?;
    if text.len() > 16384 {
        return Err("直播输入超过 16 KiB 限制".into());
    }
    Ok(text)
}

fn validate_url(raw: &str, schemes: &[&str]) -> Result<(), String> {
    let url = url::Url::parse(raw).map_err(|_| "直播地址无效")?;
    if raw.chars().any(char::is_control)
        || !schemes.contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("直播地址协议、主机或格式无效".into());
    }
    Ok(())
}

fn push_settings(text: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(text).map_err(|_| "推流配置不是合法 JSON")?;
    if let Some(raw) = value.get("rtmp_push_url") {
        if value.get("server").is_some() || value.get("key").is_some() {
            return Err("完整推流地址不能与 server/key 混用".into());
        }
        let raw = raw.as_str().ok_or("完整推流地址必须是字符串")?;
        validate_url(raw, &["rtmp", "rtmps"])?;
        if raw.trim() != raw {
            return Err("完整推流地址不能包含首尾空白".into());
        }
        // Parse only for validation. Split the original bytes before its query:
        // decoding %2F or splitting on a slash inside a signature changes the key.
        let path_end = raw.find('?').unwrap_or(raw.len());
        let authority_start = raw.find("://").ok_or("完整推流地址格式无效")? + 3;
        let split = raw[..path_end]
            .rfind('/')
            .filter(|index| *index >= authority_start && *index + 1 < path_end)
            .ok_or("完整推流地址缺少流名称")?;
        return Ok(json!({"server":&raw[..=split],"key":&raw[split + 1..],"use_auth":false}));
    }
    let server = value["server"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("推流配置缺少 server")?;
    let key = value["key"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("推流配置缺少 key")?;
    validate_url(server, &["rtmp", "rtmps"])?;
    if key.chars().any(char::is_control) {
        return Err("推流密钥包含控制字符".into());
    }
    // Preserve signed bytes exactly; never split or reserialize a stream key.
    Ok(json!({"server":server,"key":key,"use_auth":false}))
}

/// FieldMap keys recovered from classes13 `X.0VaJ.LIZLLL` create-room builder.
const CREATE_ROOM_KEYS: &[&str] = &[
    "title",
    "cover_uri",
    "game",
    "tags",
    "gen_replay",
    "record_screen",
    "gift_auth",
    "orientation",
    "webcast",
    "acquaintance",
    "enable_realtime_replay",
    "has_commerce_goods",
    "live_audio",
    "auto_cover",
    "third_party",
    "category",
    "screen_shot",
    "live_room_mode",
    "chat_auth",
    "share_auth",
    "introduction",
    "digg_auth",
    "deliver",
    "xigua_cover_uri",
    "virtual_live_info",
    "poi_id",
    "circle_id",
    "resolution_key",
    "prop_id",
    "fansclub_auth",
    "redpacket_audience_auth",
    "paid_live_type",
    "ticket_id",
    "paid_live_create_rights",
    "desire_id",
    "probe_recommend_resolution",
    "is_user_select",
    "challenge_id",
    "create_room_source",
    "audience_display_type",
    "vr_type",
    "enable_health_score_check",
    "ab_params",
    "sub_orientation",
    "frame_scale",
    "continue_room",
    "uid",
    "visibility_range",
    "disable_location_permission",
    "audience",
    "stream_type",
    "push_stream_type",
    "live_type_audio",
    "live_type_screenshot",
    "live_type_third_party",
    "live_type_normal",
    "hashtag",
    "topic_id",
    "visibility",
    "platform",
];

fn create_room_form(text: &str) -> Result<Vec<(String, String)>, String> {
    let value: Value = serde_json::from_str(text).map_err(|_| "开播参数不是合法 JSON")?;
    let object = value
        .as_object()
        .ok_or("开播参数必须是 JSON 对象，键为 APK 证实的 FieldMap 字段")?;
    let mut form = Vec::new();
    for (key, value) in object {
        if !CREATE_ROOM_KEYS.contains(&key.as_str()) {
            return Err(format!("开播参数含未证实字段: {key}"));
        }
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            Value::Bool(flag) => {
                if *flag {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            _ => return Err(format!("开播参数 {key} 必须是字符串、数字或布尔")),
        };
        if text.chars().any(char::is_control) {
            return Err(format!("开播参数 {key} 含控制字符"));
        }
        if text.len() > 4096 {
            return Err(format!("开播参数 {key} 过长"));
        }
        form.push((key.clone(), text));
    }
    if form.is_empty() {
        return Err("开播参数不能为空".into());
    }
    if !form.iter().any(|(key, _)| key == "title") {
        return Err("开播参数至少需要 title".into());
    }
    Ok(form)
}

fn create_room_summary(response: &Value) -> Result<Value, String> {
    response_success(response, "直播房间")?;
    let room = response_room(response)?;
    let room_id = object_id(room, &["id_str", "id"], "直播间")?;
    Ok(json!({"status_code":0,"ok":true,"room_id":room_id,"has_data":true}))
}

fn create_info_summary(response: &Value) -> Result<Value, String> {
    response_success(response, "开播预览")?;
    Ok(json!({
        "status_code":0,"ok":true,
        "has_data":response.get("data").is_some(),
        "keys":response.as_object().map(|object| object.keys().cloned().collect::<Vec<_>>()).unwrap_or_default(),
    }))
}

fn create_check_summary(response: &Value) -> Result<Value, String> {
    response_success(response, "直播检查")?;
    Ok(json!({"status_code":0,"ok":true,"has_data":response.get("data").is_some()}))
}

pub(crate) fn permission_summary(response: &Value) -> Result<Value, String> {
    if response["status_code"] != 0 {
        return Err("直播权限接口未确认有效响应".into());
    }
    let data = response["data"].as_object().ok_or("直播权限配置格式无效")?;
    let authentication_type = data
        .get("authentication_type")
        .and_then(Value::as_u64)
        .ok_or("直播权限响应缺少受支持的认证类型")?;
    let mut switches = serde_json::Map::new();
    for name in [
        "show_gift_setting_switch",
        "show_live_life_entry",
        "buy_subscribe_switch",
        "shop_playback",
        "shop_request_explain",
        "group_chat",
        "highlight",
        "live_commercial",
        "play_back",
        "realtime_playback",
        "record",
    ] {
        if let Some(value) = data.get(name).filter(|v| sdk_setting_scalar(v)) {
            switches.insert(name.into(), value.clone());
        }
    }
    let mut schedule = serde_json::Map::new();
    if let Some(config) = data.get("broadcast_config").and_then(Value::as_object) {
        for name in [
            "scheduled_master_switch",
            "scheduled_profile_switch",
            "scheduled_notice_switch",
            "scheduled_days",
            "scheduled_notice_switch_display",
        ] {
            if let Some(value) = config.get(name).filter(|v| sdk_setting_scalar(v)) {
                schedule.insert(name.into(), value.clone());
            }
        }
        for name in ["scheduled_time", "scheduled_time_text"] {
            if let Some(value) = config
                .get(name)
                .and_then(Value::as_str)
                .filter(|s| s.len() <= 256 && !s.chars().any(char::is_control))
            {
                schedule.insert(name.into(), json!(value));
            }
        }
    }
    // Do not expose last_room, stream URLs or arbitrary server-added fields.
    Ok(json!({"authenticationType": authentication_type,
        "switches":switches,"schedule":schedule,
        "availableSections":data.keys().filter(|k| k.as_str() != "last_room").collect::<Vec<_>>()}))
}

fn sdk_setting_scalar(value: &Value) -> bool {
    value.is_boolean() || value.as_i64().is_some_and(|v| i32::try_from(v).is_ok())
}

fn room_stream_type(room: &Value) -> Option<&'static str> {
    // 40.8.0 Room.getStreamType order; do not apply Java's absent-field
    // defaults to an unrecognized or incomplete raw server response.
    if room.get("room_layout")?.as_i64()? == 1 || room.get("layout")?.as_i64()? == 1 {
        return Some("media");
    }
    for (field, kind) in [
        ("live_type_screenshot", "screen_record"),
        ("live_type_third_party", "third_party"),
        ("live_type_audio", "audio"),
    ] {
        if room.get(field)?.as_bool()? {
            return Some(kind);
        }
    }
    Some("video")
}

fn room_summary(response: &Value, requested: &str) -> Result<Value, String> {
    let web = response["data"]["data"].is_array();
    let (room, owner) = if web {
        (
            response["data"]["data"]
                .get(0)
                .filter(|room| room.is_object())
                .ok_or("网页直播接口未返回房间")?,
            &response["data"]["user"],
        )
    } else {
        (&response["data"], &response["data"]["owner"])
    };
    let id = object_id(room, &["id_str", "id"], "直播间")?;
    if !web && id.parse::<u64>().ok().filter(|id| *id > 0) != requested.parse::<u64>().ok() {
        return Err("返回的直播间与请求目标不一致".into());
    }
    // Never dump the room object: it can contain signed stream URLs or keys.
    Ok(json!({
        "room_id":if web { requested } else { &id },
        "live_id":if web { Some(&id) } else { None },
        "web_rid":if web { Some(requested) } else { None },
        "title":room["title"].as_str(),
        "status":crate::net::integer(&room["status"]),
        "sdk_stream_type":room_stream_type(room),
        "owner_id":owner["id_str"].as_str().or_else(|| owner["sec_uid"].as_str()),
        "owner_name":owner["nickname"].as_str()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_type_follows_sdk_priority_and_missing_fields_never_mean_mobile_video() {
        let mut room = json!({"room_layout":0,"layout":0,"live_type_screenshot":false,
            "live_type_third_party":false,"live_type_audio":false});
        assert_eq!(room_stream_type(&room), Some("video"));
        room["live_type_audio"] = json!(true);
        assert_eq!(room_stream_type(&room), Some("audio"));
        room["live_type_third_party"] = json!(true);
        assert_eq!(room_stream_type(&room), Some("third_party"));
        room["live_type_screenshot"] = json!(true);
        assert_eq!(room_stream_type(&room), Some("screen_record"));
        room["layout"] = json!(1);
        assert_eq!(room_stream_type(&room), Some("media"));
        assert_eq!(room_stream_type(&json!({"room_layout":1})), Some("media"));
        assert_eq!(room_stream_type(&json!({})), None);
        assert_eq!(room_stream_type(&json!({"room_layout":0,"layout":0})), None);
        assert_eq!(
            room_stream_type(&json!({"room_layout":0,"layout":0,
            "live_type_screenshot":"false"})),
            None
        );
    }

    #[test]
    fn permission_output_excludes_room_credentials_and_rejects_unrecognized_data() {
        let response = json!({"status_code":0,"data":{"authentication_type":1,
            "show_live_life_entry":true,"last_room":{"stream_key":"private-fixture"},
            "future_secret":"another-private-fixture","buy_subscribe_switch":"private-switch"}});
        let summary = permission_summary(&response).unwrap();
        assert_eq!(summary["authenticationType"], 1);
        assert_eq!(summary["switches"]["show_live_life_entry"], true);
        assert!(!summary.to_string().contains("private"));
        assert!(!summary.to_string().contains("last_room"));
        for bad in [
            json!({"status_code":20003}),
            json!({"status_code":0,"data":{}}),
            json!({"status_code":0,"data":{"authentication_type":"unknown"}}),
        ] {
            assert!(permission_summary(&bad).is_err());
        }
    }

    #[test]
    fn permission_settings_follow_sdk_fields_without_inventing_broadcast_access() {
        let response = json!({"status_code":0,"data":{"authentication_type":0,
            "record":-1,"shop_playback":true,"realtime_playback":false,
            "highlight":4294967296u64,"group_chat":"private-fixture",
            "broadcast_config":{"scheduled_master_switch":true,"scheduled_notice_switch":2,
                "scheduled_days":3,"scheduled_time":"20:30","scheduled_time_text":"每天 20:30",
                "future_push_key":"private-fixture","scheduled_profile_switch":"private-fixture"},
            "last_room":{"rtmp_push_url":"private-fixture"}}});
        let summary = permission_summary(&response).unwrap();
        assert_eq!(summary["switches"]["record"], -1);
        assert_eq!(summary["switches"]["shop_playback"], true);
        assert_eq!(summary["schedule"]["scheduled_time"], "20:30");
        assert_eq!(summary["schedule"]["scheduled_notice_switch"], 2);
        assert!(summary["switches"].get("highlight").is_none());
        assert!(summary.get("canBroadcast").is_none());
        assert!(!summary.to_string().contains("private-fixture"));
    }

    #[test]
    fn room_summary_binds_target_and_excludes_stream_credentials() {
        let response = json!({"data":{"id_str":"123","title":"fixture","status":2,
            "stream_url":{"rtmp_push_url":"private-stream-secret"},"extra_secret":"private-token"}});
        let summary = room_summary(&response, "123").unwrap().to_string();
        assert!(!summary.contains("private"));
        assert!(room_summary(&response, "124").is_err());
        assert!(room_summary(&json!({"data":{}}), "123").is_err());

        let web = json!({"status_code":0,"data":{"data":[{"id_str":"456","title":"web",
            "status":2,"stream_url":{"flv_pull_url":{"HD":"private-stream-secret"}}}],
            "user":{"sec_uid":"public-sec-uid","nickname":"owner"}}});
        let summary = room_summary(&web, "123").unwrap();
        assert_eq!(summary["room_id"], "123");
        assert_eq!(summary["web_rid"], "123");
        assert_eq!(summary["live_id"], "456");
        assert_eq!(summary["owner_id"], "public-sec-uid");
        assert!(!summary.to_string().contains("private"));
        assert!(room_summary(&json!({"data":{"data":[]}}), "123").is_err());
    }

    #[test]
    fn full_push_url_keeps_encoded_key_and_query_bytes_intact() {
        let url = "rtmps://push.example:443/live/opaque%2Fkey?sig=a/b%2Bc==&ts=123";
        let settings = push_settings(&json!({"rtmp_push_url":url}).to_string()).unwrap();
        assert_eq!(settings["server"], "rtmps://push.example:443/live/");
        assert_eq!(settings["key"], "opaque%2Fkey?sig=a/b%2Bc==&ts=123");
        assert_eq!(
            format!(
                "{}{}",
                settings["server"].as_str().unwrap(),
                settings["key"].as_str().unwrap()
            ),
            url
        );
        for bad in [
            json!({"rtmp_push_url":"rtmp://push.example/live/?private-secret=a/b"}),
            json!({"rtmp_push_url":"rtmp://push.example?private-secret=a/b"}),
            json!({"rtmp_push_url":" rtmp://push.example/live/private-secret"}),
            json!({"rtmp_push_url":"rtmp://push.example/live/private-secret", "key":"conflict"}),
            json!({"rtmp_push_url":null}),
        ] {
            assert!(
                !push_settings(&bad.to_string())
                    .unwrap_err()
                    .contains("private-secret")
            );
        }
    }

    #[test]
    fn preserves_signed_stream_keys_and_never_echoes_bad_input() {
        let settings = push_settings(
            r#"{"server":"rtmps://push.example/live/","key":"opaque%2Fkey?token=a%2Bb&ts=123"}"#,
        )
        .unwrap();
        assert_eq!(settings["key"], "opaque%2Fkey?token=a%2Bb&ts=123");
        for text in [
            "private-invalid-json",
            r#"{"server":"file:///private","key":"secret"}"#,
            r#"{"server":"rtmps://user:secret@push.example/live/","key":"secret"}"#,
            r#"{"server":"rtmps://push.example/live/","key":"secret\n"}"#,
        ] {
            let error = push_settings(text).unwrap_err();
            assert!(!error.contains("private"));
            assert!(!error.contains("secret"));
        }
    }

    #[test]
    fn create_room_form_only_accepts_apk_fieldmap_keys() {
        let form = create_room_form(r#"{"title":"fixture","live_room_mode":"0"}"#).unwrap();
        assert_eq!(form.len(), 2);
        assert!(create_room_form(r#"{"title":"fixture","unknown":"x"}"#).is_err());
        assert!(create_room_form(r#"{"live_room_mode":"0"}"#).is_err());
        assert!(create_room_form(r#"{"title":"bad\n"}"#).is_err());
    }
    fn created_room_fixture() -> Value {
        json!({"status_code":0,"data":{"room":{
            "id_str":"123", "stream_id_str":"456", "owner":{"id_str":"789"},
            "stream_url":{
                "rtmps_push_url":"rtmps://push.example/live/opaque%2Fkey?sig=a/b%2Bc==",
                "rtmp_pull_url":"rtmp://pull.example/live/do-not-use",
                "rtmp_push_url_params":{"secret":"do-not-publish"}
            },
            "create_block_extra":{"ticket":"do-not-publish"}
        }}})
    }

    #[test]
    fn live_session_keeps_account_room_and_stream_and_is_obs_compatible() {
        let response = created_room_fixture();
        let session = LiveSession::from_response(&response, "789").unwrap();
        assert_eq!(session.room_id, "123");
        assert_eq!(session.stream_id, "456");
        session.require_account("789").unwrap();
        assert!(session.require_account("790").is_err());
        let document = session.to_value().to_string();
        let settings = push_settings(&document).unwrap();
        assert_eq!(settings["server"], "rtmps://push.example/live/");
        assert_eq!(settings["key"], "opaque%2Fkey?sig=a/b%2Bc==");
        assert!(!document.contains("do-not-publish"));
        assert!(!document.contains("rtmp_pull_url"));
        let summary = create_room_summary(&response).unwrap().to_string();
        assert!(!summary.contains("push.example"));
        assert!(!summary.contains("opaque"));
    }

    #[test]
    fn session_rejects_wrong_owner_invalid_ids_and_malformed_secret_without_echoing_it() {
        assert!(LiveSession::from_response(&created_room_fixture(), "790").is_err());
        for invalid in [
            json!({"schema":2,"account_id":"789","room_id":"123","stream_id":"456"}),
            json!({"schema":1,"account_id":"789","room_id":"0","stream_id":"456"}),
            json!({"schema":1,"account_id":"789","room_id":"123","stream_id":"x"}),
            json!({"schema":1,"account_id":"789","room_id":"123","stream_id":"456",
                "rtmp_push_url":"https://private-secret.example/key"}),
        ] {
            let error = LiveSession::from_value(&invalid).unwrap_err();
            assert!(!error.contains("private-secret"));
        }
        let mut response = created_room_fixture();
        response["status_code"] = json!(20003);
        response["status_msg"] = json!("private-ticket");
        assert!(
            !LiveSession::from_response(&response, "789")
                .unwrap_err()
                .contains("private")
        );
    }

    #[test]
    fn session_does_not_invent_a_push_url_from_pull_addresses_or_lists() {
        let response = json!({"status_code":0,"data":{
            "id":123,"stream_id":456,
            "stream_url":{
                "rtmp_pull_url":"rtmp://pull.example/read-only",
                "push_urls":["rtmp://push.example/ambiguous"]
            }
        }});
        let session = LiveSession::from_response(&response, "789").unwrap();
        assert!(session.push_url.is_none());
        assert!(session.to_value().get("rtmp_push_url").is_none());
    }

    #[test]
    fn private_session_output_is_new_atomic_and_has_restrictive_unix_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.json");
        let pending = SessionOutput::prepare(&path).unwrap();
        assert!(!path.exists());
        let session = LiveSession::from_response(&created_room_fixture(), "789").unwrap();
        pending.save(&session).unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved, session.to_value());
        assert!(SessionOutput::prepare(&path).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(SessionOutput::prepare(Path::new("-")).is_err());
    }

    #[test]
    fn private_session_output_race_preserves_existing_file_and_recoverable_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.json");
        let pending = SessionOutput::prepare(&path).unwrap();
        std::fs::write(&path, "existing-user-data").unwrap();
        let session = LiveSession::from_response(&created_room_fixture(), "789").unwrap();
        let error = pending.save(&session).unwrap_err();
        assert!(!error.contains("opaque"));
        assert!(!error.contains("push.example"));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "existing-user-data"
        );
        let recovery = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|candidate| candidate != &path)
            .unwrap();
        let saved: Value = serde_json::from_slice(&std::fs::read(recovery).unwrap()).unwrap();
        assert_eq!(saved, session.to_value());
    }

    #[test]
    fn accepted_finish_is_separate_from_room_state_observation() {
        let mut summary = json!({"request_accepted":true,"status_reported":"finish"});
        finish_verification(
            &mut summary,
            Ok(json!({"data":{"id_str":"123","status":4,"stream_url":{"secret":"private"}}})),
            "123",
        )
        .unwrap();
        assert_eq!(summary["verification"], "room_info_observed");
        assert_eq!(summary["observed_room_status"], 4);
        assert!(summary.get("end_confirmed").is_none());
        assert!(summary.get("closed").is_none());
        assert!(!summary.to_string().contains("private"));
        let error = finish_verification(&mut summary, Err("private-network-error".into()), "123")
            .unwrap_err();
        assert!(error.contains("finish 已接受"));
        assert!(error.contains("douyin live info 123"));
        assert!(!error.contains("private"));
        assert_eq!(summary["verification"], "room_info_unavailable");
        assert!(summary["observed_room_status"].is_null());
        assert!(!summary.to_string().contains("private"));
    }

    #[test]
    fn lifecycle_summaries_reject_incomplete_success_and_do_not_echo_raw_status_messages() {
        assert!(create_room_summary(&json!({"status_code":0,"data":{}})).is_err());
        assert!(create_room_summary(&json!({"status_code":0,"data":{"id":0}})).is_err());
        assert!(create_room_summary(&json!({"data":{"id":123}})).is_err());
        for response in [
            json!({"status_code":20003,"status_msg":"private-ticket"}),
            json!({"status_msg":"private-ticket"}),
        ] {
            assert!(
                !create_check_summary(&response)
                    .unwrap_err()
                    .contains("private")
            );
        }
        let summary = create_check_summary(
            &json!({"status_code":0,"status_msg":"private-ticket","data":{"ticket":"private"}}),
        )
        .unwrap();
        assert!(!summary.to_string().contains("private"));
    }

    #[test]
    fn interrupted_wait_returns_without_waiting_for_the_next_heartbeat() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut signal: InterruptSignal = Box::pin(std::future::ready(Ok(())));
        assert!(wait_for_interrupt(&runtime, &mut signal, Duration::from_secs(3600)).unwrap());
        let mut pending: InterruptSignal = Box::pin(std::future::pending());
        assert!(!wait_for_interrupt(&runtime, &mut pending, Duration::ZERO).unwrap());
    }

    #[test]
    fn accepted_room_with_missing_stream_id_is_identifiable_without_exposing_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("live.json");
        let output = SessionOutput::prepare(&path).unwrap();
        let mut response = created_room_fixture();
        response["data"]["room"]
            .as_object_mut()
            .unwrap()
            .remove("stream_id_str");
        let error = save_created_session(&response, "789", Some(output)).unwrap_err();
        assert!(error.contains("123"));
        assert!(!error.contains("opaque"));
        assert!(!error.contains("push.example"));
        assert!(!path.exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn accepted_string_status_can_create_and_save_a_live_session() {
        let mut response = created_room_fixture();
        response["status_code"] = json!("0");
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        let output = SessionOutput::prepare(&path).unwrap();
        let summary = save_created_session(&response, "789", Some(output)).unwrap();
        assert_eq!(summary["session_saved"], true);
        assert_eq!(summary["status_code"], 0);
        let saved: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved["stream_id"], "456");
        response["status_code"] = json!("20003");
        assert!(create_room_summary(&response).is_err());
    }

    #[test]
    fn failed_partial_write_keeps_private_recovery_file_and_marks_incomplete_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        let output = SessionOutput::prepare(&path).unwrap();
        let recovery = output.pending.path().to_owned();
        let session = LiveSession::from_response(&created_room_fixture(), "789").unwrap();
        let error = output
            .save_with_io(
                &session,
                |file, bytes| {
                    file.write_all(&bytes[..7])?;
                    Err(std::io::Error::other("private-write-failure"))
                },
                |_| panic!("sync must not run after failed write"),
            )
            .unwrap_err();
        assert!(error.contains("可能不完整"));
        assert!(error.contains(recovery.to_str().unwrap()));
        assert!(!error.contains("private-write-failure"));
        assert!(!error.contains("opaque"));
        assert!(!path.exists());
        assert_eq!(std::fs::read(recovery).unwrap().len(), 7);
    }

    #[test]
    fn failed_sync_keeps_complete_json_in_private_recovery_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.json");
        let output = SessionOutput::prepare(&path).unwrap();
        let recovery = output.pending.path().to_owned();
        let session = LiveSession::from_response(&created_room_fixture(), "789").unwrap();
        let error = output
            .save_with_io(
                &session,
                |file, bytes| file.write_all(bytes),
                |_| Err(std::io::Error::other("private-sync-failure")),
            )
            .unwrap_err();
        assert!(error.contains("JSON 已完整写入"));
        assert!(error.contains(recovery.to_str().unwrap()));
        assert!(!error.contains("private-sync-failure"));
        assert!(!error.contains("opaque"));
        assert!(!path.exists());
        let recovered: Value = serde_json::from_slice(&std::fs::read(&recovery).unwrap()).unwrap();
        assert_eq!(recovered, session.to_value());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(recovery).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn missing_string_identifiers_fall_back_to_numbers_but_invalid_values_do_not() {
        for missing in [Value::Null, json!("")] {
            let mut response = created_room_fixture();
            let room = &mut response["data"]["room"];
            room["id_str"] = missing.clone();
            room["id"] = json!(123);
            room["stream_id_str"] = missing.clone();
            room["stream_id"] = json!(456);
            room["owner"]["id_str"] = missing;
            room["owner"]["id"] = json!(789);
            let session = LiveSession::from_response(&response, "789").unwrap();
            assert_eq!(session.room_id, "123");
            assert_eq!(session.stream_id, "456");
            for field in ["id_str", "stream_id_str"] {
                let mut invalid = response.clone();
                invalid["data"]["room"][field] = json!("private-invalid-id");
                assert!(LiveSession::from_response(&invalid, "789").is_err());
            }
        }
    }

    #[test]
    fn finish_readback_without_target_room_status_is_an_incomplete_operation() {
        for response in [
            json!({"data":{"id_str":"123"}}),
            json!({"data":{"id_str":"124","status":4}}),
        ] {
            let mut summary = json!({"request_accepted":true,"status_reported":"finish"});
            assert!(finish_verification(&mut summary, Ok(response), "123").is_err());
            assert_eq!(summary["request_accepted"], true);
            assert_eq!(summary["verification"], "room_info_unavailable");
            assert!(summary["observed_room_status"].is_null());
        }
        let mut summary = json!({"request_accepted":true,"status_reported":"finish"});
        finish_verification(
            &mut summary,
            Ok(json!({"data":{"id_str":null,"id":123,"status":"4"}})),
            "123",
        )
        .unwrap();
        assert_eq!(summary["observed_room_status"], 4);
    }
}
