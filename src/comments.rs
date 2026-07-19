use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::Duration;

use clap::{Args, ValueEnum};
use reqwest::blocking::Client;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT,
};
use serde_json::{Map, Value, json};

use crate::{cookie, settings};

const BASE_URL: &str = "https://www.douyin.com";
const COMMENT_LIST: &str = "/aweme/v1/web/comment/list/";
const COMMENT_REPLIES: &str = "/aweme/v1/web/comment/list/reply/";
pub(crate) const DEFAULT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/132.0.0.0 Safari/537.36";
const SIGN_SCRIPT: &str = include_str!("../assets/douyin.js");

#[derive(Debug, Args)]
pub struct CommentArgs {
    /// 作品 ID、视频 URL 或图文 URL
    target: String,
    /// 最多抓取一级评论数，0 表示不限制
    #[arg(short, long, default_value_t = 100)]
    limit: usize,
    /// 每页请求数量
    #[arg(long, default_value_t = 20)]
    count: usize,
    /// 同时抓取评论楼中楼回复
    #[arg(long)]
    with_replies: bool,
    /// 每条评论最多抓取回复数，0 表示不限制
    #[arg(long, default_value_t = 20)]
    reply_limit: usize,
    /// 分页请求间隔秒数
    #[arg(long = "sleep", default_value_t = 0.8)]
    sleep_seconds: f64,
    /// 输出文件；不传则输出到 stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// 输出格式
    #[arg(long = "format", value_enum, default_value_t = OutputFormat::Raw)]
    output_format: OutputFormat,
    #[arg(long, default_value = "user")]
    comment_role: String,
    #[arg(long, default_value = "assistant")]
    reply_role: String,
    #[arg(long, default_value_t = 0)]
    min_comment_digg: i64,
    #[arg(long, default_value_t = 0)]
    min_reply_digg: i64,
    #[arg(long)]
    include_single_comments: bool,
    /// 本次运行使用的 Cookie；默认读取保存的 Cookie
    #[arg(short, long, env = "DOUYIN_COOKIE")]
    cookie: Option<String>,
}

#[derive(Clone, Debug, ValueEnum)]
enum OutputFormat {
    Raw,
    ChatmlJsonl,
    ChatmlJson,
}

pub fn run(args: CommentArgs) -> Result<(), String> {
    let saved = settings::load().map_err(|error| error.to_string())?;
    let cookie_value = args
        .cookie
        .clone()
        .or_else(|| {
            saved
                .get("cookie")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "未登录。请先运行: douyin auth cookie-login".to_owned())?;
    if !cookie::validate(&cookie_value) {
        return Err("Cookie 格式校验失败".to_owned());
    }
    let user_agent = saved
        .get("userAgent")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_USER_AGENT);
    let aweme_id = extract_aweme_id(&args.target)?;
    let crawler = CommentCrawler::new(&cookie_value, user_agent)?;
    let data = crawler.crawl(&aweme_id, &args)?;
    let output = match args.output_format {
        OutputFormat::Raw => {
            serde_json::to_string_pretty(&data).map_err(|error| error.to_string())?
        }
        OutputFormat::ChatmlJson => serde_json::to_string_pretty(&format_chatml(&data, &args))
            .map_err(|error| error.to_string())?,
        OutputFormat::ChatmlJsonl => format_chatml(&data, &args)
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?
            .join("\n"),
    };
    write_output(&output, args.output.as_deref())?;
    if let Some(path) = args.output {
        eprintln!("评论已保存: {}", path.display());
    }
    Ok(())
}

struct CommentCrawler {
    client: Client,
    user_agent: String,
}

impl CommentCrawler {
    fn new(cookie: &str, user_agent: &str) -> Result<Self, String> {
        let mut headers = HeaderMap::new();
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("zh-CN,zh;q=0.9"));
        headers.insert(REFERER, HeaderValue::from_static("https://www.douyin.com/"));
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(user_agent).map_err(|error| error.to_string())?,
        );
        headers.insert(
            COOKIE,
            HeaderValue::from_str(cookie).map_err(|error| error.to_string())?,
        );
        let client = Client::builder()
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            user_agent: user_agent.to_owned(),
        })
    }

    fn crawl(&self, aweme_id: &str, args: &CommentArgs) -> Result<Value, String> {
        let mut comments = Vec::new();
        let mut cursor = 0_i64;
        let mut has_more = true;
        while has_more && !reached_limit(comments.len(), args.limit) {
            let page = self.fetch_page(
                COMMENT_LIST,
                vec![
                    ("aweme_id", aweme_id.to_owned()),
                    ("cursor", cursor.to_string()),
                    ("count", args.count.to_string()),
                    ("item_type", "0".to_owned()),
                ],
            )?;
            let values = page
                .get("comments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if values.is_empty() {
                break;
            }
            for raw in values {
                let mut comment = normalize_comment(&raw);
                if args.with_replies {
                    let comment_id = comment.get("id").and_then(Value::as_str).unwrap_or("");
                    comment["replies"] =
                        Value::Array(self.crawl_replies(aweme_id, comment_id, args)?);
                }
                comments.push(comment);
                if reached_limit(comments.len(), args.limit) {
                    break;
                }
            }
            cursor = page.get("cursor").and_then(Value::as_i64).unwrap_or(0);
            has_more = truthy(page.get("has_more"));
            pause(has_more, args.sleep_seconds);
        }
        Ok(json!({"aweme_id": aweme_id, "comments": comments}))
    }

    fn crawl_replies(
        &self,
        aweme_id: &str,
        comment_id: &str,
        args: &CommentArgs,
    ) -> Result<Vec<Value>, String> {
        let mut replies = Vec::new();
        let mut cursor = 0_i64;
        let mut has_more = true;
        while has_more && !reached_limit(replies.len(), args.reply_limit) {
            let page = self.fetch_page(
                COMMENT_REPLIES,
                vec![
                    ("item_id", aweme_id.to_owned()),
                    ("comment_id", comment_id.to_owned()),
                    ("cursor", cursor.to_string()),
                    ("count", args.count.to_string()),
                    ("item_type", "0".to_owned()),
                ],
            )?;
            let values = page
                .get("comments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if values.is_empty() {
                break;
            }
            for raw in values {
                replies.push(normalize_comment(&raw));
                if reached_limit(replies.len(), args.reply_limit) {
                    break;
                }
            }
            cursor = page.get("cursor").and_then(Value::as_i64).unwrap_or(0);
            has_more = truthy(page.get("has_more"));
            pause(has_more, args.sleep_seconds);
        }
        Ok(replies)
    }

    fn fetch_page(&self, path: &str, mut params: Vec<(&str, String)>) -> Result<Value, String> {
        params.extend([
            ("device_platform", "webapp".to_owned()),
            ("aid", "6383".to_owned()),
            ("channel", "channel_pc_web".to_owned()),
        ]);
        let query = params
            .iter()
            .map(|(key, value)| {
                let encoded: String =
                    url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
                format!("{key}={encoded}")
            })
            .collect::<Vec<_>>()
            .join("&");
        let sign_function = if path.contains("reply") {
            "sign_reply"
        } else {
            "sign_datail"
        };
        let signature = sign(sign_function, &query, &self.user_agent)?;
        params.push(("a_bogus", signature));
        let response = self
            .client
            .get(format!("{BASE_URL}{path}"))
            .query(&params)
            .send()
            .map_err(|error| error.to_string())?;
        let status = response.status();
        let text = response.text().map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!("评论请求失败: {status} {text}"));
        }
        if text.is_empty() {
            return Err("响应体为空，Cookie 可能已失效".to_owned());
        }
        let data: Value =
            serde_json::from_str(&text).map_err(|error| format!("评论响应不是 JSON: {error}"))?;
        if contains_verify_check(&data) {
            return Err("触发验证码，请完成验证后再继续".to_owned());
        }
        if data.get("status_code").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Err(format!("评论接口返回失败状态: {text}"));
        }
        Ok(data)
    }
}

pub(crate) fn sign(function: &str, query: &str, user_agent: &str) -> Result<String, String> {
    let query = serde_json::to_string(query).map_err(|error| error.to_string())?;
    let user_agent = serde_json::to_string(user_agent).map_err(|error| error.to_string())?;
    let script = format!("{SIGN_SCRIPT}\nprocess.stdout.write({function}({query}, {user_agent}));");
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|error| {
            format!("无法启动 Node.js 签名运行时: {error}。评论抓取需要 node 命令。")
        })?;
    if !output.status.success() {
        return Err(format!(
            "生成 a_bogus 失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let value = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    if value.trim().is_empty() {
        Err("生成 a_bogus 得到空结果".to_owned())
    } else {
        Ok(value)
    }
}

pub fn extract_aweme_id(target: &str) -> Result<String, String> {
    let target = target.trim();
    if target.chars().all(|value| value.is_ascii_digit()) && !target.is_empty() {
        return Ok(target.to_owned());
    }
    let mut url = reqwest::Url::parse(target).map_err(|_| format!("无法识别作品 ID: {target}"))?;
    if url.host_str() == Some("v.douyin.com") {
        url = Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|error| error.to_string())?
            .get(url)
            .send()
            .map_err(|error| error.to_string())?
            .url()
            .clone();
    }
    let parts: Vec<_> = url
        .path_segments()
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        .collect();
    for marker in ["video", "note"] {
        if let Some(index) = parts.iter().position(|value| *value == marker)
            && let Some(value) = parts
                .get(index + 1)
                .filter(|value| value.chars().all(|c| c.is_ascii_digit()))
        {
            return Ok((*value).to_owned());
        }
    }
    parts
        .last()
        .filter(|value| value.chars().all(|c| c.is_ascii_digit()))
        .map(|value| (*value).to_owned())
        .ok_or_else(|| format!("无法识别作品 ID: {target}"))
}

pub fn normalize_comment(comment: &Value) -> Value {
    let user = comment.get("user").and_then(Value::as_object);
    json!({
        "id": first_string(comment, &["cid", "comment_id"]),
        "text": first_string(comment, &["text"]),
        "create_time": comment.get("create_time").cloned().unwrap_or(Value::Null),
        "digg_count": comment.get("digg_count").cloned().unwrap_or_else(|| json!(0)),
        "reply_comment_total": comment.get("reply_comment_total").cloned().unwrap_or_else(|| json!(0)),
        "ip_label": first_string(comment, &["ip_label"]),
        "user": {
            "uid": object_string(user, "uid"), "sec_uid": object_string(user, "sec_uid"),
            "nickname": object_string(user, "nickname"), "unique_id": object_string(user, "unique_id")
        }
    })
}

fn format_chatml(data: &Value, args: &CommentArgs) -> Vec<Value> {
    let aweme_id = data.get("aweme_id").and_then(Value::as_str).unwrap_or("");
    let mut records = Vec::new();
    for comment in data
        .get("comments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = comment
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if text.is_empty() || digg(comment) < args.min_comment_digg {
            continue;
        }
        let replies = comment
            .get("replies")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if replies.is_empty() && args.include_single_comments {
            records.push(json!({
                "messages":[{"role":args.comment_role,"content":text}],
                "metadata": metadata(aweme_id, comment, None)
            }));
        } else {
            for reply in replies {
                let reply_text = reply
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim();
                if reply_text.is_empty() || digg(&reply) < args.min_reply_digg {
                    continue;
                }
                records.push(json!({
                    "messages":[{"role":args.comment_role,"content":text},{"role":args.reply_role,"content":reply_text}],
                    "metadata": metadata(aweme_id, comment, Some(&reply))
                }));
            }
        }
    }
    records
}

fn metadata(aweme_id: &str, comment: &Value, reply: Option<&Value>) -> Value {
    let mut result = Map::from_iter([
        (
            "source".to_owned(),
            json!(if reply.is_some() {
                "douyin_comment_reply"
            } else {
                "douyin_comment"
            }),
        ),
        ("aweme_id".to_owned(), json!(aweme_id)),
        (
            "comment_id".to_owned(),
            json!(first_string(comment, &["id"])),
        ),
        ("comment_digg_count".to_owned(), json!(digg(comment))),
        (
            "comment_create_time".to_owned(),
            comment.get("create_time").cloned().unwrap_or(Value::Null),
        ),
        (
            "comment_user".to_owned(),
            user_metadata(comment.get("user")),
        ),
        (
            "quality_score".to_owned(),
            json!(digg(comment) + reply.map(digg).unwrap_or(0)),
        ),
    ]);
    if let Some(reply) = reply {
        result.extend([
            ("reply_id".to_owned(), json!(first_string(reply, &["id"]))),
            ("reply_digg_count".to_owned(), json!(digg(reply))),
            (
                "reply_create_time".to_owned(),
                reply.get("create_time").cloned().unwrap_or(Value::Null),
            ),
            ("reply_user".to_owned(), user_metadata(reply.get("user"))),
        ]);
    }
    Value::Object(result)
}

fn user_metadata(user: Option<&Value>) -> Value {
    let object = user.and_then(Value::as_object);
    json!({"uid":object_string(object,"uid"),"sec_uid":object_string(object,"sec_uid"),"nickname":object_string(object,"nickname"),"unique_id":object_string(object,"unique_id")})
}

fn first_string(value: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
        .to_owned()
}

fn object_string(object: Option<&Map<String, Value>>, key: &str) -> String {
    object
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}

fn digg(value: &Value) -> i64 {
    value
        .get("digg_count")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()))
        .unwrap_or(0)
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_bool()
            .unwrap_or_else(|| value.as_i64().unwrap_or(0) != 0)
    })
}

fn reached_limit(length: usize, limit: usize) -> bool {
    limit > 0 && length >= limit
}

fn pause(has_more: bool, seconds: f64) {
    if has_more && seconds > 0.0 {
        thread::sleep(Duration::from_secs_f64(seconds));
    }
}

fn contains_verify_check(value: &Value) -> bool {
    match value {
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key == "verify_check" || contains_verify_check(value)),
        Value::Array(values) => values.iter().any(contains_verify_check),
        Value::String(value) => value == "verify_check",
        _ => false,
    }
}

fn write_output(text: &str, path: Option<&Path>) -> Result<(), String> {
    if let Some(path) = path {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, format!("{text}\n")).map_err(|error| error.to_string())
    } else {
        println!("{text}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommentArgs, OutputFormat, extract_aweme_id, format_chatml, normalize_comment, sign,
    };
    use serde_json::json;

    #[test]
    fn extracts_raw_and_url_aweme_ids() {
        assert_eq!(
            extract_aweme_id("7380000000000000000").unwrap(),
            "7380000000000000000"
        );
        assert_eq!(
            extract_aweme_id("https://www.douyin.com/video/7380000000000000000?x=1").unwrap(),
            "7380000000000000000"
        );
        assert_eq!(
            extract_aweme_id("https://www.douyin.com/note/7380000000000000000").unwrap(),
            "7380000000000000000"
        );
    }

    #[test]
    fn normalizes_comment_fields() {
        let value = normalize_comment(&json!({
            "cid":"1","text":"你好","create_time":1710000000,"digg_count":3,"reply_comment_total":2,"ip_label":"上海",
            "user":{"uid":"u1","sec_uid":"sec","nickname":"用户","unique_id":"unique"}
        }));
        assert_eq!(value["id"], "1");
        assert_eq!(value["user"]["nickname"], "用户");
        assert_eq!(value["digg_count"], 3);
    }

    #[test]
    fn bundled_signer_returns_a_bogus_value() {
        let value = sign(
            "sign_datail",
            "aweme_id=7380000000000000000&device_platform=webapp&aid=6383",
            super::DEFAULT_USER_AGENT,
        )
        .unwrap();
        assert!(value.ends_with('='));
        assert!(value.len() > 20);
    }

    #[test]
    fn chatml_pairs_comments_and_replies() {
        let args = CommentArgs {
            target: String::new(),
            limit: 100,
            count: 20,
            with_replies: true,
            reply_limit: 20,
            sleep_seconds: 0.0,
            output: None,
            output_format: OutputFormat::ChatmlJsonl,
            comment_role: "user".to_owned(),
            reply_role: "assistant".to_owned(),
            min_comment_digg: 0,
            min_reply_digg: 0,
            include_single_comments: false,
            cookie: None,
        };
        let records = format_chatml(
            &json!({
                "aweme_id":"7380000000000000000",
                "comments":[{"id":"c1","text":"这车能买吗？","digg_count":8,"user":{},"replies":[
                    {"id":"r1","text":"先查维保和事故。","digg_count":12,"user":{}}
                ]}]
            }),
            &args,
        );
        assert_eq!(records[0]["messages"][0]["role"], "user");
        assert_eq!(records[0]["messages"][1]["content"], "先查维保和事故。");
        assert_eq!(records[0]["metadata"]["quality_score"], 20);
    }
}
