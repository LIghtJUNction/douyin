use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::publish_cover::{CoverReceipt, CoverSnapshot};
use crate::publish_options::{Backend, Options};
use crate::{oauth, openapi::OpenApiClient, store};

use crate::openapi::MAX_VIDEO_BYTES;
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const PUBLISH_SCOPE: &str = "video.create.bind";

#[derive(Args)]
pub struct PublishArgs {
    #[command(subcommand)]
    command: PublishCommand,
}

#[derive(Subcommand)]
enum PublishCommand {
    /// Rust 原生上传和投稿；使用当前创作者会话，无需运行手机或浏览器
    #[command(visible_alias = "native")]
    App(crate::publish_native::NativeArgs),
    /// 上传并投稿；自动保存进度，可先用 --dry-run 检查
    Video(VideoArgs),
    /// 使用已上传的视频继续创建作品，不重新上传
    Resume(ResumeArgs),
    /// 只读查看本地投稿记录，不访问网络
    Status { receipt: PathBuf },
}

#[derive(Args)]
struct VideoArgs {
    /// 本地视频文件
    file: PathBuf,
    #[command(flatten)]
    options: VideoOptions,
    /// 授权用户 open_id；必须与当前 OAuth 账号一致
    #[arg(long)]
    open_id: Option<String>,
    /// 单次文件或每片上传超时（秒），普通 API 仍使用 30 秒
    #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=86400))]
    upload_timeout: u64,
    /// 自定义进度文件；默认保存到本机配置目录，拒绝覆盖已有文件
    #[arg(long)]
    receipt: Option<PathBuf>,
    /// 仅上传并保存记录，之后用 publish resume 创建作品
    #[arg(long, conflicts_with = "dry_run")]
    upload_only: bool,
    /// 只验证文件、参数与本地 OAuth 权限，不上传、不创建文件
    #[arg(long)]
    dry_run: bool,
    /// 确认上传和创建作品
    #[arg(long)]
    yes: bool,
}

#[derive(Args, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoOptions {
    /// 视频标题，可包含话题；真正 @ 用户还需要平台用户标识
    #[arg(short, long, default_value = "")]
    title: String,
    /// 发布可见范围
    #[arg(long, value_enum, default_value_t = Visibility::Public)]
    visibility: Visibility,
    /// 从视频指定秒数截取封面，必须是有限非负数
    #[arg(long, conflicts_with = "cover")]
    cover_tsp: Option<f64>,
    #[command(flatten)]
    #[serde(default)]
    extra: Options,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Visibility {
    Public,
    Private,
    Friends,
}

#[derive(Args)]
struct ResumeArgs {
    receipt: PathBuf,
    /// 创建结果未知时，仅在核实账号没有重复作品后显式重试
    #[arg(long)]
    retry_create: bool,
    #[arg(long)]
    yes: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Stage {
    Uploading,
    CoverUploading,
    Uploaded,
    Creating,
    Unknown,
    Submitted,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u8,
    stage: Stage,
    client_key: String,
    open_id: String,
    bytes: u64,
    options: VideoOptions,
    video_id: Option<String>,
    item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cover: Option<CoverReceipt>,
}

struct Identity<'a> {
    token: &'a str,
    open_id: &'a str,
    client_key: &'a str,
}

// The stable sidecar inode keeps concurrent resume processes serialized while
// the receipt itself is replaced atomically. The OS releases this lock on exit.
struct Journal {
    path: PathBuf,
    _lock: File,
}

impl Drop for Journal {
    fn drop(&mut self) {
        // Release now even if a concurrent process launch inherited a descriptor.
        let _ = self._lock.unlock();
    }
}

pub fn run(args: PublishArgs) -> Result<(), String> {
    let result = match args.command {
        PublishCommand::App(args) => crate::publish_native::run(args)?,
        PublishCommand::Video(args) => publish_video(args)?,
        PublishCommand::Resume(args) => resume(args)?,
        PublishCommand::Status { receipt } => match crate::publish_native::status(&receipt)? {
            Some(value) => value,
            None => summary(&read_receipt(&receipt)?, &receipt),
        },
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|_| "投稿结果序列化失败")?
    );
    if result["ok"] == false {
        if let Some(reason) = result.get("reason").and_then(Value::as_str) {
            return Err(reason.to_owned());
        }
        return Err("投稿结果尚未确认或未通过审核；请查看输出阶段及投稿记录，再回查本人作品。不会自动重复投稿".into());
    }
    Ok(())
}

impl VideoOptions {
    fn validate(&self) -> Result<(), String> {
        if self.title.chars().count() > 1000 {
            return Err("视频标题不能超过 1000 个字符".into());
        }
        if self.cover_tsp.is_some_and(|v| !v.is_finite() || v < 0.0) {
            return Err("封面时间必须是有限的非负秒数".into());
        }
        if self.cover_tsp.is_some() && self.extra.cover.is_some() {
            return Err("--cover 与 --cover-tsp 互斥，必须明确选择封面来源".into());
        }
        self.extra.validate(Backend::OpenApi)?;
        Ok(())
    }

    fn body(&self, video_id: &str, cover: Option<&CoverReceipt>) -> Result<Value, String> {
        self.validate()?;
        let mut body = json!({
            "video_id": video_id,
            "text": self.title,
            "private_status": match self.visibility {
                Visibility::Public => 0,
                Visibility::Private => 1,
                Visibility::Friends => 2,
            },
        });
        if let Some(value) = self.cover_tsp {
            body["cover_tsp"] = json!(value);
        }
        match (self.extra.cover.as_ref(), cover) {
            (Some(_), Some(cover)) => {
                cover.validate()?;
                let image_id = cover
                    .image_id
                    .as_deref()
                    .ok_or("自定义封面缺少已确认的 image_id；停止创建作品")?;
                body["custom_cover_image_url"] = json!(image_id);
            }
            (None, None) => {}
            _ => return Err("封面请求与上传记录不一致；停止创建作品".into()),
        }
        Ok(body)
    }
}

fn identity<'a>(saved: &'a Value, requested: Option<&str>) -> Result<Identity<'a>, String> {
    let token = oauth::field(saved, "accessToken")
        .ok_or("OAuth 未授权；请先完成 douyin auth login --scope user_info,video.create.bind")?;
    let open_id = oauth::field(saved, "openId").ok_or("OAuth 缺少 openId，请重新授权后再投稿")?;
    let client_key =
        oauth::field(saved, "clientKey").ok_or("OAuth 缺少 clientKey，请重新授权后再投稿")?;
    if requested.is_some_and(|id| id.trim().is_empty() || id != open_id) {
        return Err("--open-id 必须与当前 OAuth 授权账号一致".into());
    }
    if saved["scopesVerified"] != true {
        return Err("OAuth 投稿权限尚未核实；请刷新 token 或重新授权 video.create.bind".into());
    }
    if !saved["scopes"]
        .as_array()
        .is_some_and(|scopes| scopes.iter().any(|v| v == PUBLISH_SCOPE))
    {
        return Err("OAuth 未授予 video.create.bind；请完成包含该权限的用户授权".into());
    }
    if !saved["expiresAt"].is_null() {
        let expires = saved["expiresAt"]
            .as_u64()
            .ok_or("OAuth 到期时间格式无效，请刷新 token")?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "系统时间无效")?
            .as_secs();
        if expires <= now {
            return Err("OAuth access token 已过期，请先运行 douyin auth refresh".into());
        }
    }
    Ok(Identity {
        token,
        open_id,
        client_key,
    })
}

fn validate_file(path: &Path) -> Result<u64, String> {
    let metadata = std::fs::metadata(path).map_err(|_| "无法读取投稿视频文件")?;
    if !metadata.is_file() {
        return Err("投稿路径必须是文件".into());
    }
    match metadata.len() {
        0 => Err("投稿视频文件不能为空".into()),
        bytes if bytes > MAX_VIDEO_BYTES => Err("视频超过抖音开放平台 4GB 限制".into()),
        bytes => Ok(bytes),
    }
}

fn publish_video(args: VideoArgs) -> Result<Value, String> {
    if !args.yes && !args.dry_run {
        return Err("投稿会上传文件并创建抖音作品，需要显式 --yes；只检查可使用 --dry-run".into());
    }
    if args.dry_run
        && let Some(report) = args.options.extra.blocked_preflight(Backend::OpenApi)
    {
        return Ok(report);
    }
    args.options.validate()?;
    let bytes = validate_file(&args.file)?;
    let cover = args
        .options
        .extra
        .cover
        .as_deref()
        .map(CoverSnapshot::read)
        .transpose()?;
    if args.receipt.as_ref().is_some_and(|p| p.exists()) {
        return Err("拒绝覆盖已有投稿记录；请使用 publish resume 或选择新路径".into());
    }
    let saved = oauth::load()?;
    let identity = identity(&saved, args.open_id.as_deref())?;
    if args.dry_run {
        return Ok(json!({
            "ok": true, "dry_run": true, "bytes": bytes,
            "upload_mode": if bytes > 50 * 1024 * 1024 { "multipart" } else { "single" },
            "upload_timeout": args.upload_timeout, "visibility": args.options.visibility,
            "scope": PUBLISH_SCOPE, "authorization": "locally_verified",
            "backend": "openapi", "uploaded": false, "submitted": false,
            "settings": args.options.extra.report(Backend::OpenApi, None),
            "cover": cover.as_ref().map(|c| &c.receipt),
            "cover_validation": if cover.is_some() { Some("jpeg_markers_checked") } else { None },
        }));
    }
    let client = OpenApiClient::new()?;
    let mut receipt = Receipt {
        schema: 1,
        stage: Stage::Uploading,
        client_key: identity.client_key.to_owned(),
        open_id: identity.open_id.to_owned(),
        bytes,
        options: args.options,
        video_id: None,
        item_id: None,
        cover: cover.as_ref().map(|c| c.receipt.clone()),
    };
    let journal = Journal::create(args.receipt.as_deref(), &receipt)?;
    eprintln!("投稿记录：{}", journal.path.display());
    eprintln!("正在上传视频...");
    let uploaded = client.upload_video_with_timeout(
        identity.token,
        identity.open_id,
        &args.file,
        Duration::from_secs(args.upload_timeout),
    )?;
    receipt.video_id = Some(response_id(
        &uploaded,
        "/data/video/video_id",
        "上传响应缺少 video_id",
    )?);
    finish_uploads(
        &client,
        &identity,
        &journal,
        &mut receipt,
        cover.as_ref(),
        Duration::from_secs(args.upload_timeout),
    )?;
    if args.upload_only {
        return Ok(summary(&receipt, &journal.path));
    }
    create_video(&client, &identity, &journal, receipt)
}

fn finish_uploads(
    client: &OpenApiClient,
    identity: &Identity<'_>,
    journal: &Journal,
    receipt: &mut Receipt,
    cover: Option<&CoverSnapshot>,
    timeout: Duration,
) -> Result<(), String> {
    if let Some(cover) = cover {
        // Save the completed video and cover intent before sending image bytes.
        // A lost image response cannot silently become a coverless create.
        receipt.stage = Stage::CoverUploading;
        journal.write(receipt)?;
        let uploaded = client.upload_cover(identity.token, identity.open_id, cover, timeout)?;
        let image_id = response_id(
            &uploaded,
            "/data/image/image_id",
            "封面响应缺少 image_id；停止创建作品",
        )?;
        receipt
            .cover
            .as_mut()
            .ok_or("封面记录缺失；停止创建作品")?
            .image_id = Some(image_id);
    }
    receipt.options.body(
        receipt.video_id.as_deref().ok_or("缺少视频上传标识")?,
        receipt.cover.as_ref(),
    )?;
    receipt.stage = Stage::Uploaded;
    journal.write(receipt)
}

fn resume(args: ResumeArgs) -> Result<Value, String> {
    if !args.yes {
        return Err("继续投稿需要显式 --yes".into());
    }
    let journal = Journal::open(&args.receipt)?;
    let receipt = read_receipt(&journal.path)?;
    if receipt.stage == Stage::Submitted {
        return Ok(summary(&receipt, &journal.path));
    }
    can_resume(&receipt, args.retry_create)?;
    let saved = oauth::load()?;
    let identity = identity(&saved, Some(&receipt.open_id))?;
    if identity.client_key != receipt.client_key {
        return Err("投稿记录与当前 OAuth 应用不一致".into());
    }
    create_video(&OpenApiClient::new()?, &identity, &journal, receipt)
}

fn can_resume(receipt: &Receipt, retry_create: bool) -> Result<(), String> {
    match receipt.stage {
        Stage::Uploading => Err("视频尚未完整上传，请重新运行 publish video".into()),
        Stage::CoverUploading => Err("封面上传结果尚未完整保存；保留回执并核实，不能省略封面或自动重传后创建作品".into()),
        Stage::Creating | Stage::Unknown if !retry_create => Err(
            "上次创建结果待核实；请先在创作者中心检查作品，确认没有重复投稿后再使用 --retry-create --yes".into(),
        ),
        _ => Ok(()),
    }
}

fn create_video(
    client: &OpenApiClient,
    identity: &Identity<'_>,
    journal: &Journal,
    mut receipt: Receipt,
) -> Result<Value, String> {
    let video_id = receipt
        .video_id
        .as_deref()
        .ok_or("投稿记录缺少已上传的视频标识")?;
    let body = receipt.options.body(video_id, receipt.cover.as_ref())?;
    receipt.stage = Stage::Creating;
    journal.write(&receipt)?;
    eprintln!("正在创建作品...");
    let mut request =
        crate::openapi::Request::new(reqwest::Method::POST, "/api/douyin/v1/video/create_video/");
    request.token = Some(identity.token);
    request
        .params
        .push(("open_id".into(), identity.open_id.into()));
    request.json = Some(body);
    let result = client
        .request(request)
        .and_then(|value| response_id(&value, "/data/item_id", "投稿响应缺少 item_id"));
    match result {
        Ok(item_id) => {
            receipt.item_id = Some(item_id);
            receipt.stage = Stage::Submitted;
            journal
                .write(&receipt)
                .map_err(|_| "作品已创建，但保存最终记录失败；请先检查创作者中心，勿重复投稿")?;
            Ok(summary(&receipt, &journal.path))
        }
        Err(error) => {
            receipt.stage = Stage::Unknown;
            journal.write(&receipt)?;
            Err(format!(
                "{error}；已保留上传记录，创建结果待核实，请先检查创作者中心再决定是否重试"
            ))
        }
    }
}

fn response_id(value: &Value, pointer: &str, error: &str) -> Result<String, String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 16 * 1024)
        .map(str::to_owned)
        .ok_or_else(|| error.to_owned())
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

impl Journal {
    fn create(path: Option<&Path>, receipt: &Receipt) -> Result<Self, String> {
        let path = if let Some(path) = path {
            path.to_owned()
        } else {
            let dir = store::root().join("publish");
            std::fs::create_dir_all(&dir).map_err(|_| "无法创建投稿记录目录")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| "无法保护投稿记录目录")?;
            }
            let file = tempfile::Builder::new()
                .prefix("video-")
                .suffix(".json")
                .tempfile_in(dir)
                .map_err(|_| "无法创建投稿记录")?;
            let (_, path) = file.keep().map_err(|_| "无法保留投稿记录")?;
            let journal = Self::open(&path)?;
            journal.write(receipt)?;
            return Ok(journal);
        };
        let journal = Self::open(&path)?;
        let mut pending = tempfile::NamedTempFile::new_in(parent(&path))
            .map_err(|_| "无法创建投稿记录，请检查目标目录")?;
        write_json(&mut pending, receipt)?;
        pending
            .persist_noclobber(&path)
            .map_err(|_| "拒绝覆盖已有投稿记录，或目标路径不可写")?;
        sync_parent(&path)?;
        Ok(journal)
    }

    fn open(path: &Path) -> Result<Self, String> {
        let path = if path.exists() {
            path.canonicalize().map_err(|_| "无法解析投稿记录路径")?
        } else {
            path.to_owned()
        };
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(PathBuf::from(lock_path))
            .map_err(|_| "无法锁定投稿记录，请检查文件路径")?;
        lock.try_lock()
            .map_err(|_| "另一个进程正在使用这份投稿记录")?;
        Ok(Self {
            path: path.to_owned(),
            _lock: lock,
        })
    }

    fn write(&self, receipt: &Receipt) -> Result<(), String> {
        let mut pending =
            tempfile::NamedTempFile::new_in(parent(&self.path)).map_err(|_| "无法保存投稿记录")?;
        write_json(&mut pending, receipt)?;
        pending
            .persist(&self.path)
            .map_err(|_| "无法原子保存投稿记录")?;
        sync_parent(&self.path)?;
        Ok(())
    }
}

fn sync_parent(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(parent(path))
        .and_then(|dir| dir.sync_all())
        .map_err(|_| "无法持久保存投稿记录目录")?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn write_json(file: &mut tempfile::NamedTempFile, receipt: &Receipt) -> Result<(), String> {
    serde_json::to_writer_pretty(&mut *file, receipt).map_err(|_| "无法序列化投稿记录")?;
    file.write_all(b"\n")
        .and_then(|_| file.as_file().sync_all())
        .map_err(|_| "无法写入投稿记录".to_owned())
}

fn read_receipt(path: &Path) -> Result<Receipt, String> {
    let file = File::open(path).map_err(|_| "无法读取投稿记录")?;
    let mut bytes = Vec::new();
    file.take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取投稿记录")?;
    if bytes.len() as u64 > MAX_RECEIPT_BYTES {
        return Err("投稿记录超过大小限制".into());
    }
    let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| "投稿记录格式无效")?;
    if receipt.schema != 1
        || receipt.client_key.trim().is_empty()
        || receipt.open_id.trim().is_empty()
        || receipt.bytes == 0
        || receipt.bytes > MAX_VIDEO_BYTES
        || (receipt.stage != Stage::Uploading
            && receipt
                .video_id
                .as_ref()
                .is_none_or(|v| v.trim().is_empty()))
        || (receipt.stage == Stage::Submitted
            && receipt.item_id.as_ref().is_none_or(|v| v.trim().is_empty()))
    {
        return Err("投稿记录内容无效或版本不受支持".into());
    }
    receipt.options.validate()?;
    match (receipt.options.extra.cover.as_ref(), receipt.cover.as_ref()) {
        (Some(_), Some(cover)) => {
            cover.validate()?;
            if !matches!(receipt.stage, Stage::Uploading | Stage::CoverUploading)
                && cover.image_id.is_none()
            {
                return Err("投稿记录缺少已确认的自定义封面 image_id".into());
            }
        }
        (None, None) if receipt.stage != Stage::CoverUploading => {}
        _ => return Err("投稿记录的封面意图不一致".into()),
    }
    Ok(receipt)
}

fn summary(receipt: &Receipt, path: &Path) -> Value {
    json!({
        "ok": matches!(receipt.stage, Stage::Uploaded | Stage::Submitted),
        "stage": receipt.stage,
        "receipt": path,
        "video_id": receipt.video_id,
        "item_id": receipt.item_id,
        "visibility": receipt.options.visibility,
        "cover": receipt.cover,
        "settings": receipt.options.extra.report(Backend::OpenApi,
            receipt.cover.as_ref().and_then(|cover| cover.image_id.as_deref())),
        "create_sent": match receipt.stage {
            Stage::Submitted => Some(true),
            Stage::Creating | Stage::Unknown => None,
            _ => Some(false),
        },
        "review_status": if receipt.stage == Stage::Submitted { Some("not_queried") } else { None },
        "requires_verification": matches!(receipt.stage, Stage::CoverUploading | Stage::Creating | Stage::Unknown),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn options() -> VideoOptions {
        VideoOptions {
            title: "测试 #Rust".into(),
            visibility: Visibility::Private,
            cover_tsp: Some(2.3),
            extra: Options::default(),
        }
    }
    fn uploaded() -> Receipt {
        Receipt {
            schema: 1,
            stage: Stage::Uploaded,
            client_key: "app".into(),
            open_id: "account".into(),
            bytes: 10,
            options: options(),
            video_id: Some("encrypted-video".into()),
            item_id: None,
            cover: None,
        }
    }
    fn authorized() -> Value {
        json!({"clientKey":"app","openId":"account","accessToken":"private-token",
            "scopes":[PUBLISH_SCOPE],"scopesVerified":true,"expiresAt":u64::MAX})
    }

    #[test]
    fn body_uses_actual_visibility_and_cover_mapping() {
        let mut opts = options();
        for (visibility, code) in [
            (Visibility::Public, 0),
            (Visibility::Private, 1),
            (Visibility::Friends, 2),
        ] {
            opts.visibility = visibility;
            let body = opts.body("encrypted", None).unwrap();
            assert_eq!(body["private_status"], code);
            assert_eq!(body["cover_tsp"], 2.3);
            assert_eq!(body["text"], opts.title);
            assert_eq!(body["video_id"], "encrypted");
        }
        opts.cover_tsp = None;
        assert!(
            opts.body("encrypted", None)
                .unwrap()
                .get("cover_tsp")
                .is_none()
        );
    }

    #[test]
    fn parameters_and_oauth_are_checked_before_upload() {
        let mut opts = options();
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            opts.cover_tsp = Some(invalid);
            assert!(opts.validate().is_err());
        }
        opts.cover_tsp = Some(0.0);
        opts.title = "中".repeat(1000);
        assert!(opts.validate().is_ok());
        opts.title.push('中');
        assert!(opts.validate().is_err());
        let saved = authorized();
        assert!(identity(&saved, Some("account")).is_ok());
        for invalid in ["", "  ", "other-account"] {
            assert!(identity(&saved, Some(invalid)).is_err());
        }
        for (key, value) in [
            ("scopesVerified", json!(false)),
            ("scopes", json!(["video.create"])),
            ("expiresAt", json!(0)),
        ] {
            let mut saved = authorized();
            saved[key] = value;
            assert!(identity(&saved, None).is_err());
        }
    }

    #[test]
    fn receipt_is_private_atomic_and_cannot_be_overwritten_or_used_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        let journal = Journal::create(Some(&path), &uploaded()).unwrap();
        assert!(Journal::open(&path).is_err());
        assert_eq!(read_receipt(&path).unwrap().stage, Stage::Uploaded);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("private-token"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        drop(journal);
        assert!(Journal::create(Some(&path), &uploaded()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    #[cfg(unix)]
    #[test]
    fn receipt_drop_releases_lock_while_a_duplicate_descriptor_is_still_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        let journal = Journal::create(Some(&path), &uploaded()).unwrap();
        // A concurrent process launch may temporarily inherit this description.
        let duplicate = journal._lock.try_clone().unwrap();
        assert!(Journal::open(&path).is_err());
        drop(journal);
        let next = Journal::open(&path).unwrap();
        assert!(Journal::open(&path).is_err());
        drop(next);
        drop(duplicate);
    }

    #[cfg(unix)]
    #[test]
    fn receipt_aliases_share_the_same_lock_and_update_the_original_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        let alias = dir.path().join("alias.json");
        let journal = Journal::create(Some(&path), &uploaded()).unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(Journal::open(&alias).is_err());
        drop(journal);
        let journal = Journal::open(&alias).unwrap();
        let mut receipt = uploaded();
        receipt.stage = Stage::Unknown;
        journal.write(&receipt).unwrap();
        assert_eq!(read_receipt(&path).unwrap().stage, Stage::Unknown);
        assert!(alias.is_symlink());
    }

    #[test]
    fn uncertain_creation_requires_an_explicit_retry() {
        let mut receipt = uploaded();
        assert!(can_resume(&receipt, false).is_ok());
        for stage in [Stage::Creating, Stage::Unknown] {
            receipt.stage = stage;
            assert!(can_resume(&receipt, false).is_err());
            assert!(can_resume(&receipt, true).is_ok());
        }
        receipt.stage = Stage::Uploading;
        assert!(can_resume(&receipt, true).is_err());
        receipt.stage = Stage::CoverUploading;
        assert!(can_resume(&receipt, true).is_err());
    }

    #[test]
    fn custom_cover_requires_matching_receipt_and_survives_resume_without_local_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        let mut receipt = uploaded();
        receipt.options.cover_tsp = None;
        receipt.options.extra.cover = Some(dir.path().join("deleted-after-upload.jpg"));
        assert!(receipt.options.body("video", None).is_err());
        receipt.cover = Some(CoverReceipt {
            bytes: 20,
            sha256: "a".repeat(64),
            image_id: None,
        });
        assert!(
            receipt
                .options
                .body("video", receipt.cover.as_ref())
                .is_err()
        );
        receipt.cover.as_mut().unwrap().image_id = Some("encrypted-image-id".into());
        let journal = Journal::create(Some(&path), &receipt).unwrap();
        let restored = read_receipt(&path).unwrap();
        let body = restored
            .options
            .body("video", restored.cover.as_ref())
            .unwrap();
        assert_eq!(body["custom_cover_image_url"], "encrypted-image-id");
        assert!(body.get("cover_tsp").is_none());
        assert!(can_resume(&restored, false).is_ok());
        let report = summary(&restored, &path);
        assert_eq!(report["settings"]["custom_cover"]["encoded"], true);
        assert_eq!(report["create_sent"], false);
        assert_eq!(report["settings"]["ai_generated"]["encoded"], false);
        drop(journal);
        receipt.options.extra.cover = None;
        assert!(
            receipt
                .options
                .body("video", receipt.cover.as_ref())
                .is_err()
        );
        receipt.stage = Stage::Unknown;
        assert_eq!(summary(&receipt, &path)["create_sent"], Value::Null);
    }

    #[test]
    fn missing_image_id_keeps_cover_pending_and_never_sends_create() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let client = OpenApiClient::with_base(
            url::Url::parse(&format!("http://{}/", server.server_addr())).unwrap(),
        )
        .unwrap();
        let worker = std::thread::spawn(move || {
            let mut request = server
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .unwrap();
            assert!(
                request
                    .url()
                    .starts_with("/api/douyin/v1/video/upload_image/?")
            );
            let mut bytes = Vec::new();
            request.as_reader().read_to_end(&mut bytes).unwrap();
            assert!(bytes.windows(12).any(|w| w == b"name=\"image\""));
            request
                .respond(tiny_http::Response::from_string(
                    r#"{"data":{"error_code":0,"image":{}}}"#,
                ))
                .unwrap();
            assert!(
                server
                    .recv_timeout(Duration::from_millis(100))
                    .unwrap()
                    .is_none()
            );
        });
        let dir = tempfile::tempdir().unwrap();
        let cover_path = dir.path().join("cover.jpg");
        std::fs::write(&cover_path, b"\xff\xd8\xfffixture\xff\xd9").unwrap();
        let cover = CoverSnapshot::read(&cover_path).unwrap();
        let mut receipt = uploaded();
        receipt.options.cover_tsp = None;
        receipt.options.extra.cover = Some(cover_path);
        receipt.cover = Some(cover.receipt.clone());
        let path = dir.path().join("receipt.json");
        let journal = Journal::create(Some(&path), &receipt).unwrap();
        let saved = authorized();
        let identity = identity(&saved, None).unwrap();
        assert!(
            finish_uploads(
                &client,
                &identity,
                &journal,
                &mut receipt,
                Some(&cover),
                Duration::from_secs(2)
            )
            .is_err()
        );
        let restored = read_receipt(&path).unwrap();
        assert_eq!(restored.stage, Stage::CoverUploading);
        assert_eq!(restored.video_id.as_deref(), Some("encrypted-video"));
        assert!(can_resume(&restored, true).is_err());
        assert!(
            restored
                .options
                .body("video", restored.cover.as_ref())
                .is_err()
        );
        worker.join().unwrap();
    }

    #[test]
    fn uploaded_image_id_reaches_exactly_one_create_and_is_saved_in_receipt() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let client = OpenApiClient::with_base(
            url::Url::parse(&format!("http://{}/", server.server_addr())).unwrap(),
        )
        .unwrap();
        let worker = std::thread::spawn(move || {
            for step in 0..2 {
                let mut request = server
                    .recv_timeout(Duration::from_secs(3))
                    .unwrap()
                    .unwrap();
                let mut bytes = Vec::new();
                request.as_reader().read_to_end(&mut bytes).unwrap();
                let response = if step == 0 {
                    assert!(
                        request
                            .url()
                            .starts_with("/api/douyin/v1/video/upload_image/?")
                    );
                    json!({"data":{"error_code":0,"image":{"image_id":"server-image/+id"}}})
                } else {
                    assert!(
                        request
                            .url()
                            .starts_with("/api/douyin/v1/video/create_video/?")
                    );
                    let body: Value = serde_json::from_slice(&bytes).unwrap();
                    assert_eq!(body["custom_cover_image_url"], "server-image/+id");
                    assert_eq!(body["video_id"], "encrypted-video");
                    assert!(body.get("cover_tsp").is_none());
                    json!({"data":{"error_code":0,"item_id":"created-item"}})
                };
                request
                    .respond(tiny_http::Response::from_string(response.to_string()))
                    .unwrap();
            }
            assert!(
                server
                    .recv_timeout(Duration::from_millis(100))
                    .unwrap()
                    .is_none()
            );
        });
        let dir = tempfile::tempdir().unwrap();
        let cover_path = dir.path().join("cover.jpg");
        std::fs::write(&cover_path, b"\xff\xd8\xfffixture\xff\xd9").unwrap();
        let cover = CoverSnapshot::read(&cover_path).unwrap();
        let mut receipt = uploaded();
        receipt.options.cover_tsp = None;
        receipt.options.extra.cover = Some(cover_path);
        receipt.cover = Some(cover.receipt.clone());
        let path = dir.path().join("receipt.json");
        let journal = Journal::create(Some(&path), &receipt).unwrap();
        let saved = authorized();
        let identity = identity(&saved, None).unwrap();
        finish_uploads(
            &client,
            &identity,
            &journal,
            &mut receipt,
            Some(&cover),
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(read_receipt(&path).unwrap().stage, Stage::Uploaded);
        let result = create_video(&client, &identity, &journal, receipt).unwrap();
        assert_eq!(result["settings"]["custom_cover"]["encoded"], true);
        assert_eq!(result["create_sent"], true);
        let restored = read_receipt(&path).unwrap();
        assert_eq!(restored.stage, Stage::Submitted);
        assert_eq!(
            restored.cover.unwrap().image_id.as_deref(),
            Some("server-image/+id")
        );
        worker.join().unwrap();
    }

    #[test]
    fn create_failure_keeps_video_for_recovery_and_success_leaves_review_unqueried() {
        for success in [false, true] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let client = OpenApiClient::with_base(
                url::Url::parse(&format!("http://{}/", server.local_addr().unwrap())).unwrap(),
            )
            .unwrap();
            let worker = std::thread::spawn(move || {
                let (mut stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut headers = Vec::new();
                let mut byte = [0];
                while !headers.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    headers.push(byte[0]);
                }
                let headers = String::from_utf8(headers).unwrap();
                assert!(
                    headers.starts_with("POST /api/douyin/v1/video/create_video/?open_id=account ")
                );
                assert!(!headers.lines().next().unwrap().contains("private-token"));
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|v| v.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let body: Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(body, options().body("encrypted-video", None).unwrap());
                let response = if success {
                    json!({"data":{"error_code":0,"item_id":"created-item"}})
                } else {
                    json!({"data":{"error_code":28001008,"description":"private-token"}})
                }
                .to_string();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                )
                .unwrap();
            });
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("receipt.json");
            let journal = Journal::create(Some(&path), &uploaded()).unwrap();
            let saved = authorized();
            let result = create_video(
                &client,
                &identity(&saved, None).unwrap(),
                &journal,
                uploaded(),
            );
            worker.join().unwrap();
            let receipt = read_receipt(&path).unwrap();
            assert_eq!(receipt.video_id.as_deref(), Some("encrypted-video"));
            if success {
                assert_eq!(receipt.stage, Stage::Submitted);
                assert_eq!(result.unwrap()["review_status"], "not_queried");
            } else {
                assert_eq!(receipt.stage, Stage::Unknown);
                let error = result.unwrap_err();
                assert!(error.contains("28001008"));
                assert!(!error.contains("private-token"));
                assert!(can_resume(&receipt, false).is_err());
            }
        }
    }
}
