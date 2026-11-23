//! Official OpenAPI transport, deliberately separate from the native session.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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
    pub(crate) fn with_base(base: Url) -> Result<Self, String> {
        let client = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
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
        read_response(request.send().map_err(|_| "OpenAPI 请求失败或超时")?)
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

    /// Upload a video, using a separate five-minute timeout for each upload request.
    pub fn upload_video(
        &self,
        token: &str,
        open_id: &str,
        file: &std::path::Path,
    ) -> Result<Value, String> {
        self.upload_video_with_timeout(token, open_id, file, Duration::from_secs(300))
    }

    /// Stream the file without buffering it in memory. Files above 50 MiB use
    /// Douyin's multipart protocol; this timeout applies separately to every step.
    pub fn upload_video_with_timeout(
        &self,
        token: &str,
        open_id: &str,
        file: &std::path::Path,
        timeout: Duration,
    ) -> Result<Value, String> {
        self.upload_video_inner(token, open_id, file, timeout, Chunking::DEFAULT)
    }

    /// Official v1 image upload. Only an already checked private JPEG snapshot
    /// is accepted; this never reuses native Creator credentials or VOD URIs.
    pub(crate) fn upload_cover(
        &self,
        token: &str,
        open_id: &str,
        cover: &crate::publish_cover::CoverSnapshot,
        timeout: Duration,
    ) -> Result<Value, String> {
        if token.trim().is_empty() || open_id.trim().is_empty() || timeout.is_zero() {
            return Err("封面上传需要有效 OAuth 身份和非零超时".into());
        }
        cover.receipt.validate()?;
        let part = Part::reader_with_length(cover.reader()?, cover.receipt.bytes)
            .file_name("cover.jpg")
            .mime_str("image/jpeg")
            .map_err(|_| "无法设置 JPEG 封面类型")?;
        let request = self
            .client
            .post(self.url("/api/douyin/v1/video/upload_image/")?)
            .query(&[("open_id", open_id)])
            .header("access-token", token)
            .timeout(timeout)
            .multipart(Form::new().part("image", part));
        read_response(
            request
                .send()
                .map_err(|_| "封面上传失败或超时；未创建作品")?,
        )
    }

    fn upload_video_inner(
        &self,
        token: &str,
        open_id: &str,
        path: &std::path::Path,
        timeout: Duration,
        chunking: Chunking,
    ) -> Result<Value, String> {
        if token.trim().is_empty() || open_id.trim().is_empty() {
            return Err("投稿上传需要 OAuth accessToken 和 openId".into());
        }
        if timeout.is_zero() {
            return Err("投稿上传超时必须大于 0".into());
        }
        // Keep one opened file throughout the upload so a path replacement cannot
        // silently switch a later part to a different file.
        let mut file = File::open(path).map_err(|_| "无法打开投稿视频文件")?;
        let metadata = file.metadata().map_err(|_| "无法读取投稿视频文件信息")?;
        if !metadata.is_file() {
            return Err("投稿路径必须是文件".into());
        }
        let size = metadata.len();
        if size == 0 || size > MAX_VIDEO_BYTES {
            return Err("投稿视频文件大小必须在 1 字节到 4 GiB 之间".into());
        }
        let name = path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("video.mp4")
            .to_owned();
        let request = |endpoint| {
            self.url(endpoint).map(|url| {
                self.client
                    .post(url)
                    .query(&[("open_id", open_id)])
                    .header("access-token", token)
                    .timeout(timeout)
            })
        };
        if size <= chunking.threshold {
            let part = Part::reader_with_length(file.take(size), size).file_name(name);
            return send_upload(
                request("/api/douyin/v1/video/upload_video/")?
                    .multipart(Form::new().part("video", part)),
            );
        }

        let initialized = send_upload(
            request("/api/douyin/v1/video/init_video_part_upload/")?
                .header("content-type", "application/json")
                .body(""),
        )?;
        let upload_id = initialized
            .pointer("/data/upload_id")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or("分片上传初始化响应缺少 upload_id")?;
        let mut offset = 0;
        let mut part_number = 1u64;
        while offset < size {
            let remaining = size - offset;
            // The documented minimum also applies to the tail: combine a short
            // tail with the preceding chunk instead of sending a tiny final part.
            let part_size =
                if remaining <= chunking.size || remaining - chunking.size < chunking.minimum {
                    remaining
                } else {
                    chunking.size
                };
            file.seek(SeekFrom::Start(offset))
                .map_err(|_| "无法定位投稿视频分片")?;
            let reader = file
                .try_clone()
                .map_err(|_| "无法读取投稿视频分片")?
                .take(part_size);
            let part = Part::reader_with_length(reader, part_size).file_name(name.clone());
            send_upload(
                request("/api/douyin/v1/video/upload_video_part/")?
                    .query(&[
                        ("upload_id", upload_id.to_owned()),
                        ("part_number", part_number.to_string()),
                    ])
                    .multipart(Form::new().part("video", part)),
            )
            .map_err(|error| format!("视频分片 {part_number} 上传失败：{error}"))?;
            offset += part_size;
            part_number += 1;
        }
        send_upload(
            request("/api/douyin/v1/video/complete_video_part_upload/")?
                .query(&[("upload_id", upload_id)])
                .header("content-type", "application/json")
                .body(""),
        )
    }
}

pub const MAX_VIDEO_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Chunking {
    threshold: u64,
    size: u64,
    minimum: u64,
}

impl Chunking {
    const DEFAULT: Self = Self {
        threshold: 50 * 1024 * 1024,
        size: 20 * 1024 * 1024,
        minimum: 5 * 1024 * 1024,
    };
}

fn send_upload(request: reqwest::blocking::RequestBuilder) -> Result<Value, String> {
    read_response(request.send().map_err(|_| "投稿视频上传失败或超时")?)
}

fn read_response(response: reqwest::blocking::Response) -> Result<Value, String> {
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
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "OpenAPI 返回非 JSON 内容")?;
    validate_response(&value)?;
    Ok(value)
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
    struct CapturedRequest {
        target: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    struct Reply {
        status: u16,
        body: String,
        location: Option<String>,
    }

    fn ok_reply(body: Value) -> Reply {
        Reply {
            status: 200,
            body: body.to_string(),
            location: None,
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> CapturedRequest {
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            head.push(byte[0]);
            assert!(head.len() < 16 * 1024);
        }
        let head = String::from_utf8(head).unwrap();
        let mut lines = head.lines();
        let first = lines.next().unwrap();
        assert!(first.starts_with("POST "));
        let target = first.split_whitespace().nth(1).unwrap().to_owned();
        let headers: BTreeMap<_, _> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let length: usize = headers["content-length"].parse().unwrap();
        assert!(length < 1024 * 1024);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        CapturedRequest {
            target,
            headers,
            body,
        }
    }

    fn accept_request(server: &TcpListener) -> std::net::TcpStream {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            match server.accept() {
                Ok((stream, _)) => return stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "missing upload request"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("accept failed: {error}"),
            }
        }
    }

    fn upload_fixture(
        replies: Vec<Reply>,
    ) -> (
        OpenApiClient,
        std::thread::JoinHandle<(Vec<CapturedRequest>, TcpListener)>,
    ) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let base = Url::parse(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let mut stream = accept_request(&server);
                requests.push(read_request(&mut stream));
                write!(
                    stream,
                    "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len()
                )
                .unwrap();
                if let Some(location) = reply.location {
                    write!(stream, "Location: {location}\r\n").unwrap();
                }
                write!(stream, "\r\n{}", reply.body).unwrap();
            }
            (requests, server)
        });
        (OpenApiClient::with_base(base).unwrap(), worker)
    }

    fn fixture_file(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(bytes).unwrap();
        file
    }

    fn upload_query(request: &CapturedRequest) -> (String, BTreeMap<String, String>) {
        let url = Url::parse(&format!("http://fixture{}", request.target)).unwrap();
        (
            url.path().to_owned(),
            url.query_pairs().into_owned().collect(),
        )
    }

    fn multipart_video(request: &CapturedRequest) -> &[u8] {
        let content_type = &request.headers["content-type"];
        assert!(content_type.starts_with("multipart/form-data; boundary="));
        let boundary = content_type.split("boundary=").nth(1).unwrap();
        let prefix = format!("--{boundary}\r\n");
        let suffix = format!("\r\n--{boundary}--\r\n");
        assert!(request.body.starts_with(prefix.as_bytes()));
        assert!(request.body.ends_with(suffix.as_bytes()));
        let separator = request
            .body
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap();
        let part_headers = String::from_utf8_lossy(&request.body[..separator]).to_ascii_lowercase();
        assert!(part_headers.contains("content-disposition: form-data; name=\"video\"; filename="));
        &request.body[separator + 4..request.body.len() - suffix.len()]
    }

    fn assert_upload_credentials(request: &CapturedRequest) {
        assert_eq!(request.headers["access-token"], "fixture-token");
        assert!(!request.headers.contains_key("cookie"));
        assert!(!request.headers.contains_key("authorization"));
        assert!(!request.target.contains("fixture-token"));
        assert_eq!(upload_query(request).1["open_id"], "user/+ fixture");
    }

    fn assert_no_more_requests(server: TcpListener) {
        assert_eq!(
            server.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn small_video_upload_sends_sized_multipart_and_isolated_credentials() {
        let bytes = b"\0\0\0\x18ftypmp42fixture-video";
        let file = fixture_file(bytes);
        let uploaded = json!({"data":{"error_code":0,"video":{"video_id":"encrypted-video"}}});
        let (client, worker) = upload_fixture(vec![ok_reply(uploaded.clone())]);
        assert_eq!(
            client
                .upload_video("fixture-token", "user/+ fixture", file.path())
                .unwrap(),
            uploaded
        );
        let (requests, server) = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(
            upload_query(request).0,
            "/api/douyin/v1/video/upload_video/"
        );
        assert_upload_credentials(request);
        assert_eq!(multipart_video(request), bytes);
        assert_no_more_requests(server);
    }

    #[test]
    fn jpeg_cover_upload_uses_official_image_field_and_same_oauth_identity() {
        let bytes = b"\xff\xd8\xfffixture-jpeg\xff\xd9";
        let file = fixture_file(bytes);
        let cover = crate::publish_cover::CoverSnapshot::read(file.path()).unwrap();
        let response = json!({"data":{"error_code":0,"image":{"image_id":"encrypted-cover"}}});
        let (client, worker) = upload_fixture(vec![ok_reply(response.clone())]);
        let actual = client
            .upload_cover(
                "fixture-token",
                "user/+ fixture",
                &cover,
                Duration::from_secs(2),
            )
            .unwrap();
        assert_eq!(actual, response);
        let (requests, server) = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!(
            upload_query(request).0,
            "/api/douyin/v1/video/upload_image/"
        );
        assert_upload_credentials(request);
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"image\"; filename=\"cover.jpg\""));
        assert!(body.contains("image/jpeg"));
        assert!(
            request
                .body
                .windows(bytes.len())
                .any(|window| window == bytes)
        );
        assert!(!body.contains("name=\"video\""));
        assert_no_more_requests(server);
    }

    const TEST_CHUNKS: Chunking = Chunking {
        threshold: 8,
        size: 6,
        minimum: 2,
    };

    #[test]
    fn chunked_upload_preserves_order_bytes_and_merges_short_tail() {
        let bytes: Vec<u8> = (0..19).collect();
        let file = fixture_file(&bytes);
        let uploaded = json!({"data":{"error_code":0,"video":{"video_id":"encrypted-video"}}});
        let mut replies = vec![ok_reply(
            json!({"data":{"error_code":0,"upload_id":"upload/+ fixture"}}),
        )];
        replies.extend((0..3).map(|_| ok_reply(json!({"data":{"error_code":0}}))));
        replies.push(ok_reply(uploaded.clone()));
        let (client, worker) = upload_fixture(replies);
        assert_eq!(
            client
                .upload_video_inner(
                    "fixture-token",
                    "user/+ fixture",
                    file.path(),
                    Duration::from_secs(3),
                    TEST_CHUNKS,
                )
                .unwrap(),
            uploaded
        );
        let (requests, server) = worker.join().unwrap();
        assert_eq!(requests.len(), 5);
        assert_eq!(
            upload_query(&requests[0]).0,
            "/api/douyin/v1/video/init_video_part_upload/"
        );
        assert!(requests[0].body.is_empty());
        assert_eq!(requests[0].headers["content-type"], "application/json");
        let mut rebuilt = Vec::new();
        for (index, request) in requests[1..4].iter().enumerate() {
            assert_upload_credentials(request);
            let (path, query) = upload_query(request);
            assert_eq!(path, "/api/douyin/v1/video/upload_video_part/");
            assert_eq!(query["upload_id"], "upload/+ fixture");
            assert_eq!(query["part_number"], (index + 1).to_string());
            let payload = multipart_video(request);
            assert_eq!(payload.len(), [6, 6, 7][index]);
            rebuilt.extend_from_slice(payload);
        }
        assert_eq!(rebuilt, bytes);
        assert_eq!(
            upload_query(&requests[4]).0,
            "/api/douyin/v1/video/complete_video_part_upload/"
        );
        assert_eq!(
            upload_query(&requests[4]).1["upload_id"],
            "upload/+ fixture"
        );
        assert_eq!(requests[4].headers["content-type"], "application/json");
        assert!(requests[4].body.is_empty());
        for request in &requests {
            assert_upload_credentials(request);
        }
        assert_no_more_requests(server);
    }

    #[test]
    fn rejected_part_stops_before_later_parts_and_completion() {
        let file = fixture_file(&[1; 19]);
        let (client, worker) = upload_fixture(vec![
            ok_reply(json!({"data":{"error_code":0,"upload_id":"upload-id"}})),
            ok_reply(json!({"data":{"error_code":0}})),
            ok_reply(json!({"data":{"error_code":2114021,"description":"secret-token"}})),
        ]);
        let error = client
            .upload_video_inner(
                "fixture-token",
                "user/+ fixture",
                file.path(),
                Duration::from_secs(3),
                TEST_CHUNKS,
            )
            .unwrap_err();
        assert!(error.contains("分片 2"));
        assert!(error.contains("2114021"));
        assert!(!error.contains("secret-token"));
        let (requests, server) = worker.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert_no_more_requests(server);
    }

    #[test]
    fn missing_upload_id_stops_before_sending_video_bytes() {
        let file = fixture_file(&[1; 19]);
        let (client, worker) = upload_fixture(vec![ok_reply(json!({"data":{"error_code":0}}))]);
        assert!(
            client
                .upload_video_inner(
                    "fixture-token",
                    "user/+ fixture",
                    file.path(),
                    Duration::from_secs(3),
                    TEST_CHUNKS,
                )
                .unwrap_err()
                .contains("upload_id")
        );
        let (requests, server) = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].body.is_empty());
        assert_no_more_requests(server);
    }

    #[test]
    fn upload_redirect_never_forwards_file_or_credentials() {
        let destination = TcpListener::bind("127.0.0.1:0").unwrap();
        destination.set_nonblocking(true).unwrap();
        let (client, worker) = upload_fixture(vec![Reply {
            status: 307,
            body: String::new(),
            location: Some(format!(
                "http://{}/stolen",
                destination.local_addr().unwrap()
            )),
        }]);
        let file = fixture_file(b"private video");
        assert!(
            client
                .upload_video("fixture-token", "user/+ fixture", file.path())
                .unwrap_err()
                .contains("307")
        );
        let (requests, server) = worker.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert_no_more_requests(server);
        assert_no_more_requests(destination);
    }

    #[test]
    fn upload_uses_its_configured_request_timeout() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        server.set_nonblocking(true).unwrap();
        let base = Url::parse(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let worker = std::thread::spawn(move || {
            let mut stream = accept_request(&server);
            let request = read_request(&mut stream);
            std::thread::sleep(Duration::from_millis(300));
            let body = r#"{"data":{"error_code":0,"video":{"video_id":"late"}}}"#;
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            request
        });
        let file = fixture_file(b"timeout fixture");
        assert!(
            OpenApiClient::with_base(base)
                .unwrap()
                .upload_video_with_timeout(
                    "fixture-token",
                    "user/+ fixture",
                    file.path(),
                    Duration::from_millis(30),
                )
                .unwrap_err()
                .contains("上传失败或超时")
        );
        assert_upload_credentials(&worker.join().unwrap());
    }
}
