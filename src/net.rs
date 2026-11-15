//! Native API transport. Credentials and imported device identity stay together.
use std::cell::RefCell;
use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::{
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
        let session = store::load()?;
        auth::validate_session(&session)?;
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "无法创建抖音请求客户端")?;
        let mode = std::env::var("DOUYIN_NATIVE_TRANSPORT").unwrap_or_else(|_| "direct".into());
        if !matches!(mode.as_str(), "direct" | "phone" | "duckemu") {
            return Err(
                "DOUYIN_NATIVE_TRANSPORT 仅支持 direct、duckemu（本地仿真）或 phone（协议调试）"
                    .into(),
            );
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

    pub fn get(&self, path: &str, params: &[(String, String)]) -> Result<Value, String> {
        self.get_url(native_url(path)?, params)
    }

    pub(crate) fn live_room_info(&self, room_id: &str) -> Result<Value, String> {
        if room_id.parse::<u64>().ok().filter(|id| *id > 0).is_none()
            || !room_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("直播间 ID 必须是正整数".into());
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

    fn get_url(&self, mut url: Url, params: &[(String, String)]) -> Result<Value, String> {
        let identity = auth::request_identity(&self.session)?;
        if params
            .iter()
            .any(|(key, _)| identity.iter().any(|(reserved, _)| key == reserved))
        {
            return Err("请求参数不能覆盖登录设备的身份和版本".into());
        }
        let connected = self.phone.borrow().is_some();
        let use_phone = connected || self.mode == "phone";
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
        url.query_pairs_mut()
            .extend_pairs(identity)
            .extend_pairs(params.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let host = url.host_str().ok_or("接口缺少主机")?;
        if self.mode == "duckemu" && host != "aweme.snssdk.com" {
            return Err("DuckDroid2 签名通道只允许抖音原生 API 主机".into());
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
            let mut value =
                HeaderValue::from_str(&auth::cookie_header(cookies, host, url.path(), now)?)
                    .map_err(|_| "Cookie 请求头格式无效")?;
            value.set_sensitive(true);
            effective.insert("Cookie", value);
        } else {
            effective.extend(native_headers);
        }
        if host == "aweme.snssdk.com" {
            effective.insert(
                "x-ss-req-ticket",
                crate::native_auth::request_ticket()?
                    .parse()
                    .map_err(|_| "毫秒请求票据格式无效")?,
            );
        }
        if self.mode == "duckemu" {
            effective.extend(self.duckemu.sign(&url, &effective)?);
        }
        let response = self
            .client
            .get(url.clone())
            .headers(effective)
            .send()
            .map_err(|_| "抖音请求失败或超时")?;
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
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| response_format_error(&bytes, &content_type, &encoding))?;
        check_response(&value)?;
        Ok(value)
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

fn response_format_error(bytes: &[u8], content_type: &str, encoding: &str) -> &'static str {
    if bytes.is_empty() {
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
    }
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

fn check_response(value: &Value) -> Result<(), String> {
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
