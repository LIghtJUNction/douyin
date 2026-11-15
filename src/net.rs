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
use url::Url;

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

    pub fn post(&self, path: &str, form: &[(String, String)]) -> Result<Value, String> {
        self.request_url(Method::POST, native_url(path)?, &[], Some(form))
    }

    pub(crate) fn live_room_info(&self, room_id: &str) -> Result<Value, String> {
        if room_id.parse::<u64>().ok().filter(|id| *id > 0).is_none()
            || !room_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("直播间 ID 必须是正整数".into());
        }
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

    pub(crate) fn live_permissions(&self) -> Result<Value, String> {
        if self.mode != "direct" {
            return Err("直播权限查询使用独立请求，请使用 direct".into());
        }
        crate::native_auth::live_permission_auth(&self.session)?;
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

    fn get_url(&self, url: Url, params: &[(String, String)]) -> Result<Value, String> {
        self.request_url(Method::GET, url, params, None)
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
        if use_phone && form.is_some() {
            return Err("手机协议调试通道尚不支持 POST 请求".into());
        }
        if use_phone {
            if url.host_str() != Some("aweme.snssdk.com") {
                return Err("手机协议调试通道尚不支持直播接口，请使用 direct".into());
            }
            let mut phone = self.phone.borrow_mut();
            if phone.is_none() {
                *phone = Some(crate::phone::PhoneClient::connect(&self.session)?);
            }
            let result = phone
                .as_mut()
                .ok_or("手机通道未初始化")?
                .get(url.path(), params);
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
        let (native_headers, agent) =
            if host == "webcast.amemv.com" && url.path() == "/webcast/user/permission/" {
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
        // Verified native Token credentials do not depend on WebView Cookie
        // validity. Keep the legacy path for old sessions and other hosts.
        if native_headers.is_empty() {
            let cookies = self.session["cookies"]
                .as_array()
                .ok_or("登录配置缺少 Cookie")?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| "系统时间无效")?
                .as_secs();
            let cookie = if host == "webcast.amemv.com" && url.path() == "/webcast/room/info/" {
                auth::cookie_header(cookies, host, url.path(), now).unwrap_or_default()
            } else {
                auth::cookie_header(cookies, host, url.path(), now)?
            };
            let mut value = HeaderValue::from_str(&cookie).map_err(|_| "Cookie 请求头格式无效")?;
            value.set_sensitive(true);
            effective.insert("Cookie", value);
        } else {
            effective.extend(native_headers);
        }
        if host == "aweme.snssdk.com"
            || (host == "webcast.amemv.com" && url.path() == "/webcast/room/info/")
        {
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
        if bytes[stream.byte_offset()..]
            .iter()
            .any(|byte| !byte.is_ascii_whitespace())
        {
            check_response(&value)?;
            return Err("接口 JSON 后存在非空白附加数据".into());
        }
        check_response(&value)?;
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

fn body_digest(body: &[u8]) -> String {
    format!("{:X}", Md5::digest(body))
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
        Some(code) => Err(format!("抖音接口报告错误码 {code}")),
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
