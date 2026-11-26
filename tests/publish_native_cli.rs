use std::path::Path;
use std::process::{Command, Output};

const MISSING_FILE: &str = "private-missing-native-publish-source.mp4";

#[test]
fn unsupported_native_settings_stop_even_prepare_only_before_media_or_auth() {
    let home = tempfile::tempdir().unwrap();
    for setting in [
        vec!["--cover", "cover.jpg"],
        vec!["--ai-generated"],
        vec!["--short-title", "短标题"],
    ] {
        let mut args = vec![
            "publish",
            "native",
            MISSING_FILE,
            "--title",
            "AI 文案",
            "--prepare-only",
        ];
        args.extend(setting);
        let out = run(home.path(), &args);
        let error = failure(&out);
        assert!(error.contains("options_check"), "{error}");
        assert!(!error.contains("ffprobe"));
        unchanged(home.path());
    }
}

fn command(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_douyin"));
    command
        .args(args)
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
        .env_remove("ANDROID_SERIAL")
        .env_remove("DOUYIN_CLIENT_KEY")
        .env_remove("DOUYIN_CLIENT_SECRET")
        .env_remove("DOUYIN_ACCESS_TOKEN")
        .env_remove("DOUYIN_COOKIE")
        .env_remove("DOUYIN_NATIVE_TRANSPORT");
    command
}

fn run(home: &Path, args: &[&str]) -> Output {
    command(home, args).output().unwrap()
}

fn failure(output: &Output) -> String {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(!error.is_empty());
    error
}

fn unchanged(home: &Path) {
    assert_eq!(std::fs::read_dir(home).unwrap().count(), 0);
}

#[test]
fn native_submission_requires_yes_before_files_credentials_or_external_tools() {
    let home = tempfile::tempdir().unwrap();
    for subcommand in ["app", "native"] {
        let args = vec!["publish", subcommand, MISSING_FILE, "--title", "测试投稿"];
        let output = run(home.path(), &args);
        let error = failure(&output);
        assert!(error.contains("--yes"), "{error}");
        assert!(!error.contains(MISSING_FILE), "{error}");
        assert!(!error.contains("OAuth"), "{error}");
        assert!(!error.contains("ffprobe"), "{error}");
        unchanged(home.path());
    }
}

#[test]
fn both_preview_modes_bypass_submission_confirmation_but_still_validate_files() {
    let home = tempfile::tempdir().unwrap();
    for subcommand in ["app", "native"] {
        for preview in ["--dry-run", "--prepare-only"] {
            let output = run(
                home.path(),
                &[
                    "publish",
                    subcommand,
                    MISSING_FILE,
                    "--title",
                    "只验证",
                    preview,
                ],
            );
            let error = failure(&output);
            assert!(error.contains("文件") || error.contains("视频"), "{error}");
            assert!(!error.contains("需要显式 --yes"), "{error}");
            unchanged(home.path());
        }
    }
}

#[test]
fn title_is_required_nonblank_and_counted_in_unicode_characters() {
    let home = tempfile::tempdir().unwrap();
    let missing_title = run(home.path(), &["publish", "app", MISSING_FILE, "--dry-run"]);
    assert_eq!(missing_title.status.code(), Some(2));
    assert!(failure(&missing_title).contains("--title"));

    let overlong = "中".repeat(1001);
    for title in ["", "   ", overlong.as_str()] {
        let output = run(
            home.path(),
            &[
                "publish",
                "app",
                MISSING_FILE,
                "--title",
                title,
                "--dry-run",
            ],
        );
        let error = failure(&output);
        assert!(
            error.contains("标题") || error.contains("--title"),
            "{error}"
        );
        unchanged(home.path());
    }

    // A 1000-character title uses 3000 UTF-8 bytes and should reach file checks.
    let boundary = "中".repeat(1000);
    let output = run(
        home.path(),
        &[
            "publish",
            "app",
            MISSING_FILE,
            "--title",
            &boundary,
            "--dry-run",
        ],
    );
    let error = failure(&output);
    assert!(error.contains("文件") || error.contains("视频"), "{error}");
    assert!(!error.contains("1000"), "{error}");
    unchanged(home.path());
}

#[test]
fn topics_reject_blank_whitespace_and_controls_but_accept_repetition() {
    let home = tempfile::tempdir().unwrap();
    for topic in ["", " ", "two words", "bad\ttopic", "bad\ntopic"] {
        let output = run(
            home.path(),
            &[
                "publish",
                "app",
                MISSING_FILE,
                "--title",
                "测试",
                "--topic",
                topic,
                "--dry-run",
            ],
        );
        let error = failure(&output);
        assert!(
            error.contains("话题") || error.contains("--topic"),
            "{error}"
        );
        unchanged(home.path());
    }

    let output = run(
        home.path(),
        &[
            "publish",
            "app",
            MISSING_FILE,
            "--title",
            "测试",
            "--topic",
            "Rust",
            "--topic",
            "命令行",
            "--dry-run",
        ],
    );
    let error = failure(&output);
    assert!(error.contains("文件") || error.contains("视频"), "{error}");
    unchanged(home.path());
}

#[test]
fn timeout_and_visibility_enums_are_rejected_before_execution() {
    let home = tempfile::tempdir().unwrap();
    for (flag, invalid) in [
        ("--timeout", "0"),
        ("--timeout", "3601"),
        ("--visibility", "everyone"),
    ] {
        let output = run(
            home.path(),
            &[
                "publish",
                "app",
                MISSING_FILE,
                "--title",
                "测试",
                flag,
                invalid,
                "--dry-run",
            ],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(failure(&output).contains(flag));
        unchanged(home.path());
    }
}

#[test]
fn incompatible_preview_options_fail_before_file_checks() {
    let home = tempfile::tempdir().unwrap();
    let output = run(
        home.path(),
        &[
            "publish",
            "native",
            MISSING_FILE,
            "--title",
            "测试",
            "--dry-run",
            "--prepare-only",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    let error = failure(&output);
    assert!(error.contains("--prepare-only"), "{error}");
    unchanged(home.path());
}

#[test]
fn native_route_has_no_ui_backend_or_device_selection() {
    let home = tempfile::tempdir().unwrap();
    for flag in ["--backend", "--serial", "--browser-url"] {
        let output = run(
            home.path(),
            &[
                "publish",
                "native",
                MISSING_FILE,
                "--title",
                "测试",
                flag,
                "unused-fixture",
                "--dry-run",
            ],
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(failure(&output).contains(flag));
        unchanged(home.path());
    }
    let output = command(
        home.path(),
        &[
            "publish",
            "native",
            MISSING_FILE,
            "--title",
            "测试",
            "--dry-run",
        ],
    )
    .env("ANDROID_SERIAL", "unconnected-fixture")
    .output()
    .unwrap();
    let error = failure(&output);
    assert!(error.contains("文件") || error.contains("视频"), "{error}");
    assert!(!error.contains("ADB") && !error.contains("ANDROID_SERIAL"));
    unchanged(home.path());
}

#[test]
fn empty_files_and_directories_are_rejected_without_loading_an_account() {
    let home = tempfile::tempdir().unwrap();
    let input_directory = tempfile::tempdir().unwrap();
    let files = tempfile::tempdir().unwrap();
    let empty = files.path().join("empty.mp4");
    std::fs::write(&empty, []).unwrap();
    for path in [empty.as_path(), input_directory.path()] {
        for subcommand in ["app", "native"] {
            let output = run(
                home.path(),
                &[
                    "publish",
                    subcommand,
                    path.to_str().unwrap(),
                    "--title",
                    "测试",
                    "--dry-run",
                ],
            );
            let error = failure(&output);
            assert!(error.contains("文件") || error.contains("视频"), "{error}");
            assert!(!error.contains("OAuth"), "{error}");
            assert!(!error.contains("ffprobe"), "{error}");
            unchanged(home.path());
        }
    }
}

#[test]
fn account_and_upload_timeout_alias_are_validated_before_credentials() {
    let home = tempfile::tempdir().unwrap();
    for account in ["", "two accounts", "line\nbreak"] {
        let output = run(
            home.path(),
            &[
                "publish",
                "native",
                MISSING_FILE,
                "--title",
                "测试",
                "--account",
                account,
                "--dry-run",
            ],
        );
        assert!(failure(&output).contains("--account"));
        unchanged(home.path());
    }
    let output = run(
        home.path(),
        &[
            "publish",
            "native",
            MISSING_FILE,
            "--title",
            "测试",
            "--upload-timeout",
            "0",
            "--dry-run",
        ],
    );
    assert_eq!(output.status.code(), Some(2));
    unchanged(home.path());
}
