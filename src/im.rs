//! Native Douyin IM helpers recovered from APK 40.8.0.
//!
//! - Aweme REST: config / stranger privilege / enter_chat / can-send / cloud-token
//! - IM SDK HTTP: `https://imapi.snssdk.com` + per-cmd path from `LX/1Tg9`
//!   (`SEND_MESSAGE` → `/v1/message/send`, `GET_MESSAGES_BY_USER` →
//!   `/v1/message/get_by_user`, …) with `application/x-protobuf` `Request`
//!
//! Push/receive realtime still uses Frontier WebSocket in the app; this CLI
//! exposes the HTTP pull/send path the SDK uses when `send_msg_by_http` is on.

use clap::{Args, Subcommand};
use serde_json::{Value, json};
use uuid_like_client_id as client_id;

use crate::im_pb::{
    self, CMD_CREATE_CONVERSATION_V2, CMD_GET_CONVERSATION_INFO_V2, CMD_GET_MESSAGES_BY_USER,
    CMD_GET_TICKET, CMD_SEND_MESSAGE, CONVERSATION_TYPE_ONE_TO_ONE, MESSAGE_TYPE_LEGACY_TEXT,
    MESSAGE_TYPE_TEXT,
};
use crate::net::NativeClient;

mod uuid_like_client_id {
    use std::time::{SystemTime, UNIX_EPOCH};

    /// Client message id shape used by the app (opaque unique string).
    pub fn new() -> String {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        format!("cli-{ms}-{:08x}", (ms as u32).wrapping_mul(0x9e37_79b9))
    }
}

#[derive(Args)]
pub struct ImArgs {
    #[command(subcommand)]
    command: ImCommand,
}

#[derive(Subcommand)]
enum ImCommand {
    /// GET /aweme/v1/im/config/（ImApi.getResourceInfo）
    Config,
    /// GET /aweme/v1/im/cloud/token/（IM SDK 独立 token，供 imapi Request.token）
    CloudToken,
    /// GET /aweme/v1/im/get/stranger_priv/
    StrangerPriv,
    /// GET im/spotlight/multi_relation/?sec_to_user_id=…（能否私信）
    CanSend {
        #[arg(long)]
        sec_user_id: String,
    },
    /// POST /aweme/v1/im/service/enter_chat/（Body = LX/0UGy Gson 字段）
    EnterChat {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        conversation_short_id: i64,
        #[arg(long, default_value_t = CONVERSATION_TYPE_ONE_TO_ONE)]
        conversation_type: i32,
        #[arg(long, default_value_t = 0)]
        source: i32,
    },
    /// imapi protobuf：GET_TICKET（cmd=2005）
    Ticket {
        #[arg(long)]
        to_user_id: i64,
        #[arg(long)]
        conversation_short_id: Option<i64>,
        #[arg(long)]
        yes: bool,
    },
    /// imapi protobuf：CREATE_CONVERSATION_V2（cmd=609）
    CreateConversation {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        yes: bool,
    },
    /// imapi protobuf：GET_CONVERSATION_INFO_V2（cmd=608）
    ConversationInfo {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        conversation_short_id: i64,
        #[arg(long, default_value_t = CONVERSATION_TYPE_ONE_TO_ONE)]
        conversation_type: i32,
        #[arg(long)]
        yes: bool,
    },
    /// imapi protobuf：GET_MESSAGES_BY_USER（cmd=200）拉会话消息增量
    Pull {
        #[arg(long, default_value_t = 0)]
        cursor: i64,
        #[arg(long, default_value_t = 20)]
        limit: i32,
        #[arg(long)]
        yes: bool,
    },
    /// imapi protobuf：SEND_MESSAGE（cmd=100）发文本私信
    Send {
        #[arg(long)]
        conversation_id: String,
        #[arg(long)]
        conversation_short_id: i64,
        #[arg(long)]
        text: String,
        /// 会话 ticket；可先用 `im ticket` / 进会话后从 SDK 响应取
        #[arg(long)]
        ticket: String,
        #[arg(long)]
        to_user_id: Option<i64>,
        #[arg(long, default_value_t = CONVERSATION_TYPE_ONE_TO_ONE)]
        conversation_type: i32,
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(args: ImArgs) -> Result<(), String> {
    let client = NativeClient::saved()?;
    let value = match args.command {
        ImCommand::Config => config(&client)?,
        ImCommand::CloudToken => cloud_token(&client)?,
        ImCommand::StrangerPriv => stranger_priv(&client)?,
        ImCommand::CanSend { sec_user_id } => can_send(&client, &sec_user_id)?,
        ImCommand::EnterChat {
            conversation_id,
            conversation_short_id,
            conversation_type,
            source,
        } => enter_chat(
            &client,
            &conversation_id,
            conversation_short_id,
            conversation_type,
            source,
        )?,
        ImCommand::Ticket {
            to_user_id,
            conversation_short_id,
            yes,
        } => {
            require_yes(yes)?;
            imapi_ticket(&client, to_user_id, conversation_short_id)?
        }
        ImCommand::CreateConversation { user_id, yes } => {
            require_yes(yes)?;
            imapi_create_conversation(&client, user_id)?
        }
        ImCommand::ConversationInfo {
            conversation_id,
            conversation_short_id,
            conversation_type,
            yes,
        } => {
            require_yes(yes)?;
            imapi_conversation_info(
                &client,
                &conversation_id,
                conversation_short_id,
                conversation_type,
            )?
        }
        ImCommand::Pull { cursor, limit, yes } => {
            require_yes(yes)?;
            imapi_pull(&client, cursor, limit)?
        }
        ImCommand::Send {
            conversation_id,
            conversation_short_id,
            text,
            ticket,
            to_user_id,
            conversation_type,
            yes,
        } => {
            require_yes(yes)?;
            imapi_send(
                &client,
                &conversation_id,
                conversation_short_id,
                conversation_type,
                &text,
                &ticket,
                to_user_id,
            )?
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|_| "IM 结果序列化失败")?
    );
    Ok(())
}

fn require_yes(yes: bool) -> Result<(), String> {
    if yes {
        Ok(())
    } else {
        Err("写操作或 IM protobuf 请求会触达账号会话，需要 --yes".into())
    }
}

pub(crate) fn config(client: &NativeClient) -> Result<Value, String> {
    // ImApi.getResourceInfo may omit status_code while still returning IM settings + log_pb.
    client.get_lenient("/aweme/v1/im/config/", &[])
}

pub(crate) fn cloud_token(client: &NativeClient) -> Result<Value, String> {
    client.get("/aweme/v1/im/cloud/token/", &[])
}

pub(crate) fn stranger_priv(client: &NativeClient) -> Result<Value, String> {
    client.get("/aweme/v1/im/get/stranger_priv/", &[])
}

pub(crate) fn can_send(client: &NativeClient, sec_user_id: &str) -> Result<Value, String> {
    if sec_user_id.trim().is_empty() {
        return Err("sec_user_id 不能为空".into());
    }
    client.get(
        "/aweme/v1/im/spotlight/multi_relation/",
        &[("sec_to_user_id".into(), sec_user_id.into())],
    )
}

/// Body fields from `LX/0UGy` `@SerializedName` on enter_chat.
pub(crate) fn enter_chat(
    client: &NativeClient,
    conversation_id: &str,
    conversation_short_id: i64,
    conversation_type: i32,
    source: i32,
) -> Result<Value, String> {
    if conversation_id.trim().is_empty() {
        return Err("conversation_id 不能为空".into());
    }
    if conversation_short_id <= 0 {
        return Err("conversation_short_id 必须为正整数".into());
    }
    let body = json!({
        "conversation_id": conversation_id,
        "conversation_short_id": conversation_short_id,
        "conversation_type": conversation_type,
        "source": source,
    });
    client.post_json(
        "/aweme/v1/im/service/enter_chat/",
        &[("im-request-tag".into(), "im-cli".into())],
        &body,
    )
}

fn imapi_ticket(
    client: &NativeClient,
    to_user_id: i64,
    conversation_short_id: Option<i64>,
) -> Result<Value, String> {
    if to_user_id <= 0 {
        return Err("to_user_id 必须为正整数".into());
    }
    if conversation_short_id.is_some_and(|v| v <= 0) {
        return Err("conversation_short_id 必须为正整数".into());
    }
    let body = im_pb::encode_get_ticket_body(to_user_id, conversation_short_id);
    imapi_call(client, CMD_GET_TICKET, body)
}

fn imapi_create_conversation(client: &NativeClient, user_id: i64) -> Result<Value, String> {
    if user_id <= 0 {
        return Err("user_id 必须为正整数".into());
    }
    let self_id: i64 = client
        .account_id()?
        .parse()
        .map_err(|_| "当前账号 id 不是整数")?;
    let mut pair = [self_id, user_id];
    pair.sort_unstable();
    let idempotent = format!("{}:{}", pair[0], pair[1]);
    let body = im_pb::encode_create_conversation_v2_body(user_id, &idempotent);
    imapi_call(client, CMD_CREATE_CONVERSATION_V2, body)
}

fn imapi_conversation_info(
    client: &NativeClient,
    conversation_id: &str,
    conversation_short_id: i64,
    conversation_type: i32,
) -> Result<Value, String> {
    if conversation_id.trim().is_empty() {
        return Err("conversation_id 不能为空".into());
    }
    if conversation_short_id <= 0 {
        return Err("conversation_short_id 必须为正整数".into());
    }
    let body = im_pb::encode_get_conversation_info_v2_body(
        conversation_id,
        conversation_short_id,
        conversation_type,
    );
    imapi_call(client, CMD_GET_CONVERSATION_INFO_V2, body)
}

fn imapi_pull(client: &NativeClient, cursor: i64, limit: i32) -> Result<Value, String> {
    if !(1..=100).contains(&limit) {
        return Err("limit 需要在 1..=100".into());
    }
    if cursor < 0 {
        return Err("cursor 不能为负".into());
    }
    let body = im_pb::encode_messages_per_user_body(cursor, limit);
    imapi_call(client, CMD_GET_MESSAGES_BY_USER, body)
}

fn imapi_send(
    client: &NativeClient,
    conversation_id: &str,
    conversation_short_id: i64,
    conversation_type: i32,
    text: &str,
    ticket: &str,
    to_user_id: Option<i64>,
) -> Result<Value, String> {
    if conversation_id.trim().is_empty() {
        return Err("conversation_id 不能为空".into());
    }
    if conversation_short_id <= 0 {
        return Err("conversation_short_id 必须为正整数".into());
    }
    let text = text.trim();
    if text.is_empty() || text.len() > 2048 {
        return Err("私信文本不能为空且不超过 2048 字节".into());
    }
    if ticket.trim().is_empty() {
        return Err("ticket 不能为空；请先用 im ticket / 进会话获取".into());
    }
    // TextContent.LJJLJLI → BaseContent.type=700 with @SerializedName("aweType");
    // TextContent.heartType @SerializedName("type") defaults to 0; itemType ctor=-1;
    // msgHint ctor="". Pull stores the same shape. Wire msg type: pull shows
    // LEGACY 7; DOUYIN_IM_MSG_TYPE=10001 forces MESSAGE_TYPE_TEXT.
    let content = json!({
        "type": 0,
        "instruction_type": 0,
        "item_type_local": -1,
        "text": text,
        "createdAt": 0,
        "is_card": false,
        "msgHint": "",
        "aweType": 700,
    })
    .to_string();
    let msg_type = match std::env::var("DOUYIN_IM_MSG_TYPE").ok().as_deref() {
        Some("10001") => MESSAGE_TYPE_TEXT,
        _ => MESSAGE_TYPE_LEGACY_TEXT,
    };
    let client_message_id = client_id::new();
    let body = im_pb::encode_send_message_body(
        conversation_id,
        conversation_type,
        conversation_short_id,
        &content,
        msg_type,
        ticket,
        &client_message_id,
        to_user_id,
        &send_ext(&client_message_id),
    );
    imapi_call(client, CMD_SEND_MESSAGE, body)
}

/// Client-side `SendMessageRequestBody.ext` keys seen on live 1:1 text bubbles.
fn send_ext(client_message_id: &str) -> Vec<(String, String)> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
        .to_string();
    vec![
        ("s:client_message_id".into(), client_message_id.into()),
        ("a:enter_method".into(), "click_message".into()),
        ("chat_scene".into(), "normal".into()),
        ("a:msg_scene".into(), "1".into()),
        ("s:chat_tag".into(), "private".into()),
        ("s:is_stranger".into(), "false".into()),
        ("s:mode".into(), "0".into()),
        ("s:ticket_mode".into(), "0".into()),
        ("a:relation_type".into(), "1:1".into()),
        ("a:access".into(), "douyin_main".into()),
        ("a:biz".into(), "douyin".into()),
        ("source_aid".into(), "1128".into()),
        ("s:s_aid".into(), "1128".into()),
        ("s:biz_aid".into(), "1128".into()),
        ("im_sdk_client_send_msg_time".into(), now_ms.clone()),
        ("im_client_send_msg_time".into(), now_ms),
        ("a:sync2dx".into(), "1".into()),
        ("s:sync_2_newdx".into(), "1".into()),
        ("s:saas_sdk".into(), "false".into()),
        ("a:ntp_ready".into(), "2".into()),
        ("a:smp_token_fetch".into(), "0".into()),
        ("a:plv".into(), "0".into()),
    ]
}

fn imapi_call(client: &NativeClient, cmd: i32, body: Vec<u8>) -> Result<Value, String> {
    let path = im_pb::http_path_for_cmd(cmd)
        .ok_or_else(|| format!("未收录 cmd={cmd} 的 imapi HTTP 路径（见 LX/1Tg9）"))?;
    let mut parts = client.im_request_parts(cmd, body)?;
    // Prefer IM cloud token when the Aweme helper returns one (seen path /aweme/v1/im/cloud/token/).
    if let Ok(cloud) = cloud_token(client)
        && let Some(token) = cloud
            .pointer("/token")
            .or_else(|| cloud.pointer("/data/token"))
            .or_else(|| cloud.pointer("/im_token"))
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
    {
        parts.token = token.into();
    }
    // SEND/CREATE: server requires certAuth. With RequestZTInfo the app uses
    // PASSPORT_CERT_AUTH + bd-ticket-guard-* (`X.1Tft.Y2` / `X.1Tsg.getZTInfo`).
    // Optional DOUYIN_IM_ZT_JSON supplies those four fields for rooted probes.
    if matches!(
        cmd,
        im_pb::CMD_SEND_MESSAGE | im_pb::CMD_CREATE_CONVERSATION_V2
    ) {
        apply_passport_zt(&mut parts)?;
    }
    let bytes = im_pb::encode_im_request(&parts);
    let raw = client.post_imapi_protobuf(path, &bytes)?;
    let response = if raw.first() == Some(&b'{') {
        serde_json::from_slice::<Value>(&raw).unwrap_or_else(|_| {
            json!({
                "parse": "json_failed",
                "summary": im_pb::summarize_protobuf(&raw),
            })
        })
    } else {
        im_pb::decode_im_response(&raw)
    };
    Ok(json!({
        "cmd": cmd,
        "path": path,
        "request_size": bytes.len(),
        "response": response,
        "response_hex_prefix": hex_prefix(&raw, 64),
    }))
}

fn hex_prefix(bytes: &[u8], n: usize) -> String {
    bytes
        .iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

/// Apply passport ticket-guard fields onto an IM Request (tags 18/100–103).
/// `DOUYIN_IM_ZT_JSON` must be a JSON object with string fields
/// `client_cert`, `client_data`, `ree_public_key`, `iter_version`
/// (seen mapping from `RequestZTInfo` / `bd-ticket-guard-*`).
fn apply_passport_zt(parts: &mut crate::im_pb::ImRequestParts) -> Result<(), String> {
    let path = match std::env::var("DOUYIN_IM_ZT_JSON") {
        Ok(p) if !p.is_empty() => p,
        _ => {
            // CERT_AUTH alone makes the server base64-decode the session token and
            // fail at the `--` separator; keep the typed error until ZT is supplied.
            return Ok(());
        }
    };
    let raw = std::fs::read_to_string(&path).map_err(|_| "无法读取 DOUYIN_IM_ZT_JSON")?;
    if raw.len() > 256 * 1024 {
        return Err("DOUYIN_IM_ZT_JSON 过大".into());
    }
    let value: Value = serde_json::from_str(&raw).map_err(|_| "DOUYIN_IM_ZT_JSON 不是有效 JSON")?;
    let field = |name: &str| -> Result<String, String> {
        let text = value
            .get(name)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("DOUYIN_IM_ZT_JSON 缺少 {name}"))?;
        if text.len() > 65536 || text.chars().any(char::is_control) {
            return Err(format!("DOUYIN_IM_ZT_JSON.{name} 格式无效"));
        }
        Ok(text.to_owned())
    };
    parts.auth_type = Some(im_pb::AUTH_TYPE_PASSPORT_CERT);
    parts.client_cert = Some(field("client_cert")?);
    parts.client_data = Some(field("client_data")?);
    parts.ree_public_key = Some(field("ree_public_key")?);
    parts.iter_version = Some(field("iter_version")?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_targets() {
        assert!(require_yes(false).is_err());
        assert!(require_yes(true).is_ok());
    }
}
