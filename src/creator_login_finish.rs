//! Finalize a confirmed native QR session without inventing publish capability.
//!
//! Inputs come only from this process's QR flow, generated P-256 key and its
//! server response. Two fixed Creator GETs confirm the account. A login result
//! is kept in a private pending file; incomplete publishing material never
//! replaces an existing active Cookie or creator-session.json.
//!
//! Account endpoints are observed in the public Creator main.a20c35af.js.
//! The ECDSA fallback envelope is observed in the official secure-consumer
//! creator signer.umd.production.js, version 1.0.0.22. No request is retried,
//! no browser environment is fabricated, and no upload/create endpoint is used.

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use p256::SecretKey;
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::DecodePrivateKey;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;
use serde_json::{Value, json};

use crate::creator_login::NativeLoginMaterial;
use crate::store;

const ORIGIN: &str = "https://creator.douyin.com";
const MEDIA_ACCOUNT: &str = "/web/api/media/user/info/";
const PC_ACCOUNT: &str = "/aweme/v1/creator/pc/user/info/";
const MAX_RESPONSE: u64 = 1024 * 1024;
const MAX_MATERIAL: usize = 256 * 1024;
const PENDING_FILENAME: &str = "native-session.pending.json";

/// The native login poller must only call this after confirmation and after
/// following an allowed, same-origin login redirect.
pub(crate) fn finalize_native_login(
    material: NativeLoginMaterial,
    timeout: Duration,
) -> Result<Value, String> {
    finalize_at_origin(material, timeout, ORIGIN)
}

fn finalize_at_origin(
    material: NativeLoginMaterial,
    timeout: Duration,
    origin: &str,
) -> Result<Value, String> {
    if timeout.is_zero() || timeout > Duration::from_secs(3600) {
        return Err("登录最终核验 timeout 必须在 1..3600 秒之间".into());
    }
    #[cfg(not(test))]
    if origin != ORIGIN {
        return Err("登录最终核验只允许固定 Creator HTTPS 同源".into());
    }
    let verified = VerifiedMaterial::new(&material)?;
    let path = pending_path(&material.pending_dir)?;
    let timestamp = now()?;
    let mut document = json!({
        "schema": 1,
        "origin": ORIGIN,
        "source": "native_qr_login",
        "stage": "identity_pending",
        "confirmed_at": timestamp,
        "cookie": material.cookie,
        "private_key_pem": material.private_key_pem,
        "public_key_base64": material.public_key_base64,
        "server_data": material.server_data,
        "creator_csrf_token": material.csrf_token,
        "ms_token": material.ms_token,
        "user_agent": material.user_agent,
        "account": Value::Null,
        "active": false,
        "publish_ready": false,
    });
    // Preserve confirmed, server-bound material before an account GET can fail.
    // This remains a pending document; it is never loaded as publish-ready.
    save_pending(&path, &document)?;
    let http = Client::builder()
        .connect_timeout(timeout.min(Duration::from_secs(15)))
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .build()
        .map_err(|_| "无法创建原生登录账号核验客户端")?;
    let account = verify_account(&http, origin, &verified);
    let missing = missing_publish_material(&material);
    let report = match account {
        Ok(account) => {
            document["stage"] = json!("identity_verified_publish_material_pending");
            document["account"] = account.clone();
            document["verified_at"] = json!(now()?);
            json!({
                "ok": true,
                "stage": "identity_verified_publish_material_pending",
                "loginConfirmed": true,
                "accountVerified": true,
                "readOnlyVerified": true,
                "publishReady": false,
                "active": false,
                "account": account,
                "missing": missing,
                "pendingMaterial": path,
                "uploadSent": false,
                "createSent": false,
                "nextAction": "继续取得本次登录的 Creator 发布材料；当前仅保存待完成会话，现有活动登录未替换",
                "credentialValuesRedacted": true,
            })
        }
        Err(error) => {
            document["stage"] = json!("identity_unconfirmed");
            document["last_error"] = json!(error);
            json!({
                "ok": false,
                "stage": "identity_unconfirmed",
                "loginConfirmed": true,
                "accountVerified": false,
                "readOnlyVerified": false,
                "publishReady": false,
                "active": false,
                "reason": error,
                "missing": missing,
                "pendingMaterial": path,
                "uploadSent": false,
                "createSent": false,
                "automaticRetry": false,
                "credentialValuesRedacted": true,
            })
        }
    };
    document["capabilities"] = json!({
        "account_verified": report["accountVerified"],
        "read_only_verified": report["readOnlyVerified"],
        "publish_ready": false,
        "active": false,
        "missing": report["missing"],
    });
    save_pending(&path, &document)?;
    Ok(report)
}

/// Secret-bearing data validated before it is stored or attached to a request.
/// Intentionally does not implement Debug or Serialize.
struct VerifiedMaterial {
    cookie: String,
    user_agent: String,
    csrf_token: Option<String>,
    ticket: String,
    ts_sign: String,
    public_key_base64: String,
    signer: SigningKey,
}

impl VerifiedMaterial {
    fn new(input: &NativeLoginMaterial) -> Result<Self, String> {
        let cookies = validate_cookie(&input.cookie)?;
        checked_header(&input.user_agent, "user_agent", 2048)?;
        checked_header(&input.public_key_base64, "public_key_base64", 256)?;
        if input.private_key_pem.len() > 8192 {
            return Err("原生登录私钥长度无效（内容已隐藏）".into());
        }
        let private = SecretKey::from_pkcs8_pem(&input.private_key_pem)
            .or_else(|_| SecretKey::from_sec1_pem(&input.private_key_pem))
            .map_err(|_| "原生登录私钥不是有效 P-256 密钥")?;
        let point = private.public_key().to_encoded_point(false);
        let supplied_public = STANDARD
            .decode(&input.public_key_base64)
            .map_err(|_| "原生登录公钥编码无效")?;
        if supplied_public != point.as_bytes() {
            return Err("原生登录私钥与本次公钥不匹配".into());
        }
        let ticket = input
            .server_data
            .get("ticket")
            .and_then(Value::as_str)
            .ok_or("确认响应缺少服务器签发的 ticket")?;
        let ts_sign = input
            .server_data
            .get("ts_sign")
            .and_then(Value::as_str)
            .ok_or("确认响应缺少服务器签发的 ts_sign")?;
        checked_header(ticket, "server_data.ticket", 8192)?;
        checked_header(ts_sign, "server_data.ts_sign", 4096)?;
        let client_cert = input
            .server_data
            .get("client_cert")
            .and_then(Value::as_str)
            .and_then(|value| value.strip_prefix("pub."))
            .ok_or("确认响应没有可核对的本机 client_cert 公钥")?;
        let certificate_public = STANDARD
            .decode(client_cert)
            .map_err(|_| "确认响应 client_cert 编码无效")?;
        if certificate_public != point.as_bytes() {
            return Err("确认响应 ticket 未绑定本次生成的公钥；拒绝混用其他登录材料".into());
        }
        if let Some(id) = cookies.get("bd_ticket_guard_ts_sign_id")
            && !id.is_empty()
            && !ts_sign.starts_with(id)
        {
            return Err("确认响应 ts_sign 与登录 Cookie 不匹配".into());
        }
        let csrf_token = match &input.csrf_token {
            Some(token) => {
                checked_header(token, "creator_csrf_token", 4096)?;
                if !cookies
                    .get("csrf_session_id")
                    .is_some_and(|v| !v.is_empty())
                {
                    return Err(
                        "Creator CSRF 缺少同次 csrf_session_id Cookie，拒绝混用 Passport CSRF"
                            .into(),
                    );
                }
                Some(token.clone())
            }
            None => None,
        };
        if let Some(token) = &input.ms_token {
            checked_header(token, "ms_token", 4096)?;
            if let Some(cookie_token) = cookies.get("msToken")
                && !cookie_token.is_empty()
                && cookie_token != token
            {
                return Err("本次登录 msToken 与 Cookie 不匹配".into());
            }
        }
        Ok(Self {
            cookie: input.cookie.clone(),
            user_agent: input.user_agent.clone(),
            csrf_token,
            ticket: ticket.to_owned(),
            ts_sign: ts_sign.to_owned(),
            public_key_base64: input.public_key_base64.clone(),
            signer: SigningKey::from(&private),
        })
    }

    fn account_headers(&self, path: &str) -> Result<HeaderMap, String> {
        if !matches!(path, MEDIA_ACCOUNT | PC_ACCOUNT) {
            return Err("原生登录最终核验仅允许已确认的 Creator 本人账号路径".into());
        }
        let timestamp = now()?;
        let text = format!("ticket={}&path={path}&timestamp={timestamp}", self.ticket);
        let signature: Signature = self.signer.sign(text.as_bytes());
        #[derive(Serialize)]
        struct Envelope<'a> {
            ts_sign: &'a str,
            req_content: &'static str,
            req_sign: String,
            timestamp: u64,
        }
        // Current official SDK's documented fallback: local-key ECDSA, no
        // server ECDH key and no trusted timestamp flag. It does not imply that
        // the server will accept this incomplete session for publishing.
        let client_data = serde_json::to_vec(&Envelope {
            ts_sign: &self.ts_sign,
            req_content: "ticket,path,timestamp",
            req_sign: STANDARD.encode(signature.to_der().as_bytes()),
            timestamp,
        })
        .map_err(|_| "无法构造原生登录账号签名")?;
        let mut headers = HeaderMap::new();
        put(&mut headers, "cookie", &self.cookie, true)?;
        put(&mut headers, "user-agent", &self.user_agent, false)?;
        put(&mut headers, "accept", "application/json", false)?;
        put(&mut headers, "origin", ORIGIN, false)?;
        put(
            &mut headers,
            "referer",
            "https://creator.douyin.com/",
            false,
        )?;
        put(&mut headers, "bd-ticket-guard-version", "2", false)?;
        put(
            &mut headers,
            "bd-ticket-guard-web-version",
            if self.ts_sign.starts_with("ts.1") {
                "1"
            } else {
                "2"
            },
            false,
        )?;
        put(&mut headers, "bd-ticket-guard-web-sign-type", "0", false)?;
        put(
            &mut headers,
            "bd-ticket-guard-ree-public-key",
            &self.public_key_base64,
            true,
        )?;
        put(
            &mut headers,
            "bd-ticket-guard-client-data",
            &STANDARD.encode(client_data),
            true,
        )?;
        if let Some(token) = &self.csrf_token {
            put(&mut headers, "x-secsdk-csrf-token", token, true)?;
        }
        Ok(headers)
    }
}

fn verify_account(
    http: &Client,
    origin: &str,
    material: &VerifiedMaterial,
) -> Result<Value, String> {
    let media = get_account(http, origin, MEDIA_ACCOUNT, true, material)?;
    let user = media
        .get("user")
        .and_then(Value::as_object)
        .ok_or("Creator 身份响应缺少 user，登录仍未确认")?;
    let media_uid = user
        .get("uid")
        .and_then(numeric_id)
        .ok_or("Creator 身份响应缺少有效 user.uid")?;
    let nickname = user
        .get("nickname")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty() && name.len() <= 2048)
        .ok_or("Creator 身份响应缺少有效昵称")?;
    let pc = get_account(http, origin, PC_ACCOUNT, false, material)?;
    let pc_uid = pc
        .get("uid")
        .and_then(numeric_id)
        .ok_or("Creator PC 身份响应缺少有效 uid")?;
    if pc_uid != media_uid {
        return Err("两次 Creator 在线核验得到不同账号，已停止激活".into());
    }
    if pc
        .get("is_confer_login")
        .is_some_and(|value| value == &json!(true) || value == &json!(1))
    {
        return Err("当前为委托登录，尚不能确认本机投稿归属".into());
    }
    Ok(json!({"id": media_uid, "nickname": nickname}))
}

fn get_account(
    http: &Client,
    origin: &str,
    path: &str,
    media: bool,
    material: &VerifiedMaterial,
) -> Result<Value, String> {
    let mut url = reqwest::Url::parse(origin).map_err(|_| "Creator 核验地址无效")?;
    url.set_path(path);
    if media {
        url.query_pairs_mut().append_pair("device_platform", "web");
    }
    let response = http
        .get(url)
        .headers(material.account_headers(path)?)
        .send()
        .map_err(|_| "Creator 在线身份核验失败或超时；未自动重试")?;
    if !response.status().is_success() {
        return Err(format!(
            "Creator 在线身份核验返回 HTTP {}，已停止",
            response.status().as_u16()
        ));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_RESPONSE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Creator 身份响应未完整读取")?;
    if bytes.len() as u64 > MAX_RESPONSE {
        return Err("Creator 身份响应超过 1 MiB".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "Creator 身份响应不是 JSON，可能要求额外验证")?;
    for pointer in [
        "/captcha",
        "/captcha_url",
        "/verify_conf",
        "/verify_center_decision_conf",
        "/data/captcha",
        "/data/verify_conf",
        "/data/verify_center_decision_conf",
    ] {
        if value.pointer(pointer).is_some_and(nonempty) {
            return Err("Creator 身份核验要求额外验证；未自动重试".into());
        }
    }
    match value.get("status_code").and_then(Value::as_i64) {
        Some(0) => Ok(value),
        Some(code) => Err(format!("Creator 在线身份核验状态码 {code}，登录仍未确认")),
        None => Err("Creator 身份响应缺少明确成功状态码".into()),
    }
}

fn missing_publish_material(input: &NativeLoginMaterial) -> Vec<&'static str> {
    let mut missing = vec![
        "creator.ticket_guard.sign_mode evidence",
        "creator.ticket_guard.server_cert when hmac",
        "creator.dtrait.blob",
        "creator.dtrait.pk1_pem",
        "creator.dtrait.pk1_version",
        "creator.dtrait.sdk_version",
        "creator.platform_params from this session",
    ];
    if input.csrf_token.is_none() {
        missing.push("creator.csrf_token");
    }
    if input.ms_token.is_none() {
        missing.push("creator.ms_token");
    }
    missing
}

fn checked_header(value: &str, name: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || !value.is_ascii()
        || value.chars().any(char::is_control)
        || HeaderValue::from_str(value).is_err()
    {
        return Err(format!("原生登录字段 {name} 无效（值已隐藏）"));
    }
    Ok(())
}

fn validate_cookie(cookie: &str) -> Result<BTreeMap<String, String>, String> {
    checked_header(cookie, "cookie", 64 * 1024)?;
    let mut result = BTreeMap::new();
    for piece in cookie.split(';').map(str::trim).filter(|v| !v.is_empty()) {
        let (name, value) = piece.split_once('=').ok_or("原生登录 Cookie 结构无效")?;
        if name.is_empty() || name.bytes().any(|b| b <= b' ' || b >= 127) {
            return Err("原生登录 Cookie 名称无效".into());
        }
        if let Some(old) = result.insert(name.to_owned(), value.to_owned())
            && old != value
            && matches!(
                name,
                "sessionid"
                    | "sessionid_ss"
                    | "sid_tt"
                    | "csrf_session_id"
                    | "bd_ticket_guard_ts_sign_id"
                    | "msToken"
            )
        {
            return Err("原生登录 Cookie 存在相互冲突的会话字段".into());
        }
    }
    if !["sessionid", "sessionid_ss"]
        .iter()
        .any(|name| result.get(*name).is_some_and(|v| !v.is_empty()))
    {
        return Err("二维码响应尚未形成已登录 Cookie；不能完成账号核验".into());
    }
    Ok(result)
}

fn numeric_id(value: &Value) -> Option<String> {
    let id = match value {
        Value::String(v) => v.clone(),
        Value::Number(v) => v.as_u64()?.to_string(),
        _ => return None,
    };
    (!id.is_empty()
        && id.len() <= 32
        && id.bytes().all(|b| b.is_ascii_digit())
        && id.bytes().any(|b| b != b'0'))
    .then_some(id)
}

fn nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Number(v) => v.as_u64() != Some(0),
        Value::String(v) => !v.is_empty() && v != "{}" && v != "null",
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
    }
}

fn put(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
    sensitive: bool,
) -> Result<(), String> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| format!("原生登录 header {name} 无效（值已隐藏）"))?;
    value.set_sensitive(sensitive);
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

fn pending_path(directory: &Path) -> Result<PathBuf, String> {
    if directory.as_os_str().is_empty() {
        return Err("原生登录待完成目录无效".into());
    }
    match fs::symlink_metadata(directory) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err("原生登录待完成目录不能是符号链接或普通文件".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("无法检查原生登录待完成目录".into()),
    }
    let path = directory.join(PENDING_FILENAME);
    if fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_file()) {
        return Err("原生登录待完成文件必须是普通文件".into());
    }
    Ok(path)
}

fn save_pending(path: &Path, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "无法编码原生登录材料")?;
    if bytes.len() > MAX_MATERIAL {
        return Err("原生登录材料超过 256 KiB，未保存".into());
    }
    store::save_at(path, value).map_err(|_| "无法安全保存原生登录待完成材料")?;
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|_| "原生登录材料已写入，但目录同步失败")?;
    }
    Ok(())
}

fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "系统时间无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use p256::pkcs8::{EncodePrivateKey, LineEnding};
    use std::thread;
    use tiny_http::{Response, Server};

    fn material(directory: &Path) -> NativeLoginMaterial {
        let key = SecretKey::from_slice(&[7u8; 32]).unwrap();
        let public = STANDARD.encode(key.public_key().to_encoded_point(false).as_bytes());
        NativeLoginMaterial {
            cookie: "sessionid=fixture-only; csrf_session_id=fixture-csrf".into(),
            private_key_pem: key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
            public_key_base64: public.clone(),
            server_data: json!({
                "ticket": "fixture-secret-ticket", "ts_sign": "ts.2.fixture-only",
                "client_cert": format!("pub.{public}")
            }),
            csrf_token: Some("fixture-secret-csrf".into()),
            ms_token: None,
            user_agent: "douyin-cli fixture-native-login".into(),
            pending_dir: directory.to_owned(),
        }
    }

    #[test]
    fn different_server_binding_is_rejected_before_account_requests() {
        let dir = tempfile::tempdir().unwrap();
        let mut input = material(dir.path());
        let different = SecretKey::from_slice(&[8u8; 32]).unwrap();
        input.server_data["client_cert"] = format!(
            "pub.{}",
            STANDARD.encode(different.public_key().to_encoded_point(false).as_bytes())
        )
        .into();
        let error = match VerifiedMaterial::new(&input) {
            Err(error) => error,
            Ok(_) => panic!("mismatched server key accepted"),
        };
        assert!(error.contains("未绑定本次"));
        assert!(!error.contains("fixture-secret-ticket"));
    }

    #[test]
    fn verified_identity_is_kept_pending_without_publish_capability() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", server.server_addr());
        let worker = thread::spawn(move || {
            for (expected, body) in [
                (
                    MEDIA_ACCOUNT,
                    json!({"status_code":0,"user":{"uid":"12345","nickname":"本人"}}),
                ),
                (
                    PC_ACCOUNT,
                    json!({"status_code":0,"uid":"12345","is_confer_login":false}),
                ),
            ] {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.method().as_str(), "GET");
                assert!(request.url().starts_with(expected));
                assert!(
                    request
                        .headers()
                        .iter()
                        .any(|h| h.field.equiv("bd-ticket-guard-client-data"))
                );
                assert!(!request.url().contains("a_bogus"));
                assert!(!request.url().contains("msToken"));
                request
                    .respond(Response::from_string(body.to_string()))
                    .unwrap();
            }
        });
        let report =
            finalize_at_origin(material(dir.path()), Duration::from_secs(5), &origin).unwrap();
        worker.join().unwrap();
        assert_eq!(report["accountVerified"], true);
        assert_eq!(report["publishReady"], false);
        assert_eq!(report["active"], false);
        assert!(!report.to_string().contains("fixture-secret"));
        let path = dir.path().join(PENDING_FILENAME);
        let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["account"]["id"], "12345");
        assert_eq!(saved["publish_ready"], false);
        assert!(!dir.path().join("creator-session.json").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn status_error_is_not_a_login_success_and_raw_response_stays_private() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", server.server_addr());
        let worker = thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            request
                .respond(Response::from_string(
                    json!({"status_code":8,"status_msg":"DO-NOT-PRINT-TOKEN"}).to_string(),
                ))
                .unwrap();
            assert!(
                server
                    .recv_timeout(Duration::from_millis(100))
                    .unwrap()
                    .is_none()
            );
        });
        let report =
            finalize_at_origin(material(dir.path()), Duration::from_secs(5), &origin).unwrap();
        worker.join().unwrap();
        assert_eq!(report["ok"], false);
        assert_eq!(report["accountVerified"], false);
        assert_eq!(report["publishReady"], false);
        assert!(!report.to_string().contains("DO-NOT-PRINT"));
        assert!(dir.path().join(PENDING_FILENAME).is_file());
    }

    #[test]
    fn two_creator_identity_reads_must_agree_before_marking_verified() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::http("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", server.server_addr());
        let worker = thread::spawn(move || {
            for body in [
                json!({"status_code":0,"user":{"uid":"12345","nickname":"本人"}}),
                json!({"status_code":0,"uid":"54321","is_confer_login":false}),
            ] {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                request
                    .respond(Response::from_string(body.to_string()))
                    .unwrap();
            }
        });
        let report =
            finalize_at_origin(material(dir.path()), Duration::from_secs(5), &origin).unwrap();
        worker.join().unwrap();
        assert_eq!(report["accountVerified"], false);
        assert_eq!(report["active"], false);
        assert_eq!(report["publishReady"], false);
        let saved: Value =
            serde_json::from_slice(&fs::read(dir.path().join(PENDING_FILENAME)).unwrap()).unwrap();
        assert!(saved["account"].is_null());
        assert_eq!(saved["stage"], "identity_unconfirmed");
    }

    #[cfg(unix)]
    #[test]
    fn pending_storage_rejects_redirecting_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        fs::create_dir(&real).unwrap();
        let alias = dir.path().join("alias");
        symlink(&real, &alias).unwrap();
        assert!(pending_path(&alias).is_err());
        let destination = real.join(PENDING_FILENAME);
        let untouched = dir.path().join("existing-active.json");
        fs::write(&untouched, b"existing active session").unwrap();
        symlink(&untouched, destination).unwrap();
        assert!(pending_path(&real).is_err());
        assert_eq!(fs::read(untouched).unwrap(), b"existing active session");
    }

    #[test]
    fn account_signatures_and_headers_do_not_claim_unavailable_material() {
        let dir = tempfile::tempdir().unwrap();
        let input = material(dir.path());
        let verified = VerifiedMaterial::new(&input).unwrap();
        let headers = verified.account_headers(MEDIA_ACCOUNT).unwrap();
        assert_eq!(headers["bd-ticket-guard-web-sign-type"], "0");
        assert!(headers.get("x-tt-session-dtrait").is_none());
        let envelope: Value = serde_json::from_slice(
            &STANDARD
                .decode(headers["bd-ticket-guard-client-data"].to_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert!(envelope.get("t_trust").is_none());
        let debug = format!("{headers:?}");
        for name in [
            "cookie",
            "x-secsdk-csrf-token",
            "bd-ticket-guard-client-data",
            "bd-ticket-guard-ree-public-key",
        ] {
            assert!(headers[name].is_sensitive());
            assert!(!debug.contains(headers[name].to_str().unwrap()));
        }
        assert!(verified.account_headers("/passport/web/logout/").is_err());
    }
}
