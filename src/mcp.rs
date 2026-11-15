//! MCP transport and protocol negotiation are provided by the official SDK.
use reqwest::Method;
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
        ToolAnnotations,
    },
    service::RequestContext,
};
use serde_json::{Map, Value, json};

use crate::{
    api, insights, oauth,
    openapi::{OpenApiClient, Request},
};

#[derive(Clone)]
pub(crate) struct DouyinServer {
    tools: Vec<Tool>,
}

impl ServerHandler for DouyinServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("douyin",env!("CARGO_PKG_VERSION")))
            .with_instructions("离线分析不联网。官方 OpenAPI 使用独立 OAuth；ADB 导入用于 native_me 和 native_live_permissions。写操作必须显式 confirmed=true。")
    }
    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools.clone()))
    }
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if self.get_tool(&request.name).is_none() {
            return Err(ErrorData::invalid_params("未知工具名称", None));
        }
        let name = request.name.into_owned();
        let args = request.arguments.unwrap_or_default();
        // reqwest::blocking and deterministic analyses must not block MCP's
        // async control loop, cancellation handling, or protocol responses.
        let result = tokio::task::spawn_blocking(move || execute(&name, &args))
            .await
            .map_err(|_| ErrorData::internal_error("工具执行线程失败", None))?;
        let result = match result {
            Ok(value) => {
                let text = serde_json::to_string(&value)
                    .map_err(|_| ErrorData::internal_error("工具结果序列化失败", None))?;
                let mut result = CallToolResult::success(vec![ContentBlock::text(text)]);
                result.structured_content = Some(value);
                result
            }
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
        };
        Ok(result.into())
    }
}

pub(crate) fn server() -> Result<DouyinServer, String> {
    Ok(DouyinServer { tools: tools()? })
}

pub fn run_stdio() -> Result<(), String> {
    let server = server()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法创建 MCP 运行时")?
        .block_on(async move {
            let service = server
                .serve(rmcp::transport::stdio())
                .await
                .map_err(|_| "MCP 初始化失败")?;
            service.waiting().await.map_err(|_| "MCP 会话异常结束")?;
            Ok(())
        })
}

fn tools() -> Result<Vec<Tool>, String> {
    let mut rows = Vec::new();
    let insight_schema = json!({"type":"object","properties":{"texts":{"type":"array","items":{"type":"string"}},"top":{"type":"integer","minimum":0,"default":20},"min_count":{"type":"integer","minimum":1,"default":2}},"required":["texts"]});
    for (name, description) in [
        ("hot_words", "离线统计热词及带权频次"),
        ("hot_memes", "离线提取重复短句和热梗候选"),
        ("demand_discovery", "离线提取带有需求关键词的原句"),
    ] {
        rows.push(tool(
            name,
            description,
            insight_schema.clone(),
            true,
            false,
        )?);
    }
    rows.push(tool(
        "auth_status",
        "只检查已保存的官方 OAuth 配置，不联网",
        schema(json!({}), &[]),
        true,
        false,
    )?);
    rows.push(tool(
        "native_search",
        "使用安卓原生会话搜索作品，单页最多 20 条",
        schema(json!({"keyword":{"type":"string","minLength":1},"count":{"type":"integer","minimum":1,"maximum":20,"default":3}}), &["keyword"]),
        true,
        true,
    )?);
    rows.push(tool(
        "native_aweme",
        "使用安卓原生会话读取单作品详情",
        schema(
            json!({"aweme_id":{"type":"string","minLength":1}}),
            &["aweme_id"],
        ),
        true,
        true,
    )?);
    rows.push(tool(
        "native_comments",
        "使用安卓原生会话读取作品评论首页",
        schema(json!({"aweme_id":{"type":"string","minLength":1},"count":{"type":"integer","minimum":1,"maximum":20,"default":3}}), &["aweme_id"]),
        true,
        true,
    )?);
    rows.push(tool(
        "native_comment_replies",
        "使用安卓原生会话读取评论回复首页",
        schema(json!({"aweme_id":{"type":"string","minLength":1},"comment_id":{"type":"string","minLength":1},"count":{"type":"integer","minimum":1,"maximum":20,"default":3}}), &["aweme_id","comment_id"]),
        true,
        true,
    )?);
    rows.push(tool(
        "native_user_posts",
        "使用安卓原生会话读取用户作品首页",
        schema(json!({"sec_user_id":{"type":"string","minLength":1},"count":{"type":"integer","minimum":1,"maximum":20,"default":3}}), &["sec_user_id"]),
        true,
        true,
    )?);
    rows.push(tool(
        "native_live_permissions",
        "使用导入的安卓 Token 独立查询当前账号直播配置，不创建直播间",
        schema(json!({}), &[]),
        true,
        true,
    )?);
    rows.push(tool(
        "native_live_create_info",
        "查询安卓开播预览配置；platform 固定为 APK 证实值 2，不创建直播间",
        schema(json!({}), &[]),
        true,
        true,
    )?);
    rows.push(tool(
        "native_digg",
        "安卓原生点赞或取消点赞；需要 confirmed=true",
        schema(
            json!({
                "aweme_id":{"type":"string","minLength":1},
                "action":{"type":"integer","minimum":0,"maximum":1,"default":1},
                "channel_id":{"type":"integer","default":0},
                "confirmed":{"type":"boolean","default":false}
            }),
            &["aweme_id"],
        ),
        false,
        true,
    )?);
    rows.push(tool(
        "native_collect",
        "安卓原生收藏或取消收藏；需要 confirmed=true",
        schema(
            json!({
                "aweme_id":{"type":"string","minLength":1},
                "action":{"type":"integer","minimum":0,"maximum":1,"default":1},
                "confirmed":{"type":"boolean","default":false}
            }),
            &["aweme_id"],
        ),
        false,
        true,
    )?);
    rows.push(tool(
        "native_me",
        "使用 ADB 导入的登录态读取当前账号资料",
        schema(json!({}), &[]),
        true,
        true,
    )?);
    rows.push(tool(
        "userinfo",
        "获取官方授权账号资料",
        authorized_schema(json!({}), &[]),
        true,
        true,
    )?);
    let page = json!({"item_id":{"type":"string","minLength":1},"cursor":{"type":"integer","minimum":0,"default":0},"count":{"type":"integer","minimum":1,"maximum":20,"default":20},"sort_type":{"type":"integer","minimum":0,"maximum":2}});
    rows.push(tool(
        "comment_list",
        "获取官方视频评论列表",
        authorized_schema(page.clone(), &["item_id"]),
        true,
        true,
    )?);
    let mut reply = page;
    reply["comment_id"] = json!({"type":"string","minLength":1});
    rows.push(tool(
        "comment_replies",
        "获取官方评论回复列表",
        authorized_schema(reply, &["item_id", "comment_id"]),
        true,
        true,
    )?);
    rows.push(tool("comment_reply","通过官方 OpenAPI 发送评论或回复；需要 confirmed=true",authorized_schema(json!({
        "item_id":{"type":"string","minLength":1},"comment_id":{"type":"string"},"content":{"type":"string","minLength":1,"maxLength":100},"confirmed":{"type":"boolean","default":false}
    }),&["item_id","content"]),false,true)?);
    rows.push(tool("im_message_send","通过官方 OpenAPI 发送文字、图片或视频私信；需要 confirmed=true",authorized_schema(json!({
        "to_user_id":{"type":"string","minLength":1},"scene":{"type":"string","enum":["im_reply_msg","im_enter_direct_msg"],"default":"im_reply_msg"},
        "msg_id":{"type":"string","minLength":1},"conversation_id":{"type":"string","minLength":1},"message_type":{"type":"string","enum":["text","image","video"],"default":"text"},
        "text":{"type":"string"},"media_id":{"type":"string"},"item_id":{"type":"string"},"confirmed":{"type":"boolean","default":false}
    }),&["to_user_id","msg_id","conversation_id"]),false,true)?);
    rows.push(tool("openapi_request","同源通用官方 OpenAPI；写操作需要 confirmed=true",schema(json!({
        "method":{"type":"string","enum":["GET","POST","PUT","PATCH","DELETE","HEAD","OPTIONS"]},"path":{"type":"string"},"token":{"type":"string"},
        "params":{"type":"object","additionalProperties":{"type":"string"}},"form":{"type":"object","additionalProperties":{"type":"string"}},"json":{"type":"object"},"confirmed":{"type":"boolean","default":false}
    }),&["method","path"]),false,true)?);
    Ok(rows)
}
fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn authorized_schema(mut properties: Value, required: &[&str]) -> Value {
    properties["token"] = json!({"type":"string"});
    properties["open_id"] = json!({"type":"string"});
    schema(properties, required)
}
fn tool(
    name: &'static str,
    description: &'static str,
    schema: Value,
    read: bool,
    network: bool,
) -> Result<Tool, String> {
    let schema = schema.as_object().cloned().ok_or("MCP 工具 schema 无效")?;
    Ok(Tool::new(name, description, schema).with_annotations(
        ToolAnnotations::new()
            .read_only(read)
            .destructive(!read)
            .idempotent(read)
            .open_world(network),
    ))
}

fn execute(name: &str, args: &Map<String, Value>) -> Result<Value, String> {
    if matches!(name, "hot_words" | "hot_memes" | "demand_discovery") {
        let texts = args
            .get("texts")
            .and_then(Value::as_array)
            .ok_or("texts 必须是字符串数组")?;
        let records = texts
            .iter()
            .map(|v| {
                v.as_str()
                    .map(|text| insights::TextRecord::new(text, None))
                    .ok_or("texts 包含非字符串记录".to_owned())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let top = number(args, "top", 20, 0, 10_000)? as usize;
        let min = number(args, "min_count", 2, 1, u64::MAX)?;
        let result = insights::analyze(&records, top, min);
        let key = match name {
            "hot_words" => "hot_words",
            "hot_memes" => "hot_memes",
            _ => "demands",
        };
        return Ok(json!({"input_count":result["input_count"],"items":result[key]}));
    }
    if name == "auth_status" {
        let saved = oauth::load()?;
        return Ok(
            json!({"authorized":oauth::field(&saved,"accessToken").is_some() && oauth::field(&saved,"openId").is_some(),"client_key_saved":oauth::field(&saved,"clientKey").is_some(),"open_id":saved["openId"],"scopes":saved["scopes"],"expires_in":saved["expiresIn"]}),
        );
    }
    if matches!(
        name,
        "native_search"
            | "native_aweme"
            | "native_comments"
            | "native_comment_replies"
            | "native_user_posts"
    ) {
        let client = crate::net::NativeClient::saved()?;
        let count = number(args, "count", 3, 1, 20)? as u32;
        return match name {
            "native_search" => client.search(required(args, "keyword")?, count),
            "native_aweme" => client.aweme(required(args, "aweme_id")?),
            "native_comments" => client.comments(required(args, "aweme_id")?, count),
            "native_comment_replies" => client.comment_replies(
                required(args, "aweme_id")?,
                required(args, "comment_id")?,
                count,
            ),
            "native_user_posts" => client.user_posts(required(args, "sec_user_id")?, count),
            _ => unreachable!("native tool names were matched above"),
        };
    }
    if name == "native_me" {
        return crate::net::NativeClient::saved()?.me();
    }
    if name == "native_live_permissions" {
        return crate::live::permission_summary(
            &crate::net::NativeClient::saved()?.live_permissions()?,
        );
    }
    if name == "native_live_create_info" {
        let response = crate::net::NativeClient::saved()?.live_create_info(2)?;
        let status = response
            .get("status_code")
            .and_then(Value::as_i64)
            .ok_or("开播预览响应缺少 status_code")?;
        return Ok(json!({
            "status_code": status,
            "ok": status == 0,
            "has_data": response.get("data").is_some(),
        }));
    }
    if name == "native_digg" {
        if args.get("confirmed") != Some(&Value::Bool(true)) {
            return Err("写操作需要 confirmed=true".into());
        }
        let client = crate::net::NativeClient::saved()?;
        let action = number(args, "action", 1, 0, 1)? as u8;
        let channel_id = number(args, "channel_id", 0, 0, 1_000_000)? as i32;
        return crate::social::digg(&client, required(args, "aweme_id")?, action, channel_id);
    }
    if name == "native_collect" {
        if args.get("confirmed") != Some(&Value::Bool(true)) {
            return Err("写操作需要 confirmed=true".into());
        }
        let client = crate::net::NativeClient::saved()?;
        let action = number(args, "action", 1, 0, 1)? as u8;
        return crate::social::collect(&client, required(args, "aweme_id")?, action);
    }
    if name == "openapi_request" {
        let method = Method::from_bytes(required(args, "method")?.as_bytes())
            .map_err(|_| "HTTP method 无效")?;
        if !matches!(
            method,
            Method::GET
                | Method::POST
                | Method::PUT
                | Method::PATCH
                | Method::DELETE
                | Method::HEAD
                | Method::OPTIONS
        ) {
            return Err("HTTP method 不受支持".into());
        }
        if !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
            confirmed(args)?;
        }
        let saved = if args.contains_key("token") {
            json!({})
        } else {
            oauth::load()?
        };
        let token = optional(args, "token")?
            .or_else(|| oauth::field(&saved, "accessToken").map(str::to_owned))
            .ok_or("缺少官方 access_token")?;
        let mut request = Request::new(method, required(args, "path")?);
        request.token = Some(&token);
        request.params = string_pairs(args.get("params"))?;
        request.form = string_pairs(args.get("form"))?;
        request.json = args.get("json").cloned();
        return OpenApiClient::new()?.request(request);
    }
    if matches!(name, "comment_reply" | "im_message_send") {
        confirmed(args)?;
    }
    let (token, open_id) =
        api::resolve_credentials(optional(args, "token")?, optional(args, "open_id")?)?;
    let path = match name {
        "userinfo" => "/oauth/userinfo/",
        "comment_list" => "/item/comment/list/",
        "comment_replies" => "/item/comment/reply/list/",
        "comment_reply" => "/item/comment/reply/",
        "im_message_send" => "/im/send/msg/",
        _ => return Err("未知 MCP 工具".into()),
    };
    let write = matches!(name, "comment_reply" | "im_message_send");
    let mut request = Request::new(if write { Method::POST } else { Method::GET }, path);
    request.token = Some(&token);
    request.params.push(("open_id".into(), open_id));
    match name {
        "comment_list" | "comment_replies" => {
            request.params.extend([
                ("item_id".into(), required(args, "item_id")?.into()),
                (
                    "cursor".into(),
                    number(args, "cursor", 0, 0, u64::MAX)?.to_string(),
                ),
                (
                    "count".into(),
                    number(args, "count", 20, 1, 20)?.to_string(),
                ),
            ]);
            if name == "comment_replies" {
                request
                    .params
                    .push(("comment_id".into(), required(args, "comment_id")?.into()));
            }
            if args.contains_key("sort_type") {
                request.params.push((
                    "sort_type".into(),
                    number(args, "sort_type", 0, 0, 2)?.to_string(),
                ));
            }
        }
        "comment_reply" => {
            let content = required(args, "content")?;
            api::validate_text(content, 100, false)?;
            let mut body = json!({"item_id":required(args,"item_id")?,"content":content});
            if let Some(id) = optional(args, "comment_id")? {
                body["comment_id"] = json!(id);
            }
            request.json = Some(body);
        }
        "im_message_send" => {
            let kind = match optional(args, "message_type")?.as_deref().unwrap_or("text") {
                "text" => api::MessageType::Text,
                "image" => api::MessageType::Image,
                "video" => api::MessageType::Video,
                _ => return Err("消息类型无效".into()),
            };
            let scene = optional(args, "scene")?.unwrap_or_else(|| "im_reply_msg".into());
            if !matches!(scene.as_str(), "im_reply_msg" | "im_enter_direct_msg") {
                return Err("私信场景无效".into());
            }
            let content = api::message_content(
                kind,
                optional(args, "text")?,
                optional(args, "media_id")?,
                optional(args, "item_id")?,
            )?;
            request.json = Some(
                json!({"to_user_id":required(args,"to_user_id")?,"scene":scene,"msg_id":required(args,"msg_id")?,"conversation_id":required(args,"conversation_id")?,"content":content}),
            );
        }
        _ => (),
    }
    OpenApiClient::new()?.request(request)
}

fn optional(args: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.clone())),
        _ => Err(format!("{key} 必须是非空字符串")),
    }
}
fn required<'a>(args: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("缺少有效 {key}"))
}
fn number(
    args: &Map<String, Value>,
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    let value = match args.get(key) {
        None => default,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("{key} 必须是非负整数"))?,
    };
    if !(min..=max).contains(&value) {
        return Err(format!("{key} 超出允许范围"));
    }
    Ok(value)
}
fn confirmed(args: &Map<String, Value>) -> Result<(), String> {
    if args.get("confirmed") == Some(&Value::Bool(true)) {
        Ok(())
    } else {
        Err("写操作需要显式 confirmed=true；未发送请求".into())
    }
}
fn string_pairs(value: Option<&Value>) -> Result<Vec<(String, String)>, String> {
    match value {
        None => Ok(vec![]),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|s| (k.clone(), s.into()))
                    .ok_or("请求参数值必须是字符串".into())
            })
            .collect(),
        _ => Err("请求参数必须是对象".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_tools_and_write_guards_are_real_not_only_schema_hints() {
        let args = json!({"texts":["求教程","求教程"],"min_count":2});
        let result = execute("demand_discovery", args.as_object().unwrap()).unwrap();
        assert_eq!(result["items"][0]["count"], 2);
        for name in ["comment_reply", "im_message_send"] {
            assert!(
                execute(name, &Map::new())
                    .unwrap_err()
                    .contains("confirmed=true")
            );
        }
        let args = json!({"method":"POST","path":"/__douyin_cli_no_network_test__/"});
        assert!(
            execute("openapi_request", args.as_object().unwrap())
                .unwrap_err()
                .contains("confirmed=true")
        );
        for name in [
            "native_me",
            "native_search",
            "native_aweme",
            "native_comments",
            "native_comment_replies",
            "native_user_posts",
        ] {
            assert!(tools().unwrap().iter().any(|tool| tool.name == name));
        }
        assert!(
            tools()
                .unwrap()
                .iter()
                .any(|t| t.name == "native_live_permissions")
        );
    }
}
