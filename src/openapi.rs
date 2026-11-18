//! Official OpenAPI transport, deliberately separate from the native session.
use std::collections::BTreeMap;
use std::io::Read;
use std::time::Duration;

use reqwest::{
    Method,
    blocking::{
        Client,
        multipart::{Form, Part},
    },
    header::{HeaderName, HeaderValue},
    redirect::Policy,
};
use serde_json::Value;
use url::Url;

pub struct OpenApiClient {
    base: Url,
    client: Client,
}

pub struct Request<'a> {
    pub method: Method,
    pub path: &'a str,
    pub token: Option<&'a str>,
    pub params: Vec<(String, String)>,
    pub form: Vec<(String, String)>,
    pub json: Option<Value>,
    pub headers: BTreeMap<String, String>,
}

impl<'a> Request<'a> {
    pub fn new(method: Method, path: &'a str) -> Self {
        Self {
            method,
            path,
            token: None,
            params: vec![],
            form: vec![],
            json: None,
            headers: BTreeMap::new(),
        }
    }
}

impl OpenApiClient {
    pub fn new() -> Result<Self, String> {
        let base = Url::parse("https://open.douyin.com/").map_err(|_| "OpenAPI 地址无效")?;
        Self::with_base(base)
    }
    fn with_base(base: Url) -> Result<Self, String> {
        let client = Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "无法创建 OpenAPI 客户端")?;
        Ok(Self { base, client })
    }
    fn url(&self, path: &str) -> Result<Url, String> {
        if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#', '\\']) {
            return Err("OpenAPI 只接受同源绝对路径，查询参数请使用 --param".into());
        }
        let url = self.base.join(path).map_err(|_| "OpenAPI 路径无效")?;
        if url.origin() != self.base.origin()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("拒绝跨域 OpenAPI 请求".into());
        }
        Ok(url)
    }
    pub fn request(&self, spec: Request<'_>) -> Result<Value, String> {
        let url = self.url(spec.path)?;
        if spec.json.is_some() && !spec.form.is_empty() {
            return Err("JSON 和表单不能同时发送".into());
        }
        let mut request = self.client.request(spec.method, url).query(&spec.params);
        if let Some(token) = spec.token.filter(|v| !v.is_empty()) {
            request = request.header("access-token", token);
        }
        for (name, value) in spec.headers {
            let name =
                HeaderName::from_bytes(name.as_bytes()).map_err(|_| "OpenAPI 请求头名称无效")?;
            if matches!(
                name.as_str(),
                "host"
                    | "cookie"
                    | "authorization"
                    | "proxy-authorization"
                    | "access-token"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
            ) {
                return Err("自定义请求头不能覆盖认证、主机或传输控制字段".into());
            }
            let value = HeaderValue::from_str(&value).map_err(|_| "OpenAPI 请求头值无效")?;
            request = request.header(name, value);
        }
        if let Some(body) = spec.json {
            request = request.json(&body);
        } else if !spec.form.is_empty() {
            request = request.form(&spec.form);
        }
        let response = request.send().map_err(|_| "OpenAPI 请求失败或超时")?;
        if !response.status().is_success() {
            return Err(format!("OpenAPI 返回 HTTP {}", response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取 OpenAPI 响应失败")?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("OpenAPI 响应超过大小限制".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "OpenAPI 返回非 JSON 内容")?;
        validate_response(&value)?;
        Ok(value)
    }
    pub fn authorize_url(
        &self,
        key: &str,
        redirect: &str,
        scopes: &[String],
        state: &str,
    ) -> Result<String, String> {
        if key.trim().is_empty()
            || scopes.is_empty()
            || scopes
                .iter()
                .any(|s| s.trim().is_empty() || s.contains(','))
        {
            return Err("OAuth 需要 client-key 和有效 scope 列表".into());
        }
        let callback = Url::parse(redirect).map_err(|_| "OAuth 回调地址无效")?;
        if !matches!(callback.scheme(), "https" | "http")
            || !callback.username().is_empty()
            || callback.password().is_some()
            || callback.fragment().is_some()
        {
            return Err("OAuth 回调必须是无用户信息和 fragment 的 HTTP(S) URL".into());
        }
        let mut url = self.url("/platform/oauth/connect/")?;
        url.query_pairs_mut().extend_pairs([
            ("client_key", key),
            ("response_type", "code"),
            ("scope", &scopes.join(",")),
            ("redirect_uri", redirect),
            ("state", state),
        ]);
        Ok(url.into())
    }
    pub fn token(&self, path: &str, fields: &[(&str, &str)]) -> Result<Value, String> {
        let mut request = Request::new(Method::POST, path);
        request.form = fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self.request(request)
    }

    pub fn upload_video(
        &self,
        token: &str,
        open_id: &str,
        file: &std::path::Path,
    ) -> Result<Value, String> {
        let url = self.url("/api/douyin/v1/video/upload_video/")?;
        let part = Part::file(file)
            .map_err(|_| "无法打开投稿视频文件")?
            .file_name(
                file.file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("video.mp4")
                    .to_owned(),
            );
        let form = Form::new().part("video", part);
        let response = self
            .client
            .post(url)
            .query(&[("open_id", open_id)])
            .header("access-token", token)
            .multipart(form)
            .send()
            .map_err(|_| "投稿视频上传失败或超时")?;
        if !response.status().is_success() {
            return Err(format!("OpenAPI 返回 HTTP {}", response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        response
            .take(16 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "读取投稿上传响应失败")?;
        if bytes.len() > 16 * 1024 * 1024 {
            return Err("投稿上传响应超过大小限制".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "投稿上传返回非 JSON 内容")?;
        validate_response(&value)?;
        Ok(value)
    }
}

fn validate_response(value: &Value) -> Result<(), String> {
    if !value.is_object() {
        return Err("OpenAPI 响应不是对象".into());
    }
    let mut found = false;
    for container in [value, &value["data"], &value["extra"]] {
        for key in ["err_no", "error_code", "status_code"] {
            if let Some(code) = container.get(key) {
                let code = crate::net::integer(code).ok_or("OpenAPI 错误码格式无效")?;
                found = true;
                if code != 0 {
                    return Err(format!("OpenAPI 报告错误码 {code}"));
                }
            }
        }
    }
    if !found && value["message"] != "success" {
        return Err("OpenAPI 未返回可识别的业务状态".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use std::net::TcpListener;
    #[test]
    fn error_bodies_never_escape_into_errors() {
        assert!(validate_response(&json!({"data":{"error_code":"0"}})).is_ok());
        let error =
            validate_response(&json!({"data":{"error_code":10008,"description":"secret-token"}}))
                .unwrap_err();
        assert!(error.contains("10008"));
        assert!(!error.contains("secret"));
        assert!(validate_response(&json!({"data":{"error_code":"bad"}})).is_err());
        assert!(validate_response(&json!({"result":true})).is_err());
    }
    #[test]
    fn token_is_only_a_header_and_redirects_are_not_followed() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let text = String::from_utf8(request).unwrap();
            assert!(text.to_lowercase().contains("access-token: fixture-token"));
            assert!(!text.lines().next().unwrap().contains("fixture-token"));
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://evil.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let client = OpenApiClient::with_base(base).unwrap();
        let mut spec = Request::new(Method::GET, "/oauth/userinfo/");
        spec.token = Some("fixture-token");
        assert!(client.request(spec).unwrap_err().contains("302"));
        thread.join().unwrap();
    }
    #[test]
    fn same_origin_paths_and_authorization_encoding_are_enforced() {
        let client = OpenApiClient::new().unwrap();
        for path in [
            "https://evil.invalid/x",
            "//evil.invalid/x",
            "/x?token=secret",
            "/\\evil.invalid/x",
        ] {
            assert!(client.url(path).is_err());
        }
        let url = client
            .authorize_url(
                "key",
                "http://127.0.0.1:8787/callback",
                &["user_info".into()],
                "random-state",
            )
            .unwrap();
        let parsed = Url::parse(&url).unwrap();
        let params: BTreeMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(params["state"], "random-state");
        assert_eq!(params["redirect_uri"], "http://127.0.0.1:8787/callback");
    }
}
