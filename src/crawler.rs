use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, ValueEnum};
use percent_encoding::percent_decode_str;
use reqwest::blocking::Client;
use reqwest::header::{
    ACCEPT, ACCEPT_LANGUAGE, COOKIE, HeaderMap, HeaderValue, REFERER, USER_AGENT,
};
use serde_json::{Map, Value, json};

use crate::comments::{DEFAULT_USER_AGENT, sign};
use crate::{cookie, settings};

const BASE_URL: &str = "https://www.douyin.com";
const USER_ID_PREFIX: &str = "MS4wLjABAAAA";

#[derive(Debug, Args)]
pub struct CrawlArgs {
    /// 作品/账号/话题/音乐 URL、ID、搜索关键词或目标文件；可多次传入
    #[arg(short = 'u', long = "urls")]
    urls: Vec<String>,
    /// 限制最大采集数量，0 表示不限制
    #[arg(short, long, default_value_t = 0)]
    limit: usize,
    /// 不下载文件，仅采集数据
    #[arg(long)]
    no_download: bool,
    /// 采集类型
    #[arg(short = 't', long = "type", value_enum, default_value_t = CrawlType::Post)]
    crawl_type: CrawlType,
    /// 下载和数据输出根目录
    #[arg(short = 'p', long = "path", default_value_os_t = default_download_root())]
    output_path: PathBuf,
    /// 本次运行使用的 Cookie；默认读取保存的 Cookie
    #[arg(short, long, env = "DOUYIN_COOKIE")]
    cookie: Option<String>,
    /// 搜索排序：0=综合，1=最多点赞，2=最新
    #[arg(long, value_parser = ["0", "1", "2"])]
    sort_type: Option<String>,
    /// 发布时间：0=不限，1=一天内，7=一周内，180=半年内
    #[arg(long, value_parser = ["0", "1", "7", "180"])]
    publish_time: Option<String>,
    /// 视频时长：空=不限，0-1、1-5、5-10000
    #[arg(long, value_parser = ["", "0-1", "1-5", "5-10000"])]
    filter_duration: Option<String>,
    /// 为每个作品保存标题文本
    #[arg(long)]
    download_title: bool,
    /// 下载作品封面
    #[arg(long)]
    download_cover: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
enum CrawlType {
    Post,
    Favorite,
    Music,
    Hashtag,
    Search,
    Following,
    Follower,
    Collection,
    Mix,
    Aweme,
}

impl CrawlType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Post => "post",
            Self::Favorite => "favorite",
            Self::Music => "music",
            Self::Hashtag => "hashtag",
            Self::Search => "search",
            Self::Following => "following",
            Self::Follower => "follower",
            Self::Collection => "collection",
            Self::Mix => "mix",
            Self::Aweme => "aweme",
        }
    }

    fn is_user_list(self) -> bool {
        matches!(self, Self::Following | Self::Follower)
    }

    fn is_account_only(self) -> bool {
        matches!(
            self,
            Self::Favorite | Self::Collection | Self::Following | Self::Follower
        )
    }
}

impl CrawlArgs {
    pub fn should_run(&self) -> bool {
        !self.urls.is_empty()
            || self.limit != 0
            || self.no_download
            || self.crawl_type != CrawlType::Post
            || self.output_path != default_download_root()
            || self.sort_type.is_some()
            || self.publish_time.is_some()
            || self.filter_duration.is_some()
            || self.download_title
            || self.download_cover
    }
}

pub fn run(args: CrawlArgs) -> Result<(), String> {
    let settings_data = settings::load().map_err(|error| error.to_string())?;
    let cookie_value = args
        .cookie
        .clone()
        .or_else(|| {
            settings_data
                .get("cookie")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "未登录。请先运行: douyin auth cookie-login".to_owned())?;
    if !cookie::validate(&cookie_value) {
        return Err("Cookie 格式校验失败".to_owned());
    }
    let user_agent = settings_data
        .get("userAgent")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_USER_AGENT);
    let filename_fields = settings_data
        .get("filenameFields")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| vec!["id".to_owned(), "title".to_owned()]);
    let filename_separator = settings_data
        .get("filenameSeparator")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("_")
        .to_owned();
    let download_title = args.download_title
        || settings_data
            .get("enableDownloadTitle")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    let download_cover = args.download_cover
        || settings_data
            .get("enableDownloadCover")
            .and_then(Value::as_bool)
            .unwrap_or(false);

    let web = WebClient::new(&cookie_value, user_agent)?;
    let targets = resolve_targets(&args.urls, args.crawl_type)?;
    let mut successes = 0_usize;
    let mut failures = 0_usize;
    for target in targets {
        eprintln!(
            "开始采集：{} ({})",
            if target.is_empty() {
                "本账号"
            } else {
                &target
            },
            args.crawl_type.as_str()
        );
        match crawl_target(
            &web,
            &target,
            &args,
            &filename_fields,
            &filename_separator,
            download_title,
            download_cover,
        ) {
            Ok(count) => {
                successes += 1;
                eprintln!("采集完成：{count} 条结果");
            }
            Err(error) => {
                failures += 1;
                eprintln!("采集失败：{error}");
            }
        }
    }
    eprintln!("任务完成：成功 {successes} 个，失败 {failures} 个");
    if failures > 0 {
        Err(format!("{failures} 个采集任务失败"))
    } else {
        Ok(())
    }
}

fn resolve_targets(inputs: &[String], crawl_type: CrawlType) -> Result<Vec<String>, String> {
    if inputs.is_empty() {
        if crawl_type.is_account_only() {
            return Ok(vec![String::new()]);
        }
        eprint!(
            "采集类型 {}，请输入目标关键词/URL链接/ID或文件路径: ",
            crawl_type.as_str()
        );
        io::stderr().flush().map_err(|error| error.to_string())?;
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .map_err(|error| error.to_string())?;
        let input = input.trim();
        if input.is_empty() {
            return Err("未输入目标".to_owned());
        }
        return resolve_targets(&[input.to_owned()], crawl_type);
    }
    let mut targets = Vec::new();
    for input in inputs {
        let path = Path::new(input);
        if path.is_file() {
            let text = fs::read_to_string(path)
                .map_err(|error| format!("读取目标文件 {} 失败: {error}", path.display()))?;
            targets.extend(
                text.lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                    .map(str::to_owned),
            );
        } else {
            targets.push(input.trim().to_owned());
        }
    }
    if targets.is_empty() {
        Err("未找到可采集目标".to_owned())
    } else {
        Ok(targets)
    }
}

fn crawl_target(
    web: &WebClient,
    input: &str,
    args: &CrawlArgs,
    filename_fields: &[String],
    filename_separator: &str,
    download_title: bool,
    download_cover: bool,
) -> Result<usize, String> {
    let target = Target::parse(web, input, args.crawl_type)?;
    let title = web
        .target_title(&target)
        .unwrap_or_else(|| target.id.clone());
    let directory_name = sanitize_filename(&format!("{}_{}", target.kind.as_str(), title), 100);
    fs::create_dir_all(&args.output_path).map_err(|error| error.to_string())?;
    let data_stem = args.output_path.join(directory_name);
    let mut results = if target.kind == CrawlType::Aweme {
        let raw = web.fetch_json(
            "/aweme/v1/web/aweme/detail/",
            vec![("aweme_id".to_owned(), target.id.clone())],
            None,
        )?;
        let detail = raw.get("aweme_detail").cloned().unwrap_or(Value::Null);
        parse_aweme(&detail, target.kind).into_iter().collect()
    } else {
        crawl_pages(web, &target, args.limit, args)?
    };
    if target.kind == CrawlType::Post {
        merge_incremental(&mut results, &data_stem.with_extension("json"))?;
        results.sort_by(|left, right| string_field(right, "id").cmp(string_field(left, "id")));
    }
    save_json(
        &data_stem.with_extension("json"),
        &Value::Array(results.clone()),
    )?;
    let manifest_path = data_stem.with_extension("txt");
    let download_options = DownloadOptions {
        kind: target.kind,
        fields: filename_fields,
        separator: filename_separator,
        download_title,
        download_cover,
    };
    write_download_manifest(&results, &data_stem, &manifest_path, &download_options)?;
    if !args.no_download && !target.kind.is_user_list() {
        download_items(web, &results, &data_stem, &download_options)?;
    } else if args.no_download {
        eprintln!("已跳过下载（--no-download）");
    }
    Ok(results.len())
}

fn crawl_pages(
    web: &WebClient,
    target: &Target,
    limit: usize,
    args: &CrawlArgs,
) -> Result<Vec<Value>, String> {
    let mut cursor = 0_i64;
    let mut log_id = String::new();
    let mut has_more = true;
    let mut results = Vec::new();
    let mut retries = 0_u8;
    while has_more && !limit_reached(results.len(), limit) {
        let request = list_request(target, cursor, &log_id, args)?;
        let response = match web.fetch_json(request.path, request.params, request.form) {
            Ok(value) => {
                retries = 0;
                value
            }
            Err(error) if retries < 9 => {
                retries += 1;
                eprintln!("采集请求失败，重试 {retries}/10：{error}");
                continue;
            }
            Err(error) => return Err(error),
        };
        cursor = ["max_cursor", "cursor", "min_time"]
            .into_iter()
            .find_map(|key| {
                response
                    .get(key)
                    .and_then(value_i64)
                    .filter(|value| *value != 0)
            })
            .unwrap_or(cursor);
        if log_id.is_empty() {
            log_id = response
                .pointer("/log_pb/impr_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned();
        }
        let items = ["aweme_list", "user_list", "data", "followings", "followers"]
            .into_iter()
            .find_map(|key| {
                response
                    .get(key)
                    .and_then(Value::as_array)
                    .filter(|values| !values.is_empty())
            })
            .cloned()
            .unwrap_or_default();
        has_more = truthy(response.get("has_more"));
        if items.is_empty() {
            if has_more && retries < 9 {
                retries += 1;
                continue;
            }
            break;
        }
        for raw in items {
            let item = raw
                .get(if target.kind.is_user_list() {
                    "user_info"
                } else {
                    "aweme_info"
                })
                .unwrap_or(&raw);
            let parsed = if target.kind.is_user_list() {
                Some(parse_user(item))
            } else {
                parse_aweme(item, target.kind)
            };
            if let Some(parsed) = parsed {
                results.push(parsed);
            }
            if limit_reached(results.len(), limit) {
                has_more = false;
                break;
            }
        }
        eprintln!("采集中，已采集到 {} 条结果", results.len());
    }
    Ok(results)
}

struct ListRequest {
    path: &'static str,
    params: Vec<(String, String)>,
    form: Option<Vec<(String, String)>>,
}

fn list_request(
    target: &Target,
    cursor: i64,
    log_id: &str,
    args: &CrawlArgs,
) -> Result<ListRequest, String> {
    let count = "18".to_owned();
    let value = match target.kind {
        CrawlType::Post => ListRequest {
            path: "/aweme/v1/web/aweme/post/",
            params: pairs([
                ("publish_video_strategy_type", "2"),
                ("max_cursor", &cursor.to_string()),
                ("locate_query", "false"),
                ("show_live_replay_strategy", "1"),
                ("need_time_list", "0"),
                ("time_list_query", "0"),
                ("whale_cut_token", ""),
                ("count", &count),
                ("sec_user_id", &target.id),
            ]),
            form: None,
        },
        CrawlType::Favorite => ListRequest {
            path: "/aweme/v1/web/aweme/favorite/",
            params: pairs([
                ("sec_user_id", &target.id),
                ("max_cursor", &cursor.to_string()),
                ("min_cursor", "0"),
                ("whale_cut_token", ""),
                ("cut_version", "1"),
                ("count", &count),
                ("publish_video_strategy_type", "2"),
            ]),
            form: None,
        },
        CrawlType::Collection => ListRequest {
            path: "/aweme/v1/web/aweme/listcollection/",
            params: pairs([
                ("sec_user_id", &target.id),
                ("publish_video_strategy_type", "2"),
            ]),
            form: Some(pairs([("cursor", &cursor.to_string()), ("count", &count)])),
        },
        CrawlType::Music => ListRequest {
            path: "/aweme/v1/web/music/aweme/",
            params: pairs([
                ("cursor", &cursor.to_string()),
                ("count", &count),
                ("music_id", &target.id),
            ]),
            form: None,
        },
        CrawlType::Hashtag => ListRequest {
            path: "/aweme/v1/web/challenge/aweme/",
            params: pairs([
                ("cursor", &cursor.to_string()),
                ("count", &count),
                ("sort_type", "1"),
                ("ch_id", &target.id),
            ]),
            form: None,
        },
        CrawlType::Mix => ListRequest {
            path: "/aweme/v1/web/mix/aweme/",
            params: pairs([
                ("cursor", &cursor.to_string()),
                ("count", &count),
                ("mix_id", &target.id),
            ]),
            form: None,
        },
        CrawlType::Search => {
            let filters = json!({
                "sort_type": args.sort_type.as_deref().unwrap_or("0"),
                "publish_time": args.publish_time.as_deref().unwrap_or("0"),
                "content_type": "1",
                "filter_duration": args.filter_duration.as_deref().unwrap_or("0"),
                "search_range": "0"
            });
            ListRequest {
                path: "/aweme/v1/web/general/search/single/",
                params: vec![
                    ("search_channel".to_owned(), "aweme_general".to_owned()),
                    ("enable_history".to_owned(), "1".to_owned()),
                    ("filter_selected".to_owned(), filters.to_string()),
                    ("keyword".to_owned(), target.id.clone()),
                    ("search_source".to_owned(), "tab_search".to_owned()),
                    ("query_correct_type".to_owned(), "1".to_owned()),
                    ("is_filter_search".to_owned(), "1".to_owned()),
                    ("from_group_id".to_owned(), String::new()),
                    ("disable_rs".to_owned(), "0".to_owned()),
                    ("offset".to_owned(), cursor.to_string()),
                    ("count".to_owned(), count),
                    ("need_filter_settings".to_owned(), "0".to_owned()),
                    ("list_type".to_owned(), "multi".to_owned()),
                    ("search_id".to_owned(), log_id.to_owned()),
                ],
                form: None,
            }
        }
        CrawlType::Following => ListRequest {
            path: "/aweme/v1/web/user/following/list/",
            params: pairs([
                ("sec_user_id", &target.id),
                ("offset", "0"),
                ("min_time", "0"),
                ("max_time", &cursor.to_string()),
                ("count", "20"),
                ("gps_access", "0"),
                ("is_top", "1"),
            ]),
            form: None,
        },
        CrawlType::Follower => ListRequest {
            path: "/aweme/v1/web/user/follower/list/",
            params: pairs([
                ("sec_user_id", &target.id),
                ("offset", "0"),
                ("min_time", "0"),
                ("max_time", &cursor.to_string()),
                ("count", "20"),
                ("gps_access", "0"),
                ("is_top", "1"),
                ("source_type", "3"),
            ]),
            form: None,
        },
        CrawlType::Aweme => return Err("aweme 类型不使用列表接口".to_owned()),
    };
    Ok(value)
}

fn pairs<const N: usize>(items: [(&str, &str); N]) -> Vec<(String, String)> {
    items
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

struct Target {
    id: String,
    url: String,
    kind: CrawlType,
}

impl Target {
    fn parse(web: &WebClient, input: &str, requested: CrawlType) -> Result<Self, String> {
        if input.is_empty() {
            let id = web.self_uid()?;
            return Ok(Self {
                id,
                url: format!("{BASE_URL}/user/self"),
                kind: requested,
            });
        }
        if let Ok(mut url) = reqwest::Url::parse(input) {
            if !url
                .host_str()
                .is_some_and(|host| host.ends_with("douyin.com"))
            {
                return Err(format!("目标不是抖音链接: {input}"));
            }
            if url.host_str() == Some("v.douyin.com") {
                url = web.redirect_url(url)?;
            }
            let parts: Vec<_> = url
                .path_segments()
                .into_iter()
                .flatten()
                .filter(|part| !part.is_empty())
                .collect();
            let id = percent_decode_str(parts.last().copied().unwrap_or(""))
                .decode_utf8_lossy()
                .into_owned();
            let marker = parts.iter().rev().nth(1).copied().unwrap_or("");
            let kind = match marker {
                "video" | "note" => CrawlType::Aweme,
                "music" => CrawlType::Music,
                "hashtag" => CrawlType::Hashtag,
                "collection" => CrawlType::Mix,
                "search" => CrawlType::Search,
                _ => requested,
            };
            if id.is_empty() {
                return Err(format!("无法从链接识别目标 ID: {input}"));
            }
            return Ok(Self {
                id,
                url: url.into(),
                kind,
            });
        }
        let valid = match requested {
            CrawlType::Search => true,
            CrawlType::Aweme | CrawlType::Music | CrawlType::Hashtag | CrawlType::Mix => {
                input.chars().all(|value| value.is_ascii_digit())
            }
            _ => input.starts_with(USER_ID_PREFIX),
        };
        if !valid {
            return Err(format!("目标输入错误: {input}"));
        }
        let url = match requested {
            CrawlType::Search => format!("{BASE_URL}/search/{input}"),
            CrawlType::Aweme => format!("{BASE_URL}/note/{input}"),
            CrawlType::Mix => format!("{BASE_URL}/collection/{input}"),
            CrawlType::Music => format!("{BASE_URL}/music/{input}"),
            CrawlType::Hashtag => format!("{BASE_URL}/hashtag/{input}"),
            _ => format!("{BASE_URL}/user/{input}"),
        };
        Ok(Self {
            id: input.to_owned(),
            url,
            kind: requested,
        })
    }
}

struct WebClient {
    client: Client,
    user_agent: String,
}

impl WebClient {
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
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            user_agent: user_agent.to_owned(),
        })
    }

    fn fetch_json(
        &self,
        path: &str,
        mut params: Vec<(String, String)>,
        form: Option<Vec<(String, String)>>,
    ) -> Result<Value, String> {
        params.extend([
            ("device_platform".to_owned(), "webapp".to_owned()),
            ("aid".to_owned(), "6383".to_owned()),
            ("channel".to_owned(), "channel_pc_web".to_owned()),
        ]);
        if matches!(
            path,
            "/aweme/v1/web/aweme/detail/"
                | "/aweme/v1/web/music/aweme/"
                | "/aweme/v1/web/user/follower/list/"
        ) {
            let query = encode_query(&params);
            params.push((
                "a_bogus".to_owned(),
                sign("sign_datail", &query, &self.user_agent)?,
            ));
        }
        let request = if let Some(form) = form {
            self.client
                .post(format!("{BASE_URL}{path}"))
                .query(&params)
                .form(&form)
        } else {
            self.client.get(format!("{BASE_URL}{path}")).query(&params)
        };
        let response = request.send().map_err(|error| error.to_string())?;
        let status = response.status();
        let text = response.text().map_err(|error| error.to_string())?;
        if !status.is_success() {
            return Err(format!("网页接口请求失败: {status} {text}"));
        }
        if text.is_empty() {
            return Err("响应体为空，Cookie 可能已失效".to_owned());
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| format!("网页接口响应不是 JSON: {error}"))?;
        if contains_verify_check(&value) {
            return Err("触发验证码，请在浏览器完成验证".to_owned());
        }
        if value.get("status_code").and_then(value_i64).unwrap_or(0) != 0 {
            return Err(format!("网页接口返回失败状态: {text}"));
        }
        Ok(value)
    }

    fn redirect_url(&self, url: reqwest::Url) -> Result<reqwest::Url, String> {
        self.client
            .get(url)
            .send()
            .map(|response| response.url().clone())
            .map_err(|error| error.to_string())
    }

    fn get_html(&self, url: &str) -> Result<String, String> {
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("HTML 请求失败: {}", response.status()));
        }
        response.text().map_err(|error| error.to_string())
    }

    fn self_uid(&self) -> Result<String, String> {
        let html = self.get_html(&format!("{BASE_URL}/user/self"))?;
        extract_escaped_value(&html, "secUid").ok_or_else(|| "无法从账号页面提取 secUid".to_owned())
    }

    fn target_title(&self, target: &Target) -> Option<String> {
        if target.kind == CrawlType::Search || target.kind == CrawlType::Aweme {
            return Some(target.id.clone());
        }
        let html = self.get_html(&target.url).ok()?;
        let key = match target.kind {
            CrawlType::Mix => "mixName",
            CrawlType::Music => "title",
            CrawlType::Hashtag => "chaName",
            _ => "nickname",
        };
        extract_escaped_value(&html, key).map(|value| sanitize_filename(&value, 100))
    }
}

fn encode_query(params: &[(String, String)]) -> String {
    params
        .iter()
        .map(|(key, value)| {
            let encoded: String = url::form_urlencoded::byte_serialize(value.as_bytes()).collect();
            format!("{key}={encoded}")
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn extract_escaped_value(text: &str, key: &str) -> Option<String> {
    for marker in [format!("{key}\\\":\\\""), format!("\"{key}\":\"")] {
        let Some(position) = text.find(&marker) else {
            continue;
        };
        let start = position + marker.len();
        let tail = &text[start..];
        let Some(end) = tail.find(if marker.contains("\\\"") {
            "\\\""
        } else {
            "\""
        }) else {
            continue;
        };
        let value = &tail[..end];
        if !value.is_empty() {
            return Some(value.replace("\\u002F", "/"));
        }
    }
    None
}

fn parse_aweme(item: &Value, crawl_type: CrawlType) -> Option<Value> {
    let kind = item
        .get("aweme_type")
        .or_else(|| item.get("awemeType"))
        .and_then(value_i64)?;
    let mut output = item
        .get("statistics")
        .or_else(|| item.get("stats"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for key in [
        "playCount",
        "downloadCount",
        "forwardCount",
        "collectCount",
        "digest",
        "exposure_count",
        "live_watch_count",
        "play_count",
        "download_count",
        "forward_count",
        "lose_count",
        "lose_comment_count",
    ] {
        output.remove(key);
    }
    let video = item.get("video").unwrap_or(&Value::Null);
    let download = if kind <= 66 || matches!(kind, 69 | 107) {
        last_url(video.pointer("/play_addr/url_list"))
            .or_else(|| last_url(item.pointer("/download/urlList")))
            .map(|value| Value::String(value.replace("watermark=1", "watermark=0")))?
    } else if kind == 68 {
        let values: Vec<_> = item
            .get("images")?
            .as_array()?
            .iter()
            .filter_map(|image| last_url(image.get("url_list").or_else(|| image.get("urlList"))))
            .map(Value::String)
            .collect();
        if values.is_empty() {
            return None;
        }
        Value::Array(values)
    } else {
        return None;
    };
    output.insert("download_addr".to_owned(), download);
    copy_alias(item, &mut output, "id", &["aweme_id", "awemeId"]);
    copy_alias(item, &mut output, "time", &["create_time", "createTime"]);
    output.insert("type".to_owned(), json!(kind));
    output.insert(
        "desc".to_owned(),
        json!(sanitize_filename(
            item.get("desc").and_then(Value::as_str).unwrap_or(""),
            100
        )),
    );
    output.insert(
        "duration".to_owned(),
        item.get("duration")
            .or_else(|| video.get("duration"))
            .cloned()
            .unwrap_or(Value::Null),
    );
    if let Some(music) = item.get("music") {
        output.insert(
            "music_title".to_owned(),
            json!(sanitize_filename(
                music.get("title").and_then(Value::as_str).unwrap_or(""),
                100
            )),
        );
        if let Some(uri) = music
            .pointer("/play_url/uri")
            .or_else(|| music.pointer("/playUrl/uri"))
        {
            output.insert("music_url".to_owned(), uri.clone());
        }
    }
    let cover = last_url(video.pointer("/cover/url_list"))
        .or_else(|| {
            video
                .get("dynamicCover")
                .and_then(Value::as_str)
                .map(|value| format!("https:{value}"))
        })
        .unwrap_or_default();
    output.insert("cover".to_owned(), json!(cover));
    if let Some(author) = item.get("author").or_else(|| item.get("authorInfo")) {
        output.insert(
            "author_avatar".to_owned(),
            json!(
                last_url(
                    author
                        .get("avatar_thumb")
                        .or_else(|| author.get("avatarThumb"))
                        .and_then(|value| value.get("url_list").or_else(|| value.get("urlList")))
                )
                .unwrap_or_default()
            ),
        );
        for (target, aliases) in [
            ("author_nickname", &["nickname"][..]),
            ("author_uid", &["sec_uid", "secUid"]),
            ("author_unique_id", &["unique_id", "uniqueId"]),
            ("author_short_id", &["short_id", "shortId"]),
        ] {
            copy_alias(author, &mut output, target, aliases);
        }
        output.insert(
            "author_signature".to_owned(),
            json!(sanitize_filename(
                author
                    .get("signature")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                100
            )),
        );
    }
    if let Some(tags) = item
        .get("text_extra")
        .or_else(|| item.get("textExtra"))
        .and_then(Value::as_array)
    {
        output.insert("text_extra".to_owned(), Value::Array(tags.iter().map(|tag| json!({
            "tag_id": tag.get("hashtag_id").or_else(|| tag.get("hashtagId")).cloned().unwrap_or(Value::Null),
            "tag_name": tag.get("hashtag_name").or_else(|| tag.get("hashtagName")).cloned().unwrap_or(Value::Null)
        })).collect()));
    }
    if crawl_type == CrawlType::Mix
        && let Some(number) = item.pointer("/mix_info/statis/current_episode")
    {
        output.insert("no".to_owned(), number.clone());
    }
    Some(Value::Object(output))
}

fn parse_user(item: &Value) -> Value {
    let mut output = Map::new();
    output.insert(
        "nickname".to_owned(),
        json!(sanitize_filename(
            item.get("nickname").and_then(Value::as_str).unwrap_or(""),
            100
        )),
    );
    output.insert(
        "signature".to_owned(),
        json!(sanitize_filename(
            item.get("signature").and_then(Value::as_str).unwrap_or(""),
            100
        )),
    );
    output.insert(
        "avatar".to_owned(),
        json!(
            item.pointer("/avatar_thumb/url_list/0")
                .and_then(Value::as_str)
                .unwrap_or("")
        ),
    );
    for key in [
        "sec_uid",
        "uid",
        "short_id",
        "unique_id",
        "unique_id_modify_time",
        "aweme_count",
        "favoriting_count",
        "follower_count",
        "following_count",
        "constellation",
        "create_time",
        "enterprise_verify_reason",
        "is_gov_media_vip",
        "live_status",
        "total_favorited",
        "share_qrcode_uri",
    ] {
        if let Some(value) = item.get(key).filter(|value| !value.is_null()) {
            output.insert(key.to_owned(), value.clone());
        }
    }
    if let Some(room_id) = item.get("room_id").filter(|value| !value.is_null()) {
        output.insert("live_room_id".to_owned(), room_id.clone());
        let id = room_id
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| room_id.to_string());
        output.insert(
            "live_room_url".to_owned(),
            json!([
                format!("http://pull-flv-f26.douyincdn.com/media/stream-{id}.flv"),
                format!("http://pull-hls-f26.douyincdn.com/media/stream-{id}.m3u8")
            ]),
        );
    }
    if item
        .pointer("/original_musician/music_count")
        .and_then(value_i64)
        .unwrap_or(0)
        > 0
    {
        output.insert(
            "original_musician".to_owned(),
            item["original_musician"].clone(),
        );
    }
    Value::Object(output)
}

fn merge_incremental(results: &mut Vec<Value>, path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let old: Value =
        serde_json::from_str(&fs::read_to_string(path).map_err(|error| error.to_string())?)
            .map_err(|error| format!("旧采集数据无效: {error}"))?;
    let old_values = old.as_array().cloned().unwrap_or_default();
    let old_ids: HashMap<_, _> = old_values
        .iter()
        .filter_map(|value| value.get("id").map(|id| (id.to_string(), ())))
        .collect();
    results.retain(|value| {
        value
            .get("id")
            .is_none_or(|id| !old_ids.contains_key(&id.to_string()))
    });
    results.extend(old_values);
    Ok(())
}

struct DownloadOptions<'a> {
    kind: CrawlType,
    fields: &'a [String],
    separator: &'a str,
    download_title: bool,
    download_cover: bool,
}

fn write_download_manifest(
    results: &[Value],
    data_stem: &Path,
    manifest: &Path,
    options: &DownloadOptions<'_>,
) -> Result<(), String> {
    let mut lines = String::new();
    if options.kind.is_user_list() {
        for value in results
            .iter()
            .filter_map(|value| value.get("sec_uid").and_then(Value::as_str))
        {
            lines.push_str(&format!("{BASE_URL}/user/{value}\n"));
        }
    } else {
        for item in results {
            let filename = item_filename(item, options.kind, options.fields, options.separator);
            let item_dir = item_directory(data_stem, item, options.kind, &filename);
            match item.get("download_addr") {
                Some(Value::Array(urls)) => {
                    for (index, url) in urls.iter().filter_map(Value::as_str).enumerate() {
                        lines.push_str(&format!(
                            "{url}\n dir={}\n out={}_{}.jpeg\n",
                            item_dir.display(),
                            string_field(item, "id"),
                            index + 1
                        ));
                    }
                }
                Some(Value::String(url)) => lines.push_str(&format!(
                    "{url}\n dir={}\n out={filename}.mp4\n",
                    data_stem.display()
                )),
                _ => {}
            }
            if options.download_cover
                && let Some(url) = item
                    .get("cover")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
            {
                lines.push_str(&format!(
                    "{url}\n dir={}\n out={}_cover.jpg\n",
                    item_dir.display(),
                    string_field(item, "id")
                ));
            }
            if options.download_title {
                write_title(item, &item_dir)?;
            }
        }
    }
    if !lines.is_empty() {
        fs::write(manifest, lines).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn download_items(
    web: &WebClient,
    results: &[Value],
    data_stem: &Path,
    options: &DownloadOptions<'_>,
) -> Result<(), String> {
    fs::create_dir_all(data_stem).map_err(|error| error.to_string())?;
    for item in results {
        let filename = item_filename(item, options.kind, options.fields, options.separator);
        let item_dir = item_directory(data_stem, item, options.kind, &filename);
        match item.get("download_addr") {
            Some(Value::Array(urls)) => {
                fs::create_dir_all(&item_dir).map_err(|error| error.to_string())?;
                for (index, url) in urls.iter().filter_map(Value::as_str).enumerate() {
                    download_file(
                        &web.client,
                        url,
                        &item_dir.join(format!("{}_{}.jpeg", string_field(item, "id"), index + 1)),
                    )?;
                }
            }
            Some(Value::String(url)) if url.starts_with("http") => {
                download_file(&web.client, url, &data_stem.join(format!("{filename}.mp4")))?;
            }
            _ => {}
        }
        if options.download_cover
            && let Some(url) = item
                .get("cover")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
        {
            fs::create_dir_all(&item_dir).map_err(|error| error.to_string())?;
            download_file(
                &web.client,
                url,
                &item_dir.join(format!("{}_cover.jpg", string_field(item, "id"))),
            )?;
        }
        if options.download_title {
            write_title(item, &item_dir)?;
        }
    }
    Ok(())
}

fn download_file(client: &Client, url: &str, path: &Path) -> Result<(), String> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    eprintln!("下载: {}", path.display());
    let mut response = client
        .get(url)
        .send()
        .map_err(|error| format!("下载 {url} 失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("下载 {url} 失败: {}", response.status()));
    }
    persist_download(&mut response, path)
}

fn persist_download(reader: &mut impl io::Read, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = path.with_extension(format!(
        "{}.part",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("download")
    ));
    let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
    io::copy(reader, &mut file).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())
}

fn write_title(item: &Value, directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    fs::write(
        directory.join(format!("{}_title.txt", string_field(item, "id"))),
        string_field(item, "desc"),
    )
    .map_err(|error| error.to_string())
}

fn item_directory(data_stem: &Path, item: &Value, kind: CrawlType, filename: &str) -> PathBuf {
    if item.get("download_addr").is_some_and(Value::is_array) {
        if kind == CrawlType::Aweme {
            data_stem.parent().unwrap_or(data_stem).join(filename)
        } else {
            data_stem.join(filename)
        }
    } else {
        data_stem.to_owned()
    }
}

fn item_filename(item: &Value, kind: CrawlType, fields: &[String], separator: &str) -> String {
    let mut parts = Vec::new();
    for field in fields {
        let value = match field.as_str() {
            "id" => string_field(item, "id").to_owned(),
            "title" => string_field(item, "desc").to_owned(),
            "author" => string_field(item, "author_nickname").to_owned(),
            "type" => {
                if item.get("type").and_then(value_i64) == Some(68) {
                    "图文".to_owned()
                } else {
                    "视频".to_owned()
                }
            }
            "duration" => item
                .get("duration")
                .and_then(value_i64)
                .map(|ms| format!("{:02}-{:02}", ms / 60_000, (ms / 1_000) % 60))
                .unwrap_or_default(),
            "music" => string_field(item, "music_title").to_owned(),
            "no" => item.get("no").map(value_text).unwrap_or_default(),
            _ => String::new(),
        };
        if !value.is_empty() {
            parts.push(value);
        }
    }
    let fallback = string_field(item, "id");
    let joined = parts.join(separator);
    let base = sanitize_filename(if joined.is_empty() { fallback } else { &joined }, 200);
    if kind == CrawlType::Mix {
        item.get("no")
            .map(|value| format!("第{}集{separator}{base}", value_text(value)))
            .unwrap_or(base)
    } else {
        base
    }
}

fn save_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    text.push('\n');
    fs::write(path, text).map_err(|error| error.to_string())
}

fn default_download_root() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Downloads")
        .join("douyin")
}

fn sanitize_filename(text: &str, max_bytes: usize) -> String {
    let filtered: String = text
        .trim()
        .chars()
        .filter(|value| {
            !matches!(value, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
                && !value.is_control()
        })
        .collect();
    let collapsed = filtered.split_whitespace().collect::<Vec<_>>().join(" ");
    let source = if collapsed.is_empty() {
        "无标题"
    } else {
        &collapsed
    };
    if source.len() <= max_bytes {
        return source.to_owned();
    }
    let mut end = max_bytes.saturating_sub(3).min(source.len());
    while !source.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", source[..end].trim())
}

fn copy_alias(source: &Value, target: &mut Map<String, Value>, key: &str, aliases: &[&str]) {
    if let Some(value) = aliases.iter().find_map(|alias| source.get(alias)).cloned() {
        target.insert(key.to_owned(), value);
    }
}

fn last_url(value: Option<&Value>) -> Option<String> {
    value?.as_array()?.last()?.as_str().map(str::to_owned)
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.parse().ok())
}

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn string_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_bool()
            .unwrap_or_else(|| value_i64(value).unwrap_or(0) != 0)
    })
}

fn limit_reached(length: usize, limit: usize) -> bool {
    limit > 0 && length >= limit
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use super::{
        CrawlType, Target, WebClient, extract_escaped_value, item_filename, parse_aweme,
        parse_user, persist_download, sanitize_filename,
    };
    use serde_json::json;

    #[test]
    fn parses_video_and_image_awemes() {
        let video = parse_aweme(&json!({
            "aweme_type":4,"aweme_id":"1","create_time":10,"desc":"标题",
            "statistics":{"digg_count":2},"video":{"play_addr":{"url_list":["https://video"]},"duration":12000},
            "author":{"nickname":"作者","sec_uid":"sec","avatar_thumb":{"url_list":["https://avatar"]}}
        }), CrawlType::Post).unwrap();
        assert_eq!(video["download_addr"], "https://video");
        assert_eq!(video["author_nickname"], "作者");
        let image = parse_aweme(&json!({
            "aweme_type":68,"aweme_id":"2","desc":"图集","images":[{"url_list":["https://image"]}]
        }), CrawlType::Aweme).unwrap();
        assert_eq!(image["download_addr"][0], "https://image");
    }

    #[test]
    fn parses_user_and_filename() {
        let user = parse_user(&json!({
            "nickname":"用户","signature":"签名","avatar_thumb":{"url_list":["https://avatar"]},"sec_uid":"sec"
        }));
        assert_eq!(user["sec_uid"], "sec");
        let item =
            json!({"id":"1","desc":"标题","author_nickname":"作者","duration":65000,"type":4});
        assert_eq!(
            item_filename(
                &item,
                CrawlType::Post,
                &["id".to_owned(), "title".to_owned()],
                "_"
            ),
            "1_标题"
        );
    }

    #[test]
    fn sanitizes_cross_platform_filenames_by_utf8_bytes() {
        assert_eq!(sanitize_filename(" a:/b*? ", 100), "ab");
        assert!(sanitize_filename("很长的中文标题", 10).len() <= 10);
    }

    #[test]
    fn target_auto_detects_and_decodes_search_urls() {
        let web = WebClient::new("sessionid=test", super::DEFAULT_USER_AGENT).unwrap();
        let target = Target::parse(
            &web,
            "https://www.douyin.com/search/%E4%BA%8C%E6%89%8B%E8%BD%A6",
            CrawlType::Post,
        )
        .unwrap();
        assert_eq!(target.kind, CrawlType::Search);
        assert_eq!(target.id, "二手车");
    }

    #[test]
    fn escaped_value_falls_back_to_plain_json() {
        assert_eq!(
            extract_escaped_value(r#"<script>{"nickname":"测试用户"}</script>"#, "nickname"),
            Some("测试用户".to_owned())
        );
        assert_eq!(
            extract_escaped_value(r#"nickname\":\"转义用户\""#, "nickname"),
            Some("转义用户".to_owned())
        );
    }

    #[test]
    fn native_downloader_atomically_writes_stream() {
        let directory =
            std::env::temp_dir().join(format!("douyin-rust-download-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("sample.bin");
        let mut body = Cursor::new(b"media");
        persist_download(&mut body, &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"media");
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }
}
