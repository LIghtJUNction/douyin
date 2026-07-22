use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::{Args, Subcommand};
use qrcode::{Color, QrCode};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Map, Value, json};

use crate::cookie;
use crate::openapi::{OpenApiClient, RequestSpec};
use crate::settings;

#[derive(Debug, Args)]
pub struct AuthArgs {
    #[command(subcommand)]
    command: AuthCommand,
}

#[derive(Debug, Subcommand)]
enum AuthCommand {
    /// 通过官方 OAuth 授权接入账号
    Login {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: Option<String>,
        #[arg(long, env = "DOUYIN_CLIENT_SECRET")]
        client_secret: Option<String>,
        #[arg(long)]
        redirect_uri: Option<String>,
        #[arg(long)]
        scope: Vec<String>,
        #[arg(long)]
        code: Option<String>,
        #[arg(long, conflicts_with = "no_qr")]
        qr: bool,
        #[arg(long, conflicts_with = "qr")]
        no_qr: bool,
        #[arg(long)]
        listen: bool,
        #[arg(long, default_value = "127.0.0.1")]
        callback_host: String,
        #[arg(long, default_value_t = 8787, value_parser = clap::value_parser!(u16).range(1..))]
        callback_port: u16,
        #[arg(long, default_value_t = 300, value_parser = clap::value_parser!(u64).range(1..=3600))]
        timeout: u64,
    },
    /// 用官方 OAuth code 换取并保存 token
    Code {
        #[arg(long)]
        code: String,
        #[arg(long, env = "DOUYIN_CLIENT_SECRET")]
        client_secret: Option<String>,
    },
    /// 刷新已保存的官方 access_token
    Refresh,
    /// 检查官方授权状态
    Status {
        #[arg(long)]
        json: bool,
    },
    /// 删除已保存的官方 OAuth token
    Logout,
    /// 保存网页端 Cookie，用于搜索、评论和下载等网页端采集
    CookieLogin {
        #[arg(long, env = "DOUYIN_COOKIE")]
        cookie: String,
    },
    /// 检查已保存 Cookie 格式，并尝试确认网页登录态
    CookieStatus {
        /// 只检查本地 Cookie 格式，不访问网络
        #[arg(long)]
        offline: bool,
    },
    /// 删除已保存的网页端 Cookie
    CookieLogout,
}

pub fn run(args: AuthArgs) -> Result<(), String> {
    match args.command {
        AuthCommand::Login {
            client_key,
            client_secret,
            redirect_uri,
            scope,
            code,
            qr: _,
            no_qr,
            listen,
            callback_host,
            callback_port,
            timeout,
        } => login(LoginOptions {
            client_key,
            client_secret,
            redirect_uri,
            scopes: scope,
            code,
            show_qr: !no_qr,
            listen,
            callback_host,
            callback_port,
            timeout,
        }),
        AuthCommand::Code {
            code,
            client_secret,
        } => exchange_code(&code, client_secret),
        AuthCommand::Refresh => refresh(),
        AuthCommand::Status { json } => status(json),
        AuthCommand::Logout => logout(),
        AuthCommand::CookieLogin { cookie } => cookie_login(&cookie),
        AuthCommand::CookieStatus { offline } => cookie_status(offline),
        AuthCommand::CookieLogout => cookie_logout(),
    }
}

struct LoginOptions {
    client_key: Option<String>,
    client_secret: Option<String>,
    redirect_uri: Option<String>,
    scopes: Vec<String>,
    code: Option<String>,
    show_qr: bool,
    listen: bool,
    callback_host: String,
    callback_port: u16,
    timeout: u64,
}

fn login(options: LoginOptions) -> Result<(), String> {
    let data = settings::load().map_err(|error| error.to_string())?;
    let saved = settings::openapi(&data);
    let client_key = options
        .client_key
        .or_else(|| saved_string(&saved, "clientKey"))
        .ok_or_else(|| missing_client_key(options.show_qr))?;
    let client_secret = options
        .client_secret
        .or_else(|| saved_string(&saved, "clientSecret"));
    let mut redirect_uri = options
        .redirect_uri
        .or_else(|| saved_string(&saved, "redirectUri"));
    let scopes = if options.scopes.is_empty() {
        saved
            .get("scopes")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|values: &Vec<String>| !values.is_empty())
            .unwrap_or_else(|| vec!["user_info".to_owned()])
    } else {
        options.scopes
    };
    if options.listen {
        redirect_uri = Some(format!(
            "http://{}:{}/callback",
            options.callback_host, options.callback_port
        ));
    }
    let redirect_uri =
        redirect_uri.ok_or_else(|| "缺少 redirect_uri，请传入 --redirect-uri".to_owned())?;
    let state = (options.listen && options.code.is_none())
        .then(random_state)
        .transpose()?;
    let client = OpenApiClient::new()?;
    let url = client.authorize_url(&client_key, &redirect_uri, &scopes, state.as_deref())?;
    println!("请在浏览器打开以下官方授权链接：\n{url}");
    if options.show_qr {
        print_qr(&url)?;
    }

    let mut code = options.code;
    if options.listen && code.is_none() {
        println!("正在等待授权回调: {redirect_uri}");
        code = Some(wait_for_code(
            &options.callback_host,
            options.callback_port,
            state.as_deref(),
            Duration::from_secs(options.timeout),
        )?);
    }
    let mut updates = Map::from_iter([
        ("clientKey".to_owned(), json!(client_key)),
        (
            "clientSecret".to_owned(),
            json!(client_secret.clone().unwrap_or_default()),
        ),
        ("redirectUri".to_owned(), json!(redirect_uri)),
        ("scopes".to_owned(), json!(scopes)),
    ]);
    if let Some(code) = code {
        let secret =
            client_secret.ok_or_else(|| "使用 code 换 token 需要 --client-secret".to_owned())?;
        let response = client.access_token(&client_key, &secret, &code)?;
        updates.extend(extract_token_fields(&response));
        print_json(&response)?;
    } else {
        println!("授权完成后运行：douyin auth code --code 授权码");
    }
    save_openapi(updates)?;
    println!(
        "官方授权配置已保存: {}",
        settings::settings_file().display()
    );
    Ok(())
}

fn exchange_code(code: &str, client_secret: Option<String>) -> Result<(), String> {
    let data = settings::load().map_err(|error| error.to_string())?;
    let saved = settings::openapi(&data);
    let client_key = saved_string(&saved, "clientKey")
        .ok_or_else(|| "缺少 client_key，请先运行 douyin auth login".to_owned())?;
    let secret = client_secret
        .or_else(|| saved_string(&saved, "clientSecret"))
        .ok_or_else(|| "缺少 client_secret，请传入 --client-secret".to_owned())?;
    let response = OpenApiClient::new()?.access_token(&client_key, &secret, code)?;
    let mut updates = extract_token_fields(&response);
    updates.insert("clientSecret".to_owned(), json!(secret));
    save_openapi(updates)?;
    print_json(&response)?;
    println!("官方 token 已保存: {}", settings::settings_file().display());
    Ok(())
}

fn refresh() -> Result<(), String> {
    let data = settings::load().map_err(|error| error.to_string())?;
    let saved = settings::openapi(&data);
    let client_key = saved_string(&saved, "clientKey");
    let refresh_token = saved_string(&saved, "refreshToken");
    let (Some(client_key), Some(refresh_token)) = (client_key, refresh_token) else {
        return Err("缺少 client_key 或 refresh_token，请重新授权".to_owned());
    };
    let response = OpenApiClient::new()?.refresh_token(&client_key, &refresh_token)?;
    save_openapi(extract_token_fields(&response))?;
    print_json(&response)?;
    println!("官方 token 已刷新");
    Ok(())
}

fn status(json_output: bool) -> Result<(), String> {
    let data = settings::load().map_err(|error| error.to_string())?;
    let saved = settings::openapi(&data);
    let token = saved_string(&saved, "accessToken");
    let open_id = saved_string(&saved, "openId");
    let authorized = token.is_some() && open_id.is_some();
    let mut output = json!({
        "authorized": authorized,
        "connected": false,
        "configFile": settings::settings_file(),
        "openId": open_id.clone().unwrap_or_default(),
        "scopes": saved.get("scopes").cloned().unwrap_or_else(|| json!([]))
    });
    let (Some(token), Some(open_id)) = (token, open_id) else {
        if json_output {
            return print_json(&output);
        }
        println!("未完成官方授权");
        return Ok(());
    };
    if !json_output {
        println!("已保存官方授权: {}", settings::settings_file().display());
        println!("open_id: {open_id}");
        println!("正在检查官方 OpenAPI 连通性...");
    }
    match OpenApiClient::new()?.request(RequestSpec {
        method: "GET",
        path: "/oauth/userinfo/",
        token: Some(&token),
        params: Some(HashMap::from([("open_id".to_owned(), open_id)])),
        auth_required: true,
        ..RequestSpec::default()
    }) {
        Ok(userinfo) => {
            output["connected"] = json!(true);
            output["userinfo"] = userinfo.clone();
            print_json(if json_output { &output } else { &userinfo })
        }
        Err(error) if json_output => {
            output["error"] = json!(error);
            print_json(&output)?;
            Err("官方 OpenAPI 连通性检查失败".to_owned())
        }
        Err(error) => Err(format!("官方 OpenAPI 连通性检查失败: {error}")),
    }
}

fn logout() -> Result<(), String> {
    save_openapi(Map::from_iter([
        ("accessToken".to_owned(), json!("")),
        ("refreshToken".to_owned(), json!("")),
        ("openId".to_owned(), json!("")),
        ("expiresIn".to_owned(), json!(0)),
    ]))?;
    println!("已清除官方授权 token");
    Ok(())
}

fn cookie_login(value: &str) -> Result<(), String> {
    let value = value.trim();
    if !cookie::validate(value) {
        return Err("Cookie 格式校验失败，未保存".to_owned());
    }
    let mut data = settings::load().map_err(|error| error.to_string())?;
    data["cookie"] = json!(value);
    settings::save(&data).map_err(|error| error.to_string())?;
    println!("Cookie 已保存: {}", settings::settings_file().display());
    Ok(())
}

fn cookie_status(offline: bool) -> Result<(), String> {
    let data = settings::load().map_err(|error| error.to_string())?;
    let value = data
        .get("cookie")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if value.is_empty() {
        println!("未保存 Cookie");
        return Ok(());
    }
    if !cookie::validate(value) {
        return Err(format!(
            "已保存 Cookie，但格式无效: {}",
            settings::settings_file().display()
        ));
    }
    if offline {
        println!("Cookie 格式有效: {}", settings::settings_file().display());
        return Ok(());
    }
    println!("正在确认网页登录态...");
    match cookie::probe(value) {
        Ok(true) => {
            println!(
                "Cookie 网页登录态有效: {}",
                settings::settings_file().display()
            );
            Ok(())
        }
        Ok(false) => Err("Cookie 已保存，但网页登录态无效或已过期".to_owned()),
        Err(error) => Err(format!(
            "Cookie 已保存且格式有效，但无法确认网页登录态: {error}\n可运行 douyin auth cookie-status --offline 仅检查本地格式"
        )),
    }
}

fn cookie_logout() -> Result<(), String> {
    let mut data = settings::load().map_err(|error| error.to_string())?;
    data["cookie"] = json!("");
    settings::save(&data).map_err(|error| error.to_string())?;
    println!("已清除 Cookie");
    Ok(())
}

fn save_openapi(updates: Map<String, Value>) -> Result<(), String> {
    let mut data = settings::load().map_err(|error| error.to_string())?;
    let openapi = data
        .get_mut("openapi")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| "openapi 配置格式无效".to_owned())?;
    openapi.extend(updates);
    settings::save(&data).map_err(|error| error.to_string())
}

fn extract_token_fields(data: &Value) -> Map<String, Value> {
    let source = data
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(data);
    [
        ("access_token", "accessToken"),
        ("refresh_token", "refreshToken"),
        ("open_id", "openId"),
        ("expires_in", "expiresIn"),
    ]
    .into_iter()
    .filter_map(|(source_key, target_key)| {
        source
            .get(source_key)
            .cloned()
            .map(|value| (target_key.to_owned(), value))
    })
    .collect()
}

fn saved_string(values: &Map<String, Value>, key: &str) -> Option<String> {
    values
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn missing_client_key(show_qr: bool) -> String {
    let qr_line =
        show_qr.then_some("--qr 只会把官方 OAuth 授权链接渲染成二维码，仍然需要 client_key。\n");
    format!(
        "当前命令是官方 OpenAPI OAuth 授权，需要开放平台 client_key。\n{}这不是网页端 Cookie 扫码登录，不能直接生成可保存 Cookie 的登录二维码。\n\n可选方案：\n  1. 官方 OpenAPI：传入 --client-key，或设置 DOUYIN_CLIENT_KEY。\n  2. 网页端采集：从浏览器复制 Cookie 后运行：\n     douyin auth cookie-login --cookie 'sessionid=...; ttwid=...'",
        qr_line.unwrap_or("")
    )
}

fn random_state() -> Result<String, String> {
    let mut bytes = [0_u8; 18];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "无法生成 OAuth state".to_owned())?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn print_qr(value: &str) -> Result<(), String> {
    let code = QrCode::new(value.as_bytes()).map_err(|error| error.to_string())?;
    let width = code.width();
    println!();
    for y in (0..width).step_by(2) {
        let mut line = String::new();
        for x in 0..width {
            let top = code[(x, y)] == Color::Dark;
            let bottom = y + 1 < width && code[(x, y + 1)] == Color::Dark;
            line.push(match (top, bottom) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        println!(" {line} ");
    }
    println!();
    Ok(())
}

fn wait_for_code(
    host: &str,
    port: u16,
    expected_state: Option<&str>,
    timeout: Duration,
) -> Result<String, String> {
    let listener = TcpListener::bind((host, port))
        .map_err(|_| format!("无法监听 {host}:{port}，请换一个 --callback-port"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    while started.elapsed() < timeout {
        match listener.accept() {
            Ok((mut stream, _)) => return handle_callback(&mut stream, expected_state),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("等待授权回调超时，未获取到 code".to_owned())
}

fn handle_callback(stream: &mut TcpStream, expected_state: Option<&str>) -> Result<String, String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut buffer = [0_u8; 8192];
    let count = stream
        .read(&mut buffer)
        .map_err(|error| error.to_string())?;
    let request = String::from_utf8_lossy(&buffer[..count]);
    let target = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| "授权回调请求无效".to_owned())?;
    let url = reqwest::Url::parse(&format!("http://localhost{target}"))
        .map_err(|error| format!("授权回调 URL 无效: {error}"))?;
    if url.path() != "/callback" {
        send_html(stream, 404, "未找到回调路径")?;
        return Err("授权回调路径无效".to_owned());
    }
    let params: HashMap<_, _> = url.query_pairs().into_owned().collect();
    if let Some(error) = params
        .get("error")
        .or_else(|| params.get("error_description"))
    {
        send_html(stream, 400, "授权失败，可以关闭此页面并返回终端。")?;
        return Err(format!("授权失败: {error}"));
    }
    if expected_state
        .is_some_and(|expected| params.get("state").map(String::as_str) != Some(expected))
    {
        send_html(stream, 400, "state 不匹配。")?;
        return Err("授权回调 state 不匹配，已拒绝".to_owned());
    }
    let Some(code) = params.get("code") else {
        send_html(stream, 400, "回调缺少 code。")?;
        return Err("授权回调缺少 code".to_owned());
    };
    send_html(stream, 200, "授权完成，可以关闭此页面并返回终端。")?;
    Ok(code.to_owned())
}

fn send_html(stream: &mut TcpStream, status: u16, body: &str) -> Result<(), String> {
    let content =
        format!("<!doctype html><meta charset='utf-8'><title>Douyin CLI</title><p>{body}</p>");
    let reason = if status == 200 { "OK" } else { "Bad Request" };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{content}",
        content.len()
    )
    .map_err(|error| error.to_string())
}

fn print_json(value: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{extract_token_fields, missing_client_key};
    use serde_json::json;

    #[test]
    fn extracts_nested_token_fields() {
        let fields = extract_token_fields(&json!({"data": {
            "access_token": "access", "refresh_token": "refresh", "open_id": "open", "expires_in": 1
        }}));
        assert_eq!(fields["accessToken"], "access");
        assert_eq!(fields["openId"], "open");
    }

    #[test]
    fn missing_key_message_explains_cookie_alternative() {
        let message = missing_client_key(true);
        assert!(message.contains("官方 OpenAPI OAuth 授权"));
        assert!(message.contains("--qr 只会把官方 OAuth 授权链接渲染成二维码"));
        assert!(message.contains("douyin auth cookie-login --cookie"));
    }
}
