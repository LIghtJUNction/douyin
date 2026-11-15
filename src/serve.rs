//! Local JSON HTTP API for programmatic Douyin CLI calls.

use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Args;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::net::NativeClient;

#[derive(Debug, Args)]
pub struct ServeArgs {
    /// 监听主机；非回环地址必须同时提供 --token
    #[arg(long, env = "DOUYIN_SERVE_HOST", default_value = "127.0.0.1")]
    host: String,
    /// 监听端口
    #[arg(long, env = "DOUYIN_SERVE_PORT", default_value_t = 8789)]
    port: u16,
    /// 可选 Bearer 令牌；设置后所有业务接口都需要 Authorization
    #[arg(long, env = "DOUYIN_SERVE_TOKEN", hide_env_values = true)]
    token: Option<String>,
}

#[derive(Clone)]
struct AppState {
    token: Option<String>,
}

#[derive(Deserialize)]
struct AuthStatusQuery {
    #[serde(default)]
    offline: bool,
}

#[derive(Deserialize)]
struct CountQuery {
    #[serde(default = "default_count")]
    count: u32,
}

fn default_count() -> u32 {
    3
}

#[derive(Deserialize)]
struct SearchBody {
    keyword: String,
    #[serde(default = "default_count")]
    count: u32,
}

#[derive(Deserialize)]
struct DiggBody {
    aweme_id: String,
    #[serde(default = "default_action")]
    action: u8,
    #[serde(default)]
    channel_id: i32,
    #[serde(default)]
    yes: bool,
}

#[derive(Deserialize)]
struct CollectBody {
    aweme_id: String,
    #[serde(default = "default_action")]
    action: u8,
    #[serde(default)]
    yes: bool,
}

#[derive(Deserialize)]
struct FollowBody {
    user_id: Option<String>,
    sec_user_id: Option<String>,
    #[serde(default = "default_action")]
    action: u8,
    #[serde(default = "default_from")]
    from: i32,
    #[serde(default = "default_from_pre")]
    from_pre: i32,
    #[serde(default)]
    from_action: i32,
    #[serde(default = "default_channel")]
    channel_id: i32,
    #[serde(default)]
    yes: bool,
}

#[derive(Deserialize)]
struct CommentBody {
    aweme_id: String,
    text: String,
    reply_id: Option<String>,
    #[serde(default)]
    channel_id: i32,
    #[serde(default)]
    yes: bool,
}

fn default_action() -> u8 {
    1
}
fn default_from() -> i32 {
    191
}
fn default_from_pre() -> i32 {
    27
}
fn default_channel() -> i32 {
    0x0004_9446
}

pub fn run(args: ServeArgs) -> Result<(), String> {
    if args.port == 0 {
        return Err("端口不能为 0".into());
    }
    let host: std::net::IpAddr = args
        .host
        .parse()
        .map_err(|_| "主机地址无效，请使用 IPv4/IPv6 地址")?;
    let listen = SocketAddr::new(host, args.port);
    let token = args
        .token
        .map(|token| validate_token(token))
        .transpose()?;
    if !listen.ip().is_loopback() && token.is_none() {
        return Err("非回环监听必须设置 --token，避免未认证暴露账号能力".into());
    }
    let state = AppState { token };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法创建 HTTP 服务运行时")?
        .block_on(async move {
            let cancel = CancellationToken::new();
            let router = Router::new()
                .route("/health", get(health))
                .route("/v1/me", get(me))
                .route("/v1/auth/status", get(auth_status))
                .route("/v1/search", post(search))
                .route("/v1/aweme/{aweme_id}", get(aweme))
                .route("/v1/comments/{aweme_id}", get(comments))
                .route("/v1/users/{sec_user_id}/posts", get(user_posts))
                .route("/v1/social/digg", post(digg))
                .route("/v1/social/collect", post(collect))
                .route("/v1/social/follow", post(follow))
                .route("/v1/social/comment", post(comment))
                .route("/v1/live/permissions", get(live_permissions))
                .route("/v1/live/create-info", get(live_create_info))
                .route("/v1/live/info/{room_id}", get(live_info))
                .with_state(Arc::new(state));
            let listener = tokio::net::TcpListener::bind(listen)
                .await
                .map_err(|_| "HTTP 监听地址绑定失败")?;
            let address = listener.local_addr().map_err(|_| "无法读取 HTTP 监听地址")?;
            eprintln!("Douyin HTTP API: http://{address}/");
            eprintln!("健康检查: http://{address}/health");
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = tokio::signal::ctrl_c().await;
                    cancel.cancel();
                })
                .await
                .map_err(|_| "HTTP 服务异常结束".into())
        })
}

async fn health() -> Json<Value> {
    Json(json!({"ok": true}))
}

async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(|| NativeClient::saved()?.me())).await
}

async fn auth_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AuthStatusQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(move || crate::auth::status(query.offline))).await
}

async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<SearchBody>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(move || {
        NativeClient::saved()?.search(&body.keyword, body.count)
    }))
    .await
}

async fn aweme(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(aweme_id): Path<String>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(move || NativeClient::saved()?.aweme(&aweme_id))).await
}

async fn comments(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(aweme_id): Path<String>,
    Query(query): Query<CountQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(move || {
        NativeClient::saved()?.comments(&aweme_id, query.count)
    }))
    .await
}

async fn user_posts(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(sec_user_id): Path<String>,
    Query(query): Query<CountQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(move || {
        NativeClient::saved()?.user_posts(&sec_user_id, query.count)
    }))
    .await
}

async fn digg(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<DiggBody>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    if !body.yes {
        return error_response(StatusCode::BAD_REQUEST, "写操作需要 yes=true");
    }
    json_result(blocking(move || {
        let client = NativeClient::saved()?;
        crate::social::digg(&client, &body.aweme_id, body.action, body.channel_id)
    }))
    .await
}

async fn collect(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CollectBody>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    if !body.yes {
        return error_response(StatusCode::BAD_REQUEST, "写操作需要 yes=true");
    }
    json_result(blocking(move || {
        let client = NativeClient::saved()?;
        crate::social::collect(&client, &body.aweme_id, body.action)
    }))
    .await
}

async fn follow(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<FollowBody>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    if !body.yes {
        return error_response(StatusCode::BAD_REQUEST, "写操作需要 yes=true");
    }
    json_result(blocking(move || {
        let client = NativeClient::saved()?;
        crate::social::follow(
            &client,
            body.user_id.as_deref(),
            body.sec_user_id.as_deref(),
            body.action,
            body.from,
            body.from_pre,
            body.from_action,
            body.channel_id,
        )
    }))
    .await
}

async fn comment(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(body): Json<CommentBody>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    if !body.yes {
        return error_response(StatusCode::BAD_REQUEST, "写操作需要 yes=true");
    }
    json_result(blocking(move || {
        let client = NativeClient::saved()?;
        crate::social::publish_comment(
            &client,
            &body.aweme_id,
            &body.text,
            body.reply_id.as_deref(),
            body.channel_id,
        )
    }))
    .await
}

async fn live_permissions(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(|| {
        crate::live::permission_summary(&NativeClient::saved()?.live_permissions()?)
    }))
    .await
}

async fn live_create_info(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(|| {
        let response = NativeClient::saved()?.live_create_info(2)?;
        let status = response
            .get("status_code")
            .and_then(Value::as_i64)
            .ok_or("开播预览响应缺少 status_code")?;
        Ok(json!({
            "status_code": status,
            "ok": status == 0,
            "has_data": response.get("data").is_some(),
        }))
    }))
    .await
}

async fn live_info(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(room_id): Path<String>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return response;
    }
    json_result(blocking(move || {
        let raw = NativeClient::saved()?.live_room_info(&room_id)?;
        crate::live::room_summary(&raw, &room_id)
    }))
    .await
}

fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let Some(expected) = &state.token else {
        return Ok(());
    };
    let provided = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(bearer_token);
    if provided.is_some_and(|token| token_eq(token, expected)) {
        Ok(())
    } else {
        Err(error_response(StatusCode::UNAUTHORIZED, "需要有效的 Bearer 令牌"))
    }
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

fn validate_token(token: String) -> Result<String, String> {
    if token.is_empty()
        || token.len() > 4096
        || token.bytes().any(|byte| byte < 0x21 || byte == 0x7f)
    {
        return Err("服务令牌为空、过长或包含非法字符".into());
    }
    Ok(token)
}

async fn blocking<F>(work: F) -> Result<Value, String>
where
    F: FnOnce() -> Result<Value, String> + Send + 'static,
{
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| "后台任务失败".to_owned())?
}

async fn json_result(result: impl std::future::Future<Output = Result<Value, String>>) -> Response {
    match result.await {
        Ok(value) => (StatusCode::OK, Json(value)).into_response(),
        Err(error) => error_response(StatusCode::BAD_GATEWAY, &error),
    }
}

fn error_response(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_loopback_requires_token_rule() {
        assert!(validate_token("ok-token".into()).is_ok());
        assert!(validate_token("".into()).is_err());
        assert!(validate_token("has space".into()).is_err());
        assert!(token_eq("abc", "abc"));
        assert!(!token_eq("abc", "abd"));
        assert!(!token_eq("abc", "abcd"));
    }
}
