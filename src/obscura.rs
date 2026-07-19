use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

pub fn manifest(version: &str, config_file: &Path) -> Value {
    json!({
        "name": "douyin-cli",
        "version": version,
        "homepage": "https://github.com/LIghtJUNction/douyin",
        "entrypoint": "douyin",
        "configFile": config_file,
        "auth": {
            "type": "oauth2",
            "login": ["douyin", "auth", "login"],
            "exchangeCode": ["douyin", "auth", "code", "--code", "<code>"],
            "status": ["douyin", "auth", "status", "--json"],
            "refresh": ["douyin", "auth", "refresh"],
            "logout": ["douyin", "auth", "logout"]
        },
        "openapi": {
            "output": "json",
            "tokenSource": "saved-oauth-config-or-env",
            "commands": {
                "userinfo": ["douyin", "api", "userinfo"],
                "commentList": ["douyin", "api", "comment-list", "--item-id", "<item_id>"],
                "commentReplies": ["douyin", "api", "comment-replies", "--item-id", "<item_id>", "--comment-id", "<comment_id>"],
                "commentReply": ["douyin", "api", "comment-reply", "--item-id", "<item_id>", "--comment-id", "<comment_id>", "--content", "<content>", "--yes"],
                "imMessageSend": ["douyin", "api", "im-message-send", "--to-user-id", "<to_user_id>", "--text", "<text>", "--yes"],
                "request": ["douyin", "api", "request", "<method>", "<path>"]
            }
        },
        "environment": {
            "clientKey": "DOUYIN_CLIENT_KEY",
            "clientSecret": "DOUYIN_CLIENT_SECRET",
            "accessToken": "DOUYIN_ACCESS_TOKEN",
            "home": "DOUYIN_HOME"
        }
    })
}

pub fn status(binary: &str) -> Value {
    match find_in_path(binary) {
        Some(path) => {
            let version = Command::new(&path)
                .arg("--version")
                .output()
                .ok()
                .map(|output| {
                    let bytes = if output.stdout.is_empty() {
                        output.stderr
                    } else {
                        output.stdout
                    };
                    String::from_utf8_lossy(&bytes).trim().to_owned()
                })
                .filter(|value| !value.is_empty());
            json!({"available": true, "binary": binary, "path": path, "version": version})
        }
        None => json!({"available": false, "binary": binary, "path": null, "version": null}),
    }
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let direct = PathBuf::from(binary);
    if direct.components().count() > 1 && direct.is_file() {
        return Some(direct);
    }
    env::split_paths(&env::var_os("PATH")?).find_map(|directory| {
        let candidate = directory.join(binary);
        candidate.is_file().then_some(candidate)
    })
}
