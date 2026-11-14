use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn root() -> PathBuf {
    if let Some(path) = std::env::var_os("DOUYIN_HOME").filter(|p| !p.is_empty()) {
        return path.into();
    }
    if let Some(path) = std::env::var_os(if cfg!(windows) {
        "APPDATA"
    } else {
        "XDG_CONFIG_HOME"
    }) {
        return PathBuf::from(path).join("douyin-cli");
    }
    PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config/douyin-cli")
}

pub fn session_file() -> PathBuf {
    root().join("session.json")
}

pub fn load() -> Result<Value, String> {
    let bytes = fs::read(session_file())
        .map_err(|_| "没有可读取的设备登录配置，请运行 douyin auth --adb")?;
    serde_json::from_slice(&bytes).map_err(|_| "设备登录配置损坏".into())
}

pub fn save(value: &Value) -> Result<(), String> {
    save_at(&session_file(), value).map_err(|_| "无法安全保存登录配置".into())
}

pub(crate) fn save_at(path: &Path, value: &Value) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("缺少配置目录"))?;
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    let mut pending = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut pending, value)?;
    pending.write_all(b"\n")?;
    pending.as_file().sync_all()?;
    pending.persist(path).map_err(|e| e.error)?;
    Ok(())
}

pub fn logout() -> Result<(), String> {
    match fs::remove_file(session_file()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("无法删除本地设备登录配置".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn replaces_complete_credentials_without_exposing_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/session.json");
        save_at(&path, &json!({"account": "old", "secret": "old"})).unwrap();
        save_at(&path, &json!({"account": "new"})).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(value, json!({"account":"new"}));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }
}
