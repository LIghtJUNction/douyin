use std::process::Command;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT,
};
use serde_json::Value;

use crate::{cookie, err};

pub const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";

const SIGN_SCRIPT: &str = include_str!("../assets/douyin.js");

/// Resolves the crawl credentials: an explicit cookie beats the saved one, and the
/// saved user agent (when set) beats [`DEFAULT_USER_AGENT`].
pub fn credentials<'a>(
    saved: &'a Value,
    override_cookie: Option<&str>,
) -> Result<(String, &'a str), String> {
    let cookie_value = override_cookie
        .map(str::to_owned)
        .or_else(|| {
            saved
                .get("cookie")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "未登录。请先运行: douyin auth cookie-login".to_owned())?;
    if !cookie::validate(&cookie_value) {
        return Err("Cookie 格式校验失败".to_owned());
    }
    let user_agent = saved
        .get("userAgent")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_USER_AGENT);
    Ok((cookie_value, user_agent))
}

/// Builds a blocking client presenting the standard Douyin web headers.
pub fn web_client(cookie: &str, user_agent: &str, timeout_seconds: u64) -> Result<Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9"));
    headers.insert(REFERER, HeaderValue::from_static("https://www.douyin.com/"));
    headers.insert(USER_AGENT, HeaderValue::from_str(user_agent).map_err(err)?);
    headers.insert(COOKIE, HeaderValue::from_str(cookie).map_err(err)?);
    Client::builder()
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(err)
}

/// Douyin sometimes returns numeric fields (cursor, has_more, status_code) as quoted
/// strings instead of JSON numbers; parse either representation the same way.
pub fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.parse().ok())
}

pub fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_bool()
            .unwrap_or_else(|| value_i64(value).unwrap_or(0) != 0)
    })
}

pub fn contains_verify_check(value: &Value) -> bool {
    match value {
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key == "verify_check" || contains_verify_check(value)),
        Value::Array(values) => values.iter().any(contains_verify_check),
        Value::String(value) => value == "verify_check",
        _ => false,
    }
}

pub fn encode_query<K: AsRef<str>, V: AsRef<str>>(params: &[(K, V)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            let encoded: String =
                url::form_urlencoded::byte_serialize(value.as_ref().as_bytes()).collect();
            format!("{}={encoded}", key.as_ref())
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// True when a non-zero `limit` has been reached.
pub fn limit_reached(length: usize, limit: usize) -> bool {
    limit > 0 && length >= limit
}

/// Runs the bundled `assets/douyin.js` signer through Node.js to produce an `a_bogus` value.
pub fn sign(function: &str, query: &str, user_agent: &str) -> Result<String, String> {
    let query = serde_json::to_string(query).map_err(err)?;
    let user_agent = serde_json::to_string(user_agent).map_err(err)?;
    let script = format!("{SIGN_SCRIPT}\nprocess.stdout.write({function}({query}, {user_agent}));");
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| format!("无法启动 Node.js 签名运行时: {error}。抓取需要 node 命令。"))?;
    if !output.status.success() {
        return Err(format!(
            "生成 a_bogus 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(err)?;
    if value.trim().is_empty() {
        Err("生成 a_bogus 得到空结果".to_owned())
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_USER_AGENT, sign};

    #[test]
    fn bundled_signer_returns_a_bogus_value() {
        let value = sign(
            "sign_datail",
            "aweme_id=7380000000000000000&device_platform=webapp&aid=6383",
            DEFAULT_USER_AGENT,
        )
        .unwrap();
        assert!(value.ends_with('='));
        assert!(value.len() > 20);
    }
}
