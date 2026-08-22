use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use clap::{Args, ValueEnum};
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};

use crate::err;
use crate::net::{self, sign};
use crate::{fs_utils, settings};

const BASE_URL: &str = "https://www.douyin.com";
const COMMENT_LIST: &str = "/aweme/v1/web/comment/list/";
const COMMENT_REPLIES: &str = "/aweme/v1/web/comment/list/reply/";

#[derive(Debug, Args)]
pub struct CommentArgs {
    /// 作品 ID、视频 URL 或图文 URL
    target: String,
    /// 最多抓取一级评论数，0 表示不限制
    #[arg(short, long, default_value_t = 100)]
    limit: usize,
    /// 每页请求数量（抖音网页接口上限为 20）
    #[arg(long, default_value_t = 20, value_parser = parse_comment_count)]
    count: usize,
    /// 同时抓取评论楼中楼回复
    #[arg(long)]
    with_replies: bool,
    /// 每条评论最多抓取回复数，0 表示不限制
    #[arg(long, default_value_t = 20)]
    reply_limit: usize,
    /// 分页请求间隔秒数
    #[arg(long = "sleep", visible_alias = "sleep-seconds", default_value_t = 0.8, value_parser = parse_non_negative_f64)]
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
    let saved = settings::load().map_err(err)?;
    let (cookie_value, user_agent) = net::credentials(&saved, args.cookie.as_deref())?;
    let aweme_id = extract_aweme_id(&args.target)?;
    let crawler = CommentCrawler::new(&cookie_value, user_agent)?;
    let data = crawler.crawl(&aweme_id, &args)?;
    let output = match args.output_format {
        OutputFormat::Raw => serde_json::to_string_pretty(&data).map_err(err)?,
        OutputFormat::ChatmlJson => {
            serde_json::to_string_pretty(&format_chatml(&data, &args)).map_err(err)?
        }
        OutputFormat::ChatmlJsonl => format_chatml(&data, &args)
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(err)?
            .join("\n"),
    };
    fs_utils::write_output(&output, args.output.as_deref())?;
    if let Some(path) = args.output {
        eprintln!("评论已保存: {}", path.display());
    }
    Ok(())
}

struct CommentCrawler {
    client: Client,
    user_agent: String,
    common_params: Vec<(&'static str, String)>,
}

impl CommentCrawler {
    fn new(cookie: &str, user_agent: &str) -> Result<Self, String> {
        Ok(Self {
            client: net::web_client(cookie, user_agent, 30)?,
            user_agent: user_agent.to_owned(),
            common_params: net::web_query_params(cookie),
        })
    }

    fn crawl(&self, aweme_id: &str, args: &CommentArgs) -> Result<Value, String> {
        let comments = self.crawl_pages(
            COMMENT_LIST,
            vec![("aweme_id", aweme_id.to_owned())],
            args.limit,
            args,
            |raw| {
                let mut comment = normalize_comment(raw);
                if args.with_replies {
                    let comment_id = comment.get("id").and_then(Value::as_str).unwrap_or("");
                    comment["replies"] =
                        Value::Array(self.crawl_replies(aweme_id, comment_id, args)?);
                }
                Ok(comment)
            },
        )?;
        Ok(json!({"aweme_id": aweme_id, "comments": comments}))
    }

    fn crawl_replies(
        &self,
        aweme_id: &str,
        comment_id: &str,
        args: &CommentArgs,
    ) -> Result<Vec<Value>, String> {
        self.crawl_pages(
            COMMENT_REPLIES,
            vec![
                ("item_id", aweme_id.to_owned()),
                ("comment_id", comment_id.to_owned()),
            ],
            args.reply_limit,
            args,
            |raw| Ok(normalize_comment(raw)),
        )
    }

    /// Pages through a comment endpoint, applying `normalize` to every raw comment
    /// until the endpoint reports no more data or `limit` is reached.
    fn crawl_pages(
        &self,
        path: &str,
        base_params: Vec<(&'static str, String)>,
        limit: usize,
        args: &CommentArgs,
        mut normalize: impl FnMut(&Value) -> Result<Value, String>,
    ) -> Result<Vec<Value>, String> {
        let mut items = Vec::new();
        let mut cursor = 0_i64;
        let mut has_more = true;
        while has_more && !net::limit_reached(items.len(), limit) {
            let mut params = base_params.clone();
            params.extend([
                ("cursor", cursor.to_string()),
                ("count", args.count.to_string()),
                ("item_type", "0".to_owned()),
                ("insert_ids", String::new()),
                ("whale_cut_token", String::new()),
                ("cut_version", "1".to_owned()),
                ("rcFT", String::new()),
            ]);
            let page = self.fetch_page(path, params)?;
            let values = page
                .get("comments")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if values.is_empty() {
                break;
            }
            for raw in &values {
                items.push(normalize(raw)?);
                if net::limit_reached(items.len(), limit) {
                    break;
                }
            }
            let next_cursor = page
                .get("cursor")
                .and_then(net::value_i64)
                .unwrap_or(cursor);
            has_more = net::truthy(page.get("has_more"));
            if has_more && next_cursor == cursor {
                return Err("评论分页游标没有推进，已停止以避免重复请求".to_owned());
            }
            cursor = next_cursor;
            pause(has_more, args.sleep_seconds);
        }
        Ok(items)
    }

    fn fetch_page(&self, path: &str, mut params: Vec<(&str, String)>) -> Result<Value, String> {
        params.extend(self.common_params.clone());
        let query = net::encode_query(&params);
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
            .map_err(err)?;
        let status = response.status();
        let text = response.text().map_err(err)?;
        if !status.is_success() {
            return Err(format!("评论请求失败: {status} {text}"));
        }
        if text.is_empty() {
            return Err("响应体为空，Cookie 可能已失效".to_owned());
        }
        let data: Value =
            serde_json::from_str(&text).map_err(|error| format!("评论响应不是 JSON: {error}"))?;
        if net::contains_verify_check(&data) {
            return Err("触发验证码，请完成验证后再继续".to_owned());
        }
        if data
            .get("status_code")
            .and_then(net::value_i64)
            .unwrap_or(0)
            != 0
        {
            return Err(format!("评论接口返回失败状态: {text}"));
        }
        Ok(data)
    }
}

fn parse_comment_count(value: &str) -> Result<usize, String> {
    let value = value
        .parse::<usize>()
        .map_err(|error| format!("无效页大小: {error}"))?;
    if !(1..=20).contains(&value) {
        return Err("每页请求数量必须在 1..=20 范围内".to_owned());
    }
    Ok(value)
}

fn parse_non_negative_f64(value: &str) -> Result<f64, String> {
    let value = value
        .parse::<f64>()
        .map_err(|error| format!("无效秒数: {error}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err("秒数必须是有限的非负数".to_owned());
    }
    Ok(value)
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
            .map_err(err)?
            .get(url)
            .send()
            .map_err(err)?
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
            json!(digg(comment) + reply.map_or(0, digg)),
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

fn pause(has_more: bool, seconds: f64) {
    if has_more && seconds > 0.0 {
        thread::sleep(Duration::from_secs_f64(seconds));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommentArgs, OutputFormat, extract_aweme_id, format_chatml, normalize_comment,
        parse_comment_count, parse_non_negative_f64,
    };
    use crate::test_support::must;
    use serde_json::json;

    #[test]
    fn validates_page_size_and_sleep_values() {
        assert_eq!(must(parse_comment_count("20")), 20);
        assert!(parse_comment_count("0").is_err());
        assert!(parse_comment_count("21").is_err());
        assert_eq!(must(parse_non_negative_f64("0.5")), 0.5);
        assert!(parse_non_negative_f64("NaN").is_err());
        assert!(parse_non_negative_f64("-1").is_err());
    }

    #[test]
    fn extracts_raw_and_url_aweme_ids() {
        assert_eq!(
            must(extract_aweme_id("7380000000000000000")),
            "7380000000000000000"
        );
        assert_eq!(
            must(extract_aweme_id(
                "https://www.douyin.com/video/7380000000000000000?x=1"
            )),
            "7380000000000000000"
        );
        assert_eq!(
            must(extract_aweme_id(
                "https://www.douyin.com/note/7380000000000000000"
            )),
            "7380000000000000000"
        );
    }

    #[test]
    fn normalizes_comment_fields() {
        let value = normalize_comment(&json!({
            "cid":"1","text":"你好","create_time":1_710_000_000,"digg_count":3,"reply_comment_total":2,"ip_label":"上海",
            "user":{"uid":"u1","sec_uid":"sec","nickname":"用户","unique_id":"unique"}
        }));
        assert_eq!(value["id"], "1");
        assert_eq!(value["user"]["nickname"], "用户");
        assert_eq!(value["digg_count"], 3);
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
