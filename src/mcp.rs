use std::collections::HashMap;
use std::io::{self, BufRead, Write};

use serde_json::{Map, Value, json};

use crate::insights::{self, TextRecord};
use crate::openapi::{OpenApiClient, RequestSpec, im_message_body};
use crate::settings;

const PROTOCOL_VERSION: &str = "2025-11-25";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run_stdio() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(error) => {
                write_message(
                    &mut stdout,
                    &error_response(Value::Null, -32700, &format!("Parse error: {error}")),
                )?;
                continue;
            }
        };
        if let Some(response) = handle_message(&request) {
            write_message(&mut stdout, &response)?;
        }
    }
    Ok(())
}

pub fn handle_message(request: &Value) -> Option<Value> {
    if let Some(messages) = request.as_array() {
        let responses: Vec<_> = messages.iter().filter_map(handle_message).collect();
        return (!responses.is_empty()).then_some(Value::Array(responses));
    }
    let id = request.get("id").cloned()?;
    let method = request.get("method").and_then(Value::as_str);
    let result = match method {
        Some("initialize") => Ok(initialize(request)),
        Some("ping") => Ok(json!({})),
        Some("tools/list") => Ok(json!({"tools": tools()})),
        Some("tools/call") => call_tool(request),
        Some(method) => {
            return Some(error_response(
                id,
                -32601,
                &format!("Method not found: {method}"),
            ));
        }
        None => {
            return Some(error_response(
                id,
                -32600,
                "Invalid Request: missing method",
            ));
        }
    };
    Some(match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(ToolError::Unknown(name)) => {
            error_response(id, -32602, &format!("Unknown tool: {name}"))
        }
        Err(ToolError::Execution(message)) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": message}],
                "isError": true
            }
        }),
    })
}

fn initialize(request: &Value) -> Value {
    let requested = request
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(PROTOCOL_VERSION);
    let version = match requested {
        "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25" => requested,
        _ => PROTOCOL_VERSION,
    };
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "douyin", "version": SERVER_VERSION},
        "instructions": "抖音开放平台 OpenAPI MCP 服务器。默认读取 douyin auth 保存的 access_token/open_id，也可以在工具参数中显式传入。"
    })
}

fn tools() -> Vec<Value> {
    vec![
        tool("hot_words", "离线发现输入文本中的热词。基于可解释的频次启发式，不表示理解真实语义。", insights_schema(), true),
        tool("hot_memes", "离线发现重复短句、口头禅、emoji 与固定表达。", insights_schema(), true),
        tool("demand_discovery", "离线提取包含购买、求助、功能或问题意图信号的原句。", insights_schema(), true),
        tool("auth_status", "查看本机是否已保存抖音开放平台授权信息。", json!({"type":"object","properties":{}}), true),
        tool("userinfo", "获取官方授权用户信息。", auth_schema(json!({})), true),
        tool("comment_list", "获取官方接口中的视频评论列表。", auth_schema(json!({
            "item_id":{"type":"string"}, "cursor":{"type":"integer","default":0}, "count":{"type":"integer","default":20}
        })).with_required(&["item_id"]), true),
        tool("comment_replies", "获取官方接口中的评论回复列表。", auth_schema(json!({
            "item_id":{"type":"string"}, "comment_id":{"type":"string"}, "cursor":{"type":"integer","default":0}, "count":{"type":"integer","default":20}
        })).with_required(&["item_id", "comment_id"]), true),
        tool("comment_reply", "通过官方 OpenAPI 回复视频或评论。", auth_schema(json!({
            "item_id":{"type":"string"}, "content":{"type":"string"}, "comment_id":{"type":"string"}
        })).with_required(&["item_id", "content"]), false),
        tool("im_message_send", "通过企业号 OpenAPI 发送私信消息。", auth_schema(json!({
            "to_user_id":{"type":"string"}, "message_type":{"type":"string","enum":["text","image","video","card"],"default":"text"},
            "text":{"type":"string"}, "media_id":{"type":"string"}, "item_id":{"type":"string"}, "card_id":{"type":"string"},
            "persona_id":{"type":"string"}, "client_msg_id":{"type":"string"}
        })).with_required(&["to_user_id"]), false),
        tool("openapi_request", "调用任意官方 OpenAPI 路径。", json!({
            "type":"object",
            "properties":{
                "method":{"type":"string"}, "path":{"type":"string"}, "token":{"type":"string"},
                "params":{"type":"object","additionalProperties":{"type":"string"}},
                "json_body":{"type":["object","array"]}, "form":{"type":"object","additionalProperties":{"type":"string"}},
                "headers":{"type":"object","additionalProperties":{"type":"string"}}
            },
            "required":["method","path"]
        }), false),
    ]
}

fn insights_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "texts":{"type":"array","items":{"type":"string"}},
            "top":{"type":"integer","default":20,"minimum":0},
            "min_count":{"type":"integer","default":2,"minimum":1}
        },
        "required":["texts"]
    })
}

fn tool(name: &str, description: &str, schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only,
            "openWorldHint": true
        }
    })
}

trait SchemaExt {
    fn with_required(self, required: &[&str]) -> Value;
}

impl SchemaExt for Value {
    fn with_required(mut self, required: &[&str]) -> Value {
        self["required"] = json!(required);
        self
    }
}

fn auth_schema(extra: Value) -> Value {
    let mut properties = extra.as_object().cloned().unwrap_or_default();
    properties.insert("token".to_owned(), json!({"type":"string"}));
    properties.insert("open_id".to_owned(), json!({"type":"string"}));
    json!({"type":"object", "properties": properties})
}

fn call_tool(request: &Value) -> Result<Value, ToolError> {
    let name = request
        .pointer("/params/name")
        .and_then(Value::as_str)
        .ok_or_else(|| ToolError::Execution("缺少工具名称".to_owned()))?;
    let args = request
        .pointer("/params/arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let result = execute_tool(name, &args)?;
    let text = serde_json::to_string(&result).map_err(execution)?;
    Ok(json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": result,
        "isError": false
    }))
}

fn execute_tool(name: &str, args: &Map<String, Value>) -> Result<Value, ToolError> {
    if matches!(name, "hot_words" | "hot_memes" | "demand_discovery") {
        return execute_insights_tool(name, args);
    }
    if name == "auth_status" {
        let saved = saved_openapi()?;
        return Ok(json!({
            "authorized": saved_string(&saved, "accessToken").is_some() && saved_string(&saved, "openId").is_some(),
            "client_key_saved": saved_string(&saved, "clientKey").is_some(),
            "open_id": saved_string(&saved, "openId"),
            "scopes": saved.get("scopes").cloned().unwrap_or_else(|| json!([])),
            "expires_in": saved.get("expiresIn").cloned().unwrap_or_else(|| json!(0))
        }));
    }
    let client = OpenApiClient::new().map_err(ToolError::Execution)?;
    match name {
        "userinfo" => {
            let (token, open_id) = resolve_auth(args)?;
            request(
                &client,
                "GET",
                "/oauth/userinfo/",
                &token,
                Some(HashMap::from([("open_id".to_owned(), open_id)])),
                None,
            )
        }
        "comment_list" => {
            let (token, open_id) = resolve_auth(args)?;
            request(
                &client,
                "GET",
                "/item/comment/list/",
                &token,
                Some(HashMap::from([
                    ("open_id".to_owned(), open_id),
                    ("item_id".to_owned(), required_string(args, "item_id")?),
                    ("cursor".to_owned(), integer(args, "cursor", 0).to_string()),
                    ("count".to_owned(), integer(args, "count", 20).to_string()),
                ])),
                None,
            )
        }
        "comment_replies" => {
            let (token, open_id) = resolve_auth(args)?;
            request(
                &client,
                "GET",
                "/item/comment/reply/list/",
                &token,
                Some(HashMap::from([
                    ("open_id".to_owned(), open_id),
                    ("item_id".to_owned(), required_string(args, "item_id")?),
                    (
                        "comment_id".to_owned(),
                        required_string(args, "comment_id")?,
                    ),
                    ("cursor".to_owned(), integer(args, "cursor", 0).to_string()),
                    ("count".to_owned(), integer(args, "count", 20).to_string()),
                ])),
                None,
            )
        }
        "comment_reply" => {
            let (token, open_id) = resolve_auth(args)?;
            let mut body = Map::from_iter([
                (
                    "item_id".to_owned(),
                    json!(required_string(args, "item_id")?),
                ),
                (
                    "content".to_owned(),
                    json!(required_string(args, "content")?),
                ),
            ]);
            if let Some(value) = optional_string(args, "comment_id") {
                body.insert("comment_id".to_owned(), json!(value));
            }
            request(
                &client,
                "POST",
                "/item/comment/reply/",
                &token,
                Some(HashMap::from([("open_id".to_owned(), open_id)])),
                Some(Value::Object(body)),
            )
        }
        "im_message_send" => {
            let (token, open_id) = resolve_auth(args)?;
            let message_type =
                optional_string(args, "message_type").unwrap_or_else(|| "text".to_owned());
            let (key, source, error) = match message_type.as_str() {
                "text" => ("text", "text", "message_type=text 需要 text"),
                "image" => ("media_id", "media_id", "message_type=image 需要 media_id"),
                "video" => ("item_id", "item_id", "message_type=video 需要 item_id"),
                "card" => ("card_id", "card_id", "message_type=card 需要 card_id"),
                value => return Err(ToolError::Execution(format!("不支持的私信类型: {value}"))),
            };
            let value = optional_string(args, source)
                .ok_or_else(|| ToolError::Execution(error.to_owned()))?;
            let body = im_message_body(
                &required_string(args, "to_user_id")?,
                &message_type,
                json!({key: value}),
                optional_string(args, "persona_id").as_deref(),
                optional_string(args, "client_msg_id").as_deref(),
            );
            request(
                &client,
                "POST",
                "/enterprise/im/message/send/",
                &token,
                Some(HashMap::from([("open_id".to_owned(), open_id)])),
                Some(body),
            )
        }
        "openapi_request" => {
            let saved = saved_openapi()?;
            let token = optional_string(args, "token")
                .or_else(|| saved_string(&saved, "accessToken"))
                .ok_or_else(|| {
                    ToolError::Execution(
                        "调用 OpenAPI 需要 access-token 或 client-token".to_owned(),
                    )
                })?;
            client
                .request(RequestSpec {
                    method: &required_string(args, "method")?,
                    path: &required_string(args, "path")?,
                    token: Some(&token),
                    params: string_map(args, "params")?,
                    json_body: args.get("json_body").cloned(),
                    form: string_map(args, "form")?,
                    headers: string_map(args, "headers")?,
                    auth_required: true,
                })
                .map_err(ToolError::Execution)
        }
        value => Err(ToolError::Unknown(value.to_owned())),
    }
}

fn execute_insights_tool(name: &str, args: &Map<String, Value>) -> Result<Value, ToolError> {
    let texts = args
        .get("texts")
        .and_then(Value::as_array)
        .ok_or_else(|| ToolError::Execution("texts 必须是字符串数组".to_owned()))?;
    let records = texts
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(|text| TextRecord::new(text, None))
                .ok_or_else(|| ToolError::Execution("texts 必须是字符串数组".to_owned()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let top = non_negative_integer(args, "top", 20)?;
    let min_count = positive_integer(args, "min_count", 2)?;
    let result = insights::analyze(&records, top, min_count);
    let key = match name {
        "hot_words" => "hot_words",
        "hot_memes" => "hot_memes",
        "demand_discovery" => "demands",
        _ => return Err(ToolError::Unknown(name.to_owned())),
    };
    Ok(json!({
        "input_count": result["input_count"],
        key: result[key]
    }))
}

fn request(
    client: &OpenApiClient,
    method: &str,
    path: &str,
    token: &str,
    params: Option<HashMap<String, String>>,
    json_body: Option<Value>,
) -> Result<Value, ToolError> {
    client
        .request(RequestSpec {
            method,
            path,
            token: Some(token),
            params,
            json_body,
            auth_required: true,
            ..RequestSpec::default()
        })
        .map_err(ToolError::Execution)
}

fn resolve_auth(args: &Map<String, Value>) -> Result<(String, String), ToolError> {
    let saved = saved_openapi()?;
    let token = optional_string(args, "token")
        .or_else(|| saved_string(&saved, "accessToken"))
        .ok_or_else(|| {
            ToolError::Execution("缺少 access_token，请先运行 douyin auth login".to_owned())
        })?;
    let open_id = optional_string(args, "open_id")
        .or_else(|| saved_string(&saved, "openId"))
        .ok_or_else(|| {
            ToolError::Execution("缺少 open_id，请先运行 douyin auth login".to_owned())
        })?;
    Ok((token, open_id))
}

fn saved_openapi() -> Result<Map<String, Value>, ToolError> {
    settings::load()
        .map(|data| settings::openapi(&data))
        .map_err(execution)
}

fn required_string(args: &Map<String, Value>, key: &str) -> Result<String, ToolError> {
    optional_string(args, key).ok_or_else(|| ToolError::Execution(format!("缺少必填参数: {key}")))
}

fn optional_string(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn saved_string(args: &Map<String, Value>, key: &str) -> Option<String> {
    optional_string(args, key)
}

fn integer(args: &Map<String, Value>, key: &str, default: i64) -> i64 {
    args.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn non_negative_integer(
    args: &Map<String, Value>,
    key: &str,
    default: usize,
) -> Result<usize, ToolError> {
    let value = args
        .get(key)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| ToolError::Execution(format!("{key} 必须是非负整数")))
        })
        .transpose()?
        .unwrap_or(default as u64);
    usize::try_from(value).map_err(execution)
}

fn positive_integer(args: &Map<String, Value>, key: &str, default: u64) -> Result<u64, ToolError> {
    let value = args
        .get(key)
        .map(|value| {
            value
                .as_u64()
                .filter(|value| *value > 0)
                .ok_or_else(|| ToolError::Execution(format!("{key} 必须是正整数")))
        })
        .transpose()?
        .unwrap_or(default);
    Ok(value)
}

fn string_map(
    args: &Map<String, Value>,
    key: &str,
) -> Result<Option<HashMap<String, String>>, ToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let object = value
        .as_object()
        .ok_or_else(|| ToolError::Execution(format!("{key} 必须是对象")))?;
    object
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|value| (key.clone(), value.to_owned()))
                .ok_or_else(|| ToolError::Execution(format!("{key} 的值必须是字符串")))
        })
        .collect::<Result<HashMap<_, _>, _>>()
        .map(Some)
}

fn execution(error: impl ToString) -> ToolError {
    ToolError::Execution(error.to_string())
}

enum ToolError {
    Unknown(String),
    Execution(String),
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn write_message(writer: &mut impl Write, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, value).map_err(|error| error.to_string())?;
    writer.write_all(b"\n").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::handle_message;
    use serde_json::json;

    #[test]
    fn initialize_negotiates_supported_version() {
        let response = handle_message(&json!({
            "jsonrpc":"2.0", "id":1, "method":"initialize",
            "params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}
        })).unwrap();
        assert_eq!(response["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(response["result"]["serverInfo"]["name"], "douyin");
    }

    #[test]
    fn tools_list_exposes_openapi_and_offline_insights_tools() {
        let response =
            handle_message(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).unwrap();
        let names: Vec<_> = response["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        for expected in [
            "auth_status",
            "userinfo",
            "comment_list",
            "comment_replies",
            "comment_reply",
            "im_message_send",
            "openapi_request",
            "hot_words",
            "hot_memes",
            "demand_discovery",
        ] {
            assert!(names.contains(&expected));
        }
    }

    #[test]
    fn offline_insights_tool_call_does_not_require_authorization() {
        let response = handle_message(&json!({
            "jsonrpc":"2.0","id":5,"method":"tools/call","params":{
                "name":"demand_discovery",
                "arguments":{"texts":["求链接","求链接"],"top":5,"min_count":2}
            }
        }))
        .unwrap();
        assert_eq!(
            response["result"]["structuredContent"]["demands"][0]["text"],
            "求链接"
        );
        assert_eq!(response["result"]["isError"], false);
    }

    #[test]
    fn unknown_tool_is_protocol_error() {
        let response = handle_message(&json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"missing","arguments":{}}
        }))
        .unwrap();
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn batch_omits_notification_responses() {
        let response = handle_message(&json!([
            {"jsonrpc":"2.0","method":"notifications/initialized"},
            {"jsonrpc":"2.0","id":4,"method":"ping"}
        ]))
        .unwrap();
        assert_eq!(response.as_array().unwrap().len(), 1);
        assert_eq!(response[0]["id"], 4);
    }
}
