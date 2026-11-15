//! Minimal protobuf wire encoding for ByteDance IM (`imapi.snssdk.com`).
//! Field numbers and cmd values come from the 40.8.0 Wire-generated adapters
//! (`SendMessageRequestBody`, `Request`, `RequestBody`, `IMCMD`).

pub const CMD_SEND_MESSAGE: i32 = 100;
pub const CMD_GET_MESSAGES_BY_USER: i32 = 200;
pub const CMD_GET_MESSAGES_BY_USER_INIT_V2: i32 = 203;
pub const CMD_GET_CONVERSATION_INFO_V2: i32 = 608;
pub const CMD_CREATE_CONVERSATION_V2: i32 = 609;
pub const CMD_GET_TICKET: i32 = 2005;

/// `MESSAGE_TYPE_TEXT.getValue()` = 10001 (`MessageType.<clinit>`).
/// Douyin 1:1 chat texts observed in pull use legacy value 7 + aweType JSON instead.
pub const MESSAGE_TYPE_TEXT: i32 = 10001;
/// `LEGACY_MESSAGE_TYPE_TEXT.getValue()` — seen on live 1:1 text bubbles (aweType=700).
pub const MESSAGE_TYPE_LEGACY_TEXT: i32 = 7;
/// `Refer.ANDROID` from obfuscated `LX/1Trj`.
pub const REFER_ANDROID: i32 = 1;
/// `AuthType.CERT_AUTH` — server error text `authType must be certAuth`.
pub const AUTH_TYPE_CERT: i32 = 3;
/// `AuthType.PASSPORT_CERT_AUTH` — used when `RequestZTInfo` is non-null (`X.1Tft.Y2`).
pub const AUTH_TYPE_PASSPORT_CERT: i32 = 4;
/// `TICKET_TYPE_APP` from obfuscated `LX/1Tzz`.
pub const TICKET_TYPE_APP: i32 = 1;
/// 1:1 conversation type used across CreateConversationV2 / SendMessage.
pub const CONVERSATION_TYPE_ONE_TO_ONE: i32 = 1;
/// `Request.sdk_version` from `X.1Tft.N2` const-string (not the cloud-token `-3.0.4` suffix).
pub const IM_SDK_VERSION: &str = "5.0.3.0-rc.11-SNAPSHOT";
/// `Request.build_number` from the same `N2` site (`"5030"`).
pub const IM_BUILD_NUMBER: &str = "5030";

/// Per-cmd HTTP path from `LX/1Tg9` SparseArray (APK 40.8.0). Not all cmds share
/// `v1/message/send` — posting pull to send yields the JSON stub `Method:0`.
pub fn http_path_for_cmd(cmd: i32) -> Option<&'static str> {
    Some(match cmd {
        CMD_SEND_MESSAGE => "/v1/message/send",
        CMD_GET_MESSAGES_BY_USER => "/v1/message/get_by_user",
        CMD_GET_MESSAGES_BY_USER_INIT_V2 => "/v2/message/get_by_user_init",
        CMD_GET_CONVERSATION_INFO_V2 => "/v2/conversation/get_info",
        CMD_CREATE_CONVERSATION_V2 => "/v2/conversation/create",
        CMD_GET_TICKET => "/v1/conversation/get_ticket",
        _ => return None,
    })
}

pub struct ImRequestParts {
    pub cmd: i32,
    pub sequence_id: i64,
    pub sdk_version: String,
    pub token: String,
    pub inbox_type: i32,
    pub build_number: String,
    pub device_id: String,
    pub device_platform: String,
    pub device_type: String,
    pub os_version: String,
    pub version_code: String,
    /// `Request.headers` tag 15; APK N2 always inserts `flow-tag=new`.
    pub headers: Vec<(String, String)>,
    /// `Request.biz` tag 21 / `Request.access` tag 22 from IClientBridge.
    pub biz: Option<String>,
    pub access: Option<String>,
    /// `Request.auth_type` tag 18; only when ZT/cert auth is active.
    pub auth_type: Option<i32>,
    /// Passport ticket-guard fields from `RequestZTInfo` (`X.1Tsg.getZTInfo`):
    /// `bd-ticket-guard-client-cert` → tag 100, `client-data` → 101,
    /// `ree-public-key` → 102, `iteration-version` → 103.
    pub client_cert: Option<String>,
    pub client_data: Option<String>,
    pub ree_public_key: Option<String>,
    pub iter_version: Option<String>,
    pub body: Vec<u8>,
}

pub fn encode_im_request(parts: &ImRequestParts) -> Vec<u8> {
    let mut out = Vec::new();
    write_varint_field(&mut out, 1, parts.cmd as u64);
    write_varint_field(&mut out, 2, parts.sequence_id as u64);
    write_string_field(&mut out, 3, &parts.sdk_version);
    write_string_field(&mut out, 4, &parts.token);
    write_varint_field(&mut out, 5, REFER_ANDROID as u64);
    write_varint_field(&mut out, 6, parts.inbox_type as u64);
    if !parts.build_number.is_empty() {
        write_string_field(&mut out, 7, &parts.build_number);
    }
    write_bytes_field(&mut out, 8, &parts.body);
    write_string_field(&mut out, 9, &parts.device_id);
    write_string_field(&mut out, 11, &parts.device_platform);
    write_string_field(&mut out, 12, &parts.device_type);
    write_string_field(&mut out, 13, &parts.os_version);
    write_string_field(&mut out, 14, &parts.version_code);
    if !parts.headers.is_empty() {
        // protobuf map<string,string>: repeated entries as length-delimited messages
        // with key=1, value=2 (Wire Internal map encoding).
        for (k, v) in &parts.headers {
            let mut entry = Vec::new();
            write_string_field(&mut entry, 1, k);
            write_string_field(&mut entry, 2, v);
            write_bytes_field(&mut out, 15, &entry);
        }
    }
    if let Some(auth) = parts.auth_type {
        write_varint_field(&mut out, 18, auth as u64);
    }
    if let Some(biz) = &parts.biz {
        write_string_field(&mut out, 21, biz);
    }
    if let Some(access) = &parts.access {
        write_string_field(&mut out, 22, access);
    }
    // Tags from `X.1Tpm.encodedSize` (Request ADAPTER).
    if let Some(v) = &parts.client_cert {
        write_string_field(&mut out, 100, v);
    }
    if let Some(v) = &parts.client_data {
        write_string_field(&mut out, 101, v);
    }
    if let Some(v) = &parts.ree_public_key {
        write_string_field(&mut out, 102, v);
    }
    if let Some(v) = &parts.iter_version {
        write_string_field(&mut out, 103, v);
    }
    out
}

/// `RequestBody.send_message_body` = tag 100.
/// Field tags from `LX/1Tnc.encode` (SendMessageRequestBody ADAPTER).
#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the recovered protobuf fields"
)]
pub fn encode_send_message_body(
    conversation_id: &str,
    conversation_type: i32,
    conversation_short_id: i64,
    content: &str,
    message_type: i32,
    ticket: &str,
    client_message_id: &str,
    to_user_id: Option<i64>,
    ext: &[(String, String)],
) -> Vec<u8> {
    let mut msg = Vec::new();
    write_string_field(&mut msg, 1, conversation_id);
    write_varint_field(&mut msg, 2, conversation_type as u64);
    write_varint_field(&mut msg, 3, conversation_short_id as u64);
    write_string_field(&mut msg, 4, content);
    // tag 5 = ext map (seen keys on live 1:1 bubbles / send path).
    for (k, v) in ext {
        let mut entry = Vec::new();
        write_string_field(&mut entry, 1, k);
        write_string_field(&mut entry, 2, v);
        write_bytes_field(&mut msg, 5, &entry);
    }
    write_varint_field(&mut msg, 6, message_type as u64);
    write_string_field(&mut msg, 7, ticket);
    write_string_field(&mut msg, 8, client_message_id);
    if let Some(uid) = to_user_id {
        write_varint_field(&mut msg, 33, uid as u64);
    }
    let mut body = Vec::new();
    write_bytes_field(&mut body, 100, &msg);
    body
}

/// `RequestBody.messages_per_user_body` = tag 200.
/// Tags from `LX/1ToP.encode`: cursor=1, limit=2, interval=3, …
pub fn encode_messages_per_user_body(cursor: i64, limit: i32) -> Vec<u8> {
    let mut msg = Vec::new();
    write_varint_field(&mut msg, 1, cursor as u64);
    write_varint_field(&mut msg, 2, limit as u64);
    let mut body = Vec::new();
    write_bytes_field(&mut body, 200, &msg);
    body
}

/// `RequestBody.create_conversation_v2_body` = tag 609.
/// Tags from `LX/1To8.encode`: conversation_type=1, participants=2 (repeated),
/// idempotent_id=4.
pub fn encode_create_conversation_v2_body(participant_uid: i64, idempotent_id: &str) -> Vec<u8> {
    let mut msg = Vec::new();
    write_varint_field(&mut msg, 1, CONVERSATION_TYPE_ONE_TO_ONE as u64);
    write_varint_field(&mut msg, 2, participant_uid as u64);
    write_string_field(&mut msg, 4, idempotent_id);
    let mut body = Vec::new();
    write_bytes_field(&mut body, 609, &msg);
    body
}

/// `RequestBody.get_ticket_body` = tag 2005.
/// Tags from `LX/1Tzy.encode`: ticket_type=1, conversation_type=2, to_id=4,
/// conversation_short_id=5.
pub fn encode_get_ticket_body(to_user_id: i64, conversation_short_id: Option<i64>) -> Vec<u8> {
    let mut msg = Vec::new();
    write_varint_field(&mut msg, 1, TICKET_TYPE_APP as u64);
    write_varint_field(&mut msg, 2, CONVERSATION_TYPE_ONE_TO_ONE as u64);
    write_varint_field(&mut msg, 4, to_user_id as u64);
    if let Some(sid) = conversation_short_id {
        write_varint_field(&mut msg, 5, sid as u64);
    }
    let mut body = Vec::new();
    write_bytes_field(&mut body, 2005, &msg);
    body
}

/// `RequestBody.get_conversation_info_v2_body` = tag 608.
/// Tags from `LX/1TsI.encode`: conversation_id=1, conversation_short_id=2,
/// conversation_type=3, need_coreinfo_related=4, need_message=10.
pub fn encode_get_conversation_info_v2_body(
    conversation_id: &str,
    conversation_short_id: i64,
    conversation_type: i32,
) -> Vec<u8> {
    let mut msg = Vec::new();
    write_string_field(&mut msg, 1, conversation_id);
    write_varint_field(&mut msg, 2, conversation_short_id as u64);
    write_varint_field(&mut msg, 3, conversation_type as u64);
    write_varint_field(&mut msg, 4, 1); // need_coreinfo_related
    write_varint_field(&mut msg, 10, 1); // need_message
    let mut body = Vec::new();
    write_bytes_field(&mut body, 608, &msg);
    body
}

/// Decode IM `Response` tags recovered from `LX/1TpT.encode`.
pub fn decode_im_response(bytes: &[u8]) -> serde_json::Value {
    let summary = summarize_protobuf(bytes);
    let mut out = serde_json::json!({
        "size": summary["size"],
        "fields": summary["fields"],
    });
    if let Some(arr) = summary["fields"].as_array() {
        for f in arr {
            let tag = f.get("field").and_then(|v| v.as_u64()).unwrap_or(0);
            match tag {
                1 => {
                    if let Some(v) = f.get("varint") {
                        out["cmd"] = v.clone();
                    }
                }
                2 => {
                    if let Some(v) = f.get("varint") {
                        out["sequence_id"] = v.clone();
                    }
                }
                3 => {
                    if let Some(v) = f.get("varint") {
                        out["status_code"] = v.clone();
                        out["ok"] = serde_json::json!(v.as_u64() == Some(0));
                    }
                }
                4 => {
                    if let Some(v) = f.get("string") {
                        out["error_desc"] = v.clone();
                    }
                }
                7 => {
                    if let Some(v) = f.get("string") {
                        out["log_id"] = v.clone();
                    }
                }
                6 => {
                    if let Some(nested) = f.get("nested") {
                        out["body"] = nested.clone();
                    } else if let Some(n) = f.get("bytes") {
                        out["body_bytes"] = n.clone();
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// Best-effort protobuf field walk for CLI summaries (not a full decoder).
pub fn summarize_protobuf(bytes: &[u8]) -> serde_json::Value {
    let mut fields = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let (key, n) = match read_varint(bytes, i) {
            Some(v) => v,
            None => break,
        };
        i = n;
        let field = (key >> 3) as u32;
        let wire = (key & 7) as u32;
        match wire {
            0 => {
                let (v, n) = match read_varint(bytes, i) {
                    Some(v) => v,
                    None => break,
                };
                i = n;
                fields.push(serde_json::json!({"field": field, "varint": v}));
            }
            1 => {
                if i + 8 > bytes.len() {
                    break;
                }
                i += 8;
                fields.push(serde_json::json!({"field": field, "fixed64": true}));
            }
            2 => {
                let (len, n) = match read_varint(bytes, i) {
                    Some(v) => v,
                    None => break,
                };
                i = n;
                let len = len as usize;
                if i + len > bytes.len() {
                    break;
                }
                let slice = &bytes[i..i + len];
                i += len;
                if let Ok(text) = std::str::from_utf8(slice)
                    && text
                        .chars()
                        .all(|c| !c.is_control() || c == '\n' || c == '\t')
                    && text.len() <= 512
                {
                    fields.push(serde_json::json!({"field": field, "string": text}));
                    continue;
                }
                fields.push(serde_json::json!({
                    "field": field,
                    "bytes": len,
                    "nested": summarize_protobuf(slice),
                }));
            }
            5 => {
                if i + 4 > bytes.len() {
                    break;
                }
                i += 4;
                fields.push(serde_json::json!({"field": field, "fixed32": true}));
            }
            _ => break,
        }
        if fields.len() > 64 {
            break;
        }
    }
    serde_json::json!({ "fields": fields, "size": bytes.len() })
}

fn write_varint_field(out: &mut Vec<u8>, field: u32, value: u64) {
    write_varint(out, u64::from(field << 3));
    write_varint(out, value);
}

fn write_string_field(out: &mut Vec<u8>, field: u32, value: &str) {
    write_bytes_field(out, field, value.as_bytes());
}

fn write_bytes_field(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    write_varint(out, u64::from((field << 3) | 2));
    write_varint(out, value.len() as u64);
    out.extend_from_slice(value);
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
            out.push(byte);
        } else {
            out.push(byte);
            break;
        }
    }
}

fn read_varint(data: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while i < data.len() {
        let b = data[i];
        i += 1;
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Some((result, i));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_send_message_with_cmd_100_body_tag() {
        let body = encode_send_message_body(
            "0:1:2",
            1,
            99,
            r#"{"text":"hi"}"#,
            MESSAGE_TYPE_TEXT,
            "ticket",
            "client-1",
            Some(123),
            &[],
        );
        let req = encode_im_request(&ImRequestParts {
            cmd: CMD_SEND_MESSAGE,
            sequence_id: 1,
            sdk_version: IM_SDK_VERSION.into(),
            token: "tok".into(),
            inbox_type: 0,
            build_number: IM_BUILD_NUMBER.into(),
            device_id: "1".into(),
            device_platform: "android".into(),
            device_type: "phone".into(),
            os_version: "16".into(),
            version_code: "400800".into(),
            headers: vec![("flow-tag".into(), "new".into())],
            biz: Some("douyin".into()),
            access: Some("douyin_main".into()),
            auth_type: Some(AUTH_TYPE_CERT),
            client_cert: None,
            client_data: None,
            ree_public_key: None,
            iter_version: None,
            body,
        });
        assert!(!req.is_empty());
        let summary = summarize_protobuf(&req);
        assert!(summary["size"].as_u64().unwrap() > 20);
        assert_eq!(
            http_path_for_cmd(CMD_GET_MESSAGES_BY_USER),
            Some("/v1/message/get_by_user")
        );
        assert_eq!(
            http_path_for_cmd(CMD_SEND_MESSAGE),
            Some("/v1/message/send")
        );
    }

    #[test]
    fn write_varint_roundtrips_small_values() {
        let mut buf = Vec::new();
        write_varint(&mut buf, 0);
        write_varint(&mut buf, 127);
        write_varint(&mut buf, 128);
        assert_eq!(buf, vec![0, 127, 0x80, 0x01]);
    }
}
