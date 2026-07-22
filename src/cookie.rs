use std::collections::HashMap;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, USER_AGENT};
use reqwest::redirect::Policy;
use serde_json::Value;

const USER_AGENT_VALUE: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/124.0 Safari/537.36";

pub fn validate(cookie: &str) -> bool {
    let cookie = cookie.trim();
    if cookie.is_empty() || !cookie.contains('=') {
        return false;
    }
    parse(cookie)
        .keys()
        .any(|key| key.eq_ignore_ascii_case("sessionid") || key.eq_ignore_ascii_case("ttwid"))
}

pub fn parse(cookie: &str) -> HashMap<String, String> {
    cookie
        .split(';')
        .filter_map(|item| {
            let (key, value) = item.trim().split_once('=')?;
            let key = key.trim();
            let value = value.trim();
            (!key.is_empty() && !value.is_empty()).then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

pub fn probe(cookie: &str) -> Result<bool, String> {
    probe_sso(cookie)
}

fn client(cookie: &str) -> Result<Client, String> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(USER_AGENT_VALUE));
    headers.insert(
        COOKIE,
        HeaderValue::from_str(cookie)
            .map_err(|error| format!("Cookie 无法作为 HTTP 请求头: {error}"))?,
    );
    Client::builder()
        .default_headers(headers)
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| format!("创建网页登录态检查客户端失败: {error}"))
}

fn probe_sso(cookie: &str) -> Result<bool, String> {
    let response = client(cookie)?
        .get("https://sso.douyin.com/check_login/")
        .send()
        .map_err(|error| format!("发送网页登录态检查请求失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "网页登录态检查返回 HTTP 状态 {}",
            response.status()
        ));
    }
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response
        .bytes()
        .map_err(|error| format!("读取网页登录态检查响应失败: {error}"))?;
    parse_login_probe_response(&body, content_type.as_deref())
}

fn parse_login_probe_response(body: &[u8], content_type: Option<&str>) -> Result<bool, String> {
    let content_type = safe_content_type(content_type);
    let body: Value = serde_json::from_slice(body).map_err(|_| {
        format!(
            "网页登录态检查返回非 JSON 内容（Content-Type: {content_type}），可能遇到验证码、风控或上游接口变化"
        )
    })?;
    body.get("has_login")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            format!(
                "网页登录态检查返回无法识别的 JSON 结构（Content-Type: {content_type}），可能是上游接口变化"
            )
        })
}

fn safe_content_type(content_type: Option<&str>) -> String {
    let value: String = content_type
        .unwrap_or("unknown")
        .chars()
        .filter(|value| {
            value.is_ascii_alphanumeric()
                || matches!(value, '/' | '+' | '-' | '.' | ';' | '=' | ' ')
        })
        .take(80)
        .collect();
    if value.trim().is_empty() {
        "unknown".to_owned()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_login_probe_response, validate};

    #[test]
    fn validates_compatible_cookie_fields() {
        assert!(validate("sessionid=abc; ttwid=def"));
        assert!(validate("TTWID=def"));
        assert!(!validate("foo=bar"));
        assert!(!validate("sessionid"));
    }

    #[test]
    fn parses_values_containing_equals_signs() {
        let values = parse("sessionid=a=b; ttwid=c");
        assert_eq!(values["sessionid"], "a=b");
        assert_eq!(values["ttwid"], "c");
    }

    #[test]
    fn login_probe_accepts_logged_in_json() {
        assert_eq!(
            parse_login_probe_response(br#"{"has_login":true}"#, Some("application/json")),
            Ok(true)
        );
    }

    #[test]
    fn login_probe_accepts_logged_out_json() {
        assert_eq!(
            parse_login_probe_response(br#"{"has_login":false}"#, Some("application/json")),
            Ok(false)
        );
    }

    #[test]
    fn login_probe_rejects_anonymous_search_payload() {
        let result =
            parse_login_probe_response(br#"{"status_code":0,"data":[]}"#, Some("application/json"));
        assert!(result.is_err());
    }

    #[test]
    fn login_probe_reports_html_without_echoing_body() {
        let unique_body = "<html>UNIQUE_PRIVATE_RESPONSE_BODY</html>";
        let error =
            parse_login_probe_response(unique_body.as_bytes(), Some("text/html; charset=utf-8"))
                .expect_err("HTML must not be accepted as a login response");
        assert!(
            error.contains("text/html; charset=utf-8")
                && error.contains("验证码、风控或上游接口变化")
                && !error.contains("UNIQUE_PRIVATE_RESPONSE_BODY")
        );
    }
}
