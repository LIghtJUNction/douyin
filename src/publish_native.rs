//! Native HTTP publishing with durable account-and-video submission intent.
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use clap::{Args, ValueEnum};
use ring::digest::{Context, SHA256};
use serde_json::{Value, json};

use crate::publish_journal::Journal;
use crate::publish_native_client::CreatorClient as Session;

const MAX_VIDEO_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_RECORD_BYTES: u64 = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Visibility {
    Public,
    Private,
    Friends,
}
impl Visibility {
    fn name(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Friends => "friends",
        }
    }
}

#[derive(Args, Debug)]
pub(crate) struct NativeArgs {
    /// 本地视频文件；需 ffprobe 检查视频流
    file: PathBuf,
    /// 完整作品文案，可包含 #话题
    #[arg(short, long)]
    title: String,
    /// 追加话题文本，可重复；不带空格
    #[arg(long)]
    topic: Vec<String>,
    #[arg(long, value_enum, default_value_t = Visibility::Public)]
    visibility: Visibility,
    /// 确认当前创作者账号 ID，必须与服务端身份一致
    #[arg(long)]
    account: Option<String>,
    /// 上传与普通 API 的等待上限（秒）
    #[arg(long, alias = "upload-timeout", default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
    timeout: u64,
    /// 只检查文件、当前账号和发布凭据，不上传或创建投稿记录
    #[arg(long, conflicts_with = "prepare_only")]
    dry_run: bool,
    /// 原生上传并生成投稿预览，停止在最终创建作品之前
    #[arg(long, alias = "upload-only", conflicts_with = "dry_run")]
    prepare_only: bool,
    /// 明确确认投稿；结果未知时只回查，不自动重复发送
    #[arg(long)]
    yes: bool,
}

struct Media {
    path: PathBuf,
    bytes: u64,
    sha256: String,
    duration: f64,
    width: u64,
    height: u64,
}

pub(crate) fn run(args: NativeArgs) -> Result<Value, String> {
    if !args.yes && !args.dry_run && !args.prepare_only {
        return Err(
            "投稿需要显式 --yes；只检查请用 --dry-run，仅上传和核对请用 --prepare-only".into(),
        );
    }
    let title = normalized_title(&args.title, &args.topic)?;
    validate_options(&args)?;
    let media = inspect_media(&args.file).map_err(|e| at("video_check", e))?;
    let mut session =
        Session::load(Duration::from_secs(args.timeout)).map_err(|e| at("connect", e))?;
    let account = session.account().map_err(|e| at("account_check", e))?;
    let account_id = verified_account_id(&account)?.to_owned();
    if args.account.as_deref().is_some_and(|id| id != account_id) {
        return Err(at(
            "account_check",
            "当前登录账号与 --account 不一致，已停止",
        ));
    }
    let backend = "native";
    eprintln!(
        "发布账号：{}（{}）；后端：{}；可见范围：{}",
        account
            .get("nickname")
            .and_then(Value::as_str)
            .unwrap_or("未提供昵称"),
        account_id,
        backend,
        args.visibility.name()
    );
    if args.dry_run {
        let readiness = session
            .preflight()
            .map_err(|e| at("publish_credentials", e))?;
        return Ok(json!({
            "ok":true, "dry_run":true, "backend":backend, "account":account,
            "title":title, "visibility":args.visibility.name(),
            "video":media.summary(), "credentials":readiness, "uploaded":false, "submitted":false
        }));
    }
    let metadata = json!({
        "kind":"local-publish", "schema":1, "account_id":account_id,
        "video_sha256":media.sha256, "bytes":media.bytes,
        "title":title, "visibility":args.visibility.name(), "backend":backend,
        "stage":"prepared", "created_at":now()?, "review_status":"not_queried"
    });
    let mut journal =
        Journal::open(&account_id, &media.sha256, &metadata).map_err(|e| at("journal", e))?;
    eprintln!("投稿记录：{}", journal.path().display());

    // Re-running an uncertain or completed job performs no upload and no submit.
    if journal.submission_started() {
        reconcile(&mut session, &mut journal)?;
        return Ok(summary(&journal, true));
    }

    session
        .preflight()
        .map_err(|e| at("publish_credentials", e))?;
    // The uploader receives a private, stable snapshot. A source changed
    // during the copy is rejected before a byte is uploaded.
    let snapshot = snapshot_media(&media).map_err(|e| at("video_snapshot", e))?;
    let review = session
        .prepare(snapshot.path(), &title, args.visibility.name())
        .map_err(|e| at("prepare", e))?;
    if serde_json::to_vec(&review)
        .map_err(|_| "无法编码发布前核对结果")?
        .len()
        > 24 * 1024
    {
        return Err(at("review", "发布前核对结果过大，已停止"));
    }
    validate_review(&review, &media, &account_id, &title, args.visibility.name())?;
    let checked_account = session.account().map_err(|e| at("account_recheck", e))?;
    if verified_account_id(&checked_account)? != account_id {
        return Err(at("account_recheck", "账号在准备过程中发生变化，已停止"));
    }
    let mut record = journal.value().clone();
    record["review"] = review;
    record["prepared_at"] = json!(now()?);
    journal.save(record).map_err(|e| at("journal", e))?;
    if args.prepare_only {
        let mut result = summary(&journal, false);
        result["prepare_only"] = json!(true);
        result["submitted"] = json!(false);
        result["account"] = account;
        return Ok(result);
    }

    // The durable intent precedes the only call that is allowed to publish.
    let mut record = journal.value().clone();
    record["stage"] = json!("submitting");
    record["submitted_at"] = json!(now()?);
    journal
        .save(record)
        .map_err(|e| at("submission_intent", e))?;
    let mut record = journal.value().clone();
    match session.submit_once() {
        Ok(outcome) => apply_outcome(&mut record, outcome),
        Err(error) => {
            record["stage"] = json!("unknown");
            record["last_error"] = json!(bounded_error(&error));
        }
    }
    // Even if recording the response fails, perform one read-only lookup before
    // reporting the storage error. The earlier durable intent prevents re-send.
    if !persist_attempt(&mut session, &mut journal, record)? {
        reconcile(&mut session, &mut journal)?;
    }
    Ok(summary(&journal, false))
}

fn validate_review(
    review: &Value,
    media: &Media,
    account_id: &str,
    title: &str,
    visibility: &str,
) -> Result<(), String> {
    if review["video_sha256"].as_str() != Some(media.sha256.as_str())
        || review["bytes"].as_u64() != Some(media.bytes)
        || review["account_id"].as_str() != Some(account_id)
        || review["title"].as_str() != Some(title)
        || review["visibility"].as_str() != Some(visibility)
        || review["ready_to_publish"] != true
        || review["create_sent"] != false
        || !review["video_id"].as_str().is_some_and(|v| !v.is_empty())
    {
        return Err(at(
            "review",
            "已上传视频、账号、文案或可见范围与本次核对结果不一致，停止创建作品",
        ));
    }
    Ok(())
}

fn persist_attempt(
    session: &mut Session,
    journal: &mut Journal,
    mut record: Value,
) -> Result<bool, String> {
    if journal.save(record.clone()).is_ok() {
        return Ok(false);
    }
    let lookup = match session.account() {
        Ok(account) if verified_account_id(&account).ok() == record["account_id"].as_str() => {
            session.reconcile(&record)
        }
        _ => Err("无法重新确认账号；保留提交意图，停止自动操作".into()),
    };
    match lookup {
        Ok(outcome) => apply_outcome(&mut record, outcome),
        Err(error) => {
            apply_outcome(
                &mut record,
                json!({"stage":"unknown","review_status":"unknown"}),
            );
            record["last_error"] = json!(bounded_error(&error));
        }
    }
    record["last_checked_at"] = json!(now()?);
    let observed = record["stage"].as_str().unwrap_or("unknown").to_owned();
    journal.save(record).map_err(|_| at(
        "submission_receipt",
        format!("提交结果记录写入失败，已尝试只读回查（{observed}）；原提交意图仍保护重复执行，请检查磁盘并回查作品")
    ))?;
    Ok(true)
}

fn reconcile(session: &mut Session, journal: &mut Journal) -> Result<(), String> {
    let account = session.account().map_err(|e| at("reconcile_account", e))?;
    if verified_account_id(&account)? != journal.value()["account_id"].as_str().unwrap_or("") {
        return Err(at("reconcile_account", "当前账号与投稿记录不同，停止回查"));
    }
    let mut record = journal.value().clone();
    match session.reconcile(journal.value()) {
        Ok(outcome) => apply_outcome(&mut record, outcome),
        Err(error) => {
            apply_outcome(
                &mut record,
                json!({"stage":"unknown","review_status":"unknown"}),
            );
            record["last_error"] = json!(bounded_error(&error));
        }
    }
    record["last_checked_at"] = json!(now()?);
    journal.save(record).map_err(|e| at("reconcile_receipt", e))
}

fn known_submission(stage: &str) -> bool {
    matches!(
        stage,
        "submitted" | "reviewing" | "public" | "private" | "friends" | "rejected"
    )
}

fn apply_outcome(record: &mut Value, outcome: Value) {
    let previous = record["stage"].as_str().unwrap_or("unknown").to_owned();
    let observed = outcome["stage"].as_str().unwrap_or("unknown");
    let next = if known_submission(observed) {
        observed
    } else if known_submission(&previous) {
        // Acceptance remains proven, but old visibility/review is not current.
        "submitted"
    } else {
        "unknown"
    };
    if next == "submitted" && previous != "submitted" && known_submission(&previous) {
        record["last_known_stage"] = json!(previous);
    }
    if next != "public"
        && let Some(url) = record.as_object_mut().and_then(|m| m.remove("share_url"))
    {
        record["last_known_share_url"] = url;
    }
    record["stage"] = json!(next);
    for field in [
        "item_id",
        "video_id",
        "share_url",
        "review_status",
        "evidence",
        "submitted_at",
    ] {
        if let Some(value) = outcome.get(field).filter(|value| !value.is_null()) {
            // Store minimal evidence, never raw network responses or credentials.
            if serde_json::to_vec(value).is_ok_and(|bytes| bytes.len() <= 4096) {
                record[field] = value.clone();
            }
        }
    }
}

fn summary(journal: &Journal, replay: bool) -> Value {
    let mut value = journal.value().clone();
    let stage = value["stage"].as_str().unwrap_or("unknown").to_owned();
    value["ok"] = json!(stage == "prepared" || (known_submission(&stage) && stage != "rejected"));
    value["reconciled_existing"] = json!(replay);
    value["receipt"] = json!(journal.path());
    value["submitted"] = json!(known_submission(&stage));
    value["automatic_resubmit"] = json!(false);
    value
}

/// Add local records to the existing read-only publish status command without
/// changing the schema or semantics of the OAuth records.
pub(crate) fn status(path: &Path) -> Result<Option<Value>, String> {
    let file = File::open(path).map_err(|_| "无法读取投稿记录")?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取投稿记录")?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("投稿记录超过 64 KiB".into());
    }
    let mut value: Value = serde_json::from_slice(&bytes).map_err(|_| "投稿记录不是有效 JSON")?;
    if value["kind"] != "local-publish" {
        return Ok(None);
    }
    if value["schema"] != 1
        || value["account_id"].as_str().is_none()
        || value["video_sha256"].as_str().is_none()
        || !matches!(
            value["stage"].as_str(),
            Some(
                "prepared"
                    | "submitting"
                    | "unknown"
                    | "submitted"
                    | "reviewing"
                    | "public"
                    | "private"
                    | "friends"
                    | "rejected"
            )
        )
    {
        return Err("本地投稿记录损坏或版本未知".into());
    }
    value["receipt"] = json!(path);
    value["read_only"] = json!(true);
    value["automatic_resubmit"] = json!(false);
    Ok(Some(value))
}

fn normalized_title(title: &str, topics: &[String]) -> Result<String, String> {
    if title.trim().is_empty() || title.chars().any(|c| c.is_control() && c != '\n') {
        return Err("标题不能为空或含控制字符".into());
    }
    let mut title = title.to_owned();
    let mut seen = std::collections::HashSet::new();
    for topic in topics {
        let topic = topic.strip_prefix('#').unwrap_or(topic);
        if topic.is_empty()
            || topic
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == '#')
        {
            return Err("话题不能为空，不能包含空格、控制字符或额外 #".into());
        }
        if seen.insert(topic) {
            title.push_str(" #");
            title.push_str(topic);
        }
    }
    if title.chars().count() > 1000 {
        return Err("标题和话题合计不能超过 1000 个字符".into());
    }
    Ok(title)
}

fn validate_options(args: &NativeArgs) -> Result<(), String> {
    if args.account.as_deref().is_some_and(|value| {
        value.is_empty() || value.chars().any(|c| c.is_whitespace() || c.is_control())
    }) {
        return Err("--account 必须是非空账号 ID".into());
    }
    Ok(())
}

fn verified_account_id(account: &Value) -> Result<&str, String> {
    account["id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .ok_or_else(|| at("account_check", "后端未确认当前账号 ID，停止操作"))
}

impl Media {
    fn summary(&self) -> Value {
        json!({"bytes":self.bytes,"sha256":self.sha256,"duration_seconds":self.duration,"width":self.width,"height":self.height})
    }
}

fn inspect_media(path: &Path) -> Result<Media, String> {
    let path = path.canonicalize().map_err(|_| "无法读取视频文件")?;
    let metadata = std::fs::metadata(&path).map_err(|_| "无法读取视频文件属性")?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_VIDEO_BYTES {
        return Err("视频必须是非空普通文件，当前本地投稿入口上限为 4 GiB".into());
    }
    let bytes = bounded_command(
        Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_type,width,height:format=duration",
                "-of",
                "json",
                "-i",
            ])
            .arg(&path),
        Duration::from_secs(20),
        64 * 1024,
    )
    .map_err(|_| "无法验证视频，请安装 ffprobe 并检查文件是否为可解码的视频")?;
    let probe: Value = serde_json::from_slice(&bytes).map_err(|_| "ffprobe 未返回有效视频信息")?;
    let stream = probe["streams"]
        .as_array()
        .and_then(|streams| streams.first())
        .ok_or("文件没有视频流")?;
    let duration = probe["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .ok_or("视频时长不可识别")?;
    let width = stream["width"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or("视频宽度无效")?;
    let height = stream["height"]
        .as_u64()
        .filter(|v| *v > 0)
        .ok_or("视频高度无效")?;
    if stream["codec_type"] != "video" || !duration.is_finite() || duration <= 0.0 {
        return Err("视频流或时长无效".into());
    }
    let (sha256, bytes) = hash_reader(File::open(&path).map_err(|_| "无法读取视频")?, None)?;
    if bytes != metadata.len() {
        return Err("视频在检查过程中发生变化，请重试".into());
    }
    Ok(Media {
        path,
        bytes,
        sha256,
        duration,
        width,
        height,
    })
}

fn snapshot_media(media: &Media) -> Result<tempfile::NamedTempFile, String> {
    let suffix = media
        .path
        .extension()
        .and_then(|v| v.to_str())
        .filter(|v| !v.is_empty() && v.len() <= 12 && v.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|v| format!(".{v}"))
        .unwrap_or_else(|| ".mp4".into());
    let mut copy = tempfile::Builder::new()
        .prefix("douyin-publish-")
        .suffix(&suffix)
        .tempfile()
        .map_err(|_| "无法创建视频临时快照")?;
    let (sha256, bytes) = hash_reader(
        File::open(&media.path).map_err(|_| "无法读取视频")?,
        Some(copy.as_file_mut()),
    )?;
    if sha256 != media.sha256 || bytes != media.bytes {
        return Err("视频在检查后发生变化，已停止上传".into());
    }
    copy.as_file().sync_all().map_err(|_| "无法保存视频快照")?;
    // Probe the exact immutable bytes that the backend will upload, not only
    // the original pathname which another process could have replaced.
    let checked = inspect_media(copy.path())?;
    if checked.sha256 != media.sha256 || checked.bytes != media.bytes {
        return Err("视频快照验证不一致，已停止上传".into());
    }
    Ok(copy)
}

fn hash_reader(mut input: File, mut copy: Option<&mut File>) -> Result<(String, u64), String> {
    let mut context = Context::new(&SHA256);
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = input.read(&mut buffer).map_err(|_| "读取视频失败")?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_VIDEO_BYTES {
            return Err("视频超过 4 GiB 本地限制".into());
        }
        context.update(&buffer[..count]);
        if let Some(writer) = &mut copy {
            writer
                .write_all(&buffer[..count])
                .map_err(|_| "视频快照写入失败，请检查磁盘空间")?;
        }
    }
    let digest = context
        .finish()
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((digest, total))
}

fn bounded_command(
    command: &mut Command,
    timeout: Duration,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法启动视频检查程序")?;
    let stdout = child.stdout.take().ok_or("无法读取视频检查输出")?;
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        stdout
            .take(max_bytes as u64 + 1)
            .read_to_end(&mut output)
            .map(|_| output)
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("视频检查超时或中断");
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "视频检查输出线程异常")?
        .map_err(|_| "读取视频检查结果失败")?;
    if !status?.success() || bytes.len() > max_bytes {
        return Err("视频检查失败或输出超过限制".into());
    }
    Ok(bytes)
}

fn at(stage: &str, error: impl AsRef<str>) -> String {
    format!("阶段 {stage}：{}", error.as_ref())
}
fn bounded_error(error: &str) -> String {
    error.chars().take(1024).collect()
}
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "系统时间无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn topics_are_normalized_without_losing_unicode() {
        assert_eq!(
            normalized_title("标题", &["Rust".into(), "#开源".into(), "Rust".into()]).unwrap(),
            "标题 #Rust #开源"
        );
        for bad in ["", "#", "two words", "a#b", "line\nbreak"] {
            assert!(normalized_title("标题", &[bad.into()]).is_err());
        }
        assert!(normalized_title(" \n", &[]).is_err());
        assert!(normalized_title(&"字".repeat(1001), &[]).is_err());
    }
    #[test]
    fn uncertain_review_does_not_erase_observed_submission_or_invent_success() {
        let mut record = json!({"stage":"submitted","item_id":"123"});
        apply_outcome(
            &mut record,
            json!({"stage":"unknown","review_status":"unknown","item_id":null}),
        );
        assert_eq!(record["stage"], "submitted");
        assert_eq!(record["item_id"], "123");
        let mut record = json!({"stage":"submitting"});
        apply_outcome(&mut record, json!({"ok":true,"stage":"prepared"}));
        assert_eq!(record["stage"], "unknown");
        let mut record = json!({
            "stage":"public", "item_id":"123", "share_url":"https://www.douyin.com/video/123"
        });
        apply_outcome(
            &mut record,
            json!({"stage":"unknown","review_status":"unknown"}),
        );
        assert_eq!(record["stage"], "submitted");
        assert_eq!(record["last_known_stage"], "public");
        assert_eq!(record["item_id"], "123");
        assert!(record.get("share_url").is_none());
        assert_eq!(
            record["last_known_share_url"],
            "https://www.douyin.com/video/123"
        );
    }
    #[test]
    fn uploaded_bytes_identity_and_review_must_match_before_create() {
        let media = Media {
            path: PathBuf::from("fixture.mp4"),
            bytes: 1024,
            sha256: "a".repeat(64),
            duration: 1.0,
            width: 16,
            height: 16,
        };
        let review = json!({
            "video_sha256":media.sha256, "bytes":media.bytes,
            "account_id":"123", "title":"文案 #Rust", "visibility":"private",
            "ready_to_publish":true, "create_sent":false, "video_id":"v123"
        });
        assert!(validate_review(&review, &media, "123", "文案 #Rust", "private").is_ok());
        for (field, value) in [
            ("video_sha256", json!("b".repeat(64))),
            ("bytes", json!(2048)),
            ("account_id", json!("456")),
            ("title", json!("unexpected")),
            ("visibility", json!("public")),
            ("ready_to_publish", json!(false)),
            ("create_sent", json!(true)),
            ("video_id", json!("")),
        ] {
            let mut changed = review.clone();
            changed[field] = value;
            assert!(
                validate_review(&changed, &media, "123", "文案 #Rust", "private").is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn source_changes_are_rejected_before_upload() {
        let mut input = tempfile::NamedTempFile::new().unwrap();
        input.write_all(b"original bytes").unwrap();
        let (sha256, bytes) = hash_reader(File::open(input.path()).unwrap(), None).unwrap();
        let media = Media {
            path: input.path().to_owned(),
            bytes,
            sha256,
            duration: 1.0,
            width: 1,
            height: 1,
        };
        std::fs::write(input.path(), b"changed bytes!").unwrap();
        assert!(snapshot_media(&media).is_err());
    }
}
