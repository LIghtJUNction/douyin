use std::env;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde_json::{Map, Value, json};

use crate::fs_utils;

pub fn config_root() -> PathBuf {
    if let Some(path) = env::var_os("DOUYIN_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }

    if cfg!(windows)
        && let Some(path) = env::var_os("APPDATA").filter(|value| !value.is_empty())
    {
        return PathBuf::from(path).join("douyin-cli");
    }

    if let Some(path) = env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(path).join("douyin-cli");
    }

    home_dir().join(".config").join("douyin-cli")
}

pub fn settings_file() -> PathBuf {
    config_root().join("config").join("settings.json")
}

pub fn load() -> io::Result<Value> {
    let path = settings_file();
    let mut settings = defaults();
    match fs::read_to_string(&path) {
        Ok(text) => {
            let stored: Value = serde_json::from_str(&text).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("配置文件不是合法 JSON（{}）: {error}", path.display()),
                )
            })?;
            merge(&mut settings, stored);
            Ok(settings)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(settings),
        Err(error) => Err(error),
    }
}

pub fn save(settings: &Value) -> io::Result<()> {
    let path = settings_file();
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("配置文件路径缺少父目录"))?;
    fs::create_dir_all(parent)?;

    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    fs_utils::atomic_write(&path, &bytes)
}

pub fn openapi(settings: &Value) -> Map<String, Value> {
    settings
        .get("openapi")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

pub fn defaults() -> Value {
    json!({
        "cookie": "",
        "openapi": {
            "clientKey": "",
            "clientSecret": "",
            "redirectUri": "",
            "scopes": [],
            "accessToken": "",
            "refreshToken": "",
            "openId": "",
            "expiresIn": 0
        },
        "userAgent": "",
        "downloadPath": home_dir().join("Downloads").join("douyin").to_string_lossy(),
        "enableIncrementalFetch": true,
        "enableDownloadTitle": false,
        "enableDownloadCover": false,
        "filenameFields": ["id", "title"],
        "filenameSeparator": "_"
    })
}

fn home_dir() -> PathBuf {
    env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::merge;
    use serde_json::json;

    #[test]
    fn merge_preserves_defaults_and_unknown_fields() {
        let mut base = json!({"cookie": "", "openapi": {"openId": ""}});
        merge(
            &mut base,
            json!({"cookie": "sessionid=x", "openapi": {"custom": true}}),
        );
        assert_eq!(base["cookie"], "sessionid=x");
        assert_eq!(base["openapi"]["openId"], "");
        assert_eq!(base["openapi"]["custom"], true);
    }
}
