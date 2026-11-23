//! Native Creator HTTP publishing. The only write to the Creator service is a
//! one-shot create_v2 request. Journal ownership and --yes gating live above us.
//!
//! Protocol observations (not a supported public API contract):
//! Observed 2026-11-19 from public Creator application assets:
//! https://lf-fe-creator.douyinstatic.com/obj/douyn-creator-scm-cdn/douyin-creator-master-new/static/js/main.a20c35af.js
//! https://lf-fe-creator.douyinstatic.com/obj/douyn-creator-scm-cdn/douyin-creator-content-new/micro/static/js/douyin_content_index_micro_new.ac16fbc2.js
//! - creator master main.a20c35af.js: current-user GETs.
//! - creator content douyin_content_index_micro_new.ac16fbc2.js: work status codes.
//! - cv-cat/DouYin_Spider fc8b9c2490a9561265425c26058290f01c064eda:
//!   create_v2 and upload/auth/v5 wire fields. Independently implemented here.
use std::collections::{BTreeSet, HashSet};
use std::io::Read;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Request};
use reqwest::header::HeaderMap;
use reqwest::{Method, Url};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};

use crate::creator_auth::CreatorAuth;
use crate::publish_native_upload::{self, UploadCredentials, UploadedVideo};

const ORIGIN: &str = "https://creator.douyin.com";
const MEDIA_ACCOUNT: &str = "/web/api/media/user/info/";
const PC_ACCOUNT: &str = "/aweme/v1/creator/pc/user/info/";
const UPLOAD_AUTH: &str = "/web/api/media/upload/auth/v5/";
const CREATE: &str = "/web/api/media/aweme/create_v2/";
const WORK_LIST: &str = "/janus/douyin/creator/pc/work_list";
const MAX_JSON: u64 = 4 * 1024 * 1024;
const MAX_WORK_PAGES: usize = 12;

// A small boundary lets fixtures exercise the real HTTP path with synthetic
// credentials. Production has exactly one implementation and one fixed origin.
trait CreatorSession {
    fn headers(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<HeaderMap, String>;
    fn sign_query(
        &self,
        params: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<String, String>;
    fn ensure_publish_ready(&self) -> Result<Value, String>;
    fn account_id(&self) -> Option<&str>;
    fn platform_params(&self) -> &[(String, String)];
}

impl CreatorSession for CreatorAuth {
    fn headers(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<HeaderMap, String> {
        CreatorAuth::headers(self, method, path, body)
    }
    fn sign_query(
        &self,
        params: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<String, String> {
        CreatorAuth::sign_query(self, params, body)
    }
    fn ensure_publish_ready(&self) -> Result<Value, String> {
        CreatorAuth::ensure_publish_ready(self)
    }
    fn account_id(&self) -> Option<&str> {
        CreatorAuth::account_id(self)
    }
    fn platform_params(&self) -> &[(String, String)] {
        CreatorAuth::platform_params(self)
    }
}

struct Prepared {
    body: Vec<u8>,
    review: Value,
}

pub(crate) struct CreatorClient {
    auth: Box<dyn CreatorSession>,
    http: Client,
    origin: Url,
    timeout: Duration,
    verified_account: Option<String>,
    prepared: Option<Prepared>,
    attempted: bool,
}

impl CreatorClient {
    pub(crate) fn load(timeout: Duration) -> Result<Self, String> {
        Self::new(Box::new(CreatorAuth::load(timeout)?), timeout, ORIGIN)
    }

    fn new(auth: Box<dyn CreatorSession>, timeout: Duration, origin: &str) -> Result<Self, String> {
        let origin = Url::parse(origin).map_err(|_| "[连接] Creator 地址无效")?;
        #[cfg(not(test))]
        if origin.as_str().trim_end_matches('/') != ORIGIN {
            return Err("[连接] Creator 地址必须使用固定官方同源".into());
        }
        let http = Client::builder()
            .connect_timeout(timeout.min(Duration::from_secs(15)))
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .build()
            .map_err(|_| "[连接] 无法创建 Creator HTTP 客户端")?;
        Ok(Self {
            auth,
            http,
            origin,
            timeout,
            verified_account: None,
            prepared: None,
            attempted: false,
        })
    }

    /// A live, same-origin identity check; never trusts a cached nickname or UID.
    pub(crate) fn account(&mut self) -> Result<Value, String> {
        let media = self.get(
            MEDIA_ACCOUNT,
            vec![("device_platform".into(), "web".into())],
            false,
            "账号核验",
        )?;
        let user = media
            .get("user")
            .and_then(Value::as_object)
            .ok_or("[账号核验] Creator 响应缺少 user，无法确认登录")?;
        let media_id = user
            .get("uid")
            .and_then(numeric_id)
            .ok_or("[账号核验] Creator 响应缺少有效 user.uid")?;
        let nickname = user
            .get("nickname")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty() && v.len() <= 2048)
            .ok_or("[账号核验] Creator 响应缺少有效昵称")?;
        let pc = self.get(PC_ACCOUNT, vec![], false, "账号核验")?;
        let pc_id = pc
            .get("uid")
            .and_then(numeric_id)
            .ok_or("[账号核验] Creator PC 响应缺少 uid，登录状态未知")?;
        if pc_id != media_id
            || self
                .auth
                .account_id()
                .is_some_and(|bound| bound != media_id)
            || self
                .verified_account
                .as_deref()
                .is_some_and(|old| old != media_id)
        {
            return Err("[账号核验] 当前账号与同次登录凭据或本次任务不一致，已停止".into());
        }
        if bool_value(pc.get("is_confer_login")) == Some(true) {
            return Err("[账号核验] 当前为委托登录，尚不能核实投稿归属，已停止".into());
        }
        self.verified_account = Some(media_id.clone());
        Ok(json!({"id":media_id,"nickname":nickname}))
    }

    /// No upload authorization, upload, or create request is sent by this method.
    pub(crate) fn preflight(&mut self) -> Result<Value, String> {
        if self.verified_account.is_none() {
            self.account()?;
        }
        let ready = self
            .auth
            .ensure_publish_ready()
            .map_err(|e| format!("[发布凭据] {e}"))?;
        if self.auth.account_id() != self.verified_account.as_deref() {
            return Err("[发布凭据] 缺少与在线账号一致的同次登录绑定".into());
        }
        Ok(json!({
            "ready":true,"transport":"native_http","account_verified":true,
            "session":ready,"upload_sent":false,"create_sent":false
        }))
    }

    pub(crate) fn prepare(
        &mut self,
        file: &Path,
        title: &str,
        visibility: &str,
    ) -> Result<Value, String> {
        if self.attempted || self.prepared.is_some() {
            return Err("[上传准备] 本客户端已准备或提交；请使用投稿记录回查".into());
        }
        let visibility_type = visibility_number(visibility)?;
        if title.chars().count() > 1000 || title.contains('\0') {
            return Err("[上传准备] 文案超过 1000 字符或含 NUL".into());
        }
        let metadata = std::fs::metadata(file).map_err(|_| "[上传准备] 无法读取视频文件")?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err("[上传准备] 视频必须为非空普通文件".into());
        }
        // This gate is deliberately before even the first STS request.
        self.preflight()?;
        let account = self.account()?;
        let user_id = account["id"]
            .as_str()
            .ok_or("[账号核验] 缺少账号标识")?
            .to_owned();
        let upload = publish_native_upload::upload_video(file, &user_id, self.timeout, || {
            self.fetch_upload_auth()
        })
        .map_err(|e| format!("[视频上传] {e}"))?;
        let creation_id = new_creation_id()?;
        self.install_prepared(upload, title, visibility, visibility_type, &creation_id)
    }

    fn fetch_upload_auth(&self) -> Result<UploadCredentials, String> {
        self.auth
            .ensure_publish_ready()
            .map_err(|e| format!("[上传授权] {e}"))?;
        let response = self.get(UPLOAD_AUTH, vec![], true, "上传授权")?;
        publish_native_upload::parse_upload_auth(&response).map_err(|e| format!("[上传授权] {e}"))
    }

    fn install_prepared(
        &mut self,
        upload: UploadedVideo,
        title: &str,
        visibility: &str,
        visibility_type: u8,
        creation_id: &str,
    ) -> Result<Value, String> {
        let body = make_create_body(&upload, title, visibility_type, creation_id)?;
        let bytes = serde_json::to_vec(&body).map_err(|_| "[投稿准备] 无法编码请求")?;
        // Validate the exact final bytes can be signed. The request is not sent.
        self.build_request(
            Method::POST,
            CREATE,
            vec![("read_aid".into(), "2906".into())],
            Some(&bytes),
            true,
            "投稿准备",
        )?;
        let review = json!({
            "transport":"native_http","account_id":self.verified_account,
            "video_id":upload.video_id,"poster_uri":upload.poster_uri,
            "video_sha256":upload.sha256,"video_md5":upload.md5,
            "bytes":upload.size,"width":upload.width,"height":upload.height,
            "duration":upload.duration,"format":upload.format,
            "title":title,"visibility":visibility,"creation_id":creation_id,
            "settings":crate::publish_options::Options::default()
                .report(crate::publish_options::Backend::Native, None),
            "cover_source":"uploaded_video_snapshot",
            "ready_to_publish":true,"create_sent":false
        });
        self.prepared = Some(Prepared {
            body: bytes,
            review: review.clone(),
        });
        Ok(review)
    }

    /// The durable intent must already have been written by publish_journal.
    /// Every outcome after execute() is recorded as a receipt, never retried.
    pub(crate) fn submit_once(&mut self) -> Result<Value, String> {
        if self.attempted {
            return Err("[提交] 本次提交已经尝试，禁止再次 POST；请回查投稿记录".into());
        }
        self.preflight()?;
        self.account()?;
        let prepared = self
            .prepared
            .as_ref()
            .ok_or("[提交] 尚未完成视频上传与审阅")?;
        let request = self.build_request(
            Method::POST,
            CREATE,
            vec![("read_aid".into(), "2906".into())],
            Some(&prepared.body),
            true,
            "提交",
        )?;
        let review = prepared.review.clone();
        self.attempted = true;
        let response = self.execute_json(request, "提交");
        let video_id = review["video_id"].clone();
        let creation_id = review["creation_id"].clone();
        let receipt = match response {
            Ok(value) => {
                let item_id = value.get("item_id").and_then(numeric_id);
                if let Some(item_id) = item_id {
                    json!({
                        "stage":"submitted","item_id":item_id,"video_id":video_id,
                        "creation_id":creation_id,"review_status":"not_queried",
                        "evidence":"creator_create_v2_acknowledged"
                    })
                } else {
                    unknown(&video_id, "creator_create_v2_missing_item_id")
                }
            }
            Err(error) => {
                // execute_json errors contain only a local stage and numeric
                // status, never a server body, signed URL or credential.
                let mut receipt = unknown(&video_id, "creator_create_v2_response_unconfirmed");
                receipt["evidence"] = json!({
                    "source":"creator_create_v2","detail":error,
                    "reconciliation_required":true
                });
                receipt
            }
        };
        Ok(receipt)
    }

    pub(crate) fn reconcile(&mut self, receipt: &Value) -> Result<Value, String> {
        let account = self.account()?;
        let account_id = account["id"].as_str().ok_or("[回查] 缺少在线账号")?;
        if let Some(expected) = receipt.get("account_id").and_then(Value::as_str)
            && expected != account_id
        {
            return Err("[回查] 投稿记录所属账号与在线账号不同，已停止".into());
        }
        if let Some(expected) = receipt
            .pointer("/review/account_id")
            .and_then(Value::as_str)
            && expected != account_id
        {
            return Err("[回查] 审阅记录所属账号与在线账号不同，已停止".into());
        }
        let item_id = match receipt.get("item_id").filter(|v| !v.is_null()) {
            Some(value) => Some(numeric_id(value).ok_or("[回查] 投稿记录的 item_id 无效")?),
            None => None,
        };
        let video_id = receipt_video_id(receipt)?;
        if item_id.is_none() && video_id.is_none() {
            return Ok(unknown(&Value::Null, "receipt_has_no_exact_identifier"));
        }
        let mut cursor = "0".to_owned();
        let mut visited = BTreeSet::new();
        let mut matches: Vec<Value> = Vec::new();
        let mut matched_ids = HashSet::new();
        let mut exhausted = false;
        for _ in 0..MAX_WORK_PAGES {
            if !visited.insert(cursor.clone()) {
                break;
            }
            let response = self.get(
                WORK_LIST,
                vec![
                    ("scene".into(), "star_atlas".into()),
                    ("device_platform".into(), "android".into()),
                    ("status".into(), "4".into()),
                    ("count".into(), "18".into()),
                    ("max_cursor".into(), cursor.clone()),
                ],
                false,
                "作品回查",
            )?;
            let items = response
                .get("aweme_list")
                .and_then(Value::as_array)
                .ok_or("[作品回查] 响应缺少 aweme_list，已停止")?;
            for item in items {
                let Some(found_id) = item.get("aweme_id").and_then(numeric_id) else {
                    continue;
                };
                let is_match = if let Some(expected) = item_id.as_deref() {
                    found_id == expected
                } else {
                    video_id.as_deref().is_some_and(|expected| {
                        work_video_ids(item).iter().any(|id| id == expected)
                    })
                };
                if !is_match {
                    continue;
                }
                if let Some(author) = item.pointer("/author/uid").and_then(numeric_id)
                    && author != account_id
                {
                    return Err("[作品回查] 匹配作品的作者与当前账号不一致，已停止".into());
                }
                if matched_ids.insert(found_id) {
                    matches.push(item.clone());
                }
            }
            if item_id.is_some() && !matches.is_empty() {
                return classify_work(&matches[0], video_id.as_deref());
            }
            if matches.len() > 1 {
                return Ok(unknown(
                    &json!(video_id),
                    "multiple_works_match_uploaded_video_id",
                ));
            }
            let more = bool_value(response.get("has_more"));
            if more == Some(false) {
                exhausted = true;
                break;
            }
            if more != Some(true) {
                break;
            }
            let Some(next) = response.get("max_cursor").and_then(cursor_value) else {
                break;
            };
            if next == cursor {
                break;
            }
            cursor = next;
        }
        if exhausted && matches.len() == 1 {
            return classify_work(&matches[0], video_id.as_deref());
        }
        let evidence = if matches.is_empty() {
            "exact_work_not_found_no_resubmission"
        } else {
            "work_scan_incomplete_no_resubmission"
        };
        let mut result = unknown(&json!(video_id), evidence);
        if let Some(item_id) = item_id {
            result["item_id"] = json!(item_id);
        }
        Ok(result)
    }

    fn get(
        &self,
        path: &str,
        params: Vec<(String, String)>,
        signed: bool,
        stage: &str,
    ) -> Result<Value, String> {
        let request = self.build_request(Method::GET, path, params, None, signed, stage)?;
        self.execute_json(request, stage)
    }

    fn build_request(
        &self,
        method: Method,
        path: &str,
        mut params: Vec<(String, String)>,
        body: Option<&[u8]>,
        signed: bool,
        stage: &str,
    ) -> Result<Request, String> {
        let allowed = match method {
            Method::GET => matches!(path, MEDIA_ACCOUNT | PC_ACCOUNT | UPLOAD_AUTH | WORK_LIST),
            Method::POST => path == CREATE,
            _ => false,
        };
        if !allowed {
            return Err(format!("[{stage}] Creator 请求路径不在允许列表"));
        }
        for (key, value) in self.auth.platform_params() {
            if params.iter().any(|(existing, _)| existing == key) {
                continue;
            }
            if matches!(key.as_str(), "msToken" | "a_bogus" | "cookie" | "read_aid") {
                return Err(format!("[{stage}] 平台参数含保留凭据字段"));
            }
            params.push((key.clone(), value.clone()));
        }
        let query = if signed {
            self.auth
                .sign_query(&params, body)
                .map_err(|e| format!("[{stage}] {e}"))?
        } else {
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(params.iter())
                .finish()
        };
        let headers = self
            .auth
            .headers(method.as_str(), path, body)
            .map_err(|e| format!("[{stage}] {e}"))?;
        let mut url = self.origin.clone();
        url.set_path(path);
        url.set_query((!query.is_empty()).then_some(&query));
        let mut request = self.http.request(method, url).headers(headers);
        if let Some(body) = body {
            request = request.body(body.to_vec());
        }
        request
            .build()
            .map_err(|_| format!("[{stage}] 无法构建 Creator 请求"))
    }

    fn execute_json(&self, request: Request, stage: &str) -> Result<Value, String> {
        let response = self
            .http
            .execute(request)
            .map_err(|_| format!("[{stage}] Creator 请求失败或超时，已停止"))?;
        if !response.status().is_success() {
            return Err(format!(
                "[{stage}] Creator 返回 HTTP {}，已停止",
                response.status().as_u16()
            ));
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_JSON + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| format!("[{stage}] 响应未完整读取，已停止"))?;
        if bytes.len() as u64 > MAX_JSON {
            return Err(format!("[{stage}] 响应超过 4 MiB，已停止"));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| format!("[{stage}] 响应不是 JSON，可能需要验证，已停止"))?;
        check_success(&value, stage)?;
        Ok(value)
    }
}

fn check_success(value: &Value, stage: &str) -> Result<(), String> {
    // Presence of a challenge is a stop even if a wrapper advertises success.
    for path in [
        "/verify_center_decision_conf",
        "/verify_conf",
        "/captcha",
        "/captcha_url",
        "/data/verify_center_decision_conf",
        "/data/verify_conf",
        "/data/captcha",
    ] {
        if value.pointer(path).is_some_and(nonempty) {
            return Err(format!("[{stage}] 平台要求额外验证，已停止"));
        }
    }
    match value.get("status_code").and_then(Value::as_i64) {
        Some(0) => Ok(()),
        Some(code) => Err(format!("[{stage}] Creator 状态码 {code}，已停止")),
        None => Err(format!("[{stage}] 缺少明确成功状态码，已停止")),
    }
}

fn nonempty(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::String(v) => !v.is_empty() && v != "{}" && v != "null",
        Value::Array(v) => !v.is_empty(),
        Value::Object(v) => !v.is_empty(),
        Value::Number(v) => v.as_u64() != Some(0),
    }
}

fn visibility_number(visibility: &str) -> Result<u8, String> {
    match visibility {
        "public" => Ok(0),
        "private" => Ok(1),
        "friends" => Ok(2),
        _ => Err("[投稿准备] 可见范围必须为 public、private 或 friends".into()),
    }
}

fn new_creation_id() -> Result<String, String> {
    let mut bytes = [0u8; 8];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "[投稿准备] 无法生成本次创建标识")?;
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let prefix: String = bytes
        .iter()
        .map(|v| ALPHABET[usize::from(*v) % ALPHABET.len()] as char)
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "[投稿准备] 系统时间无效")?
        .as_millis();
    Ok(format!("{prefix}{now}"))
}

fn make_create_body(
    upload: &UploadedVideo,
    title: &str,
    visibility: u8,
    creation_id: &str,
) -> Result<Value, String> {
    if !valid_video_id(&upload.video_id)
        || upload.poster_uri.is_empty()
        || upload.poster_uri.len() > 2048
        || creation_id.is_empty()
    {
        return Err("[投稿准备] 视频或封面标识无效".into());
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "[投稿准备] 系统时间无效")?
        .as_secs();
    let chapter = json!({
        "chapter_abstract":"","chapter_details":[],"chapter_type":1,
        "chapter_tools_info":{
            "chapter_recommend_detail":[],"chapter_recommend_abstract":"",
            "chapter_source":2,"chapter_recommend_type":-2,"create_date":now,
            "is_pc":"1","is_pre_generated":"0","is_syn":"1"
        }
    });
    let cover = json!({
        "coverInfo":{"firstFrameCoverUri":upload.poster_uri,"uri":upload.poster_uri,
            "videoName":"","url":"","posterDelay":0},
        "recommendServerInfo":{"res":[],"times":[]},"recommendCoverList":[],
        "recommendCoverInfo":{"isFromRecommend":false,"isDefaultSelect":false,
            "isRecommendClickFrom":"","selectInfo":{},"editingInfo":{}},
        "recommendCoverTime":0,"coverUrl":"","coverHorizontalInfo":null,
        "coverHorizontalUrl":"","pasterInfo":null,"stateInfo":null,"croppedCoverInfo":null,
        "uploadBackgroundInfo":null,"uploadPasterInfo":null,"uploadCoverStateInfo":null,
        "xiguaCoverInfo":{"posterDelay":0},"xiguaPasterInfo":null,"xiguaStateInfo":null,
        "xiguaUploadCoverStateInfo":null,"xiguaUploadBackgroundInfo":null,
        "xiguaUploadPasterInfo":null,"editXigua":false,"coverSource":"","previewVideoList":[]
    });
    Ok(json!({"item":{
        "common":{
            "media_type":4,"video_id":upload.video_id,"text":title,"caption":title,"item_title":"",
            "visibility_type":visibility,"timing":0,"creation_id":creation_id,"download":1,
            "activity":"[]","text_extra":"[]","challenges":"[]","mentions":"[]",
            "hashtag_source":"","hot_sentence":"","interaction_stickers":"[]",
            "music_source":0,"music_id":null
        },
        "cover":{"poster":upload.poster_uri,"poster_delay":0,"cover_text_uri":null,
            "cover_text":null,"cover_tools_info":"{}",
            "cover_tools_extend_info":cover.to_string()},
        "mix":{},"selected_member":{"is_selected_member_video":false},
        "chapter":{"chapter":chapter.to_string()},"anchor":{},
        "sync":{"should_sync":false,"sync_to_toutiao":0},"open_platform":{},
        "assistant":{"is_preview":0,"is_post_assistant":1}
    }}))
}

fn numeric_id(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(v) => v.clone(),
        Value::Number(v) => v.as_u64()?.to_string(),
        _ => return None,
    };
    (!text.is_empty()
        && text.len() <= 32
        && text.bytes().all(|v| v.is_ascii_digit())
        && text.bytes().any(|v| v != b'0'))
    .then_some(text)
}

fn valid_video_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 256
        && text
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"_-".contains(&v))
}

fn receipt_video_id(receipt: &Value) -> Result<Option<String>, String> {
    let mut found = None;
    for path in ["/video_id", "/review/video_id"] {
        if let Some(value) = receipt.pointer(path).filter(|v| !v.is_null()) {
            let id = value
                .as_str()
                .filter(|v| valid_video_id(v))
                .ok_or("[回查] 投稿记录的 video_id 无效")?;
            if found.as_deref().is_some_and(|old| old != id) {
                return Err("[回查] 投稿记录包含冲突的 video_id".into());
            }
            found = Some(id.to_owned());
        }
    }
    Ok(found)
}

fn work_video_ids(work: &Value) -> Vec<String> {
    ["/video_id", "/video/vid", "/video/play_addr/uri"]
        .into_iter()
        .filter_map(|path| work.pointer(path).and_then(Value::as_str))
        .filter(|id| valid_video_id(id))
        .map(str::to_owned)
        .collect()
}

fn cursor_value(value: &Value) -> Option<String> {
    match value {
        Value::String(v)
            if v.len() <= 32 && !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) =>
        {
            Some(v.clone())
        }
        Value::Number(v) => v.as_u64().map(|v| v.to_string()),
        _ => None,
    }
}

fn bool_value(value: Option<&Value>) -> Option<bool> {
    match value {
        Some(Value::Bool(v)) => Some(*v),
        Some(Value::Number(v)) => match v.as_u64() {
            Some(0) => Some(false),
            Some(1) => Some(true),
            _ => None,
        },
        _ => None,
    }
}

fn unknown(video_id: &Value, evidence: &str) -> Value {
    json!({"stage":"unknown","video_id":video_id,"review_status":"unknown","evidence":evidence})
}

fn classify_work(work: &Value, video_id: Option<&str>) -> Result<Value, String> {
    let item_id = work
        .get("aweme_id")
        .and_then(numeric_id)
        .ok_or("[作品回查] 匹配作品缺少有效 ID")?;
    let status = work.get("status").and_then(Value::as_object);
    let code = work.get("status_value").and_then(Value::as_i64);
    let flag = |key: &str| status.and_then(|s| s.get(key));
    let reviewed = bool_value(flag("reviewed"));
    let reviewing = bool_value(flag("in_reviewing"));
    let blocked = [
        work.pointer("/review_struct/status"),
        work.pointer("/status/review_result/review_status"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_i64)
    .any(|v| [1, 2, 6, 20].contains(&v));
    let timer = work.pointer("/timer/status").and_then(Value::as_i64);
    let restricted = [
        "is_private",
        "self_see",
        "reviewed_friend_see",
        "part_see",
        "dont_share_status",
    ]
    .iter()
    .any(|key| bool_value(flag(key)) == Some(true));
    let visibility = flag("private_status").and_then(Value::as_i64);
    let (stage, review_status) = if code == Some(22) || bool_value(flag("is_delete")) == Some(true)
    {
        ("rejected", "deleted")
    } else if code == Some(23) || bool_value(flag("is_prohibited")) == Some(true) {
        ("rejected", "unavailable")
    } else if reviewing == Some(true) || (code == Some(141) && reviewed == Some(false)) {
        ("reviewing", "reviewing")
    } else if timer.is_some_and(|v| v != 1) || blocked || code == Some(144) {
        ("submitted", "restricted_or_processing")
    } else if reviewed == Some(true) && reviewing != Some(true) {
        match code {
            Some(102)
                if !restricted
                    && !visibility.is_some_and(|v| v != 0)
                    && bool_value(flag("allow_share")) != Some(false)
                    && bool_value(work.pointer("/aweme_control/can_share")) != Some(false) =>
            {
                ("public", "approved")
            }
            Some(140) if visibility != Some(0) => ("private", "approved"),
            Some(143) if !matches!(visibility, Some(0 | 1)) => ("friends", "approved"),
            _ => ("submitted", "unknown"),
        }
    } else {
        ("submitted", "unknown")
    };
    let mut evidence = json!({
        "source":"creator_work_list","status_value":code,"reviewed":reviewed,
        "in_reviewing":reviewing,"private_status":visibility
    });
    if let Some(timer) = timer {
        evidence["timer_status"] = json!(timer);
    }
    let mut result = json!({
        "stage":stage,"item_id":item_id,"video_id":video_id,
        "review_status":review_status,"evidence":evidence
    });
    if stage == "public" {
        result["share_url"] = json!(format!("https://www.douyin.com/video/{item_id}"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use tiny_http::{Header, Response, Server};

    struct TestAuth {
        ready: bool,
        signed: Arc<Mutex<Vec<Vec<u8>>>>,
    }
    impl CreatorSession for TestAuth {
        fn headers(
            &self,
            method: &str,
            _path: &str,
            _body: Option<&[u8]>,
        ) -> Result<HeaderMap, String> {
            let mut headers = HeaderMap::new();
            headers.insert("cookie", "sessionid=fixture-only".parse().unwrap());
            headers.insert("user-agent", "douyin-cli-fixture".parse().unwrap());
            if method == "POST" {
                self.ensure_publish_ready()?;
                headers.insert("content-type", "application/json".parse().unwrap());
                headers.insert("x-secsdk-csrf-token", "fixture-csrf".parse().unwrap());
                headers.insert(
                    "bd-ticket-guard-client-data",
                    "fixture-proof".parse().unwrap(),
                );
            }
            Ok(headers)
        }
        fn sign_query(
            &self,
            params: &[(String, String)],
            body: Option<&[u8]>,
        ) -> Result<String, String> {
            self.signed
                .lock()
                .unwrap()
                .push(body.unwrap_or_default().to_vec());
            let mut result = url::form_urlencoded::Serializer::new(String::new());
            result.extend_pairs(params.iter());
            result.append_pair("msToken", "fixture-token");
            result.append_pair("a_bogus", "fixture-signature");
            Ok(result.finish())
        }
        fn ensure_publish_ready(&self) -> Result<Value, String> {
            if self.ready {
                Ok(json!({"ready":true}))
            } else {
                Err("缺少同次登录 ticket".into())
            }
        }
        fn account_id(&self) -> Option<&str> {
            Some("123")
        }
        fn platform_params(&self) -> &[(String, String)] {
            &[]
        }
    }

    #[derive(Clone)]
    struct Step {
        method: &'static str,
        path: &'static str,
        status: u16,
        body: String,
        location: Option<String>,
    }
    impl Step {
        fn get(path: &'static str, value: Value) -> Self {
            Self {
                method: "GET",
                path,
                status: 200,
                body: value.to_string(),
                location: None,
            }
        }
        fn create(body: &str) -> Self {
            Self {
                method: "POST",
                path: CREATE,
                status: 200,
                body: body.into(),
                location: None,
            }
        }
    }
    struct Seen {
        method: String,
        url: String,
        body: Vec<u8>,
        cookie: Option<String>,
    }
    struct Fixture {
        address: String,
        seen: Arc<Mutex<Vec<Seen>>>,
        task: JoinHandle<()>,
    }
    fn serve(steps: Vec<Step>) -> Fixture {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = format!("http://{}", server.server_addr());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let records = seen.clone();
        let task = thread::spawn(move || {
            for step in steps {
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .expect("expected fixture request");
                assert_eq!(request.method().as_str(), step.method);
                assert_eq!(request.url().split('?').next().unwrap(), step.path);
                let mut body = Vec::new();
                request.as_reader().read_to_end(&mut body).unwrap();
                let cookie = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("cookie"))
                    .map(|h| h.value.as_str().to_owned());
                records.lock().unwrap().push(Seen {
                    method: request.method().as_str().into(),
                    url: request.url().into(),
                    body,
                    cookie,
                });
                let mut response = Response::from_string(step.body)
                    .with_status_code(step.status)
                    .with_header(Header::from_bytes("Content-Type", "application/json").unwrap());
                if let Some(location) = step.location {
                    response =
                        response.with_header(Header::from_bytes("Location", location).unwrap());
                }
                request.respond(response).unwrap();
            }
            assert!(
                server
                    .recv_timeout(Duration::from_millis(200))
                    .unwrap()
                    .is_none(),
                "unexpected additional request"
            );
        });
        Fixture {
            address,
            seen,
            task,
        }
    }
    fn identity_steps() -> Vec<Step> {
        vec![
            Step::get(
                MEDIA_ACCOUNT,
                json!({"status_code":0,"user":{"uid":"123","nickname":"测试账户"}}),
            ),
            Step::get(
                PC_ACCOUNT,
                json!({"status_code":0,"uid":"123","is_confer_login":false}),
            ),
        ]
    }
    fn client(address: &str, ready: bool) -> (CreatorClient, Arc<Mutex<Vec<Vec<u8>>>>) {
        let signed = Arc::new(Mutex::new(Vec::new()));
        let auth = TestAuth {
            ready,
            signed: signed.clone(),
        };
        let client = CreatorClient::new(Box::new(auth), Duration::from_secs(3), address).unwrap();
        (client, signed)
    }
    fn uploaded() -> UploadedVideo {
        UploadedVideo {
            video_id: "v_fixture_uploaded".into(),
            poster_uri: "tos-cn/fixture-cover".into(),
            width: 720,
            height: 1280,
            duration: 3.0,
            size: 12,
            format: Some("mp4".into()),
            md5: "a".repeat(32),
            sha256: "b".repeat(64),
        }
    }
    fn prime(client: &mut CreatorClient) -> Value {
        client.verified_account = Some("123".into());
        client
            .install_prepared(
                uploaded(),
                "中文原样 #话题 😀\n第二行",
                "private",
                1,
                "testcreation123",
            )
            .unwrap()
    }
    fn work(id: &str, video_id: &str, status_value: i64, reviewed: bool) -> Value {
        json!({
            "aweme_id":id,"author":{"uid":"123"},"video":{"play_addr":{"uri":video_id}},
            "status_value":status_value,
            "status":{"reviewed":reviewed,"in_reviewing":!reviewed,"is_delete":false,
                "is_prohibited":false,"private_status":0,"is_private":false}
        })
    }

    #[test]
    fn exact_caption_cover_visibility_and_body_are_signed_once_per_request() {
        let mut steps = identity_steps();
        steps.push(Step::create(r#"{"status_code":0,"item_id":"9001"}"#));
        let fixture = serve(steps);
        let (mut client, signed) = client(&fixture.address, true);
        let review = prime(&mut client);
        let result = client.submit_once().unwrap();
        assert_eq!(result["stage"], "submitted");
        assert_eq!(result["review_status"], "not_queried");
        assert!(client.submit_once().is_err());
        assert!(!review.to_string().contains("fixture-token"));
        assert!(!review.to_string().contains("sessionid"));
        fixture.task.join().unwrap();
        let seen = fixture.seen.lock().unwrap();
        assert_eq!(seen.iter().filter(|r| r.method == "POST").count(), 1);
        let post = seen.iter().find(|r| r.method == "POST").unwrap();
        let body: Value = serde_json::from_slice(&post.body).unwrap();
        assert_eq!(body["item"]["common"]["text"], "中文原样 #话题 😀\n第二行");
        assert_eq!(
            body["item"]["common"]["caption"],
            body["item"]["common"]["text"]
        );
        assert_eq!(body["item"]["common"]["item_title"], "");
        assert_eq!(body["item"]["common"]["visibility_type"], 1);
        assert_eq!(body["item"]["common"]["media_type"], 4);
        assert_eq!(body["item"]["cover"]["poster"], "tos-cn/fixture-cover");
        assert_eq!(*signed.lock().unwrap().last().unwrap(), post.body);
        assert!(post.url.contains("read_aid=2906"));
        assert_eq!(post.cookie.as_deref(), Some("sessionid=fixture-only"));
    }

    #[test]
    fn unknown_create_response_reconciles_by_exact_video_without_another_post() {
        let mut steps = identity_steps();
        steps.push(Step::create("<html>verify</html>"));
        steps.extend(identity_steps());
        steps.push(Step::get(
            WORK_LIST,
            json!({
                "status_code":0,"has_more":false,
                "aweme_list":[work("9002","v_fixture_uploaded",141,false)]
            }),
        ));
        let fixture = serve(steps);
        let (mut client, _) = client(&fixture.address, true);
        let review = prime(&mut client);
        let outcome = client.submit_once().unwrap();
        assert_eq!(outcome["stage"], "unknown");
        assert!(
            outcome["evidence"]["detail"]
                .as_str()
                .unwrap()
                .contains("[提交]")
        );
        assert!(client.submit_once().is_err());
        let receipt = json!({"account_id":"123","review":review,"stage":"unknown","video_id":outcome["video_id"]});
        let reconciled = client.reconcile(&receipt).unwrap();
        assert_eq!(reconciled["stage"], "reviewing");
        assert_eq!(reconciled["item_id"], "9002");
        fixture.task.join().unwrap();
        assert_eq!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.method == "POST")
                .count(),
            1
        );
    }

    #[test]
    fn missing_signing_bundle_stops_before_sts_or_upload() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = format!("http://{}", server.server_addr());
        let (mut client, _) = client(&address, false);
        client.verified_account = Some("123".into());
        let file = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(file.path(), b"fixture").unwrap();
        let error = client.prepare(file.path(), "hello", "private").unwrap_err();
        assert!(error.contains("发布凭据"));
        assert!(
            server
                .recv_timeout(Duration::from_millis(100))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn account_mismatch_stops_before_create() {
        let fixture = serve(vec![
            identity_steps()[0].clone(),
            Step::get(PC_ACCOUNT, json!({"status_code":0,"uid":"456"})),
        ]);
        let (mut client, _) = client(&fixture.address, true);
        prime(&mut client);
        assert!(client.submit_once().unwrap_err().contains("账号"));
        fixture.task.join().unwrap();
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
    }

    #[test]
    fn cookie_is_not_forwarded_to_redirect_target() {
        let other = Server::http("127.0.0.1:0").unwrap();
        let fixture = serve(vec![Step {
            method: "GET",
            path: MEDIA_ACCOUNT,
            status: 302,
            body: "{}".into(),
            location: Some(format!("http://{}/capture", other.server_addr())),
        }]);
        let (mut client, _) = client(&fixture.address, true);
        assert!(client.account().unwrap_err().contains("302"));
        assert!(
            other
                .recv_timeout(Duration::from_millis(200))
                .unwrap()
                .is_none()
        );
        fixture.task.join().unwrap();
        assert_eq!(
            fixture.seen.lock().unwrap()[0].cookie.as_deref(),
            Some("sessionid=fixture-only")
        );
    }

    #[test]
    fn same_title_and_time_never_identify_a_submission() {
        let mut unrelated = work("9003", "v_unrelated", 102, true);
        unrelated["desc"] = json!("中文原样 #话题 😀\n第二行");
        let mut steps = identity_steps();
        steps.push(Step::get(
            WORK_LIST,
            json!({"status_code":0,"has_more":0,"aweme_list":[unrelated]}),
        ));
        let fixture = serve(steps);
        let (mut client, _) = client(&fixture.address, true);
        let receipt = json!({"account_id":"123","title":"中文原样 #话题 😀\n第二行","review":{"video_id":"v_fixture_uploaded"}});
        let result = client.reconcile(&receipt).unwrap();
        assert_eq!(result["stage"], "unknown");
        assert!(result.get("item_id").is_none());
        fixture.task.join().unwrap();
    }

    #[test]
    fn existing_receipt_with_item_id_reconciles_read_only_after_restart() {
        let mut steps = identity_steps();
        steps.push(Step::get(
            WORK_LIST,
            json!({
                "status_code":0,"has_more":true,"max_cursor":1234,
                "aweme_list":[work("9010","v_fixture_uploaded",102,true)]
            }),
        ));
        let fixture = serve(steps);
        let (mut client, _) = client(&fixture.address, true);
        let result = client
            .reconcile(&json!({
                "account_id":"123","item_id":"9010","review":{"video_id":"v_fixture_uploaded"}
            }))
            .unwrap();
        assert_eq!(result["stage"], "public");
        fixture.task.join().unwrap();
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
        assert!(
            !fixture
                .seen
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .url
                .contains("a_bogus")
        );
    }

    #[test]
    fn multiple_exact_video_matches_are_ambiguous() {
        let mut steps = identity_steps();
        steps.push(Step::get(
            WORK_LIST,
            json!({
                "status_code":0,"has_more":false,"aweme_list":[
                    work("9011","v_fixture_uploaded",102,true),
                    work("9012","v_fixture_uploaded",102,true)
                ]
            }),
        ));
        let fixture = serve(steps);
        let (mut client, _) = client(&fixture.address, true);
        let result = client
            .reconcile(&json!({"account_id":"123","video_id":"v_fixture_uploaded"}))
            .unwrap();
        assert_eq!(result["stage"], "unknown");
        assert_eq!(result["evidence"], "multiple_works_match_uploaded_video_id");
        fixture.task.join().unwrap();
    }

    #[test]
    fn pagination_uses_response_cursor_and_only_exact_uploaded_video() {
        let mut steps = identity_steps();
        steps.extend([
            Step::get(
                WORK_LIST,
                json!({"status_code":0,"has_more":true,"max_cursor":88,
                "aweme_list":[work("9013","v_unrelated",102,true)]}),
            ),
            Step::get(
                WORK_LIST,
                json!({"status_code":0,"has_more":false,"max_cursor":99,
                "aweme_list":[work("9014","v_fixture_uploaded",141,false)]}),
            ),
        ]);
        let fixture = serve(steps);
        let (mut client, _) = client(&fixture.address, true);
        let result = client
            .reconcile(&json!({"account_id":"123","video_id":"v_fixture_uploaded"}))
            .unwrap();
        assert_eq!(result["stage"], "reviewing");
        fixture.task.join().unwrap();
        assert!(
            fixture
                .seen
                .lock()
                .unwrap()
                .last()
                .unwrap()
                .url
                .contains("max_cursor=88")
        );
    }

    #[test]
    fn status_codes_need_review_and_visibility_evidence() {
        assert_eq!(
            classify_work(&work("9020", "v1", 102, true), Some("v1")).unwrap()["stage"],
            "public"
        );
        assert_eq!(
            classify_work(&work("9020", "v1", 102, false), Some("v1")).unwrap()["stage"],
            "reviewing"
        );
        let mut private = work("9020", "v1", 140, true);
        private["status"]["private_status"] = json!(1);
        private["status"]["is_private"] = json!(true);
        assert_eq!(classify_work(&private, None).unwrap()["stage"], "private");
        private["status_value"] = json!(143);
        private["status"]["private_status"] = json!(2);
        assert_eq!(classify_work(&private, None).unwrap()["stage"], "friends");
        let mut future = work("9020", "v1", 777, true);
        assert_eq!(classify_work(&future, None).unwrap()["stage"], "submitted");
        future["status_value"] = json!(102);
        future["status"]["review_result"] = json!({"review_status":1});
        assert_ne!(classify_work(&future, None).unwrap()["stage"], "public");
        future["status"] = json!({});
        assert_eq!(
            classify_work(&future, None).unwrap()["review_status"],
            "unknown"
        );
    }

    #[test]
    fn challenge_error_and_invalid_receipt_never_become_success() {
        assert!(
            check_success(
                &json!({"status_code":0,"verify_conf":{"required":true}}),
                "fixture"
            )
            .is_err()
        );
        assert!(check_success(&json!({"status_code":8}), "fixture").is_err());
        assert!(check_success(&json!({"item_id":"42"}), "fixture").is_err());
        assert!(receipt_video_id(&json!({"video_id":"v1","review":{"video_id":"v2"}})).is_err());
        assert!(receipt_video_id(&json!({"video_id":"https://foreign.example/"})).is_err());
        let (client, _) = client("http://127.0.0.1:9", true);
        for path in [
            "https://foreign.example/api",
            "//foreign.example/api",
            "/unapproved",
            "/web/api/media/aweme/delete/",
        ] {
            assert!(
                client
                    .build_request(Method::POST, path, vec![], Some(b"{}"), true, "fixture")
                    .is_err()
            );
        }
    }

    #[test]
    fn all_supported_visibility_values_are_explicit() {
        for (name, number) in [("public", 0), ("private", 1), ("friends", 2)] {
            assert_eq!(visibility_number(name).unwrap(), number);
            let body = make_create_body(&uploaded(), "title", number, "creation").unwrap();
            assert_eq!(body["item"]["common"]["visibility_type"], number);
        }
        assert!(visibility_number("automatic").is_err());
    }
}
