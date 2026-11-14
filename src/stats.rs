//! Offline statistics over both legacy flattened exports and native API JSON.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use serde_json::{Value, json};

use crate::io;

const METRICS: [&str; 4] = ["likes", "comments", "collects", "shares"];
const WEIGHTS: [f64; 4] = [0.35, 0.20, 0.20, 0.25];

#[derive(Debug, Args)]
pub struct StatsArgs {
    /// JSON、JSONL 文件，或 - 读取标准输入
    input: String,
    #[arg(long)]
    author: Option<String>,
    #[arg(long, value_enum, default_value = "score")]
    sort: SortMetric,
    #[arg(long, default_value_t = 10)]
    top: usize,
    #[arg(long, value_enum, default_value = "json")]
    format: OutputFormat,
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
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

struct Item {
    id: String,
    desc: String,
    author: String,
    uid: String,
    metrics: [Option<u64>; 4],
    duration: Option<u64>,
    time: Option<u64>,
    topics: BTreeSet<String>,
    score: f64,
}

impl Item {
    fn interactions(&self) -> u64 {
        self.metrics
            .iter()
            .flatten()
            .copied()
            .fold(0, u64::saturating_add)
    }
    fn sort_value(&self, sort: SortMetric) -> u64 {
        match sort {
            SortMetric::Score => 0,
            SortMetric::Interactions => self.interactions(),
            SortMetric::Likes => self.metrics[0].unwrap_or(0),
            SortMetric::Comments => self.metrics[1].unwrap_or(0),
            SortMetric::Collects => self.metrics[2].unwrap_or(0),
            SortMetric::Shares => self.metrics[3].unwrap_or(0),
            SortMetric::Duration => self.duration.unwrap_or(0),
            SortMetric::Latest => self.time.unwrap_or(0),
        }
    }
}

pub fn run(args: StatsArgs) -> Result<(), String> {
    let result = analyze_json(
        &io::read_input(&args.input)?,
        args.author.as_deref(),
        args.sort,
        args.top,
    )?;
    let rendered = match args.format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&result).map_err(|_| "统计结果序列化失败")?
        }
        OutputFormat::Markdown => render_markdown(&result),
    };
    io::write_output(&rendered, args.output.as_deref())
}

pub fn analyze_json(
    input: &str,
    author: Option<&str>,
    sort: SortMetric,
    top: usize,
) -> Result<Value, String> {
    let documents = io::json_documents(input)?;
    let mut items = Vec::new();
    for document in &documents {
        collect(document, &mut items);
    }
    let input_count = items.len();
    items.retain(|item| author.is_none_or(|name| item.author == name));
    if items.is_empty() {
        return Err("没有符合筛选条件的有效作品记录".into());
    }
    let maxima: [u64; 4] = std::array::from_fn(|i| {
        items
            .iter()
            .filter_map(|item| item.metrics[i])
            .max()
            .unwrap_or(0)
    });
    for item in &mut items {
        item.score = WEIGHTS
            .iter()
            .enumerate()
            .map(|(i, weight)| {
                if maxima[i] == 0 {
                    0.0
                } else {
                    weight * (item.metrics[i].unwrap_or(0) as f64).ln_1p()
                        / (maxima[i] as f64).ln_1p()
                }
            })
            .sum::<f64>()
            * 100.0;
    }
    items.sort_by(|a, b| {
        (if matches!(sort, SortMetric::Score) {
            b.score.total_cmp(&a.score)
        } else {
            b.sort_value(sort).cmp(&a.sort_value(sort))
        })
        .then_with(|| a.id.cmp(&b.id))
    });
    let mut coverage = serde_json::Map::new();
    let mut summary = serde_json::Map::new();
    for (i, name) in METRICS.iter().enumerate() {
        let values: Vec<_> = items.iter().filter_map(|item| item.metrics[i]).collect();
        coverage.insert((*name).into(), json!(values.len()));
        summary.insert((*name).into(), metric_summary(&values));
    }
    summary.insert(
        "interactions".into(),
        metric_summary(&items.iter().map(Item::interactions).collect::<Vec<_>>()),
    );
    for (name, values) in [
        (
            "duration_ms",
            items
                .iter()
                .filter_map(|item| item.duration)
                .collect::<Vec<_>>(),
        ),
        (
            "published_time",
            items.iter().filter_map(|item| item.time).collect(),
        ),
    ] {
        coverage.insert(name.into(), json!(values.len()));
        let mut range = metric_summary(&values);
        range["min"] = json!(values.iter().min());
        range["max"] = json!(values.iter().max());
        if name == "published_time" {
            range["earliest"] = range["min"].clone();
            range["latest"] = range["max"].clone();
        }
        summary.insert(name.into(), range);
    }
    let mut authors: BTreeMap<(&str, &str), Vec<&Item>> = BTreeMap::new();
    let mut topics: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    let mut buckets: BTreeMap<&str, Vec<&Item>> = BTreeMap::new();
    for item in &items {
        authors
            .entry((&item.uid, &item.author))
            .or_default()
            .push(item);
        for topic in &item.topics {
            topics.entry(topic).or_default().push(item);
        }
        let bucket = match item.duration {
            None => "unknown",
            Some(0..=59_999) => "<1m",
            Some(60_000..=299_999) => "1-5m",
            _ => ">=5m",
        };
        buckets.entry(bucket).or_default().push(item);
    }
    let author_stats: Vec<_> = authors
        .into_iter()
        .map(|((uid, name), group)| {
            let mut result = group_summary(&group);
            result["author_uid"] = json!(uid);
            result["author_nickname"] = json!(name);
            result
        })
        .collect();
    let topic_stats: Vec<_> = topics
        .into_iter()
        .map(|(name, group)| {
            let mut result = group_summary(&group);
            result["topic"] = json!(name);
            result
        })
        .collect();
    let duration_buckets: Vec<_> = buckets
        .into_iter()
        .map(|(name, group)| {
            let mut result = group_summary(&group);
            result["bucket"] = json!(name);
            result
        })
        .collect();
    let top_items:Vec<_> = items.iter().take(top).enumerate().map(|(i,item)|json!({
        "rank":i+1,"id":item.id,"desc":item.desc,"author_uid":item.uid,"author_nickname":item.author,
        "digg_count":item.metrics[0],"comment_count":item.metrics[1],"collect_count":item.metrics[2],
        "share_count":item.metrics[3],"interactions":item.interactions(),"score":item.score,
        "duration":item.duration,"time":item.time,"topics":item.topics,
    })).collect();
    Ok(
        json!({"input_count":input_count,"matched_count":items.len(),"filter":{"author":author},
        "sort":sort.to_possible_value().map(|v|v.get_name().to_owned()).unwrap_or_default(),
        "score_formula":"100 × (0.35×likes_norm + 0.20×comments_norm + 0.20×collects_norm + 0.25×shares_norm), norm=ln(1+x)/ln(1+max)",
        "metric_coverage":coverage,"summary":summary,"authors":author_stats,"topics":topic_stats,
        "duration_buckets":duration_buckets,"top_items":top_items,
        "limitations":"仅统计元数据，不分析画面和声音；缺少播放量，score 不是互动率。"}),
    )
}

fn collect(value: &Value, items: &mut Vec<Item>) {
    match value {
        Value::Array(values) => {
            for value in values {
                collect(value, items);
            }
        }
        Value::Object(_) => {
            if let Some(id) = string(value, &["id", "aweme_id", "awemeId"]) {
                let stats = value.get("statistics").unwrap_or(value);
                let metrics = [
                    numeric(stats, &["digg_count", "diggCount"]),
                    numeric(stats, &["comment_count", "commentCount"]),
                    numeric(stats, &["collect_count", "collectCount"]),
                    numeric(stats, &["share_count", "shareCount"]),
                ];
                let topics = value
                    .get("text_extra")
                    .or_else(|| value.get("textExtra"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|v| string(v, &["tag_name", "tagName", "hashtag_name"]))
                    .map(|v| v.trim_start_matches('#').to_owned())
                    .filter(|v| !v.is_empty())
                    .collect();
                items.push(Item {
                    id,
                    desc: string(value, &["desc", "title"]).unwrap_or_default(),
                    author: string(value, &["author_nickname", "authorNickname"])
                        .or_else(|| string(&value["author"], &["nickname"]))
                        .unwrap_or_default(),
                    uid: string(value, &["author_uid", "authorUid"])
                        .or_else(|| string(&value["author"], &["uid"]))
                        .unwrap_or_default(),
                    duration: numeric(value, &["duration"])
                        .or_else(|| numeric(&value["video"], &["duration"])),
                    time: numeric(value, &["time", "create_time", "createTime"]),
                    metrics,
                    topics,
                    score: 0.0,
                });
            } else {
                for key in ["items", "aweme_list", "data", "aweme_detail", "aweme_info"] {
                    if let Some(child) = value.get(key) {
                        collect(child, items);
                    }
                }
            }
        }
        _ => (),
    }
}

fn numeric(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        value[*key]
            .as_u64()
            .or_else(|| value[*key].as_str()?.parse().ok())
    })
}
fn string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value[*key]
            .as_str()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .or_else(|| value[*key].as_u64().map(|v| v.to_string()))
    })
}
fn metric_summary(values: &[u64]) -> Value {
    if values.is_empty() {
        return json!({"total":null,"average":null,"median":null});
    }
    let sum: u128 = values.iter().map(|v| u128::from(*v)).sum();
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let middle = sorted.len() / 2;
    let median = if sorted.len().is_multiple_of(2) {
        (sorted[middle - 1] as f64 + sorted[middle] as f64) / 2.0
    } else {
        sorted[middle] as f64
    };
    json!({"total":sum.min(u128::from(u64::MAX)) as u64,"average":sum as f64 / values.len() as f64,"median":median})
}
fn group_summary(items: &[&Item]) -> Value {
    let mut result = json!({"count":items.len(),"interactions":items.iter().map(|i|i.interactions()).fold(0,u64::saturating_add)});
    for (i, name) in METRICS.iter().enumerate() {
        result[*name] = json!(
            items
                .iter()
                .filter_map(|item| item.metrics[i])
                .fold(0, u64::saturating_add)
        );
    }
    result
}
fn render_markdown(result: &Value) -> String {
    let mut output = format!(
        "# 作品统计\n\n作品数：{}\n\n| ID | 作者 | 标题 | 综合分 | 互动数 |\n| --- | --- | --- | ---: | ---: |\n",
        result["matched_count"]
    );
    if let Some(items) = result["top_items"].as_array() {
        for item in items {
            let escape = |key: &str| {
                item[key]
                    .as_str()
                    .unwrap_or("")
                    .replace('|', "\\|")
                    .replace(['\n', '\r'], " ")
            };
            output.push_str(&format!(
                "| {} | {} | {} | {:.2} | {} |\n",
                escape("id"),
                escape("author_nickname"),
                escape("desc"),
                item["score"].as_f64().unwrap_or(0.0),
                item["interactions"]
            ));
        }
    }
    output.push_str("\n仅统计元数据，综合分不是互动率。\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_and_flattened_exports_produce_same_scores() {
        let old = r#"[{"id":"1","author_nickname":"作者","digg_count":10,"comment_count":5,"collect_count":1,"share_count":2}]"#;
        let native = r#"{"data":{"aweme_list":[{"aweme_id":"1","author":{"nickname":"作者"},"statistics":{"digg_count":10,"comment_count":5,"collect_count":1,"share_count":2}}]}}"#;
        let a = analyze_json(old, None, SortMetric::Score, 10).unwrap();
        let b = analyze_json(native, None, SortMetric::Score, 10).unwrap();
        assert_eq!(a["top_items"], b["top_items"]);
        assert_eq!(a["top_items"][0]["score"], 100.0);
    }
    #[test]
    fn missing_metrics_are_not_reported_as_observed_zeroes() {
        let r = analyze_json("{\"id\":\"1\"}", None, SortMetric::Score, 10).unwrap();
        assert_eq!(r["metric_coverage"]["likes"], 0);
        assert_eq!(r["summary"]["likes"]["total"], Value::Null);
        assert_eq!(r["top_items"][0]["score"], 0.0);
    }
    #[test]
    fn jsonl_filtering_ties_and_top_zero_work() {
        let input =
            "{\"id\":\"2\",\"author_nickname\":\"a\"}\n{\"id\":\"1\",\"author_nickname\":\"a\"}";
        let r = analyze_json(input, Some("a"), SortMetric::Score, 10).unwrap();
        assert_eq!(r["top_items"][0]["id"], "1");
        assert!(analyze_json(input, Some("b"), SortMetric::Score, 10).is_err());
        assert_eq!(
            analyze_json(input, None, SortMetric::Score, 0).unwrap()["top_items"],
            json!([])
        );
    }

    #[test]
    fn integer_ranking_does_not_lose_precision_above_f64_integer_range() {
        let input = r#"[{"id":"a","digg_count":18446744073709551614},{"id":"b","digg_count":18446744073709551615}]"#;
        let result = analyze_json(input, None, SortMetric::Likes, 10).unwrap();
        assert_eq!(result["top_items"][0]["id"], "b");
        assert_eq!(result["summary"]["likes"]["total"], u64::MAX);
    }
}
