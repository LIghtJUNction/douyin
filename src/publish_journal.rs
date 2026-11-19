//! Durable, account-and-content keyed journal for local publishing.
//!
//! The sidecar lock inode is never replaced or removed. A submission intent is
//! persisted before any external submit action, and every later invocation must
//! reconcile it instead of automatically sending another submission.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use ring::digest::{Context, SHA256};
use serde_json::Value;

const MAX_RECORD_BYTES: u64 = 64 * 1024;
const IMMUTABLE_FIELDS: &[&str] = &[
    "schema",
    "account_id",
    "video_sha256",
    "title",
    "visibility",
    "backend",
];

#[derive(Debug)]
pub(crate) struct Journal {
    path: PathBuf,
    value: Value,
    _lock: LockGuard,
}

impl Journal {
    pub(crate) fn open(
        account_id: &str,
        video_sha256: &str,
        metadata: &Value,
    ) -> Result<Self, String> {
        Self::open_in(
            &crate::store::root().join("publish-local"),
            account_id,
            video_sha256,
            metadata,
        )
    }

    fn open_in(
        directory: &Path,
        account_id: &str,
        video_sha256: &str,
        metadata: &Value,
    ) -> Result<Self, String> {
        validate_identity(account_id, video_sha256)?;
        validate_record(metadata, account_id, video_sha256)?;
        // Reject oversized metadata before creating directories or a lock.
        let initial_bytes = encode(metadata)?;
        prepare_directory(directory)?;

        let path = record_path(directory, account_id, video_sha256);
        let lock_path = path.with_extension("lock");
        require_regular_if_present(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options.open(&lock_path).map_err(|_| "无法打开投稿记录锁")?;
        lock.try_lock()
            .map_err(|_| "另一个进程正在处理同一账号和视频，请稍后回查投稿结果")?;
        // Early errors must unlock explicitly too: a concurrent process spawn
        // can briefly inherit a descriptor, so merely closing this one is not
        // sufficient to release its shared OS lock.
        let lock = LockGuard(lock);
        protect_file(&lock.0)?;

        let exists = require_regular_if_present(&path)?;
        let value = if exists {
            let value = read_record(&path)?;
            validate_record(&value, account_id, video_sha256)?;
            if !immutable_fields_match(&value, metadata) {
                return Err(format!(
                    "同一账号和视频已有投稿记录，但标题、可见范围或后端不同：{}；请先回查已有记录，拒绝覆盖或重复投稿",
                    path.display()
                ));
            }
            value
        } else {
            if stage(metadata) != Some("prepared") {
                return Err("新投稿记录必须从 prepared 阶段开始".into());
            }
            write_new(&path, &initial_bytes)?;
            metadata.clone()
        };

        Ok(Self {
            path,
            value,
            _lock: lock,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn value(&self) -> &Value {
        &self.value
    }

    /// A recorded submit intent is sufficient to prohibit automatic resubmission,
    /// including a rejected work or a crash before the caller saw a response.
    pub(crate) fn submission_started(&self) -> bool {
        stage(&self.value) != Some("prepared")
    }

    pub(crate) fn save(&mut self, value: Value) -> Result<(), String> {
        let account_id = self.value["account_id"]
            .as_str()
            .ok_or("投稿记录缺少账号")?;
        let video_sha256 = self.value["video_sha256"]
            .as_str()
            .ok_or("投稿记录缺少视频摘要")?;
        validate_record(&value, account_id, video_sha256)?;
        if !immutable_fields_match(&self.value, &value) {
            return Err("投稿记录的账号、视频、标题、可见范围和后端不可修改".into());
        }
        if self.submission_started() && stage(&value) == Some("prepared") {
            return Err("已有提交意图的记录只能回查，不能退回 prepared 或自动重复投稿".into());
        }
        let bytes = encode(&value)?;
        let directory = self.path.parent().ok_or("投稿记录目录无效")?;
        require_regular_if_present(&self.path)?;
        let pending = write_pending(directory, &bytes)?;
        pending
            .persist(&self.path)
            .map_err(|_| "无法原子保存投稿记录")?;
        // The replacement may already be visible even if syncing the directory
        // fails. Keep the new intent in memory so an error cannot regress it.
        self.value = value;
        sync_directory(directory)
    }
}

#[derive(Debug)]
struct LockGuard(File);

impl Drop for LockGuard {
    fn drop(&mut self) {
        // This guard already exists while validating an existing record, so an
        // error there cannot leave an inherited descriptor holding the lock.
        let _ = self.0.unlock();
    }
}

fn validate_identity(account_id: &str, video_sha256: &str) -> Result<(), String> {
    if account_id.is_empty()
        || account_id.len() > 256
        || account_id
            .chars()
            .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("投稿账号 ID 无效".into());
    }
    if video_sha256.len() != 64
        || !video_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("视频 SHA-256 必须是 64 位小写十六进制".into());
    }
    Ok(())
}

fn stage(value: &Value) -> Option<&str> {
    value.get("stage").and_then(Value::as_str)
}

fn validate_record(value: &Value, account_id: &str, video_sha256: &str) -> Result<(), String> {
    if !value.is_object()
        || value["schema"].as_u64() != Some(1)
        || value["account_id"].as_str() != Some(account_id)
        || value["video_sha256"].as_str() != Some(video_sha256)
    {
        return Err("投稿记录版本、账号或视频摘要不匹配，拒绝继续".into());
    }
    let title = value["title"].as_str().ok_or("投稿记录缺少标题")?;
    if title.len() > 16 * 1024 || title.contains('\0') {
        return Err("投稿记录标题无效".into());
    }
    if !matches!(
        value["visibility"].as_str(),
        Some("public" | "private" | "friends")
    ) {
        return Err("投稿记录可见范围无效".into());
    }
    let backend = value["backend"].as_str().ok_or("投稿记录缺少后端")?;
    if backend.is_empty()
        || backend.len() > 64
        || backend.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("投稿记录后端无效".into());
    }
    if !matches!(
        stage(value),
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
    ) {
        return Err("投稿记录阶段未知或损坏；请核实已有作品，不能自动重复投稿".into());
    }
    Ok(())
}

fn immutable_fields_match(left: &Value, right: &Value) -> bool {
    IMMUTABLE_FIELDS
        .iter()
        .all(|field| left[*field] == right[*field])
}

fn record_path(directory: &Path, account_id: &str, video_sha256: &str) -> PathBuf {
    let mut hash = Context::new(&SHA256);
    hash.update(b"douyin.publish-local.v1\0");
    hash.update(&(account_id.len() as u64).to_be_bytes());
    hash.update(account_id.as_bytes());
    hash.update(video_sha256.as_bytes());
    let digest: String = hash
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    directory.join(format!("{digest}.json"))
}

fn require_regular_if_present(path: &Path) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(true),
        Ok(_) => Err("投稿记录或锁必须是普通文件，不能使用符号链接或目录".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("无法检查投稿记录路径".into()),
    }
}

fn prepare_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => return Err("投稿记录目录必须是普通目录，不能使用符号链接".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| "无法创建投稿记录目录")?;
        }
        Err(_) => return Err("无法检查投稿记录目录".into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法保护投稿记录目录")?;
    }
    Ok(())
}

fn protect_file(file: &File) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| "无法保护投稿记录文件")?;
    }
    #[cfg(not(unix))]
    let _ = file;
    Ok(())
}

fn read_record(path: &Path) -> Result<Value, String> {
    let file = File::open(path).map_err(|_| "无法读取已有投稿记录")?;
    protect_file(&file)?;
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取已有投稿记录")?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("投稿记录超过 64 KiB 限制，拒绝继续".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "投稿记录损坏，不能自动重复投稿".into())
}

fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|_| "无法序列化投稿记录")?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err("投稿记录超过 64 KiB 限制，拒绝保存".into());
    }
    Ok(bytes)
}

fn write_pending(directory: &Path, bytes: &[u8]) -> Result<tempfile::NamedTempFile, String> {
    let mut pending =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| "无法创建临时投稿记录")?;
    protect_file(pending.as_file())?;
    pending
        .write_all(bytes)
        .and_then(|_| pending.as_file().sync_all())
        .map_err(|_| "无法持久保存投稿记录")?;
    Ok(pending)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = path.parent().ok_or("投稿记录目录无效")?;
    write_pending(directory, bytes)?
        .persist_noclobber(path)
        .map_err(|_| "投稿记录已存在或无法创建，拒绝覆盖")?;
    sync_directory(directory)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| "无法持久保存投稿记录目录，请先回查已有记录")?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::process::Command;

    const ACCOUNT: &str = "123456";

    fn metadata() -> Value {
        json!({
            "schema": 1,
            "account_id": ACCOUNT,
            "video_sha256": "a".repeat(64),
            "title": "测试视频 #Rust",
            "visibility": "private",
            "backend": "app",
            "stage": "prepared"
        })
    }

    fn open(directory: &Path, value: &Value) -> Result<Journal, String> {
        Journal::open_in(directory, ACCOUNT, &"a".repeat(64), value)
    }

    #[test]
    fn same_video_with_changed_title_or_backend_cannot_create_a_second_record() {
        let directory = tempfile::tempdir().unwrap();
        let original = metadata();
        let mut journal = open(directory.path(), &original).unwrap();
        let path = journal.path().to_owned();
        let mut submitting = original.clone();
        submitting["stage"] = json!("submitting");
        journal.save(submitting).unwrap();
        drop(journal);
        let recorded = fs::read(&path).unwrap();

        for (field, replacement) in [
            ("title", json!("改了标题")),
            ("visibility", json!("public")),
            ("backend", json!("web")),
        ] {
            let mut different = original.clone();
            different[field] = replacement;
            let error = open(directory.path(), &different).unwrap_err();
            assert!(error.contains("已有投稿记录"), "{field}: {error}");
            assert!(error.contains(path.to_str().unwrap()));
            assert_eq!(fs::read(&path).unwrap(), recorded);
        }
        let reopened = open(directory.path(), &original).unwrap();
        assert_eq!(reopened.path(), path);
        assert!(reopened.submission_started());
        assert_eq!(reopened.value()["stage"], "submitting");
        assert_eq!(
            fs::read_dir(directory.path())
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
                .count(),
            1
        );
    }

    #[test]
    fn early_error_unlocks_even_when_a_descriptor_was_inherited() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("record.lock");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.try_lock().unwrap();
        // A duplicated descriptor models the shared lock lifetime of a fork.
        let inherited = file.try_clone().unwrap();
        let result: Result<(), String> = {
            let _guard = LockGuard(file);
            Err("metadata mismatch".into())
        };
        assert!(result.is_err());
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        contender.try_lock().unwrap();
        contender.unlock().unwrap();
        drop(inherited);
    }

    #[test]
    fn same_video_is_locked_across_processes_and_released_on_exit() {
        const DIRECTORY_ENV: &str = "DOUYIN_JOURNAL_LOCK_TEST_DIRECTORY";
        const EXPECT_LOCKED_ENV: &str = "DOUYIN_JOURNAL_LOCK_TEST_EXPECT_LOCKED";
        if let Some(directory) = std::env::var_os(DIRECTORY_ENV) {
            let result = open(Path::new(&directory), &metadata());
            if std::env::var_os(EXPECT_LOCKED_ENV).is_some() {
                assert!(result.unwrap_err().contains("另一个进程"));
            } else {
                assert!(result.is_ok());
            }
            return;
        }

        let directory = tempfile::tempdir().unwrap();
        let journal = open(directory.path(), &metadata()).unwrap();
        let run_child = |locked: bool| {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "publish_journal::tests::same_video_is_locked_across_processes_and_released_on_exit",
                ])
                .env(DIRECTORY_ENV, directory.path());
            if locked {
                child.env(EXPECT_LOCKED_ENV, "1");
            } else {
                child.env_remove(EXPECT_LOCKED_ENV);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        };
        run_child(true);
        drop(journal);
        run_child(false);
    }

    #[test]
    fn submitting_survives_abrupt_process_exit_and_cannot_be_reset() {
        const DIRECTORY_ENV: &str = "DOUYIN_JOURNAL_CRASH_TEST_DIRECTORY";
        if let Some(directory) = std::env::var_os(DIRECTORY_ENV) {
            let mut journal = open(Path::new(&directory), &metadata()).unwrap();
            let mut submitted_intent = journal.value().clone();
            submitted_intent["stage"] = json!("submitting");
            journal.save(submitted_intent).unwrap();
            // Deliberately skip Drop to model a process disappearing after its
            // durable submit intent, before the network result was recorded.
            std::process::exit(0);
        }

        let directory = tempfile::tempdir().unwrap();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "publish_journal::tests::submitting_survives_abrupt_process_exit_and_cannot_be_reset",
            ])
            .env(DIRECTORY_ENV, directory.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut recovered = open(directory.path(), &metadata()).unwrap();
        assert!(recovered.submission_started());
        assert_eq!(recovered.value()["stage"], "submitting");
        assert!(recovered.save(metadata()).unwrap_err().contains("不能退回"));
        assert_eq!(recovered.value()["stage"], "submitting");
    }

    #[test]
    fn all_observed_submission_outcomes_block_automatic_resubmission() {
        let directory = tempfile::tempdir().unwrap();
        let mut journal = open(directory.path(), &metadata()).unwrap();
        assert!(!journal.submission_started());
        for state in [
            "submitting",
            "unknown",
            "submitted",
            "reviewing",
            "public",
            "private",
            "friends",
            "rejected",
        ] {
            let mut value = journal.value().clone();
            value["stage"] = json!(state);
            journal.save(value).unwrap();
            assert!(journal.submission_started(), "{state}");
            assert!(journal.save(metadata()).is_err(), "{state}");
        }
    }

    #[test]
    fn corrupted_oversized_and_misbound_records_are_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let journal = open(directory.path(), &metadata()).unwrap();
        let path = journal.path().to_owned();
        drop(journal);

        let mut wrong_account = metadata();
        wrong_account["account_id"] = json!("another-account");
        let mut wrong_video = metadata();
        wrong_video["video_sha256"] = json!("b".repeat(64));
        let mut unknown_stage = metadata();
        unknown_stage["stage"] = json!("unrecognized");
        let mut wrong_schema = metadata();
        wrong_schema["schema"] = json!(2);
        let mut missing_metadata = metadata();
        missing_metadata.as_object_mut().unwrap().remove("title");
        let cases = [
            b"{broken".to_vec(),
            vec![b' '; MAX_RECORD_BYTES as usize + 1],
            serde_json::to_vec(&wrong_account).unwrap(),
            serde_json::to_vec(&wrong_video).unwrap(),
            serde_json::to_vec(&unknown_stage).unwrap(),
            serde_json::to_vec(&wrong_schema).unwrap(),
            serde_json::to_vec(&missing_metadata).unwrap(),
        ];
        for corrupted in cases {
            fs::write(&path, &corrupted).unwrap();
            assert!(open(directory.path(), &metadata()).is_err());
            assert_eq!(fs::read(&path).unwrap(), corrupted);
        }
    }

    #[test]
    fn saves_replace_a_complete_record_atomically_and_preserve_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let mut journal = open(directory.path(), &metadata()).unwrap();
        let path = journal.path().to_owned();
        let mut previous_handle = File::open(&path).unwrap();
        let mut updated = journal.value().clone();
        updated["stage"] = json!("submitting");
        updated["attempted_at"] = json!(1_795_100_000_u64);
        journal.save(updated.clone()).unwrap();

        let mut old_bytes = Vec::new();
        previous_handle.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&old_bytes).unwrap()["stage"],
            "prepared"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            updated
        );
        assert_eq!(journal.value(), &updated);

        for field in IMMUTABLE_FIELDS {
            let mut invalid = updated.clone();
            invalid[*field] = json!("changed");
            assert!(journal.save(invalid).is_err(), "{field}");
            assert_eq!(journal.value(), &updated);
        }
        let mut oversized = updated.clone();
        oversized["unexpected_large_response"] = json!("x".repeat(MAX_RECORD_BYTES as usize));
        assert!(journal.save(oversized).is_err());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(&path).unwrap()).unwrap(),
            updated
        );
    }

    #[test]
    fn record_key_uses_only_the_account_and_canonical_video_hash() {
        let directory = Path::new("records");
        let digest = "a".repeat(64);
        let path = record_path(directory, ACCOUNT, &digest);
        assert_eq!(path, record_path(directory, ACCOUNT, &digest));
        assert_ne!(path, record_path(directory, "other-account", &digest));
        assert_ne!(path, record_path(directory, ACCOUNT, &"b".repeat(64)));
        assert_eq!(path.file_stem().unwrap().len(), 64);
        assert!(!path.to_string_lossy().contains(ACCOUNT));
        assert!(validate_identity(ACCOUNT, &"A".repeat(64)).is_err());
        assert!(validate_identity("account with spaces", &digest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn records_locks_and_directory_keep_private_permissions_after_replacement() {
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().unwrap();
        let directory = temporary.path().join("publish-local");
        let mut journal = open(&directory, &metadata()).unwrap();
        let mut value = journal.value().clone();
        value["stage"] = json!("submitting");
        journal.save(value).unwrap();
        for (path, mode) in [
            (directory, 0o700),
            (journal.path().to_owned(), 0o600),
            (journal.path().with_extension("lock"), 0o600),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                mode
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_record_or_lock_is_rejected_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        for replace_lock in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let journal = open(directory.path(), &metadata()).unwrap();
            let path = if replace_lock {
                journal.path().with_extension("lock")
            } else {
                journal.path().to_owned()
            };
            drop(journal);
            let target = directory.path().join("unrelated");
            fs::write(&target, b"preserve").unwrap();
            fs::remove_file(&path).unwrap();
            symlink(&target, &path).unwrap();
            assert!(open(directory.path(), &metadata()).is_err());
            assert_eq!(fs::read(&target).unwrap(), b"preserve");
        }
    }
}
