//! Deterministic, offline text-frequency heuristics for content insights.

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use clap::{Args, ValueEnum};
use serde_json::{Map, Value, json};

use crate::fs_utils;

const DEFAULT_TOP: usize = 20;
const DEFAULT_MIN_COUNT: u64 = 2;
const STOP_WORDS: &[&str] = &[
    "一个",
    "一些",
    "不是",
    "不要",
    "不过",
    "为了",
    "为什么",
    "什么",
    "他们",
    "你们",
    "但是",
    "可以",
    "可能",
    "因为",
    "所以",
    "这个",
    "那个",
    "这些",
    "那些",
    "真的",
    "已经",
    "还是",
    "就是",
    "然后",
    "如果",
    "我们",
    "我的",
    "你的",
    "他的",
    "她的",
    "它的",
    "怎么",
    "怎样",
    "哪里",
    "有没有",
    "能不能",
    "需要",
    "想要",
    "推荐",
    "多少",
    "多少钱",
    "链接",
    "教程",
    "求",
    "的",
    "了",
    "呢",
    "吗",
    "啊",
    "呀",
    "吧",
    "也",
    "都",
    "和",
    "与",
    "或",
    "在",
    "是",
    "有",
    "就",
    "很",
    "太",
    "还",
    "又",
    "被",
    "把",
    "给",
    "到",
    "从",
    "对",
    "让",
    "要",
];
const ENGLISH_STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have", "he",
    "her", "his", "i", "in", "is", "it", "me", "my", "not", "of", "on", "or", "our", "she", "so",
    "that", "the", "their", "them", "they", "this", "to", "was", "we", "were", "what", "when",
    "where", "which", "who", "why", "with", "you", "your",
];
const DEMAND_SIGNALS: &[&str] = &[
    "想要",
    "求",
    "哪里",
    "怎么",
    "有没有",
    "能不能",
    "需要",
    "推荐",
    "多少钱",
    "链接",
    "教程",
    "怎么买",
    "在哪买",
    "同款",
    "价格",
    "求助",
    "如何",
    "问题",
    "希望",
    "建议",
    "支持",
];
const MEME_MARKERS: &[&str] = &[
    "绝绝子",
    "笑死",
    "谁懂啊",
    "太真实",
    "这谁顶得住",
    "我哭死",
    "绷不住",
    "破防了",
    "yyds",
    "YYDS",
    "666",
];

/// One input text and its optional interaction-derived weight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRecord {
    pub text: String,
    pub weight: Option<u64>,
}

impl TextRecord {
    pub fn new(text: impl Into<String>, weight: Option<u64>) -> Self {
        Self {
            text: text.into(),
            weight,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum OutputFormat {
    Json,
    Markdown,
}

/// Discover hot words, memes, and demand-like source lines from local text.
#[derive(Debug, Args)]
pub struct InsightsArgs {
    /// 输入文件路径，或 - 从 stdin 读取
    input: String,
    /// 每类结果最多输出条数
    #[arg(long, default_value_t = DEFAULT_TOP)]
    top: usize,
    /// 结果至少出现次数
    #[arg(long, default_value_t = DEFAULT_MIN_COUNT)]
    min_count: u64,
    /// 输出格式
    #[arg(long, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
    /// 输出文件；不传则输出到 stdout
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub fn run(args: InsightsArgs) -> Result<(), String> {
    let input = read_input(&args.input)?;
    let records = parse_records(&input);
    let result = analyze(&records, args.top, args.min_count);
    let rendered = match args.format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
        }
        OutputFormat::Markdown => render_markdown(&result),
    };
    write_output(&rendered, args.output.as_deref())
}

/// Analyze records without network access or semantic-model inference.
pub fn analyze(records: &[TextRecord], top: usize, min_count: u64) -> Value {
    let mut words: HashMap<String, Aggregate> = HashMap::new();
    let mut memes: HashMap<String, Aggregate> = HashMap::new();
    let mut demands: HashMap<String, DemandAggregate> = HashMap::new();

    for record in records {
        let text = normalize_space(&record.text);
        if text.is_empty() {
            continue;
        }
        let weight = record.weight.unwrap_or(1).max(1);
        for word in word_tokens(&text) {
            words.entry(word).or_default().add(weight);
        }
        for meme in meme_candidates(&text) {
            memes.entry(meme).or_default().add(weight);
        }
        let signals: Vec<_> = DEMAND_SIGNALS
            .iter()
            .filter(|signal| text.contains(**signal))
            .map(|signal| (*signal).to_owned())
            .collect();
        if !signals.is_empty() {
            demands
                .entry(text)
                .or_default()
                .add(weight, signals.into_iter());
        }
    }

    json!({
        "input_count": records.len(),
        "hot_words": ranked_aggregates(words, top, min_count),
        "hot_memes": ranked_aggregates(memes, top, min_count),
        "demands": ranked_demands(demands, top, min_count),
    })
}

/// Parse JSON, JSONL, or non-empty plain-text lines into weighted records.
pub fn parse_records(input: &str) -> Vec<TextRecord> {
    if let Ok(value) = serde_json::from_str::<Value>(input)
        && matches!(
            &value,
            Value::Object(_) | Value::Array(_) | Value::String(_)
        )
    {
        let mut records = Vec::new();
        extract_value(&value, 1, &mut records);
        if !records.is_empty() {
            return records;
        }
    }

    let lines: Vec<_> = input
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if !lines.is_empty() {
        let parsed: Result<Vec<Value>, _> = lines
            .iter()
            .map(|line| serde_json::from_str::<Value>(line))
            .collect();
        if let Ok(values) = parsed {
            let mut records = Vec::new();
            for value in values {
                let mut extracted = Vec::new();
                extract_value(&value, 1, &mut extracted);
                if extracted.is_empty() {
                    records.clear();
                    break;
                }
                records.extend(extracted);
            }
            if !records.is_empty() {
                return records;
            }
        }
    }

    lines
        .into_iter()
        .map(|line| TextRecord::new(line.trim(), None))
        .collect()
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

fn extract_value(value: &Value, inherited_weight: u64, records: &mut Vec<TextRecord>) {
    match value {
        Value::String(text) => push_record(records, text, inherited_weight),
        Value::Array(values) => {
            for value in values {
                extract_value(value, inherited_weight, records);
            }
        }
        Value::Object(object) => {
            let metadata_weight = object
                .get("metadata")
                .and_then(Value::as_object)
                .and_then(object_weight);
            let weight = object_weight(object)
                .or(metadata_weight)
                .unwrap_or(inherited_weight)
                .max(1);
            if let Some(messages) = object.get("messages").and_then(Value::as_array) {
                for message in messages {
                    if let Some(content) = message.get("content").and_then(Value::as_str) {
                        push_record(records, content, weight);
                    }
                }
            } else {
                for key in ["text", "desc", "content"] {
                    if let Some(text) = object.get(key).and_then(Value::as_str) {
                        push_record(records, text, weight);
                    }
                }
                if let Some(tag) = object.get("tag") {
                    extract_tag(tag, weight, records);
                }
                if let Some(text_extra) = object.get("text_extra") {
                    extract_text_extra(text_extra, weight, records);
                }
            }
            for (key, nested) in object {
                if matches!(
                    key.as_str(),
                    "messages"
                        | "text"
                        | "desc"
                        | "content"
                        | "tag"
                        | "text_extra"
                        | "weight"
                        | "digg_count"
                        | "quality_score"
                        | "score"
                ) {
                    continue;
                }
                if nested.is_array() || nested.is_object() {
                    extract_value(nested, weight, records);
                }
            }
        }
        _ => {}
    }
}

fn extract_text_extra(value: &Value, weight: u64, records: &mut Vec<TextRecord>) {
    match value {
        Value::Array(values) => {
            for value in values {
                extract_text_extra(value, weight, records);
            }
        }
        Value::Object(object) => {
            if let Some(tag_name) = object.get("tag_name").and_then(Value::as_str) {
                let tag_name = tag_name.trim();
                if !tag_name.is_empty() {
                    let topic = if tag_name.starts_with('#') {
                        tag_name.to_owned()
                    } else {
                        format!("#{tag_name}")
                    };
                    push_record(records, &topic, weight);
                }
            }
        }
        _ => {}
    }
}

fn extract_tag(value: &Value, weight: u64, records: &mut Vec<TextRecord>) {
    match value {
        Value::String(text) => push_record(records, text, weight),
        Value::Array(values) => {
            for value in values {
                extract_tag(value, weight, records);
            }
        }
        Value::Object(object) => {
            for key in ["text", "name", "title", "tag_name"] {
                if let Some(text) = object.get(key).and_then(Value::as_str) {
                    push_record(records, text, weight);
                }
            }
        }
        _ => {}
    }
}

fn push_record(records: &mut Vec<TextRecord>, text: &str, weight: u64) {
    let text = text.trim();
    if !text.is_empty() {
        records.push(TextRecord::new(text, Some(weight)));
    }
}

fn object_weight(object: &Map<String, Value>) -> Option<u64> {
    ["weight", "quality_score", "digg_count", "score"]
        .into_iter()
        .find_map(|key| numeric_value(object.get(key)?))
        .map(|value| value.saturating_add(1))
}

fn numeric_value(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|value| value.try_into().ok()))
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

#[derive(Default)]
struct Aggregate {
    count: u64,
    score: u64,
}

impl Aggregate {
    fn add(&mut self, weight: u64) {
        self.count = self.count.saturating_add(1);
        self.score = self.score.saturating_add(weight);
    }
}

#[derive(Default)]
struct DemandAggregate {
    count: u64,
    score: u64,
    signals: Vec<String>,
}

impl DemandAggregate {
    fn add(&mut self, weight: u64, signals: impl Iterator<Item = String>) {
        self.count = self.count.saturating_add(1);
        self.score = self.score.saturating_add(weight);
        for signal in signals {
            if !self.signals.contains(&signal) {
                self.signals.push(signal);
            }
        }
    }
}

fn ranked_aggregates(values: HashMap<String, Aggregate>, top: usize, min_count: u64) -> Vec<Value> {
    let mut values: Vec<_> = values
        .into_iter()
        .filter(|(_, value)| value.count >= min_count)
        .collect();
    values.sort_by(|left, right| {
        right
            .1
            .score
            .cmp(&left.1.score)
            .then_with(|| right.1.count.cmp(&left.1.count))
            .then_with(|| left.0.cmp(&right.0))
    });
    values
        .into_iter()
        .take(top)
        .map(|(text, value)| json!({"text":text, "count":value.count, "score":value.score}))
        .collect()
}

fn ranked_demands(
    values: HashMap<String, DemandAggregate>,
    top: usize,
    min_count: u64,
) -> Vec<Value> {
    let mut values: Vec<_> = values
        .into_iter()
        .filter(|(_, value)| value.count >= min_count)
        .collect();
    values.sort_by(|left, right| {
        right
            .1
            .score
            .cmp(&left.1.score)
            .then_with(|| right.1.count.cmp(&left.1.count))
            .then_with(|| left.0.cmp(&right.0))
    });
    values
        .into_iter()
        .take(top)
        .map(|(text, value)| {
            json!({"text":text, "count":value.count, "score":value.score, "signals":value.signals})
        })
        .collect()
}

fn word_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let chars: Vec<_> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '#' {
            let start = index;
            index += 1;
            while index < chars.len() && is_word_char(chars[index]) {
                index += 1;
            }
            if index > start + 1 {
                tokens.push(chars[start..index].iter().collect::<String>());
            }
        } else if chars[index].is_ascii_alphanumeric() {
            let start = index;
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '_' | '-' | '.'))
            {
                index += 1;
            }
            let token = chars[start..index]
                .iter()
                .collect::<String>()
                .to_ascii_lowercase();
            if !ENGLISH_STOP_WORDS.contains(&token.as_str()) {
                tokens.push(token);
            }
        } else if is_han(chars[index]) {
            let start = index;
            index += 1;
            while index < chars.len() && is_han(chars[index]) {
                index += 1;
            }
            let segment = chars[start..index].iter().collect::<String>();
            for chunk in split_han_chunks(&segment) {
                let length = chunk.chars().count();
                if (2..=8).contains(&length) && !STOP_WORDS.contains(&chunk.as_str()) {
                    tokens.push(chunk.clone());
                }
                if length > 4 {
                    let chunk_chars: Vec<_> = chunk.chars().collect();
                    for size in 2..=4 {
                        for window in chunk_chars.windows(size) {
                            let token = window.iter().collect::<String>();
                            if !STOP_WORDS.contains(&token.as_str()) {
                                tokens.push(token);
                            }
                        }
                    }
                }
            }
        } else {
            index += 1;
        }
    }
    tokens
}

fn split_han_chunks(segment: &str) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut offset = 0;
    while offset < segment.len() {
        let rest = &segment[offset..];
        let stop = STOP_WORDS
            .iter()
            .filter(|word| rest.starts_with(**word))
            .max_by_key(|word| word.len());
        if let Some(stop) = stop {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            offset += stop.len();
        } else if let Some(character) = rest.chars().next() {
            current.push(character);
            offset += character.len_utf8();
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn meme_candidates(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for clause in text.split(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '，' | '。' | '！' | '？' | ',' | '.' | '!' | '?' | '；' | ';' | '：' | ':'
            )
    }) {
        let clause = clause.trim();
        let length = clause.chars().count();
        if (2..=18).contains(&length)
            && (MEME_MARKERS.iter().any(|marker| clause.contains(marker))
                || clause.chars().any(is_emoji)
                || length <= 10)
        {
            candidates.push(clause.to_owned());
        }
        let emoji: String = clause
            .chars()
            .filter(|character| is_emoji(*character))
            .collect();
        if !emoji.is_empty() {
            candidates.push(emoji);
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn normalize_space(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_word_char(character: char) -> bool {
    is_han(character) || character.is_ascii_alphanumeric() || character == '_'
}

fn is_han(character: char) -> bool {
    matches!(character, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}')
}

fn is_emoji(character: char) -> bool {
    matches!(
        character,
        '\u{1f000}'..='\u{1faff}' | '\u{2600}'..='\u{26ff}' | '\u{2700}'..='\u{27bf}'
    )
}

fn render_markdown(result: &Value) -> String {
    let mut output = format!("# 内容洞察\n\n输入记录：{}\n", result["input_count"]);
    for (title, key) in [
        ("热词", "hot_words"),
        ("热梗", "hot_memes"),
        ("需求发现", "demands"),
    ] {
        output.push_str(&format!("\n## {title}\n\n"));
        let Some(items) = result[key].as_array() else {
            continue;
        };
        if items.is_empty() {
            output.push_str("- 无\n");
            continue;
        }
        for item in items {
            let signals = item["signals"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("、")
                })
                .filter(|value| !value.is_empty())
                .map(|value| format!("；信号：{value}"))
                .unwrap_or_default();
            output.push_str(&format!(
                "- {}（次数：{}，分数：{}{signals}）\n",
                item["text"].as_str().unwrap_or(""),
                item["count"],
                item["score"]
            ));
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{TextRecord, analyze, parse_records};
    use serde_json::json;

    #[test]
    fn hot_words_exclude_stop_words_and_keep_topic() {
        let result = analyze(
            &[
                TextRecord::new("这个露营灯真的好用 #露营装备", None),
                TextRecord::new("露营灯推荐 #露营装备", None),
            ],
            20,
            2,
        );
        assert!(
            result["hot_words"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "露营灯" && value["count"] == 2)
        );
        assert!(
            result["hot_words"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "#露营装备" && value["count"] == 2)
        );
        assert!(
            !result["hot_words"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "这个" || value["text"] == "真的")
        );
    }

    #[test]
    fn repeated_meme_requires_minimum_count() {
        let result = analyze(
            &[
                TextRecord::new("绝绝子 😂", None),
                TextRecord::new("绝绝子 😂", None),
                TextRecord::new("只出现一次", None),
            ],
            20,
            2,
        );
        assert!(
            result["hot_memes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "绝绝子" && value["count"] == 2)
        );
    }

    #[test]
    fn demands_include_signals_and_sort_by_interaction_weight() {
        let result = analyze(
            &[
                TextRecord::new("求购买链接", Some(3)),
                TextRecord::new("求购买链接", Some(3)),
                TextRecord::new("怎么安装教程", Some(10)),
                TextRecord::new("怎么安装教程", Some(10)),
            ],
            20,
            2,
        );
        assert_eq!(result["demands"][0]["text"], "怎么安装教程");
        assert_eq!(result["demands"][0]["signals"], json!(["怎么", "教程"]));
    }

    #[test]
    fn raw_comments_extract_replies_and_interaction_weights() {
        let records = parse_records(
            r#"{"comments":[{"text":"主评论","digg_count":8,"replies":[{"text":"回复","digg_count":3}]}]}"#,
        );
        assert_eq!(
            records,
            vec![
                TextRecord::new("主评论", Some(9)),
                TextRecord::new("回复", Some(4))
            ]
        );
    }

    #[test]
    fn jsonl_extracts_chatml_content() {
        let records = parse_records(
            "{\"messages\":[{\"role\":\"user\",\"content\":\"第一条\"}]}\n{\"messages\":[{\"role\":\"assistant\",\"content\":\"第二条\"}]}",
        );
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn crawler_text_extra_extracts_repeated_topic() {
        let records = parse_records(
            r##"[
                {"desc":"第一次露营","text_extra":[{"tag_name":"露营装备"}]},
                {"desc":"帐篷体验","text_extra":[{"tag_name":"#露营装备"}]}
            ]"##,
        );
        let result = analyze(&records, 20, 2);
        assert!(
            result["hot_words"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "#露营装备" && value["count"] == 2)
        );
    }

    #[test]
    fn chatml_metadata_quality_score_orders_demands() {
        let records = parse_records(
            r#"[
                {"messages":[{"role":"user","content":"求低分链接"}],"metadata":{"quality_score":1}},
                {"messages":[{"role":"user","content":"求低分链接"}],"metadata":{"quality_score":1}},
                {"messages":[{"role":"user","content":"求高分链接"}],"metadata":{"quality_score":20}},
                {"messages":[{"role":"user","content":"求高分链接"}],"metadata":{"quality_score":20}}
            ]"#,
        );
        let result = analyze(&records, 20, 2);
        assert_eq!(result["demands"][0]["text"], "求高分链接");
    }

    #[test]
    fn numeric_jsonl_falls_back_to_plain_text() {
        let records = parse_records("666\n666\n");
        let result = analyze(&records, 20, 2);
        assert_eq!(result["input_count"], 2);
        assert!(
            result["hot_memes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "666" && value["count"] == 2)
        );
    }

    #[test]
    fn single_numeric_json_falls_back_to_plain_text() {
        let records = parse_records("666");
        let result = analyze(&records, 20, 1);
        assert_eq!(result["input_count"], 1);
        assert!(
            result["hot_memes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["text"] == "666" && value["count"] == 1)
        );
    }

    #[test]
    fn plain_text_extracts_each_non_empty_line() {
        let records = parse_records("第一条\n\n第二条\n");
        assert_eq!(
            records,
            vec![
                TextRecord::new("第一条", None),
                TextRecord::new("第二条", None)
            ]
        );
    }
}
