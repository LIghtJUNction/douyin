use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

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

#[test]
fn writes_require_confirmation_before_reading_files_or_auth() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec!["publish", "video", "private-missing-video"],
        vec!["publish", "resume", "private-missing-receipt"],
    ] {
        let out = run(home.path(), &args);
        assert!(!out.status.success());
        assert!(out.stdout.is_empty());
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(error.contains("--yes"), "{error}");
        assert!(!error.contains("private-missing"));
    }
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn invalid_options_and_files_fail_before_authorization() {
    let home = tempfile::tempdir().unwrap();
    for value in ["NaN", "inf", "-1"] {
        let arg = format!("--cover-tsp={value}");
        let out = run(home.path(), &["publish", "video", "missing", &arg, "--yes"]);
        assert!(!out.status.success());
        assert!(String::from_utf8_lossy(&out.stderr).contains("封面时间"));
    }
    let video = home.path().join("empty.mp4");
    std::fs::write(&video, []).unwrap();
    let out = run(
        home.path(),
        &["publish", "video", video.to_str().unwrap(), "--yes"],
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("不能为空"));
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&video)
        .unwrap();
    file.set_len(4 * 1024 * 1024 * 1024 + 1).unwrap();
    let out = run(
        home.path(),
        &["publish", "video", video.to_str().unwrap(), "--yes"],
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("4GB"));
    let out = run(
        home.path(),
        &["publish", "video", home.path().to_str().unwrap(), "--yes"],
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("必须是文件"));
}

#[test]
fn dry_run_checks_authorization_without_network_or_receipt_writes() {
    let home = tempfile::tempdir().unwrap();
    let video = home.path().join("video.mp4");
    std::fs::write(&video, b"local preflight fixture").unwrap();
    let oauth = home.path().join("oauth.json");
    let mut auth = json!({
        "clientKey":"fixture-app", "openId":"fixture-account",
        "accessToken":"private-access-token", "refreshToken":"private-refresh-token",
        "scopes":["video.create.bind"], "scopesVerified":true, "expiresAt":u64::MAX
    });
    std::fs::write(&oauth, serde_json::to_vec(&auth).unwrap()).unwrap();
    let out = run(
        home.path(),
        &["publish", "video", video.to_str().unwrap(), "--dry-run"],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["dry_run"], true);
    assert_eq!(value["upload_mode"], "single");
    assert_eq!(value["scope"], "video.create.bind");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("private-"));
    assert!(!home.path().join("publish").exists());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 2);

    for (key, value, expected) in [
        ("scopesVerified", json!(false), "尚未核实"),
        ("scopes", json!(["user_info"]), "未授予"),
        ("expiresAt", json!(0), "已过期"),
    ] {
        let original = auth[key].clone();
        auth[key] = value;
        std::fs::write(&oauth, serde_json::to_vec(&auth).unwrap()).unwrap();
        let out = run(
            home.path(),
            &["publish", "video", video.to_str().unwrap(), "--dry-run"],
        );
        assert!(!out.status.success());
        let error = String::from_utf8_lossy(&out.stderr);
        assert!(error.contains(expected), "{error}");
        assert!(!error.contains("private-"));
        auth[key] = original;
    }
}

#[test]
fn submitted_receipts_are_readable_and_resume_never_reposts() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("receipt.json");
    let receipt = json!({
        "schema":1,"stage":"submitted","client_key":"fixture-app","open_id":"fixture-account",
        "bytes":10,"options":{"title":"test","visibility":"private","cover_tsp":null},
        "video_id":"uploaded-video","item_id":"created-item"
    });
    std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    for args in [
        vec!["publish", "status", path.to_str().unwrap()],
        vec!["publish", "resume", path.to_str().unwrap(), "--yes"],
    ] {
        let out = run(home.path(), &args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(value["stage"], "submitted");
        assert_eq!(value["item_id"], "created-item");
    }
    assert_eq!(
        serde_json::from_slice::<Value>(&std::fs::read(&path).unwrap()).unwrap(),
        receipt
    );
}

#[test]
fn uncertain_receipt_requires_verification_before_loading_auth() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join("receipt.json");
    let receipt = json!({
        "schema":1,"stage":"unknown","client_key":"fixture-app","open_id":"fixture-account",
        "bytes":10,"options":{"title":"test","visibility":"private","cover_tsp":null},
        "video_id":"uploaded-video","item_id":null
    });
    std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
    let out = run(
        home.path(),
        &["publish", "resume", path.to_str().unwrap(), "--yes"],
    );
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("--retry-create"), "{error}");
    assert!(!error.contains("OAuth"));
}
