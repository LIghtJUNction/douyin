//! DuckDroid2 native-signature transport over a local Unix socket.
//! The emulator is an explicit external backend, never a phone fallback.

use std::cell::RefCell;
use std::io::{Read, Write};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use url::Url;

const DEFAULT_SOCKET: &str = "/tmp/duckemu-dy7/dy7.sock";
const MAX_FRAME: usize = 1024 * 1024;
const SIGNATURE_HEADERS: [&str; 7] = [
    "X-Perseus",
    "X-Medusa",
    "X-Argus",
    "X-Gorgon",
    "X-Helios",
    "X-Ladon",
    "X-Khronos",
];

#[derive(Default)]
pub(crate) struct DuckemuClient {
    session: RefCell<Option<String>>,
}

impl DuckemuClient {
    pub(crate) fn sign(&self, url: &Url, headers: &HeaderMap) -> Result<HeaderMap, String> {
        let path = std::env::var("DOUYIN_DUCKEMU_SOCKET").unwrap_or_else(|_| DEFAULT_SOCKET.into());
        if url.host_str() != Some("aweme.snssdk.com")
            && !(url.host_str() == Some("webcast.amemv.com") && url.path() == "/webcast/room/info/")
        {
            return Err("DuckDroid2 签名通道只允许原生 API 和公开房间查询".into());
        }
        let mut input = String::new();
        for (name, value) in headers {
            let value = value.to_str().map_err(|_| "签名请求头不是有效文本")?;
            if value.contains(['\r', '\n']) {
                return Err("签名请求头包含换行".into());
            }
            input.push_str(name.as_str());
            input.push_str("\r\n");
            input.push_str(value);
            input.push_str("\r\n");
        }
        if url.as_str().len() > 65536 || input.len() > 256 * 1024 {
            return Err("DuckDroid2 签名输入过大".into());
        }
        let ticket = headers
            .get("x-ss-req-ticket")
            .ok_or("DuckDroid2 签名缺少毫秒请求票据")?
            .to_str()
            .map_err(|_| "毫秒请求票据无效")?;
        if ticket.is_empty() || !ticket.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("毫秒请求票据格式无效".into());
        }
        self.sign_at(&path, url.as_str(), &input, ticket)
    }

    #[cfg(unix)]
    fn sign_at(
        &self,
        path: &str,
        url: &str,
        headers: &str,
        ticket: &str,
    ) -> Result<HeaderMap, String> {
        for attempt in 0..120 {
            let id = format!("douyin-{}-{attempt}", std::process::id());
            let mut request = json!({"version":1,"id":id,"app":"dy7","op":"sign",
                "params":{"url":url,"headers":headers,"wall_time_ms":ticket}});
            if let Some(session) = self.session.borrow().as_deref() {
                request["session"] = json!({"id":session});
            }
            let response = exchange(path, &request)?;
            if response["ok"] == true {
                let instance = response["instance"]
                    .as_str()
                    .filter(|value| value.len() <= 256 && !value.chars().any(char::is_control))
                    .ok_or("DuckDroid2 返回了无效实例标识")?;
                *self.session.borrow_mut() = Some(instance.to_owned());
                return signed_headers(&response);
            }
            let code = response["error"]["code"].as_str().unwrap_or("UNKNOWN");
            if code != "OVERLOADED" {
                let code = safe_code(code);
                return Err(format!("DuckDroid2 签名失败（{code}）"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("等待 DuckDroid2 实例就绪超时".into())
    }

    #[cfg(not(unix))]
    fn sign_at(
        &self,
        _path: &str,
        _url: &str,
        _headers: &str,
        _ticket: &str,
    ) -> Result<HeaderMap, String> {
        Err("DuckDroid2 签名通道当前仅支持 Unix 系统".into())
    }
}

#[cfg(unix)]
fn exchange(path: &str, request: &Value) -> Result<Value, String> {
    use std::os::unix::net::UnixStream;

    let body = serde_json::to_vec(request).map_err(|_| "无法编码 DuckDroid2 请求")?;
    if body.len() > MAX_FRAME {
        return Err("DuckDroid2 请求帧过大".into());
    }
    let mut stream =
        UnixStream::connect(path).map_err(|_| "无法连接 DuckDroid2，请先启动本地 dy7 签名服务")?;
    stream
        .set_read_timeout(Some(Duration::from_secs(130)))
        .map_err(|_| "无法设置 DuckDroid2 读取超时")?;
    stream
        .set_write_timeout(Some(Duration::from_secs(30)))
        .map_err(|_| "无法设置 DuckDroid2 写入超时")?;
    stream
        .write_all(&(body.len() as u32).to_be_bytes())
        .and_then(|()| stream.write_all(&body))
        .map_err(|_| "发送 DuckDroid2 签名请求失败")?;
    let mut prefix = [0; 4];
    stream
        .read_exact(&mut prefix)
        .map_err(|_| "DuckDroid2 响应长度不完整")?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAX_FRAME {
        return Err("DuckDroid2 响应帧过大".into());
    }
    let mut response = vec![0; length];
    stream
        .read_exact(&mut response)
        .map_err(|_| "DuckDroid2 响应不完整")?;
    serde_json::from_slice(&response).map_err(|_| "DuckDroid2 响应不是有效 JSON".into())
}

fn signed_headers(response: &Value) -> Result<HeaderMap, String> {
    let result = response["result"]
        .as_object()
        .ok_or("DuckDroid2 响应缺少签名结果")?;
    let mut headers = HeaderMap::new();
    for name in SIGNATURE_HEADERS {
        let value = result
            .get(name)
            .and_then(Value::as_str)
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 65536
                    && value.is_ascii()
                    && !value.chars().any(char::is_control)
            })
            .ok_or("DuckDroid2 返回的七组签名头不完整或无效")?;
        let mut value = HeaderValue::from_str(value).map_err(|_| "签名头格式无效")?;
        value.set_sensitive(true);
        headers.insert(name, value);
    }
    Ok(headers)
}

fn safe_code(code: &str) -> String {
    let filtered: String = code
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .take(64)
        .collect();
    if filtered.is_empty() {
        "UNKNOWN".into()
    } else {
        filtered
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    #[test]
    fn exchanges_bounded_frames_and_pins_the_native_instance() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dy7.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (sender, receiver) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut prefix = [0; 4];
                stream.read_exact(&mut prefix).unwrap();
                let mut body = vec![0; u32::from_be_bytes(prefix) as usize];
                stream.read_exact(&mut body).unwrap();
                let request: Value = serde_json::from_slice(&body).unwrap();
                sender.send(request).unwrap();
                let result = json!({"X-Perseus":"p","X-Medusa":"m","X-Argus":"a",
                    "X-Gorgon":"g","X-Helios":"h","X-Ladon":"l","X-Khronos":"1700000000"});
                let response = json!({"ok":true,"instance":"dy7-w0","result":result});
                let body = serde_json::to_vec(&response).unwrap();
                stream
                    .write_all(&(body.len() as u32).to_be_bytes())
                    .unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let client = DuckemuClient::default();
        let first = client.sign_at(
            path.to_str().unwrap(),
            "https://aweme.snssdk.com/aweme/v2/search_feed/?aid=1128",
            "cookie\r\nfixture\r\n",
            "1700000000000",
        );
        let second = client.sign_at(
            path.to_str().unwrap(),
            "https://aweme.snssdk.com/aweme/v2/search_feed/?aid=1128",
            "cookie\r\nfixture\r\n",
            "1700000000000",
        );
        assert_eq!(first.unwrap()["x-argus"], "a");
        assert_eq!(second.unwrap()["x-medusa"], "m");
        server.join().unwrap();
        let first: Value = receiver.recv().unwrap();
        let second: Value = receiver.recv().unwrap();
        assert_eq!(first["params"]["wall_time_ms"], "1700000000000");
        assert!(first.get("session").is_none());
        assert_eq!(second["session"]["id"], "dy7-w0");
    }

    #[test]
    fn rejects_non_json_and_incomplete_signature_results() {
        assert!(signed_headers(&json!({"result":{}})).is_err());
        assert!(signed_headers(&json!({"result":[] })).is_err());
        assert_eq!(safe_code("BAD\r\nsecret"), "BADsecret");
        assert_eq!(safe_code("\r\n"), "UNKNOWN");
        let client = DuckemuClient::default();
        let other_host = Url::parse("https://media.example/video").unwrap();
        assert!(client.sign(&other_host, &HeaderMap::new()).is_err());
    }
}
