//! Streamable HTTP MCP transport and authenticated event forwarding.

use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::post,
};
use clap::Args;
use reqwest::{header::HeaderValue, redirect::Policy};
use ring::rand::SecureRandom;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use url::{Host, Url};

#[derive(Debug, Args)]
pub struct McpArgs {
    /// Streamable HTTP 监听地址，仅允许回环地址；省略时使用 stdio
    #[arg(long, env = "DOUYIN_MCP_LISTEN")]
    listen: Option<SocketAddr>,
    /// 事件转发 Webhook；远程地址必须使用 HTTPS
    #[arg(long, env = "DOUYIN_MCP_WEBHOOK", hide_env_values = true)]
    webhook: Option<String>,
    /// 事件入口 Bearer 令牌，设置后拒绝未认证事件
    #[arg(long, env = "DOUYIN_MCP_EVENT_TOKEN", hide_env_values = true)]
    event_token: Option<String>,
    /// 转发 Webhook 时携带的 Bearer 令牌
    #[arg(long, env = "DOUYIN_MCP_WEBHOOK_TOKEN", hide_env_values = true)]
    webhook_token: Option<String>,
}

#[derive(Clone)]
struct Webhook {
    url: String,
    token: Option<String>,
    client: reqwest::blocking::Client,
}

struct EventConfig {
    webhook: Option<Webhook>,
    event_token: Option<String>,
}

pub fn run(args: McpArgs) -> Result<(), String> {
    let Some(listen) = args.listen else {
        if args.webhook.is_some() || args.event_token.is_some() || args.webhook_token.is_some() {
            return Err("配置事件 Webhook 时必须同时使用 --listen".into());
        }
        return crate::mcp::run_stdio();
    };
    if !listen.ip().is_loopback() {
        return Err("MCP HTTP 仅允许监听回环地址；远程访问请使用带认证的反向代理".into());
    }
    if args.webhook_token.is_some() && args.webhook.is_none() {
        return Err("--webhook-token 需要同时配置 --webhook".into());
    }
    if args.event_token.is_some() && args.webhook.is_none() {
        return Err("--event-token 需要同时配置 --webhook，避免接收后丢弃事件".into());
    }
    let webhook = args
        .webhook
        .map(|url| Webhook::new(url, args.webhook_token))
        .transpose()?;
    let event_token = args
        .event_token
        .map(|token| validate_token(token, "事件令牌"))
        .transpose()?;
    let config = Arc::new(EventConfig {
        webhook,
        event_token,
    });
    let server = crate::mcp::server()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法创建 MCP HTTP 运行时")?
        .block_on(async move {
            let cancel = CancellationToken::new();
            let service: StreamableHttpService<_, LocalSessionManager> = StreamableHttpService::new(
                move || Ok(server.clone()),
                Default::default(),
                StreamableHttpServerConfig::default().with_cancellation_token(cancel.clone()),
            );
            let router = Router::new()
                .nest_service("/mcp", service)
                .route("/events", post(receive_event))
                .with_state(config)
                .layer(DefaultBodyLimit::max(1024 * 1024));
            let listener = tokio::net::TcpListener::bind(listen)
                .await
                .map_err(|_| "MCP HTTP 监听地址绑定失败")?;
            let address = listener.local_addr().map_err(|_| "无法读取 MCP 监听地址")?;
            eprintln!("MCP Streamable HTTP: http://{address}/mcp");
            eprintln!("事件接收入口: http://{address}/events");
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    // Tear down active Streamable HTTP SSE sessions; axum stop alone
                    // leaves those streams running until the client disconnects.
                    cancel.cancel();
                })
                .await
                .map_err(|_| "MCP HTTP 服务异常结束".into())
        })
}

impl Webhook {
    fn new(url: String, token: Option<String>) -> Result<Self, String> {
        validate_webhook_url(&url)?;
        let token = token
            .map(|token| validate_token(token, "Webhook 令牌"))
            .transpose()?;
        let client = reqwest::blocking::Client::builder()
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| "无法创建 Webhook 客户端")?;
        Ok(Self { url, token, client })
    }

    fn forward(&self, event: Value) -> Result<(), String> {
        let mut request = self.client.post(&self.url).json(&event);
        if let Some(token) = &self.token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| "Webhook 令牌格式无效")?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        let response = request.send().map_err(|_| "Webhook 请求失败或超时")?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err("Webhook 未确认接收事件".into())
        }
    }
}

fn validate_token(token: String, name: &str) -> Result<String, String> {
    if token.is_empty()
        || token.len() > 4096
        || token.bytes().any(|byte| byte < 0x21 || byte == 0x7f)
    {
        return Err(format!("{name}为空、过长或包含非法字符"));
    }
    Ok(token)
}

fn validate_webhook_url(raw: &str) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|_| "Webhook URL 无效")?;
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Webhook URL 不能包含用户信息或片段".into());
    }
    let loopback = match url.host() {
        Some(Host::Ipv4(address)) => address.is_loopback(),
        Some(Host::Ipv6(address)) => address.is_loopback(),
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        return Err("Webhook 仅允许 HTTPS 或回环 HTTP 地址".into());
    }
    Ok(())
}

fn authorized(config: &EventConfig, headers: &HeaderMap) -> bool {
    let Some(expected) = &config.event_token else {
        return true;
    };
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token)
        .is_some_and(|provided| token_eq(provided, expected))
}

fn bearer_token(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer")
        && !token.is_empty()
        && !token.contains(char::is_whitespace)
    {
        Some(token)
    } else {
        None
    }
}

fn token_eq(provided: &str, expected: &str) -> bool {
    let left = provided.as_bytes();
    let right = expected.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

fn event_envelope(body: &[u8]) -> Result<Value, String> {
    let input: Value = serde_json::from_slice(body).map_err(|_| "事件必须是合法 JSON")?;
    let object = input.as_object().ok_or("事件必须是 JSON 对象")?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .filter(|kind| {
            !kind.is_empty()
                && kind.len() <= 64
                && kind
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        })
        .ok_or("事件 type 必须是 1..=64 字符的消息类型")?;
    let mut entropy = [0; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut entropy)
        .map_err(|_| "无法生成事件 ID")?;
    let id = entropy
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "系统时间无效")?
        .as_secs();
    Ok(json!({
        "id":id,
        "type":kind,
        "source":"douyin-mcp",
        "timestamp":timestamp,
        "data":object.get("data").cloned().unwrap_or(Value::Null)
    }))
}

async fn receive_event(
    State(config): State<Arc<EventConfig>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorized(&config, &headers) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"accepted":false}))).into_response();
    }
    let event = match event_envelope(&body) {
        Ok(event) => event,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"accepted":false,"error":error})),
            )
                .into_response();
        }
    };
    let Some(webhook) = config.webhook.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"accepted":false,"error":"未配置 Webhook"})),
        )
            .into_response();
    };
    let result = tokio::task::spawn_blocking({
        let event = event.clone();
        move || webhook.forward(event)
    })
    .await;
    match result {
        Ok(Ok(())) => (
            StatusCode::ACCEPTED,
            Json(json!({"accepted":true,"id":event["id"]})),
        )
            .into_response(),
        Ok(Err(_)) | Err(_) => (
            StatusCode::BAD_GATEWAY,
            Json(json!({"accepted":false,"error":"Webhook 转发失败"})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_event_shape_and_transport_destinations() {
        for valid in [
            "https://example.com/events",
            "http://127.0.0.1:8080/events",
            "http://[::1]:8080/events",
            "http://localhost:8080/events",
        ] {
            assert!(validate_webhook_url(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "http://example.com/events",
            "file:///tmp/events",
            "https://user:secret@example.com/events",
            "https://example.com/events#fragment",
        ] {
            assert!(validate_webhook_url(invalid).is_err(), "{invalid}");
        }
        let event = event_envelope(br#"{"type":"message","data":{"text":"fixture"}}"#).unwrap();
        assert_eq!(event["type"], "message");
        assert_eq!(event["data"]["text"], "fixture");
        assert_eq!(event["id"].as_str().unwrap().len(), 32);
        assert!(event_envelope(br#"{"type":"bad type"}"#).is_err());
        assert!(event_envelope(br#"{"data":{}}"#).is_err());
        assert!(event_envelope(b"[]").is_err());
    }
}
