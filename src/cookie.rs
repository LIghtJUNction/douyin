//! Web-session credentials are kept separate from Android and OAuth sessions.
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

use clap::Args;
use serde_json::{Value, json};

use crate::store;

#[derive(Debug, Args)]
pub struct LoginArgs {
    #[arg(
        long,
        env = "DOUYIN_COOKIE",
        hide_env_values = true,
        required_unless_present = "cookie_file",
        conflicts_with = "cookie_file"
    )]
    cookie: Option<String>,
    /// 从私有文件读取 Cookie，- 表示标准输入
    #[arg(long)]
    cookie_file: Option<PathBuf>,
}

fn parse(header: &str) -> Result<BTreeMap<String, String>, String> {
    let header = header.trim_matches([' ', '\t']);
    if header.is_empty()
        || header.len() > 65536
        || header
            .bytes()
            .any(|b| !b.is_ascii() || b < 0x20 || b == 0x7f)
    {
        return Err("Cookie 为空、超过 64 KiB 或含非法字符".into());
    }
    let mut cookies = BTreeMap::new();
    for pair in header.split(';').filter(|part| !part.trim().is_empty()) {
        let (name, value) = pair
            .trim()
            .split_once('=')
            .ok_or("Cookie 必须使用 name=value 格式")?;
        let name = name.trim();
        let value = value.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            || !value
                .bytes()
                .all(|b| matches!(b,0x21|0x23..=0x2b|0x2d..=0x3a|0x3c..=0x5b|0x5d..=0x7e))
        {
            return Err("Cookie 名称或值不符合请求头格式".into());
        }
        if let Some(previous) = cookies.insert(name.to_owned(), value.to_owned())
            && previous != value
        {
            return Err("Cookie 同名记录冲突，拒绝猜测登录态".into());
        }
    }
    if !["sessionid", "sessionid_ss", "ttwid"]
        .iter()
        .any(|name| cookies.get(*name).is_some_and(|v| !v.is_empty()))
    {
        return Err("Cookie 缺少 sessionid、sessionid_ss 或 ttwid".into());
    }
    Ok(cookies)
}

pub fn login(args: LoginArgs) -> Result<Value, String> {
    let input = match args.cookie_file {
        Some(path) => crate::io::read_input(path.to_str().ok_or("Cookie 文件路径不是 UTF-8")?)?,
        None => args
            .cookie
            .ok_or("请通过 --cookie、DOUYIN_COOKIE 或 --cookie-file 提供 Cookie")?,
    };
    // Cookie files commonly end with a single newline; inline values may not.
    let input = if input.ends_with('\n') {
        input.trim_end_matches(['\r', '\n'])
    } else {
        &input
    };
    let cookies = parse(input)?;
    let header = cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    store::save_at(
        &store::root().join("cookie.json"),
        &json!({"cookie":header}),
    )
    .map_err(|_| "无法安全保存网页 Cookie")?;
    Ok(summary(Some(&cookies), false))
}

fn visitor_cookie_valid(header: &str) -> bool {
    parse(header).is_ok_and(|cookies| {
        cookies.get("ttwid").is_some_and(|value| !value.is_empty())
            && [
                "uifid",
                "uifid_temp",
                "uifidtemp",
                "UIFID",
                "UIFID_TEMP",
                "UIFIDTEMP",
            ]
            .iter()
            .any(|name| cookies.get(*name).is_some_and(|value| value.len() >= 32))
    })
}

fn merge_guest_cookies(
    existing: Option<&str>,
    offered: &[(String, String)],
) -> Result<String, String> {
    let mut cookies = existing.map(parse).transpose()?.unwrap_or_default();
    if ["sessionid", "sessionid_ss"]
        .iter()
        .any(|name| cookies.contains_key(*name))
    {
        return Err(
            "登录 Cookie 缺少 UIFID，请从同一浏览器会话重新导出，不能与匿名访客身份混用".into(),
        );
    }
    for (name, value) in offered {
        if matches!(name.as_str(), "ttwid" | "UIFID_TEMP" | "s_v_web_id") {
            cookies.insert(name.clone(), value.clone());
        }
    }
    let header = cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; ");
    if !visitor_cookie_valid(&header) {
        return Err("直播首页未下发完整的 ttwid 和 UIFID_TEMP 访客身份".into());
    }
    Ok(header)
}

/// Load or mint a browser visitor identity without confusing it with account login.
pub fn ensure_guest() -> Result<String, String> {
    let existing = load()?;
    if let Some(header) = existing
        .as_deref()
        .filter(|header| visitor_cookie_valid(header))
    {
        return Ok(header.to_owned());
    }
    if existing
        .as_deref()
        .map(parse)
        .transpose()?
        .is_some_and(|cookies| {
            ["sessionid", "sessionid_ss"]
                .iter()
                .any(|name| cookies.contains_key(*name))
        })
    {
        return Err(
            "登录 Cookie 缺少 UIFID，请从同一浏览器会话重新导出，不能与匿名访客身份混用".into(),
        );
    }
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "无法创建网页访客身份客户端")?;
    let response = client
        .get("https://live.douyin.com/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36",
        )
        .header("Accept-Language", "zh-CN,zh;q=0.9")
        .send()
        .map_err(|_| "获取网页访客身份失败或超时")?;
    if !response.status().is_success() {
        return Err(format!(
            "网页访客身份接口返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let offered = response
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .filter_map(|value| value.split(';').next())
        .filter_map(|pair| pair.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect::<Vec<_>>();
    let header = merge_guest_cookies(existing.as_deref(), &offered)?;
    store::save_at(
        &store::root().join("cookie.json"),
        &json!({"cookie":header}),
    )
    .map_err(|_| "无法安全保存网页访客 Cookie")?;
    Ok(header)
}

pub fn load() -> Result<Option<String>, String> {
    for (path, legacy) in [
        (store::root().join("cookie.json"), false),
        (store::root().join("config/settings.json"), true),
    ] {
        let value: Value = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "Cookie 配置不是合法 JSON")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("无法读取 Cookie 配置".into()),
        };
        if !value.is_object() {
            return Err("Cookie 配置必须是对象".into());
        }
        return match value.get("cookie") {
            Some(Value::String(cookie)) if !cookie.trim().is_empty() => Ok(Some(cookie.clone())),
            Some(Value::String(_)) | None if legacy => Ok(None),
            _ => Err("Cookie 配置缺少有效 cookie 字符串".into()),
        };
    }
    Ok(None)
}

fn summary(cookies: Option<&BTreeMap<String, String>>, verified: bool) -> Value {
    json!({"source":"web-cookie","configured":cookies.is_some(),"formatValid":cookies.is_some(),"onlineVerified":verified,
        "cookieCount":cookies.map_or(0,BTreeMap::len),"hasSessionCookie":cookies.is_some_and(|values|["sessionid","sessionid_ss"].iter().any(|name|values.get(*name).is_some_and(|v|!v.is_empty())))})
}

pub fn status(offline: bool) -> Result<Value, String> {
    let Some(header) = load()? else {
        return Ok(summary(None, false));
    };
    let cookies = parse(&header)?;
    if offline {
        return Ok(summary(Some(&cookies), false));
    }
    if !probe(&header)? {
        return Err("网页登录态已过期或未登录".into());
    }
    Ok(summary(Some(&cookies), true))
}

fn probe(header: &str) -> Result<bool, String> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法创建网页登录检查客户端")?;
    let response=client.get("https://sso.douyin.com/check_login/").header("Cookie",header).header("User-Agent","Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36").send().map_err(|_|"网页登录检查请求失败或超时")?;
    if !response.status().is_success() {
        return Err(format!(
            "网页登录检查返回 HTTP {}",
            response.status().as_u16()
        ));
    }
    let mut body = Vec::new();
    response
        .take(1024 * 1024 + 1)
        .read_to_end(&mut body)
        .map_err(|_| "无法读取网页登录检查响应")?;
    if body.len() > 1024 * 1024 {
        return Err("网页登录检查响应超过 1 MiB 限制".into());
    }
    parse_probe(&body)
}

fn parse_probe(body: &[u8]) -> Result<bool, String> {
    let value: Value = serde_json::from_slice(body)
        .map_err(|_| "网页登录检查返回非 JSON，可能遇到验证码或风控")?;
    value["has_login"]
        .as_bool()
        .ok_or_else(|| "网页登录检查缺少 has_login，无法确认登录态".into())
}

pub fn logout() -> Result<Value, String> {
    let legacy = store::root().join("config/settings.json");
    match std::fs::read(&legacy) {
        Ok(bytes) => {
            let mut value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "旧版 Cookie 配置不是合法 JSON")?;
            let object = value.as_object_mut().ok_or("旧版 Cookie 配置必须是对象")?;
            if object.remove("cookie").is_some() {
                store::save_at(&legacy, &value).map_err(|_| "无法清除旧版 Cookie")?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("无法读取旧版 Cookie 配置以注销".into()),
    }
    match std::fs::remove_file(store::root().join("cookie.json")) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("无法清除网页 Cookie".into()),
    }
    Ok(json!({"source":"web-cookie","loggedOut":true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_cookie_grammar_without_printing_values() {
        let parsed = parse("sessionid=fixture==; ttwid=public; sessionid=fixture==;").unwrap();
        assert_eq!(parsed["sessionid"], "fixture==");
        assert!(
            !summary(Some(&parsed), false)
                .to_string()
                .contains("fixture")
        );
        for bad in [
            "sessionid=one; sessionid=two",
            "sessionid=secret\r\nHost: evil",
            "sessionid=a b",
            "sessionid=\"quoted\"",
            "sessionid=",
            "other=anonymous",
        ] {
            assert!(parse(bad).is_err());
        }
    }
    #[test]
    fn merges_only_complete_anonymous_visitor_identities() {
        let offered = vec![
            ("ttwid".into(), "visitor-ttwid".into()),
            (
                "UIFID_TEMP".into(),
                "0123456789abcdef0123456789abcdef".into(),
            ),
            ("ignored".into(), "private-fixture".into()),
        ];
        let header = merge_guest_cookies(Some("ttwid=old; odin_tt=public"), &offered).unwrap();
        let cookies = parse(&header).unwrap();
        assert_eq!(cookies["odin_tt"], "public");
        assert_eq!(cookies["UIFID_TEMP"].len(), 32);
        assert!(!cookies.contains_key("ignored"));
        assert!(merge_guest_cookies(Some("ttwid=old"), &[]).is_err());
        let error = merge_guest_cookies(Some("sessionid=private-fixture"), &offered).unwrap_err();
        assert!(!error.contains("private-fixture"));
    }

    #[test]
    fn requires_explicit_login_evidence_and_hides_response_bodies() {
        assert!(parse_probe(br#"{"has_login":true}"#).unwrap());
        assert!(!parse_probe(br#"{"has_login":false}"#).unwrap());
        for body in [
            br#"{"status_code":0,"data":[]}"#.as_slice(),
            b"<html>private-fixture</html>",
        ] {
            let error = parse_probe(body).unwrap_err();
            assert!(!error.contains("private-fixture"));
        }
    }
}
