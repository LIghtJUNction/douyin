//! Deterministic text counts and traceable demand signals; no network or model.
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use clap::Args;
use serde_json::{Value, json};

use crate::{io, stats::OutputFormat};

const DEMAND_SIGNALS: &[&str] = &[
    "想要",
    "需要",
    "求",
    "哪里",
    "怎么",
    "如何",
    "有没有",
    "能不能",
    "推荐",
    "多少钱",
    "价格",
    "链接",
    "教程",
    "怎么买",
    "在哪买",
    "同款",
    "问题",
    "希望",
    "建议",
    "支持",
];
const STOP: &[&str] = &[
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
const ENGLISH_STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "but", "by", "for", "from", "has", "have", "he",
    "her", "his", "i", "in", "is", "it", "me", "my", "not", "of", "on", "or", "our", "she", "so",
    "that", "the", "their", "them", "they", "this", "to", "was", "we", "were", "what", "when",
    "where", "which", "who", "why", "with", "you", "your",
];
const MEMES: &[&str] = &[
    "绝绝子",
    "笑死",
    "谁懂啊",
    "太真实",
    "这谁顶得住",
    "我哭死",
    "绷不住",
    "破防了",
    "yyds",
    "666",
];

#[derive(Debug, Args)]
pub struct InsightsArgs {
    /// JSON、JSONL、ChatML 或纯文本；- 读取标准输入
    input: String,
    #[arg(long, default_value_t = 20)]
    top: usize,
    #[arg(long, default_value_t = 2)]
    min_count: u64,
    #[arg(long, value_enum, default_value = "json")]
    format: OutputFormat,
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Default)]
struct Tally {
    count: u64,
    score: u64,
    signals: BTreeSet<&'static str>,
}
impl Tally {
    fn add(&mut self, weight: u64) {
        self.count = self.count.saturating_add(1);
        self.score = self.score.saturating_add(weight);
    }
}

pub fn run(args: InsightsArgs) -> Result<(), String> {
    let input = io::read_input(&args.input)?;
    let records = parse_records(&input)?;
    let result = analyze(&records, args.top, args.min_count);
    let rendered = match args.format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&result).map_err(|_| "分析结果序列化失败")?
        }
        OutputFormat::Markdown => render_markdown(&result),
    };
    io::write_output(&rendered, args.output.as_deref())
}

pub fn parse_records(input: &str) -> Result<Vec<TextRecord>, String> {
    let mut records = Vec::new();
    match io::json_documents(input) {
        Ok(documents) => {
            for value in &documents {
                extract(value, 1, &mut records);
            }
        }
        Err(error) if input.trim_start().starts_with(['{', '[', '"']) => return Err(error),
        Err(_) => {
            for line in input.lines() {
                push(&mut records, line, 1);
            }
        }
    }
    if records.is_empty() {
        return Err("输入没有可分析的文本记录".into());
    }
    Ok(records)
}

fn extract(value: &Value, inherited: u64, records: &mut Vec<TextRecord>) {
    match value {
        Value::String(text) => push(records, text, inherited),
        Value::Array(values) => {
            for child in values {
                extract(child, inherited, records);
            }
        }
        Value::Object(map) => {
            let weight = weight(value)
                .or_else(|| weight(&value["metadata"]))
                .unwrap_or(inherited);
            if let Some(messages) = value["messages"].as_array() {
                for message in messages {
                    if let Some(text) = message["content"].as_str() {
                        push(records, text, weight);
                    }
                }
            } else {
                for key in ["text", "content", "desc"] {
                    if let Some(text) = value[key].as_str() {
                        push(records, text, weight);
                    }
                }
                if let Some(tags) = value["text_extra"].as_array() {
                    for tag in tags {
                        if let Some(name) = tag["tag_name"]
                            .as_str()
                            .or_else(|| tag["hashtag_name"].as_str())
                        {
                            push(
                                records,
                                &format!("#{}", name.trim_start_matches('#')),
                                weight,
                            );
                        }
                    }
                }
                if let Some(tags) = map.get("tag") {
                    extract_tags(tags, weight, records);
                }
            }
            for (key, child) in map {
                if !matches!(
                    key.as_str(),
                    "messages" | "text" | "content" | "desc" | "text_extra" | "tag" | "metadata"
                ) && (child.is_object() || child.is_array())
                {
                    extract(child, weight, records);
                }
            }
        }
        _ => (),
    }
}

fn extract_tags(value: &Value, weight: u64, records: &mut Vec<TextRecord>) {
    match value {
        Value::String(text) => push(records, text, weight),
        Value::Array(tags) => {
            for tag in tags {
                extract_tags(tag, weight, records);
            }
        }
        Value::Object(_) => {
            for key in ["text", "name", "title", "tag_name"] {
                if let Some(text) = value[key].as_str() {
                    push(records, text, weight);
                }
            }
        }
        _ => (),
    }
}
fn weight(value: &Value) -> Option<u64> {
    ["weight", "quality_score", "digg_count", "score"]
        .iter()
        .find_map(|key| {
            value[*key]
                .as_u64()
                .or_else(|| value[*key].as_str()?.parse().ok())
        })
        .map(|n| n.saturating_add(1))
}
fn push(records: &mut Vec<TextRecord>, text: &str, weight: u64) {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !text.is_empty() {
        records.push(TextRecord::new(text, Some(weight.max(1))));
    }
}

pub fn analyze(records: &[TextRecord], top: usize, min_count: u64) -> Value {
    let mut words = BTreeMap::<String, Tally>::new();
    let mut memes = BTreeMap::<String, Tally>::new();
    let mut demands = BTreeMap::<String, Tally>::new();
    for record in records {
        let text = record.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let weight = record.weight.unwrap_or(1).max(1);
        for token in tokens(&text) {
            words.entry(token).or_default().add(weight);
        }
        let mut candidates = BTreeSet::new();
        for clause in text.split(|c: char| c.is_whitespace() || "，。！？,.!?；;：:".contains(c))
        {
            let length = clause.chars().count();
            if (2..=18).contains(&length)
                && (length <= 10
                    || MEMES.iter().any(|m| clause.to_lowercase().contains(m))
                    || clause.chars().any(emoji))
            {
                candidates.insert(clause.to_owned());
            }
            let emojis: String = clause.chars().filter(|c| emoji(*c)).collect();
            if !emojis.is_empty() {
                candidates.insert(emojis);
            }
        }
        for candidate in candidates {
            memes.entry(candidate).or_default().add(weight);
        }
        let signals: BTreeSet<_> = DEMAND_SIGNALS
            .iter()
            .copied()
            .filter(|signal| text.contains(signal))
            .collect();
        if !signals.is_empty() {
            let tally = demands.entry(text).or_default();
            tally.add(weight);
            tally.signals.extend(signals);
        }
    }
    json!({"input_count":records.len(),"hot_words":rank(words,top,min_count,false),
        "hot_memes":rank(memes,top,min_count,false),"demands":rank(demands,top,min_count,true)})
}

fn rank(values: BTreeMap<String, Tally>, top: usize, min_count: u64, signals: bool) -> Vec<Value> {
    let mut rows: Vec<_> = values
        .into_iter()
        .filter(|(_, t)| t.count >= min_count)
        .collect();
    rows.sort_by(|a, b| {
        b.1.score
            .cmp(&a.1.score)
            .then_with(|| b.1.count.cmp(&a.1.count))
            .then_with(|| a.0.cmp(&b.0))
    });
    rows.into_iter()
        .take(top)
        .map(|(text, t)| {
            let mut row = json!({"text":text,"count":t.count,"score":t.score});
            if signals {
                row["signals"] = json!(t.signals);
            }
            row
        })
        .collect()
}

fn tokens(text: &str) -> Vec<String> {
    let chars: Vec<_> = text.chars().collect();
    let mut result = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        if chars[index] == '#' {
            index += 1;
            while index < chars.len()
                && (han(chars[index])
                    || chars[index].is_ascii_alphanumeric()
                    || chars[index] == '_')
            {
                index += 1;
            }
            if index > start + 1 {
                result.push(chars[start..index].iter().collect());
            }
        } else if chars[index].is_ascii_alphanumeric() {
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
            if !ENGLISH_STOP.contains(&token.as_str()) {
                result.push(token);
            }
        } else if han(chars[index]) {
            index += 1;
            while index < chars.len() && han(chars[index]) {
                index += 1;
            }
            let segment: String = chars[start..index].iter().collect();
            let mut offset = 0;
            let mut chunk = String::new();
            while offset < segment.len() {
                let rest = &segment[offset..];
                if let Some(stop) = STOP
                    .iter()
                    .filter(|stop| rest.starts_with(**stop))
                    .max_by_key(|s| s.len())
                {
                    add_chunk(&chunk, &mut result);
                    chunk.clear();
                    offset += stop.len();
                } else if let Some(c) = rest.chars().next() {
                    chunk.push(c);
                    offset += c.len_utf8();
                }
            }
            add_chunk(&chunk, &mut result);
        } else {
            index += 1;
        }
    }
    result
}
fn add_chunk(chunk: &str, result: &mut Vec<String>) {
    let chars: Vec<_> = chunk.chars().collect();
    if (2..=8).contains(&chars.len()) {
        result.push(chunk.to_owned());
    }
    if chars.len() > 4 {
        for size in 2..=4 {
            for window in chars.windows(size) {
                let token: String = window.iter().collect();
                if !STOP.contains(&token.as_str()) {
                    result.push(token);
                }
            }
        }
    }
}
fn han(c: char) -> bool {
    matches!(c,'\u{3400}'..='\u{4dbf}'|'\u{4e00}'..='\u{9fff}'|'\u{f900}'..='\u{faff}'|'\u{20000}'..='\u{2fa1f}')
}
fn emoji(c: char) -> bool {
    matches!(c,'\u{1f000}'..='\u{1faff}'|'\u{2600}'..='\u{27bf}')
}
fn render_markdown(result: &Value) -> String {
    let mut output = format!("# 内容洞察\n\n输入记录：{}\n", result["input_count"]);
    for (title, key) in [
        ("热词", "hot_words"),
        ("热梗候选", "hot_memes"),
        ("需求线索", "demands"),
    ] {
        output.push_str(&format!("\n## {title}\n\n"));
        if let Some(items) = result[key].as_array() {
            if items.is_empty() {
                output.push_str("- 无\n");
            }
            for row in items {
                output.push_str(&format!(
                    "- {}（次数 {}，权重 {}）\n",
                    row["text"].as_str().unwrap_or(""),
                    row["count"],
                    row["score"]
                ));
            }
        }
    }
    output.push_str("\n结果来自词频与关键词规则，不代表语义理解或市场结论。\n");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_chatml_and_comment_replies_without_author_names() {
        let input = r#"{"comments":[{"text":"需要教程","digg_count":3,"user":{"nickname":"不应分析的作者"},"replies":[{"text":"求链接"}]}],"messages":[{"role":"user","content":"Rust rust"}],"metadata":{"weight":5}}"#;
        let rows = parse_records(input).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].text, "Rust rust");
        assert!(rows.iter().all(|r| !r.text.contains("作者")));
        assert_eq!(
            rows.iter().find(|r| r.text == "需要教程").unwrap().weight,
            Some(4)
        );
    }
    #[test]
    fn weighted_count_and_unicode_are_deterministic() {
        let rows = vec![
            TextRecord::new("Rust rust #测试", Some(4)),
            TextRecord::new("Rust #测试", Some(2)),
        ];
        let result = analyze(&rows, 20, 2);
        let words = result["hot_words"].as_array().unwrap();
        let rust = words.iter().find(|row| row["text"] == "rust").unwrap();
        assert_eq!(rust["count"], 3);
        assert_eq!(rust["score"], 10);
        assert!(words.iter().any(|row| row["text"] == "#测试"));
        assert_eq!(analyze(&rows, 0, 1)["hot_words"], json!([]));
    }
    #[test]
    fn plain_text_and_jsonl_work_but_malformed_json_is_not_counted_as_prose() {
        assert_eq!(parse_records("求教程\n求链接").unwrap().len(), 2);
        assert_eq!(
            parse_records("{\"text\":\"求教程\"}\n{\"text\":\"求链接\"}")
                .unwrap()
                .len(),
            2
        );
        assert!(parse_records("{\"text\": broken}").is_err());
        let result = analyze(&parse_records("求教程\n求教程").unwrap(), 20, 2);
        assert_eq!(result["demands"][0]["count"], 2);
    }
}
