use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const PACKAGE: &str = "com.ss.android.ugc.aweme";

/// All requests retain the explicitly selected transport. Never pick the first
/// phone in a multi-device installation or use the host's identity as a fallback.
pub struct Adb {
    serial: String,
}

impl Adb {
    pub fn connect(serial: Option<&str>) -> Result<Self, String> {
        let output = capture(Command::new("adb").args(["devices", "-l"]))?;
        let serial = select_device(&String::from_utf8_lossy(&output), serial)?;
        Ok(Self { serial })
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// Only internal, constant commands should be passed to the remote shell.
    /// Output and stderr can contain credentials and must not appear in errors.
    pub(crate) fn shell(&self, command: &str, root: bool) -> Result<Vec<u8>, String> {
        capture(&mut self.remote_shell(command, root))
    }

    fn remote_shell(&self, command: &str, root: bool) -> Command {
        let mut process = Command::new("adb");
        // The shell v2 protocol carries the remote exit status and separates
        // stderr. exec-out does neither reliably; a failed cat can look like a
        // successful read containing its error message. -T keeps binary data raw.
        process.args([
            "-s",
            &self.serial,
            "shell",
            "-T",
            &remote_command(command, root),
        ]);
        process
    }

    /// PackageManager exposes the app's API and update versions separately from
    /// the APK version. This small read-only helper does not attach to the app.
    pub(crate) fn package_metadata(&self) -> Result<Value, String> {
        use ring::rand::SecureRandom;
        let mut nonce = [0u8; 8];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "无法生成临时元数据文件名")?;
        let suffix = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = format!("/data/local/tmp/douyin-cli-metadata-{suffix}.dex");
        let mut file = tempfile::tempfile().map_err(|_| "无法创建元数据程序临时文件")?;
        file.write_all(include_bytes!("../assets/device-metadata/classes.dex"))
            .and_then(|_| file.seek(SeekFrom::Start(0)).map(|_| ()))
            .map_err(|_| "无法准备元数据程序")?;
        self.shell(&format!("umask 077; set -C; : > {path}"), true)
            .map_err(|_| "无法在手机上创建临时元数据文件")?;
        let result = (|| {
            capture_input(
                &mut self.remote_shell(&format!("cat > {path}"), true),
                Stdio::from(file),
            )
            .map_err(|_| "无法向手机传送元数据读取程序")?;
            let command = format!(
                "chmod 400 {path} && CLASSPATH={path} app_process /system/bin org.lightjunction.douyin.DeviceMetadata"
            );
            let bytes = self
                .shell(&command, true)
                .map_err(|_| "无法执行手机安装包元数据读取程序")?;
            serde_json::from_slice(&bytes)
                .map_err(|_| "无法解析手机安装包元数据，未导入登录态".into())
        })();
        if self.shell(&format!("rm -f {path}"), true).is_err() {
            eprintln!("无法清理手机上的临时元数据读取程序：{path}");
        }
        result
    }

    fn property(&self, key: &str) -> Result<String, String> {
        let bytes = self.shell(&format!("getprop {key}"), false)?;
        let value = String::from_utf8(bytes).map_err(|_| "设备属性编码无效")?;
        let value = value.trim();
        if value.is_empty() {
            return Err(format!("缺少设备属性 {key}"));
        }
        Ok(value.to_owned())
    }

    pub fn inspect(&self) -> Result<Value, String> {
        let installed = self.shell(&format!("pm path {PACKAGE}"), false)?;
        if !installed.starts_with(b"package:") {
            return Err("所选设备未安装抖音主应用".into());
        }
        let root = self
            .shell("id -u", true)
            .is_ok_and(|bytes| bytes.trim_ascii() == b"0");
        Ok(json!({
            "serial": self.serial,
            "package": PACKAGE,
            "model": self.property("ro.product.model")?,
            "brand": self.property("ro.product.brand")?,
            "androidRelease": self.property("ro.build.version.release")?,
            "sdk": self.property("ro.build.version.sdk")?,
            "root": root,
            "authenticated": false
        }))
    }
}

fn remote_command(command: &str, root: bool) -> String {
    let quoted = command.replace('\'', "'\\''");
    format!("{} -c '{quoted}'", if root { "su" } else { "sh" })
}

pub(crate) fn capture(command: &mut Command) -> Result<Vec<u8>, String> {
    capture_input(command, Stdio::null())
}

fn capture_input(command: &mut Command, input: Stdio) -> Result<Vec<u8>, String> {
    const MAX_BYTES: usize = 16 * 1024 * 1024;
    let mut child = command
        .stdin(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "无法运行 adb，请检查安装和 PATH")?;
    let stdout = child.stdout.take().ok_or("无法读取 ADB 输出")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break Err("ADB 请求超时或连接中断，请检查设备授权".to_owned());
            }
        }
    };
    let bytes = reader
        .join()
        .map_err(|_| "ADB 输出读取线程异常")?
        .map_err(|_| "读取 ADB 输出失败")?;
    if !status?.success() {
        return Err("设备命令执行失败，请检查连接、手机解锁和 root 授权".into());
    }
    if bytes.len() > MAX_BYTES {
        return Err("ADB 响应超过大小限制".into());
    }
    Ok(bytes)
}

fn select_device(list: &str, requested: Option<&str>) -> Result<String, String> {
    let devices: Vec<_> = list
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            (state == "device").then_some(serial)
        })
        .collect();
    if let Some(serial) = requested {
        if devices.contains(&serial) {
            return Ok(serial.to_owned());
        }
        return Err("指定设备未连接、离线或尚未授权".into());
    }
    match devices.as_slice() {
        [serial] => Ok((*serial).to_owned()),
        [] => Err("没有已授权的 ADB 设备".into()),
        _ => Err("存在多个设备，请通过 --serial 或 ANDROID_SERIAL 指定".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{capture, capture_input, remote_command, select_device};

    #[test]
    fn never_uses_an_unauthorized_device() {
        assert!(
            select_device(
                "List of devices attached\na unauthorized\nb offline\n",
                None
            )
            .is_err()
        );
    }

    #[test]
    fn multiple_devices_require_selection() {
        let list = "List of devices attached\na device product:p\nb device\n";
        assert!(select_device(list, None).is_err());
        assert_eq!(select_device(list, Some("b")).unwrap(), "b");
        assert!(select_device(list, Some("c")).is_err());
    }

    #[test]
    fn accepts_usb_and_tcp_transports() {
        assert_eq!(
            select_device("List of devices attached\n192.168.1.2:5555 device\n", None).unwrap(),
            "192.168.1.2:5555"
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_preserves_binary_and_nonzero_exit_status() {
        use std::process::Command;
        let command = remote_command(
            "printf '\\000\\377'; printf 'do not expose stderr' >&2",
            false,
        );
        assert_eq!(
            capture(Command::new("sh").args(["-c", &command])).unwrap(),
            vec![0, 255]
        );
        let command = remote_command("printf 'credential must not be returned'; exit 7", false);
        let error = capture(Command::new("sh").args(["-c", &command])).unwrap_err();
        assert!(!error.contains("credential"));
    }
    #[cfg(unix)]
    #[test]
    fn helper_upload_passes_binary_stdin_with_explicit_eof() {
        use std::io::{Seek, SeekFrom, Write};
        use std::process::{Command, Stdio};
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&[0, 255, 10, 128]).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        assert_eq!(
            capture_input(&mut Command::new("cat"), Stdio::from(file)).unwrap(),
            [0, 255, 10, 128]
        );
    }
}
