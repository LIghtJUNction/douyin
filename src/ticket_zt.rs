//! Passport ticket-guard materials for paths that require cert auth.
//!
//! Seen in APK 40.8.0:
//! - IM `RequestZTInfo` (`X.1Tsg.getZTInfo`) maps
//!   `bd-ticket-guard-client-cert|client-data|ree-public-key|iteration-version`
//!   onto `Request` tags 100–103 with `auth_type=PASSPORT_CERT_AUTH`.
//! - `/aweme/v1/comment/publish/` is listed with create/livechat as a guarded
//!   path (`X.1ZZk`); empty HTTP bodies match missing guard headers.
//!
//! Materials are read through the rooted TokenObject / TicketGuard providers
//! and the on-device `ree_hmac` Binder call — private keys stay on the phone.

use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::adb::Adb;

const TOKEN_URI: &str = "content://com.ss.android.ugc.aweme.TokenObjectProvider/get_token_object";
const SNAPSHOT_URI: &str =
    "content://com.ss.android.ugc.aweme.TicketGuardProvider/query_subprocess_sign_data";
const REE_HMAC_URI: &str = "content://com.ss.android.ugc.aweme.TicketGuardProvider/ree_hmac";
const CLIENT_CERT_KEVA: &str = "/data/data/com.ss.android.ugc.aweme/files/keva/repo/sp_TicketGuardHelper/sp_TicketGuardHelper.blk";

/// Four passport ticket-guard fields plus the signed path they cover.
#[derive(Clone, Debug)]
pub struct PassportZt {
    pub client_cert: String,
    pub client_data: String,
    pub ree_public_key: String,
    pub iter_version: String,
    pub path: String,
}

impl PassportZt {
    pub fn http_headers(&self) -> Vec<(String, String)> {
        vec![
            (
                "bd-ticket-guard-client-cert".into(),
                self.client_cert.clone(),
            ),
            (
                "bd-ticket-guard-client-data".into(),
                self.client_data.clone(),
            ),
            (
                "bd-ticket-guard-ree-public-key".into(),
                self.ree_public_key.clone(),
            ),
            (
                "bd-ticket-guard-iteration-version".into(),
                self.iter_version.clone(),
            ),
            ("bd-ticket-guard-version".into(), "3".into()),
        ]
    }

    pub fn to_json(&self) -> Value {
        json!({
            "client_cert": self.client_cert,
            "client_data": self.client_data,
            "ree_public_key": self.ree_public_key,
            "iter_version": self.iter_version,
            "path": self.path,
        })
    }
}

/// Build passport ZT for `path` (must start with `/`, no query).
/// Requires a rooted phone with the logged-in Douyin 40.8.0 process materials.
pub fn for_path(path: &str, serial: Option<&str>) -> Result<PassportZt, String> {
    validate_path(path)?;
    let adb = Adb::connect(serial)?;
    let token_row = adb.shell(&format!("content query --uri {TOKEN_URI} --user 0"), true)?;
    let (ticket, ts_sign_ree) = parse_token_ticket_and_ree_proof(&token_row)?;
    let snapshot = adb.shell(
        &format!("content query --uri '{SNAPSHOT_URI}' --user 0"),
        true,
    )?;
    let ree_public_key = parse_field(&snapshot, "ree_public_key")?;
    let client_cert = read_client_cert_b64(&adb)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_secs();
    if timestamp == 0 {
        return Err("系统时间无效".into());
    }
    let message = format!("ticket={ticket}&path={path}&timestamp={timestamp}");
    let mut uri = url::Url::parse(REE_HMAC_URI).map_err(|_| "TicketGuard 签名地址无效")?;
    uri.query_pairs_mut()
        .append_pair("content", &message)
        .append_pair("path", path);
    let hmac_row = adb.shell(
        &format!("content query --uri '{}' --user 0", uri.as_str()),
        true,
    )?;
    if let Some(err) = optional_field(&hmac_row, "exception_message")
        .filter(|v| !matches!(v.as_str(), "" | "NULL" | "null"))
    {
        let _ = err;
        return Err("手机 TicketGuard 未完成路径签名".into());
    }
    let req_sign_ree = parse_field(&hmac_row, "ree_hmac_result")?;
    let client_data = software_client_data(timestamp, &ts_sign_ree, &req_sign_ree)?;
    Ok(PassportZt {
        client_cert,
        client_data,
        ree_public_key,
        iter_version: "3".into(),
        path: path.into(),
    })
}

fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.len() > 4096
        || path.contains(['?', '#', '\\'])
        || path.chars().any(char::is_control)
    {
        return Err("ticket-guard 路径无效".into());
    }
    Ok(())
}

fn software_client_data(
    timestamp: u64,
    ts_sign_ree: &str,
    req_sign_ree: &str,
) -> Result<String, String> {
    if ts_sign_ree.len() > 16384
        || ts_sign_ree.chars().any(char::is_control)
        || matches!(ts_sign_ree, "NULL" | "null")
        || req_sign_ree.is_empty()
        || req_sign_ree.len() > 4096
        || !req_sign_ree.is_ascii()
        || req_sign_ree.chars().any(char::is_control)
    {
        return Err("ticket-guard client-data 证明无效".into());
    }
    let value = json!({
        "req_content": "ticket,path,timestamp",
        "timestamp": timestamp,
        "ts_sign": "",
        "ts_sign_ree": ts_sign_ree,
        "req_sign": "",
        "req_sign_ree": req_sign_ree,
    });
    let bytes = serde_json::to_vec(&value).map_err(|_| "client-data 编码失败")?;
    Ok(STANDARD.encode(bytes))
}

fn parse_token_ticket_and_ree_proof(bytes: &[u8]) -> Result<(String, String), String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "Token Provider 响应编码无效")?;
    let row = text
        .trim_end_matches(['\r', '\n'])
        .strip_prefix("Row: 0 token=")
        .ok_or("Token Provider 未返回登录快照")?;
    if !row.ends_with(", is_login=1") {
        return Err("Token Provider 未处于已登录状态".into());
    }
    let (ticket, rest) = row
        .split_once(", ts_sign=")
        .ok_or("Token Provider 缺少 ts_sign")?;
    if ticket.is_empty() || ticket.len() > 16384 || !ticket.is_ascii() {
        return Err("Token Provider ticket 无效".into());
    }
    let (ts_json, _) = rest
        .split_once(", attest_ts_sign=")
        .ok_or("Token Provider ts_sign 边界无效")?;
    let proofs = crate::guard::TokenProofs::parse(ts_json)?;
    let ts_sign_ree = proofs.require_ree()?.to_owned();
    Ok((ticket.to_owned(), ts_sign_ree))
}

fn parse_field(bytes: &[u8], name: &str) -> Result<String, String> {
    optional_field(bytes, name).ok_or_else(|| format!("TicketGuard 响应缺少 {name}"))
}

fn optional_field(bytes: &[u8], name: &str) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let row = text.trim_end_matches(['\r', '\n']);
    let marker = format!("{name}=");
    let start = row.find(&marker)? + marker.len();
    let tail = &row[start..];
    let end = tail.find(", ").unwrap_or(tail.len());
    let value = &tail[..end];
    if value.is_empty() || value.len() > 65536 || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_owned())
}

fn read_client_cert_b64(adb: &Adb) -> Result<String, String> {
    let bytes = adb.shell(&format!("cat {CLIENT_CERT_KEVA}"), true)?;
    if bytes.len() > 256 * 1024 {
        return Err("client-cert 存储过大".into());
    }
    // Keva stores the base64 PEM as a printable run beginning with the
    // `-----BEGIN CERTIFICATE-----` prefix encoded as base64 (`LS0tLS1...`).
    let text = String::from_utf8_lossy(&bytes);
    let start = text
        .find("LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t")
        .ok_or("手机未提供 client-cert")?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '='))
        .unwrap_or(rest.len());
    let cert = &rest[..end];
    if cert.len() < 64 || cert.len() > 8192 {
        return Err("client-cert 长度异常".into());
    }
    let pem = STANDARD
        .decode(cert)
        .map_err(|_| "client-cert 不是有效 base64")?;
    if !pem.starts_with(b"-----BEGIN CERTIFICATE-----") {
        return Err("client-cert 不是 PEM 证书".into());
    }
    Ok(cert.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_bad_paths() {
        assert!(validate_path("").is_err());
        assert!(validate_path("v1/x").is_err());
        assert!(validate_path("/v1/x?q=1").is_err());
        assert!(validate_path("/v1/message/send").is_ok());
    }

    #[test]
    fn client_data_roundtrip_shape() {
        let encoded = software_client_data(1700000000, "ts.1.fixture", "AQID").unwrap();
        let raw = STANDARD.decode(encoded).unwrap();
        let value: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(value["req_content"], "ticket,path,timestamp");
        assert_eq!(value["ts_sign"], "");
        assert_eq!(value["req_sign_ree"], "AQID");
    }

    #[test]
    #[ignore = "needs rooted phone; writes DOUYIN_IM_ZT_OUT or /tmp/re/douyin-im/tg-keva/zt.json"]
    fn refresh_zt_for_im_send() {
        let out = std::env::var("DOUYIN_IM_ZT_OUT")
            .unwrap_or_else(|_| "/tmp/re/douyin-im/tg-keva/zt.json".into());
        let zt = for_path("/v1/message/send", None).expect("ZT");
        std::fs::write(&out, zt.to_json().to_string()).expect("write zt");
        eprintln!("wrote {out} path={}", zt.path);
    }
}
