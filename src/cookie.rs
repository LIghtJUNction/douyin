use std::collections::HashMap;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, USER_AGENT};
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

pub fn probe(cookie: &str) -> bool {
    probe_web(cookie).unwrap_or(false) || probe_sso(cookie).unwrap_or(false)
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
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())
}

fn probe_web(cookie: &str) -> Result<bool, String> {
    let response = client(cookie)?
        .get("https://www.douyin.com/aweme/v1/web/general/search/single/")
        .query(&[
            ("keyword", "抖音"),
            ("offset", "0"),
            ("count", "1"),
            ("search_channel", "aweme_general"),
        ])
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Ok(false);
    }
    let body: Value = response.json().map_err(|error| error.to_string())?;
    Ok(body.get("status_code").and_then(Value::as_i64) == Some(0) && !contains_verify_check(&body))
}

fn probe_sso(cookie: &str) -> Result<bool, String> {
    let response = client(cookie)?
        .get("https://sso.douyin.com/check_login/")
        .send()
        .map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Ok(false);
    }
    let body: Value = response.json().map_err(|error| error.to_string())?;
    Ok(body.get("has_login").and_then(Value::as_bool) == Some(true))
}

fn contains_verify_check(value: &Value) -> bool {
    match value {
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key == "verify_check" || contains_verify_check(value)),
        Value::Array(values) => values.iter().any(contains_verify_check),
        Value::String(value) => value == "verify_check",
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, validate};

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
}
