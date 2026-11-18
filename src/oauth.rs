use std::fs;
use std::net::IpAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::Args;
use reqwest::Method;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value, json};
use tiny_http::{Header, Response, Server};
use url::Url;

use crate::{
    openapi::{OpenApiClient, Request},
    store,
};

#[derive(Args)]
pub struct LoginArgs {
    #[arg(long, env = "DOUYIN_CLIENT_KEY")]
    client_key: Option<String>,
    #[arg(long, env = "DOUYIN_CLIENT_SECRET", hide_env_values = true)]
    client_secret: Option<String>,
    #[arg(long)]
    redirect_uri: Option<String>,
    #[arg(long, value_delimiter = ',')]
    scope: Vec<String>,
    #[arg(long, conflicts_with = "listen")]
    code: Option<String>,
    #[arg(long, conflicts_with = "no_qr")]
    qr: bool,
    #[arg(long, conflicts_with = "qr")]
    no_qr: bool,
    #[arg(long)]
    listen: bool,
    #[arg(long, default_value = "127.0.0.1")]
    callback_host: IpAddr,
    #[arg(long,default_value_t=8787,value_parser=clap::value_parser!(u16).range(1..))]
    callback_port: u16,
    #[arg(long,default_value_t=300,value_parser=clap::value_parser!(u64).range(1..=3600))]
    timeout: u64,
}

pub fn load() -> Result<Value, String> {
    let path = store::root().join("oauth.json");
    match fs::read(&path) {
        Ok(bytes) => {
            let value: Value =
                serde_json::from_slice(&bytes).map_err(|_| "OAuth 配置不是合法 JSON")?;
            if !value.is_object() {
                return Err("OAuth 配置必须是对象".into());
            }
            Ok(value)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let legacy = store::root().join("config/settings.json");
            match fs::read(legacy) {
                Ok(bytes) => {
                    let value: Value =
                        serde_json::from_slice(&bytes).map_err(|_| "旧版配置不是合法 JSON")?;
                    Ok(value
                        .get("openapi")
                        .filter(|v| v.is_object())
                        .cloned()
                        .unwrap_or_else(|| json!({})))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
                Err(_) => Err("无法读取旧版 OAuth 配置".into()),
            }
        }
        Err(_) => Err("无法读取 OAuth 配置".into()),
    }
}

fn save(value: &Value) -> Result<(), String> {
    store::save_at(&store::root().join("oauth.json"), value)
        .map_err(|_| "无法安全保存 OAuth 配置".into())
}
pub(crate) fn field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value[key].as_str().filter(|v| !v.trim().is_empty())
}
fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    field(value, key).ok_or_else(|| format!("OAuth 缺少 {key}，请先运行 auth login"))
}

pub fn login(args: LoginArgs) -> Result<Value, String> {
    if args.listen && !args.callback_host.is_loopback() {
        return Err("OAuth 本机监听必须绑定回环地址".into());
    }
    let mut saved = load()?;
    let key = args
        .client_key
        .or_else(|| field(&saved, "clientKey").map(str::to_owned))
        .ok_or("缺少 --client-key 或 DOUYIN_CLIENT_KEY")?;
    if field(&saved, "clientKey").is_some_and(|old| old != key) {
        clear_tokens(&mut saved);
        saved
            .as_object_mut()
            .ok_or("OAuth 配置无效")?
            .remove("clientSecret");
    }
    let secret = args
        .client_secret
        .or_else(|| field(&saved, "clientSecret").map(str::to_owned));
    let scopes = if args.scope.is_empty() {
        vec!["user_info".into()]
    } else {
        args.scope
    };
    let redirect = args
        .redirect_uri
        .or_else(|| {
            (!args.listen)
                .then(|| field(&saved, "redirectUri").map(str::to_owned))
                .flatten()
        })
        .unwrap_or_else(|| {
            format!(
                "http://{}:{}/callback",
                if args.callback_host.is_ipv6() {
                    format!("[{}]", args.callback_host)
                } else {
                    args.callback_host.to_string()
                },
                args.callback_port
            )
        });
    saved["clientKey"] = json!(key);
    saved["redirectUri"] = json!(redirect);
    saved["requestedScopes"] = json!(scopes);
    if let Some(secret) = secret {
        saved["clientSecret"] = json!(secret);
    }
    if let Some(code) = args.code {
        return exchange(saved, &code);
    }
    let state = random_state()?;
    let client = OpenApiClient::new()?;
    let url = client.authorize_url(required(&saved, "clientKey")?, &redirect, &scopes, &state)?;
    let server = if args.listen {
        let callback = Url::parse(&redirect).map_err(|_| "OAuth 回调地址无效")?;
        if callback.scheme() != "http"
            || callback
                .host_str()
                .and_then(|s| s.trim_matches(['[', ']']).parse::<IpAddr>().ok())
                != Some(args.callback_host)
            || callback.port_or_known_default() != Some(args.callback_port)
            || callback.query().is_some()
        {
            return Err("--listen 的回调地址必须与本机监听地址、端口一致，且不包含查询参数".into());
        }
        Some(
            Server::http((args.callback_host, args.callback_port))
                .map_err(|_| "无法绑定 OAuth 本机回调端口")?,
        )
    } else {
        None
    };
    saved["pending"] = json!({"state":state,"expiresAt":now()?.saturating_add(args.timeout)});
    save(&saved)?;
    println!("{url}");
    if !args.no_qr {
        let code = qrcode::QrCode::new(url.as_bytes()).map_err(|_| "无法生成 OAuth 二维码")?;
        eprintln!(
            "{}",
            code.render::<qrcode::render::unicode::Dense1x2>().build()
        );
    }
    if let Some(server) = server {
        eprintln!("正在等待本机 OAuth 回调...");
        let path = Url::parse(&redirect)
            .map_err(|_| "OAuth 回调地址无效")?
            .path()
            .to_owned();
        let code = wait_callback(&server, &path, &state, Duration::from_secs(args.timeout))?;
        exchange(saved, &code)
    } else {
        Ok(json!({"configured":true,"authorized":false,"next":"douyin auth code --code <code>"}))
    }
}

pub fn code(code: &str, secret: Option<String>) -> Result<Value, String> {
    let mut saved = load()?;
    if let Some(secret) = secret {
        saved["clientSecret"] = json!(secret);
    }
    exchange(saved, code)
}
fn exchange(mut saved: Value, code: &str) -> Result<Value, String> {
    if code.trim().is_empty() {
        return Err("OAuth code 不能为空".into());
    }
    let response = OpenApiClient::new()?.token(
        "/oauth/access_token/",
        &[
            ("client_key", required(&saved, "clientKey")?),
            ("client_secret", required(&saved, "clientSecret")?),
            ("code", code),
            ("grant_type", "authorization_code"),
        ],
    )?;
    apply_token_response(&mut saved, &response, true)?;
    save(&saved)?;
    Ok(summary(&saved, false))
}
pub fn refresh() -> Result<Value, String> {
    let mut saved = load()?;
    let response = OpenApiClient::new()?.token(
        "/oauth/refresh_token/",
        &[
            ("client_key", required(&saved, "clientKey")?),
            ("refresh_token", required(&saved, "refreshToken")?),
            ("grant_type", "refresh_token"),
        ],
    )?;
    apply_token_response(&mut saved, &response, false)?;
    save(&saved)?;
    Ok(summary(&saved, false))
}

fn apply_token_response(saved: &mut Value, response: &Value, fresh: bool) -> Result<(), String> {
    if !saved.is_object() {
        return Err("OAuth 配置无效".into());
    }
    let data = &response["data"];
    if field(data, "access_token").is_none()
        || (field(data, "open_id").is_none() && (fresh || field(saved, "openId").is_none()))
    {
        return Err("OAuth 响应缺少 access_token 或 open_id，未更新配置".into());
    }
    if !fresh
        && let (Some(old), Some(new)) = (field(saved, "openId"), field(data, "open_id"))
        && old != new
    {
        return Err("刷新返回了其他账号的 token，未更新配置".into());
    }
    let expires = data
        .get("expires_in")
        .map(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.parse().ok())
                .ok_or("OAuth 响应的 expires_in 无效，未更新配置")
        })
        .transpose()?;
    let scopes = data
        .get("scope")
        .map(|value| {
            value
                .as_str()
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .collect::<Vec<_>>()
                })
                .ok_or("OAuth 响应的 scope 无效，未更新配置")
        })
        .transpose()?;
    let updated_at = now()?;
    if fresh {
        clear_tokens(saved);
    }
    for (remote, local) in [
        ("access_token", "accessToken"),
        ("refresh_token", "refreshToken"),
        ("open_id", "openId"),
    ] {
        if let Some(value) = field(data, remote) {
            saved[local] = json!(value);
        }
    }
    if let Some(expires) = expires {
        saved["expiresIn"] = json!(expires);
        saved["expiresAt"] = json!(updated_at.saturating_add(expires));
    }
    if let Some(scopes) = scopes {
        saved["scopes"] = json!(scopes);
        saved["scopesVerified"] = json!(true);
    }
    saved["updatedAt"] = json!(updated_at);
    saved
        .as_object_mut()
        .ok_or("OAuth 配置无效")?
        .remove("pending");
    Ok(())
}

pub fn status(offline: bool) -> Result<Value, String> {
    let saved = load()?;
    let mut result = summary(&saved, false);
    if offline || result["authorized"] != true {
        return Ok(result);
    }
    let mut request = Request::new(Method::GET, "/oauth/userinfo/");
    request.token = Some(required(&saved, "accessToken")?);
    request
        .params
        .push(("open_id".into(), required(&saved, "openId")?.into()));
    let response = OpenApiClient::new()?.request(request)?;
    if let Some(id) = response["data"]["open_id"].as_str()
        && Some(id) != field(&saved, "openId")
    {
        return Err("官方接口账号与保存的 open_id 不一致".into());
    }
    result["connected"] = json!(true);
    Ok(result)
}

pub fn logout() -> Result<(), String> {
    let mut saved = load()?;
    clear_tokens(&mut saved);
    save(&saved)?;
    let legacy = store::root().join("config/settings.json");
    if legacy.exists() {
        let mut value: Value =
            serde_json::from_slice(&fs::read(&legacy).map_err(|_| "无法读取旧版配置以清除 token")?)
                .map_err(|_| "旧版配置损坏，无法清除 token")?;
        if value["openapi"].is_object() {
            clear_tokens(&mut value["openapi"]);
            store::save_at(&legacy, &value).map_err(|_| "无法清除旧版 OAuth token")?;
        }
    }
    Ok(())
}
fn clear_tokens(saved: &mut Value) {
    if let Some(map) = saved.as_object_mut() {
        for key in [
            "accessToken",
            "refreshToken",
            "openId",
            "expiresIn",
            "expiresAt",
            "updatedAt",
            "pending",
            "scopes",
            "scopesVerified",
        ] {
            map.remove(key);
        }
    }
}
fn summary(saved: &Value, connected: bool) -> Value {
    let scopes_verified = saved["scopesVerified"] == true;
    json!({
        "source": "oauth",
        "authorized": field(saved, "accessToken").is_some() && field(saved, "openId").is_some(),
        "connected": connected,
        "openId": saved["openId"],
        "requestedScopes": saved["requestedScopes"],
        "scopes": if scopes_verified { saved["scopes"].clone() } else { Value::Null },
        "scopesVerified": scopes_verified,
        "expiresAt": saved["expiresAt"],
    })
}

fn random_state() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "生成 OAuth state 失败")?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
fn now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|_| "系统时间无效".into())
}

fn callback_code(target: &str, path: &str, state: &str) -> Result<Option<String>, String> {
    if !target.starts_with('/') || target.starts_with("//") {
        return Err("回调请求目标无效".into());
    }
    let url = Url::parse(&format!("http://localhost{target}")).map_err(|_| "回调 URL 无效")?;
    if url.path() != path {
        return Ok(None);
    }
    let fields: Vec<_> = url.query_pairs().collect();
    let states: Vec<_> = fields
        .iter()
        .filter(|(key, _)| key == "state")
        .map(|(_, value)| value.as_ref())
        .collect();
    if states.as_slice() != [state] {
        return Err("OAuth 回调 state 不匹配".into());
    }
    if fields.iter().any(|(key, _)| key == "error") {
        return Err("OAuth 授权未完成或被拒绝".into());
    }
    let codes: Vec<_> = fields
        .iter()
        .filter(|(key, _)| key == "code")
        .map(|(_, value)| value.as_ref())
        .collect();
    match codes.as_slice() {
        [code] if !code.trim().is_empty() => Ok(Some((*code).into())),
        _ => Err("OAuth 回调缺少唯一有效 code".into()),
    }
}

fn wait_callback(
    server: &Server,
    path: &str,
    state: &str,
    timeout: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let Some(request) = server
            .recv_timeout(Duration::from_millis(200))
            .map_err(|_| "OAuth 回调监听失败")?
        else {
            continue;
        };
        let outcome = if request.method() != &tiny_http::Method::Get {
            Err("仅接受 GET 回调".into())
        } else {
            callback_code(request.url(), path, state)
        };
        let (status, text) = match &outcome {
            Ok(Some(_)) => (200, "登录回调已收到，可以关闭此页面。"),
            Ok(None) => (404, "没有此回调路径。"),
            Err(_) => (400, "回调无效，请返回终端重新检查授权流程。"),
        };
        let header = Header::from_bytes(b"Content-Type", b"text/plain; charset=utf-8")
            .map_err(|_| "无法生成回调响应")?;
        let responded = request.respond(
            Response::from_string(text)
                .with_status_code(status)
                .with_header(header),
        );
        if let Ok(Some(code)) = outcome {
            return Ok(code);
        }
        if matches!(&outcome,Err(error) if error=="OAuth 授权未完成或被拒绝") {
            return Err("OAuth 授权未完成或被拒绝".into());
        }
        if responded.is_err() {
            continue;
        }
    }
    Err("等待 OAuth 回调超时，可重新授权或使用 auth code 手动交换".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_rejects_missing_duplicate_and_wrong_state() {
        assert_eq!(
            callback_code(
                "/callback?code=a%2Bb&state=expected",
                "/callback",
                "expected"
            )
            .unwrap(),
            Some("a+b".into())
        );
        for request in [
            "/callback?code=a",
            "/callback?code=a&state=wrong",
            "/callback?code=a&state=expected&state=expected",
            "/callback?code=a&code=b&state=expected",
            "/callback?error=denied&state=expected",
        ] {
            assert!(callback_code(request, "/callback", "expected").is_err());
        }
        assert_eq!(
            callback_code("/favicon.ico", "/callback", "expected").unwrap(),
            None
        );
    }
    #[test]
    fn token_updates_preserve_refresh_and_do_not_mix_accounts() {
        let mut saved = json!({"accessToken":"old","refreshToken":"keep","openId":"account"});
        apply_token_response(&mut saved,&json!({"data":{"access_token":"new","open_id":"account","expires_in":"3600","scope":"user_info,item.comment"}}),false).unwrap();
        assert_eq!(saved["refreshToken"], "keep");
        assert_eq!(saved["accessToken"], "new");
        assert_eq!(saved["expiresIn"], 3600);
        let before = saved.clone();
        assert!(
            apply_token_response(
                &mut saved,
                &json!({"data":{"access_token":"other","open_id":"other"}}),
                false
            )
            .is_err()
        );
        assert_eq!(saved, before);
        assert!(apply_token_response(&mut saved, &json!({"data":{}}), true).is_err());
        let status = summary(&saved, false);
        assert!(!status.to_string().contains("refreshToken"));
        assert!(!status.to_string().contains("accessToken"));
    }
    #[test]
    fn fresh_authorization_drops_previous_accounts_optional_token_fields() {
        let mut saved = json!({
            "clientKey": "key",
            "clientSecret": "keep-client-secret",
            "accessToken": "old-access",
            "refreshToken": "old-refresh",
            "openId": "old-account",
            "expiresIn": 3600,
            "expiresAt": 999,
            "scopes": ["user_info", "video.create.bind"],
            "scopesVerified": true,
            "requestedScopes": ["user_info"],
            "pending": {"state": "old-pending"},
        });
        apply_token_response(
            &mut saved,
            &json!({"data": {"access_token": "new-access", "open_id": "new-account"}}),
            true,
        )
        .unwrap();
        assert_eq!(saved["accessToken"], "new-access");
        assert_eq!(saved["openId"], "new-account");
        assert_eq!(saved["clientSecret"], "keep-client-secret");
        assert_eq!(saved["requestedScopes"], json!(["user_info"]));
        for key in [
            "refreshToken",
            "expiresIn",
            "expiresAt",
            "scopes",
            "scopesVerified",
            "pending",
        ] {
            assert!(saved.get(key).is_none(), "{key}");
        }
        assert_eq!(summary(&saved, false)["scopesVerified"], false);
    }

    #[test]
    fn invalid_token_responses_preserve_all_previous_state() {
        let before = json!({
            "accessToken": "old",
            "refreshToken": "keep",
            "openId": "account",
            "scopes": ["user_info"],
            "scopesVerified": true,
            "requestedScopes": ["video.create.bind"],
            "expiresAt": 999,
        });
        for response in [
            json!({"data": {}}),
            json!({"data": {"access_token": "new"}}),
            json!({"data": {"access_token": "new", "open_id": "other", "expires_in": -1}}),
            json!({"data": {"access_token": "new", "open_id": "other", "expires_in": "invalid"}}),
            json!({"data": {"access_token": "new", "open_id": "other", "scope": ["user_info"]}}),
        ] {
            let mut saved = before.clone();
            assert!(apply_token_response(&mut saved, &response, true).is_err());
            assert_eq!(saved, before);
        }
        let mut saved = before.clone();
        assert!(
            apply_token_response(
                &mut saved,
                &json!({"data": {"access_token": "new", "open_id": "other"}}),
                false,
            )
            .is_err()
        );
        assert_eq!(saved, before);
    }

    #[test]
    fn refresh_preserves_verified_scopes_when_omitted_and_records_reduced_grants() {
        let mut saved = json!({
            "accessToken": "old",
            "refreshToken": "keep",
            "openId": "account",
            "scopes": ["user_info", "video.create.bind"],
            "scopesVerified": true,
            "requestedScopes": ["user_info", "video.create.bind"],
        });
        apply_token_response(&mut saved, &json!({"data": {"access_token": "new"}}), false).unwrap();
        assert_eq!(saved["refreshToken"], "keep");
        assert_eq!(saved["scopes"], json!(["user_info", "video.create.bind"]));
        assert_eq!(saved["scopesVerified"], true);

        apply_token_response(
            &mut saved,
            &json!({"data": {"access_token": "newer", "scope": " user_info "}}),
            false,
        )
        .unwrap();
        assert_eq!(saved["scopes"], json!(["user_info"]));
        assert_eq!(
            saved["requestedScopes"],
            json!(["user_info", "video.create.bind"])
        );

        apply_token_response(
            &mut saved,
            &json!({"data": {"access_token": "latest", "scope": ""}}),
            false,
        )
        .unwrap();
        assert_eq!(saved["scopes"], json!([]));
        assert_eq!(saved["scopesVerified"], true);
    }

    #[test]
    fn refresh_does_not_promote_legacy_scopes_without_server_evidence() {
        let mut saved = json!({
            "accessToken": "old",
            "refreshToken": "keep",
            "openId": "account",
            "scopes": ["video.create.bind"],
        });
        apply_token_response(&mut saved, &json!({"data": {"access_token": "new"}}), false).unwrap();
        let status = summary(&saved, false);
        assert_eq!(status["scopesVerified"], false);
        assert!(status["scopes"].is_null());

        apply_token_response(
            &mut saved,
            &json!({"data": {"access_token": "newer", "scope": "user_info,video.create.bind"}}),
            false,
        )
        .unwrap();
        let status = summary(&saved, false);
        assert_eq!(status["scopesVerified"], true);
        assert_eq!(status["scopes"], json!(["user_info", "video.create.bind"]));
    }

    #[test]
    fn loopback_listener_survives_unrelated_and_invalid_requests() {
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        let thread = std::thread::spawn(move || {
            wait_callback(&server, "/callback", "state", Duration::from_secs(5)).unwrap()
        });
        let client = reqwest::blocking::Client::new();
        assert_eq!(
            client
                .get(format!("http://{address}/favicon.ico"))
                .send()
                .unwrap()
                .status(),
            404
        );
        assert_eq!(
            client
                .get(format!("http://{address}/callback?code=x&state=wrong"))
                .send()
                .unwrap()
                .status(),
            400
        );
        assert_eq!(
            client
                .get(format!("http://{address}/callback?code=valid&state=state"))
                .send()
                .unwrap()
                .status(),
            200
        );
        assert_eq!(thread.join().unwrap(), "valid");
    }
}
