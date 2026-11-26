//! Native Creator QR login: fresh keys, scoped HTTP cookies and explicit stops.
//!
//! Current Creator LoginCardV3 (571.ce070588.js) configures aid 2906
//! and its public app-key. douyin-login-new 2.4.22 calls get_qrcode with
//! GET and check_qrconnect with a form POST. A fresh P-256 public key
//! accompanies the provider request. No sampled browser device profile,
//! msToken, ticket or dtrait is fabricated.
//!
//! A successful QR response is only a pending login. In a real anonymous
//! probe on 2026-11-19, get_qrcode succeeded and the first unscanned poll
//! returned error_code 4031 without a challenge. That is server_blocked,
//! not evidence that scanning or solving a CAPTCHA would make login work.
//! Confirmed credentials remain inactive until independent account checks.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use clap::Args;
use p256::SecretKey;
use p256::elliptic_curve::{rand_core::OsRng, sec1::ToEncodedPoint};
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, LineEnding};
use qrcode::{Color, QrCode};
use reqwest::Method;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE};
use ring::{digest, hmac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::store;

const ORIGIN: &str = "https://creator.douyin.com";
// Public client configuration, not an account secret. Current LoginCardV3:
// creator_pc_vmok_common/static/js/async/571.ce070588.js, build 1.0.0.413.
// It loads douyin-login-new 2.4.22 and WebInterfaceSdk pro 3.4.20.
// The SDK's app-key middleware and p_no formula are in chunk
// 6a82236d8bafdc4b.umd.production.js; no sampled browser profile is used.
const PUBLIC_APP_KEY: &str = "6ddd3ec693f3a124adb29b91b244ece5";
const UI_VERSION: &str = "2.4.22";
const PASSPORT_VERSION: &str = "3.4.20";
// LoginCardV3 normalizes the default through new URL(...).href, yielding
// this trailing slash. Both requests must use the exact same callback.
const NEXT: &str = "https://creator.douyin.com/";
const GET_QR: &str = "/passport/web/get_qrcode/";
const CHECK_QR: &str = "/passport/web/check_qrconnect/";
const CREATOR_CSRF: &str = "/web/api/media/anchor/search";
const PENDING_FILE: &str = "login.pending.json";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const MAX_PENDING: u64 = 512 * 1024;
const LOCAL_RESUME_WINDOW: u64 = 30 * 60;
const UA: &str = concat!(
    "douyin-cli/",
    env!("CARGO_PKG_VERSION"),
    " (native creator login)"
);

#[derive(Args)]
#[command(about = "原生扫码身份验证；当前不能从扫码结果激活投稿会话，先用 --capabilities 查看边界")]
pub struct CreatorLoginArgs {
    /// 离线查看真实登录与投稿能力；不生成二维码、不读取凭据、不访问网络
    #[arg(long, conflicts_with_all = ["qr_only", "poll_once", "resume"])]
    capabilities: bool,
    /// Maximum time to wait in this invocation, in seconds.
    #[arg(long, default_value_t = 180, value_parser = clap::value_parser!(u64).range(5..=1800))]
    timeout: u64,
    /// Seconds between polls; a rejection never triggers an automatic retry.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u64).range(5..=60))]
    poll_interval: u64,
    /// Obtain and save a QR challenge; use --resume to continue the same login.
    #[arg(long, conflicts_with = "poll_once")]
    qr_only: bool,
    /// Poll once, save the state, then stop; pending login can be resumed.
    #[arg(long, conflicts_with = "qr_only")]
    poll_once: bool,
    /// Resume the private pending directory returned by an earlier invocation.
    #[arg(long, value_name = "PENDING_DIR")]
    resume: Option<PathBuf>,
}

/// Secret-bearing handoff. Never serialize this as a CLI result or Debug log.
pub(crate) struct NativeLoginMaterial {
    pub cookie: String,
    pub private_key_pem: String,
    pub public_key_base64: String,
    pub server_data: Value,
    /// Creator X-Ware-Csrf-Token material, not passport_csrf_token.
    pub csrf_token: Option<String>,
    pub ms_token: Option<String>,
    pub user_agent: String,
    pub pending_dir: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingSession {
    schema: u32,
    origin: String,
    stage: String,
    created_at: u64,
    last_poll_at: Option<u64>,
    last_http_status: Option<u16>,
    last_business_code: Option<i64>,
    private_key_pem: String,
    public_key_base64: String,
    user_agent: String,
    cookies: Vec<ScopedCookie>,
    creator_csrf_token: Option<String>,
    server_data: Option<Value>,
    challenge: Option<Challenge>,
    redirect_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Challenge {
    token: String,
    index_url: String,
    is_frontier: bool,
    /// Preserve the server's value without guessing whether it is a TTL or epoch.
    server_expire_time: Option<Value>,
    received_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedCookie {
    name: String,
    value: String,
    domain: String,
    path: String,
    host_only: bool,
    secure: bool,
    expires_at: Option<u64>,
}

struct Reply {
    status: u16,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq)]
enum Poll {
    New,
    Scanned,
    Confirmed,
    Expired,
    Cancelled,
    Stopped,
}

struct LoginFlow {
    client: Client,
    origin: Url,
    directory: PathBuf,
    pending: PendingSession,
}

pub fn run(args: CreatorLoginArgs) -> Result<Value, String> {
    if args.capabilities {
        return Ok(capabilities());
    }
    if !(5..=1800).contains(&args.timeout) || !(5..=60).contains(&args.poll_interval) {
        return Err("creator_login: timeout 必须为 5..1800 秒，poll-interval 为 5..60 秒".into());
    }
    eprintln!(
        "当前扫码流程仅核验身份并保存待完成材料，尚不能激活投稿会话；重复扫码不会补齐 Creator 发布材料。可先用 auth creator-login --capabilities 查看可行路径。"
    );
    let started = Instant::now();
    let total = Duration::from_secs(args.timeout);
    // The sidecar inode is stable. Hold its OS lock across load, validation,
    // all HTTP requests, atomic saves and finalization, including early errors.
    let (directory, pending, _session_lock) = match args.resume {
        Some(directory) => {
            let (pending, guard) = resume_pending(&directory)?;
            (directory, pending, guard)
        }
        None => new_pending(&store::root().join("creator-login"))?,
    };
    let timeout = total.min(Duration::from_secs(20));
    let mut flow = LoginFlow::new(directory, pending, timeout)?;
    if flow.pending.challenge.is_none() {
        flow.begin()?;
    }
    if flow.pending.stage == "confirmed" || flow.pending.stage == "identity_pending" {
        return flow.finish(timeout);
    }
    flow.show_qr()?;
    if args.qr_only {
        return Ok(flow.public_state("qr_ready"));
    }

    loop {
        let previous = flow
            .pending
            .last_poll_at
            .unwrap_or_else(|| flow.pending.challenge.as_ref().map_or(0, |c| c.received_at));
        let wait = previous
            .saturating_add(args.poll_interval)
            .saturating_sub(now()?);
        let remaining = total.saturating_sub(started.elapsed());
        if remaining.is_zero() || Duration::from_secs(wait) >= remaining {
            flow.set_stage("timed_out")?;
            return Err(flow.error(
                "timed_out",
                "等待已结束；可用 --resume 继续同次二维码，未完成登录",
            ));
        }
        if wait > 0 {
            std::thread::sleep(Duration::from_secs(wait));
        }
        match flow.poll()? {
            Poll::New => {}
            Poll::Scanned => {
                eprintln!("二维码已扫描，等待账号所有者确认。");
            }
            Poll::Confirmed => return flow.finish(timeout),
            Poll::Expired => {
                return Err(flow.error("expired", "服务器明确报告二维码已过期；未自动换码"));
            }
            Poll::Cancelled => {
                return Err(flow.error("cancelled", "服务器报告本次登录已取消"));
            }
            Poll::Stopped => {
                return Err(flow.error("qr_stopped", "服务器报告二维码已被拒绝或失效；未自动换码"));
            }
        }
        if args.poll_once {
            return Ok(flow.public_state(&flow.pending.stage));
        }
    }
}

impl PendingSession {
    fn new() -> Result<Self, String> {
        let key = SecretKey::random(&mut OsRng);
        let private_key_pem = key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|_| "creator_login: 无法编码本次 P-256 私钥")?
            .to_string();
        let public_key_base64 =
            STANDARD.encode(key.public_key().to_encoded_point(false).as_bytes());
        Ok(Self {
            schema: 1,
            origin: ORIGIN.into(),
            stage: "initializing".into(),
            created_at: now()?,
            last_poll_at: None,
            last_http_status: None,
            last_business_code: None,
            private_key_pem,
            public_key_base64,
            user_agent: UA.into(),
            cookies: Vec::new(),
            creator_csrf_token: None,
            server_data: None,
            challenge: None,
            redirect_url: None,
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.origin != ORIGIN || self.user_agent != UA {
            return Err("creator_login: pending 配置来源、版本或客户端不匹配".into());
        }
        validate_key(&self.private_key_pem, &self.public_key_base64)?;
        let current = now()?;
        if self.created_at > current.saturating_add(30)
            || current.saturating_sub(self.created_at) > LOCAL_RESUME_WINDOW
        {
            return Err(
                "creator_login: pending 超出本地 30 分钟继续窗口；此限制不代表服务器二维码过期"
                    .into(),
            );
        }
        if self.cookies.len() > 100
            || self.cookies.iter().any(|c| !c.valid_for_store())
            || self
                .creator_csrf_token
                .as_ref()
                .is_some_and(|v| !safe_value(v, 8192))
        {
            return Err("creator_login: pending Cookie 或 CSRF 格式无效（内容已隐藏）".into());
        }
        if let Some(challenge) = &self.challenge {
            if !safe_value(&challenge.token, 8192) {
                return Err("creator_login: pending 二维码 token 格式无效（内容已隐藏）".into());
            }
            validate_qr_url(&challenge.index_url)?;
        }
        if !matches!(
            self.stage.as_str(),
            "initializing"
                | "qr_ready"
                | "new"
                | "scanned"
                | "timed_out"
                | "transport_error"
                | "confirmed"
                | "identity_pending"
        ) {
            return Err(format!(
                "creator_login: 此 pending 已停止，stage={}；不会自动重试或换码",
                public_stage(&self.stage)
            ));
        }
        if self.challenge.is_none() && self.stage != "initializing" {
            return Err("creator_login: pending 缺少可继续的二维码，未重新取码".into());
        }
        Ok(())
    }
}

impl LoginFlow {
    fn new(directory: PathBuf, pending: PendingSession, timeout: Duration) -> Result<Self, String> {
        Self::build(
            directory,
            pending,
            timeout,
            Url::parse(ORIGIN).expect("fixed Creator URL"),
        )
    }

    fn build(
        directory: PathBuf,
        pending: PendingSession,
        timeout: Duration,
        origin: Url,
    ) -> Result<Self, String> {
        #[cfg(not(test))]
        if origin.as_str() != format!("{ORIGIN}/") {
            return Err("creator_login: 只允许固定 Creator HTTPS 同源".into());
        }
        let client = Client::builder()
            .timeout(timeout)
            .connect_timeout(timeout.min(Duration::from_secs(10)))
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .user_agent(&pending.user_agent)
            .build()
            .map_err(|_| "creator_login: 无法建立验证 TLS 的 HTTP 客户端")?;
        Ok(Self {
            client,
            origin,
            directory,
            pending,
        })
    }

    fn persist(&self) -> Result<(), String> {
        let value = serde_json::to_value(&self.pending)
            .map_err(|_| "creator_login: 无法编码私密 pending 材料")?;
        store::save_at(&self.directory.join(PENDING_FILE), &value)
            .map_err(|_| "creator_login: 无法安全保存本次私密 pending 材料".into())
    }

    fn set_stage(&mut self, stage: &str) -> Result<(), String> {
        self.pending.stage = stage.into();
        self.persist()
    }

    fn error(&self, stage: &str, detail: &str) -> String {
        format!(
            "creator_login stage={stage}: {detail}；pending={}",
            self.directory.display()
        )
    }

    fn public_state(&self, stage: &str) -> Value {
        let challenge = self.pending.challenge.as_ref();
        json!({
            "ok": true,
            "stage": stage,
            "httpStatus": self.pending.last_http_status,
            "businessCode": self.pending.last_business_code,
            "authenticated": false,
            "accountVerified": false,
            "publishReady": false,
            "active": false,
            "qrActivationImplemented": false,
            "pendingDir": self.directory,
            "resumeArguments": ["auth", "creator-login", "--resume", self.directory.to_string_lossy()],
            "qrUrl": challenge.map(|c| c.index_url.as_str()),
            "qrSvg": self.directory.join("qrcode.svg"),
            "serverExpireTime": challenge.and_then(|c| c.server_expire_time.as_ref()),
            "localResumeWindowSeconds": LOCAL_RESUME_WINDOW
        })
    }

    fn begin(&mut self) -> Result<(), String> {
        self.pending.validate()?;
        self.persist()?;
        let home = self.request(Method::GET, "/", &[], None, "bootstrap")?;
        self.require_http(&home, "bootstrap")?;
        let csrf = self.request(Method::HEAD, CREATOR_CSRF, &[], None, "creator_csrf")?;
        self.require_http(&csrf, "creator_csrf")?;
        // A missing Creator token is represented as missing; Passport CSRF is
        // a different cookie and is never substituted here.
        self.pending.creator_csrf_token = creator_csrf(&csrf.headers);
        self.persist()?;

        let mut params = sdk_params();
        params.extend([
            ("next".into(), NEXT.into()),
            ("need_short_url".into(), "true".into()),
            ("need_logo".into(), "false".into()),
        ]);
        let response = self.request(Method::GET, GET_QR, &params, None, "get_qrcode")?;
        let value = self.json_reply(&response, "get_qrcode")?;
        self.require_business(&value, "get_qrcode")?;
        self.pending.challenge = Some(match parse_challenge(&value["data"]) {
            Ok(challenge) => challenge,
            Err(error) => {
                self.set_stage("response_unknown")?;
                return Err(self.error("get_qrcode", &error));
            }
        });
        self.set_stage("qr_ready")?;
        self.write_qr_svg()?;
        Ok(())
    }

    fn show_qr(&self) -> Result<(), String> {
        let challenge = self
            .pending
            .challenge
            .as_ref()
            .ok_or_else(|| self.error("show_qrcode", "pending 缺少二维码"))?;
        let code = QrCode::new(challenge.index_url.as_bytes())
            .map_err(|_| self.error("show_qrcode", "无法渲染服务器二维码地址"))?;
        eprintln!(
            "{}",
            code.render::<qrcode::render::unicode::Dense1x2>().build()
        );
        eprintln!("本次登录二维码页面：{}", challenge.index_url);
        eprintln!(
            "pending={}；这是待登录状态，扫码后仍须通过服务器绑定和账号核验。",
            self.directory.display()
        );
        Ok(())
    }

    fn write_qr_svg(&self) -> Result<(), String> {
        let challenge = self
            .pending
            .challenge
            .as_ref()
            .ok_or_else(|| self.error("show_qrcode", "pending 缺少二维码"))?;
        let code = QrCode::new(challenge.index_url.as_bytes())
            .map_err(|_| self.error("show_qrcode", "无法渲染服务器二维码地址"))?;
        let width = code.width() + 8;
        let mut svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {width}" shape-rendering="crispEdges"><rect width="{width}" height="{width}" fill="white"/><path fill="black" d=""#
        );
        for y in 0..code.width() {
            for x in 0..code.width() {
                if code[(x, y)] == Color::Dark {
                    svg.push_str(&format!("M{},{}h1v1h-1z", x + 4, y + 4));
                }
            }
        }
        svg.push_str("\"/></svg>\n");
        save_private_bytes(&self.directory.join("qrcode.svg"), svg.as_bytes())
    }

    fn poll(&mut self) -> Result<Poll, String> {
        let challenge = self
            .pending
            .challenge
            .as_ref()
            .ok_or_else(|| self.error("poll", "pending 缺少二维码"))?;
        if !matches!(
            self.pending.stage.as_str(),
            "qr_ready" | "new" | "scanned" | "timed_out" | "transport_error"
        ) {
            return Err(self.error("poll", "本次登录已停止；禁止重复请求"));
        }
        let form = vec![
            ("need_logo".into(), "false".into()),
            // The UI disables the WebSocket, while its QR polling hook
            // still explicitly sends is_frontier:true. These are distinct.
            ("is_frontier".into(), "true".into()),
            ("token".into(), challenge.token.clone()),
            ("is_new_login".into(), "1".into()),
            ("next".into(), NEXT.into()),
            ("need_short_url".into(), "true".into()),
        ];
        self.pending.last_poll_at = Some(now()?);
        self.persist()?;
        let response = self.request(Method::POST, CHECK_QR, &sdk_params(), Some(&form), "poll")?;
        let value = self.json_reply(&response, "poll")?;
        self.require_business(&value, "poll")?;
        let state = match value["data"]["status"].as_str() {
            Some("1" | "new") => Poll::New,
            Some("2" | "scanned") => Poll::Scanned,
            Some("3" | "confirmed") => Poll::Confirmed,
            Some("expired") => Poll::Expired,
            Some("cancelled" | "canceled") => Poll::Cancelled,
            Some("4" | "5" | "refused") => Poll::Stopped,
            _ => {
                self.set_stage("response_unknown")?;
                return Err(self.error("poll", "未识别的二维码状态；未将响应视为登录成功"));
            }
        };
        let stage = match state {
            Poll::New => "new",
            Poll::Scanned => "scanned",
            Poll::Confirmed => "confirmed",
            Poll::Expired => "expired",
            Poll::Cancelled => "cancelled",
            Poll::Stopped => "qr_stopped",
        };
        if state == Poll::Confirmed {
            self.pending.redirect_url = value["data"]["redirect_url"]
                .as_str()
                .filter(|v| !v.is_empty())
                .map(str::to_owned);
            if let Some(url) = &self.pending.redirect_url
                && let Err(error) = checked_redirect(&self.origin, url)
            {
                self.set_stage("redirect_blocked")?;
                return Err(error);
            }
        }
        self.set_stage(stage)?;
        Ok(state)
    }

    fn finish(&mut self, timeout: Duration) -> Result<Value, String> {
        if !matches!(
            self.pending.stage.as_str(),
            "confirmed" | "identity_pending"
        ) {
            return Err(self.error("finalize", "尚未收到服务器明确确认状态"));
        }
        if let Some(redirect) = self.pending.redirect_url.clone() {
            if let Err(error) = self.follow_redirects(&redirect) {
                self.set_stage("redirect_blocked")?;
                return Err(error);
            }
            self.pending.redirect_url = None;
            self.persist()?;
        }
        let server_data = match self.pending.server_data.as_ref() {
            Some(data) => data.clone(),
            None => {
                self.set_stage("confirmed_unverified")?;
                return Err(self.error(
                    "confirmed_unverified",
                    "服务器已确认扫码，但没有返回可核验的本次公钥票据；未激活登录",
                ));
            }
        };
        if let Err(error) = validate_server_binding(&server_data, &self.pending.public_key_base64) {
            self.set_stage(if error.contains("key_mismatch") {
                "key_mismatch"
            } else {
                "confirmed_unverified"
            })?;
            return Err(error);
        }
        self.set_stage("identity_pending")?;
        let account_url = self
            .origin
            .join("/web/api/media/user/info/")
            .map_err(|_| "creator_login: 固定账号地址无效")?;
        let cookie = cookie_header(&self.pending.cookies, &account_url, now()?);
        let ms_token =
            cookie_value(&self.pending.cookies, &account_url, "msToken", now()?).map(str::to_owned);
        let material = NativeLoginMaterial {
            cookie,
            private_key_pem: self.pending.private_key_pem.clone(),
            public_key_base64: self.pending.public_key_base64.clone(),
            server_data,
            csrf_token: self.pending.creator_csrf_token.clone(),
            ms_token,
            user_agent: self.pending.user_agent.clone(),
            pending_dir: self.directory.clone(),
        };
        crate::creator_login_finish::finalize_native_login(material, timeout)
    }

    fn follow_redirects(&mut self, initial: &str) -> Result<(), String> {
        let mut url = checked_redirect(&self.origin, initial)?;
        for _ in 0..4 {
            let response = self.request_url(Method::GET, url.clone(), None, "login_redirect")?;
            if (300..400).contains(&response.status) {
                let next = response
                    .headers
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| self.error("login_redirect", "重定向缺少 Location；已停止"))?;
                let candidate = url
                    .join(next)
                    .map_err(|_| self.error("login_redirect", "重定向地址无效；已停止"))?;
                url = checked_redirect(&self.origin, candidate.as_str())?;
            } else {
                self.require_http(&response, "login_redirect")?;
                return Ok(());
            }
        }
        Err(self.error("login_redirect", "超过 4 次同源重定向；已停止"))
    }

    fn request(
        &mut self,
        method: Method,
        path: &str,
        query: &[(String, String)],
        form: Option<&[(String, String)]>,
        stage: &str,
    ) -> Result<Reply, String> {
        let mut url = self
            .origin
            .join(path)
            .map_err(|_| self.error(stage, "固定请求地址无效"))?;
        if !query.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(query.iter().map(|(k, v)| (k, v)));
        }
        self.request_url(method, url, form, stage)
    }

    fn request_url(
        &mut self,
        method: Method,
        mut url: Url,
        form: Option<&[(String, String)]>,
        stage: &str,
    ) -> Result<Reply, String> {
        checked_redirect(&self.origin, url.as_str())?;
        let passport = matches!(url.path(), GET_QR | CHECK_QR);
        let aid_signature = if passport {
            let query: Vec<_> = url
                .query_pairs()
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect();
            let wire = passport_wire(url.path(), &query, form.unwrap_or(&[]), now_millis()?)?;
            url.set_query(None);
            url.query_pairs_mut().extend_pairs(&wire.query);
            Some(wire.aid_signature)
        } else {
            None
        };
        let current = now()?;
        let cookie = cookie_header(&self.pending.cookies, &url, current);
        let mut request = self
            .client
            .request(method.clone(), url.clone())
            .header(
                "accept",
                if passport {
                    "application/json, text/javascript"
                } else {
                    "application/json, text/html;q=0.9"
                },
            )
            .header("origin", ORIGIN)
            .header("referer", NEXT);
        if !cookie.is_empty() {
            request = request.header("cookie", sensitive_header(&cookie)?);
        }
        if let Some(signature) = aid_signature {
            request = request
                .header("x-tt-passport-aid-sign", sensitive_header(&signature)?)
                .header("content-type", "application/x-www-form-urlencoded");
            let csrf = cookie_value(&self.pending.cookies, &url, "passport_csrf_token", current)
                .or_else(|| {
                    cookie_value(
                        &self.pending.cookies,
                        &url,
                        "passport_csrf_token_default",
                        current,
                    )
                });
            if let Some(csrf) = csrf {
                request = request.header("x-tt-passport-csrf-token", sensitive_header(csrf)?);
            }
        }
        if method == Method::HEAD && url.path() == CREATOR_CSRF {
            request = request
                .header("x-secsdk-csrf-request", "1")
                .header("x-secsdk-csrf-version", "1.2.22");
        }
        if method == Method::POST && url.path() == CHECK_QR {
            request = request
                .header("web-sdk-version", "1")
                .header(
                    "bd-ticket-guard-ree-public-key",
                    sensitive_header(&self.pending.public_key_base64)?,
                )
                .header("bd-ticket-guard-version", "2")
                .header("bd-ticket-guard-web-version", "2")
                .header("bd-ticket-guard-web-sign-type", "0");
        }
        if let Some(form) = form {
            request = request.form(form);
        }
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => {
                self.set_stage("transport_error")?;
                let detail = if error.is_timeout() {
                    "HTTP 请求超时；响应未知，未自动重试"
                } else {
                    "HTTP 请求失败；响应未知，未自动重试"
                };
                return Err(self.error(stage, detail));
            }
        };
        let status = response.status().as_u16();
        self.pending.last_http_status = Some(status);
        self.pending.last_business_code = None;
        let headers = response.headers().clone();
        if let Err(error) = absorb_cookies(&mut self.pending.cookies, &url, &headers, current) {
            self.set_stage("response_unknown")?;
            return Err(error);
        }
        let captured = capture_server_data(&headers, &self.pending.cookies, &url, current);
        if let Ok(Some(data)) = &captured {
            self.pending.server_data = Some(data.clone());
        }
        self.persist()?;
        if let Err(error) = captured {
            self.set_stage("response_unknown")?;
            return Err(error);
        }
        let mut bytes = Vec::new();
        if response
            .take(MAX_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .is_err()
        {
            self.set_stage("response_unknown")?;
            return Err(self.error(stage, "无法读取 HTTP 响应；内容已隐藏，已停止"));
        }
        if bytes.len() as u64 > MAX_RESPONSE {
            self.set_stage("response_unknown")?;
            return Err(self.error(stage, "HTTP 响应超过 2 MiB，已停止"));
        }
        Ok(Reply {
            status,
            headers,
            bytes,
        })
    }

    fn require_http(&mut self, response: &Reply, stage: &str) -> Result<(), String> {
        if response
            .headers
            .contains_key("x-tt-verify-passport-decision")
        {
            self.set_stage("verification_required")?;
            return Err(self.error(
                "verification_required",
                "服务器返回验证决策头；已停止，未尝试绕过",
            ));
        }
        if !(200..300).contains(&response.status) {
            self.set_stage("http_error")?;
            return Err(self.error(stage, &format!("HTTP {}；未自动重试", response.status)));
        }
        Ok(())
    }

    fn json_reply(&mut self, response: &Reply, stage: &str) -> Result<Value, String> {
        self.require_http(response, stage)?;
        match serde_json::from_slice(&response.bytes) {
            Ok(value) => Ok(value),
            Err(_) => {
                self.set_stage("response_unknown")?;
                Err(self.error(stage, "服务器响应不是已知 JSON；内容已隐藏，已停止"))
            }
        }
    }

    fn require_business(&mut self, value: &Value, stage: &str) -> Result<(), String> {
        let code = value["data"]["error_code"].as_i64();
        self.pending.last_business_code = code;
        if code == Some(4031) {
            self.set_stage("server_blocked")?;
            return Err(self.error(
                "server_blocked",
                &format!(
                    "HTTP {}，error_code=4031，服务器阻止访问此登录网站；响应未给出可解验证挑战，未自动换码或重试",
                    self.pending.last_http_status.map_or_else(|| "unknown".into(), |s| s.to_string())
                ),
            ));
        }
        if code != Some(0) || value["message"].as_str() != Some("success") {
            self.set_stage("server_error")?;
            let detail = match code {
                Some(code) => format!("服务器 error_code={code}；未自动重试，响应内容已隐藏"),
                None => "服务器缺少明确成功字段；未继续，响应内容已隐藏".into(),
            };
            return Err(self.error(stage, &detail));
        }
        Ok(())
    }
}

pub(crate) fn capabilities() -> Value {
    json!({
        "ok": true, "readOnly": true, "networkAccess": false,
        "qrActivationImplemented": false,
        "qrResult": "pending_identity_material_only",
        "missingImplementation": ["Creator publish material collection", "verified active-session activation"],
        "routes": [
            {"command":"auth creator-import FILE", "requires":"legally obtained complete bundle from the same Creator login", "verification":"local binding check, then publish --dry-run verifies live account", "onlineValidated":false},
            {"command":"auth login --scope user_info,video.create.bind", "requires":"approved OpenAPI application and actual user-granted scope", "publishCommand":"publish video", "separateFromCreatorSession":true}
        ],
        "aiDeclarationParameterVerified": false,
        "nextAction": "缺少完整 Creator 会话包时，使用具备审批权限的官方 OAuth 路线，或由账号所有者在正常 Creator 界面操作；CLI 不能把普通 Cookie 或重复扫码变成完整发布凭据"
    })
}

fn sdk_params() -> Vec<(String, String)> {
    [
        ("aid", "2906"),
        ("is_from_iesaccountsaas", "1"),
        ("is_from_ttaccountsdk", "1"),
        ("is_new_login", "1"),
        ("p_ui", UI_VERSION),
        ("passport_jssdk_version", PASSPORT_VERSION),
        ("passport_jssdk_type", "normal"),
        ("account_sdk_source", "web"),
        ("language", "zh"),
        ("p_js_v", PASSPORT_VERSION),
        ("p_js_t", "pro"),
        // Optional SDKs are absent in the native client. The official
        // interceptor uses 0 when absent; no browser values are copied.
        ("p_zt", "0"),
        ("p_ver", "0"),
        ("p_ver_real", "0"),
        ("p_bd", "0"),
        ("request_host", "https%3A%2F%2Fcreator.douyin.com"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

struct PassportWire {
    query: Vec<(String, String)>,
    aid_signature: String,
}

/// The public application parameter does not prove user identity. That
/// requires a confirmed, server-bound ticket and the account checks.
fn passport_wire(
    path: &str,
    query: &[(String, String)],
    form: &[(String, String)],
    timestamp_ms: u64,
) -> Result<PassportWire, String> {
    if !matches!(path, GET_QR | CHECK_QR) {
        return Err("creator_login: 不为未知 Passport 路径生成签名".into());
    }
    let mut fields = BTreeMap::new();
    for (key, value) in query {
        if matches!(key.as_str(), "sign" | "qs" | "ts" | "p_ts" | "p_no")
            || fields.insert(key.clone(), value.clone()).is_some()
        {
            return Err("creator_login: Passport query 包含重复或外部签名字段".into());
        }
    }
    let (timestamp, aid_signature) = aid_signature(path, timestamp_ms)?;
    fields.insert("ts".into(), timestamp);
    fields.insert("p_ts".into(), timestamp_ms.to_string());
    let p_no = format!(
        "p_bd=0&p_ca=0&p_ts={timestamp_ms}&p_ver=0&p_zt=0&passport_jssdk_version={PASSPORT_VERSION}"
    );
    fields.insert(
        "p_no".into(),
        hex(digest::digest(&digest::SHA256, p_no.as_bytes()).as_ref()),
    );

    // Public SDK: sorted query keys, first 10; all form keys sorted. Values
    // are canonical strings before percent encoding, including true/false.
    let keys: Vec<_> = fields.keys().take(10).cloned().collect();
    let query_text = keys
        .iter()
        .map(|k| format!("{k}={}", fields[k]))
        .collect::<Vec<_>>()
        .join("&");
    let mut body = BTreeMap::new();
    for (key, value) in form {
        if body.insert(key, value).is_some() {
            return Err("creator_login: Passport form 包含重复字段".into());
        }
    }
    let form_text = body
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let preimage = format!("{query_text}&{form_text}&app_key={PUBLIC_APP_KEY}");
    fields.insert(
        "sign".into(),
        hex(digest::digest(&digest::SHA256, preimage.as_bytes()).as_ref()),
    );
    fields.insert(
        "qs".into(),
        keys.join(",")
            .bytes()
            .map(|b| format!("{:x}", b ^ 5))
            .collect(),
    );
    Ok(PassportWire {
        query: fields.into_iter().collect(),
        aid_signature,
    })
}

/// HKDF-SHA256(salt=UTC noon text, IKM=public app-key, info=empty, len=32),
/// then HMAC-SHA256 of aid/path/ts. One expand block suffices for 32 bytes.
fn aid_signature(path: &str, timestamp_ms: u64) -> Result<(String, String), String> {
    if !matches!(path, GET_QR | CHECK_QR) {
        return Err("creator_login: aid-sign 路径无效".into());
    }
    let noon = timestamp_ms / 1000 / 86400 * 86400 + 43200;
    let timestamp = noon.to_string();
    let salt = hmac::Key::new(hmac::HMAC_SHA256, timestamp.as_bytes());
    let extracted = hmac::sign(&salt, PUBLIC_APP_KEY.as_bytes());
    let key = hmac::Key::new(hmac::HMAC_SHA256, extracted.as_ref());
    let expanded = hmac::sign(&key, &[1]);
    let key = hmac::Key::new(hmac::HMAC_SHA256, expanded.as_ref());
    let message = format!("aid=2906&path={path}&ts={timestamp}");
    Ok((
        timestamp,
        hex(hmac::sign(&key, message.as_bytes()).as_ref()),
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_millis() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "creator_login: 系统时钟早于 Unix epoch")?
        .as_millis();
    u64::try_from(millis).map_err(|_| "creator_login: 系统时钟超出范围".into())
}

fn parse_challenge(data: &Value) -> Result<Challenge, String> {
    let token = data["token"]
        .as_str()
        .filter(|v| safe_value(v, 8192))
        .ok_or("服务器未返回有效二维码 token（内容已隐藏）")?;
    let index_url = data["qrcode_index_url"]
        .as_str()
        .ok_or("服务器未返回二维码页面地址")?;
    validate_qr_url(index_url)?;
    let is_frontier = match &data["is_frontier"] {
        Value::Bool(v) => *v,
        Value::Null => false,
        _ => return Err("服务器 is_frontier 不是已知布尔契约".into()),
    };
    Ok(Challenge {
        token: token.into(),
        index_url: index_url.into(),
        is_frontier,
        server_expire_time: data.get("expire_time").filter(|v| v.is_number()).cloned(),
        received_at: now()?,
    })
}

fn creator_csrf(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-ware-csrf-token")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').nth(1))
        .map(str::trim)
        .filter(|v| safe_value(v, 8192))
        .map(str::to_owned)
}

fn validate_key(private: &str, public: &str) -> Result<(), String> {
    if private.len() > 8192 || public.len() > 256 {
        return Err("creator_login: 本次 P-256 材料长度无效".into());
    }
    let key = SecretKey::from_pkcs8_pem(private)
        .map_err(|_| "creator_login: pending 私钥不是合法 P-256 PKCS8")?;
    let supplied = STANDARD
        .decode(public)
        .map_err(|_| "creator_login: pending 公钥编码无效")?;
    if supplied != key.public_key().to_encoded_point(false).as_bytes() {
        return Err("creator_login: pending 私钥与公钥不匹配".into());
    }
    Ok(())
}

fn validate_server_binding(data: &Value, public: &str) -> Result<(), String> {
    for key in ["ticket", "ts_sign"] {
        if !data[key].as_str().is_some_and(|v| safe_value(v, 8192)) {
            return Err(format!(
                "creator_login stage=confirmed_unverified: 服务器缺少 {key}，未激活登录"
            ));
        }
    }
    let bound = data["client_cert"]
        .as_str()
        .and_then(|v| v.strip_prefix("pub."))
        .and_then(|v| STANDARD.decode(v).ok())
        .ok_or("creator_login stage=confirmed_unverified: 缺少可核验的本机 client_cert")?;
    let expected = STANDARD
        .decode(public)
        .map_err(|_| "creator_login: 本次公钥编码无效")?;
    if bound != expected {
        return Err(
            "creator_login stage=key_mismatch: 服务器票据没有绑定本次公钥；拒绝混用登录材料".into(),
        );
    }
    Ok(())
}

fn capture_server_data(
    headers: &HeaderMap,
    cookies: &[ScopedCookie],
    url: &Url,
    at: u64,
) -> Result<Option<Value>, String> {
    let encoded = headers
        .get("bd-ticket-guard-server-data")
        .and_then(|v| v.to_str().ok())
        .or_else(|| cookie_value(cookies, url, "bd_ticket_guard_server_data", at));
    let Some(encoded) = encoded else {
        return Ok(None);
    };
    if encoded.len() > 64 * 1024 {
        return Err("creator_login: 服务器票据超过长度限制（内容已隐藏）".into());
    }
    let decoded = percent_encoding::percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|_| "creator_login: 服务器票据 URL 编码无效（内容已隐藏）")?;
    let bytes = STANDARD
        .decode(decoded.as_bytes())
        .map_err(|_| "creator_login: 服务器票据 base64 编码无效（内容已隐藏）")?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "creator_login: 服务器票据不是 JSON（内容已隐藏）")?;
    if !value.is_object() {
        return Err("creator_login: 服务器票据不是对象（内容已隐藏）".into());
    }
    Ok(Some(value))
}

fn checked_redirect(origin: &Url, candidate: &str) -> Result<Url, String> {
    let url = origin
        .join(candidate)
        .map_err(|_| "creator_login: 登录重定向 URL 无效（地址已隐藏）")?;
    if url.origin() != origin.origin()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("creator_login: 登录重定向越过已验证 Creator 同源边界；未发送凭据".into());
    }
    Ok(url)
}

fn validate_qr_url(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|_| "creator_login: 二维码页面地址无效")?;
    if value.len() > 8192
        || url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.host_str() != Some("api.amemv.com")
        || url.path()
            != "/ucenter_web/app/aweme/scan_login/index/douyin_scan_code_login/cn/app/index.html"
        || !url
            .query_pairs()
            .any(|(k, v)| k == "token" && !v.is_empty())
    {
        return Err("creator_login: 未识别的服务器二维码页面契约；未展示或访问地址".into());
    }
    Ok(())
}

fn sensitive_header(value: &str) -> Result<HeaderValue, String> {
    let mut header =
        HeaderValue::from_str(value).map_err(|_| "creator_login: 请求头格式无效（内容已隐藏）")?;
    header.set_sensitive(true);
    Ok(header)
}

fn safe_value(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && !value.bytes().any(|b| b.is_ascii_control() || b == b';')
}

fn cookie_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

impl ScopedCookie {
    fn valid_for_store(&self) -> bool {
        cookie_name(&self.name)
            && safe_value(&self.value, 32 * 1024)
            && matches!(self.domain.as_str(), "creator.douyin.com" | "douyin.com")
            && self.path.starts_with('/')
            && self.path.len() <= 4096
            && !self.path.bytes().any(|b| b.is_ascii_control())
    }

    fn applies(&self, url: &Url, at: u64) -> bool {
        let host = url.host_str().unwrap_or_default();
        let domain_matches = if self.host_only {
            host == self.domain
        } else {
            host == self.domain || host.ends_with(&format!(".{}", self.domain))
        };
        let request_path = url.path();
        let path_matches = request_path == self.path
            || (request_path.starts_with(&self.path)
                && (self.path.ends_with('/')
                    || request_path.as_bytes().get(self.path.len()) == Some(&b'/')));
        domain_matches
            && path_matches
            && (!self.secure || url.scheme() == "https")
            && self.expires_at.is_none_or(|v| v > at)
    }
}

fn cookie_header(cookies: &[ScopedCookie], url: &Url, at: u64) -> String {
    let mut selected: Vec<_> = cookies.iter().filter(|c| c.applies(url, at)).collect();
    selected.sort_by_key(|c| std::cmp::Reverse(c.path.len()));
    selected
        .into_iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

fn cookie_value<'a>(
    cookies: &'a [ScopedCookie],
    url: &Url,
    name: &str,
    at: u64,
) -> Option<&'a str> {
    cookies
        .iter()
        .filter(|c| c.name == name && c.applies(url, at))
        .max_by_key(|c| c.path.len())
        .map(|c| c.value.as_str())
}

fn absorb_cookies(
    cookies: &mut Vec<ScopedCookie>,
    url: &Url,
    headers: &HeaderMap,
    at: u64,
) -> Result<(), String> {
    for raw in headers.get_all(SET_COOKIE) {
        let raw = raw
            .to_str()
            .map_err(|_| "creator_login: Set-Cookie 非法（内容已隐藏）")?;
        let mut parts = raw.split(';');
        let Some((name, value)) = parts.next().and_then(|v| v.trim().split_once('=')) else {
            return Err("creator_login: Set-Cookie 缺少名称（内容已隐藏）".into());
        };
        if !cookie_name(name) || (!value.is_empty() && !safe_value(value, 32 * 1024)) {
            return Err("creator_login: Set-Cookie 格式无效（内容已隐藏）".into());
        }
        let host = url.host_str().unwrap_or_default();
        let default_path = url
            .path()
            .rsplit_once('/')
            .map_or("/", |(v, _)| if v.is_empty() { "/" } else { v });
        let mut cookie = ScopedCookie {
            name: name.into(),
            value: value.into(),
            domain: host.into(),
            path: default_path.into(),
            host_only: true,
            secure: false,
            expires_at: None,
        };
        let mut max_age = None;
        for attribute in parts {
            let (key, value) = attribute
                .trim()
                .split_once('=')
                .unwrap_or((attribute.trim(), ""));
            match key.to_ascii_lowercase().as_str() {
                "domain" => {
                    let domain = value.trim().trim_start_matches('.').to_ascii_lowercase();
                    if !matches!(domain.as_str(), "creator.douyin.com" | "douyin.com")
                        || !(host == domain || host.ends_with(&format!(".{domain}")))
                    {
                        return Err("creator_login: 服务器 Cookie 指向非 Creator 域，已停止".into());
                    }
                    cookie.domain = domain;
                    cookie.host_only = false;
                }
                "path" if value.starts_with('/') => cookie.path = value.into(),
                "secure" => cookie.secure = true,
                "max-age" => max_age = value.parse::<i64>().ok(),
                "expires" => cookie.expires_at = cookie_date(value),
                _ => {}
            }
        }
        if let Some(age) = max_age {
            cookie.expires_at = Some(if age <= 0 {
                0
            } else {
                at.saturating_add(age as u64)
            });
        }
        cookies.retain(|c| {
            !(c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path)
        });
        if !cookie.value.is_empty() && cookie.expires_at.is_none_or(|v| v > at) {
            if cookie.path.len() > 4096 || cookie.path.bytes().any(|b| b.is_ascii_control()) {
                return Err("creator_login: Cookie Path 无效，已停止".into());
            }
            cookies.push(cookie);
        }
        if cookies.len() > 100 {
            return Err("creator_login: Cookie 数量超过限制，已停止".into());
        }
    }
    Ok(())
}

/// Parse the standard IMF-fixdate used by these HTTP Set-Cookie responses.
/// Unrecognized date formats are session cookies; Max-Age still takes precedence.
fn cookie_date(value: &str) -> Option<u64> {
    let parts: Vec<_> = value.split_ascii_whitespace().collect();
    if parts.len() != 6 || parts[5] != "GMT" {
        return None;
    }
    let day = parts[1].parse::<i64>().ok()?;
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == parts[2])? as i64
        + 1;
    let mut year = parts[3].parse::<i64>().ok()?;
    let time: Vec<_> = parts[4].split(':').collect();
    if time.len() != 3 || !(1601..=9999).contains(&year) || !(1..=31).contains(&day) {
        return None;
    }
    let hour = time[0].parse::<i64>().ok()?;
    let minute = time[1].parse::<i64>().ok()?;
    let second = time[2].parse::<i64>().ok()?;
    if !(0..24).contains(&hour) || !(0..60).contains(&minute) || !(0..60).contains(&second) {
        return None;
    }
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * shifted_month + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    Some((days * 86400 + hour * 3600 + minute * 60 + second).max(0) as u64)
}

fn new_pending(base: &Path) -> Result<(PathBuf, PendingSession, PendingLock), String> {
    fs::create_dir_all(base).map_err(|_| "creator_login: 无法创建 pending 目录")?;
    let directory = tempfile::Builder::new()
        .prefix("native-")
        .tempdir_in(base)
        .map_err(|_| "creator_login: 无法创建独立 pending 目录")?
        .keep();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "creator_login: 无法设置私密目录权限")?;
    }
    let guard = PendingLock::acquire(&directory)?;
    let pending = PendingSession::new()?;
    let document = serde_json::to_value(&pending).map_err(|_| "creator_login: 无法编码 pending")?;
    store::save_at(&directory.join(PENDING_FILE), &document)
        .map_err(|_| "creator_login: 无法安全保存本次密钥")?;
    Ok((directory, pending, guard))
}

struct PendingLock(File);

impl PendingLock {
    fn acquire(directory: &Path) -> Result<Self, String> {
        let metadata =
            fs::symlink_metadata(directory).map_err(|_| "creator_login: 无法读取 pending 目录")?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("creator_login: pending 必须是普通目录".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err("creator_login: pending 目录权限允许其他用户读取；拒绝加载".into());
            }
        }
        let path = directory.join("login.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err("creator_login: pending 锁不是普通文件".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("creator_login: 无法检查 pending 锁".into()),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(path)
            .map_err(|_| "creator_login: 无法打开 pending 锁")?;
        file.try_lock()
            .map_err(|_| "creator_login: 另一个进程正在处理此登录会话；未发出请求")?;
        let guard = Self(file);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            guard
                .0
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|_| "creator_login: 无法保护 pending 锁权限")?;
        }
        Ok(guard)
    }
}

impl Drop for PendingLock {
    fn drop(&mut self) {
        // Explicitly unlock even if a concurrently spawned child inherited
        // a descriptor. Never unlink or replace the lock inode.
        let _ = self.0.unlock();
    }
}

fn resume_pending(directory: &Path) -> Result<(PendingSession, PendingLock), String> {
    let guard = PendingLock::acquire(directory)?;
    let pending = load_pending(directory)?;
    Ok((pending, guard))
}

fn load_pending(directory: &Path) -> Result<PendingSession, String> {
    let metadata =
        fs::symlink_metadata(directory).map_err(|_| "creator_login: 无法读取 pending 目录")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("creator_login: pending 必须是普通目录".into());
    }
    let path = directory.join(PENDING_FILE);
    let metadata = fs::symlink_metadata(&path).map_err(|_| "creator_login: pending 文件不存在")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_PENDING {
        return Err("creator_login: pending 文件类型或大小无效".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0
            || fs::metadata(directory)
                .map_err(|_| "creator_login: 无法检查目录权限")?
                .permissions()
                .mode()
                & 0o077
                != 0
        {
            return Err("creator_login: pending 文件/目录权限允许其他用户读取；拒绝加载".into());
        }
    }
    let mut bytes = Vec::new();
    File::open(&path)
        .map_err(|_| "creator_login: 无法打开 pending 文件")?
        .take(MAX_PENDING + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "creator_login: 无法读取 pending 文件")?;
    if bytes.len() as u64 > MAX_PENDING {
        return Err("creator_login: pending 文件超过限制".into());
    }
    let pending: PendingSession = serde_json::from_slice(&bytes)
        .map_err(|_| "creator_login: pending JSON 格式无效（内容已隐藏）")?;
    pending.validate()?;
    Ok(pending)
}

fn save_private_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("creator_login: 缺少二维码目录")?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "creator_login: 无法创建二维码文件")?;
    file.write_all(bytes)
        .map_err(|_| "creator_login: 无法写入二维码文件")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "creator_login: 无法持久化二维码文件")?;
    file.persist(path)
        .map_err(|_| "creator_login: 无法保存二维码文件")?;
    Ok(())
}

fn public_stage(stage: &str) -> &str {
    match stage {
        "server_blocked"
        | "verification_required"
        | "expired"
        | "cancelled"
        | "response_unknown"
        | "server_error"
        | "http_error"
        | "confirmed_unverified"
        | "key_mismatch"
        | "redirect_blocked"
        | "qr_stopped" => stage,
        _ => "invalid",
    }
}

fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "creator_login: 系统时钟早于 Unix epoch".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};
    use std::thread::{self, JoinHandle};

    const QR_URL: &str = "https://api.amemv.com/ucenter_web/app/aweme/scan_login/index/douyin_scan_code_login/cn/app/index.html?token=fixture-login-token&next_url=https%3A%2F%2Fcreator.douyin.com%2F";

    struct ExpectedReply {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl ExpectedReply {
        fn json(value: Value) -> Self {
            Self {
                status: 200,
                headers: Vec::new(),
                body: value.to_string(),
            }
        }
    }

    struct RecordedRequest {
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: String,
    }

    impl RecordedRequest {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    fn read_request(stream: &mut TcpStream) -> RecordedRequest {
        // macOS inherits nonblocking mode from the listener; Linux does not.
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            let mut byte = [0u8; 1];
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
            assert!(head.len() < 64 * 1024);
        }
        let text = String::from_utf8(head).unwrap();
        let mut lines = text.split("\r\n");
        let first: Vec<_> = lines.next().unwrap().split_whitespace().collect();
        let headers: Vec<_> = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().into()))
            .collect();
        let length = headers
            .iter()
            .find(|(k, _)| k == "content-length")
            .map(|(_, v): &(String, String)| v.parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(length < 64 * 1024);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        RecordedRequest {
            method: first[0].into(),
            path: first[1].into(),
            headers,
            body: String::from_utf8(body).unwrap(),
        }
    }

    fn server(replies: Vec<ExpectedReply>) -> (Url, JoinHandle<Vec<RecordedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let started = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                started.elapsed() < Duration::from_secs(5),
                                "fixture request timed out"
                            );
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                };
                let request = read_request(&mut stream);
                let mut response = format!(
                    "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len(),
                );
                for (name, value) in reply.headers {
                    response.push_str(&format!("{name}: {value}\r\n"));
                }
                response.push_str("\r\n");
                stream.write_all(response.as_bytes()).unwrap();
                if request.method != "HEAD" {
                    stream.write_all(reply.body.as_bytes()).unwrap();
                }
                requests.push(request);
            }
            requests
        });
        (origin, handle)
    }

    fn test_flow(origin: Url) -> (tempfile::TempDir, LoginFlow) {
        let temp = tempfile::tempdir().unwrap();
        let (directory, pending, _guard) = new_pending(temp.path()).unwrap();
        let flow = LoginFlow::build(directory, pending, Duration::from_secs(3), origin).unwrap();
        (temp, flow)
    }

    fn ready(flow: &mut LoginFlow) {
        flow.pending.challenge = Some(Challenge {
            token: "fixture-login-token".into(),
            index_url: QR_URL.into(),
            is_frontier: false,
            server_expire_time: Some(json!(180)),
            received_at: now().unwrap(),
        });
        flow.set_stage("qr_ready").unwrap();
    }

    #[test]
    fn fresh_flow_preserves_callback_and_form_and_keeps_pending_private() {
        let replies = vec![
            ExpectedReply {
                status: 200,
                headers: vec![("Set-Cookie".into(), "gd_random=fixture-only; Path=/".into())],
                body: String::new(),
            },
            ExpectedReply {
                status: 200,
                headers: vec![
                    (
                        "X-Ware-Csrf-Token".into(),
                        "0,creator-csrf-fixture,0".into(),
                    ),
                    (
                        "Set-Cookie".into(),
                        "csrf_session_id=creator-session-fixture; Path=/".into(),
                    ),
                ],
                body: String::new(),
            },
            ExpectedReply {
                status: 200,
                headers: vec![(
                    "Set-Cookie".into(),
                    "passport_csrf_token=passport-csrf-fixture; Path=/".into(),
                )],
                body: json!({
                    "message":"success",
                    "data":{"error_code":0,"token":"fixture-login-token","qrcode_index_url":QR_URL,
                            "is_frontier":false,"expire_time":180}
                })
                .to_string(),
            },
            ExpectedReply::json(
                json!({"message":"success","data":{"error_code":0,"status":"new"}}),
            ),
        ];
        let (origin, handle) = server(replies);
        let (_temp, mut flow) = test_flow(origin);
        flow.begin().unwrap();
        assert_eq!(
            flow.pending.creator_csrf_token.as_deref(),
            Some("creator-csrf-fixture")
        );
        assert_eq!(flow.poll().unwrap(), Poll::New);
        let public = flow.public_state("new");
        assert_eq!(public["authenticated"], false);
        assert_eq!(public["publishReady"], false);
        assert!(!public.to_string().contains("PRIVATE KEY"));
        assert!(!public.to_string().contains("passport-csrf-fixture"));
        let svg = fs::read_to_string(flow.directory.join("qrcode.svg")).unwrap();
        assert!(svg.starts_with("<svg"));
        assert!(!svg.contains(QR_URL)); // Only the QR modules, no URL injection.
        let requests = handle.join().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[1].method, "HEAD");
        assert_eq!(requests[1].header("x-secsdk-csrf-request"), Some("1"));
        let qr_url =
            Url::parse(&format!("https://creator.douyin.com{}", requests[2].path)).unwrap();
        let get_pairs: Vec<_> = qr_url.query_pairs().collect();
        let form: Vec<_> = url::form_urlencoded::parse(requests[3].body.as_bytes()).collect();
        assert!(get_pairs.iter().any(|(k, v)| k == "next" && v == NEXT));
        assert!(form.iter().any(|(k, v)| k == "next" && v == NEXT));
        assert!(
            form.iter()
                .any(|(k, v)| k == "token" && v == "fixture-login-token")
        );
        assert!(!requests[3].path.contains("fixture-login-token"));
        assert_eq!(requests[3].method, "POST");
        assert_eq!(
            requests[3]
                .headers
                .iter()
                .filter(|(k, _)| k == "x-tt-passport-csrf-token")
                .count(),
            1
        );
        assert_eq!(
            requests[3]
                .headers
                .iter()
                .filter(|(k, _)| k == "accept")
                .count(),
            1
        );
        assert!(form.iter().any(|(k, v)| k == "is_frontier" && v == "true"));
        for request in &requests[2..] {
            assert_eq!(request.header("x-tt-passport-aid-sign").unwrap().len(), 64);
            let parsed =
                Url::parse(&format!("https://creator.douyin.com{}", request.path)).unwrap();
            let fields: BTreeMap<_, _> = parsed.query_pairs().collect();
            assert_eq!(fields["p_ui"], "2.4.22");
            assert_eq!(fields["passport_jssdk_version"], "3.4.20");
            assert_eq!(fields["sign"].len(), 64);
            assert_eq!(fields["p_no"].len(), 64);
            assert!(
                request
                    .path
                    .contains("request_host=https%253A%252F%252Fcreator.douyin.com")
            );
        }

        assert_eq!(
            requests[3].header("x-tt-passport-csrf-token"),
            Some("passport-csrf-fixture")
        );
        assert_eq!(
            requests[3].header("bd-ticket-guard-web-sign-type"),
            Some("0")
        );
        assert_eq!(
            requests[3].header("bd-ticket-guard-ree-public-key"),
            Some(flow.pending.public_key_base64.as_str())
        );
        assert!(
            requests
                .iter()
                .all(|r| !r.path.contains("msToken") && !r.path.contains("a_bogus"))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(flow.directory.join(PENDING_FILE))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(flow.directory.join("qrcode.svg"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn rejection_4031_is_terminal_and_never_retries_or_claims_captcha() {
        let (origin, handle) = server(vec![ExpectedReply::json(json!({
            "message":"error","data":{"error_code":4031,"description":"fixture secret response"}
        }))]);
        let (_temp, mut flow) = test_flow(origin);
        ready(&mut flow);
        let error = flow.poll().unwrap_err();
        assert!(error.contains("server_blocked"));
        assert!(error.contains("4031"));
        assert!(!error.contains("fixture secret response"));
        assert_eq!(flow.pending.stage, "server_blocked");
        assert_eq!(flow.pending.last_http_status, Some(200));
        assert_eq!(flow.pending.last_business_code, Some(4031));
        assert!(flow.poll().unwrap_err().contains("禁止重复请求"));
        assert_eq!(handle.join().unwrap().len(), 1);
    }

    #[test]
    fn explicit_expiry_is_terminal_without_new_qr() {
        let (origin, handle) = server(vec![ExpectedReply::json(json!({
            "message":"success","data":{"error_code":0,"status":"expired"}
        }))]);
        let (_temp, mut flow) = test_flow(origin);
        ready(&mut flow);
        assert_eq!(flow.poll().unwrap(), Poll::Expired);
        assert_eq!(flow.pending.stage, "expired");
        assert!(flow.poll().is_err());
        assert_eq!(handle.join().unwrap().len(), 1);
    }

    #[test]
    fn unknown_status_is_not_login_success() {
        let (origin, handle) = server(vec![ExpectedReply::json(json!({
            "message":"success","data":{"error_code":0,"status":"new_server_state","redirect_url":"https://evil.invalid/"}
        }))]);
        let (_temp, mut flow) = test_flow(origin);
        ready(&mut flow);
        assert!(flow.poll().unwrap_err().contains("未识别"));
        assert_eq!(flow.pending.stage, "response_unknown");
        assert!(
            !flow.public_state("response_unknown")["publishReady"]
                .as_bool()
                .unwrap()
        );
        assert!(flow.poll().is_err());
        assert_eq!(handle.join().unwrap().len(), 1);
    }

    #[test]
    fn confirmed_still_requires_the_new_public_key_and_server_ticket() {
        let other = PendingSession::new().unwrap();
        let data = json!({
            "ticket":"fixture-ticket",
            "ts_sign":"fixture-ts-sign",
            "client_cert":format!("pub.{}", other.public_key_base64)
        });
        let (origin, handle) = server(vec![ExpectedReply {
            status: 200,
            headers: vec![(
                "bd-ticket-guard-server-data".into(),
                STANDARD.encode(data.to_string()),
            )],
            body: json!({"message":"success","data":{"error_code":0,"status":"confirmed"}})
                .to_string(),
        }]);
        let (_temp, mut flow) = test_flow(origin);
        ready(&mut flow);
        assert_eq!(flow.poll().unwrap(), Poll::Confirmed);
        assert!(
            flow.finish(Duration::from_secs(3))
                .unwrap_err()
                .contains("key_mismatch")
        );
        assert_eq!(flow.pending.stage, "key_mismatch");
        assert!(
            load_pending(&flow.directory)
                .err()
                .unwrap()
                .contains("不会自动重试")
        );
        assert_eq!(handle.join().unwrap().len(), 1);
        let own = json!({
            "ticket":"fixture-ticket","ts_sign":"fixture-ts-sign",
            "client_cert":format!("pub.{}", flow.pending.public_key_base64)
        });
        assert!(validate_server_binding(&own, &flow.pending.public_key_base64).is_ok());
        assert!(
            validate_server_binding(
                &json!({"client_cert":own["client_cert"]}),
                &flow.pending.public_key_base64
            )
            .is_err()
        );
    }

    #[test]
    fn redirects_never_send_cookie_outside_creator_origin() {
        let origin = Url::parse(ORIGIN).unwrap();
        for blocked in [
            "https://creator.douyin.com.evil.invalid/",
            "https://evil.invalid/?ticket=secret",
            "http://creator.douyin.com/",
            "https://creator.douyin.com:444/",
            "https://x:password@creator.douyin.com/",
            "//evil.invalid/",
        ] {
            let error = checked_redirect(&origin, blocked).unwrap_err();
            assert!(!error.contains("ticket=secret"));
            assert!(!error.contains("password"));
        }
        assert!(checked_redirect(&origin, "/passport/callback/?ticket=fixture").is_ok());
        let (origin, handle) = server(vec![ExpectedReply {
            status: 302,
            headers: vec![(
                "Location".into(),
                "https://evil.invalid/?ticket=fixture".into(),
            )],
            body: String::new(),
        }]);
        let (_temp, mut flow) = test_flow(origin);
        assert!(
            flow.follow_redirects("/callback/")
                .unwrap_err()
                .contains("同源")
        );
        assert_eq!(handle.join().unwrap().len(), 1);
    }

    #[test]
    fn verification_response_stops_without_retry() {
        let (origin, handle) = server(vec![ExpectedReply {
            status: 200,
            headers: vec![(
                "x-tt-verify-passport-decision".into(),
                "fixture-private-decision".into(),
            )],
            body: json!({"message":"success","data":{"error_code":0,"status":"new"}}).to_string(),
        }]);
        let (_temp, mut flow) = test_flow(origin);
        ready(&mut flow);
        let error = flow.poll().unwrap_err();
        assert!(error.contains("verification_required"));
        assert!(!error.contains("fixture-private-decision"));
        assert!(flow.poll().is_err());
        assert_eq!(handle.join().unwrap().len(), 1);
    }

    #[test]
    fn cookies_obey_domain_path_secure_expiry_and_deletion() {
        let url = Url::parse("https://creator.douyin.com/passport/web/get_qrcode/").unwrap();
        let mut cookies = Vec::new();
        let mut headers = HeaderMap::new();
        for cookie in [
            "sessionid=fixture; Domain=.douyin.com; Path=/; Secure; HttpOnly",
            "passport_csrf_token=fixture-csrf; Path=/passport/; Secure",
            "obsolete=old; Path=/; Max-Age=0",
        ] {
            headers.append(SET_COOKIE, cookie.parse().unwrap());
        }
        absorb_cookies(&mut cookies, &url, &headers, 100).unwrap();
        assert_eq!(cookies.len(), 2);
        assert!(cookie_header(&cookies, &url, 100).starts_with("passport_csrf_token="));
        assert_eq!(
            cookie_header(
                &cookies,
                &Url::parse("https://creator.douyin.com/web/api/").unwrap(),
                100
            ),
            "sessionid=fixture"
        );
        assert!(
            cookie_header(&cookies, &Url::parse("https://evil.invalid/").unwrap(), 100).is_empty()
        );
        assert!(
            cookie_header(
                &cookies,
                &Url::parse("http://creator.douyin.com/").unwrap(),
                100
            )
            .is_empty()
        );
        let mut delete = HeaderMap::new();
        delete.insert(
            SET_COOKIE,
            "sessionid=gone; Domain=.douyin.com; Path=/; Max-Age=0"
                .parse()
                .unwrap(),
        );
        absorb_cookies(&mut cookies, &url, &delete, 100).unwrap();
        assert_eq!(cookies.len(), 1);
        let mut invalid = HeaderMap::new();
        invalid.insert(
            SET_COOKIE,
            "sessionid=secret; Domain=.evil.invalid; Path=/"
                .parse()
                .unwrap(),
        );
        let error = absorb_cookies(&mut cookies, &url, &invalid, 100).unwrap_err();
        assert!(!error.contains("secret"));
        assert_eq!(cookie_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            cookie_date("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(1445412480)
        );
    }

    #[test]
    fn resume_preserves_the_same_key_and_rejects_blocked_or_stale_states() {
        let temp = tempfile::tempdir().unwrap();
        let (directory, pending, _guard) = new_pending(temp.path()).unwrap();
        let key = pending.public_key_base64.clone();
        let mut flow = LoginFlow::new(directory.clone(), pending, Duration::from_secs(1)).unwrap();
        ready(&mut flow);
        let loaded = load_pending(&directory).unwrap();
        assert_eq!(loaded.public_key_base64, key);
        assert_eq!(loaded.challenge.unwrap().token, "fixture-login-token");
        flow.set_stage("server_blocked").unwrap();
        assert!(
            load_pending(&directory)
                .err()
                .unwrap()
                .contains("不会自动重试")
        );
        flow.pending.stage = "qr_ready".into();
        flow.pending.created_at = now().unwrap().saturating_sub(LOCAL_RESUME_WINDOW + 10);
        flow.persist().unwrap();
        assert!(
            load_pending(&directory)
                .err()
                .unwrap()
                .contains("本地 30 分钟")
        );
    }

    #[test]
    fn server_header_is_not_confused_with_creator_or_passport_csrf() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ware-csrf-token", "0,creator-only,0".parse().unwrap());
        headers.insert(
            SET_COOKIE,
            "passport_csrf_token=passport-only; Path=/".parse().unwrap(),
        );
        assert_eq!(creator_csrf(&headers).as_deref(), Some("creator-only"));
        let data = json!({"ticket":"fixture","ts_sign":"fixture","client_cert":"pub.fixture"});
        headers.insert(
            "bd-ticket-guard-server-data",
            STANDARD.encode(data.to_string()).parse().unwrap(),
        );
        assert_eq!(
            capture_server_data(&headers, &[], &Url::parse(ORIGIN).unwrap(), 100).unwrap(),
            Some(data)
        );
        headers.insert(
            "bd-ticket-guard-server-data",
            "private-invalid-base64".parse().unwrap(),
        );
        let error =
            capture_server_data(&headers, &[], &Url::parse(ORIGIN).unwrap(), 100).unwrap_err();
        assert!(!error.contains("private-invalid-base64"));
    }

    #[test]
    fn malformed_json_is_persisted_as_terminal_and_never_repolled() {
        let (origin, handle) = server(vec![ExpectedReply {
            status: 200,
            headers: Vec::new(),
            body: "<html>private challenge body</html>".into(),
        }]);
        let (_temp, mut flow) = test_flow(origin);
        ready(&mut flow);
        let error = flow.poll().unwrap_err();
        assert!(!error.contains("private challenge body"));
        assert_eq!(flow.pending.stage, "response_unknown");
        assert!(flow.poll().is_err());
        assert!(load_pending(&flow.directory).is_err());
        assert_eq!(handle.join().unwrap().len(), 1);
        assert!(sensitive_header("fixture-session").unwrap().is_sensitive());
    }

    #[test]
    fn public_app_key_and_sdk_hashes_match_fixed_cross_language_vectors() {
        let timestamp = 1_795_094_000_123;
        let (ts, get_signature) = aid_signature(GET_QR, timestamp).unwrap();
        assert_eq!(ts, "1795089600");
        assert_eq!(
            get_signature,
            "0683cec9a56171dc3ee6e5f1c49ebac1bfa16f29aa8cc89632d909d38e8645c2"
        );
        assert_eq!(
            aid_signature(CHECK_QR, timestamp).unwrap().1,
            "1c81ad8b5b54892f899ff1b31546fbf74cf87cfe4c8fab92b63947c4f17ee8be"
        );
        assert_eq!(aid_signature(GET_QR, 1_795_048_000_000).unwrap().0, ts);
        assert!(aid_signature("/unverified/path", timestamp).is_err());
        let form = [
            ("need_logo", "false"),
            ("is_frontier", "true"),
            ("token", "fixture-login-token"),
            ("is_new_login", "1"),
            ("next", NEXT),
            ("need_short_url", "true"),
        ]
        .map(|(k, v)| (k.into(), v.into()));
        let wire = passport_wire(CHECK_QR, &sdk_params(), &form, timestamp).unwrap();
        let fields: BTreeMap<_, _> = wire.query.into_iter().collect();
        assert_eq!(
            fields["p_no"],
            "25b6ccee007bfbbf4e312cc40c176575395d9219af528f70d9be667ad1f63630"
        );
        assert_eq!(
            fields["sign"],
            "f7dfda8640e80f74c05e07fa0a8278d2812545f166827932c4573e990e9e88cb"
        );
        assert_eq!(
            fields["qs"],
            "6466666a706b715a76616e5a766a7077666029646c61296c765a63776a685a6c60766466666a706b7176646476296c765a63776a685a71716466666a706b7176616e296c765a6b60725a696a626c6b2969646b627064626029755a676129755a6f765a7129755a6f765a7329755a6b6a"
        );
        assert_eq!(fields["p_ui"], "2.4.22");
        assert_eq!(fields["passport_jssdk_version"], "3.4.20");
        assert_eq!(fields["request_host"], "https%3A%2F%2Fcreator.douyin.com");
        let mut tampered = form.to_vec();
        tampered[2].1 = "other-fixture-token".into();
        let altered: BTreeMap<_, _> = passport_wire(CHECK_QR, &sdk_params(), &tampered, timestamp)
            .unwrap()
            .query
            .into_iter()
            .collect();
        assert_ne!(fields["sign"], altered["sign"]);
        let mut duplicate = sdk_params();
        duplicate.push(("aid".into(), "2906".into()));
        assert!(passport_wire(CHECK_QR, &duplicate, &form, timestamp).is_err());
    }

    #[test]
    fn successful_envelope_with_invalid_qr_schema_is_terminal_before_resume() {
        for data in [
            json!({"error_code":0,"qrcode_index_url":QR_URL}),
            json!({"error_code":0,"token":"fixture-login-token"}),
            json!({"error_code":0,"token":"fixture-login-token","qrcode_index_url":"https://evil.invalid/?token=private"}),
            json!({"error_code":0,"token":"fixture-login-token","qrcode_index_url":QR_URL,"is_frontier":"unknown"}),
        ] {
            let (origin, handle) = server(vec![
                ExpectedReply {
                    status: 200,
                    headers: Vec::new(),
                    body: String::new(),
                },
                ExpectedReply {
                    status: 200,
                    headers: Vec::new(),
                    body: String::new(),
                },
                ExpectedReply::json(json!({"message":"success","data":data})),
            ]);
            let (_temp, mut flow) = test_flow(origin);
            let error = flow.begin().unwrap_err();
            assert!(!error.contains("token=private"));
            assert_eq!(flow.pending.stage, "response_unknown");
            assert!(resume_pending(&flow.directory).is_err());
            // Direct re-entry also stops before another GET.
            assert!(flow.begin().is_err());
            assert_eq!(handle.join().unwrap().len(), 3);
        }
    }

    #[test]
    fn pending_lock_rejects_second_owner_and_releases_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let (directory, _pending, guard) = new_pending(temp.path()).unwrap();
        let path = directory.join("login.lock");
        assert!(path.is_file());
        assert!(PendingLock::acquire(&directory).is_err());
        drop(guard);
        let next = PendingLock::acquire(&directory).unwrap();
        assert!(path.is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(next);
        assert!(PendingLock::acquire(&directory).is_ok());
    }

    #[test]
    fn failed_resume_validation_releases_lock_without_http_or_state_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let (directory, _pending, guard) = new_pending(temp.path()).unwrap();
        drop(guard);
        fs::write(directory.join(PENDING_FILE), b"{invalid pending").unwrap();
        assert!(resume_pending(&directory).is_err());
        let guard = PendingLock::acquire(&directory).unwrap();
        assert_eq!(
            fs::read(directory.join(PENDING_FILE)).unwrap(),
            b"{invalid pending"
        );
        drop(guard);
    }

    #[test]
    fn qr_url_whitelist_is_display_only_and_exact() {
        assert!(validate_qr_url(QR_URL).is_ok());
        for url in [
            "http://api.amemv.com/ucenter_web/app/aweme/scan_login/index/douyin_scan_code_login/cn/app/index.html?token=x",
            "https://api.amemv.com.evil.invalid/ucenter_web/app/aweme/scan_login/index/douyin_scan_code_login/cn/app/index.html?token=x",
            "https://api.amemv.com/other?token=x",
            "https://creator.douyin.com/?token=x",
        ] {
            assert!(validate_qr_url(url).is_err());
        }
    }
}
