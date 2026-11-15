//! Browser-Cookie transport with exact a_bogus query/body signing.

use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use reqwest::{Method, blocking::Client, header::HeaderValue, redirect::Policy};
use serde_json::Value;
use url::Url;

use crate::web_sign::WebSigner;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";
const BROWSER_PARAMS: [(&str, &str); 28] = [
    ("device_platform", "webapp"),
    ("aid", "6383"),
    ("channel", "channel_pc_web"),
    ("update_version_code", "170400"),
    ("pc_client_type", "1"),
    ("pc_libra_divert", "Windows"),
    ("version_code", "190500"),
    ("version_name", "19.5.0"),
    ("cookie_enabled", "true"),
    ("screen_width", "1920"),
    ("screen_height", "1080"),
    ("browser_language", "zh-CN"),
    ("browser_platform", "Win32"),
    ("browser_name", "Chrome"),
    ("browser_version", "125.0.0.0"),
    ("browser_online", "true"),
    ("engine_name", "Blink"),
    ("engine_version", "125.0.0.0"),
    ("os_name", "Windows"),
    ("os_version", "10"),
    ("cpu_core_num", "16"),
    ("device_memory", "8"),
    ("platform", "PC"),
    ("downlink", "10"),
    ("effective_type", "4g"),
    ("round_trip_time", "50"),
    ("support_h265", "1"),
    ("support_dash", "1"),
];

fn cookie_value<'a>(cookie: Option<&'a str>, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|expected| {
        cookie?.split(';').find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == *expected && !value.is_empty()).then_some(value)
        })
    })
}

fn route_public(url: &mut Url) -> Result<bool, String> {
    let browser_path = match url.path() {
        "/aweme/v1/user/profile/other/" => "/aweme/v1/web/user/profile/other/",
        "/aweme/v2/comment/list/" => "/aweme/v1/web/comment/list/",
        "/aweme/v1/comment/list/reply/" => "/aweme/v1/web/comment/list/reply/",
        _ => return Ok(false),
    };
    url.set_path(browser_path);
    if url.host_str() == Some("aweme.snssdk.com") {
        url.set_host(Some("www.douyin.com"))
            .map_err(|_| "网页接口主机无效")?;
    }
    Ok(true)
}

fn route_protected(url: &mut Url) -> Result<bool, String> {
    let browser_path = match url.path() {
        "/aweme/v1/aweme/detail/" => "/aweme/v1/web/aweme/detail/",
        "/aweme/v1/aweme/post/" => "/aweme/v1/web/aweme/post/",
        "/aweme/v1/aweme/favorite/" => "/aweme/v1/web/aweme/favorite/",
        "/aweme/v1/aweme/listcollection/" => "/aweme/v1/web/aweme/listcollection/",
        "/aweme/v1/mix/aweme/" => "/aweme/v1/web/mix/aweme/",
        "/aweme/v1/music/aweme/" => "/aweme/v1/web/music/aweme/",
        "/aweme/v1/feed/" => "/aweme/v1/web/tab/feed/",
        _ => return Ok(false),
    };
    url.set_path(browser_path);
    if url.host_str() == Some("aweme.snssdk.com") {
        url.set_host(Some("www.douyin.com"))
            .map_err(|_| "网页接口主机无效")?;
    }
    Ok(true)
}

pub(crate) fn request(
    method: Method,
    mut url: Url,
    params: &[(String, String)],
    form: Option<&[(String, String)]>,
) -> Result<Value, String> {
    let browser_public = route_public(&mut url)?;
    let web_sign_required = route_protected(&mut url)?;
    let browser_route = browser_public || web_sign_required;
    if let Ok(host) = std::env::var("DOUYIN_WEB_HOST") {
        if !matches!(host.as_str(), "www.douyin.com" | "aweme.snssdk.com") {
            return Err("DOUYIN_WEB_HOST 仅支持 www.douyin.com 或 aweme.snssdk.com".into());
        }
        if url.host_str() != Some("live.douyin.com") {
            url.set_host(Some(&host)).map_err(|_| "网页主机配置无效")?;
        }
    }
    let live_browser = url.host_str() == Some("live.douyin.com");
    let cookie = if browser_route || live_browser {
        Some(crate::cookie::ensure_guest()?)
    } else {
        crate::cookie::load()?
    };
    let uifid = if web_sign_required {
        Some(
            cookie_value(
                cookie.as_deref(),
                &[
                    "uifid",
                    "uifid_temp",
                    "uifidtemp",
                    "UIFID",
                    "UIFID_TEMP",
                    "UIFIDTEMP",
                ],
            )
            .filter(|value| value.len() >= 32)
            .ok_or("此网页接口需要 Cookie 中有效的 UIFID 访客标识，请重新登录或更新 Cookie")?
            .to_owned(),
        )
    } else {
        None
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_millis() as u64;
    let verify_fp = cookie_value(cookie.as_deref(), &["s_v_web_id"])
        .map(str::to_owned)
        .or_else(|| (!web_sign_required).then(|| crate::web_sign::generate_verify_fp(timestamp)));
    let body = form.map(|fields| {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                fields
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .finish()
    });
    let mut query = url.query_pairs_mut();
    if browser_route {
        query.extend_pairs(BROWSER_PARAMS);
    }
    if let Some(uifid) = &uifid {
        query.append_pair("uifid", uifid);
    }
    query
        .extend_pairs(
            params
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_str())),
        )
        .append_pair("msToken", &crate::web_sign::generate_ms_token(107));
    if let Some(verify_fp) = &verify_fp {
        query.append_pair("verifyFp", verify_fp);
        if web_sign_required {
            query.append_pair("fp", verify_fp);
        }
    }
    drop(query);
    if uifid.is_some() {
        let canonical = crate::web_sign::canonical_query(url.query().unwrap_or(""))?;
        url.set_query(Some(&canonical));
    }
    let signature = WebSigner::default().sign(
        url.query().unwrap_or(""),
        body.as_deref().unwrap_or(""),
        USER_AGENT,
        timestamp,
    );
    url.query_pairs_mut().append_pair("a_bogus", &signature);
    let web_sign = if let Some(uifid) = &uifid {
        let timestamp = timestamp / 1000;
        let (query, signature) =
            crate::web_sign::web_signature(url.query().unwrap_or(""), uifid, timestamp)?;
        url.set_query(Some(&query));
        Some((signature, timestamp))
    } else {
        None
    };

    let referer = if url.host_str() == Some("live.douyin.com") {
        "https://live.douyin.com/"
    } else {
        "https://www.douyin.com/"
    };
    let client = Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "无法创建网页请求客户端")?;
    let mut request = client
        .request(method, url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .header("Accept-Encoding", "gzip, deflate")
        .header("Accept-Language", "zh-CN,zh;q=0.9,en-US;q=0.8,en;q=0.7")
        .header("Referer", referer);
    if web_sign_required {
        request = request.header("x-tt-argus", "1");
    }
    if let (Some(uifid), Some((signature, timestamp))) = (&uifid, &web_sign) {
        let mut visitor = HeaderValue::from_str(uifid).map_err(|_| "网页访客标识格式无效")?;
        visitor.set_sensitive(true);
        request = request
            .header("uifid", visitor)
            .header("x-secsdk-web-signature", signature)
            .header("x-secsdk-web-expire", timestamp.to_string());
    }
    if let Some(cookie) = cookie.filter(|value| !value.trim().is_empty()) {
        let mut value = HeaderValue::from_str(&cookie).map_err(|_| "网页 Cookie 请求头无效")?;
        value.set_sensitive(true);
        request = request.header("Cookie", value);
    }
    if let Some(body) = body {
        request = request
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("x-ss-stub", format!("{:X}", Md5::digest(body.as_bytes())))
            .body(body);
    }
    let response = request.send().map_err(|_| "网页接口请求失败或超时")?;
    if !response.status().is_success() {
        return Err(format!("网页接口返回 HTTP {}", response.status().as_u16()));
    }
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let encoding = response
        .headers()
        .get("content-encoding")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut bytes = Vec::new();
    response
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "读取网页接口响应失败")?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("网页接口响应超过 16 MiB 限制".into());
    }
    let mut stream = serde_json::Deserializer::from_slice(&bytes).into_iter::<Value>();
    let value = stream
        .next()
        .transpose()
        .map_err(|error| {
            format!(
                "网页接口返回非 JSON 内容（{} 字节，Content-Type: {content_type}，Content-Encoding: {encoding}，JSON: {error}）",
                bytes.len()
            )
        })?
        .ok_or("网页接口返回空响应")?;
    if bytes[stream.byte_offset()..]
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        crate::net::check_response(&value)?;
        if !value["comments"].is_array() {
            return Err("网页接口 JSON 后存在非空白附加数据".into());
        }
    }
    crate::net::check_response(&value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, mpsc};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn routes_only_protected_native_endpoints_to_browser_endpoints() {
        for (native, browser) in [
            ("/aweme/v1/aweme/detail/", "/aweme/v1/web/aweme/detail/"),
            ("/aweme/v1/aweme/post/", "/aweme/v1/web/aweme/post/"),
            ("/aweme/v1/aweme/favorite/", "/aweme/v1/web/aweme/favorite/"),
            (
                "/aweme/v1/aweme/listcollection/",
                "/aweme/v1/web/aweme/listcollection/",
            ),
            ("/aweme/v1/mix/aweme/", "/aweme/v1/web/mix/aweme/"),
            ("/aweme/v1/music/aweme/", "/aweme/v1/web/music/aweme/"),
            ("/aweme/v1/feed/", "/aweme/v1/web/tab/feed/"),
        ] {
            let mut url = Url::parse(&format!("https://aweme.snssdk.com{native}")).unwrap();
            assert!(route_protected(&mut url).unwrap(), "{native}");
            assert_eq!(url.host_str(), Some("www.douyin.com"));
            assert_eq!(url.path(), browser);
        }

        for (native, browser) in [
            (
                "/aweme/v1/user/profile/other/",
                "/aweme/v1/web/user/profile/other/",
            ),
            ("/aweme/v2/comment/list/", "/aweme/v1/web/comment/list/"),
            (
                "/aweme/v1/comment/list/reply/",
                "/aweme/v1/web/comment/list/reply/",
            ),
        ] {
            let mut url = Url::parse(&format!("https://aweme.snssdk.com{native}")).unwrap();
            assert!(route_public(&mut url).unwrap());
            assert!(!route_protected(&mut url).unwrap());
            assert_eq!(url.host_str(), Some("www.douyin.com"));
            assert_eq!(url.path(), browser);
        }

        let mut search = Url::parse("https://aweme.snssdk.com/aweme/v2/search_feed/").unwrap();
        assert!(!route_public(&mut search).unwrap());
        assert!(!route_protected(&mut search).unwrap());
        assert_eq!(search.host_str(), Some("aweme.snssdk.com"));
        assert_eq!(search.path(), "/aweme/v2/search_feed/");
    }

    #[test]
    fn signs_browser_detail_with_the_cookie_visitor_identity() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        crate::store::save_at(
            &home.path().join("cookie.json"),
            &serde_json::json!({"cookie":"ttwid=fixture; UIFID_TEMP=0123456789abcdef0123456789abcdef; s_v_web_id=verify_fixture"}),
        )
        .unwrap();
        unsafe { std::env::set_var("DOUYIN_HOME", home.path()) };
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr();
        let server_thread = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            let target = request.url().to_string();
            for required in [
                "/aweme/v1/web/aweme/detail/",
                "aweme_id=7698380132884519355",
                "uifid=0123456789abcdef0123456789abcdef",
                "verifyFp=verify_fixture",
                "a_bogus=",
                "timestamp=",
                "x-secsdk-web-signature=",
            ] {
                assert!(target.contains(required), "missing {required}");
            }
            assert_eq!(
                request
                    .headers()
                    .iter()
                    .find(|header| header.field.equiv("uifid"))
                    .map(|header| header.value.as_str()),
                Some("0123456789abcdef0123456789abcdef")
            );
            request
                .respond(tiny_http::Response::from_string(
                    r#"{"status_code":0,"aweme_detail":{}}"#,
                ))
                .unwrap();
        });
        let url = Url::parse(&format!("http://{address}/aweme/v1/aweme/detail/")).unwrap();
        let value = request(
            Method::GET,
            url,
            &[("aweme_id".into(), "7698380132884519355".into())],
            None,
        )
        .unwrap();
        assert_eq!(value["status_code"], 0);
        server_thread.join().unwrap();
        unsafe { std::env::remove_var("DOUYIN_HOME") };
    }

    #[test]
    fn signs_and_sends_the_exact_form_and_parses_the_response() {
        let _guard = ENV_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        crate::store::save_at(
            &home.path().join("cookie.json"),
            &serde_json::json!({"cookie":"ttwid=fixture"}),
        )
        .unwrap();
        unsafe { std::env::set_var("DOUYIN_HOME", home.path()) };
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr();
        let (sender, receiver) = mpsc::channel();
        let server_thread = std::thread::spawn(move || {
            let mut request = server.recv().unwrap();
            let target = request.url().to_string();
            for required in [
                "a_bogus=",
                "msToken=",
                "verifyFp=",
                "a=1",
                "aweme_id=7698380132884519355",
            ] {
                assert!(target.contains(required), "missing {required}");
            }
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            sender.send(body).unwrap();
            let response = tiny_http::Response::from_string(r#"{"status_code":0}"#).with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").unwrap(),
            );
            request.respond(response).unwrap();
        });
        let url = Url::parse(&format!("http://{address}/aweme/v2/search_feed/?a=1")).unwrap();
        let value = request(
            Method::POST,
            url,
            &[("aweme_id".into(), "7698380132884519355".into())],
            Some(&[("a".into(), "b".into())]),
        )
        .unwrap();
        assert_eq!(value["status_code"], 0);
        assert_eq!(receiver.recv().unwrap(), "a=b");
        server_thread.join().unwrap();
        unsafe { std::env::remove_var("DOUYIN_HOME") };
    }
}
