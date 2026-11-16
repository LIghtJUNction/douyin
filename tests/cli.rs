use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_douyin"))
        .args(args)
        .env("DOUYIN_HOME", home)
        .env("ANDROID_SERIAL", "unconnected-test-device")
        .env_remove("DOUYIN_CLIENT_KEY")
        .env_remove("DOUYIN_CLIENT_SECRET")
        .env_remove("DOUYIN_ACCESS_TOKEN")
        .env_remove("DOUYIN_COOKIE")
        .output()
        .unwrap()
}

#[test]
fn live_permissions_rejects_cookie_only_sessions_without_network_or_phone() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("session.json"),
        serde_json::to_vec(&json!({
            "schema":1,"source":"adb","device":{"deviceId":"1234","installId":"5678"},
            "account":{"id":"123"},"cookies":[],"userAgent":"fixture-agent"
        }))
        .unwrap(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_douyin"))
        .args(["live", "permissions"])
        .env("DOUYIN_HOME", home.path())
        .env("DOUYIN_NATIVE_TRANSPORT", "direct")
        .env("PATH", "/nonexistent")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("需要原生 Token")
    );
}

#[test]
fn obs_changes_require_confirmation_before_files_or_connections() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec![
            "live",
            "obs",
            "configure-push",
            "--stream-file",
            "/nonexistent/private",
        ],
        vec![
            "live",
            "obs",
            "add-source",
            "--url-file",
            "/nonexistent/private",
            "--scene",
            "test",
        ],
        vec!["live", "obs", "stop"],
    ] {
        let output = run(home.path(), &args);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("需要 --yes"));
        assert!(!error.contains("private"));
    }
}

#[test]
fn live_mutations_require_confirmation_before_reading_private_files() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec!["live", "create", "--params-file", "missing-private-input"],
        vec![
            "live",
            "continue",
            "--resolution-key",
            "fixture",
            "--output",
            "missing-private-input",
        ],
        vec![
            "live",
            "anchor-status",
            "--session-file",
            "missing-private-input",
            "--status",
            "play",
        ],
        vec!["live", "finish", "--session-file", "missing-private-input"],
        vec![
            "live",
            "heartbeat",
            "--session-file",
            "missing-private-input",
            "--interval",
            "10",
            "--count",
            "2",
        ],
        vec!["live", "leave", "--room-id", "123"],
    ] {
        let output = run(home.path(), &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(output.stdout.is_empty());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("需要 --yes"), "{error}");
        assert!(!error.contains("missing-private-input"));
    }
}

#[test]
fn live_create_refuses_existing_output_before_authentication() {
    let home = tempfile::tempdir().unwrap();
    let params = home.path().join("create.json");
    let output = home.path().join("private-session.json");
    std::fs::write(&params, r#"{"title":"fixture"}"#).unwrap();
    std::fs::write(&output, b"private-existing-data").unwrap();
    let result = run(
        home.path(),
        &[
            "live",
            "create",
            "--params-file",
            params.to_str().unwrap(),
            "--output",
            output.to_str().unwrap(),
            "--yes",
        ],
    );
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("拒绝覆盖"));
    assert!(!String::from_utf8_lossy(&result.stderr).contains("private-existing-data"));
    assert_eq!(std::fs::read(output).unwrap(), b"private-existing-data");
}

#[test]
fn live_heartbeat_requires_a_bounded_explicit_schedule() {
    let home = tempfile::tempdir().unwrap();
    for (interval, count) in [("0", "1"), ("3601", "1"), ("1", "0"), ("1", "10001")] {
        let output = run(
            home.path(),
            &[
                "live",
                "heartbeat",
                "--session-file",
                "missing-private-input",
                "--interval",
                interval,
                "--count",
                count,
                "--yes",
            ],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
    }
}

#[test]
fn manual_oauth_setup_is_isolated_and_does_not_print_secrets() {
    let home = tempfile::tempdir().unwrap();
    let output = run(
        home.path(),
        &[
            "auth",
            "login",
            "--client-key",
            "fixture-key",
            "--client-secret",
            "fixture-secret-not-for-stdout",
            "--redirect-uri",
            "http://127.0.0.1:8787/callback",
            "--scope",
            "user_info,item.comment",
            "--no-qr",
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("/platform/oauth/connect/"));
    assert!(!stdout.contains("fixture-secret"));
    let saved: Value =
        serde_json::from_slice(&std::fs::read(home.path().join("oauth.json")).unwrap()).unwrap();
    assert_eq!(saved["clientKey"], "fixture-key");
    assert_eq!(saved["scopes"], json!(["user_info", "item.comment"]));
    assert!(
        saved["pending"]["state"]
            .as_str()
            .is_some_and(|s| s.len() >= 40)
    );
    assert!(!home.path().join("session.json").exists());
    let status = run(home.path(), &["auth", "status", "--oauth", "--offline"]);
    assert!(status.status.success());
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["authorized"], false);
    assert_eq!(status["connected"], false);
}

#[test]
fn phone_worker_rejects_nonlocal_input_before_initializing_the_sdk() {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_douyin"))
        .arg("__phone-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(
        input,
        "{}",
        json!({"address":"192.0.2.1:27043","pid":1,"expected":{"private":"must-not-print"}})
    )
    .unwrap();
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("仅连接本地 ADB 转发"));
    assert!(!error.contains("must-not-print"));
}

#[test]
fn cookie_commands_keep_auth_types_separate_and_do_not_resurrect_legacy_state() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    std::fs::create_dir(&config).unwrap();
    std::fs::write(config.join("settings.json"),json!({"cookie":"sessionid=legacy-secret","custom":true,"openapi":{"clientKey":"retained-key"}}).to_string()).unwrap();
    let status = run(home.path(), &["auth", "cookie-status", "--offline"]);
    assert!(status.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["configured"],
        true
    );
    assert!(!String::from_utf8_lossy(&status.stdout).contains("legacy-secret"));
    let file = home.path().join("input-cookie.txt");
    std::fs::write(
        &file,
        "sessionid=new-private-fixture; ttwid=public-fixture\n",
    )
    .unwrap();
    let login = run(
        home.path(),
        &[
            "auth",
            "cookie-login",
            "--cookie-file",
            file.to_str().unwrap(),
        ],
    );
    assert!(
        login.status.success(),
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(!String::from_utf8_lossy(&login.stdout).contains("new-private-fixture"));
    assert_eq!(
        serde_json::from_slice::<Value>(&login.stdout).unwrap()["onlineVerified"],
        false
    );
    assert!(!home.path().join("session.json").exists());
    assert!(!home.path().join("oauth.json").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(home.path().join("cookie.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(
        run(home.path(), &["auth", "cookie-logout"])
            .status
            .success()
    );
    let status = run(home.path(), &["auth", "cookie-status", "--offline"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["configured"],
        false
    );
    let legacy: Value =
        serde_json::from_slice(&std::fs::read(config.join("settings.json")).unwrap()).unwrap();
    assert!(legacy.get("cookie").is_none());
    assert_eq!(legacy["custom"], true);
    assert_eq!(legacy["openapi"]["clientKey"], "retained-key");
}

#[test]
fn logout_clears_legacy_tokens_without_deleting_other_settings() {
    let home = tempfile::tempdir().unwrap();
    let config = home.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let legacy = json!({"cookie":"legacy-cookie","custom":true,"openapi":{"clientKey":"key","clientSecret":"secret",
        "accessToken":"token","refreshToken":"refresh","openId":"account"}});
    std::fs::write(config.join("settings.json"), legacy.to_string()).unwrap();
    std::fs::write(home.path().join("session.json"), "{}").unwrap();
    let status = run(home.path(), &["auth", "status", "--oauth", "--offline"]);
    assert!(status.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["authorized"],
        true
    );
    assert!(run(home.path(), &["auth", "logout"]).status.success());
    let legacy: Value =
        serde_json::from_slice(&std::fs::read(config.join("settings.json")).unwrap()).unwrap();
    assert_eq!(legacy["cookie"], "legacy-cookie");
    assert_eq!(legacy["custom"], true);
    assert_eq!(legacy["openapi"]["clientKey"], "key");
    for key in ["accessToken", "refreshToken", "openId"] {
        assert!(legacy["openapi"].get(key).is_none());
    }
    assert!(!home.path().join("session.json").exists());
    let status = run(home.path(), &["auth", "status", "--oauth", "--offline"]);
    assert_eq!(
        serde_json::from_slice::<Value>(&status.stdout).unwrap()["authorized"],
        false
    );
}

#[test]
fn generic_write_and_credential_header_override_are_rejected_before_sending() {
    let home = tempfile::tempdir().unwrap();
    let write = run(
        home.path(),
        &[
            "api",
            "request",
            "POST",
            "/__douyin_cli_no_network_test__/",
            "--token",
            "fixture-token",
        ],
    );
    assert!(!write.status.success());
    assert!(String::from_utf8_lossy(&write.stderr).contains("--yes"));
    assert!(!String::from_utf8_lossy(&write.stderr).contains("fixture-token"));
    let header = run(
        home.path(),
        &[
            "api",
            "request",
            "GET",
            "/__douyin_cli_no_network_test__/",
            "--token",
            "fixture-token",
            "--header",
            "access-token=other-fixture",
        ],
    );
    assert!(!header.status.success());
    assert!(String::from_utf8_lossy(&header.stderr).contains("自定义请求头不能覆盖"));
    let external = run(
        home.path(),
        &[
            "api",
            "request",
            "GET",
            "https://evil.invalid/test",
            "--token",
            "fixture-token",
        ],
    );
    assert!(!external.status.success());
    assert!(String::from_utf8_lossy(&external.stderr).contains("同源"));
}

#[test]
fn help_never_prints_secret_environment_values() {
    let home = tempfile::tempdir().unwrap();
    for args in [
        vec!["api", "userinfo", "--help"],
        vec!["api", "client-token", "--help"],
        vec!["auth", "login", "--help"],
        vec!["auth", "code", "--help"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_douyin"))
            .args(args)
            .env("DOUYIN_HOME", home.path())
            .env("DOUYIN_CLIENT_SECRET", "fixture-client-secret")
            .env("DOUYIN_ACCESS_TOKEN", "fixture-access-token")
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("fixture-client-secret"));
        assert!(!text.contains("fixture-access-token"));
    }
}
