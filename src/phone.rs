//! Own the phone service separately from an isolated Frida worker. A stalled
//! SDK call can then be killed without leaving the CLI or MCP server blocked.
use crate::adb::Adb;
use ring::rand::SecureRandom;
use serde_json::{Value, json};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const FRAME_LIMIT: u64 = 20 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(45);

pub(crate) struct PhoneClient {
    worker: Child,
    input: ChildStdin,
    output: Receiver<Result<Value, String>>,
    sequence: u64,
    _service: Service,
}

struct Service {
    _lease: File,
    adb: Adb,
    path: String,
    pid: Option<u32>,
    process: Option<Child>,
    forward: Option<u16>,
}

impl Service {
    fn start(session: &Value) -> Result<(Self, u32), String> {
        // Keep one application attachment per saved session. In particular,
        // concurrent MCP calls must not race separate injected runtimes.
        let lease = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(crate::store::root().join("phone.lock"))
            .map_err(|_| "无法创建手机通道锁")?;
        lease
            .try_lock()
            .map_err(|_| "当前登录配置的手机通道正在使用中，请稍后重试")?;
        let serial = session["device"]["serial"]
            .as_str()
            .ok_or("登录配置缺少手机序列号")?;
        let adb = Adb::connect(Some(serial))?;
        if adb.shell("am get-current-user", false)?.trim_ascii() != b"0" {
            return Err("请切换到手机主用户后使用手机通道".into());
        }
        adb.shell("am start -W -n com.ss.android.ugc.aweme/com.ss.android.ugc.aweme.splash.SplashActivity",false)?;
        let pid = String::from_utf8(adb.shell("pidof com.ss.android.ugc.aweme", false)?)
            .map_err(|_| "应用进程编码无效")?
            .trim()
            .parse()
            .map_err(|_| "无法唯一确定抖音主进程")?;
        let groups = String::from_utf8(adb.shell(&format!("cat /proc/{pid}/cgroup"), true)?)
            .map_err(|_| "应用进程分组编码无效")?;
        if let Some(path) = unified_cgroup_path(&groups)? {
            let events = adb.shell(&format!("cat /sys/fs/cgroup{path}/cgroup.events"), true)?;
            let events = String::from_utf8(events).map_err(|_| "应用冻结状态编码无效")?;
            if cgroup_frozen(&events)? {
                return Err("手机抖音进程已被系统冻结，请解锁手机并将抖音置于前台后重试".into());
            }
        }
        let abi = String::from_utf8(adb.shell("getprop ro.product.cpu.abi", false)?)
            .map_err(|_| "手机架构编码无效")?;
        let binary = crate::phone_support::server(abi.trim())?;
        let mut nonce = [0u8; 8];
        ring::rand::SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| "无法生成手机临时通道名")?;
        let suffix = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = format!("/data/local/tmp/douyin-cli-phone-{suffix}");
        adb.shell(&format!("umask 077; set -C; : > {path}"), true)?;
        let mut service = Self {
            _lease: lease,
            adb,
            path,
            pid: None,
            process: None,
            forward: None,
        };
        let upload = Command::new("adb")
            .args([
                "-s",
                service.adb.serial(),
                "shell",
                "-T",
                &format!("su -c 'cat > {}'", service.path),
            ])
            .stdin(Stdio::from(
                File::open(binary).map_err(|_| "无法读取手机支持程序")?,
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "无法启动手机程序传输")?;
        wait(upload, Duration::from_secs(60))?;
        service
            .adb
            .shell(&format!("chmod 700 {}", service.path), true)?;
        let port = 30000 + u16::from_le_bytes([nonce[0], nonce[1]]) % 10000;
        let remote = format!("su -c 'echo $$; exec {} -l 127.0.0.1:{port}'", service.path);
        let mut process = Command::new("adb")
            .args(["-s", service.adb.serial(), "shell", "-T", &remote])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "无法启动手机通道")?;
        let stdout = process.stdout.take().ok_or("缺少手机通道启动管道")?;
        service.process = Some(process);
        let (send, receive) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            let result = reader
                .by_ref()
                .take(32)
                .read_line(&mut line)
                .ok()
                .and_then(|_| line.trim().parse::<u32>().ok())
                .filter(|pid| *pid > 1);
            let _ = send.send(result);
            let _ = std::io::copy(&mut reader, &mut std::io::sink());
        });
        service.pid = Some(
            receive
                .recv_timeout(Duration::from_secs(5))
                .ok()
                .flatten()
                .ok_or("手机通道未返回启动状态")?,
        );
        let forwarded = adb_command(
            service.adb.serial(),
            &["forward", "tcp:0", &format!("tcp:{port}")],
        )?;
        service.forward = Some(
            forwarded
                .trim()
                .parse()
                .map_err(|_| "ADB 未返回有效转发端口")?,
        );
        Ok((service, pid))
    }
}

fn unified_cgroup_path(groups: &str) -> Result<Option<&str>, String> {
    let mut found = None;
    for line in groups.lines() {
        if let Some(path) = line.strip_prefix("0::") {
            if found.is_some()
                || !path.starts_with('/')
                || !path
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"/_.-".contains(&b))
                || path.split('/').any(|part| matches!(part, "." | ".."))
            {
                return Err("应用进程分组路径无效".into());
            }
            found = Some(path);
        }
    }
    Ok(found)
}

fn cgroup_frozen(events: &str) -> Result<bool, String> {
    let mut state = None;
    for line in events.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some("frozen") {
            let value = match (parts.next(), parts.next()) {
                (Some("0"), None) => false,
                (Some("1"), None) => true,
                _ => return Err("应用冻结状态无效".into()),
            };
            if state.replace(value).is_some() {
                return Err("应用冻结状态重复".into());
            }
        }
    }
    Ok(state.unwrap_or(false))
}

impl Drop for Service {
    fn drop(&mut self) {
        let kill = match self.pid {
            Some(pid) => format!(
                "if [ \"$(readlink /proc/{pid}/exe 2>/dev/null)\" = {} ]; then kill {pid}; fi; ",
                self.path
            ),
            None => format!(
                "for item in /proc/[0-9]*/exe; do if [ \"$(readlink \"$item\" 2>/dev/null)\" = {} ]; then p=${{item#/proc/}}; p=${{p%/exe}}; kill \"$p\"; fi; done; ",
                self.path
            ),
        };
        if self
            .adb
            .shell(&format!("{kill}rm -f {}", self.path), true)
            .is_err()
        {
            eprintln!("手机临时通道清理未完成：{}", self.path);
        }
        if let Some(port) = self.forward {
            let _ = adb_command(
                self.adb.serial(),
                &["forward", "--remove", &format!("tcp:{port}")],
            );
        }
        if let Some(mut process) = self.process.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
}

fn adb_command(serial: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new("adb");
    command.args(["-s", serial]).args(args);
    crate::adb::capture(&mut command)
        .and_then(|bytes| String::from_utf8(bytes).map_err(|_| "ADB 响应编码无效".into()))
}

fn wait(mut child: Child, timeout: Duration) -> Result<(), String> {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err("手机支持程序传输失败".into()),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("手机支持程序传输超时".into());
            }
        }
    }
}

fn frames(stdout: impl Read + Send + 'static) -> Receiver<Result<Value, String>> {
    let (send, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            match reader
                .by_ref()
                .take(FRAME_LIMIT + 1)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) => break,
                Ok(_) if bytes.len() as u64 <= FRAME_LIMIT => {
                    let result = serde_json::from_slice(&bytes)
                        .map_err(|_| "手机通道返回了无效协议数据".into());
                    if send.send(result).is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = send.send(Err("手机通道响应超过限制或读取失败".into()));
                    break;
                }
            }
        }
    });
    receive
}

impl PhoneClient {
    pub(crate) fn connect(session: &Value) -> Result<Self, String> {
        let (service, pid) = Service::start(session)?;
        let mut child = Command::new(std::env::current_exe().map_err(|_| "无法定位当前 CLI 程序")?)
            .arg("__phone-worker")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| "无法启动手机通道工作进程")?;
        let input = child.stdin.take().ok_or("缺少手机通道输入")?;
        let output = frames(child.stdout.take().ok_or("缺少手机通道输出")?);
        let mut client = Self {
            worker: child,
            input,
            output,
            sequence: 0,
            _service: service,
        };
        let device = &session["device"];
        let expected = json!({"userId":session["account"]["id"],"deviceId":device["deviceId"],"installId":device["installId"],"versionCode":device["versionCode"],"manifestVersionCode":device["manifestVersionCode"],"updateVersionCode":device["updateVersionCode"]});
        client.send(&json!({"id":0,"address":format!("127.0.0.1:{}",client._service.forward.ok_or("缺少手机转发端口")?),"pid":pid,"expected":expected}))?;
        client
            .receive()
            .map_err(|error| format!("手机通道初始化失败：{error}"))?;
        Ok(client)
    }
    fn send(&mut self, value: &Value) -> Result<(), String> {
        writeln!(self.input, "{value}")
            .and_then(|_| self.input.flush())
            .map_err(|_| "手机通道输入失败".into())
    }
    fn receive(&mut self) -> Result<Value, String> {
        receive_response(&self.output, self.sequence, DEADLINE)
    }
    pub(crate) fn get(&mut self, path: &str, params: &[(String, String)]) -> Result<Value, String> {
        self.sequence = self.sequence.checked_add(1).ok_or("手机通道请求序号溢出")?;
        self.send(&json!({"id":self.sequence,"path":path,"params":params}))?;
        self.receive()
            .map_err(|error| format!("手机接口请求失败：{error}"))
    }
}

const INIT_PHASES: [&str; 6] = [
    "初始化 SDK",
    "连接支持服务",
    "附加应用进程",
    "创建应用脚本",
    "加载应用脚本",
    "核对应用身份",
];

fn receive_response(
    output: &Receiver<Result<Value, String>>,
    id: u64,
    timeout: Duration,
) -> Result<Value, String> {
    let started = Instant::now();
    let mut phase = 0usize;
    loop {
        let remaining = timeout.saturating_sub(started.elapsed());
        let value = output.recv_timeout(remaining).map_err(|_| {
            if phase == 0 {
                "手机通道已断开或响应超时".to_owned()
            } else {
                format!("手机通道已断开或响应超时（{}）", INIT_PHASES[phase - 1])
            }
        })??;
        if let Some(next) = value.get("phase") {
            let next = next.as_u64().ok_or("手机通道初始化阶段无效")?;
            if id != 0
                || value["id"].as_u64() != Some(0)
                || next != (phase + 1) as u64
                || next > INIT_PHASES.len() as u64
            {
                return Err("手机通道初始化阶段或顺序无效".into());
            }
            phase = next as usize;
            continue;
        }
        return decode_response(value, id);
    }
}

fn worker_phase(phase: u8) -> Result<(), String> {
    write_frame(json!({"id":0,"phase":phase}))
}

fn decode_response(value: Value, id: u64) -> Result<Value, String> {
    if value["id"].as_u64() != Some(id) {
        return Err("手机通道响应与当前请求不匹配".into());
    }
    if value["ok"] != true {
        return Err(message(
            value["code"].as_str().unwrap_or("PHONE_GET_FAILED"),
        ));
    }
    value
        .get("value")
        .cloned()
        .ok_or_else(|| "手机通道缺少响应数据".into())
}

impl Drop for PhoneClient {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
    }
}

fn message(code: &str) -> String {
    match code {
        "ACCOUNT_CHANGED" => "手机当前账号与导入账号不同，请重新运行 auth --adb",
        "DEVICE_CHANGED" | "APP_CHANGED" => "手机设备或应用版本已变化，请重新运行 auth --adb",
        "SESSION_MISSING" => "手机应用尚未登录或未完成初始化，请打开抖音确认登录后重试",
        "BAD_REQUEST" => "手机通道拒绝无效路径、重复参数或覆盖设备身份的请求",
        "APP_BRIDGE_UNSUPPORTED" => "当前应用版本的手机通道尚未支持",
        _ => "手机通道请求失败或未返回可识别数据",
    }
    .into()
}

fn code(error: frida::Error) -> &'static str {
    if let frida::Error::RpcJsError { message } = error {
        for code in [
            "ACCOUNT_CHANGED",
            "DEVICE_CHANGED",
            "APP_CHANGED",
            "SESSION_MISSING",
            "BAD_REQUEST",
            "APP_BRIDGE_UNSUPPORTED",
        ] {
            if message.contains(code) {
                return code;
            }
        }
    }
    "PHONE_GET_FAILED"
}

struct Silent;
impl frida::ScriptHandler for Silent {
    fn on_message(&mut self, _message: frida::Message, _data: Option<Vec<u8>>) {}
}

/// Private subprocess entrypoint. The supervisor owns all phone resources and
/// enforces a deadline even if an SDK RPC never returns.
pub fn worker() -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    input
        .by_ref()
        .take(1024 * 1024)
        .read_line(&mut line)
        .map_err(|_| "工作进程初始化输入失败")?;
    let init: Value = serde_json::from_str(&line).map_err(|_| "工作进程初始化数据无效")?;
    let address = init["address"].as_str().ok_or("缺少本地转发地址")?;
    let socket: std::net::SocketAddr = address.parse().map_err(|_| "本地转发地址无效")?;
    if socket.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST) || socket.port() == 0 {
        return Err("工作进程仅连接本地 ADB 转发".into());
    }
    let pid = init["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 1)
        .ok_or("无效的手机进程")?;
    // Frida's process runtime is initialized once in this isolated subprocess.
    worker_phase(1)?;
    let frida = unsafe { frida::Frida::obtain() };
    let manager = frida::DeviceManager::obtain(&frida);
    worker_phase(2)?;
    let device = manager
        .get_remote_device(address)
        .map_err(|_| "无法连接手机通道支持服务")?;
    worker_phase(3)?;
    let session = device.attach(pid).map_err(|_| "无法附加到手机抖音进程")?;
    let source = format!(
        "{}\nconst Java=bridge;\nglobalThis.douyinReadPaths={};\n{}",
        include_str!("../assets/phone/java-bridge.js"),
        include_str!("../assets/phone/read-paths.json"),
        include_str!("../assets/phone/agent.js")
    );
    let mut options = frida::ScriptOption::new()
        .set_name("douyin-cli-readonly")
        .set_runtime(frida::ScriptRuntime::QJS);
    worker_phase(4)?;
    let mut script = session
        .create_script(&source, &mut options)
        .map_err(|_| "无法创建手机应用通道")?;
    // This handler is deliberately stateless; RPC responses are delivered to
    // the SDK's own channel. Never print application logs or error payloads.
    script
        .handle_message(Silent)
        .map_err(|_| "无法注册手机通道响应")?;
    worker_phase(5)?;
    script.load().map_err(|_| "无法加载手机应用通道")?;
    worker_phase(6)?;
    let result = script.exports.call("init", Some(json!([init["expected"]])));
    let ready = result.is_ok();
    write_result(0, result)?;
    if !ready {
        return Ok(());
    }
    loop {
        line.clear();
        let size = input
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_line(&mut line)
            .map_err(|_| "读取手机工作请求失败")?;
        if size == 0 {
            break;
        }
        if size > 1024 * 1024 {
            return Err("手机工作请求超过限制".into());
        }
        let request: Value =
            serde_json::from_str(&line).map_err(|_| "手机工作请求不是合法 JSON")?;
        let id = request["id"].as_u64().ok_or("手机工作请求缺少序号")?;
        write_result(
            id,
            script
                .exports
                .call("fetch", Some(json!([request["path"], request["params"]]))),
        )?;
    }
    let _ = script.unload();
    let _ = session.detach();
    Ok(())
}

fn write_result(id: u64, result: Result<Option<Value>, frida::Error>) -> Result<(), String> {
    let value = match result {
        Ok(value) => json!({"id":id,"ok":true,"value":value}),
        Err(error) => json!({"id":id,"ok":false,"code":code(error)}),
    };
    write_frame(value)
}

fn write_frame(value: Value) -> Result<(), String> {
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "{value}")
        .and_then(|_| stdout.flush())
        .map_err(|_| "输出手机通道响应失败".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_effective_freezing_and_validates_kernel_paths() {
        assert_eq!(
            unified_cgroup_path("0::/uid_10000/pid_123\n").unwrap(),
            Some("/uid_10000/pid_123")
        );
        assert_eq!(unified_cgroup_path("1:freezer:/apps\n").unwrap(), None);
        for bad in ["0::/../private", "0::/app;private", "0::/a\n0::/b"] {
            assert!(!unified_cgroup_path(bad).unwrap_err().contains("private"));
        }
        assert!(cgroup_frozen("populated 1\nfrozen 1\n").unwrap());
        assert!(!cgroup_frozen("populated 1\nfrozen 0\n").unwrap());
        assert!(!cgroup_frozen("populated 1\n").unwrap());
        for bad in ["frozen private", "frozen 0\nfrozen 1", "frozen 1 private"] {
            assert!(!cgroup_frozen(bad).unwrap_err().contains("private"));
        }
    }

    #[test]
    fn initialization_progress_is_bounded_and_never_echoes_worker_data() {
        let (send, recv) = mpsc::channel();
        send.send(Ok(json!({"id":0,"phase":1}))).unwrap();
        send.send(Ok(json!({"id":0,"phase":2}))).unwrap();
        drop(send);
        assert!(
            receive_response(&recv, 0, Duration::from_millis(10))
                .unwrap_err()
                .contains("连接支持服务")
        );
        let (send, recv) = mpsc::channel();
        send.send(Ok(json!({"id":0,"phase":"private"}))).unwrap();
        assert!(
            !receive_response(&recv, 0, Duration::ZERO)
                .unwrap_err()
                .contains("private")
        );
        let (send, recv) = mpsc::channel();
        send.send(Ok(json!({"id":0,"phase":2}))).unwrap();
        assert!(receive_response(&recv, 0, Duration::ZERO).is_err());
        let (send, recv) = mpsc::channel();
        send.send(Ok(json!({"id":1,"phase":1}))).unwrap();
        assert!(receive_response(&recv, 1, Duration::ZERO).is_err());
    }
    #[test]
    fn bounds_and_sanitizes_subprocess_protocol() {
        let receiver = frames(std::io::Cursor::new(
            b"{\"ok\":true,\"value\":{\"count\":3}}\nnot-json-private\n",
        ));
        assert_eq!(receiver.recv().unwrap().unwrap()["value"]["count"], 3);
        assert!(!receiver.recv().unwrap().unwrap_err().contains("private"));
        assert!(message("unknown-private").find("private").is_none());
        assert!(
            decode_response(json!({"id":1,"ok":true,"value":{"other":"private"}}), 2)
                .unwrap_err()
                .find("private")
                .is_none()
        );
        assert_eq!(
            decode_response(json!({"id":2,"ok":true,"value":{"count":3}}), 2).unwrap()["count"],
            3
        );
    }
}
