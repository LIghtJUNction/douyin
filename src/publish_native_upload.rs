//! Native creator VOD upload. This module never creates a Douyin post.
//!
//! Protocol references (read for field contracts, not copied as implementation):
//! - cv-cat/DouYin_Spider at fc8b9c2490a9561265425c26058290f01c064eda,
//!   dy_apis/douyin_creator_api.py and utils/imagex_sign.py.
//! - AWS Signature Version 4 canonical request and HMAC key derivation.
//!
//! Creator authentication belongs to the caller. Its cookie never enters the
//! upload client. STS credentials and storage authorization are kept private.
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use md5::{Digest as _, Md5};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use ring::{
    digest, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::Serialize;
use serde_json::{Value, json};
use url::Url;

const VOD_ORIGIN: &str = "https://vod.bytedanceapi.com/";
const CREATOR_ORIGIN: &str = "https://creator.douyin.com";
const REGION: &str = "cn-north-1";
const SERVICE: &str = "vod";
const API_VERSION: &str = "2020-11-19";
const MIB: u64 = 1024 * 1024;
const MAX_JSON: u64 = 2 * MIB;

pub(crate) struct UploadCredentials {
    access_key: String,
    secret_key: String,
    session_token: String,
    server_at_receipt: u64,
    received_at: Instant,
    expires_at: Option<u64>,
    #[cfg(test)]
    frozen_time: Option<u64>,
}

impl std::fmt::Debug for UploadCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UploadCredentials { [redacted] }")
    }
}

impl UploadCredentials {
    fn now(&self) -> Result<u64, String> {
        #[cfg(test)]
        if let Some(now) = self.frozen_time {
            return self.check_expiration(now);
        }
        let now = self
            .server_at_receipt
            .saturating_add(self.received_at.elapsed().as_secs());
        self.check_expiration(now)
    }

    fn check_expiration(&self, now: u64) -> Result<u64, String> {
        if self
            .expires_at
            .is_some_and(|expiry| expiry <= now.saturating_add(15))
        {
            return Err("native/upload/auth: 临时上传凭据已过期或即将过期，请重新获取".into());
        }
        Ok(now)
    }
}

/// Parse the actual auth/v5 response. No credentials are serialized or logged.
pub(crate) fn parse_upload_auth(value: &Value) -> Result<UploadCredentials, String> {
    if value["status_code"].as_i64().is_some_and(|code| code != 0) {
        return Err("native/upload/auth: 创作者中心拒绝临时上传授权".into());
    }
    let auth_text = value["auth"]
        .as_str()
        .filter(|s| s.len() <= 131072)
        .ok_or("native/upload/auth: 响应缺少 auth JSON 字符串")?;
    let auth: Value = serde_json::from_str(auth_text)
        .map_err(|_| "native/upload/auth: 临时授权内容不是有效 JSON")?;
    let access_key = secret_field(&auth, &["AccessKeyID", "AccessKeyId"], 4096)?;
    if access_key
        .bytes()
        .any(|b| !b.is_ascii_alphanumeric() && !b"-_".contains(&b))
    {
        return Err("native/upload/auth: AccessKeyID 格式无效".into());
    }
    let secret_key = secret_field(&auth, &["SecretAccessKey"], 8192)?;
    let session_token = secret_field(&auth, &["SessionToken"], 65536)?;
    let server_at_receipt = optional_time(&auth["CurrentTime"])?.unwrap_or(unix_now()?);
    let expires_at = optional_time(&auth["ExpiredTime"])?;
    let result = UploadCredentials {
        access_key,
        secret_key,
        session_token,
        server_at_receipt,
        received_at: Instant::now(),
        expires_at,
        #[cfg(test)]
        frozen_time: None,
    };
    result.now()?;
    Ok(result)
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct UploadedVideo {
    pub video_id: String,
    pub poster_uri: String,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub size: u64,
    pub format: Option<String>,
    /// MD5 of the bytes actually uploaded, checked against the gateway when present.
    pub md5: String,
    /// SHA-256 of the uploaded bytes, for comparison with the caller's review record.
    pub sha256: String,
}

/// Apply -> transfer bytes -> fresh creator STS -> Commit(GetMeta, Snapshot).
///
/// The callback must make a newly authenticated auth/v5 request each time.
/// Upload failure stops before Commit; this function never calls an aweme API.
pub(crate) fn upload_video<F>(
    path: &Path,
    user_id: &str,
    timeout: Duration,
    refresh_auth: F,
) -> Result<UploadedVideo, String>
where
    F: FnMut() -> Result<UploadCredentials, String>,
{
    UploadEngine::production(timeout)?.upload(path, user_id, refresh_auth)
}

struct UploadEngine {
    client: Client,
    #[cfg(test)]
    fixture_origin: Option<Url>,
}

impl UploadEngine {
    fn production(timeout: Duration) -> Result<Self, String> {
        let timeout = timeout
            .max(Duration::from_secs(1))
            .min(Duration::from_secs(900));
        let client = Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout.min(Duration::from_secs(20)))
            .timeout(timeout)
            .user_agent(concat!("douyin-cli/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|_| "native/upload: 无法创建 HTTPS 上传客户端")?;
        Ok(Self {
            client,
            #[cfg(test)]
            fixture_origin: None,
        })
    }

    fn upload<F>(
        &self,
        path: &Path,
        user_id: &str,
        mut refresh_auth: F,
    ) -> Result<UploadedVideo, String>
    where
        F: FnMut() -> Result<UploadCredentials, String>,
    {
        if user_id.is_empty()
            || user_id == "0"
            || user_id.len() > 32
            || !user_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("native/upload: 必须提供已核验的创作者数字账号 ID".into());
        }
        let mut file = File::open(path).map_err(|_| "native/upload/file: 无法打开视频")?;
        let before = file
            .metadata()
            .map_err(|_| "native/upload/file: 无法读取视频元数据")?;
        let size = before.len();
        if !before.is_file() || size == 0 {
            return Err("native/upload/file: 视频必须是非空普通文件".into());
        }
        let sts = refresh_auth().map_err(|_| "native/upload/auth: 无法取得本次上传的临时授权")?;
        let node = self.apply(&sts, size, user_id)?;
        // The apply STS is intentionally not reused for the post-transfer Commit.
        drop(sts);
        let mut fingerprints = Fingerprints::new();
        let plan = transfer_plan(size);
        if plan.direct {
            let bytes = read_chunk(&mut file, size as usize)?;
            fingerprints.update(&bytes);
            let crc = crc32_hex(&bytes);
            let url = self.storage_url(&node, &[])?;
            self.storage_post(&node, user_id, url, Some(&crc), bytes, "transfer")?;
        } else {
            let upload_id = if let Some(id) = node.upload_id.clone() {
                id
            } else {
                let url = self.storage_url(
                    &node,
                    &[("uploadmode", "part".into()), ("phase", "init".into())],
                )?;
                let response = self.storage_post(&node, user_id, url, None, Vec::new(), "init")?;
                response
                    .pointer("/data/uploadid")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty() && s.len() <= 4096)
                    .ok_or("native/upload/init: 分片初始化缺少 uploadid")?
                    .to_owned()
            };
            let mut offset = 0u64;
            let mut checksums = Vec::new();
            while offset < size {
                let count = (size - offset).min(plan.part_size) as usize;
                let bytes = read_chunk(&mut file, count)?;
                fingerprints.update(&bytes);
                let crc = crc32_hex(&bytes);
                let number = checksums.len() + 1;
                let url = self.storage_url(
                    &node,
                    &[
                        ("uploadid", upload_id.clone()),
                        ("part_number", number.to_string()),
                        ("phase", "transfer".into()),
                        ("part_offset", offset.to_string()),
                    ],
                )?;
                self.storage_post(&node, user_id, url, Some(&crc), bytes, "transfer")?;
                checksums.push(crc);
                offset += count as u64;
            }
            let body = checksums
                .iter()
                .enumerate()
                .map(|(i, crc)| format!("{}:{crc}", i + 1))
                .collect::<Vec<_>>()
                .join(",")
                .into_bytes();
            let url = self.storage_url(
                &node,
                &[
                    ("uploadmode", "part".into()),
                    ("phase", "finish".into()),
                    ("uploadid", upload_id),
                ],
            )?;
            self.storage_post(&node, user_id, url, None, body, "finish")?;
        }
        let after = file
            .metadata()
            .map_err(|_| "native/upload/file: 无法再次核对视频元数据")?;
        if after.len() != size || before.modified().ok() != after.modified().ok() {
            return Err("native/upload/file: 视频在上传期间发生变化，已停止提交上传会话".into());
        }
        let (md5, sha256) = fingerprints.finish();
        let commit_sts = refresh_auth().map_err(
            |_| "native/upload/auth: 字节上传完成，但无法重新取得 Commit 授权；未创建作品",
        )?;
        self.commit(&commit_sts, &node, user_id, size, md5, sha256)
    }

    fn apply(
        &self,
        sts: &UploadCredentials,
        size: u64,
        user_id: &str,
    ) -> Result<UploadNode, String> {
        let query = vec![
            ("Action", "ApplyUploadInner".into()),
            ("Version", API_VERSION.into()),
            ("SpaceName", "aweme".into()),
            ("FileType", "video".into()),
            ("IsInner", "1".into()),
            ("FileSize", size.to_string()),
            ("app_id", "2906".into()),
            ("user_id", user_id.into()),
            ("s", self.random_parameter()?),
        ];
        let value = self.gateway(sts, "GET", &query, Vec::new(), "apply")?;
        parse_upload_node(&value)
    }

    fn commit(
        &self,
        sts: &UploadCredentials,
        node: &UploadNode,
        user_id: &str,
        size: u64,
        md5: String,
        sha256: String,
    ) -> Result<UploadedVideo, String> {
        let query = vec![
            ("Action", "CommitUploadInner".into()),
            ("Version", API_VERSION.into()),
            ("SpaceName", "aweme".into()),
            ("app_id", "2906".into()),
            ("user_id", user_id.into()),
        ];
        let body = serde_json::to_vec(&json!({
            "SessionKey":node.session_key,
            "Functions":[{"name":"GetMeta"},{"name":"Snapshot","input":{"SnapshotTime":0}}]
        }))
        .map_err(|_| "native/upload/commit: 无法编码上传提交请求")?;
        let value = self.gateway(sts, "POST", &query, body, "commit")?;
        parse_uploaded_video(&value, size, md5, sha256)
    }

    fn gateway(
        &self,
        sts: &UploadCredentials,
        method: &str,
        query: &[(&str, String)],
        body: Vec<u8>,
        stage: &str,
    ) -> Result<Value, String> {
        let signed = sign_vod(sts, method, query, &body)?;
        let mut url = Url::parse(VOD_ORIGIN).map_err(|_| "native/upload: VOD 固定地址无效")?;
        url.set_query(Some(&canonical_query(query)));
        let url = self.test_route(url);
        let mut request = if method == "GET" {
            self.client.get(url)
        } else {
            self.client.post(url)
        };
        request = request
            .headers(signed)
            .header("origin", CREATOR_ORIGIN)
            .header("referer", format!("{CREATOR_ORIGIN}/"));
        if method == "POST" {
            request = request
                .header("content-type", "text/plain;charset=UTF-8")
                .body(body);
        }
        let response = request
            .send()
            .map_err(|_| format!("native/upload/{stage}: VOD 请求失败或超时"))?;
        let value = read_json(response, stage)?;
        if let Some(code) = value
            .pointer("/ResponseMetadata/Error/Code")
            .or_else(|| value.pointer("/Error/Code"))
        {
            return Err(format!(
                "native/upload/{stage}: VOD 拒绝请求（{}）",
                safe_code(code)
            ));
        }
        Ok(value)
    }

    fn storage_url(&self, node: &UploadNode, query: &[(&str, String)]) -> Result<Url, String> {
        let mut url = node.url.clone();
        if !query.is_empty() {
            url.set_query(Some(&canonical_query(query)));
        }
        Ok(self.test_route(url))
    }

    fn storage_post(
        &self,
        node: &UploadNode,
        user_id: &str,
        url: Url,
        crc: Option<&str>,
        body: Vec<u8>,
        stage: &str,
    ) -> Result<Value, String> {
        let mut request = self
            .client
            .post(url)
            .headers(node.extra_headers.clone())
            .header("authorization", node.authorization.clone())
            .header("x-storage-u", encode_component(user_id))
            .header("origin", CREATOR_ORIGIN)
            .header("referer", format!("{CREATOR_ORIGIN}/"))
            .header("content-type", "application/octet-stream");
        if let Some(crc) = crc {
            request = request.header("content-crc32", crc);
        }
        let response = request
            .body(body)
            .send()
            .map_err(|_| format!("native/upload/{stage}: 存储上传失败或超时"))?;
        let value = read_json(response, stage)?;
        if value["code"].as_i64() != Some(2000) {
            return Err(format!(
                "native/upload/{stage}: 存储节点未确认成功（{}）",
                safe_code(&value["code"])
            ));
        }
        Ok(value)
    }

    fn random_parameter(&self) -> Result<String, String> {
        #[cfg(test)]
        if self.fixture_origin.is_some() {
            return Ok("fixture".into());
        }
        let mut random = [0u8; 12];
        SystemRandom::new()
            .fill(&mut random)
            .map_err(|_| "native/upload/apply: 无法生成上传随机参数")?;
        Ok(hex(&random))
    }

    fn test_route(&self, url: Url) -> Url {
        #[cfg(test)]
        if let Some(ref fixture) = self.fixture_origin {
            let mut routed = fixture.clone();
            routed.set_path(url.path());
            routed.set_query(url.query());
            return routed;
        }
        url
    }

    #[cfg(test)]
    fn fixture(origin: Url) -> Self {
        assert_eq!(origin.scheme(), "http");
        assert_eq!(origin.host_str(), Some("127.0.0.1"));
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        Self {
            client,
            fixture_origin: Some(origin),
        }
    }
}

struct UploadNode {
    url: Url,
    authorization: HeaderValue,
    session_key: String,
    upload_id: Option<String>,
    extra_headers: HeaderMap,
}

fn parse_upload_node(value: &Value) -> Result<UploadNode, String> {
    let node = value
        .pointer("/Result/InnerUploadAddress/UploadNodes/0")
        .ok_or("native/upload/apply: VOD 未返回上传节点")?;
    let store = node
        .pointer("/StoreInfos/0")
        .ok_or("native/upload/apply: VOD 未返回存储地址")?;
    let host = node["UploadHost"]
        .as_str()
        .ok_or("native/upload/apply: 上传节点缺少主机")?;
    let uri = store["StoreUri"]
        .as_str()
        .ok_or("native/upload/apply: 上传节点缺少资源路径")?;
    let url = validated_storage_url(host, uri)?;
    let mut authorization = store["Auth"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536)
        .and_then(|s| HeaderValue::from_str(s).ok())
        .ok_or("native/upload/apply: 存储授权格式无效")?;
    authorization.set_sensitive(true);
    let session_key = node["SessionKey"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 65536)
        .ok_or("native/upload/apply: 缺少上传会话键")?
        .to_owned();
    let upload_id = store["UploadID"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    if upload_id.as_ref().is_some_and(|id| id.len() > 4096) {
        return Err("native/upload/apply: 分片会话标识过长".into());
    }
    let extra_headers = parse_upload_headers(node.get("UploadHeader"))?;
    Ok(UploadNode {
        url,
        authorization,
        session_key,
        upload_id,
        extra_headers,
    })
}

fn validated_storage_url(host: &str, uri: &str) -> Result<Url, String> {
    if host.is_empty()
        || host.len() > 253
        || host
            .bytes()
            .any(|b| !b.is_ascii_alphanumeric() && !b".-".contains(&b))
    {
        return Err("native/upload/apply: 上传主机格式无效".into());
    }
    let host = host.to_ascii_lowercase();
    const SUFFIXES: [&str; 6] = [
        "snssdk.com",
        "bytedancevod.com",
        "douyinvod.com",
        "douyin.com",
        "bytecdn.cn",
        "volces.com",
    ];
    if !SUFFIXES
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
    {
        return Err("native/upload/apply: VOD 返回未识别的上传域，已停止转发存储授权".into());
    }
    if uri.is_empty()
        || uri.len() > 8192
        || uri.starts_with('/')
        || uri
            .bytes()
            .any(|b| b <= 0x20 || b == 0x7f || b"?#\\".contains(&b))
        || uri.split('/').any(|s| matches!(s, "." | ".."))
    {
        return Err("native/upload/apply: 存储资源路径无效".into());
    }
    let url = Url::parse(&format!("https://{host}/upload/v1/{uri}"))
        .map_err(|_| "native/upload/apply: 无法构建存储地址")?;
    if url.scheme() != "https"
        || url.host_str() != Some(host.as_str())
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/upload/v1/")
    {
        return Err("native/upload/apply: 存储地址不符合上传协议".into());
    }
    Ok(url)
}

fn parse_upload_headers(value: Option<&Value>) -> Result<HeaderMap, String> {
    let mut entries = Vec::new();
    match value {
        None | Some(Value::Null) => {}
        Some(Value::Object(map)) => {
            for (k, v) in map {
                entries.push((
                    k.as_str(),
                    v.as_str()
                        .ok_or("native/upload/apply: 上传扩展头不是字符串")?,
                ));
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                entries.push((
                    item["Key"]
                        .as_str()
                        .ok_or("native/upload/apply: 上传扩展头缺少 Key")?,
                    item["Value"]
                        .as_str()
                        .ok_or("native/upload/apply: 上传扩展头缺少 Value")?,
                ));
            }
        }
        _ => return Err("native/upload/apply: 上传扩展头格式不受支持".into()),
    }
    if entries.len() > 64 {
        return Err("native/upload/apply: 上传扩展头数量过多".into());
    }
    let mut headers = HeaderMap::new();
    for (name, value) in entries {
        let lower = name.to_ascii_lowercase();
        if !lower.starts_with("x-")
            || lower.starts_with("x-amz-")
            || matches!(lower.as_str(), "x-storage-u" | "x-secsdk-csrf-token")
            || value.len() > 8192
        {
            return Err("native/upload/apply: 上传扩展头试图覆盖保留头，已停止".into());
        }
        let name = HeaderName::from_bytes(lower.as_bytes())
            .map_err(|_| "native/upload/apply: 上传扩展头名称无效")?;
        let value =
            HeaderValue::from_str(value).map_err(|_| "native/upload/apply: 上传扩展头值无效")?;
        if let Some(existing) = headers.get(&name) {
            if existing != &value {
                return Err("native/upload/apply: 同名上传扩展头冲突".into());
            }
        }
        headers.insert(name, value);
    }
    Ok(headers)
}

fn parse_uploaded_video(
    value: &Value,
    size: u64,
    md5: String,
    sha256: String,
) -> Result<UploadedVideo, String> {
    let item = value
        .pointer("/Result/Results/0")
        .ok_or("native/upload/commit: 缺少视频提交结果")?;
    let meta = item
        .get("SourceInfo")
        .filter(|v| v.is_object())
        .or_else(|| item.get("VideoMeta").filter(|v| v.is_object()))
        .ok_or("native/upload/commit: 缺少视频元信息")?;
    let video_id = item["Vid"]
        .as_str()
        .or_else(|| meta["Vid"].as_str())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 128
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
        })
        .ok_or("native/upload/commit: 未取得有效视频 ID")?
        .to_owned();
    let poster_uri = item["PosterUri"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or("native/upload/commit: Snapshot 未返回封面资源，未创建作品")?
        .to_owned();
    let width = meta["Width"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        .ok_or("native/upload/commit: 缺少有效视频宽度")? as u32;
    let height = meta["Height"]
        .as_u64()
        .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        .ok_or("native/upload/commit: 缺少有效视频高度")? as u32;
    let duration = meta["Duration"]
        .as_f64()
        .filter(|n| n.is_finite() && *n > 0.0)
        .ok_or("native/upload/commit: 缺少有效视频时长")?;
    if meta["Size"]
        .as_u64()
        .is_some_and(|returned| returned != size)
    {
        return Err("native/upload/commit: 服务端视频字节数与上传文件不一致".into());
    }
    if let Some(server_md5) = meta["Md5"].as_str().filter(|s| !s.is_empty()) {
        if !server_md5.eq_ignore_ascii_case(&md5) {
            return Err("native/upload/commit: 服务端 MD5 与实际上传字节不一致".into());
        }
    }
    let format = meta["Format"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 32)
        .map(str::to_owned);
    Ok(UploadedVideo {
        video_id,
        poster_uri,
        width,
        height,
        duration,
        size,
        format,
        md5,
        sha256,
    })
}

fn sign_vod(
    sts: &UploadCredentials,
    method: &str,
    query: &[(&str, String)],
    body: &[u8],
) -> Result<HeaderMap, String> {
    if !matches!(method, "GET" | "POST") {
        return Err("native/upload/sign: 不支持的 VOD 方法".into());
    }
    let (date, timestamp) = aws_timestamp(sts.now()?)?;
    let payload_hash = hex(digest::digest(&digest::SHA256, body).as_ref());
    let mut signed = BTreeMap::new();
    signed.insert("x-amz-date", timestamp.clone());
    signed.insert("x-amz-security-token", sts.session_token.clone());
    if method == "POST" {
        signed.insert("x-amz-content-sha256", payload_hash.clone());
    }
    let names = signed.keys().copied().collect::<Vec<_>>().join(";");
    let headers = signed
        .iter()
        .map(|(k, v)| format!("{k}:{v}\n"))
        .collect::<String>();
    let canonical = format!(
        "{method}\n/\n{}\n{headers}\n{names}\n{payload_hash}",
        canonical_query(query)
    );
    let scope = format!("{date}/{REGION}/{SERVICE}/aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{timestamp}\n{scope}\n{}",
        hex(digest::digest(&digest::SHA256, canonical.as_bytes()).as_ref())
    );
    let date_key = hmac_sha256(
        format!("AWS4{}", sts.secret_key).as_bytes(),
        date.as_bytes(),
    );
    let region_key = hmac_sha256(&date_key, REGION.as_bytes());
    let service_key = hmac_sha256(&region_key, SERVICE.as_bytes());
    let signing_key = hmac_sha256(&service_key, b"aws4_request");
    let signature = hex(&hmac_sha256(&signing_key, to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={names}, Signature={signature}",
        sts.access_key
    );
    let mut result = HeaderMap::new();
    for (name, value) in signed {
        result.insert(
            HeaderName::from_static(name),
            HeaderValue::from_str(&value)
                .map_err(|_| "native/upload/sign: 临时授权含非法请求头字符")?,
        );
    }
    let mut authorization =
        HeaderValue::from_str(&authorization).map_err(|_| "native/upload/sign: 签名头格式无效")?;
    authorization.set_sensitive(true);
    result.insert("authorization", authorization);
    if let Some(token) = result.get_mut("x-amz-security-token") {
        token.set_sensitive(true);
    }
    Ok(result)
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let result = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(result.as_ref());
    bytes
}

fn canonical_query(query: &[(&str, String)]) -> String {
    let mut fields = query
        .iter()
        .map(|(k, v)| (encode_component(k), encode_component(v)))
        .collect::<Vec<_>>();
    fields.sort();
    fields
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_component(value: &str) -> String {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            encoded.push(b as char);
        } else {
            encoded.push('%');
            encoded.push(DIGITS[(b >> 4) as usize] as char);
            encoded.push(DIGITS[(b & 15) as usize] as char);
        }
    }
    encoded
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        result.push(DIGITS[(b >> 4) as usize] as char);
        result.push(DIGITS[(b & 15) as usize] as char);
    }
    result
}

struct TransferPlan {
    direct: bool,
    part_size: u64,
}

fn transfer_plan(size: u64) -> TransferPlan {
    TransferPlan {
        direct: size <= 3 * MIB,
        part_size: if size >= 500 * MIB { 10 * MIB } else { 5 * MIB },
    }
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb88320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}
const CRC_TABLE: [u32; 256] = crc_table();

fn crc32_hex(bytes: &[u8]) -> String {
    let mut crc = u32::MAX;
    for b in bytes {
        crc = CRC_TABLE[((crc ^ (*b as u32)) & 255) as usize] ^ (crc >> 8);
    }
    format!("{:08x}", !crc)
}

struct Fingerprints {
    md5: Md5,
    sha256: digest::Context,
}
impl Fingerprints {
    fn new() -> Self {
        Self {
            md5: Md5::new(),
            sha256: digest::Context::new(&digest::SHA256),
        }
    }
    fn update(&mut self, bytes: &[u8]) {
        self.md5.update(bytes);
        self.sha256.update(bytes);
    }
    fn finish(self) -> (String, String) {
        (
            hex(&self.md5.finalize()),
            hex(self.sha256.finish().as_ref()),
        )
    }
}

fn read_chunk(file: &mut File, length: usize) -> Result<Vec<u8>, String> {
    if length > 10 * MIB as usize {
        return Err("native/upload/file: 分片长度超过内存限制".into());
    }
    let mut bytes = vec![0u8; length];
    file.read_exact(&mut bytes)
        .map_err(|_| "native/upload/file: 视频在上传期间缩短或读取失败")?;
    Ok(bytes)
}

fn read_json(response: reqwest::blocking::Response, stage: &str) -> Result<Value, String> {
    let status = response.status();
    if !status.is_success() {
        return Err(format!("native/upload/{stage}: HTTP {}", status.as_u16()));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_JSON + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| format!("native/upload/{stage}: 无法读取上传响应"))?;
    if bytes.len() as u64 > MAX_JSON {
        return Err(format!("native/upload/{stage}: 响应超过大小限制"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| format!("native/upload/{stage}: 上传服务未返回有效 JSON"))
}

fn safe_code(value: &Value) -> String {
    if let Some(code) = value.as_i64() {
        return code.to_string();
    }
    value
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
        .unwrap_or("unknown")
        .to_owned()
}

fn secret_field(value: &Value, names: &[&str], max: usize) -> Result<String, String> {
    let text = names
        .iter()
        .find_map(|name| value[*name].as_str())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= max
                && s.trim() == *s
                && s.bytes().all(|b| b >= 0x21 && b <= 0x7e)
        })
        .ok_or("native/upload/auth: 临时上传凭据缺少必要字段或格式无效")?;
    Ok(text.to_owned())
}

fn unix_now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|v| v.as_secs())
        .map_err(|_| "native/upload/time: 本机时钟早于 Unix 时间起点".into())
}

fn optional_time(value: &Value) -> Result<Option<u64>, String> {
    if value.is_null() {
        return Ok(None);
    }
    if let Some(epoch) = value.as_u64() {
        return normalize_epoch(epoch).map(Some);
    }
    if let Some(text) = value.as_str() {
        if let Ok(epoch) = text.parse::<u64>() {
            return normalize_epoch(epoch).map(Some);
        }
        return parse_utc(text).map(Some);
    }
    Err("native/upload/auth: 临时凭据时间格式无效".into())
}

fn normalize_epoch(epoch: u64) -> Result<u64, String> {
    let seconds = if epoch >= 1_000_000_000_000 {
        epoch / 1000
    } else {
        epoch
    };
    if (946684800..=253402300799).contains(&seconds) {
        Ok(seconds)
    } else {
        Err("native/upload/auth: 临时凭据时间超出支持范围".into())
    }
}

fn leap(year: u32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}
fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn parse_utc(text: &str) -> Result<u64, String> {
    let text = text
        .strip_suffix('Z')
        .or_else(|| text.strip_suffix("+00:00"))
        .ok_or("native/upload/auth: 临时凭据时间必须使用明确 UTC")?;
    let main = if let Some((main, fraction)) = text.split_once('.') {
        if fraction.is_empty()
            || fraction.len() > 9
            || !fraction.bytes().all(|b| b.is_ascii_digit())
        {
            return Err("native/upload/auth: 临时凭据 UTC 小数秒格式无效".into());
        }
        main
    } else {
        text
    };
    if main.len() != 19
        || !main.is_ascii()
        || &main[4..5] != "-"
        || &main[7..8] != "-"
        || &main[10..11] != "T"
        || &main[13..14] != ":"
        || &main[16..17] != ":"
    {
        return Err("native/upload/auth: 临时凭据 UTC 时间格式无效".into());
    }
    let number = |start: usize, end: usize| {
        main[start..end]
            .parse::<u32>()
            .map_err(|_| "native/upload/auth: 临时凭据 UTC 数字无效".to_owned())
    };
    let (year, month, day) = (number(0, 4)?, number(5, 7)?, number(8, 10)?);
    let (hour, minute, second) = (number(11, 13)?, number(14, 16)?, number(17, 19)?);
    if !(2000..=9999).contains(&year)
        || day == 0
        || day > days_in_month(year, month)
        || hour >= 24
        || minute >= 60
        || second >= 60
    {
        return Err("native/upload/auth: 临时凭据 UTC 时间范围无效".into());
    }
    let mut days = 0u64;
    for y in 1970..year {
        days += if leap(y) { 366 } else { 365 };
    }
    for m in 1..month {
        days += days_in_month(year, m) as u64;
    }
    days += (day - 1) as u64;
    Ok(days * 86400 + hour as u64 * 3600 + minute as u64 * 60 + second as u64)
}

fn aws_timestamp(epoch: u64) -> Result<(String, String), String> {
    if epoch > 253402300799 {
        return Err("native/upload/sign: 签名时间超出支持范围".into());
    }
    let mut days = epoch / 86400;
    let seconds = epoch % 86400;
    let mut year = 1970u32;
    loop {
        let n = if leap(year) { 366 } else { 365 };
        if days < n {
            break;
        }
        days -= n;
        year += 1;
    }
    let mut month = 1u32;
    while days >= days_in_month(year, month) as u64 {
        days -= days_in_month(year, month) as u64;
        month += 1;
    }
    let day = days + 1;
    let date = format!("{year:04}{month:02}{day:02}");
    Ok((
        date.clone(),
        format!(
            "{date}T{:02}{:02}{:02}Z",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    const APPLY_SIGNATURE: &str = "AWS4-HMAC-SHA256 Credential=AKFIRST/20231114/cn-north-1/vod/aws4_request, SignedHeaders=x-amz-date;x-amz-security-token, Signature=bf37ce19b8bb41b1391bd0cca570d50512b47ba1a92295ebfeb701b6c83c7c6a";
    const COMMIT_SIGNATURE: &str = "AWS4-HMAC-SHA256 Credential=AKSECOND/20231114/cn-north-1/vod/aws4_request, SignedHeaders=x-amz-content-sha256;x-amz-date;x-amz-security-token, Signature=00cefe2e63f3d21ab60ee40631a63173d731bfa7ab987a1b2be5e43e87d279c6";
    const SIX_MIB_MD5: &str = "b56590730ff55d0e899cf74bc916d99d";
    const SIX_MIB_SHA256: &str = "c126bea7f5169cf05030eb29025a1f351e177d6f1efdd4e56919ab1c170f965f";
    const COMMIT_BODY: &str = r#"{"Functions":[{"name":"GetMeta"},{"input":{"SnapshotTime":0},"name":"Snapshot"}],"SessionKey":"fixture-session"}"#;

    fn credentials(second: bool) -> UploadCredentials {
        let auth = json!({
            "AccessKeyID":if second{"AKSECOND"}else{"AKFIRST"},
            "SecretAccessKey":if second{"fixture-second-secret"}else{"fixture-first-secret"},
            "SessionToken":if second{"fixture-second-token"}else{"fixture-first-token"},
            "CurrentTime":"2023-11-14T22:13:20Z","ExpiredTime":"2023-11-14T23:13:20Z"
        });
        let mut credentials =
            parse_upload_auth(&json!({"status_code":0,"auth":auth.to_string()})).unwrap();
        credentials.frozen_time = Some(1_700_000_000);
        credentials
    }

    fn apply_query(size: u64) -> Vec<(&'static str, String)> {
        vec![
            ("Action", "ApplyUploadInner".into()),
            ("Version", API_VERSION.into()),
            ("SpaceName", "aweme".into()),
            ("FileType", "video".into()),
            ("IsInner", "1".into()),
            ("FileSize", size.to_string()),
            ("app_id", "2906".into()),
            ("user_id", "123".into()),
            ("s", "fixture".into()),
        ]
    }

    fn node_response(upload_id: Option<&str>) -> Value {
        json!({"Result":{"InnerUploadAddress":{"UploadNodes":[{
            "UploadHost":"upload.fixture.douyin.com","SessionKey":"fixture-session",
            "UploadHeader":[{"Key":"X-Upload-Fixture","Value":"present"}],
            "StoreInfos":[{"StoreUri":"fixture/video","Auth":"fixture-storage-auth","UploadID":upload_id}]
        }]}}})
    }

    fn commit_response(size: u64, md5: &str) -> Value {
        json!({"Result":{"Results":[{
            "Vid":"v-fixture-video","PosterUri":"fixture/snapshot",
            "SourceInfo":{"Width":320,"Height":240,"Duration":1.25,"Size":size,"Format":"mp4","Md5":md5}
        }]}})
    }

    #[test]
    fn vod_signatures_match_independently_computed_wire_vectors() {
        // Expected vectors were computed with Python hashlib/hmac/urllib.parse.
        let get = sign_vod(&credentials(false), "GET", &apply_query(6 * MIB), b"").unwrap();
        assert_eq!(get["authorization"].to_str().unwrap(), APPLY_SIGNATURE);
        assert_eq!(get["x-amz-date"], "20231114T221320Z");
        assert!(!get.contains_key("x-amz-content-sha256"));
        let commit_query = vec![
            ("Action", "CommitUploadInner".into()),
            ("Version", API_VERSION.into()),
            ("SpaceName", "aweme".into()),
            ("app_id", "2906".into()),
            ("user_id", "123".into()),
        ];
        let post = sign_vod(
            &credentials(true),
            "POST",
            &commit_query,
            COMMIT_BODY.as_bytes(),
        )
        .unwrap();
        assert_eq!(post["authorization"].to_str().unwrap(), COMMIT_SIGNATURE);
        assert_eq!(
            post["x-amz-content-sha256"],
            "7929d9bcc175f63ccd6b2acc7ae6715d26b7a64e9b6db43310e9537b3c5f13cb"
        );
        assert_eq!(
            canonical_query(&[("space", "a b".into()), ("符号", "+/".into())]),
            "%E7%AC%A6%E5%8F%B7=%2B%2F&space=a%20b"
        );
    }

    #[test]
    fn crc32_and_partition_boundaries_match_upload_contract() {
        assert_eq!(crc32_hex(b""), "00000000");
        assert_eq!(crc32_hex(b"123456789"), "cbf43926");
        assert!(transfer_plan(3 * MIB).direct);
        assert!(!transfer_plan(3 * MIB + 1).direct);
        assert_eq!(transfer_plan(3 * MIB + 1).part_size, 5 * MIB);
        assert_eq!(transfer_plan(499 * MIB).part_size, 5 * MIB);
        assert_eq!(transfer_plan(500 * MIB).part_size, 10 * MIB);
    }

    #[test]
    fn auth_parser_checks_expiry_and_redacts_values() {
        let error = parse_upload_auth(&json!({"status_code":0,"auth":"private-fixture-not-json"}))
            .unwrap_err();
        assert!(!error.contains("private-fixture"));
        assert!(!format!("{:?}", credentials(false)).contains("fixture-first"));
        let expired = json!({
            "AccessKeyID":"AKFIXTURE","SecretAccessKey":"private-fixture-secret",
            "SessionToken":"private-fixture-token","CurrentTime":1_700_000_000,"ExpiredTime":1_700_000_001
        });
        assert!(parse_upload_auth(&json!({"auth":expired.to_string()})).is_err());
        assert_eq!(parse_utc("2024-02-29T00:00:00Z").unwrap(), 1_709_164_800);
        assert!(parse_utc("2023-02-29T00:00:00Z").is_err());
        assert!(parse_utc("2024-02-29T00:00:00.invalidZ").is_err());
        assert_eq!(
            parse_utc("2024-02-29T00:00:00.123Z").unwrap(),
            1_709_164_800
        );
        assert_eq!(aws_timestamp(1_700_000_000).unwrap().1, "20231114T221320Z");
    }

    #[test]
    fn storage_routes_and_headers_do_not_forward_account_authority() {
        assert!(validated_storage_url("tos-lf-x.snssdk.com", "bucket/object").is_ok());
        assert!(validated_storage_url("node.bytedancevod.com", "bucket/object").is_ok());
        for host in [
            "127.0.0.1",
            "localhost",
            "snssdk.com.evil.invalid",
            "evil.invalid",
            "node.snssdk.com:443",
        ] {
            assert!(validated_storage_url(host, "bucket/object").is_err());
        }
        for uri in [
            "../escape",
            "/escape",
            "object?secret=value",
            "object#fragment",
        ] {
            assert!(validated_storage_url("node.snssdk.com", uri).is_err());
        }
        for name in [
            "Cookie",
            "Authorization",
            "Host",
            "x-storage-u",
            "x-secsdk-csrf-token",
            "x-amz-security-token",
            "x-amz-date",
        ] {
            assert!(parse_upload_headers(Some(&json!({name:"private-fixture"}))).is_err());
        }
        let headers =
            parse_upload_headers(Some(&json!([{"Key":"X-Upload-Fixture","Value":"ok"}]))).unwrap();
        assert_eq!(headers["x-upload-fixture"], "ok");
    }

    #[test]
    fn commit_requires_id_snapshot_metadata_and_matching_file_digest() {
        let value = commit_response(3, "900150983cd24fb0d6963f7d28e17f72");
        let uploaded = parse_uploaded_video(
            &value,
            3,
            "900150983cd24fb0d6963f7d28e17f72".into(),
            "fixture-sha".into(),
        )
        .unwrap();
        assert_eq!(uploaded.video_id, "v-fixture-video");
        assert_eq!(uploaded.poster_uri, "fixture/snapshot");
        let serialized = serde_json::to_string(&uploaded).unwrap();
        assert!(!serialized.contains("SessionKey") && !serialized.contains("SecretAccessKey"));
        assert!(
            parse_uploaded_video(
                &value,
                4,
                "900150983cd24fb0d6963f7d28e17f72".into(),
                "fixture".into()
            )
            .is_err()
        );
        assert!(parse_uploaded_video(&value, 3, "wrong".into(), "fixture".into()).is_err());
        let mut missing = value;
        missing["Result"]["Results"][0]["PosterUri"] = Value::Null;
        assert!(
            parse_uploaded_video(
                &missing,
                3,
                "900150983cd24fb0d6963f7d28e17f72".into(),
                "fixture".into()
            )
            .is_err()
        );
    }

    struct Request {
        method: String,
        url: Url,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    fn read_request(stream: &mut TcpStream) -> Request {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            assert!(header.len() < 32768, "fixture request headers too large");
            let mut b = [0u8; 1];
            stream.read_exact(&mut b).unwrap();
            header.push(b[0]);
        }
        let text = String::from_utf8(header).unwrap();
        let mut lines = text.lines();
        let line = lines.next().unwrap().split_whitespace().collect::<Vec<_>>();
        let method = line[0].to_owned();
        let url = Url::parse(&format!("http://127.0.0.1{}", line[1])).unwrap();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
            .collect::<BTreeMap<_, _>>();
        assert!(!headers.contains_key("cookie"));
        let len = headers
            .get("content-length")
            .map(|v| v.parse::<usize>().unwrap())
            .unwrap_or(0);
        assert!(len <= 10 * MIB as usize);
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).unwrap();
        Request {
            method,
            url,
            headers,
            body,
        }
    }

    fn fixture<F>(count: usize, handler: F) -> (UploadEngine, thread::JoinHandle<()>)
    where
        F: Fn(usize, &Request) -> (u16, Value) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        listener.set_nonblocking(true).unwrap();
        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            for index in 0..count {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing fixture request {index}");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("fixture accept failed: {e}"),
                    }
                };
                let request = read_request(&mut stream);
                let (status, value) = handler(index, &request);
                let body = serde_json::to_vec(&value).unwrap();
                let location = if status == 302 {
                    "Location: http://127.0.0.1:9/must-not-follow\r\n"
                } else {
                    ""
                };
                write!(stream,"HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{location}Connection: close\r\n\r\n",body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        (UploadEngine::fixture(origin), handle)
    }

    #[test]
    fn multipart_http_fixture_checks_bytes_crc_auth_refresh_and_commit() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        let bytes = (0..6 * MIB as usize)
            .map(|n| (n % 251) as u8)
            .collect::<Vec<_>>();
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();
        drop(bytes);
        let (engine, server) = fixture(6, |index, request| {
            let query = request
                .url
                .query_pairs()
                .into_owned()
                .collect::<BTreeMap<_, _>>();
            match index {
                0 => {
                    assert_eq!(request.method, "GET");
                    assert_eq!(
                        query.get("Action").map(String::as_str),
                        Some("ApplyUploadInner")
                    );
                    assert_eq!(request.headers["authorization"], APPLY_SIGNATURE);
                    assert_eq!(
                        request.headers["x-amz-security-token"],
                        "fixture-first-token"
                    );
                    (200, node_response(None))
                }
                1 => {
                    assert_eq!(request.method, "POST");
                    assert_eq!(request.url.path(), "/upload/v1/fixture/video");
                    assert_eq!(query.get("phase").map(String::as_str), Some("init"));
                    assert!(request.body.is_empty());
                    assert_eq!(request.headers["authorization"], "fixture-storage-auth");
                    assert!(!request.headers.contains_key("x-amz-security-token"));
                    (
                        200,
                        json!({"code":2000,"data":{"uploadid":"fixture-upload-id"}}),
                    )
                }
                2 | 3 => {
                    let offset = if index == 2 { 0 } else { 5 * MIB as usize };
                    let size = if index == 2 {
                        5 * MIB as usize
                    } else {
                        MIB as usize
                    };
                    assert_eq!(request.body.len(), size);
                    assert!(
                        request
                            .body
                            .iter()
                            .enumerate()
                            .all(|(i, b)| *b == ((offset + i) % 251) as u8)
                    );
                    assert_eq!(query.get("phase").map(String::as_str), Some("transfer"));
                    assert_eq!(query["part_offset"], offset.to_string());
                    assert_eq!(query["part_number"], (index - 1).to_string());
                    assert_eq!(
                        request.headers["content-crc32"],
                        if index == 2 { "c88bfcac" } else { "d0ce8a89" }
                    );
                    assert_eq!(request.headers["authorization"], "fixture-storage-auth");
                    assert_eq!(request.headers["x-storage-u"], "123");
                    assert_eq!(request.headers["x-upload-fixture"], "present");
                    assert!(!request.headers.contains_key("x-amz-security-token"));
                    (200, json!({"code":2000}))
                }
                4 => {
                    assert_eq!(query.get("phase").map(String::as_str), Some("finish"));
                    assert_eq!(request.body, b"1:c88bfcac,2:d0ce8a89");
                    (200, json!({"code":2000}))
                }
                5 => {
                    assert_eq!(request.method, "POST");
                    assert_eq!(
                        query.get("Action").map(String::as_str),
                        Some("CommitUploadInner")
                    );
                    assert_eq!(request.headers["authorization"], COMMIT_SIGNATURE);
                    assert_eq!(
                        request.headers["x-amz-security-token"],
                        "fixture-second-token"
                    );
                    assert_eq!(request.headers["content-type"], "text/plain;charset=UTF-8");
                    assert_eq!(request.body, COMMIT_BODY.as_bytes());
                    (200, commit_response(6 * MIB, SIX_MIB_MD5))
                }
                _ => unreachable!(),
            }
        });
        let mut calls = 0;
        let result = engine.upload(file.path(), "123", || {
            calls += 1;
            Ok(credentials(calls == 2))
        });
        server.join().unwrap();
        let uploaded = result.unwrap();
        assert_eq!(calls, 2);
        assert_eq!(uploaded.size, 6 * MIB);
        assert_eq!(uploaded.md5, SIX_MIB_MD5);
        assert_eq!(uploaded.sha256, SIX_MIB_SHA256);
        assert_eq!(uploaded.width, 320);
        assert_eq!(uploaded.height, 240);
        assert_eq!(uploaded.duration, 1.25);
    }

    #[test]
    fn direct_http_fixture_transfers_exact_bytes_without_init_or_finish() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"abc").unwrap();
        file.flush().unwrap();
        let (engine, server) = fixture(3, |index, request| match index {
            0 => (200, node_response(None)),
            1 => {
                assert_eq!(request.url.path(), "/upload/v1/fixture/video");
                assert!(request.url.query().is_none());
                assert_eq!(request.body, b"abc");
                assert_eq!(request.headers["content-crc32"], "352441c2");
                assert!(!request.headers.contains_key("x-amz-security-token"));
                (200, json!({"code":2000}))
            }
            2 => (200, commit_response(3, "900150983cd24fb0d6963f7d28e17f72")),
            _ => unreachable!(),
        });
        let mut calls = 0;
        let result = engine.upload(file.path(), "123", || {
            calls += 1;
            Ok(credentials(calls == 2))
        });
        server.join().unwrap();
        assert!(result.is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    fn transfer_failure_stops_before_auth_refresh_and_commit() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"abc").unwrap();
        file.flush().unwrap();
        let (engine, server) = fixture(2, |index, _| {
            if index == 0 {
                (200, node_response(None))
            } else {
                (200, json!({"code":4001,"message":"private-fixture-body"}))
            }
        });
        let mut calls = 0;
        let result = engine.upload(file.path(), "123", || {
            calls += 1;
            Ok(credentials(false))
        });
        server.join().unwrap();
        assert_eq!(calls, 1);
        let error = result.unwrap_err();
        assert!(error.contains("transfer"));
        assert!(!error.contains("private-fixture"));
    }

    #[test]
    fn upload_redirect_is_not_followed() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(b"abc").unwrap();
        file.flush().unwrap();
        let (engine, server) = fixture(2, |index, _| {
            if index == 0 {
                (200, node_response(None))
            } else {
                (302, json!({}))
            }
        });
        let mut calls = 0;
        let result = engine.upload(file.path(), "123", || {
            calls += 1;
            Ok(credentials(false))
        });
        server.join().unwrap();
        assert_eq!(calls, 1);
        assert!(result.unwrap_err().contains("HTTP 302"));
    }
}
