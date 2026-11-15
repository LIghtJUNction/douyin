//! Android Token authentication. Software/TEE request signing is separate.
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{blocking::Client, header::HeaderMap, redirect::Policy};
use serde_json::{Value, json};

use crate::{adb::Adb, auth};

const HOST: &str = "aweme.snssdk.com";
const TOKEN_URI: &str = "content://com.ss.android.ugc.aweme.TokenObjectProvider/get_token_object";

pub(crate) fn supported(session: &Value) -> bool {
    let d = &session["device"];
    d["versionName"] == "40.8.0"
        && d["versionCode"] == "400800"
        && d["manifestVersionCode"] == "400801"
        && d["updateVersionCode"] == "40809900"
}

fn header_text<'a>(value: &'a Value, field: &str, max: usize) -> Result<&'a str, String> {
    let text = value[field].as_str().ok_or("原生 Token 登录配置字段无效")?;
    if text.is_empty()
        || text.len() > max
        || text.chars().any(char::is_control)
        || reqwest::header::HeaderValue::from_str(text).is_err()
        || matches!(text, "NULL" | "null")
    {
        return Err("原生 Token 登录配置字段格式无效".into());
    }
    Ok(text)
}

pub(crate) fn validate(session: &Value) -> Result<(), String> {
    let Some(login) = session.get("nativeAuth") else {
        return Ok(());
    };
    if !supported(session)
        || login["schema"] != 1
        || login["source"] != "android-token-provider"
        || login["sdkVersion"] != "2"
        || login["passportSdkVersion"] != "60760"
        || login["boundUserId"] != session["account"]["id"]
        || login["boundUserId"].as_str().is_none_or(|v| {
            v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) || v.bytes().all(|b| b == b'0')
        })
        || login["verifiedAt"].as_u64().is_none_or(|v| v == 0)
    {
        return Err("原生 Token 与保存的应用版本或账号不一致，请重新 auth --adb".into());
    }
    if !header_text(login, "token", 16384)?.is_ascii() {
        return Err("原生 Token 编码无效".into());
    }
    header_text(login, "userAgent", 4096)?;
    if login.get("tokenTlbTag").is_some() && !header_text(login, "tokenTlbTag", 16384)?.is_ascii() {
        return Err("原生 Token 标签编码无效".into());
    }
    Ok(())
}

pub(crate) fn request_ticket() -> Result<String, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?;
    Ok(request_ticket_from(duration))
}

fn request_ticket_from(duration: Duration) -> String {
    duration.as_millis().to_string()
}

fn credential_headers(login: &Value) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    for (name, field) in [
        ("x-tt-token", "token"),
        ("sdk-version", "sdkVersion"),
        ("passport-sdk-version", "passportSdkVersion"),
    ] {
        let mut value = header_text(login, field, 16384)?
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|_| "原生认证请求头格式无效")?;
        if name == "x-tt-token" {
            value.set_sensitive(true);
        }
        headers.insert(name, value);
    }
    if login.get("tokenTlbTag").is_some() {
        let mut tag = header_text(login, "tokenTlbTag", 16384)?
            .parse::<reqwest::header::HeaderValue>()
            .map_err(|_| "原生 Token 标签请求头格式无效")?;
        tag.set_sensitive(true);
        headers.insert("token-tlb-tag", tag);
    }
    Ok(headers)
}

pub(crate) fn headers(session: &Value, host: &str) -> Result<HeaderMap, String> {
    // imapi.snssdk.com is the ByteDance IM SDK HTTP gate; it reuses the same Token.
    if (host == HOST || host == "imapi.snssdk.com") && session.get("nativeAuth").is_some() {
        validate(session)?;
        credential_headers(&session["nativeAuth"])
    } else {
        Ok(HeaderMap::new())
    }
}

/// Credentials for the separately verified fixed, read-only webcast endpoint.
/// This does not extend the general host allowlist to other webcast requests.
pub(crate) fn live_permission_auth(session: &Value) -> Result<(HeaderMap, &str), String> {
    validate(session)?;
    let login = session
        .get("nativeAuth")
        .ok_or("直播权限查询需要原生 Token，请重新 auth --adb")?;
    Ok((
        credential_headers(login)?,
        header_text(login, "userAgent", 4096)?,
    ))
}

pub(crate) fn user_agent<'a>(session: &'a Value, host: &str) -> Result<&'a str, String> {
    if (host == HOST || host == "imapi.snssdk.com") && session.get("nativeAuth").is_some() {
        header_text(&session["nativeAuth"], "userAgent", 4096)
    } else {
        header_text(session, "userAgent", 4096)
    }
}

#[derive(PartialEq, Eq)]
struct TokenSnapshot {
    token: String,
    tlb_tag: Option<String>,
}

impl TokenSnapshot {
    fn matches_login(&self, login: &Value) -> bool {
        login["token"].as_str() == Some(self.token.as_str())
            && login.get("tokenTlbTag").and_then(Value::as_str) == self.tlb_tag.as_deref()
    }
}

fn token_row(bytes: &[u8]) -> Result<Option<TokenSnapshot>, String> {
    if bytes.len() > 65536 {
        return Err("Token Provider 响应过大".into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| "Token Provider 响应编码无效")?
        .trim_end_matches(['\r', '\n']);
    if text == "No result found." {
        return Ok(None);
    }
    let row = text
        .strip_prefix("Row: 0 token=")
        .ok_or_else(|| {
            // Describe the envelope only; never echo token/proof/tracking data.
            format!(
                "Token SDK 未返回受支持的单行快照（字节数={}，行数={}，Row 行数={}，空响应={}，查询错误={}）",
                bytes.len(),
                text.lines().count(),
                text.lines().filter(|line| line.starts_with("Row: ")).count(),
                text.is_empty(),
                text.starts_with("Error while accessing provider")
                    || text.starts_with("Error:")
            )
        })?;
    if row.chars().any(char::is_control) {
        return Err("Token SDK 快照包含控制字符".into());
    }
    if !row.ends_with(", is_login=1") {
        return Err("手机 Token SDK 尚未提供已登录材料".into());
    }
    let (token, _) = row
        .split_once(", ts_sign=")
        .ok_or("Token SDK 快照字段无效")?;
    if !header_text(&json!({"token":token}), "token", 16384)?.is_ascii() {
        return Err("原生 Token 编码无效".into());
    }
    if row.matches(", track_data=").count() != 1 {
        return Err("Token SDK 元数据边界有歧义".into());
    }
    let (_, tail) = row
        .split_once(", track_data=")
        .ok_or("Token SDK 缺少元数据边界")?;
    // Decode only the boundary; never store tracking data or its key/proof fields.
    let end = if tail.starts_with("NULL, token_tlb_tag=") {
        4
    } else {
        let mut stream =
            serde_json::Deserializer::from_str(tail).into_iter::<serde::de::IgnoredAny>();
        stream
            .next()
            .ok_or("Token SDK 元数据为空")?
            .map_err(|_| "Token SDK 元数据格式无效")?;
        stream.byte_offset()
    };
    let tag = tail[end..]
        .strip_prefix(", token_tlb_tag=")
        .and_then(|v| v.strip_suffix(", is_login=1"))
        .ok_or("Token SDK 标签边界无效")?;
    if tag.contains(", ") || tag.len() > 16384 {
        return Err("Token SDK 标签字段有歧义".into());
    }
    let tlb_tag = if tag.is_empty() || matches!(tag, "NULL" | "null") {
        None
    } else {
        if !header_text(&json!({"tag":tag}), "tag", 16384)?.is_ascii() {
            return Err("原生 Token 标签编码无效".into());
        }
        Some(tag.to_owned())
    };
    Ok(Some(TokenSnapshot {
        token: token.to_owned(),
        tlb_tag,
    }))
}

fn read_token(adb: &Adb) -> Result<Option<TokenSnapshot>, String> {
    token_row(&adb.shell(&format!("content query --uri '{TOKEN_URI}' --user 0"), true)?)
}

pub(crate) fn confirm_import(adb: &Adb, session: &Value) -> Result<(), String> {
    if let Some(login) = session.get("nativeAuth") {
        validate(session)?;
        if read_token(adb)?.is_none_or(|snapshot| !snapshot.matches_login(login)) {
            return Err("验证期间手机 Token 或配套标签已变化，未保存新的登录配置，请重试".into());
        }
    }
    Ok(())
}

/// Import the Token, its companion routing tag and protocol metadata, not signing keys.
/// Existing Cookie-only sessions remain supported for uninspected APK builds.
pub(crate) fn import(adb: &Adb, session: &Value) -> Result<Option<(Value, Value)>, String> {
    if !supported(session) {
        return Ok(None);
    }
    let read = || read_token(adb);
    // login_adb opens the app before reading device/install identity. Do not
    // launch it here, after the candidate identity snapshot has been taken.
    let mut first = read()?;
    let mut stable = None;
    for _ in 0..4 {
        let second = read()?;
        if first.is_some() && first == second {
            stable = first;
            break;
        }
        first = second;
        std::thread::sleep(Duration::from_millis(250));
    }
    let token = stable.ok_or("Token SDK 尚未提供稳定材料，请保持手机解锁并打开抖音后重试")?;
    let agent = session["nativeTransport"]["cronetDefaultUserAgent"]
        .as_str()
        .or_else(|| session["userAgent"].as_str())
        .ok_or("缺少原生请求 User-Agent")?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_secs();
    let mut login = json!({"schema":1,"source":"android-token-provider","token":token.token.clone(),
        "sdkVersion":"2","passportSdkVersion":"60760","userAgent":agent,
        "verifiedAt":timestamp});
    if let Some(tag) = &token.tlb_tag {
        login["tokenTlbTag"] = json!(tag);
    }
    // Bootstrap account identity from the phone SDK Token, never WebView Cookie.
    let account = request_account(session, &login)?;
    login["boundUserId"] = account["id"].clone();
    let mut candidate = session.clone();
    candidate["account"] = account.clone();
    candidate["nativeAuth"] = login.clone();
    verify_online(&candidate)?;
    // A rotation while checking account identity must not save a stale/mixed import.
    if read()?.is_none_or(|snapshot| !snapshot.matches_login(&login)) {
        return Err("验证期间手机 Token 或配套标签已变化，未保存新的登录配置，请重试".into());
    }
    Ok(Some((login, account)))
}

/// Verify the imported account with Token only. Never add Cookie or use ADB here.
pub(crate) fn verify_online(session: &Value) -> Result<(), String> {
    validate(session)?;
    if session.get("nativeAuth").is_none() {
        return Ok(());
    }
    let account = request_account(session, &session["nativeAuth"])?;
    if account["id"] != session["account"]["id"] {
        return Err("原生 Token 未确认保存的同一账号，可能已失效，请重新 auth --adb".into());
    }
    Ok(())
}

fn request_account(session: &Value, login: &Value) -> Result<Value, String> {
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|_| "无法创建原生 Token 验证客户端")?;
    let params = auth::request_identity(session)?;
    let headers = credential_headers(login)?;
    let agent = header_text(login, "userAgent", 4096)?;
    bounded_verify(|| fetch_account(&client, &params, &headers, agent))
}

enum VerifyFailure {
    Transport(&'static str),
    Rejected(String),
}

fn bounded_verify<T>(mut attempt: impl FnMut() -> Result<T, VerifyFailure>) -> Result<T, String> {
    for index in 0..2 {
        match attempt() {
            Ok(value) => return Ok(value),
            Err(VerifyFailure::Rejected(error)) => return Err(error),
            Err(VerifyFailure::Transport(error)) if index == 1 => return Err(error.into()),
            Err(VerifyFailure::Transport(_)) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    unreachable!("bounded verifier always returns on the second attempt")
}

fn fetch_account(
    client: &Client,
    params: &[(&str, &str)],
    headers: &HeaderMap,
    agent: &str,
) -> Result<Value, VerifyFailure> {
    let response = client
        .get("https://aweme.snssdk.com/aweme/v1/user/profile/self/")
        .query(params)
        .headers(headers.clone())
        .header("User-Agent", agent)
        .header(
            "x-ss-req-ticket",
            request_ticket().map_err(VerifyFailure::Rejected)?,
        )
        .send()
        .map_err(|_| VerifyFailure::Transport("原生 Token 账号验证请求失败"))?;
    if !response.status().is_success() {
        return Err(VerifyFailure::Rejected(format!(
            "原生 Token 账号验证返回 HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| VerifyFailure::Transport("读取原生 Token 验证响应失败"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(VerifyFailure::Rejected("原生 Token 验证响应过大".into()));
    }
    let body: Value = serde_json::from_slice(&bytes)
        .map_err(|_| VerifyFailure::Rejected("原生 Token 验证响应无法识别".into()))?;
    parse_profile_account(&body).map_err(VerifyFailure::Rejected)
}

fn parse_profile_account(body: &Value) -> Result<Value, String> {
    let user = &body["user"];
    let id = user["uid"]
        .as_str()
        .map(str::to_owned)
        .or_else(|| user["uid"].as_u64().map(|v| v.to_string()));
    if body["status_code"] != 0
        || id.as_deref().is_none_or(|v| {
            v.is_empty()
                || v.len() > 24
                || !v.bytes().all(|b| b.is_ascii_digit())
                || v.bytes().all(|b| b == b'0')
        })
    {
        return Err("原生 Token 未确认有效登录账号，请在手机上重新登录".into());
    }
    // Keep only public account identifiers, never the rest of the profile body.
    Ok(
        json!({"id":id.unwrap(),"nickname":user["nickname"].as_str().unwrap_or(""),
        "secUserId":user["sec_uid"].as_str().unwrap_or("")}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ticket_uses_milliseconds_without_truncating_to_seconds() {
        assert_eq!(request_ticket_from(Duration::new(1, 234_567_890)), "1234");
        assert!(request_ticket().unwrap().len() >= 13);
    }

    #[test]
    fn retries_only_transport_failures_and_never_repeats_account_rejection() {
        let mut calls = 0;
        assert!(
            bounded_verify(|| {
                calls += 1;
                if calls == 1 {
                    Err(VerifyFailure::Transport("network"))
                } else {
                    Ok(())
                }
            })
            .is_ok()
        );
        assert_eq!(calls, 2);
        calls = 0;
        assert!(
            bounded_verify(|| {
                calls += 1;
                Err::<(), _>(VerifyFailure::Rejected("wrong account".into()))
            })
            .is_err()
        );
        assert_eq!(calls, 1);
        calls = 0;
        assert!(
            bounded_verify(|| {
                calls += 1;
                Err::<(), _>(VerifyFailure::Transport("network"))
            })
            .is_err()
        );
        assert_eq!(calls, 2);
    }

    fn session() -> Value {
        json!({"account":{"id":"123"},"device":{"versionName":"40.8.0","versionCode":"400800",
            "manifestVersionCode":"400801","updateVersionCode":"40809900"},"userAgent":"fixture-legacy-agent",
            "nativeAuth":{"schema":1,"source":"android-token-provider","token":"private-fixture-token",
                "sdkVersion":"2","passportSdkVersion":"60760","userAgent":"fixture-sdk-agent","boundUserId":"123","verifiedAt":1}})
    }

    #[test]
    fn bootstrap_requires_authenticated_self_profile_and_keeps_only_public_identity() {
        for body in [
            json!({"status_code":8}),
            json!({"status_code":0}),
            json!({"status_code":0,"user":{"uid":0}}),
            json!({"status_code":0,"user":{"uid":"000"}}),
            json!({"status_code":0,"user":{"uid":"1e3"}}),
            json!({"status_code":0,"user":{"uid":"1234567890123456789012345"}}),
        ] {
            assert!(parse_profile_account(&body).is_err());
        }
        let body = json!({"status_code":0,"user":{"uid":"9007199254740993",
            "nickname":"fixture","sec_uid":"public-fixture-id", "phone":"private-fixture"}});
        assert_eq!(
            parse_profile_account(&body).unwrap(),
            json!({"id":"9007199254740993",
            "nickname":"fixture","secUserId":"public-fixture-id"})
        );
        assert_eq!(
            parse_profile_account(&json!({"status_code":0,"user":{"uid":123}})).unwrap()["id"],
            "123"
        );
    }

    #[test]
    fn credentials_are_version_bound_and_do_not_cross_hosts() {
        let s = session();
        let (live_headers, live_agent) = live_permission_auth(&s).unwrap();
        assert_eq!(live_agent, "fixture-sdk-agent");
        assert!(live_headers["x-tt-token"].is_sensitive());
        assert!(live_headers.get("cookie").is_none());
        let mut legacy = s.clone();
        legacy.as_object_mut().unwrap().remove("nativeAuth");
        assert!(live_permission_auth(&legacy).is_err());
        let h = headers(&s, HOST).unwrap();
        assert_eq!(h["sdk-version"], "2");
        assert!(h["x-tt-token"].is_sensitive());
        assert_eq!(h["passport-sdk-version"], "60760");
        assert!(h.get("cookie").is_none());
        for host in [
            "webcast.amemv.com",
            "aweme.snssdk.com.evil.test",
            "media.example",
        ] {
            assert!(headers(&s, host).unwrap().is_empty());
            assert_eq!(user_agent(&s, host).unwrap(), "fixture-legacy-agent");
        }
        for field in ["boundUserId", "passportSdkVersion", "sdkVersion"] {
            let mut bad = s.clone();
            bad["nativeAuth"][field] = json!("wrong");
            assert!(validate(&bad).is_err());
        }
        let mut bad = s.clone();
        bad["device"]["manifestVersionCode"] = json!("400802");
        assert!(validate(&bad).is_err());
        let mut bad = s;
        bad["nativeAuth"]["token"] = json!("private-secret\r\ninjected: yes");
        assert!(!validate(&bad).unwrap_err().contains("private-secret"));
    }

    #[test]
    fn token_and_tag_must_stay_together_without_retaining_tracking_material() {
        let row = b"Row: 0 token=fixture, ts_sign={}, track_data={\"nested\":{\"value\":\"a, b=c\"}}, token_tlb_tag=fixture-tag, is_login=1";
        let snapshot = token_row(row).unwrap().unwrap();
        assert!(snapshot.matches_login(&json!({"token":"fixture","tokenTlbTag":"fixture-tag"})));
        assert!(!snapshot.matches_login(&json!({"token":"fixture","tokenTlbTag":"rotated"})));
        assert!(!snapshot.matches_login(&json!({"token":"fixture"})));
        assert!(!snapshot.matches_login(&json!({"token":"rotated","tokenTlbTag":"fixture-tag"})));
        for track in ["NULL", "null", "{}"] {
            let row = format!(
                "Row: 0 token=fixture, ts_sign={{}}, track_data={track}, token_tlb_tag=NULL, is_login=1"
            );
            assert!(
                token_row(row.as_bytes())
                    .unwrap()
                    .unwrap()
                    .matches_login(&json!({"token":"fixture"}))
            );
        }
        for tail in [
            "{} trailing, token_tlb_tag=private-secret",
            "{}, token_tlb_tag=fixture, extra=private-secret",
            "{}, token_tlb_tag=非ASCII",
            "{}, token_tlb_tag=fixture, token_tlb_tag=private-secret",
        ] {
            let row = format!("Row: 0 token=fixture, ts_sign={{}}, track_data={tail}, is_login=1");
            assert!(
                !token_row(row.as_bytes())
                    .err()
                    .unwrap()
                    .contains("private-secret")
            );
        }
    }

    #[test]
    fn optional_token_tag_is_sensitive_and_has_the_same_host_scope_as_token() {
        let mut s = session();
        assert!(validate(&s).is_ok());
        assert!(headers(&s, HOST).unwrap().get("token-tlb-tag").is_none());
        s["nativeAuth"]["tokenTlbTag"] = json!("fixture-tag");
        let h = headers(&s, HOST).unwrap();
        assert_eq!(h["token-tlb-tag"], "fixture-tag");
        assert!(h["token-tlb-tag"].is_sensitive());
        assert!(headers(&s, "media.example").unwrap().is_empty());
        assert!(headers(&s, "webcast.amemv.com").unwrap().is_empty());
        assert!(live_permission_auth(&s).unwrap().0["token-tlb-tag"].is_sensitive());
        for bad in [
            json!(null),
            json!(""),
            json!("NULL"),
            json!("非ASCII"),
            json!("private-secret\r\nx: injected"),
        ] {
            s["nativeAuth"]["tokenTlbTag"] = bad;
            assert!(!validate(&s).unwrap_err().contains("private-secret"));
        }
    }

    #[test]
    fn token_reader_distinguishes_unready_from_logged_out_and_malformed() {
        assert!(token_row(b"No result found.\n").unwrap().is_none());
        assert_eq!(
            token_row(b"Row: 0 token=fixture, ts_sign={}, track_data={}, token_tlb_tag=NULL, is_login=1\n")
                .unwrap()
                .as_ref().map(|v| v.token.as_str()),
            Some("fixture")
        );
        for row in [
            "Row: 0 token=NULL, ts_sign={}, is_login=1",
            "Row: 0 token=private-secret, ts_sign={}, is_login=0",
            "Row: 0 token=private-secret\nRow: 1 token=x",
        ] {
            assert!(
                !token_row(row.as_bytes())
                    .err()
                    .unwrap()
                    .contains("private-secret")
            );
        }
    }
}
