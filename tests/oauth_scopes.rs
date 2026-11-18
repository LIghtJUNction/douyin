use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_douyin"))
        .args(args)
        .env("DOUYIN_HOME", home)
        .env_remove("DOUYIN_CLIENT_KEY")
        .env_remove("DOUYIN_CLIENT_SECRET")
        .env_remove("DOUYIN_ACCESS_TOKEN")
        .env_remove("DOUYIN_COOKIE")
        .output()
        .unwrap()
}

fn save(home: &Path, value: &Value) {
    std::fs::write(home.join("oauth.json"), serde_json::to_vec(value).unwrap()).unwrap();
}

fn load(home: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(home.join("oauth.json")).unwrap()).unwrap()
}

fn status(home: &Path) -> Value {
    let result = run(home, &["auth", "status", "--oauth", "--offline"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}

#[test]
fn unfinished_reauthorization_preserves_current_tokens_granted_scopes() {
    let home = tempfile::tempdir().unwrap();
    save(
        home.path(),
        &json!({
            "clientKey": "fixture-key",
            "accessToken": "fixture-secret-token",
            "refreshToken": "fixture-secret-refresh",
            "openId": "fixture-account",
            "scopes": ["user_info"],
            "scopesVerified": true,
        }),
    );
    let result = run(
        home.path(),
        &[
            "auth",
            "login",
            "--scope",
            "user_info,video.create.bind",
            "--no-qr",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let saved = load(home.path());
    assert_eq!(saved["accessToken"], "fixture-secret-token");
    assert_eq!(saved["refreshToken"], "fixture-secret-refresh");
    assert_eq!(saved["scopes"], json!(["user_info"]));
    assert_eq!(saved["scopesVerified"], true);
    assert_eq!(
        saved["requestedScopes"],
        json!(["user_info", "video.create.bind"])
    );
    assert!(saved["pending"].is_object());
    let current = status(home.path());
    assert_eq!(current["authorized"], true);
    assert_eq!(current["scopes"], json!(["user_info"]));
    assert_eq!(
        current["requestedScopes"],
        json!(["user_info", "video.create.bind"])
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("fixture-secret"));
    assert!(!current.to_string().contains("fixture-secret"));
}

#[test]
fn legacy_scope_metadata_is_not_reported_as_a_verified_grant() {
    let home = tempfile::tempdir().unwrap();
    save(
        home.path(),
        &json!({
            "clientKey": "fixture-key",
            "accessToken": "fixture-secret-token",
            "openId": "fixture-account",
            "scopes": ["video.create.bind"],
        }),
    );
    let current = status(home.path());
    assert_eq!(current["authorized"], true);
    assert_eq!(current["scopesVerified"], false);
    assert!(current["scopes"].is_null());
    let result = run(
        home.path(),
        &[
            "auth",
            "login",
            "--scope",
            "user_info,video.create.bind",
            "--no-qr",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(status(home.path())["scopesVerified"], false);
}

#[test]
fn changing_client_clears_grants_and_invalid_login_preserves_saved_state() {
    let home = tempfile::tempdir().unwrap();
    let before = json!({
        "clientKey": "old-key",
        "clientSecret": "fixture-old-secret",
        "accessToken": "fixture-old-token",
        "openId": "old-account",
        "scopes": ["video.create.bind"],
        "scopesVerified": true,
    });
    save(home.path(), &before);
    let result = run(
        home.path(),
        &[
            "auth",
            "login",
            "--client-key",
            "new-key",
            "--code",
            " ",
            "--no-qr",
        ],
    );
    assert!(!result.status.success());
    assert_eq!(load(home.path()), before);

    let result = run(
        home.path(),
        &[
            "auth",
            "login",
            "--client-key",
            "new-key",
            "--scope",
            "user_info",
            "--no-qr",
        ],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let saved = load(home.path());
    for key in [
        "accessToken",
        "openId",
        "clientSecret",
        "scopes",
        "scopesVerified",
    ] {
        assert!(saved.get(key).is_none(), "{key}");
    }
    assert_eq!(saved["requestedScopes"], json!(["user_info"]));
    assert_eq!(status(home.path())["authorized"], false);
}

#[test]
fn logout_clears_verified_grants_in_current_and_legacy_storage() {
    let home = tempfile::tempdir().unwrap();
    let config = json!({
        "clientKey": "fixture-key",
        "clientSecret": "fixture-client-secret",
        "accessToken": "fixture-token",
        "refreshToken": "fixture-refresh",
        "openId": "fixture-account",
        "scopes": ["video.create.bind"],
        "scopesVerified": true,
        "requestedScopes": ["video.create.bind"],
    });
    save(home.path(), &config);
    std::fs::create_dir(home.path().join("config")).unwrap();
    let legacy_path = home.path().join("config/settings.json");
    std::fs::write(
        &legacy_path,
        serde_json::to_vec(&json!({"custom": true, "openapi": config})).unwrap(),
    )
    .unwrap();
    let result = run(home.path(), &["auth", "logout"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let saved = load(home.path());
    let legacy: Value = serde_json::from_slice(&std::fs::read(&legacy_path).unwrap()).unwrap();
    assert_eq!(legacy["custom"], true);
    for value in [&saved, &legacy["openapi"]] {
        for key in [
            "accessToken",
            "refreshToken",
            "openId",
            "scopes",
            "scopesVerified",
        ] {
            assert!(value.get(key).is_none(), "{key}");
        }
        assert_eq!(value["clientKey"], "fixture-key");
        assert_eq!(value["requestedScopes"], json!(["video.create.bind"]));
    }
    let current = status(home.path());
    assert_eq!(current["authorized"], false);
    assert_eq!(current["scopesVerified"], false);
    assert!(current["scopes"].is_null());
}
