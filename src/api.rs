use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};

use clap::{Args, Subcommand, ValueEnum};
use reqwest::Method;
use serde_json::{Value, json};

use crate::{
    io, oauth,
    openapi::{OpenApiClient, Request},
};

#[derive(Args)]
pub struct ApiArgs {
    #[command(subcommand)]
    command: ApiCommand,
}

#[derive(Args)]
struct AppCredentials {
    #[arg(long, env = "DOUYIN_CLIENT_KEY")]
    client_key: String,
    #[arg(long, env = "DOUYIN_CLIENT_SECRET", hide_env_values = true)]
    client_secret: String,
}

#[derive(Args)]
struct AuthOptions {
    #[arg(long, env = "DOUYIN_ACCESS_TOKEN", hide_env_values = true)]
    token: Option<String>,
    #[arg(long)]
    open_id: Option<String>,
}

#[derive(Args)]
struct CommentPage {
    #[command(flatten)]
    auth: AuthOptions,
    #[arg(long)]
    item_id: String,
    #[arg(long, default_value_t = 0)]
    cursor: u64,
    #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=20))]
    count: u32,
    #[arg(long,value_parser=clap::value_parser!(u8).range(0..=2))]
    sort_type: Option<u8>,
}

#[derive(Subcommand)]
enum ApiCommand {
    /// 获取官方 client_token
    ClientToken(AppCredentials),
    /// 生成官方授权链接
    AuthorizeUrl {
        #[arg(long, env = "DOUYIN_CLIENT_KEY")]
        client_key: String,
        #[arg(long)]
        redirect_uri: String,
        #[arg(long, required = true, value_delimiter = ',')]
        scope: Vec<String>,
        #[arg(long)]
        state: Option<String>,
    },
    /// 使用 code 交换官方 access_token；使用 auth code 可安全保存
    AccessToken {
        #[command(flatten)]
        app: AppCredentials,
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
    /// 获取官方视频评论列表
    CommentList(CommentPage),
    /// 获取官方评论回复列表
    CommentReplies {
        #[command(flatten)]
        page: CommentPage,
        #[arg(long)]
        comment_id: String,
    },
    /// 通过官方接口发送评论或回复
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
    /// 通过官方接口发送文字、图片或视频私信
    ImMessageSend {
        #[command(flatten)]
        auth: AuthOptions,
        #[arg(long)]
        to_user_id: String,
        #[arg(long, value_enum, default_value = "im-reply-msg")]
        scene: Scene,
        #[arg(long)]
        msg_id: String,
        #[arg(long)]
        conversation_id: String,
        #[arg(long, value_enum, default_value = "text")]
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
    /// 同源通用 OpenAPI 请求；写请求需要确认或 --yes
    Request {
        method: String,
        path: String,
        #[arg(long, env = "DOUYIN_ACCESS_TOKEN", hide_env_values = true)]
        token: Option<String>,
        #[arg(long = "param")]
        params: Vec<String>,
        /// JSON 字符串，@文件路径，或 @- 读取 stdin
        #[arg(long = "json", conflicts_with = "forms")]
        body: Option<String>,
        #[arg(long = "form")]
        forms: Vec<String>,
        #[arg(long = "header")]
        headers: Vec<String>,
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum MessageType {
    Text,
    Image,
    Video,
}
#[derive(Clone, Copy, ValueEnum)]
enum Scene {
    #[value(name = "im-reply-msg", alias = "reply")]
    Reply,
    #[value(name = "im-enter-direct-msg", alias = "enter")]
    Enter,
}

pub fn run(args: ApiArgs) -> Result<(), String> {
    let client = OpenApiClient::new()?;
    let value = match args.command {
        ApiCommand::ClientToken(app) => client.token(
            "/oauth/client_token/",
            &[
                ("client_key", &app.client_key),
                ("client_secret", &app.client_secret),
                ("grant_type", "client_credential"),
            ],
        )?,
        ApiCommand::AccessToken { app, code } => client.token(
            "/oauth/access_token/",
            &[
                ("client_key", &app.client_key),
                ("client_secret", &app.client_secret),
                ("code", &code),
                ("grant_type", "authorization_code"),
            ],
        )?,
        ApiCommand::RefreshToken {
            client_key,
            refresh_token,
        } => client.token(
            "/oauth/refresh_token/",
            &[
                ("client_key", &client_key),
                ("refresh_token", &refresh_token),
                ("grant_type", "refresh_token"),
            ],
        )?,
        ApiCommand::RenewRefreshToken {
            client_key,
            refresh_token,
        } => client.token(
            "/oauth/renew_refresh_token/",
            &[
                ("client_key", &client_key),
                ("refresh_token", &refresh_token),
            ],
        )?,
        ApiCommand::AuthorizeUrl {
            client_key,
            redirect_uri,
            scope,
            state,
        } => {
            return io::write_output(
                &client.authorize_url(
                    &client_key,
                    &redirect_uri,
                    &scope,
                    state.as_deref().unwrap_or(""),
                )?,
                None,
            );
        }
        ApiCommand::Userinfo(auth) => {
            let (token, id) = resolve_auth(auth)?;
            let mut request = Request::new(Method::GET, "/oauth/userinfo/");
            request.token = Some(&token);
            request.params.push(("open_id".into(), id));
            client.request(request)?
        }
        ApiCommand::CommentList(page) => comment_page(&client, page, None)?,
        ApiCommand::CommentReplies { page, comment_id } => {
            comment_page(&client, page, Some(comment_id))?
        }
        ApiCommand::CommentReply {
            auth,
            item_id,
            comment_id,
            content,
            yes,
        } => {
            validate_text(&content, 100, false)?;
            let (token, id) = resolve_auth(auth)?;
            confirm_write(yes)?;
            let mut request = Request::new(Method::POST, "/item/comment/reply/");
            request.token = Some(&token);
            request.params.push(("open_id".into(), id));
            let mut body = json!({"item_id":item_id,"content":content});
            if let Some(id) = comment_id {
                body["comment_id"] = json!(id);
            }
            request.json = Some(body);
            client.request(request)?
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
            let content = message_content(message_type, text, media_id, item_id)?;
            let (token, id) = resolve_auth(auth)?;
            confirm_write(yes)?;
            let mut request = Request::new(Method::POST, "/im/send/msg/");
            request.token = Some(&token);
            request.params.push(("open_id".into(), id));
            request.json = Some(
                json!({"to_user_id":to_user_id,"scene":match scene{Scene::Reply=>"im_reply_msg",Scene::Enter=>"im_enter_direct_msg"},"msg_id":msg_id,"conversation_id":conversation_id,"content":content}),
            );
            client.request(request)?
        }
        ApiCommand::Request {
            method,
            path,
            token,
            params,
            body,
            forms,
            headers,
            yes,
        } => {
            let method = Method::from_bytes(method.to_ascii_uppercase().as_bytes())
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
                return Err("不支持此 HTTP method".into());
            }
            let saved = if token.is_none() {
                oauth::load()?
            } else {
                json!({})
            };
            let token = token
                .or_else(|| oauth::field(&saved, "accessToken").map(str::to_owned))
                .filter(|v| !v.trim().is_empty())
                .ok_or("通用 OpenAPI 请求需要 --token 或官方 OAuth 授权")?;
            let mut request = Request::new(method.clone(), &path);
            request.token = Some(&token);
            request.params = parse_pairs(params)?;
            request.form = parse_pairs(forms)?;
            request.json = body
                .map(|source| {
                    let text = if let Some(path) = source.strip_prefix('@') {
                        io::read_input(path)?
                    } else {
                        source
                    };
                    serde_json::from_str(&text).map_err(|_| "--json 不是合法 JSON".to_owned())
                })
                .transpose()?;
            let mut seen = std::collections::BTreeSet::new();
            request.headers = parse_pairs(headers)?
                .into_iter()
                .map(|(name, value)| {
                    if !seen.insert(name.to_ascii_lowercase()) {
                        return Err("自定义请求头重复".into());
                    }
                    Ok((name, value))
                })
                .collect::<Result<BTreeMap<_, _>, String>>()?;
            if !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS) {
                confirm_write(yes)?;
            }
            client.request(request)?
        }
    };
    io::write_output(
        &serde_json::to_string_pretty(&value).map_err(|_| "OpenAPI 结果序列化失败")?,
        None,
    )
}

fn comment_page(
    client: &OpenApiClient,
    page: CommentPage,
    reply: Option<String>,
) -> Result<Value, String> {
    let (token, id) = resolve_auth(page.auth)?;
    let mut request = Request::new(
        Method::GET,
        if reply.is_some() {
            "/item/comment/reply/list/"
        } else {
            "/item/comment/list/"
        },
    );
    request.token = Some(&token);
    request.params = vec![
        ("open_id".into(), id),
        ("item_id".into(), page.item_id),
        ("cursor".into(), page.cursor.to_string()),
        ("count".into(), page.count.to_string()),
    ];
    if let Some(id) = reply {
        request.params.push(("comment_id".into(), id));
    }
    if let Some(sort) = page.sort_type {
        request.params.push(("sort_type".into(), sort.to_string()));
    }
    client.request(request)
}

fn resolve_auth(auth: AuthOptions) -> Result<(String, String), String> {
    resolve_credentials(auth.token, auth.open_id)
}

pub(crate) fn resolve_credentials(
    token: Option<String>,
    open_id: Option<String>,
) -> Result<(String, String), String> {
    let saved = if token.is_none() || open_id.is_none() {
        oauth::load()?
    } else {
        json!({})
    };
    let token = token
        .or_else(|| oauth::field(&saved, "accessToken").map(str::to_owned))
        .filter(|v| !v.trim().is_empty())
        .ok_or("缺少官方 access_token，请运行 auth login")?;
    let id = open_id
        .or_else(|| oauth::field(&saved, "openId").map(str::to_owned))
        .filter(|v| !v.trim().is_empty())
        .ok_or("缺少官方 open_id，请运行 auth login")?;
    Ok((token, id))
}

fn parse_pairs(values: Vec<String>) -> Result<Vec<(String, String)>, String> {
    values
        .into_iter()
        .map(|pair| {
            let (key, value) = pair.split_once('=').ok_or("参数必须使用 KEY=VALUE 格式")?;
            if key.trim().is_empty() || key.chars().any(char::is_control) {
                return Err("参数名称无效".into());
            }
            Ok((key.to_owned(), value.to_owned()))
        })
        .collect()
}
pub(crate) fn validate_text(value: &str, max: usize, newlines: bool) -> Result<(), String> {
    if value.trim().is_empty()
        || value.chars().count() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(newlines && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(format!("文本为空、含不支持的控制字符或超过 {max} 字符"));
    }
    Ok(())
}
pub(crate) fn message_content(
    kind: MessageType,
    text: Option<String>,
    media: Option<String>,
    item: Option<String>,
) -> Result<Value, String> {
    let (number, name, key, value) = match kind {
        MessageType::Text => (1, "text", "text", text),
        MessageType::Image => (2, "image", "media_id", media),
        MessageType::Video => (3, "video", "item_id", item),
    };
    let value = value
        .filter(|v| !v.trim().is_empty())
        .ok_or("消息类型缺少对应的 text、media-id 或 item-id")?;
    if matches!(kind, MessageType::Text) {
        validate_text(&value, 1000, true)?;
    }
    let mut result = json!({"msg_type":number});
    result[name] = json!({key:value});
    Ok(result)
}
pub(crate) fn confirm_write(yes: bool) -> Result<(), String> {
    if yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        return Err("写请求需要 --yes；未发送请求".into());
    }
    eprint!("将通过官方接口写入数据，是否继续？[y/N] ");
    std::io::stderr().flush().map_err(|_| "无法输出确认提示")?;
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|_| "无法读取确认输入")?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        Err("已取消，未发送请求".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn message_content_keeps_official_shape_and_validates_unicode() {
        assert_eq!(
            message_content(MessageType::Text, Some("你好\n已收到".into()), None, None).unwrap(),
            json!({"msg_type":1,"text":{"text":"你好\n已收到"}})
        );
        assert_eq!(
            message_content(MessageType::Image, None, Some("media".into()), None).unwrap(),
            json!({"msg_type":2,"image":{"media_id":"media"}})
        );
        assert!(message_content(MessageType::Video, None, None, None).is_err());
        assert!(validate_text(&"中".repeat(101), 100, false).is_err());
        assert!(validate_text("\n", 100, false).is_err());
    }
    #[test]
    fn key_value_parser_preserves_embedded_equals_and_repeated_query_fields() {
        assert_eq!(
            parse_pairs(vec!["a=x=y".into(), "a=z".into()]).unwrap(),
            vec![("a".into(), "x=y".into()), ("a".into(), "z".into())]
        );
        assert!(parse_pairs(vec!["=secret".into()]).is_err());
        assert!(parse_pairs(vec!["token-with-no-equals".into()]).is_err());
    }
}
