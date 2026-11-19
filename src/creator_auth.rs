//! Creator-origin credentials and pure Rust signing.
//!
//! Protocol evidence: cv-cat/DouYin_Spider at
//! fc8b9c2490a9561265425c26058290f01c064eda, utils/{bd_ticket,dtrait,passport}.py,
//! builder/header.py and utils/ab_pure.py. These are observed private-web
//! contracts, not an OpenAPI guarantee. No Android Binder, browser process,
//! Python, Node, synthetic device profile or random substitute msToken is used.
//!
//! A creator-session.json is one locally exported login session. It contains
//! secrets: never derive Debug for these types and never return this document
//! from status. A successful local import does not establish online validity;
//! the publishing client must compare the authenticated account with account_id
//! immediately before uploading and again before creating a work.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aes::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use p256::ecdsa::{Signature, SigningKey, signature::Signer};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey};
use p256::{PublicKey, SecretKey};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use ring::{
    digest, hmac,
    rand::{SecureRandom, SystemRandom},
};
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::traits::PublicKeyParts;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{cookie, store, web_sign::WebSigner};

const ORIGIN: &str = "https://creator.douyin.com";
const CREATE: &str = "/web/api/media/aweme/create_v2/";
const REFERER: &str =
    "https://creator.douyin.com/creator-micro/content/post/video?enter_from=publish_page";
const MAX_BUNDLE: u64 = 256 * 1024;
const MAX_COOKIE: usize = 64 * 1024;
// Used only for legacy Cookie GETs, matching the existing web transport.
// Writes require the user agent from the exported session.
const READ_ONLY_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";
const PLATFORM_KEYS: &[&str] = &[
    "cookie_enabled",
    "screen_width",
    "screen_height",
    "browser_language",
    "browser_platform",
    "browser_name",
    "browser_version",
    "browser_online",
    "timezone_name",
    "aid",
    "support_h265",
];

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Bundle {
    schema: u32,
    origin: String,
    account_id: String,
    cookie: String,
    user_agent: String,
    ms_token: String,
    csrf_token: String,
    ticket_guard: TicketGuardBundle,
    dtrait: DtraitBundle,
    platform_params: Vec<(String, String)>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TicketGuardBundle {
    ticket: String,
    ts_sign: String,
    /// PEM SEC1/PKCS8, or a 32-byte scalar encoded as 64 hexadecimal characters.
    private_key: String,
    /// "hmac" or "ecdsa", as observed for this session. Never auto-downgrade.
    sign_mode: String,
    /// Creator-origin server ECDH certificate, "pub.<base64 SEC1>" or PEM.
    #[serde(default)]
    server_cert: Option<String>,
    /// Optional login-origin certificate used for the v2 Cookie proof.
    /// It may differ from the creator-origin signing certificate.
    #[serde(default)]
    binding_server_cert: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DtraitBundle {
    /// Actual device blob captured on the user's own normal login.
    blob: String,
    /// RSA public key and version from that SDK's dtrait configuration.
    pk1_pem: String,
    pk1_version: String,
    sdk_version: String,
}

struct DtraitMaterial {
    blob: String,
    key: [u8; 16],
    encrypted_key: String,
    key_version: String,
    sdk_version: String,
}

enum RequestSigner {
    Hmac(hmac::Key),
    Ecdsa(SigningKey),
}

struct PublishMaterial {
    ticket: String,
    ts_sign: String,
    public_key: String,
    signer: RequestSigner,
    dtrait: DtraitMaterial,
}

/// Secret-bearing object. Intentionally has no Debug or Serialize implementation.
pub struct CreatorAuth {
    cookie: String,
    user_agent: String,
    ms_token: Option<String>,
    csrf_token: Option<String>,
    account_id: Option<String>,
    platform_params: Vec<(String, String)>,
    material: Option<PublishMaterial>,
}

impl CreatorAuth {
    /// Load an atomic creator bundle, or the existing Cookie configuration for
    /// read-only account/status operations. This function performs no network I/O.
    pub fn load(timeout: Duration) -> Result<Self, String> {
        if timeout.is_zero() || timeout > Duration::from_secs(3600) {
            return Err("creator timeout 必须在 1..3600 秒之间".into());
        }
        let path = session_file();
        match fs::symlink_metadata(&path) {
            Ok(_) => Self::from_value(read_bundle(&path)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // Includes config/settings.json migration/fallback.
                Self::read_only(cookie::load()?.unwrap_or_default())
            }
            Err(_) => Err("无法读取创作者登录配置".into()),
        }
    }

    fn read_only(cookie: String) -> Result<Self, String> {
        let parsed = parse_cookie(&cookie)?;
        let ms_token = unique_cookie(&parsed, "msToken")?.map(str::to_owned);
        Ok(Self {
            cookie,
            user_agent: READ_ONLY_UA.into(),
            ms_token,
            csrf_token: None,
            account_id: None,
            platform_params: Vec::new(),
            material: None,
        })
    }

    fn from_value(value: Value) -> Result<Self, String> {
        let bundle: Bundle = serde_json::from_value(value).map_err(|_| {
            "creator 会话包格式无效；需要 schema, origin, account_id, cookie, user_agent, ms_token, csrf_token, ticket_guard, dtrait, platform_params"
                .to_string()
        })?;
        Self::from_bundle(bundle)
    }

    fn from_bundle(bundle: Bundle) -> Result<Self, String> {
        if bundle.schema != 1 || bundle.origin != ORIGIN {
            return Err("creator 会话包 schema/origin 不受支持".into());
        }
        bounded_ascii(&bundle.account_id, "account_id", 128)?;
        if !bundle.account_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("creator account_id 必须是本人账号的数字 UID".into());
        }
        bounded_header(&bundle.user_agent, "user_agent", 2048)?;
        bounded_header(&bundle.ms_token, "ms_token", 4096)?;
        bounded_header(&bundle.csrf_token, "csrf_token", 4096)?;
        bounded_ascii(&bundle.ticket_guard.ticket, "ticket_guard.ticket", 8192)?;
        bounded_ascii(&bundle.ticket_guard.ts_sign, "ticket_guard.ts_sign", 4096)?;
        let parsed = parse_cookie(&bundle.cookie)?;
        require_login_cookie(&parsed)?;
        if unique_cookie(&parsed, "csrf_session_id")?.is_none() {
            return Err(
                "creator 会话包缺少同域 Cookie csrf_session_id，不能混用 www 域 CSRF".into(),
            );
        }
        if let Some(token) = unique_cookie(&parsed, "msToken")?
            && token != bundle.ms_token
        {
            return Err("creator ms_token 与同次 Cookie 不匹配".into());
        }
        validate_platform(&bundle.platform_params, &bundle.user_agent)?;
        let secret = parse_private_key(&bundle.ticket_guard.private_key)?;
        let public_bytes = secret.public_key().to_encoded_point(false);
        verify_session_binding(
            &parsed,
            &bundle.ticket_guard,
            &secret,
            public_bytes.as_bytes(),
        )?;
        let signer = match bundle.ticket_guard.sign_mode.as_str() {
            "hmac" => {
                let cert = bundle
                    .ticket_guard
                    .server_cert
                    .as_deref()
                    .ok_or("creator hmac 缺少 ticket_guard.server_cert")?;
                RequestSigner::Hmac(derive_hmac_key(&secret, cert)?)
            }
            "ecdsa" => RequestSigner::Ecdsa(SigningKey::from(&secret)),
            _ => return Err("ticket_guard.sign_mode 只能是已观测到的 hmac 或 ecdsa".into()),
        };
        let dtrait = DtraitMaterial::new(&bundle.dtrait)?;
        Ok(Self {
            cookie: bundle.cookie,
            user_agent: bundle.user_agent,
            ms_token: Some(bundle.ms_token),
            csrf_token: Some(bundle.csrf_token),
            account_id: Some(bundle.account_id),
            platform_params: bundle.platform_params,
            material: Some(PublishMaterial {
                ticket: bundle.ticket_guard.ticket,
                ts_sign: bundle.ticket_guard.ts_sign,
                public_key: STANDARD.encode(public_bytes.as_bytes()),
                signer,
                dtrait,
            }),
        })
    }

    pub fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }

    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Captured normal browser parameters. Never substitute another machine's
    /// device profile for a missing export.
    pub fn platform_params(&self) -> &[(String, String)] {
        &self.platform_params
    }

    /// Opaque local fingerprint for detecting an account/session change. This is
    /// not a server identity or a token that grants access.
    pub fn stable_session_id(&self) -> Result<String, String> {
        let parsed = parse_cookie(&self.cookie)?;
        require_login_cookie(&parsed)?;
        let mut parts = Vec::new();
        for name in ["sessionid", "sessionid_ss", "sid_tt"] {
            if let Some(value) = unique_cookie(&parsed, name)? {
                parts.push(format!("{name}={value}"));
            }
        }
        let digest = digest::digest(&digest::SHA256, parts.join("\n").as_bytes());
        Ok(digest.as_ref().iter().map(|b| format!("{b:02x}")).collect())
    }

    pub fn ensure_publish_ready(&self) -> Result<Value, String> {
        let parsed = parse_cookie(&self.cookie)?;
        require_login_cookie(&parsed)?;
        if self.material.is_none() {
            let mut missing = vec![
                "account_id",
                "user_agent/session evidence",
                "ticket_guard.ticket",
                "ticket_guard.private_key",
                "ticket_guard.sign_mode",
                "dtrait.blob",
                "dtrait.pk1_pem",
                "dtrait.pk1_version",
                "dtrait.sdk_version",
                "platform_params",
            ];
            if self.ms_token.is_none() {
                missing.push("ms_token");
            }
            if self.csrf_token.is_none() {
                missing.push("csrf_token");
            }
            if unique_cookie(&parsed, "csrf_session_id")?.is_none() {
                missing.push("cookie.csrf_session_id");
            }
            return Err(format!(
                "创作者原生投稿凭据不完整，缺少：{}。请通过正常创作者登录导出同次会话包，再运行 douyin auth creator-import FILE；没有发送上传或投稿请求",
                missing.join(", ")
            ));
        }
        Ok(json!({
            "configured": true,
            "publishReady": true,
            "localSessionBindingVerified": true,
            "onlineVerified": false,
            "accountId": self.account_id,
            "origin": ORIGIN,
            "signingRuntime": "rust",
        }))
    }

    /// This method accepts pathname-only creator routes. It cannot attach
    /// credentials to an arbitrary caller-provided origin or redirect target.
    pub fn headers(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<HeaderMap, String> {
        self.headers_at(method, path, body, unix_seconds()?)
    }

    fn headers_at(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        timestamp: u64,
    ) -> Result<HeaderMap, String> {
        check_route(method, path)?;
        let parsed = parse_cookie(&self.cookie)?;
        require_login_cookie(&parsed)?;
        if method == "POST" {
            self.ensure_publish_ready()?;
            let body = body.ok_or("create_v2 缺少实际请求体")?;
            if body.is_empty() || body.len() > 512 * 1024 {
                return Err("create_v2 请求体为空或超过 512 KiB".into());
            }
            std::str::from_utf8(body).map_err(|_| "create_v2 请求体必须是 UTF-8 JSON")?;
        } else if body.is_some_and(|b| !b.is_empty()) {
            return Err("creator GET 不接受请求体".into());
        }
        let mut headers = HeaderMap::new();
        insert(
            &mut headers,
            "accept",
            "application/json, text/plain, */*",
            false,
        )?;
        insert(&mut headers, "user-agent", &self.user_agent, false)?;
        insert(&mut headers, "origin", ORIGIN, false)?;
        insert(&mut headers, "referer", REFERER, false)?;
        insert(&mut headers, "accept-language", "zh-CN,zh;q=0.9", false)?;
        insert(&mut headers, "cookie", &self.cookie, true)?;
        if method == "POST" {
            insert(&mut headers, "content-type", "application/json", false)?;
        }
        if let Some(csrf) = &self.csrf_token {
            insert(&mut headers, "x-secsdk-csrf-token", csrf, true)?;
        }
        if let Some(material) = &self.material {
            material.add_headers(&mut headers, path, timestamp)?;
        }
        Ok(headers)
    }

    /// Exact encoding first; the signature covers those query bytes and the
    /// exact UTF-8 request body. The caller must send this returned query intact.
    pub fn sign_query(
        &self,
        params: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<String, String> {
        if params.len() > 128 {
            return Err("creator query 参数过多".into());
        }
        let mut seen = BTreeSet::new();
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (name, value) in params {
            if name.is_empty()
                || name.len() > 128
                || value.len() > 8192
                || name == "a_bogus"
                || name == "msToken"
                || !seen.insert(name.as_str())
            {
                return Err("creator query 包含无效、重复或外部签名字段".into());
            }
            serializer.append_pair(name, value);
        }
        if let Some(token) = &self.ms_token {
            serializer.append_pair("msToken", token);
        }
        let query = serializer.finish();
        if query.len() > 32 * 1024 {
            return Err("creator query 超过 32 KiB".into());
        }
        let body = body.unwrap_or_default();
        if !body.is_empty() {
            self.ensure_publish_ready()?;
        }
        let body = std::str::from_utf8(body).map_err(|_| "creator 签名请求体必须是 UTF-8")?;
        let signer = WebSigner {
            aid: 2906,
            page_id: 33638,
        };
        let now = unix_millis()?;
        let signature = signer.sign(&query, body, &self.user_agent, now);
        let mut final_query = url::form_urlencoded::Serializer::new(query);
        final_query.append_pair("a_bogus", &signature);
        Ok(final_query.finish())
    }

    pub fn status(&self) -> Value {
        let ready = self.material.is_some();
        let cookie_present = !self.cookie.is_empty();
        json!({
            "configured": cookie_present,
            "creatorBundleConfigured": ready,
            "publishReady": ready,
            "onlineVerified": false,
            "accountId": self.account_id,
            "hasMsToken": self.ms_token.is_some(),
            "hasCreatorCsrf": self.csrf_token.is_some(),
            "localSessionBindingVerified": ready,
            "credentialValuesRedacted": true,
        })
    }
}

impl PublishMaterial {
    fn add_headers(
        &self,
        headers: &mut HeaderMap,
        path: &str,
        timestamp: u64,
    ) -> Result<(), String> {
        let message = format!("ticket={}&path={path}&timestamp={timestamp}", self.ticket);
        let (req_sign, sign_type) = match &self.signer {
            RequestSigner::Hmac(key) => (
                STANDARD.encode(hmac::sign(key, message.as_bytes()).as_ref()),
                "1",
            ),
            RequestSigner::Ecdsa(key) => {
                let signature: Signature = key.sign(message.as_bytes());
                (STANDARD.encode(signature.to_der().as_bytes()), "0")
            }
        };
        // Struct serialization preserves the browser envelope's member order.
        // The current official secure-consumer SDK adds t_trust only when it
        // uses a server-derived trusted clock. This API supplies local Unix
        // time explicitly, so a trust Cookie alone must never add that flag.
        #[derive(Serialize)]
        struct ClientData<'a> {
            ts_sign: &'a str,
            req_content: &'static str,
            req_sign: String,
            timestamp: u64,
        }
        let client_data = serde_json::to_vec(&ClientData {
            ts_sign: &self.ts_sign,
            req_content: "ticket,path,timestamp",
            req_sign,
            timestamp,
        })
        .map_err(|_| "无法构造 creator ticketguard 数据")?;
        insert(
            headers,
            "bd-ticket-guard-client-data",
            &STANDARD.encode(client_data),
            true,
        )?;
        insert(
            headers,
            "bd-ticket-guard-ree-public-key",
            &self.public_key,
            true,
        )?;
        insert(headers, "bd-ticket-guard-version", "2", false)?;
        insert(
            headers,
            "bd-ticket-guard-web-version",
            if self.ts_sign.starts_with("ts.1") {
                "1"
            } else {
                "2"
            },
            false,
        )?;
        insert(headers, "bd-ticket-guard-web-sign-type", sign_type, false)?;
        insert(
            headers,
            "x-tt-session-dtrait",
            &self.dtrait.header(path, timestamp)?,
            true,
        )?;
        Ok(())
    }
}

impl DtraitMaterial {
    fn new(bundle: &DtraitBundle) -> Result<Self, String> {
        bounded_ascii(&bundle.blob, "dtrait.blob", 32 * 1024)?;
        bounded_ascii(&bundle.pk1_version, "dtrait.pk1_version", 32)?;
        bounded_ascii(&bundle.sdk_version, "dtrait.sdk_version", 64)?;
        if !bundle
            .pk1_version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            return Err("dtrait.pk1_version 格式无效".into());
        }
        if bundle.pk1_pem.len() > 8192 {
            return Err("dtrait.pk1_pem 过长".into());
        }
        let public = RsaPublicKey::from_pkcs1_pem(&bundle.pk1_pem)
            .or_else(|_| RsaPublicKey::from_public_key_pem(&bundle.pk1_pem))
            .map_err(|_| "dtrait.pk1_pem 不是有效 RSA 公钥")?;
        if !(2048..=4096).contains(&public.n().bits()) {
            return Err("dtrait RSA 公钥长度必须在 2048..4096 位".into());
        }
        let mut key = [0u8; 16];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| "系统安全随机数不可用")?;
        let key_hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let encrypted = public
            .encrypt(
                &mut rsa::rand_core::OsRng,
                Pkcs1v15Encrypt,
                key_hex.as_bytes(),
            )
            .map_err(|_| "无法封装 dtrait 会话密钥")?;
        Ok(Self {
            blob: bundle.blob.clone(),
            key,
            encrypted_key: STANDARD.encode(encrypted),
            key_version: bundle.pk1_version.clone(),
            sdk_version: bundle.sdk_version.clone(),
        })
    }

    fn header(&self, path: &str, timestamp: u64) -> Result<String, String> {
        #[derive(Serialize)]
        struct Payload<'a> {
            dtrait: &'a str,
            timestamp: u64,
            #[serde(rename = "sdkVersion")]
            sdk_version: &'a str,
            path: &'a str,
        }
        let plaintext = serde_json::to_vec(&Payload {
            dtrait: &self.blob,
            timestamp,
            sdk_version: &self.sdk_version,
            path,
        })
        .map_err(|_| "无法构造 dtrait 请求材料")?;
        let mut iv = [0u8; 16];
        SystemRandom::new()
            .fill(&mut iv)
            .map_err(|_| "系统安全随机数不可用")?;
        let ciphertext = cbc::Encryptor::<aes::Aes128>::new((&self.key).into(), (&iv).into())
            .encrypt_padded_vec_mut::<Pkcs7>(&plaintext);
        let mut data = iv.to_vec();
        data.extend_from_slice(&ciphertext);
        Ok(format!(
            "{}_{}_{}",
            self.key_version,
            self.encrypted_key,
            STANDARD.encode(data)
        ))
    }
}

fn check_route(method: &str, path: &str) -> Result<(), String> {
    let allowed = match method {
        "POST" => path == CREATE,
        "GET" => matches!(
            path,
            "/web/api/media/user/info/"
                | "/aweme/v1/creator/pc/user/info/"
                | "/aweme/v1/creator/user/info/"
                | "/web/api/media/upload/auth/v5/"
                | "/janus/douyin/creator/pc/work_list"
        ),
        _ => false,
    };
    if !allowed {
        return Err("creator 凭据仅可用于明确允许的同源账号、上传授权、本人作品和投稿路径".into());
    }
    Ok(())
}

fn insert(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
    secret: bool,
) -> Result<(), String> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| format!("creator header {name} 格式无效（值已隐藏）"))?;
    value.set_sensitive(secret);
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

fn bounded_header(value: &str, field: &str, max: usize) -> Result<(), String> {
    if value.is_empty()
        || value.len() > max
        || value.chars().any(char::is_control)
        || HeaderValue::from_str(value).is_err()
    {
        Err(format!(
            "creator 字段 {field} 缺失、过长或格式无效（值已隐藏）"
        ))
    } else {
        Ok(())
    }
}

fn bounded_ascii(value: &str, field: &str, max: usize) -> Result<(), String> {
    if !value.is_ascii() {
        return Err(format!("creator 字段 {field} 必须是 ASCII（值已隐藏）"));
    }
    bounded_header(value, field, max)
}

/// Preserve the exact header in CreatorAuth, including legitimate same-name
/// cookies with different scopes. Security fields must nevertheless be unique.
fn parse_cookie(cookie: &str) -> Result<BTreeMap<String, Vec<String>>, String> {
    if cookie.is_empty() {
        return Ok(BTreeMap::new());
    }
    bounded_ascii(cookie, "cookie", MAX_COOKIE)?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for part in cookie.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let Some((name, value)) = part.split_once('=') else {
            return Err("creator Cookie 包含无效片段（值已隐藏）".into());
        };
        if name.is_empty()
            || name
                .bytes()
                .any(|b| b <= b' ' || b >= 127 || b"()<>@,;:\\\"/[]?={}".contains(&b))
        {
            return Err("creator Cookie 名称无效".into());
        }
        out.entry(name.to_string())
            .or_default()
            .push(value.to_string());
    }
    Ok(out)
}

fn unique_cookie<'a>(
    cookies: &'a BTreeMap<String, Vec<String>>,
    name: &str,
) -> Result<Option<&'a str>, String> {
    let Some(values) = cookies.get(name) else {
        return Ok(None);
    };
    let first = values.first().map(String::as_str).unwrap_or_default();
    if values.iter().any(|v| v != first) {
        return Err(format!(
            "creator Cookie {name} 存在多个冲突值，无法核对会话"
        ));
    }
    Ok((!first.is_empty()).then_some(first))
}

fn require_login_cookie(cookies: &BTreeMap<String, Vec<String>>) -> Result<(), String> {
    if unique_cookie(cookies, "sessionid")?.is_none()
        && unique_cookie(cookies, "sessionid_ss")?.is_none()
    {
        Err("缺少已登录 Cookie，请先正常登录创作者中心并导入本机会话".into())
    } else {
        Ok(())
    }
}

fn parse_private_key(value: &str) -> Result<SecretKey, String> {
    if value.len() > 8192 {
        return Err("ticket_guard.private_key 过长".into());
    }
    let result = if value.starts_with("-----BEGIN") {
        SecretKey::from_pkcs8_pem(value).or_else(|_| SecretKey::from_sec1_pem(value))
    } else {
        let bytes =
            decode_hex(value).ok_or("ticket_guard.private_key 必须是 P-256 PEM 或 64 位 hex")?;
        SecretKey::from_slice(&bytes)
    };
    result.map_err(|_| "ticket_guard.private_key 不是有效 P-256 私钥（值已隐藏）".into())
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() != 64 || !value.is_ascii() {
        return None;
    }
    value
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

fn server_public_key(cert: &str) -> Result<PublicKey, String> {
    if cert.len() > 16384 {
        return Err("ticket_guard server certificate 过长".into());
    }
    if let Some(point) = cert.strip_prefix("pub.") {
        let raw = STANDARD
            .decode(point)
            .map_err(|_| "ticket_guard 服务端公钥编码无效")?;
        return PublicKey::from_sec1_bytes(&raw)
            .map_err(|_| "ticket_guard 服务端 P-256 公钥无效".into());
    }
    if let Ok(public) = PublicKey::from_public_key_pem(cert) {
        return Ok(public);
    }
    // The reference receives an X.509 PEM. Locate the complete, fixed P-256
    // SPKI prefix and validate the point, without interpreting untrusted ASN.1.
    let body = cert
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<String>();
    let der = STANDARD
        .decode(body)
        .map_err(|_| "ticket_guard 服务端证书编码无效")?;
    const PREFIX: &[u8] = &[
        0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
        0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00, 0x04,
    ];
    let positions: Vec<_> = der
        .windows(PREFIX.len())
        .enumerate()
        .filter_map(|(i, w)| (w == PREFIX).then_some(i))
        .collect();
    if positions.len() != 1 {
        return Err("ticket_guard 证书没有唯一的 P-256 公钥".into());
    }
    let start = positions[0] + PREFIX.len() - 1;
    let raw = der
        .get(start..start + 65)
        .ok_or("ticket_guard 服务端证书被截断")?;
    PublicKey::from_sec1_bytes(raw).map_err(|_| "ticket_guard 服务端 P-256 公钥无效".into())
}

fn derive_hmac_key(secret: &SecretKey, cert: &str) -> Result<hmac::Key, String> {
    let public = server_public_key(cert)?;
    let shared = p256::ecdh::diffie_hellman(secret.to_nonzero_scalar(), public.as_affine());
    // HKDF-SHA256 with empty salt and info, one 32-byte output block.
    let salt = hmac::Key::new(hmac::HMAC_SHA256, &[0u8; 32]);
    let extract = hmac::sign(&salt, shared.raw_secret_bytes().as_slice());
    let prk = hmac::Key::new(hmac::HMAC_SHA256, extract.as_ref());
    let expand = hmac::sign(&prk, &[1]);
    Ok(hmac::Key::new(hmac::HMAC_SHA256, expand.as_ref()))
}

fn decode_cookie_object(value: &str) -> Result<Value, String> {
    if value.len() > 32 * 1024 {
        return Err("ticketguard Cookie 过长".into());
    }
    let mut decoded = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let pair = bytes
                .get(i + 1..i + 3)
                .ok_or("ticketguard Cookie 百分号编码无效")?;
            let a = (pair[0] as char)
                .to_digit(16)
                .ok_or("ticketguard Cookie 百分号编码无效")?;
            let b = (pair[1] as char)
                .to_digit(16)
                .ok_or("ticketguard Cookie 百分号编码无效")?;
            decoded.push(((a << 4) | b) as u8);
            i += 3;
        } else {
            // Unlike form decoding, literal '+' in standard base64 stays '+'.
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    while decoded.len() % 4 != 0 {
        decoded.push(b'=');
    }
    let data = STANDARD
        .decode(decoded)
        .map_err(|_| "ticketguard Cookie base64 无效")?;
    serde_json::from_slice(&data).map_err(|_| "ticketguard Cookie JSON 无效".into())
}

fn verify_session_binding(
    cookies: &BTreeMap<String, Vec<String>>,
    guard: &TicketGuardBundle,
    private: &SecretKey,
    public: &[u8],
) -> Result<(), String> {
    let mut public_bound = false;
    let mut sign_bound = false;
    if let Some(id) = unique_cookie(cookies, "bd_ticket_guard_ts_sign_id")? {
        if !guard.ts_sign.starts_with(id) {
            return Err("ticket_guard.ts_sign 与 Cookie 登录会话不匹配".into());
        }
        sign_bound = true;
    }
    if let Some(raw) = unique_cookie(cookies, "bd_ticket_guard_client_data")? {
        let data = decode_cookie_object(raw)?;
        if let Some(key) = data
            .get("bd-ticket-guard-ree-public-key")
            .and_then(Value::as_str)
        {
            let captured = STANDARD
                .decode(key)
                .map_err(|_| "Cookie 中的 ticketguard 公钥无效")?;
            if captured != public {
                return Err(
                    "ticket_guard.private_key 与 Cookie 公钥不匹配；不能混用不同次登录".into(),
                );
            }
            public_bound = true;
        }
    }
    if let Some(raw) = unique_cookie(cookies, "bd_ticket_guard_client_data_v2")? {
        let data = decode_cookie_object(raw)?;
        let key = data
            .get("ree_public_key")
            .and_then(Value::as_str)
            .ok_or("ticketguard v2 Cookie 缺少 ree_public_key")?;
        let captured = STANDARD
            .decode(key)
            .map_err(|_| "ticketguard v2 公钥编码无效")?;
        if captured != public {
            return Err("ticket_guard.private_key 与 v2 Cookie 公钥不匹配".into());
        }
        public_bound = true;
        let sign = data
            .get("ts_sign")
            .and_then(Value::as_str)
            .ok_or("ticketguard v2 Cookie 缺少同次 ts_sign")?;
        if sign != guard.ts_sign {
            return Err("ticket_guard.ts_sign 与 v2 Cookie 登录会话不匹配".into());
        }
        sign_bound = true;
        // If supplied, verify the existing login Cookie's proof locally.
        // Certificate scope is explicit; never fetch a different origin's key.
        if let Some(cert) = guard.binding_server_cert.as_deref() {
            if data.get("req_content").and_then(Value::as_str) != Some("sec_ts") {
                return Err("ticketguard v2 Cookie req_content 不是已知 sec_ts 契约".into());
            }
            let sec_ts = data
                .get("sec_ts")
                .and_then(Value::as_str)
                .ok_or("ticketguard v2 Cookie 缺少 sec_ts")?;
            let signature = data
                .get("req_sign")
                .and_then(Value::as_str)
                .ok_or("ticketguard v2 Cookie 缺少 req_sign")?;
            let signature = STANDARD
                .decode(signature)
                .map_err(|_| "ticketguard v2 proof 编码无效")?;
            let key = derive_hmac_key(private, cert)?;
            hmac::verify(&key, format!("sec_ts={sec_ts}").as_bytes(), &signature)
                .map_err(|_| "ticketguard v2 proof 与私钥/登录域服务端证书不匹配")?;
        }
    }
    if let Some(raw) = unique_cookie(cookies, "bd_ticket_guard_server_data")? {
        let data = decode_cookie_object(raw)?;
        if data.get("ticket").and_then(Value::as_str) != Some(guard.ticket.as_str())
            || data.get("ts_sign").and_then(Value::as_str) != Some(guard.ts_sign.as_str())
        {
            return Err("ticket_guard 与 Cookie 的服务端签发记录不匹配".into());
        }
    }
    if !public_bound || !sign_bound {
        return Err(
            "缺少可验证的同次登录绑定：需要 Cookie 公钥与 ts_sign/v2 记录；不会把未知绑定当作成功"
                .into(),
        );
    }
    Ok(())
}

fn validate_platform(params: &[(String, String)], user_agent: &str) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for (name, value) in params {
        if !PLATFORM_KEYS.contains(&name.as_str()) || !names.insert(name.as_str()) {
            return Err("platform_params 包含未知或重复字段".into());
        }
        bounded_header(value, "platform_params", 2048)?;
    }
    if PLATFORM_KEYS.iter().any(|key| !names.contains(key)) {
        return Err(
            "platform_params 缺少正常创作者会话的浏览器参数；不能使用固定他人设备档案".into(),
        );
    }
    let value = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    if value("aid") != Some("1128")
        || value("browser_version")
            != Some(user_agent.strip_prefix("Mozilla/").unwrap_or(user_agent))
    {
        return Err("platform_params 与创作者应用/同次 user_agent 不匹配".into());
    }
    for name in ["screen_width", "screen_height"] {
        if !value(name)
            .and_then(|v| v.parse::<u32>().ok())
            .is_some_and(|n| (1..=32768).contains(&n))
        {
            return Err("platform_params 屏幕尺寸无效".into());
        }
    }
    Ok(())
}

fn unix_seconds() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "系统时间早于 Unix epoch".into())
}

fn unix_millis() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间早于 Unix epoch".to_string())?
        .as_millis()
        .try_into()
        .map_err(|_| "系统时间超出签名范围".into())
}

pub fn session_file() -> PathBuf {
    store::root().join("creator-session.json")
}

fn read_bundle(path: &Path) -> Result<Value, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "无法读取 creator 会话包")?;
    if !metadata.is_file() || metadata.len() > MAX_BUNDLE {
        return Err("creator 会话包必须是普通文件且不超过 256 KiB".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err("creator 会话包含登录密钥，请先 chmod 600 FILE 再导入".into());
        }
    }
    let file = File::open(path).map_err(|_| "无法打开 creator 会话包")?;
    let mut bytes = Vec::new();
    file.take(MAX_BUNDLE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取 creator 会话包")?;
    if bytes.len() as u64 > MAX_BUNDLE {
        return Err("creator 会话包超过 256 KiB".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "creator 会话包 JSON 损坏（内容已隐藏）".into())
}

/// Local-only validation/import. Does not log in, upload, publish, or contact
/// any service. The next authenticated GET must verify account_id.
pub fn import_file(path: &Path) -> Result<Value, String> {
    import_to(path, &session_file())
}

fn import_to(source: &Path, destination: &Path) -> Result<Value, String> {
    let value = read_bundle(source)?;
    let auth = CreatorAuth::from_value(value.clone())?;
    auth.ensure_publish_ready()?;
    store::save_at(destination, &value).map_err(|_| "无法安全保存 creator 会话包")?;
    #[cfg(unix)]
    if let Some(parent) = destination.parent() {
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| "creator 会话包已保存，但目录同步失败")?;
    }
    Ok(json!({
        "imported": true,
        "accountId": auth.account_id(),
        "localSessionBindingVerified": true,
        "onlineVerified": false,
        "publishPerformed": false,
        "credentialValuesRedacted": true,
    }))
}

/// Offline status only; online identity verification belongs to the native
/// creator client so it uses exactly the same origin and HTTP configuration.
pub fn status() -> Result<Value, String> {
    CreatorAuth::load(Duration::from_secs(30)).map(|auth| auth.status())
}

pub fn logout() -> Result<Value, String> {
    let removed = match fs::remove_file(session_file()) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err("无法删除本地 creator 会话包".into()),
    };
    Ok(json!({"loggedOut": true, "removedCreatorBundle": removed}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockDecryptMut;
    use rsa::pkcs1::{EncodeRsaPublicKey, LineEnding};
    use std::sync::OnceLock;

    fn fixture() -> Value {
        static PUBLIC: OnceLock<String> = OnceLock::new();
        let rsa_public = PUBLIC.get_or_init(|| {
            let private = rsa::RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
            private
                .to_public_key()
                .to_pkcs1_pem(LineEnding::LF)
                .unwrap()
        });
        let private = SecretKey::from_slice(&[1u8; 32]).unwrap();
        let server = SecretKey::from_slice(&[2u8; 32]).unwrap();
        let public = STANDARD.encode(private.public_key().to_encoded_point(false).as_bytes());
        let cert = format!(
            "pub.{}",
            STANDARD.encode(server.public_key().to_encoded_point(false).as_bytes())
        );
        let hmac_key = derive_hmac_key(&private, &cert).unwrap();
        let ts_sign = "ts.2.fixture-only-proof";
        let proof = json!({
            "ree_public_key": public,
            "ts_sign": ts_sign,
            "req_content": "sec_ts",
            "req_sign": STANDARD.encode(hmac::sign(&hmac_key, b"sec_ts=#fixture").as_ref()),
            "sec_ts": "#fixture",
        });
        let cookie = format!(
            "sessionid=fixture-only-session; csrf_session_id=fixture-csrf; _bd_ticket_crypt_cookie=fixture-trust-cookie; bd_ticket_guard_client_data_v2={}",
            STANDARD.encode(serde_json::to_vec(&proof).unwrap())
        );
        let ua = "Mozilla/5.0 fixture-local-device";
        json!({
            "schema": 1, "origin": ORIGIN, "account_id": "12345",
            "cookie": cookie, "user_agent": ua,
            "ms_token": "fixture-server-issued-ms-token", "csrf_token": "fixture-origin-token",
            "ticket_guard": {
                "ticket": "fixture-ticket-never-log",
                "ts_sign": ts_sign,
                "private_key": "01".repeat(32),
                "sign_mode": "hmac",
                "server_cert": cert,
                "binding_server_cert": cert,
            },
            "dtrait": {
                "blob": "fixture-local-device-blob",
                "pk1_pem": rsa_public,
                "pk1_version": "d0", "sdk_version": "1.0.0.16",
            },
            "platform_params": [
                ["cookie_enabled", "true"], ["screen_width", "1280"], ["screen_height", "720"],
                ["browser_language", "zh-CN"], ["browser_platform", "Linux x86_64"],
                ["browser_name", "Mozilla"], ["browser_version", "5.0 fixture-local-device"],
                ["browser_online", "true"], ["timezone_name", "Etc/UTC"],
                ["aid", "1128"], ["support_h265", "1"]
            ],
        })
    }

    fn error_of(value: Value) -> String {
        match CreatorAuth::from_value(value) {
            Err(error) => error,
            Ok(_) => panic!("expected rejected session"),
        }
    }

    #[test]
    fn legacy_cookie_supports_reads_but_stops_before_upload_or_publish() {
        let auth = CreatorAuth::read_only("sessionid=local-test".into()).unwrap();
        let get = auth
            .headers("GET", "/web/api/media/user/info/", None)
            .unwrap();
        assert!(get["cookie"].is_sensitive());
        assert!(
            auth.ensure_publish_ready()
                .unwrap_err()
                .contains("ticket_guard.private_key")
        );
        assert!(auth.headers("POST", CREATE, Some(b"{}")).is_err());
        assert_eq!(auth.status()["publishReady"], false);
        assert!(!auth.status().to_string().contains("local-test"));
    }

    #[test]
    fn imported_private_key_and_ts_sign_must_belong_to_cookie_session() {
        let mut value = fixture();
        value["ticket_guard"]["private_key"] = Value::String("03".repeat(32));
        let error = error_of(value);
        assert!(error.contains("公钥不匹配"));
        assert!(!error.contains("fixture-ticket"));
        let mut value = fixture();
        value["ticket_guard"]["ts_sign"] = "ts.2.different-login".into();
        assert!(error_of(value).contains("登录会话不匹配"));
    }

    #[test]
    fn imported_binding_certificate_must_verify_existing_cookie_hmac() {
        let mut value = fixture();
        let server = SecretKey::from_slice(&[4u8; 32]).unwrap();
        value["ticket_guard"]["binding_server_cert"] = format!(
            "pub.{}",
            STANDARD.encode(server.public_key().to_encoded_point(false).as_bytes())
        )
        .into();
        assert!(error_of(value).contains("proof"));
    }

    #[test]
    fn ticketguard_signs_path_time_and_dtrait_is_rebuilt_with_fresh_iv() {
        let auth = CreatorAuth::from_value(fixture()).unwrap();
        let first = auth
            .headers_at("POST", CREATE, Some(b"{}"), 1700000001)
            .unwrap();
        let second = auth
            .headers_at("GET", "/web/api/media/user/info/", None, 1700000002)
            .unwrap();
        assert_ne!(
            first["bd-ticket-guard-client-data"],
            second["bd-ticket-guard-client-data"]
        );
        let data: Value = serde_json::from_slice(
            &STANDARD
                .decode(first["bd-ticket-guard-client-data"].to_str().unwrap())
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["timestamp"], 1700000001_u64);
        assert_eq!(data["req_content"], "ticket,path,timestamp");
        assert!(data.get("t_trust").is_none());
        let safe_debug = format!("{first:?}");
        for sensitive_header in [
            "cookie",
            "x-secsdk-csrf-token",
            "bd-ticket-guard-client-data",
            "bd-ticket-guard-ree-public-key",
            "x-tt-session-dtrait",
        ] {
            assert!(first[sensitive_header].is_sensitive());
            assert!(!safe_debug.contains(first[sensitive_header].to_str().unwrap()));
        }
        assert_eq!(first["bd-ticket-guard-web-sign-type"], "1");
        assert_eq!(first["bd-ticket-guard-version"], "2");
        assert!(first["bd-ticket-guard-client-data"].is_sensitive());
        let key = &auth.material.as_ref().unwrap().dtrait.key;
        let decode_payload = |headers: &HeaderMap| {
            let text = headers["x-tt-session-dtrait"].to_str().unwrap();
            let segment = text.split('_').nth(2).unwrap();
            let bytes = STANDARD.decode(segment).unwrap();
            let clear = cbc::Decryptor::<aes::Aes128>::new_from_slices(key, &bytes[..16])
                .unwrap()
                .decrypt_padded_vec_mut::<Pkcs7>(&bytes[16..])
                .unwrap();
            serde_json::from_slice::<Value>(&clear).unwrap()
        };
        assert_eq!(decode_payload(&first)["path"], CREATE);
        assert_eq!(decode_payload(&first)["timestamp"], 1700000001_u64);
        assert_eq!(decode_payload(&second)["path"], "/web/api/media/user/info/");
        assert_eq!(decode_payload(&second)["timestamp"], 1700000002_u64);
        let repeated = auth
            .headers_at("POST", CREATE, Some(b"{}"), 1700000001)
            .unwrap();
        assert_ne!(
            first["x-tt-session-dtrait"],
            repeated["x-tt-session-dtrait"]
        );
    }

    #[test]
    fn secrets_are_never_signed_for_other_origins_or_unapproved_paths() {
        let auth = CreatorAuth::read_only("sessionid=local-test".into()).unwrap();
        for path in [
            "https://example.com/web/api/media/user/info/",
            "//example.com/",
            "/web/api/media/user/info/?redirect=1",
            "/passport/web/logout/",
        ] {
            assert!(auth.headers("GET", path, None).is_err());
        }
        assert!(
            auth.headers("POST", "/web/api/media/user/info/", Some(b"{}"))
                .is_err()
        );
        assert!(
            auth.sign_query(&[("msToken".into(), "external-token".into())], None)
                .is_err()
        );
    }

    #[test]
    fn malformed_or_mixed_cookie_fields_fail_without_exposing_values() {
        let auth = CreatorAuth::read_only("sessionid=one; sessionid=two".into()).unwrap();
        assert!(
            auth.headers("GET", "/web/api/media/user/info/", None)
                .unwrap_err()
                .contains("冲突")
        );
        let mut value = fixture();
        value["cookie"] = format!(
            "{}; msToken=another-session-token",
            value["cookie"].as_str().unwrap()
        )
        .into();
        assert!(error_of(value).contains("ms_token"));
        let mut value = fixture();
        value["dtrait"]["blob"] = "".into();
        let error = error_of(value);
        assert!(error.contains("dtrait.blob"));
        assert!(!error.contains("fixture-ticket-never-log"));
    }

    #[test]
    fn import_is_private_atomic_and_does_not_replace_valid_state_with_invalid_input() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("export.json");
        let destination = dir.path().join("state/creator-session.json");
        store::save_at(&source, &fixture()).unwrap();
        let report = import_to(&source, &destination).unwrap();
        assert_eq!(report["onlineVerified"], false);
        assert_eq!(report["publishPerformed"], false);
        let saved = fs::read(&destination).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(destination.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        store::save_at(&source, &json!({"schema": 1, "cookie": "DO-NOT-ECHO"})).unwrap();
        let error = import_to(&source, &destination).unwrap_err();
        assert!(!error.contains("DO-NOT-ECHO"));
        assert_eq!(fs::read(destination).unwrap(), saved);
    }
}
