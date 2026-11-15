//! Browser-Cookie transport with exact a_bogus query/body signing.

use std::io::Read;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use reqwest::{Method, blocking::Client, header::HeaderValue, redirect::Policy};
use serde_json::Value;
use url::Url;

use crate::web_sign::WebSigner;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36";

pub(crate) fn request(
    method: Method,
    mut url: Url,
    form: Option<&[(String, String)]>,
) -> Result<Value, String> {
    if let Ok(host) = std::env::var("DOUYIN_WEB_HOST") {
        if !matches!(host.as_str(), "www.douyin.com" | "aweme.snssdk.com") {
            return Err("DOUYIN_WEB_HOST 仅支持 www.douyin.com 或 aweme.snssdk.com".into());
        }
        url.set_host(Some(&host)).map_err(|_| "网页主机配置无效")?;
    }
    let cookie = crate::cookie::load()?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_millis() as u64;
    let body = form.map(|fields| {
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                fields
                    .iter()
                    .map(|(key, value)| (key.as_str(), value.as_str())),
            )
            .finish()
    });
    url.query_pairs_mut()
        .append_pair("msToken", &crate::web_sign::generate_ms_token(107))
        .append_pair("verifyFp", &crate::web_sign::generate_verify_fp(timestamp));
    let signature = WebSigner::default().sign(
        url.query().unwrap_or(""),
        body.as_deref().unwrap_or(""),
        USER_AGENT,
        timestamp,
    );
    url.query_pairs_mut().append_pair("a_bogus", &signature);

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
        .header("x-ss-req-ticket", timestamp.to_string());
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
            for required in ["a_bogus=", "msToken=", "verifyFp=", "a=1"] {
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
        let value = request(Method::POST, url, Some(&[("a".into(), "b".into())])).unwrap();
        assert_eq!(value["status_code"], 0);
        assert_eq!(receiver.recv().unwrap(), "a=b");
        server_thread.join().unwrap();
        unsafe { std::env::remove_var("DOUYIN_HOME") };
    }
}
