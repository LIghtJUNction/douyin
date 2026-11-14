use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use crate::{
    crawler::{CrawlType, resolve_target},
    io,
    net::NativeClient,
    paging,
};

#[derive(Args)]
pub struct CommentArgs {
    /// 作品 ID、视频链接或图文链接
    target: String,
    #[arg(short, long, default_value_t = 100)]
    limit: usize,
    #[arg(long,default_value_t=20,value_parser=clap::value_parser!(u32).range(1..=20))]
    count: u32,
    #[arg(long)]
    with_replies: bool,
    #[arg(long, default_value_t = 20)]
    reply_limit: usize,
    #[arg(long="sleep",visible_alias="sleep-seconds",default_value="0.8",value_parser=sleep_seconds)]
    sleep_seconds: f64,
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[arg(long = "format", value_enum, default_value = "raw")]
    format: Format,
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
}
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Raw,
    ChatmlJson,
    ChatmlJsonl,
}

pub fn run(args: CommentArgs) -> Result<(), String> {
    if args.comment_role.trim().is_empty() || args.reply_role.trim().is_empty() {
        return Err("ChatML 角色不能为空".into());
    }
    let id = resolve_target(&args.target, CrawlType::Aweme)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("评论需要有效的数字作品 ID".into());
    }
    let client = NativeClient::saved()?;
    let delay = Duration::from_secs_f64(args.sleep_seconds);
    let mut requests = 0;
    let mut fetch = |path: &str, params: &[(String, String)]| {
        if requests > 0 && !delay.is_zero() {
            std::thread::sleep(delay);
        }
        requests += 1;
        let value = client.get(path, params)?;
        if let Some(comments) = value["comments"].as_array() {
            for comment in comments {
                if let Some(actual) = comment["aweme_id"].as_str()
                    && actual != id
                {
                    return Err("接口返回了其他作品的评论，停止采集".into());
                }
            }
        }
        Ok::<_, String>(value)
    };
    let mut data = paging::collect("0".into(), args.limit, "cid", |cursor, remaining| {
        let value = fetch(
            "/aweme/v2/comment/list/",
            &[
                ("aweme_id".into(), id.clone()),
                ("cursor".into(), cursor.into()),
                (
                    "count".into(),
                    remaining
                        .unwrap_or(args.count as usize)
                        .min(args.count as usize)
                        .to_string(),
                ),
            ],
        )?;
        paging::parse_page(&value, "comments", "cursor")
    })?;
    let mut comments = data["items"]
        .as_array()
        .cloned()
        .ok_or("评论结果缺少列表")?;
    for comment in &mut comments {
        normalize_id(comment)?;
        if args.with_replies && count(&comment["reply_comment_total"]) != Some(0) {
            let comment_id = comment["id"].as_str().ok_or("评论缺少 ID")?.to_owned();
            let replies =
                paging::collect("0".into(), args.reply_limit, "cid", |cursor, remaining| {
                    let value = fetch(
                        "/aweme/v1/comment/list/reply/",
                        &[
                            ("item_id".into(), id.clone()),
                            ("comment_id".into(), comment_id.clone()),
                            ("cursor".into(), cursor.into()),
                            (
                                "count".into(),
                                remaining
                                    .unwrap_or(args.count as usize)
                                    .min(args.count as usize)
                                    .to_string(),
                            ),
                        ],
                    )?;
                    paging::parse_page(&value, "comments", "cursor")
                })?;
            let mut rows = replies["items"]
                .as_array()
                .cloned()
                .ok_or("回复结果缺少列表")?;
            for reply in &mut rows {
                normalize_id(reply)?;
            }
            comment["replies"] = json!(rows);
            comment["replies_has_more"] = replies["has_more"].clone();
        } else {
            comment["replies"] = json!([]);
        }
    }
    data.as_object_mut().ok_or("评论结果无效")?.remove("items");
    data["aweme_id"] = json!(id);
    data["comments"] = json!(comments);
    let text = match args.format {
        Format::Raw => serde_json::to_string_pretty(&data).map_err(|_| "评论序列化失败")?,
        Format::ChatmlJson => {
            serde_json::to_string_pretty(&chatml(&data, &args)).map_err(|_| "ChatML 序列化失败")?
        }
        Format::ChatmlJsonl => chatml(&data, &args)
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "ChatML 序列化失败")?
            .join("\n"),
    };
    io::write_output(&text, args.output.as_deref())
}

fn normalize_id(comment: &mut Value) -> Result<(), String> {
    let id = comment["cid"]
        .as_str()
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .or_else(|| comment["cid"].as_u64().map(|v| v.to_string()))
        .ok_or("评论缺少稳定 ID")?;
    comment["id"] = json!(id);
    Ok(())
}
fn count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn digg(value: &Value) -> u64 {
    count(&value["digg_count"]).unwrap_or(0)
}
fn allowed(value: &Value, min: i64) -> bool {
    min <= 0 || digg(value) >= min as u64
}

fn chatml(data: &Value, args: &CommentArgs) -> Vec<Value> {
    let mut rows = Vec::new();
    for comment in data["comments"].as_array().into_iter().flatten() {
        let text = comment["text"].as_str().unwrap_or("").trim();
        if text.is_empty() || !allowed(comment, args.min_comment_digg) {
            continue;
        }
        let replies = comment["replies"].as_array();
        if replies.is_none_or(Vec::is_empty) && args.include_single_comments {
            rows.push(json!({"messages":[{"role":args.comment_role,"content":text}],"metadata":metadata(data,comment,None)}));
        } else {
            for reply in replies.into_iter().flatten() {
                let reply_text = reply["text"].as_str().unwrap_or("").trim();
                if reply_text.is_empty() || !allowed(reply, args.min_reply_digg) {
                    continue;
                }
                rows.push(json!({"messages":[{"role":args.comment_role,"content":text},{"role":args.reply_role,"content":reply_text}],"metadata":metadata(data,comment,Some(reply))}));
            }
        }
    }
    rows
}
fn metadata(data: &Value, comment: &Value, reply: Option<&Value>) -> Value {
    let mut result = json!({"source":if reply.is_some(){"douyin_comment_reply"}else{"douyin_comment"},"aweme_id":data["aweme_id"],"comment_id":comment["id"],
        "comment_digg_count":digg(comment),"comment_create_time":comment["create_time"],"comment_user":user_metadata(&comment["user"]),
        "quality_score":digg(comment).saturating_add(reply.map_or(0,digg))});
    if let Some(reply) = reply {
        result["reply_id"] = reply["id"].clone();
        result["reply_digg_count"] = json!(digg(reply));
        result["reply_create_time"] = reply["create_time"].clone();
        result["reply_user"] = user_metadata(&reply["user"]);
    }
    result
}
fn user_metadata(user: &Value) -> Value {
    json!({"uid":user["uid"].as_str().unwrap_or(""),"sec_uid":user["sec_uid"].as_str().unwrap_or(""),"nickname":user["nickname"].as_str().unwrap_or(""),"unique_id":user["unique_id"].as_str().unwrap_or("")})
}
fn sleep_seconds(value: &str) -> Result<f64, String> {
    let value = value.parse::<f64>().map_err(|_| "sleep 必须是秒数")?;
    if !value.is_finite() || !(0.0..=3600.0).contains(&value) {
        return Err("sleep 必须在 0 到 3600 秒之间".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> CommentArgs {
        CommentArgs {
            target: "1".into(),
            limit: 100,
            count: 20,
            with_replies: true,
            reply_limit: 20,
            sleep_seconds: 0.0,
            output: None,
            format: Format::ChatmlJson,
            comment_role: "user".into(),
            reply_role: "assistant".into(),
            min_comment_digg: 2,
            min_reply_digg: 3,
            include_single_comments: true,
        }
    }
    #[test]
    fn chatml_filters_scores_and_preserves_parent_reply_attribution() {
        let data = json!({"aweme_id":"1","comments":[{"id":"c1","text":"问题","digg_count":4,"user":{"uid":"u"},"replies":[{"id":"r1","text":"回答","digg_count":5},{"id":"r2","text":"低赞回复","digg_count":1}]},{"id":"c2","text":"独立评论","digg_count":2,"replies":[]},{"id":"c3","text":"低赞评论","digg_count":1,"replies":[]}]});
        let rows = chatml(&data, &args());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["messages"][1]["content"], "回答");
        assert_eq!(rows[0]["metadata"]["quality_score"], 9);
        assert_eq!(rows[0]["metadata"]["comment_user"]["uid"], "u");
        assert_eq!(rows[1]["messages"].as_array().unwrap().len(), 1);
    }
    #[test]
    fn rejects_unbounded_nonfinite_sleep_and_preserves_numeric_ids() {
        for value in ["NaN", "inf", "-1", "1e300"] {
            assert!(sleep_seconds(value).is_err());
        }
        let mut value = json!({"cid":123});
        normalize_id(&mut value).unwrap();
        assert_eq!(value["id"], "123");
    }
}
