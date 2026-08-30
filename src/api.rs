use std::collections::HashMap;
use std::io::{self, Write};

use clap::{Args, Subcommand, ValueEnum};
use serde_json::{Map, Value, json};

use crate::err;
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
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=20))]
        count: u32,
        /// 0=综合排序，1=最多点赞，2=最新发布
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=2))]
        sort_type: Option<u8>,
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
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=20))]
        count: u32,
        /// 0=综合排序，1=最多点赞，2=最新发布
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=2))]
        sort_type: Option<u8>,
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
    /// 通过官方私信接口回复或首次进入会话
    ImMessageSend {
        #[command(flatten)]
        auth: AuthOptions,
        #[arg(long)]
        to_user_id: String,
        /// 私信场景；reply/enter 也可作为别名
        #[arg(long, value_enum, default_value_t = ImScene::Reply)]
        scene: ImScene,
        /// 回调事件中的消息 ID
        #[arg(long)]
        msg_id: String,
        /// 回调事件中的会话 ID
        #[arg(long)]
        conversation_id: String,
        #[arg(long, value_enum, default_value_t = MessageType::Text)]
        message_type: MessageType,
        #[arg(long)]
        text: Option<String>,
        #[arg(long)]
        media_id: Option<String>,
        #[arg(long)]
        item_id: Option<String>,
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
}

#[derive(Clone, Debug, ValueEnum)]
enum ImScene {
    #[value(name = "im-reply-msg", alias = "reply")]
    Reply,
    #[value(name = "im-enter-direct-msg", alias = "enter")]
    Enter,
}

impl ImScene {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Reply => "im_reply_msg",
            Self::Enter => "im_enter_direct_msg",
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
            sort_type,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            let mut params = HashMap::from([
                ("open_id".to_owned(), open_id),
                ("item_id".to_owned(), item_id),
                ("cursor".to_owned(), cursor.to_string()),
                ("count".to_owned(), count.to_string()),
            ]);
            if let Some(sort_type) = sort_type {
                params.insert("sort_type".to_owned(), sort_type.to_string());
            }
            client.request(RequestSpec {
                method: "GET",
                path: "/item/comment/list/",
                token: Some(&token),
                params: Some(params),
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
            sort_type,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            let mut params = HashMap::from([
                ("open_id".to_owned(), open_id),
                ("item_id".to_owned(), item_id),
                ("comment_id".to_owned(), comment_id),
                ("cursor".to_owned(), cursor.to_string()),
                ("count".to_owned(), count.to_string()),
            ]);
            if let Some(sort_type) = sort_type {
                params.insert("sort_type".to_owned(), sort_type.to_string());
            }
            client.request(RequestSpec {
                method: "GET",
                path: "/item/comment/reply/list/",
                token: Some(&token),
                params: Some(params),
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
            validate_text(&content, "评论内容", 100, false)?;
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
            scene,
            msg_id,
            conversation_id,
            message_type,
            text,
            media_id,
            item_id,
            yes,
        } => {
            let (token, open_id) = resolve_auth(auth)?;
            let content = message_content(&message_type, text, media_id, item_id)?;
            confirm_write("将通过官方 OpenAPI 发送私信消息，是否继续？", yes)?;
            client.request(RequestSpec {
                method: "POST",
                path: "/im/send/msg/",
                token: Some(&token),
                params: Some(HashMap::from([("open_id".to_owned(), open_id)])),
                json_body: Some(im_message_body(
                    &to_user_id,
                    scene.as_str(),
                    &msg_id,
                    &conversation_id,
                    content,
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
            let data = settings::load().map_err(err)?;
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
    let data = settings::load().map_err(err)?;
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
) -> Result<Value, String> {
    let (code, kind, key, value, error) = match message_type {
        MessageType::Text => (1, "text", "text", text, "message-type=text 需要 --text"),
        MessageType::Image => (
            2,
            "image",
            "media_id",
            media_id,
            "message-type=image 需要 --media-id",
        ),
        MessageType::Video => (
            3,
            "video",
            "item_id",
            item_id,
            "message-type=video 需要 --item-id",
        ),
    };
    let value = value
        .filter(|value| !value.trim().is_empty())
        .ok_or(error)?;
    if matches!(message_type, MessageType::Text) {
        validate_text(&value, "私信文本", 1_000, true)?;
    }
    let payload = Value::Object(Map::from_iter([(key.to_owned(), json!(value))]));
    Ok(Value::Object(Map::from_iter([
        ("msg_type".to_owned(), json!(code)),
        (kind.to_owned(), payload),
    ])))
}

fn validate_text(
    value: &str,
    name: &str,
    max_chars: usize,
    forbid_links: bool,
) -> Result<(), String> {
    let length = value.chars().count();
    if length == 0 {
        return Err(format!("{name}不能为空"));
    }
    if length > max_chars {
        return Err(format!(
            "{name}不能超过 {max_chars} 个字符（当前 {length}）"
        ));
    }
    if forbid_links && (value.contains("http://") || value.contains("https://")) {
        return Err(format!("{name}不能包含链接"));
    }
    Ok(())
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
    io::stdout().flush().map_err(err)?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).map_err(err)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err("操作已取消".to_owned())
    }
}

fn print_json(value: &Value) -> Result<(), String> {
    println!("{}", serde_json::to_string_pretty(value).map_err(err)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{MessageType, message_content, parse_json, parse_key_values, validate_text};
    use crate::test_support::{must, present};
    use serde_json::json;

    #[test]
    fn text_message_requires_text_and_uses_current_content_shape() {
        assert_eq!(
            message_content(&MessageType::Text, None, None, None).unwrap_err(),
            "message-type=text 需要 --text"
        );
        assert_eq!(
            must(message_content(
                &MessageType::Text,
                Some("你好".to_owned()),
                None,
                None
            )),
            json!({"msg_type": 1, "text": {"text": "你好"}})
        );
        assert!(
            message_content(
                &MessageType::Text,
                Some("https://example.com".to_owned()),
                None,
                None
            )
            .is_err()
        );
        assert!(validate_text(&"字".repeat(101), "评论内容", 100, false).is_err());
    }

    #[test]
    fn generic_request_parsers_reject_invalid_values() {
        assert!(parse_key_values(vec!["invalid".to_owned()]).is_err());
        assert!(parse_json(Some("1".to_owned())).is_err());
        let values = present(must(parse_key_values(vec!["open_id=value".to_owned()])));
        assert_eq!(values["open_id"], "value");
    }
}
