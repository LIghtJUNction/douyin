//! Explicit developer diagnostics for the installed SDK, never a runtime fallback.
//! Private provider results stay in memory; only fixture verification is returned.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use crate::{
    adb::Adb,
    guard::{HmacSigner, ReeSigner},
};

const SNAPSHOT: &str =
    "content://com.ss.android.ugc.aweme.TicketGuardProvider/query_subprocess_sign_data";
const FIELDS: &[&str] = &[
    "ree_public_key",
    "ree_private_key",
    "hmac_key",
    "subprocess_sign_strategy",
];
const PATH: &str = "/aweme/v1/feed/";
const TICKET: &str = "fixture-ticket";
const TIMESTAMP: u64 = 1700000000;
const TOKEN: &str = "content://com.ss.android.ugc.aweme.TokenObjectProvider/get_token_object";

fn checked_adb(serial: Option<&str>) -> Result<Adb, String> {
    let adb = Adb::connect(serial)?;
    if adb.shell("am get-current-user", false)?.trim_ascii() != b"0" {
        return Err("Binder 签名探针仅支持手机主用户".into());
    }
    let metadata = adb.package_metadata()?;
    if metadata["versionName"] != "40.8.0" || metadata["manifestVersionCode"] != "400801" {
        return Err("当前应用版本尚未完成 Binder 签名探针核对".into());
    }
    Ok(adb)
}

/// Corroborate the SDK's token-associated REE key without attaching to the app.
/// This is snapshot consistency, not authenticated account or HTTP acceptance.
pub fn verify_token_key_context(serial: Option<&str>) -> Result<Value, String> {
    let adb = checked_adb(serial)?;
    let read_token = || {
        let bytes = adb.shell(&format!("content query --uri '{TOKEN}' --user 0"), true)?;
        parse_token_row(&bytes)
    };
    let token_before = read_token()?;
    let before = query(&adb, SNAPSHOT, FIELDS)?;
    let signer = ReeSigner::from_snapshot(&before)?;
    let _hmac = HmacSigner::from_snapshot(&before)?;
    let matches = signer.matches_public_key_base64(&token_before.ree_public_key)?;
    let after = query(&adb, SNAPSHOT, FIELDS)?;
    let token_after = read_token()?;
    if before != after || token_before.raw != token_after.raw {
        return Err("Token 或软件签名材料在探测期间变化，请稍后重试".into());
    }
    Ok(
        json!({"token_snapshot_stable":true,"software_snapshot_stable":true,
        "token_ree_key_matches":matches,"account_binding_verified":false,
        "independent_auth_verified":false}),
    )
}

struct TokenCursor {
    raw: Vec<u8>,
    ree_public_key: String,
}

fn parse_token_row(bytes: &[u8]) -> Result<TokenCursor, String> {
    if bytes.len() > 65536 {
        return Err("Token Provider 响应过大".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Token Provider 响应编码无效")?
        .trim_end_matches(['\r', '\n']);
    // The inspected provider returns null before TTTokenManager initialization,
    // and also after caught exceptions. A null cursor is not a logout signal.
    if text == "No result found." {
        return Err("Token SDK 暂未提供快照；请打开抖音后重试，不能据此判断登录状态".into());
    }
    let row = text
        .strip_prefix("Row: 0 token=")
        .ok_or("Token Provider 未返回单行数据")?;
    if row.chars().any(char::is_control) || !row.ends_with(", is_login=1") {
        return Err("Token Provider 未返回有效的已登录快照".into());
    }
    let (token, _) = row
        .split_once(", ts_sign=")
        .ok_or("Token Provider 字段无效")?;
    if token.is_empty() || token.len() > 16384 || matches!(token, "NULL" | "null") {
        return Err("Token Provider 登录材料无效".into());
    }
    if row.matches(", track_data=").count() != 1 {
        return Err("Token Provider 元数据字段有歧义".into());
    }
    let (_, tail) = row
        .split_once(", track_data=")
        .ok_or("Token Provider 缺少元数据")?;
    #[derive(serde::Deserialize)]
    struct Track {
        #[serde(rename = "reePub")]
        ree_public_key: String,
    }
    let mut stream = serde_json::Deserializer::from_str(tail).into_iter::<Track>();
    let track = stream
        .next()
        .ok_or("Token 元数据为空")?
        .map_err(|_| "Token 元数据格式无效")?;
    let suffix = tail[stream.byte_offset()..]
        .strip_prefix(", token_tlb_tag=")
        .ok_or("Token Provider 元数据边界无效")?;
    let tag = suffix
        .strip_suffix(", is_login=1")
        .ok_or("Token Provider 登录字段无效")?;
    if tag.len() > 16384 || tag.contains(", ") {
        return Err("Token Provider 尾部字段有歧义".into());
    }
    Ok(TokenCursor {
        raw: bytes.to_vec(),
        ree_public_key: track.ree_public_key,
    })
}

/// Check software crypto against a public fixture through Android Binder.
/// This proves neither account binding nor independent HTTP authentication.
pub fn verify_fixture(serial: Option<&str>) -> Result<Value, String> {
    let adb = checked_adb(serial)?;
    let before = query(&adb, SNAPSHOT, FIELDS)?;
    let hmac = HmacSigner::from_snapshot(&before)?;
    let _ree = ReeSigner::from_snapshot(&before)?;
    let mut uri =
        url::Url::parse("content://com.ss.android.ugc.aweme.TicketGuardProvider/ree_hmac")
            .map_err(|_| "探针接口地址无效")?;
    // The provider logs its URI. Only public fixture content may be sent here;
    // never put a real ticket, Cookie, private key or account payload into it.
    let message = format!("ticket={TICKET}&path={PATH}&timestamp={TIMESTAMP}");
    uri.query_pairs_mut()
        .append_pair("content", &message)
        .append_pair("path", PATH);
    let native = query(
        &adb,
        uri.as_str(),
        &["ree_hmac_result", "exception_message"],
    )?;
    let after = query(&adb, SNAPSHOT, FIELDS)?;
    if before != after {
        return Err("软件签名材料在探测期间变化，请稍后重试".into());
    }
    if native["exception_message"]
        .as_str()
        .is_some_and(|v| !matches!(v, "" | "NULL" | "null"))
    {
        return Err("原生 SDK 未完成软件签名，错误内容已隐藏".into());
    }
    let signature = native["ree_hmac_result"]
        .as_str()
        .ok_or("原生 SDK 未返回软件签名")?;
    let signature = STANDARD
        .decode(signature)
        .map_err(|_| "原生软件签名编码无效")?;
    Ok(json!({"snapshot_stable":true,"ree_pair_valid":true,
        "hmac_matches_native":hmac.verify_request(TICKET, PATH, TIMESTAMP, &signature),
        "account_binding_verified":false,"independent_auth_verified":false}))
}

fn query(adb: &Adb, uri: &str, fields: &[&str]) -> Result<Value, String> {
    // Both URIs and the only query payload are constructed above, never from input.
    let bytes = adb.shell(&format!("content query --uri '{uri}' --user 0"), true)?;
    parse_row(&bytes, fields)
}

fn parse_row(bytes: &[u8], fields: &[&str]) -> Result<Value, String> {
    if bytes.len() > 65536 {
        return Err("签名 Provider 响应过大".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "签名 Provider 响应编码无效")?;
    let row = text
        .trim_end_matches(['\r', '\n'])
        .strip_prefix("Row: 0 ")
        .ok_or("签名 Provider 未返回单行数据")?;
    if row.chars().any(char::is_control) {
        return Err("签名 Provider 响应包含控制字符".into());
    }
    let parts: Vec<_> = row.split(", ").collect();
    if parts.len() != fields.len() {
        return Err("签名 Provider 响应字段数量无效".into());
    }
    let mut out = serde_json::Map::new();
    for (part, expected) in parts.iter().zip(fields) {
        let (name, value) = part.split_once('=').ok_or("签名 Provider 响应字段无效")?;
        if name != *expected {
            return Err("签名 Provider 响应字段或顺序无效".into());
        }
        out.insert(name.to_owned(), json!(value));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_cursor_handles_nested_json_and_rejects_ambiguous_or_logged_out_rows() {
        fn row(track: &str) -> String {
            format!(
                "Row: 0 token=fixture, ts_sign={{}}, attest_ts_sign=fixture, token_encrypted=fixture, token_hash=fixture, track_data={track}, token_tlb_tag=fixture-tag, is_login=1\n"
            )
        }
        let good =
            row(r#"{"reePub":"fixture-public","teeCreateLog":"a, b=c","other":{"nested":1}}"#);
        assert_eq!(
            parse_token_row(good.as_bytes()).unwrap().ree_public_key,
            "fixture-public"
        );
        for bad in [
            "No result found.".to_owned(),
            good.replace("is_login=1", "is_login=0"),
            good.replace("token=fixture,", "token=NULL,"),
            good.replace(
                "token_tlb_tag=fixture-tag",
                "token_tlb_tag=fixture-tag, extra=private-secret",
            ),
            row(r#"{"reePub":"private-secret","reePub":"other"}"#),
            row(r#"{"reePub":123}"#),
            row(r#"{"other":"private-secret"}"#),
            row(r#"{"reePub":"private-secret"} trailing"#),
            row(r#"{"reePub":"fixture","other":", track_data=private-secret"}"#),
            format!("{good}Row: 1 token=private-secret"),
        ] {
            assert!(
                !parse_token_row(bad.as_bytes())
                    .err()
                    .unwrap()
                    .contains("private-secret")
            );
        }
        assert!(parse_token_row(&vec![b'x'; 65537]).is_err());
    }

    #[test]
    fn cursor_parser_preserves_base64_and_rejects_ambiguous_or_private_errors() {
        let v = parse_row(b"Row: 0 mac=YWJjZA==, error=NULL\r\n", &["mac", "error"]).unwrap();
        assert_eq!(v["mac"], "YWJjZA==");
        for bad in [
            "Row: 0 mac=private, mac=private",
            "Row: 0 error=private, mac=private",
            "Row: 0 mac=private\nRow: 1 mac=private",
            "private",
            "Row: 0 mac=private\t",
        ] {
            assert!(
                !parse_row(bad.as_bytes(), &["mac", "error"])
                    .unwrap_err()
                    .contains("private")
            );
        }
        assert!(parse_row(&vec![b'x'; 65537], &["mac"]).is_err());
    }
}
