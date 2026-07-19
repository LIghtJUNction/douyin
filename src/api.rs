use std::collections::HashMap;
use std::io::{self, Write};

use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};

use crate::openapi::{OpenApiClient, RequestSpec, im_message_body};
use crate::settings;

#[derive(Debug, Args)]
pub struct ApiArgs {
    #[command(subcommand)]
    command: ApiCommand,
}

#[derive(Debug, Subcommand)]
enum ApiCommand {
    /// 获取 client_token
    ClientToken {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: String,
        #[arg(long, env = "DOUYIN_CLIENT_SECRET")]
        client_secret: String,
    },
    /// 生成官方 OAuth 授权链接
    AuthorizeUrl {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: String,
        #[arg(long)]
        redirect_uri: String,
        #[arg(long, required = true)]
        scope: Vec<String>,
        #[arg(long)]
        state: Option<String>,
    },
    /// 用 OAuth code 换取 access_token
    AccessToken {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: String,
        #[arg(long, env = "DOUYIN_CLIENT_SECRET")]
        client_secret: String,
        #[arg(long)]
        code: String,
    },
    /// 刷新官方 access_token
    RefreshToken {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: String,
        #[arg(long)]
        refresh_token: String,
    },
    /// 续期官方 refresh_token
    RenewRefreshToken {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: String,
        #[arg(long)]
        refresh_token: String,
    },
    /// 获取官方授权用户信息
    Userinfo(AuthOptions),
    /// 调用官方接口获取视频评论列表
    CommentList {
        #[command(flatten)]
        auth: AuthOptions,
        #[arg(long)]
        item_id: String,
        #[arg(long, default_value_t = 0)]
        cursor: u64,
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
    /// 调用官方接口获取评论回复列表
    CommentReplies {
        #[command(flatten)]
        auth: AuthOptions,
        #[arg(long)]
        item_id: String,
        #[arg(long)]
        comment_id: String,
        #[arg(long, default_value_t = 0)]
        cursor: u64,
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
    /// 调用官方接口回复视频评论
    CommentReply {
        #[command(flatten)]
        auth: AuthOptions,
        #[arg(long)]
        item_id: String,
        #[arg(long)]
        comment_id: Option<String>,
        #[arg(long)]
        content: String,
        #[arg(long)]
        yes: bool,
    },
    /// 调用企业号 OpenAPI 发送私信消息
    ImMessageSend {
        #[command(flatten)]
        auth: AuthOptions,
        #[arg(long)]
        to_user_id: String,
        #[arg(long, value_enum, default_value_t = MessageType::Text)]
        message_type: MessageType,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        media_id: Option<String>,
        #[arg(long)]
        item_id: Option<String>,
        #[arg(long)]
        card_id: Option<String>,
        #[arg(long)]
        persona_id: Option<String>,
        #[arg(long)]
        client_msg_id: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    /// 调用任意官方 OpenAPI 路径
    Request {
        method: String,
        path: String,
        #[arg(long, env = "DOUYIN_ACCESS_TOKEN")]
        token: Option<String>,
        #[arg(long = "param")]
        params: Vec<String>,
        #[arg(long = "json")]
        json_text: Option<String>,
        #[arg(long = "form")]
        forms: Vec<String>,
        #[arg(long = "header")]
        headers: Vec<String>,
    },
}

#[derive(Debug, Args)]
struct AuthOptions {
    /// 默认读取已保存 token
    #[arg(long, env = "DOUYIN_ACCESS_TOKEN")]
    token: Option<String>,
    /// 默认读取已保存 open_id
    #[arg(long)]
    open_id: Option<String>,
}

#[derive(Clone, Debug, ValueEnum)]
enum MessageType {
    Text,
    Image,
    Video,
    Card,
}

impl MessageType {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Card => "card",
        }
    }
}

pub fn run(args: ApiArgs) -> Result<(), String> {
    let client = OpenApiClient::new()?;
    let response = match args.command {
        ApiCommand::ClientToken {
            client_key,
            client_secret,
        } => client.client_token(&client_key, &client_secret)?,
        ApiCommand::AuthorizeUrl {
            client_key,
            redirect_uri,
            scope,
            state,
        } => {
            println!(
                "{}",
                client.authorize_url(&client_key, &redirect_uri, &scope, state.as_deref())?
            );
            return Ok(());
        }
        ApiCommand::AccessToken {
            client_key,
            client_secret,
            code,
        } => client.access_token(&client_key, &client_secret, &code)?,
        ApiCommand::RefreshToken {
            client_key,
            refresh_token,
        } => client.refresh_token(&client_key, &refresh_token)?,
        ApiCommand::RenewRefreshToken {
            client_key,
            refresh_token,
        } => client.renew_refresh_token(&client_key, &refresh_token)?,
        ApiCommand::Userinfo(auth) => {
            let (token, open_id) = resolve_auth(auth)?;
            client.request(RequestSpec {
                method: "GET",
                path: "/oauth/userinfo/",
                token: Some(&token),
                params: Some(HashMap::from([("open_id".to_owned(), open_id)])),
                auth_required: true,
                ..RequestSpec::default()
            })?
        }
        ApiCommand::CommentList {
            auth,
            item_id,
            cursor,
            count,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            client.request(RequestSpec {
                method: "GET",
                path: "/item/comment/list/",
                token: Some(&token),
                params: Some(HashMap::from([
                    ("open_id".to_owned(), open_id),
                    ("item_id".to_owned(), item_id),
                    ("cursor".to_owned(), cursor.to_string()),
                    ("count".to_owned(), count.to_string()),
                ])),
                auth_required: true,
                ..RequestSpec::default()
            })?
        }
        ApiCommand::CommentReplies {
            auth,
            item_id,
            comment_id,
            cursor,
            count,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            client.request(RequestSpec {
                method: "GET",
                path: "/item/comment/reply/list/",
                token: Some(&token),
                params: Some(HashMap::from([
                    ("open_id".to_owned(), open_id),
                    ("item_id".to_owned(), item_id),
                    ("comment_id".to_owned(), comment_id),
                    ("cursor".to_owned(), cursor.to_string()),
                    ("count".to_owned(), count.to_string()),
                ])),
                auth_required: true,
                ..RequestSpec::default()
            })?
        }
        ApiCommand::CommentReply {
            auth,
            item_id,
            comment_id,
            content,
            yes,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            confirm_write("将通过官方 OpenAPI 发送评论回复，是否继续？", yes)?;
            let mut body = Map::from_iter([
                ("item_id".to_owned(), json!(item_id)),
                ("content".to_owned(), json!(content)),
            ]);
            if let Some(comment_id) = comment_id {
                body.insert("comment_id".to_owned(), json!(comment_id));
            }
            client.request(RequestSpec {
                method: "POST",
                path: "/item/comment/reply/",
                token: Some(&token),
                params: Some(HashMap::from([("open_id".to_owned(), open_id)])),
                json_body: Some(Value::Object(body)),
                auth_required: true,
                ..RequestSpec::default()
            })?
        }
        ApiCommand::ImMessageSend {
            auth,
            to_user_id,
            message_type,
            text,
            media_id,
            item_id,
            card_id,
            persona_id,
            client_msg_id,
            yes,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            let content = message_content(&message_type, text, media_id, item_id, card_id)?;
            confirm_write("将通过企业号 OpenAPI 发送私信消息，是否继续？", yes)?;
            client.request(RequestSpec {
                method: "POST",
                path: "/enterprise/im/message/send/",
                token: Some(&token),
                params: Some(HashMap::from([("open_id".to_owned(), open_id)])),
                json_body: Some(im_message_body(
                    &to_user_id,
                    message_type.as_str(),
                    content,
                    persona_id.as_deref(),
                    client_msg_id.as_deref(),
                )),
                auth_required: true,
                ..RequestSpec::default()
            })?
        }
        ApiCommand::Request {
            method,
            path,
            token,
            params,
            json_text,
            forms,
            headers,
        } => {
            let data = settings::load().map_err(|error| error.to_string())?;
            let saved = settings::openapi(&data);
            let token = token.or_else(|| saved_string(&saved, "accessToken"));
            client.request(RequestSpec {
                method: &method,
                path: &path,
                token: token.as_deref(),
                params: parse_key_values(params)?,
                json_body: parse_json(json_text)?,
                form: parse_key_values(forms)?,
                headers: parse_key_values(headers)?,
                auth_required: true,
            })?
        }
    };
    print_json(&response)
}

fn resolve_auth(options: AuthOptions) -> Result<(String, String), String> {
    let data = settings::load().map_err(|error| error.to_string())?;
    let saved = settings::openapi(&data);
    let token = options
        .token
        .or_else(|| saved_string(&saved, "accessToken"))
        .ok_or_else(|| "缺少 access_token，请先运行 douyin auth login".to_owned())?;
    let open_id = options
        .open_id
        .or_else(|| saved_string(&saved, "openId"))
        .ok_or_else(|| "缺少 open_id，请先运行 douyin auth login".to_owned())?;
    Ok((token, open_id))
}

fn saved_string(values: &Map<String, Value>, key: &str) -> Option<String> {
    values
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn message_content(
    message_type: &MessageType,
    text: Option<String>,
    media_id: Option<String>,
    item_id: Option<String>,
    card_id: Option<String>,
) -> Result<Value, String> {
    let (key, value, error) = match message_type {
        MessageType::Text => ("text", text, "message-type=text 需要 --text"),
        MessageType::Image => ("media_id", media_id, "message-type=image 需要 --media-id"),
        MessageType::Video => ("item_id", item_id, "message-type=video 需要 --item-id"),
        MessageType::Card => ("card_id", card_id, "message-type=card 需要 --card-id"),
    };
    let value = value.filter(|value| !value.is_empty()).ok_or(error)?;
    Ok(json!({key: value}))
}

fn parse_key_values(values: Vec<String>) -> Result<Option<HashMap<String, String>>, String> {
    if values.is_empty() {
        return Ok(None);
    }
    values
        .into_iter()
        .map(|value| {
            let (key, value) = value
                .split_once('=')
                .ok_or_else(|| format!("参数必须是 key=value 格式: {value}"))?;
            if key.is_empty() {
                return Err(format!("参数 key 不能为空: ={value}"));
            }
            Ok((key.to_owned(), value.to_owned()))
        })
        .collect::<Result<HashMap<_, _>, _>>()
        .map(Some)
}

fn parse_json(text: Option<String>) -> Result<Option<Value>, String> {
    let Some(text) = text else {
        return Ok(None);
    };
    let value: Value =
        serde_json::from_str(&text).map_err(|error| format!("--json 不是合法 JSON: {error}"))?;
    if !value.is_object() && !value.is_array() {
        return Err("--json 必须是 JSON object 或 array".to_owned());
    }
    Ok(Some(value))
}

fn confirm_write(prompt: &str, yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    print!("{prompt} [y/N]: ");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .map_err(|error| error.to_string())?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err("操作已取消".to_owned())
    }
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
    use super::{MessageType, message_content, parse_json, parse_key_values};
    use serde_json::json;

    #[test]
    fn text_message_requires_text() {
        assert_eq!(
            message_content(&MessageType::Text, None, None, None, None).unwrap_err(),
            "message-type=text 需要 --text"
        );
        assert_eq!(
            message_content(
                &MessageType::Text,
                Some("你好".to_owned()),
                None,
                None,
                None
            )
            .unwrap(),
            json!({"text": "你好"})
        );
    }

    #[test]
    fn generic_request_parsers_reject_invalid_values() {
        assert!(parse_key_values(vec!["invalid".to_owned()]).is_err());
        assert!(parse_json(Some("1".to_owned())).is_err());
        assert_eq!(
            parse_key_values(vec!["open_id=value".to_owned()])
                .unwrap()
                .unwrap()["open_id"],
            "value"
        );
    }
}
