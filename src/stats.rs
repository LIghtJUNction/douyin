//! Offline metadata statistics for crawler output.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use serde_json::{Map, Value, json};

use crate::fs_utils;

const SCORE_FORMULA: &str = "100 × (0.35×likes_norm + 0.20×comments_norm + 0.20×collects_norm + 0.25×shares_norm), norm=ln(1+x)/ln(1+max)";

/// CLI arguments for offline crawler metadata statistics.
#[derive(Debug, Args)]
pub struct StatsArgs {
    /// JSON 文件路径，或 - 从 stdin 读取
    input: String,
    /// 按 author_nickname 精确过滤
    #[arg(long)]
    author: Option<String>,
    /// Top 作品排序指标
    #[arg(long, value_enum, default_value_t = SortMetric::Score)]
    sort: SortMetric,
    /// Top 作品最多输出条数
    #[arg(long, default_value_t = 10)]
    top: usize,
    /// 输出格式
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
    /// 输出文件；不传则输出到 stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum SortMetric {
    Score,
    Interactions,
    Likes,
    Comments,
    Collects,
    Shares,
    Duration,
    Latest,
}

impl SortMetric {
    fn as_str(self) -> &'static str {
        match self {
            Self::Score => "score",
            Self::Interactions => "interactions",
            Self::Likes => "likes",
            Self::Comments => "comments",
            Self::Collects => "collects",
            Self::Shares => "shares",
            Self::Duration => "duration",
            Self::Latest => "latest",
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Markdown,
}

#[derive(Clone, Debug)]
struct Item {
    id: String,
    desc: String,
    author_nickname: String,
    author_uid: String,
    likes: Option<u64>,
    comments: Option<u64>,
    collects: Option<u64>,
    shares: Option<u64>,
    duration_ms: Option<u64>,
    publish_time: Option<u64>,
    topics: Vec<String>,
}

impl Item {
    fn interactions(&self) -> u64 {
        self.likes
            .unwrap_or(0)
            .saturating_add(self.comments.unwrap_or(0))
            .saturating_add(self.collects.unwrap_or(0))
            .saturating_add(self.shares.unwrap_or(0))
    }
}

#[derive(Clone, Debug)]
struct ScoredItem {
    item: Item,
    interactions: u64,
    score: f64,
}

#[derive(Default)]
struct GroupAggregate {
    count: u64,
    likes: u64,
    comments: u64,
    collects: u64,
    shares: u64,
    interactions: u64,
    interactions_sum: u128,
}

pub fn run(args: StatsArgs) -> Result<(), String> {
    let input = read_input(&args.input)?;
    let result = analyze_json(&input, args.author.as_deref(), args.sort, args.top)?;
    let rendered = match args.format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
        }
        OutputFormat::Markdown => render_markdown(&result),
    };
    write_output(&rendered, args.output.as_deref())
}

/// Analyze crawler JSON metadata without reading or interpreting media content.
pub fn analyze_json(
    input: &str,
    author: Option<&str>,
    sort: SortMetric,
    top: usize,
) -> Result<Value, String> {
    let value: Value =
        serde_json::from_str(input).map_err(|error| format!("输入不是合法 JSON: {error}"))?;
    let items = parse_items(&value);
    if items.is_empty() {
        return Err("输入中没有有效作品记录（作品需要 id 或 aweme_id）".to_owned());
    }
    let input_count = items.len();
    let matched: Vec<_> = items
        .into_iter()
        .filter(|item| author.is_none_or(|name| item.author_nickname == name))
        .collect();
    if matched.is_empty() {
        return Err(match author {
            Some(name) => format!("没有 author_nickname 精确匹配“{name}”的作品"),
            None => "输入中没有有效作品记录".to_owned(),
        });
    }

    let maxima = metric_maxima(&matched);
    let mut scored: Vec<_> = matched
        .into_iter()
        .map(|item| {
            let interactions = item.interactions();
            let score = score_item(&item, maxima);
            ScoredItem {
                item,
                interactions,
                score,
            }
        })
        .collect();
    sort_items(&mut scored, sort);

    Ok(json!({
        "input_count": input_count,
        "matched_count": scored.len(),
        "filter": {"author": author},
        "sort": sort.as_str(),
        "score_formula": SCORE_FORMULA,
        "metric_coverage": metric_coverage(&scored),
        "summary": summary(&scored),
        "duration_buckets": duration_buckets(&scored),
        "authors": author_stats(&scored),
        "topics": topic_stats(&scored),
        "top_items": scored.iter().take(top).enumerate().map(|(index, item)| {
            top_item_json(index + 1, item)
        }).collect::<Vec<_>>(),
        "limitations": "仅统计采集元数据；不分析媒体画面、声音、Hook、镜头、字幕。缺少播放量，因此 score 不是互动率。"
    }))
}

fn read_input(input: &str) -> Result<String, String> {
    if input == "-" {
        let mut text = String::new();
        io::stdin()
            .read_to_string(&mut text)
            .map_err(|error| error.to_string())?;
        Ok(text)
    } else {
        fs::read_to_string(input).map_err(|error| format!("无法读取 {input}: {error}"))
    }
}

fn write_output(text: &str, output: Option<&Path>) -> Result<(), String> {
    if let Some(path) = output {
        fs_utils::atomic_write(path, format!("{text}\n").as_bytes())
            .map_err(|error| error.to_string())
    } else {
        println!("{text}");
        Ok(())
    }
}

fn parse_items(value: &Value) -> Vec<Item> {
    let mut candidates = Vec::new();
    collect_candidates(value, &mut candidates);
    candidates.into_iter().filter_map(parse_item).collect()
}

fn collect_candidates<'a>(value: &'a Value, candidates: &mut Vec<&'a Value>) {
    match value {
        Value::Array(values) => candidates.extend(values),
        Value::Object(object) => {
            let mut found_container = false;
            for key in ["items", "aweme_list", "data"] {
                if let Some(values) = object.get(key).and_then(Value::as_array) {
                    candidates.extend(values);
                    found_container = true;
                }
            }
            if !found_container {
                candidates.push(value);
            }
        }
        _ => {}
    }
}

fn parse_item(value: &Value) -> Option<Item> {
    let object = value.as_object()?;
    let id = string_or_integer(object, &["id", "aweme_id", "awemeId"])?;
    if id.is_empty() {
        return None;
    }
    Some(Item {
        id,
        desc: string_value(object, &["desc"]).unwrap_or_default(),
        author_nickname: string_value(object, &["author_nickname", "authorNickname"])
            .unwrap_or_default(),
        author_uid: string_value(object, &["author_uid", "authorUid"]).unwrap_or_default(),
        likes: numeric_value(object, &["digg_count", "diggCount"]),
        comments: numeric_value(object, &["comment_count", "commentCount"]),
        collects: numeric_value(object, &["collect_count", "collectCount"]),
        shares: numeric_value(object, &["share_count", "shareCount"]),
        duration_ms: numeric_value(object, &["duration"]),
        publish_time: numeric_value(object, &["time", "create_time", "createTime"]),
        topics: topics(object.get("text_extra").or_else(|| object.get("textExtra"))),
    })
}

fn string_or_integer(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = object.get(*key)?;
        value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| value.as_u64().map(|value| value.to_string()))
    })
}

fn string_value(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn numeric_value(object: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(non_negative_integer))
}

fn non_negative_integer(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
}

fn topics(value: Option<&Value>) -> Vec<String> {
    let mut topics = BTreeSet::new();
    if let Some(values) = value.and_then(Value::as_array) {
        for value in values {
            if let Some(name) = value
                .get("tag_name")
                .or_else(|| value.get("tagName"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                topics.insert(name.trim_start_matches('#').to_owned());
            }
        }
    }
    topics.into_iter().collect()
}

#[derive(Clone, Copy)]
struct Maxima {
    likes: u64,
    comments: u64,
    collects: u64,
    shares: u64,
}

fn metric_maxima(items: &[Item]) -> Maxima {
    Maxima {
        likes: items
            .iter()
            .filter_map(|item| item.likes)
            .max()
            .unwrap_or(0),
        comments: items
            .iter()
            .filter_map(|item| item.comments)
            .max()
            .unwrap_or(0),
        collects: items
            .iter()
            .filter_map(|item| item.collects)
            .max()
            .unwrap_or(0),
        shares: items
            .iter()
            .filter_map(|item| item.shares)
            .max()
            .unwrap_or(0),
    }
}

fn score_item(item: &Item, maxima: Maxima) -> f64 {
    let weighted = 0.35 * normalized(item.likes.unwrap_or(0), maxima.likes)
        + 0.20 * normalized(item.comments.unwrap_or(0), maxima.comments)
        + 0.20 * normalized(item.collects.unwrap_or(0), maxima.collects)
        + 0.25 * normalized(item.shares.unwrap_or(0), maxima.shares);
    round_two(weighted * 100.0).clamp(0.0, 100.0)
}

fn normalized(value: u64, maximum: u64) -> f64 {
    if maximum == 0 {
        0.0
    } else {
        (value as f64).ln_1p() / (maximum as f64).ln_1p()
    }
}

fn round_two(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

fn sort_items(items: &mut [ScoredItem], sort: SortMetric) {
    items.sort_by(|left, right| {
        primary_order(left, right, sort)
            .then_with(|| right.interactions.cmp(&left.interactions))
            .then_with(|| left.item.id.cmp(&right.item.id))
    });
}

fn primary_order(left: &ScoredItem, right: &ScoredItem, sort: SortMetric) -> Ordering {
    match sort {
        SortMetric::Score => right.score.total_cmp(&left.score),
        SortMetric::Interactions => right.interactions.cmp(&left.interactions),
        SortMetric::Likes => right
            .item
            .likes
            .unwrap_or(0)
            .cmp(&left.item.likes.unwrap_or(0)),
        SortMetric::Comments => right
            .item
            .comments
            .unwrap_or(0)
            .cmp(&left.item.comments.unwrap_or(0)),
        SortMetric::Collects => right
            .item
            .collects
            .unwrap_or(0)
            .cmp(&left.item.collects.unwrap_or(0)),
        SortMetric::Shares => right
            .item
            .shares
            .unwrap_or(0)
            .cmp(&left.item.shares.unwrap_or(0)),
        SortMetric::Duration => right
            .item
            .duration_ms
            .unwrap_or(0)
            .cmp(&left.item.duration_ms.unwrap_or(0)),
        SortMetric::Latest => right
            .item
            .publish_time
            .unwrap_or(0)
            .cmp(&left.item.publish_time.unwrap_or(0)),
    }
}

fn metric_coverage(items: &[ScoredItem]) -> Value {
    json!({
        "likes": items.iter().filter(|item| item.item.likes.is_some()).count(),
        "comments": items.iter().filter(|item| item.item.comments.is_some()).count(),
        "collects": items.iter().filter(|item| item.item.collects.is_some()).count(),
        "shares": items.iter().filter(|item| item.item.shares.is_some()).count(),
        "duration_ms": items.iter().filter(|item| item.item.duration_ms.is_some()).count(),
        "published_time": items.iter().filter(|item| item.item.publish_time.is_some()).count(),
    })
}

fn summary(items: &[ScoredItem]) -> Value {
    let likes: Vec<_> = items.iter().filter_map(|item| item.item.likes).collect();
    let comments: Vec<_> = items.iter().filter_map(|item| item.item.comments).collect();
    let collects: Vec<_> = items.iter().filter_map(|item| item.item.collects).collect();
    let shares: Vec<_> = items.iter().filter_map(|item| item.item.shares).collect();
    let interactions: Vec<_> = items.iter().map(|item| item.interactions).collect();
    let durations: Vec<_> = items
        .iter()
        .filter_map(|item| item.item.duration_ms)
        .collect();
    let published: Vec<_> = items
        .iter()
        .filter_map(|item| item.item.publish_time)
        .collect();
    json!({
        "likes": metric_summary(&likes),
        "comments": metric_summary(&comments),
        "collects": metric_summary(&collects),
        "shares": metric_summary(&shares),
        "interactions": metric_summary(&interactions),
        "duration_ms": range_summary(&durations),
        "published_time": {
            "earliest": published.iter().min(),
            "latest": published.iter().max(),
        }
    })
}

fn metric_summary(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({
            "total": Value::Null,
            "average": Value::Null,
            "median": Value::Null,
        });
    }
    let total = saturated_sum(values.iter().copied());
    json!({
        "total": total,
        "average": average_values(values),
        "median": median(values),
    })
}

fn range_summary(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({
            "average": Value::Null,
            "median": Value::Null,
            "min": Value::Null,
            "max": Value::Null,
        });
    }
    json!({
        "average": average_values(values),
        "median": median(values),
        "min": values.iter().min(),
        "max": values.iter().max(),
    })
}

fn saturated_sum(values: impl Iterator<Item = u64>) -> u64 {
    values.fold(0_u64, u64::saturating_add)
}

fn average_values(values: &[u64]) -> f64 {
    let total = values
        .iter()
        .fold(0_u128, |sum, value| sum.saturating_add(*value as u128));
    average_wide(total, values.len())
}

fn average_wide(total: u128, count: usize) -> f64 {
    if count == 0 {
        0.0
    } else {
        round_two(total as f64 / count as f64)
    }
}

fn median(values: &[u64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] as f64 + sorted[middle] as f64) / 2.0
    } else {
        sorted[middle] as f64
    }
}

fn duration_buckets(items: &[ScoredItem]) -> Value {
    let mut short = Vec::new();
    let mut medium = Vec::new();
    let mut long = Vec::new();
    for item in items {
        match item.item.duration_ms {
            Some(duration) if duration < 60_000 => short.push(item.interactions),
            Some(duration) if duration < 300_000 => medium.push(item.interactions),
            Some(_) => long.push(item.interactions),
            None => {}
        }
    }
    json!({
        "under_60s": bucket(&short),
        "60_to_300s": bucket(&medium),
        "over_300s": bucket(&long),
    })
}

fn bucket(interactions: &[u64]) -> Value {
    json!({
        "count": interactions.len(),
        "average_interactions": average_values(interactions),
    })
}

fn author_stats(items: &[ScoredItem]) -> Vec<Value> {
    let mut groups: BTreeMap<(String, String), GroupAggregate> = BTreeMap::new();
    for item in items {
        if item.item.author_nickname.is_empty() {
            continue;
        }
        let group = groups
            .entry((
                item.item.author_nickname.clone(),
                item.item.author_uid.clone(),
            ))
            .or_default();
        add_to_group(group, item);
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|left, right| {
        right
            .1
            .interactions
            .cmp(&left.1.interactions)
            .then_with(|| right.1.count.cmp(&left.1.count))
            .then_with(|| left.0.0.cmp(&right.0.0))
            .then_with(|| left.0.1.cmp(&right.0.1))
    });
    groups
        .into_iter()
        .map(|((author, author_uid), group)| {
            json!({
                "author_nickname": author,
                "author_uid": author_uid,
                "count": group.count,
                "total_likes": group.likes,
                "total_comments": group.comments,
                "total_collects": group.collects,
                "total_shares": group.shares,
                "total_interactions": group.interactions,
                "average_interactions": average_wide(group.interactions_sum, group.count as usize),
            })
        })
        .collect()
}

fn add_to_group(group: &mut GroupAggregate, item: &ScoredItem) {
    group.count = group.count.saturating_add(1);
    group.likes = group.likes.saturating_add(item.item.likes.unwrap_or(0));
    group.comments = group
        .comments
        .saturating_add(item.item.comments.unwrap_or(0));
    group.collects = group
        .collects
        .saturating_add(item.item.collects.unwrap_or(0));
    group.shares = group.shares.saturating_add(item.item.shares.unwrap_or(0));
    group.interactions = group.interactions.saturating_add(item.interactions);
    group.interactions_sum = group
        .interactions_sum
        .saturating_add(item.interactions as u128);
}

fn topic_stats(items: &[ScoredItem]) -> Vec<Value> {
    let mut groups: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for item in items {
        for topic in &item.item.topics {
            let group = groups.entry(topic.clone()).or_default();
            group.0 = group.0.saturating_add(1);
            group.1 = group.1.saturating_add(item.interactions);
        }
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|left, right| {
        right
            .1
            .0
            .cmp(&left.1.0)
            .then_with(|| right.1.1.cmp(&left.1.1))
            .then_with(|| left.0.cmp(&right.0))
    });
    groups
        .into_iter()
        .map(|(topic, (count, total_interactions))| {
            json!({
                "tag_name": topic,
                "count": count,
                "total_interactions": total_interactions,
            })
        })
        .collect()
}

fn top_item_json(rank: usize, item: &ScoredItem) -> Value {
    json!({
        "rank": rank,
        "id": item.item.id,
        "desc": item.item.desc,
        "author_nickname": item.item.author_nickname,
        "author_uid": item.item.author_uid,
        "likes": item.item.likes,
        "comments": item.item.comments,
        "collects": item.item.collects,
        "shares": item.item.shares,
        "interactions": item.interactions,
        "score": item.score,
        "duration_ms": item.item.duration_ms,
        "publish_time": item.item.publish_time,
        "share_url": format!("https://www.douyin.com/video/{}", item.item.id),
    })
}

fn render_markdown(result: &Value) -> String {
    let mut output = format!(
        "# 作品表现离线统计\n\n输入作品：{}；匹配作品：{}；排序：`{}`。\n\n> 仅统计采集元数据，不分析媒体画面、声音、Hook、镜头、字幕。缺少播放量，因此综合分不是互动率。\n\n",
        result["input_count"],
        result["matched_count"],
        result["sort"].as_str().unwrap_or("")
    );
    output.push_str("## 总体汇总\n\n| 指标 | 总计 | 平均 | 中位数 |\n|---|---:|---:|---:|\n");
    for (label, key) in [
        ("点赞", "likes"),
        ("评论", "comments"),
        ("收藏", "collects"),
        ("分享", "shares"),
        ("互动合计", "interactions"),
    ] {
        let metric = &result["summary"][key];
        output.push_str(&format!(
            "| {label} | {} | {} | {} |\n",
            display_json(&metric["total"]),
            display_json(&metric["average"]),
            display_json(&metric["median"])
        ));
    }
    let duration = &result["summary"]["duration_ms"];
    output.push_str(&format!(
        "\n时长（毫秒）：平均 {}，中位数 {}，最小 {}，最大 {}。\n\n发布时间：最早 {}，最晚 {}。\n",
        display_json(&duration["average"]),
        display_json(&duration["median"]),
        display_json(&duration["min"]),
        display_json(&duration["max"]),
        display_json(&result["summary"]["published_time"]["earliest"]),
        display_json(&result["summary"]["published_time"]["latest"])
    ));

    output.push_str("\n## 字段覆盖\n\n| 字段 | 有效记录数 |\n|---|---:|\n");
    for (label, key) in [
        ("likes", "likes"),
        ("comments", "comments"),
        ("collects", "collects"),
        ("shares", "shares"),
        ("duration_ms", "duration_ms"),
        ("published_time", "published_time"),
    ] {
        output.push_str(&format!(
            "| {label} | {} |\n",
            result["metric_coverage"][key]
        ));
    }

    output.push_str("\n## 时长分桶\n\n| 时长 | 作品数 | 平均互动 |\n|---|---:|---:|\n");
    for (label, key) in [
        ("不足 60 秒", "under_60s"),
        ("60–300 秒", "60_to_300s"),
        ("300 秒及以上", "over_300s"),
    ] {
        let bucket = &result["duration_buckets"][key];
        output.push_str(&format!(
            "| {label} | {} | {} |\n",
            bucket["count"], bucket["average_interactions"]
        ));
    }

    output.push_str(
        "\n## 作者\n\n| 作者 | UID | 作品数 | 总互动 | 平均互动 |\n|---|---|---:|---:|---:|\n",
    );
    append_rows(&mut output, &result["authors"], |row| {
        format!(
            "| {} | {} | {} | {} | {} |\n",
            escape_markdown(row["author_nickname"].as_str().unwrap_or("")),
            escape_markdown(row["author_uid"].as_str().unwrap_or("")),
            row["count"],
            row["total_interactions"],
            row["average_interactions"]
        )
    });

    output.push_str("\n## 话题\n\n| 话题 | 作品数 | 总互动 |\n|---|---:|---:|\n");
    append_rows(&mut output, &result["topics"], |row| {
        format!(
            "| #{} | {} | {} |\n",
            escape_markdown(row["tag_name"].as_str().unwrap_or("")),
            row["count"],
            row["total_interactions"]
        )
    });

    output.push_str(
        "\n## Top 作品\n\n| # | 作品 | 作者 | 互动 | 综合分 |\n|---:|---|---|---:|---:|\n",
    );
    append_rows(&mut output, &result["top_items"], |row| {
        format!(
            "| {} | [{}]({}) | {} | {} | {} |\n",
            row["rank"],
            escape_markdown(row["desc"].as_str().unwrap_or("")),
            row["share_url"].as_str().unwrap_or(""),
            escape_markdown(row["author_nickname"].as_str().unwrap_or("")),
            row["interactions"],
            row["score"]
        )
    });
    output
}

fn append_rows(output: &mut String, rows: &Value, render: impl Fn(&Value) -> String) {
    if let Some(rows) = rows.as_array() {
        for row in rows {
            output.push_str(&render(row));
        }
    }
}

fn display_json(value: &Value) -> String {
    if value.is_null() {
        "无".to_owned()
    } else {
        value.to_string()
    }
}

fn escape_markdown(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('|', "\\|")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}

#[cfg(test)]
mod tests {
    use super::{SortMetric, analyze_json, median, render_markdown};
    use serde_json::json;

    fn analyze(input: &str, author: Option<&str>, sort: SortMetric) -> serde_json::Value {
        analyze_json(input, author, sort, 20).unwrap()
    }

    #[test]
    fn parses_flat_schema_and_camel_case_aliases() {
        let result = analyze(
            r#"[
                {"id":"1","desc":"snake","author_nickname":"甲","author_uid":"u1","digg_count":1,"comment_count":2,"collect_count":3,"share_count":4,"duration":5000,"time":10},
                {"aweme_id":"2","desc":"camel","authorNickname":"乙","authorUid":"u2","diggCount":"5","commentCount":"6","collectCount":"7","shareCount":"8","duration":"9000","createTime":"20"},
                {"awemeId":"3","time":5}
            ]"#,
            None,
            SortMetric::Latest,
        );
        assert_eq!(result["input_count"], 3);
        assert_eq!(result["top_items"][0]["id"], "2");
        assert_eq!(result["top_items"][0]["interactions"], 26);
    }

    #[test]
    fn author_filter_is_exact() {
        let result = analyze(
            r#"[{"id":"1","author_nickname":"陈震同学"},{"id":"2","author_nickname":"陈震"}]"#,
            Some("陈震同学"),
            SortMetric::Score,
        );
        assert_eq!(result["matched_count"], 1);
        assert_eq!(result["top_items"][0]["id"], "1");
    }

    #[test]
    fn all_sort_metrics_select_expected_primary_value() {
        let input = r#"[
            {"id":"a","digg_count":9,"comment_count":1,"collect_count":1,"share_count":1,"duration":10,"time":10},
            {"id":"b","digg_count":1,"comment_count":9,"collect_count":2,"share_count":2,"duration":30,"time":30},
            {"id":"c","digg_count":2,"comment_count":2,"collect_count":9,"share_count":9,"duration":20,"time":20}
        ]"#;
        let expectations = [
            (SortMetric::Interactions, "c"),
            (SortMetric::Likes, "a"),
            (SortMetric::Comments, "b"),
            (SortMetric::Collects, "c"),
            (SortMetric::Shares, "c"),
            (SortMetric::Duration, "b"),
            (SortMetric::Latest, "b"),
        ];
        for (sort, expected) in expectations {
            let result = analyze(input, None, sort);
            assert_eq!(result["top_items"][0]["id"], expected);
        }
        let score = analyze(input, None, SortMetric::Score);
        assert_eq!(score["top_items"][0]["id"], "c");
    }

    #[test]
    fn stable_ties_use_interactions_then_id() {
        let result = analyze(
            r#"[
                {"id":"b","digg_count":5,"comment_count":5,"duration":10},
                {"id":"a","digg_count":5,"comment_count":5,"duration":10},
                {"id":"c","digg_count":4,"comment_count":4,"duration":10}
            ]"#,
            None,
            SortMetric::Duration,
        );
        assert_eq!(
            result["top_items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn score_is_one_hundred_for_all_maxima_and_zero_for_all_zero() {
        let maximum = analyze(
            r#"[{"id":"max","digg_count":10,"comment_count":10,"collect_count":10,"share_count":10}]"#,
            None,
            SortMetric::Score,
        );
        let zero = analyze(
            r#"[{"id":"zero","digg_count":0,"comment_count":0,"collect_count":0,"share_count":0}]"#,
            None,
            SortMetric::Score,
        );
        assert_eq!(maximum["top_items"][0]["score"], 100.0);
        assert_eq!(zero["top_items"][0]["score"], 0.0);
    }

    #[test]
    fn score_formula_applies_declared_weights() {
        let result = analyze(
            r#"[
                {"id":"likes","digg_count":10},
                {"id":"comments","comment_count":10},
                {"id":"collects","collect_count":10},
                {"id":"shares","share_count":10}
            ]"#,
            None,
            SortMetric::Score,
        );
        assert_eq!(result["top_items"][0]["score"], 35.0);
        assert_eq!(result["top_items"][1]["score"], 25.0);
        assert_eq!(result["top_items"][2]["score"], 20.0);
        assert_eq!(result["top_items"][3]["score"], 20.0);
    }

    #[test]
    fn median_handles_even_and_odd_inputs() {
        assert_eq!(median(&[9, 1, 5]), 5.0);
        assert_eq!(median(&[10, 2, 6, 4]), 5.0);
    }

    #[test]
    fn duration_bucket_boundaries_are_exact() {
        let result = analyze(
            r#"[
                {"id":"short","duration":59999},
                {"id":"medium","duration":60000},
                {"id":"long","duration":300000}
            ]"#,
            None,
            SortMetric::Duration,
        );
        assert_eq!(result["duration_buckets"]["under_60s"]["count"], 1);
        assert_eq!(result["duration_buckets"]["60_to_300s"]["count"], 1);
        assert_eq!(result["duration_buckets"]["over_300s"]["count"], 1);
    }

    #[test]
    fn missing_strings_and_negative_numbers_are_safe() {
        let result = analyze(
            r#"[
                {"id":"1","digg_count":"12","comment_count":-1,"collect_count":"bad","share_count":null},
                {"id":"2","digg_count":3}
            ]"#,
            None,
            SortMetric::Likes,
        );
        assert_eq!(result["metric_coverage"]["likes"], 2);
        assert_eq!(result["metric_coverage"]["comments"], 0);
        assert_eq!(result["top_items"][0]["interactions"], 12);
    }

    #[test]
    fn interactions_and_totals_saturate_on_overflow() {
        let result = analyze(
            r#"[{"id":"1","digg_count":"18446744073709551615","comment_count":"18446744073709551615","collect_count":1,"share_count":1}]"#,
            None,
            SortMetric::Interactions,
        );
        assert_eq!(result["top_items"][0]["interactions"], json!(u64::MAX));
        assert_eq!(result["summary"]["interactions"]["total"], json!(u64::MAX));
    }

    #[test]
    fn averages_do_not_use_saturated_totals() {
        let result = analyze(
            r#"[
                {"id":"1","author_nickname":"甲","author_uid":"u1","digg_count":"18446744073709551615","duration":1000},
                {"id":"2","author_nickname":"甲","author_uid":"u1","digg_count":"18446744073709551615","duration":1000}
            ]"#,
            None,
            SortMetric::Interactions,
        );
        let expected = u64::MAX as f64;
        assert_eq!(
            result["summary"]["likes"]["average"].as_f64().unwrap(),
            expected
        );
        assert_eq!(
            result["authors"][0]["average_interactions"]
                .as_f64()
                .unwrap(),
            expected
        );
        assert_eq!(
            result["duration_buckets"]["under_60s"]["average_interactions"]
                .as_f64()
                .unwrap(),
            expected
        );
    }

    #[test]
    fn missing_metrics_are_null_but_derived_interactions_are_zero() {
        let result = analyze(r#"[{"id":"1"}]"#, None, SortMetric::Score);
        for field in ["total", "average", "median"] {
            assert!(result["summary"]["likes"][field].is_null());
        }
        for field in ["average", "median", "min", "max"] {
            assert!(result["summary"]["duration_ms"][field].is_null());
        }
        assert_eq!(result["summary"]["interactions"]["total"], 0);
        assert_eq!(result["summary"]["interactions"]["average"], 0.0);
        assert_eq!(result["summary"]["interactions"]["median"], 0.0);

        let markdown = render_markdown(&result);
        assert!(markdown.contains("| 点赞 | 无 | 无 | 无 |"));
        assert!(markdown.contains("时长（毫秒）：平均 无，中位数 无，最小 无，最大 无。"));
        assert!(!markdown.contains("null"));
    }

    #[test]
    fn authors_and_topics_have_totals_and_deterministic_order() {
        let result = analyze(
            r#"[
                {"id":"1","author_nickname":"甲","digg_count":10,"text_extra":[{"tag_name":"汽车"},{"tag_name":"汽车"}]},
                {"id":"2","author_nickname":"乙","digg_count":5,"text_extra":[{"tag_name":"旅行"}]},
                {"id":"3","author_nickname":"甲","digg_count":1,"text_extra":[{"tag_name":"旅行"}]}
            ]"#,
            None,
            SortMetric::Score,
        );
        assert_eq!(result["authors"][0]["author_nickname"], "甲");
        assert_eq!(result["authors"][0]["count"], 2);
        assert_eq!(result["topics"][0]["tag_name"], "旅行");
        assert_eq!(result["topics"][0]["count"], 2);
    }

    #[test]
    fn authors_are_grouped_by_nickname_and_uid() {
        let result = analyze(
            r#"[
                {"id":"1","author_nickname":"同名","author_uid":"u1","digg_count":2},
                {"id":"2","author_nickname":"同名","author_uid":"u2","digg_count":1}
            ]"#,
            None,
            SortMetric::Interactions,
        );
        assert_eq!(result["authors"].as_array().unwrap().len(), 2);
        assert_eq!(result["authors"][0]["author_uid"], "u1");
        assert_eq!(result["authors"][1]["author_uid"], "u2");
    }

    #[test]
    fn nested_arrays_and_single_objects_are_supported() {
        for input in [
            r#"{"items":[{"id":"1"}]}"#,
            r#"{"aweme_list":[{"id":"1"}]}"#,
            r#"{"data":[{"id":"1"}]}"#,
            r#"{"id":"1"}"#,
        ] {
            let result = analyze(input, None, SortMetric::Score);
            assert_eq!(result["input_count"], 1);
        }
    }

    #[test]
    fn invalid_empty_and_unmatched_author_inputs_return_errors() {
        assert!(analyze_json("{", None, SortMetric::Score, 10).is_err());
        assert!(analyze_json(r#"{"items":[]}"#, None, SortMetric::Score, 10).is_err());
        assert!(
            analyze_json(
                r#"[{"id":"1","author_nickname":"甲"}]"#,
                Some("乙"),
                SortMetric::Score,
                10
            )
            .is_err()
        );
    }

    #[test]
    fn markdown_contains_required_sections_and_limitations() {
        let result = analyze(
            r#"[{"id":"1","desc":"作品","author_nickname":"甲","digg_count":1}]"#,
            None,
            SortMetric::Score,
        );
        let markdown = render_markdown(&result);
        for expected in [
            "总体汇总",
            "字段覆盖",
            "时长分桶",
            "作者",
            "话题",
            "Top 作品",
            "不是互动率",
        ] {
            assert!(markdown.contains(expected));
        }
    }

    #[test]
    fn markdown_escapes_link_and_table_control_characters() {
        let result = analyze(
            r#"[{"id":"1","desc":"正常](https://evil.example) | 下一列","author_nickname":"甲","author_uid":"u|1","digg_count":1}]"#,
            None,
            SortMetric::Score,
        );
        let markdown = render_markdown(&result);
        assert!(!markdown.contains("[正常](https://evil.example)"));
        assert!(markdown.contains(r"正常\](https://evil.example) \| 下一列"));
        assert!(markdown.contains(r"| 甲 | u\|1 | 1 | 1 | 1.0 |"));
    }
}
