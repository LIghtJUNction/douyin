//! OBS transport is independent of the Android signing and room lifecycle.
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use std::{io::Read, time::Duration};

#[derive(Args)]
pub struct LiveArgs {
    #[command(subcommand)]
    command: LiveCommand,
}

#[derive(Subcommand)]
enum LiveCommand {
    /// 独立查询当前安卓账号的直播权限配置，不创建直播间
    Permissions,
    /// 使用当前采集传输查询公开直播间
    Info { room_id: String },
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
        LiveCommand::Info { room_id } => {
            let response = crate::net::NativeClient::saved()?.live_room_info(&room_id)?;
            println!("{}", room_summary(&response, &room_id)?);
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

fn read_private(path: &str) -> Result<String, String> {
    let reader: Box<dyn Read> = if path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(std::fs::File::open(path).map_err(|_| "无法读取 OBS 输入文件")?)
    };
    let mut text = String::new();
    reader
        .take(16385)
        .read_to_string(&mut text)
        .map_err(|_| "OBS 输入不是有效 UTF-8")?;
    if text.len() > 16384 {
        return Err("OBS 输入超过 16 KiB 限制".into());
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
    let id = room["id_str"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| room["id"].as_u64().map(|id| id.to_string()))
        .ok_or("直播接口未返回可识别的房间 ID")?;
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
}
