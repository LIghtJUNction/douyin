//! Native API transport. Credentials and imported device identity stay together.
use std::cell::RefCell;
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use reqwest::{
    Method,
    blocking::Client,
    header::{HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use url::{Url, form_urlencoded};

use crate::{auth, store};

pub struct NativeClient {
    client: Client,
    session: Value,
    phone: RefCell<Option<crate::phone::PhoneClient>>,
    duckemu: crate::duckemu::DuckemuClient,
    mode: String,
}

impl NativeClient {
    pub fn saved() -> Result<Self, String> {
        let mode = std::env::var("DOUYIN_NATIVE_TRANSPORT").unwrap_or_else(|_| "direct".into());
        let session = if mode == "web" {
            match store::load() {
                Ok(session) => session,
                Err(_) => serde_json::json!({}),
            }
        } else {
            let session = store::load()?;
            auth::validate_session(&session)?;
            session
        };
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "无法创建抖音请求客户端")?;
        if !matches!(mode.as_str(), "direct" | "phone" | "duckemu" | "web") {
            return Err("DOUYIN_NATIVE_TRANSPORT 仅支持 direct、web、duckemu 或 phone".into());
        }
        Ok(Self {
            client,
            session,
            phone: RefCell::new(None),
            duckemu: crate::duckemu::DuckemuClient::default(),
            mode,
        })
    }

    pub fn account_id(&self) -> Result<&str, String> {
        field(&self.session["account"], "id")
    }
    pub fn account_sec_id(&self) -> Result<&str, String> {
        field(&self.session["account"], "secUserId")
    }

    pub fn is_current_account(&self, target: &str) -> bool {
        self.session["account"]["secUserId"].as_str() == Some(target)
            || self.session["account"]["id"].as_str() == Some(target)
    }

    pub fn is_web_transport(&self) -> bool {
        self.mode == "web"
    }

    pub fn get(&self, path: &str, params: &[(String, String)]) -> Result<Value, String> {
        self.get_url(native_url(path)?, params)
    }

    /// Like `get`, but skips `status_code` gating for APIs that omit it (e.g. some IM config payloads).
    pub fn get_lenient(&self, path: &str, params: &[(String, String)]) -> Result<Value, String> {
        let value = self.request_url_lenient(Method::GET, native_url(path)?, params, None)?;
        Ok(value)
    }

    pub fn post(&self, path: &str, form: &[(String, String)]) -> Result<Value, String> {
        self.request_url(Method::POST, native_url(path)?, &[], Some(form))
    }

    pub fn post_with_query(
        &self,
        path: &str,
        query: &[(String, String)],
        form: &[(String, String)],
    ) -> Result<Value, String> {
        self.request_url(Method::POST, native_url(path)?, query, Some(form))
    }

    pub fn post_with_query_headers(
        &self,
        path: &str,
        query: &[(String, String)],
        form: &[(String, String)],
        extra_headers: &[(String, String)],
    ) -> Result<Value, String> {
        if self.mode != "direct" {
            return Err("TicketGuard 写接口请使用 direct".into());
        }
        let (status, content_type, encoding, raw) = self.dispatch_native(
            Method::POST,
            native_url(path)?,
            query,
            extra_headers,
            Some((
                "application/x-www-form-urlencoded",
                form_urlencoded::Serializer::new(String::new())
                    .extend_pairs(
                        form.iter()
                            .map(|(key, value)| (key.as_str(), value.as_str())),
                    )
                    .finish()
                    .into_bytes(),
            )),
            true,
        )?;
        if !(200..300).contains(&status) {
            return Err(format!("抖音接口返回 HTTP {status}"));
        }
        let value = decode_json_body(&raw, &content_type, &encoding)?;
        check_response(&value)?;
        Ok(value)
    }

    /// JSON POST for recovered Retrofit `@Body` endpoints (e.g. enter_chat).
    pub fn post_json(
        &self,
        path: &str,
        extra_headers: &[(String, String)],
        body: &Value,
    ) -> Result<Value, String> {
        if self.mode == "web" || self.mode == "phone" {
            return Err("JSON 写接口请使用 direct 或 duckemu".into());
        }
        let bytes = serde_json::to_vec(body).map_err(|_| "JSON 正文序列化失败")?;
        let (status, content_type, encoding, raw) = self.dispatch_native(
            Method::POST,
            native_url(path)?,
            &[],
            extra_headers,
            Some(("application/json; charset=utf-8", bytes)),
            true,
        )?;
        if !(200..300).contains(&status) {
            return Err(format!("抖音接口返回 HTTP {status}"));
        }
        let value = decode_json_body(&raw, &content_type, &encoding)?;
        check_response(&value)?;
        Ok(value)
    }

    /// IM SDK HTTP gate: `https://imapi.snssdk.com` + `application/x-protobuf`.
    pub fn post_imapi_protobuf(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, String> {
        if self.mode == "web" || self.mode == "phone" {
            return Err("IM protobuf 发送请使用 direct 或 duckemu".into());
        }
        if !path.starts_with('/') || path.contains("://") || path.contains("..") {
            return Err("imapi 路径无效".into());
        }
        let url =
            Url::parse(&format!("https://imapi.snssdk.com{path}")).map_err(|_| "imapi 地址无效")?;
        let (status, _content_type, _encoding, raw) = self.dispatch_native(
            Method::POST,
            url,
            &[],
            &[],
            Some(("application/x-protobuf", body.to_vec())),
            false, // imapi Request carries device/token inside protobuf
        )?;
        if !(200..300).contains(&status) {
            return Err(format!("imapi 返回 HTTP {status}"));
        }
        Ok(raw)
    }

    pub(crate) fn im_request_parts(
        &self,
        cmd: i32,
        body: Vec<u8>,
    ) -> Result<crate::im_pb::ImRequestParts, String> {
        let token = self.session["nativeAuth"]["token"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("IM 请求需要 nativeAuth.token")?;
        let device = &self.session["device"];
        let device_id = device["deviceId"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("登录配置缺少 deviceId")?;
        let sequence_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?
            .as_millis() as i64;
        let version_code = match &device["versionCode"] {
            Value::String(s) if !s.is_empty() => s.clone(),
            Value::Number(n) => n.to_string(),
            _ => "400800".into(),
        };
        Ok(crate::im_pb::ImRequestParts {
            cmd,
            sequence_id,
            sdk_version: crate::im_pb::IM_SDK_VERSION.into(),
            token: token.into(),
            inbox_type: 0,
            build_number: crate::im_pb::IM_BUILD_NUMBER.into(),
            device_id: device_id.into(),
            device_platform: "android".into(),
            device_type: device["model"].as_str().unwrap_or("phone").into(),
            os_version: device["androidRelease"].as_str().unwrap_or("").into(),
            version_code,
            headers: vec![("flow-tag".into(), "new".into())],
            // IClientBridge$-CC defaults: access=`douyin_main`, biz=`douyin`.
            biz: Some("douyin".into()),
            access: Some("douyin_main".into()),
            // SEND/CREATE: default CERT_AUTH; upgraded to PASSPORT_CERT + ZT fields in im.rs
            // when DOUYIN_IM_ZT_JSON is set (`authType must be certAuth`).
            auth_type: matches!(
                cmd,
                crate::im_pb::CMD_SEND_MESSAGE | crate::im_pb::CMD_CREATE_CONVERSATION_V2
            )
            .then_some(crate::im_pb::AUTH_TYPE_CERT),
            client_cert: None,
            client_data: None,
            ree_public_key: None,
            iter_version: None,
            body,
        })
    }

    pub(crate) fn live_room_info(&self, room_id: &str) -> Result<Value, String> {
        numeric_id(room_id, "直播间")?;
        if self.mode == "web" {
            let url = Url::parse("https://live.douyin.com/webcast/room/web/enter/")
                .map_err(|_| "直播接口地址无效")?;
            return self.get_url(
                url,
                &[
                    ("aid".into(), "6383".into()),
                    ("app_name".into(), "douyin_web".into()),
                    ("live_id".into(), "1".into()),
                    ("device_platform".into(), "web".into()),
                    ("language".into(), "zh-CN".into()),
                    ("enter_from".into(), "web_live".into()),
                    ("cookie_enabled".into(), "true".into()),
                    ("screen_width".into(), "1920".into()),
                    ("screen_height".into(), "1080".into()),
                    ("browser_language".into(), "zh-CN".into()),
                    ("browser_platform".into(), "Win32".into()),
                    ("browser_name".into(), "Chrome".into()),
                    ("browser_version".into(), "125.0.0.0".into()),
                    ("web_rid".into(), room_id.into()),
                    ("enter_source".into(), "".into()),
                    ("is_need_double_stream".into(), "false".into()),
                ],
            );
        }
        let url = Url::parse("https://webcast.amemv.com/webcast/room/info/")
            .map_err(|_| "直播接口地址无效")?;
        self.get_url(url, &[("room_id".into(), room_id.into())])
    }

    fn require_live_transport(&self) -> Result<(), String> {
        match self.mode.as_str() {
            "direct" => {
                crate::native_auth::live_permission_auth(&self.session)?;
                Ok(())
            }
            "phone" => Ok(()),
            _ => Err("主播直播接口仅支持 direct 或 phone 传输".into()),
        }
    }

    pub(crate) fn live_permissions(&self) -> Result<Value, String> {
        self.require_live_transport()?;
        // Confirm the current account before using its anchor identity.
        let me = self.me()?;
        let sec_id = me["sec_uid"]
            .as_str()
            .filter(|v| !v.is_empty())
            .ok_or("当前账号未返回直播查询所需的标识")?;
        let url = Url::parse("https://webcast.amemv.com/webcast/user/permission/")
            .map_err(|_| "直播权限接口地址无效")?;
        self.get_url(url, &[("sec_anchor_id".into(), sec_id.into())])
    }

    /// Preview create_info. `platform=2` is recovered from the 40.8.0 call site.
    pub(crate) fn live_create_info(&self, platform: i32) -> Result<Value, String> {
        if platform != 2 {
            return Err("create_info 的 platform 目前只接受 APK 调用点证实的值 2".into());
        }
        self.require_live_transport()?;
        let url = Url::parse("https://webcast.amemv.com/webcast/room/create_info/")
            .map_err(|_| "开播预览接口地址无效")?;
        self.get_url(url, &[("platform".into(), platform.to_string())])
    }

    pub(crate) fn live_create_room(&self, form: &[(String, String)]) -> Result<Value, String> {
        self.require_live_transport()?;
        let url = Url::parse("https://webcast.amemv.com/webcast/room/create/")
            .map_err(|_| "开播接口地址无效")?;
        self.request_url(Method::POST, url, &[], Some(form))
    }

    pub(crate) fn live_leave_room(&self, room_id: &str) -> Result<Value, String> {
        numeric_id(room_id, "直播间")?;
        self.require_live_transport()?;
        let url = Url::parse("https://webcast.amemv.com/webcast/room/leave/")
            .map_err(|_| "离开直播间接口地址无效")?;
        self.request_url(
            Method::POST,
            url,
            &[],
            Some(&[("room_id".into(), room_id.into())]),
        )
    }

    /// create_check QueryMap. Only pass keys recovered from the APK call site.
    pub(crate) fn live_create_check(&self, params: &[(String, String)]) -> Result<Value, String> {
        const ALLOWED: &[&str] = &[
            "introduction",
            "digg_auth",
            "deliver",
            "live_room_mode",
            "check_scene",
            "chat_auth",
            "share_auth",
            "orientation",
            "third_party",
            "live_audio",
            "screen_shot",
            "acquaintance",
        ];
        for (key, _) in params {
            if !ALLOWED.contains(&key.as_str()) {
                return Err(format!("create_check 不接受未证实的参数: {key}"));
            }
        }
        self.require_live_transport()?;
        let url = Url::parse("https://webcast.amemv.com/webcast/room/create_check/")
            .map_err(|_| "开播检查接口地址无效")?;
        self.get_url(url, params)
    }

    /// Resume the current account's room; this GET changes room state.
    pub(crate) fn live_continue_room(&self, resolution_key: &str) -> Result<Value, String> {
        if resolution_key.is_empty()
            || resolution_key.trim() != resolution_key
            || resolution_key.len() > 128
            || resolution_key.chars().any(char::is_control)
        {
            return Err("续播 resolution_key 必须是 1..=128 字节且不含控制字符或首尾空白".into());
        }
        self.require_live_transport()?;
        let url = Url::parse("https://webcast.amemv.com/webcast/room/continue/")
            .map_err(|_| "续播接口地址无效")?;
        self.get_url(url, &[("resolution_key".into(), resolution_key.into())])
    }

    /// StatusApi's prepare/play/pause/finish report. An accepted report is not
    /// itself a room-state confirmation; callers must inspect room info.
    pub(crate) fn live_anchor_status(
        &self,
        room_id: &str,
        stream_id: &str,
        status: u8,
        reason_no: u32,
        source: &str,
    ) -> Result<Value, String> {
        let params = live_anchor_params(room_id, stream_id, status, reason_no, source)?;
        self.require_live_transport()?;
        let url = Url::parse("https://webcast.amemv.com/webcast/room/ping/anchor/")
            .map_err(|_| "主播状态接口地址无效")?;
        self.get_url(url, &params)
    }

    fn get_url(&self, url: Url, params: &[(String, String)]) -> Result<Value, String> {
        self.request_url(Method::GET, url, params, None)
    }

    /// Shared native dispatch for form/JSON/protobuf bodies.
    /// When `attach_identity` is true, appends device identity + ts query pairs (aweme/webcast).
    fn dispatch_native(
        &self,
        method: Method,
        mut url: Url,
        params: &[(String, String)],
        extra_headers: &[(String, String)],
        raw_body: Option<(&str, Vec<u8>)>,
        attach_identity: bool,
    ) -> Result<(u16, String, String, Vec<u8>), String> {
        if self.mode == "web" || self.mode == "phone" {
            return Err("此请求路径不支持 web/phone 传输".into());
        }
        let host = url.host_str().ok_or("接口缺少主机")?.to_string();
        if attach_identity {
            let identity = auth::request_identity(&self.session)?;
            if params.iter().any(|(key, _)| {
                matches!(key.as_str(), "ts" | "_rticket")
                    || identity.iter().any(|(reserved, _)| key == reserved)
            }) {
                return Err("请求参数不能覆盖登录设备的身份和版本".into());
            }
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "系统时间无效")?;
            url.query_pairs_mut()
                .extend_pairs(identity)
                .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                .append_pair("ts", &timestamp.as_secs().to_string())
                .append_pair("_rticket", &timestamp.as_millis().to_string());
        } else if !params.is_empty() {
            url.query_pairs_mut()
                .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        if self.mode == "duckemu"
            && host != "aweme.snssdk.com"
            && host != "imapi.snssdk.com"
            && !(host == "webcast.amemv.com" && url.path() == "/webcast/room/info/")
        {
            return Err("DuckDroid2 签名通道只允许原生 API、imapi 和公开房间查询".into());
        }
        let live_policy = live_request_policy(&method, &url);
        let (native_headers, agent) = if live_policy.is_some_and(|policy| policy.token) {
            crate::native_auth::live_permission_auth(&self.session)?
        } else {
            (
                crate::native_auth::headers(&self.session, &host)?,
                crate::native_auth::user_agent(&self.session, &host)?,
            )
        };
        let mut effective = HeaderMap::new();
        effective.insert(
            "User-Agent",
            HeaderValue::from_str(agent).map_err(|_| "原生 User-Agent 格式无效")?,
        );
        effective.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate"));
        let has_token = !native_headers.is_empty();
        if has_token {
            effective.extend(native_headers);
        }
        // IM SDK also sends X-Tt-Token; reuse the same Token when present.
        if host == "imapi.snssdk.com"
            && let Some(token) = self.session["nativeAuth"]["token"]
                .as_str()
                .filter(|v| !v.is_empty())
        {
            let mut value = HeaderValue::from_str(token).map_err(|_| "X-Tt-Token 格式无效")?;
            value.set_sensitive(true);
            effective.insert("X-Tt-Token", value);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?
            .as_secs();
        let allow_empty_cookie = host == "webcast.amemv.com" && url.path() == "/webcast/room/info/";
        let cookie = match self.session["cookies"].as_array() {
            Some(cookies) if !cookies.is_empty() => {
                if allow_empty_cookie {
                    auth::cookie_header(cookies, &host, url.path(), now).unwrap_or_default()
                } else {
                    auth::cookie_header(cookies, &host, url.path(), now)?
                }
            }
            _ if has_token || host == "imapi.snssdk.com" => {
                device_install_cookie(&self.session).unwrap_or_default()
            }
            _ if allow_empty_cookie => String::new(),
            _ => return Err("登录配置缺少 Cookie".into()),
        };
        if !cookie.is_empty() {
            let mut value = HeaderValue::from_str(&cookie).map_err(|_| "Cookie 请求头格式无效")?;
            value.set_sensitive(true);
            effective.insert("Cookie", value);
        } else if !has_token && !allow_empty_cookie && host != "imapi.snssdk.com" {
            return Err("登录配置缺少 Cookie".into());
        }
        if host == "aweme.snssdk.com" || live_policy.is_some_and(|policy| policy.ticket) {
            effective.insert(
                "x-ss-req-ticket",
                crate::native_auth::request_ticket()?
                    .parse()
                    .map_err(|_| "毫秒请求票据格式无效")?,
            );
        }
        for (name, value) in extra_headers {
            effective.insert(
                reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| "额外请求头名称无效")?,
                HeaderValue::from_str(value).map_err(|_| "额外请求头值无效")?,
            );
        }
        if let Some((content_type, bytes)) = &raw_body {
            effective.insert(
                "Content-Type",
                HeaderValue::from_str(content_type).map_err(|_| "Content-Type 无效")?,
            );
            effective.insert(
                "x-ss-stub",
                HeaderValue::from_str(&body_digest(bytes)).map_err(|_| "正文摘要格式无效")?,
            );
        }
        if self.mode == "duckemu" && host == "aweme.snssdk.com" {
            effective.extend(self.duckemu.sign(&url, &effective)?);
        }
        let mut request = self.client.request(method, url.clone()).headers(effective);
        if let Some((_, bytes)) = raw_body {
            request = request.body(bytes);
        }
        let response = request.send().map_err(|_| "抖音请求失败或超时")?;
        let status = response.status().as_u16();
        let encoding = response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut bytes = Vec::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取接口响应失败")?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("接口响应超过 16 MiB 限制".into());
        }
        Ok((status, content_type, encoding, bytes))
    }

    fn request_url(
        &self,
        method: Method,
        mut url: Url,
        params: &[(String, String)],
        form: Option<&[(String, String)]>,
    ) -> Result<Value, String> {
        if self.mode == "web" {
            return crate::web::request(method, url, params, form);
        }
        let identity = auth::request_identity(&self.session)?;
        if params.iter().any(|(key, _)| {
            matches!(key.as_str(), "ts" | "_rticket")
                || identity.iter().any(|(reserved, _)| key == reserved)
        }) {
            return Err("请求参数不能覆盖登录设备的身份和版本".into());
        }
        let connected = self.phone.borrow().is_some();
        let use_phone = connected || self.mode == "phone";
        if use_phone {
            let live = live_request_policy(&method, &url).is_some();
            if !live {
                if url.host_str() != Some("aweme.snssdk.com") {
                    return Err("手机协议调试通道仅支持原生 API 和已确认的直播接口".into());
                }
                if method != Method::GET || form.is_some() {
                    return Err("手机协议调试通道的普通 API 仅支持 GET 请求".into());
                }
            }
            let mut phone = self.phone.borrow_mut();
            if phone.is_none() {
                *phone = Some(crate::phone::PhoneClient::connect(&self.session)?);
            }
            let client = phone.as_mut().ok_or("手机通道未初始化")?;
            let result = if live {
                client.request(method.as_str(), url.as_str(), params, form)
            } else {
                client.get(url.path(), params)
            };
            let value = match result {
                Ok(value) => value,
                Err(error) => {
                    *phone = None;
                    return Err(error);
                }
            };
            check_response(&value)?;
            return Ok(value);
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?;
        url.query_pairs_mut()
            .extend_pairs(identity)
            .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .append_pair("ts", &timestamp.as_secs().to_string())
            .append_pair("_rticket", &timestamp.as_millis().to_string());
        let host = url.host_str().ok_or("接口缺少主机")?;
        if self.mode == "duckemu"
            && host != "aweme.snssdk.com"
            && !(host == "webcast.amemv.com" && url.path() == "/webcast/room/info/")
        {
            return Err("DuckDroid2 签名通道只允许原生 API 和公开房间查询".into());
        }
        let live_policy = live_request_policy(&method, &url);
        let (native_headers, agent) = if live_policy.is_some_and(|policy| policy.token) {
            crate::native_auth::live_permission_auth(&self.session)?
        } else {
            (
                crate::native_auth::headers(&self.session, host)?,
                crate::native_auth::user_agent(&self.session, host)?,
            )
        };
        let mut effective = HeaderMap::new();
        effective.insert(
            "User-Agent",
            HeaderValue::from_str(agent).map_err(|_| "原生 User-Agent 格式无效")?,
        );
        effective.insert("Accept-Encoding", HeaderValue::from_static("gzip, deflate"));
        // Token authenticates the account; anonymous/device Cookie context is still
        // required for some public lists (comments). Send both when available.
        let has_token = !native_headers.is_empty();
        if has_token {
            effective.extend(native_headers);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?
            .as_secs();
        let allow_empty_cookie = host == "webcast.amemv.com" && url.path() == "/webcast/room/info/";
        let cookie = match self.session["cookies"].as_array() {
            Some(cookies) if !cookies.is_empty() => {
                if allow_empty_cookie {
                    auth::cookie_header(cookies, host, url.path(), now).unwrap_or_default()
                } else {
                    auth::cookie_header(cookies, host, url.path(), now)?
                }
            }
            _ if has_token => device_install_cookie(&self.session).unwrap_or_default(),
            _ if allow_empty_cookie => String::new(),
            _ => return Err("登录配置缺少 Cookie".into()),
        };
        if !cookie.is_empty() {
            let mut value = HeaderValue::from_str(&cookie).map_err(|_| "Cookie 请求头格式无效")?;
            value.set_sensitive(true);
            effective.insert("Cookie", value);
        } else if !has_token && !allow_empty_cookie {
            return Err("登录配置缺少 Cookie".into());
        }
        if host == "aweme.snssdk.com" || live_policy.is_some_and(|policy| policy.ticket) {
            effective.insert(
                "x-ss-req-ticket",
                crate::native_auth::request_ticket()?
                    .parse()
                    .map_err(|_| "毫秒请求票据格式无效")?,
            );
        }
        let body = form.map(|fields| {
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(
                    fields
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_str())),
                )
                .finish()
        });
        if let Some(body) = &body {
            effective.insert(
                "Content-Type",
                HeaderValue::from_static("application/x-www-form-urlencoded"),
            );
            effective.insert(
                "x-ss-stub",
                HeaderValue::from_str(&body_digest(body.as_bytes()))
                    .map_err(|_| "正文摘要格式无效")?,
            );
        }
        if self.mode == "duckemu" {
            effective.extend(self.duckemu.sign(&url, &effective)?);
        }
        let mut request = self.client.request(method, url.clone()).headers(effective);
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().map_err(|_| "抖音请求失败或超时")?;
        if !response.status().is_success() {
            return Err(format!("抖音接口返回 HTTP {}", response.status().as_u16()));
        }
        let encoding = response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let mut bytes = Vec::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取接口响应失败")?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("接口响应超过 16 MiB 限制".into());
        }
        let mut stream = serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>();
        let value = stream
            .next()
            .transpose()
            .map_err(|error| {
                format!(
                    "{}（JSON 错误: {}，行列 {}:{}）",
                    response_format_error(&bytes, &content_type, &encoding),
                    error,
                    error.line(),
                    error.column()
                )
            })?
            .ok_or_else(|| response_format_error(&bytes, &content_type, &encoding))?;
        let trailer = &bytes[stream.byte_offset()..];
        if trailer.iter().any(|byte| !byte.is_ascii_whitespace()) {
            check_response(&value)?;
            consume_trailing_json_objects(trailer)?;
            return Ok(value);
        }
        check_response(&value)?;
        Ok(value)
    }

    fn request_url_lenient(
        &self,
        method: Method,
        url: Url,
        params: &[(String, String)],
        form: Option<&[(String, String)]>,
    ) -> Result<Value, String> {
        // Reuse the normal path then skip status_code gating for known schema-flexible APIs.
        // Implemented by temporarily accepting any JSON object with log_pb / data / extra.
        let value = {
            // Call through request_url would gate; use dispatch for GET-only lenient reads.
            if method != Method::GET || form.is_some() {
                return Err("lenient 请求目前只支持无 body 的 GET".into());
            }
            let (status, content_type, encoding, raw) =
                self.dispatch_native(method, url, params, &[], None, true)?;
            if !(200..300).contains(&status) {
                return Err(format!("抖音接口返回 HTTP {status}"));
            }
            decode_json_body(&raw, &content_type, &encoding)?
        };
        if value.get("status_code").is_some() {
            check_response(&value)?;
        } else if value.get("log_pb").is_none()
            && value.get("extra").is_none()
            && value.get("data").is_none()
        {
            return Err("接口未返回可识别的业务状态，无法确认请求成功".into());
        }
        Ok(value)
    }

    pub fn search(&self, keyword: &str, count: u32) -> Result<Value, String> {
        if keyword.trim().is_empty() || !(1..=20).contains(&count) {
            return Err("搜索关键词不能为空，单页数量必须为 1..=20".into());
        }
        self.post(
            "/aweme/v2/search_feed/",
            &[
                ("keyword".into(), keyword.into()),
                ("pull_type".into(), "1".into()),
                ("offset".into(), "0".into()),
                ("count".into(), count.to_string()),
                ("search_source".into(), "normal_search".into()),
                ("search_id".into(), crate::native_auth::request_ticket()?),
            ],
        )
    }

    pub fn aweme(&self, aweme_id: &str) -> Result<Value, String> {
        numeric_id(aweme_id, "作品")?;
        let response = self.get(
            "/aweme/v1/aweme/detail/",
            &[("aweme_id".into(), aweme_id.into())],
        )?;
        let returned = response["aweme_detail"]["aweme_id"]
            .as_str()
            .or_else(|| response["aweme_detail"]["group_id"].as_str());
        if returned != Some(aweme_id) {
            return Err("作品详情与请求目标不一致".into());
        }
        Ok(response["aweme_detail"].clone())
    }

    pub fn comments(&self, aweme_id: &str, count: u32) -> Result<Value, String> {
        numeric_id(aweme_id, "作品")?;
        if !(1..=20).contains(&count) {
            return Err("评论单页数量必须为 1..=20".into());
        }
        self.get(
            "/aweme/v2/comment/list/",
            &[
                ("aweme_id".into(), aweme_id.into()),
                ("cursor".into(), "0".into()),
                ("count".into(), count.to_string()),
            ],
        )
    }

    pub fn comment_replies(
        &self,
        aweme_id: &str,
        comment_id: &str,
        count: u32,
    ) -> Result<Value, String> {
        numeric_id(aweme_id, "作品")?;
        numeric_id(comment_id, "评论")?;
        if !(1..=20).contains(&count) {
            return Err("回复单页数量必须为 1..=20".into());
        }
        self.get(
            "/aweme/v1/comment/list/reply/",
            &[
                ("item_id".into(), aweme_id.into()),
                ("comment_id".into(), comment_id.into()),
                ("cursor".into(), "0".into()),
                ("count".into(), count.to_string()),
            ],
        )
    }

    pub fn user_posts(&self, sec_user_id: &str, count: u32) -> Result<Value, String> {
        if sec_user_id.trim().is_empty()
            || !sec_user_id.bytes().all(|byte| byte.is_ascii())
            || !(1..=20).contains(&count)
        {
            return Err("用户标识或单页数量无效".into());
        }
        self.get(
            "/aweme/v1/aweme/post/",
            &[
                ("sec_user_id".into(), sec_user_id.into()),
                ("max_cursor".into(), "0".into()),
                ("count".into(), count.to_string()),
            ],
        )
    }

    pub fn me(&self) -> Result<Value, String> {
        let response = self.get("/aweme/v1/user/profile/self/", &[])?;
        let uid = response["user"]["uid"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| response["user"]["uid"].as_u64().map(|id| id.to_string()))
            .ok_or("个人资料缺少账号 ID")?;
        if uid != self.account_id()? {
            return Err("个人资料账号与当前登录配置不一致，请重新登录".into());
        }
        Ok(response["user"].clone())
    }

    pub fn user(&self, target: &str) -> Result<Value, String> {
        let numeric = !target.is_empty() && target.bytes().all(|b| b.is_ascii_digit());
        let key = if numeric { "user_id" } else { "sec_user_id" };
        let response = self.get(
            "/aweme/v1/user/profile/other/",
            &[(key.into(), target.into())],
        )?;
        let user = response
            .get("user")
            .filter(|v| v.is_object())
            .ok_or("接口未返回账号资料")?;
        let returned = user[if numeric { "uid" } else { "sec_uid" }]
            .as_str()
            .map(str::to_owned)
            .or_else(|| {
                user[if numeric { "uid" } else { "sec_uid" }]
                    .as_u64()
                    .map(|v| v.to_string())
            });
        if returned.as_deref() != Some(target) {
            return Err("返回的账号资料与请求目标不同".into());
        }
        Ok(user.clone())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LiveRequestPolicy {
    token: bool,
    ticket: bool,
}

/// Keep authentication and request-ticket routing identical in both transports.
/// The exception applies only to these HTTPS endpoints and their recovered verbs.
fn live_request_policy(method: &Method, url: &Url) -> Option<LiveRequestPolicy> {
    if url.scheme() != "https"
        || url.host_str() != Some("webcast.amemv.com")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let path = url.path();
    let expected = match path {
        "/webcast/room/create/" | "/webcast/room/leave/" => Method::POST,
        "/webcast/room/info/"
        | "/webcast/user/permission/"
        | "/webcast/room/create_info/"
        | "/webcast/room/create_check/"
        | "/webcast/room/continue/"
        | "/webcast/room/ping/anchor/" => Method::GET,
        _ => return None,
    };
    if *method != expected {
        return None;
    }
    Some(LiveRequestPolicy {
        token: path != "/webcast/room/info/",
        ticket: path != "/webcast/user/permission/",
    })
}

fn live_anchor_params(
    room_id: &str,
    stream_id: &str,
    status: u8,
    reason_no: u32,
    source: &str,
) -> Result<Vec<(String, String)>, String> {
    numeric_id(room_id, "直播间")?;
    numeric_id(stream_id, "直播流")?;
    if !(1..=4).contains(&status) {
        return Err("主播状态仅支持 prepare=1、play=2、pause=3、finish=4".into());
    }
    if reason_no > i32::MAX as u32 {
        return Err("主播状态 reason_no 超出安卓有符号整数范围".into());
    }
    if source.len() > 128 || source.chars().any(char::is_control) {
        return Err("主播状态 source 不能超过 128 字节或包含控制字符".into());
    }
    // No active sticker is selected by this CLI. Omit prop_id instead of
    // inventing a device-specific value.
    Ok(vec![
        ("room_id".into(), room_id.into()),
        ("stream_id".into(), stream_id.into()),
        ("status".into(), status.to_string()),
        ("reason_no".into(), reason_no.to_string()),
        ("source".into(), source.into()),
    ])
}

fn body_digest(body: &[u8]) -> String {
    format!("{:X}", Md5::digest(body))
}

fn decode_json_body(bytes: &[u8], content_type: &str, encoding: &str) -> Result<Value, String> {
    let mut stream = serde_json::Deserializer::from_slice(bytes).into_iter::<Value>();
    let value = stream
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "{}（JSON 错误: {}，行列 {}:{}）",
                response_format_error(bytes, content_type, encoding),
                error,
                error.line(),
                error.column()
            )
        })?
        .ok_or_else(|| response_format_error(bytes, content_type, encoding))?;
    let trailer = &bytes[stream.byte_offset()..];
    if trailer.iter().any(|byte| !byte.is_ascii_whitespace()) {
        consume_trailing_json_objects(trailer)?;
    }
    Ok(value)
}

fn response_format_error(bytes: &[u8], content_type: &str, encoding: &str) -> String {
    let reason = if bytes.is_empty() {
        "接口返回空响应，无法确认成功；签名和服务端验证尚待核对"
    } else if bytes.starts_with(&[0x1f, 0x8b]) || encoding == "gzip" {
        "接口返回未解码的 gzip 数据，需要检查响应解压"
    } else if content_type
        .split(';')
        .next()
        .is_some_and(|v| matches!(v.trim(), "application/x-protobuf" | "application/protobuf"))
    {
        "接口返回 Protobuf 数据，当前请求尚未接入该响应解码"
    } else {
        "接口返回非 JSON 内容，可能需要签名或触发风控"
    };
    let mime = content_type
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| {
            matches!(
                *value,
                "application/json"
                    | "application/x-protobuf"
                    | "application/protobuf"
                    | "application/octet-stream"
                    | "text/html"
                    | "text/plain"
            )
        })
        .unwrap_or("other");
    let encoding = match encoding {
        "gzip" | "deflate" | "br" | "zstd" | "identity" => encoding,
        _ => "other",
    };
    format!(
        "{reason}（{} 字节，Content-Type: {mime}，Content-Encoding: {encoding}）",
        bytes.len()
    )
}

fn numeric_id(value: &str, kind: &str) -> Result<(), String> {
    if value.is_empty()
        || value.parse::<u64>().ok().filter(|id| *id > 0).is_none()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("{kind} ID 必须是正整数"));
    }
    Ok(())
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("登录配置缺少 {key}"))
}

fn native_url(path: &str) -> Result<Url, String> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#', '\\']) {
        return Err("接口仅接受当前抖音主机上的绝对路径，不接受 URL 或内嵌查询参数".into());
    }
    let base = Url::parse("https://aweme.snssdk.com/").map_err(|_| "接口地址无效")?;
    let url = base.join(path).map_err(|_| "接口路径无效")?;
    if url.origin() != base.origin()
        || !(url.path().starts_with("/aweme/") || url.path().starts_with("/passport/"))
    {
        return Err("接口路径不在允许的抖音 API 范围内".into());
    }
    Ok(url)
}

pub(crate) fn integer(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_str()?.parse().ok())
}
pub(crate) fn truthy(value: &Value) -> bool {
    value
        .as_bool()
        .unwrap_or_else(|| integer(value).is_some_and(|n| n != 0))
}

fn device_install_cookie(session: &Value) -> Option<String> {
    let install_id = session["device"]["installId"]
        .as_str()
        .filter(|value| !value.is_empty() && *value != "0")
        .map(str::to_owned)
        .or_else(|| {
            integer(&session["device"]["installId"])
                .filter(|value| *value > 0)
                .map(|value| value.to_string())
        })?;
    Some(format!("install_id={install_id}"))
}

fn consume_trailing_json_objects(trailer: &[u8]) -> Result<(), String> {
    let mut offset = 0;
    while offset < trailer.len() {
        while offset < trailer.len() && trailer[offset].is_ascii_whitespace() {
            offset += 1;
        }
        if offset >= trailer.len() {
            return Ok(());
        }
        let mut stream =
            serde_json::Deserializer::from_slice(&trailer[offset..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(_)) => offset += stream.byte_offset(),
            Some(Err(_)) | None => return Err("接口 JSON 后存在非空白附加数据".into()),
        }
    }
    Ok(())
}

pub(crate) fn check_response(value: &Value) -> Result<(), String> {
    if value["search_nil_info"]["search_nil_type"] == "antispam_check"
        || value["search_nil_info"]["search_nil_item"] == "hit_shark"
    {
        return Err("搜索请求被服务端风控拦截，返回空列表不代表没有搜索结果".into());
    }
    fn risk(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().any(|(key, value)| {
                key == "verify_check" || key == "verify_center_decision_conf" || risk(value)
            }),
            Value::Array(rows) => rows.iter().any(risk),
            _ => false,
        }
    }
    if risk(value) {
        return Err("接口要求额外验证，请在手机中完成验证后重试".into());
    }
    match integer(&value["status_code"]) {
        Some(0) => Ok(()),
        Some(code) => {
            // Webcast errors can put the user-facing explanation under data
            // instead of status_msg. Select only these short scalar fields;
            // never dump data, which can also contain credentials or tickets.
            let detail = ["/status_msg", "/data/prompts", "/data/message"]
                .into_iter()
                .filter_map(|path| value.pointer(path).and_then(Value::as_str))
                .map(str::trim)
                .find(|msg| {
                    !msg.is_empty()
                        && msg.chars().count() <= 256
                        && !msg.chars().any(char::is_control)
                })
                .map(|msg| format!("：{msg}"))
                .unwrap_or_default();
            Err(format!("抖音接口报告错误码 {code}{detail}"))
        }
        None => Err("接口未返回可识别的业务状态，无法确认请求成功".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rejects_credential_destinations_outside_native_api() {
        for path in [
            "https://evil.invalid/aweme/x",
            "//evil.invalid/aweme/x",
            "/aweme/x?redirect=x",
            "/aweme/../../x",
            "/aweme/\\evil/x",
        ] {
            assert!(native_url(path).is_err(), "{path}");
        }
        assert_eq!(
            native_url("/aweme/v1/feed/").unwrap().host_str(),
            Some("aweme.snssdk.com")
        );
    }
    #[test]
    fn live_auth_policy_is_bound_to_the_exact_https_endpoint_and_method() {
        let cases = [
            (Method::GET, "/webcast/room/info/", false, true),
            (Method::GET, "/webcast/user/permission/", true, false),
            (Method::GET, "/webcast/room/create_info/", true, true),
            (Method::GET, "/webcast/room/create_check/", true, true),
            (Method::POST, "/webcast/room/create/", true, true),
            (Method::POST, "/webcast/room/leave/", true, true),
            (Method::GET, "/webcast/room/continue/", true, true),
            (Method::GET, "/webcast/room/ping/anchor/", true, true),
        ];
        for (method, path, token, ticket) in cases {
            let url = Url::parse(&format!("https://webcast.amemv.com{path}?room_id=123")).unwrap();
            assert_eq!(
                live_request_policy(&method, &url),
                Some(LiveRequestPolicy { token, ticket }),
                "{path}"
            );
            let wrong_method = if method == Method::GET {
                Method::POST
            } else {
                Method::GET
            };
            assert!(live_request_policy(&wrong_method, &url).is_none());
        }
        for raw in [
            "http://webcast.amemv.com/webcast/room/ping/anchor/",
            "https://webcast.amemv.com.evil.invalid/webcast/room/ping/anchor/",
            "https://evil.invalid/webcast/room/ping/anchor/",
            "https://webcast.amemv.com:8443/webcast/room/ping/anchor/",
            "https://user@webcast.amemv.com/webcast/room/ping/anchor/",
            "https://webcast.amemv.com/webcast/room/ping/anchor/#secret",
            "https://webcast.amemv.com/webcast/room/unknown/",
            "https://webcast.amemv.com/webcast/room/ping/anchor/child",
        ] {
            assert!(
                live_request_policy(&Method::GET, &Url::parse(raw).unwrap()).is_none(),
                "{raw}"
            );
        }
    }

    #[test]
    fn anchor_reports_validate_identity_and_keep_distinct_room_and_stream_ids() {
        for status in 1..=4 {
            let params = live_anchor_params("123", "456", status, 0, "anchor_ping").unwrap();
            let fields: std::collections::BTreeMap<_, _> = params.into_iter().collect();
            assert_eq!(fields["room_id"], "123");
            assert_eq!(fields["stream_id"], "456");
            assert_eq!(fields["status"], status.to_string());
            assert_eq!(fields["reason_no"], "0");
            assert_eq!(fields["source"], "anchor_ping");
            assert!(!fields.contains_key("prop_id"));
        }
        assert!(live_anchor_params("1", "2", 3, 0, "").is_ok());
        for invalid in ["", "0", "-1", "1&status=4", "18446744073709551616"] {
            assert!(live_anchor_params(invalid, "2", 2, 0, "").is_err());
            assert!(live_anchor_params("1", invalid, 2, 0, "").is_err());
        }
        assert!(live_anchor_params("1", "2", 0, 0, "").is_err());
        assert!(live_anchor_params("1", "2", 5, 0, "").is_err());
        assert!(live_anchor_params("1", "2", 4, u32::MAX, "").is_err());
        assert!(live_anchor_params("1", "2", 4, 0, "anchor\nping").is_err());
        assert!(live_anchor_params("1", "2", 4, 0, &"x".repeat(129)).is_err());
    }

    #[test]
    fn lifecycle_validates_parameters_before_loading_credentials_or_using_transport() {
        let client = NativeClient {
            client: Client::new(),
            session: json!({}),
            phone: RefCell::new(None),
            duckemu: crate::duckemu::DuckemuClient::default(),
            mode: "web".into(),
        };
        for invalid in ["", " HD", "HD ", "HD\n", &"x".repeat(129)] {
            assert!(
                client
                    .live_continue_room(invalid)
                    .unwrap_err()
                    .contains("resolution_key")
            );
        }
        assert!(
            client
                .live_anchor_status("0", "2", 2, 0, "")
                .unwrap_err()
                .contains("直播间")
        );
        assert!(
            client
                .live_anchor_status("1", "0", 2, 0, "")
                .unwrap_err()
                .contains("直播流")
        );
        assert!(
            client
                .live_continue_room("HD")
                .unwrap_err()
                .contains("direct")
        );
        assert!(
            client
                .live_anchor_status("1", "2", 2, 0, "")
                .unwrap_err()
                .contains("direct")
        );
    }

    #[test]
    fn public_targets_do_not_require_native_account_fields() {
        let mut client = NativeClient {
            client: Client::new(),
            session: serde_json::json!({}),
            phone: RefCell::new(None),
            duckemu: crate::duckemu::DuckemuClient::default(),
            mode: "web".into(),
        };
        assert!(!client.is_current_account("public-sec-uid"));
        client.session = serde_json::json!({"account":{"secUserId":"self-sec-uid","id":"42"}});
        assert!(client.is_current_account("self-sec-uid"));
        assert!(client.is_current_account("42"));
        assert!(!client.is_current_account("public-sec-uid"));
    }

    #[test]
    fn native_ids_are_bounded_and_do_not_accept_non_numeric_values() {
        assert!(numeric_id("123", "作品").is_ok());
        for invalid in ["", "0", "-1", "abc", "123?x=1"] {
            assert!(numeric_id(invalid, "作品").is_err());
        }
    }

    #[test]
    fn post_body_digest_uses_the_exact_transmitted_bytes() {
        assert_eq!(body_digest(b""), "D41D8CD98F00B204E9800998ECF8427E");
        assert_eq!(body_digest(b"abc"), "900150983CD24FB0D6963F7D28E17F72");
    }

    #[test]
    fn http_json_alone_is_not_business_success() {
        assert!(check_response(&json!({"status_code":"0"})).is_ok());
        for value in [
            json!({}),
            json!({"status_code":8}),
            json!({"status_code":0,"data":{"verify_check":true}}),
            json!({"status_code":0,"data":[],"search_nil_info":{"search_nil_type":"antispam_check","search_nil_item":"hit_shark"}}),
        ] {
            assert!(check_response(&value).is_err());
        }
    }

    #[test]
    fn webcast_errors_preserve_short_explanations_without_dumping_data() {
        let value = json!({"status_code":10001,"data":{
            "message":"Service Unavailable","prompts":"服务器打瞌睡了, 请稍后刷新",
            "ticket":"private-fixture","stream_url":"private-fixture"
        }});
        assert_eq!(
            check_response(&value).unwrap_err(),
            "抖音接口报告错误码 10001：服务器打瞌睡了, 请稍后刷新"
        );
        let mut value = value;
        value["status_msg"] = json!(" explicit reason ");
        assert_eq!(
            check_response(&value).unwrap_err(),
            "抖音接口报告错误码 10001：explicit reason"
        );
        value["status_msg"] = json!(" ");
        value["data"]["prompts"] = json!("invalid\nmessage");
        assert_eq!(
            check_response(&value).unwrap_err(),
            "抖音接口报告错误码 10001：Service Unavailable"
        );
        value["data"]["message"] = json!("x".repeat(257));
        assert_eq!(
            check_response(&value).unwrap_err(),
            "抖音接口报告错误码 10001"
        );
    }

    #[test]
    fn accepts_concatenated_json_trailers_and_rejects_opaque_padding() {
        assert!(consume_trailing_json_objects(br#"{"log_pb":{"impr_id":"x"}}"#).is_ok());
        assert!(consume_trailing_json_objects(br#"  {"a":1}{"b":2} "#).is_ok());
        assert!(consume_trailing_json_objects(b"\n\t").is_ok());
        assert!(consume_trailing_json_objects(b"not-json").is_err());
        assert!(consume_trailing_json_objects(&[0, 1, 2, 3]).is_err());
    }

    #[test]
    fn device_install_cookie_uses_positive_install_identifiers_only() {
        assert_eq!(
            device_install_cookie(&json!({"device":{"installId":"123"}})).as_deref(),
            Some("install_id=123")
        );
        assert_eq!(
            device_install_cookie(&json!({"device":{"installId":456}})).as_deref(),
            Some("install_id=456")
        );
        assert!(device_install_cookie(&json!({"device":{"installId":0}})).is_none());
        assert!(device_install_cookie(&json!({"device":{}})).is_none());
    }

    #[test]
    fn distinguishes_empty_encoded_and_unknown_responses_without_body_echo() {
        assert!(response_format_error(b"", "", "").contains("空响应"));
        assert!(response_format_error(&[0x1f, 0x8b, 0], "", "").contains("gzip"));
        assert!(
            response_format_error(b"opaque", "application/x-protobuf; charset=binary", "")
                .contains("Protobuf")
        );
        let error =
            response_format_error(b"private-cookie-response", "private-token", "private-key");
        assert!(!error.contains("private"));
    }
}
