use std::path::Path;
use std::process::{Command, Output};

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_douyin"))
        .args(args)
        .current_dir(home)
        .env("DOUYIN_HOME", home)
        .env("PATH", home.join("no-external-tools"))
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("all_proxy")
        .env_remove("no_proxy")
        .output()
        .unwrap()
}

#[test]
fn invalid_login_bounds_do_not_create_pending_credentials_or_contact_network() {
    let home = tempfile::tempdir().unwrap();
    for (flag, value) in [
        ("--timeout", "0"),
        ("--timeout", "4"),
        ("--timeout", "1801"),
        ("--poll-interval", "0"),
        ("--poll-interval", "4"),
        ("--poll-interval", "61"),
    ] {
        let result = run(home.path(), &["auth", "creator-login", flag, value]);
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(
            error.contains("timeout") || error.contains("poll-interval"),
            "{error}"
        );
        assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
    }
}

#[test]
fn missing_resume_never_starts_a_replacement_login() {
    let home = tempfile::tempdir().unwrap();
    let result = run(
        home.path(),
        &[
            "auth",
            "creator-login",
            "--resume",
            "missing-private-session",
            "--qr-only",
        ],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!result.stderr.is_empty());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn native_login_help_is_available_without_credentials_or_external_tools() {
    let home = tempfile::tempdir().unwrap();
    let result = run(home.path(), &["auth", "creator-login", "--help"]);
    assert!(result.status.success());
    let output = String::from_utf8_lossy(&result.stdout);
    for flag in [
        "--timeout",
        "--poll-interval",
        "--qr-only",
        "--resume",
        "--poll-once",
    ] {
        assert!(output.contains(flag), "{flag}");
    }
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}

#[test]
fn mutually_exclusive_qr_modes_fail_before_creating_session_material() {
    let home = tempfile::tempdir().unwrap();
    let result = run(
        home.path(),
        &["auth", "creator-login", "--qr-only", "--poll-once"],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert_eq!(std::fs::read_dir(home.path()).unwrap().count(), 0);
}
