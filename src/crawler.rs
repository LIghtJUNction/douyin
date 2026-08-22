use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use clap::{Args, ValueEnum};
use percent_encoding::percent_decode_str;
use reqwest::blocking::Client;
use reqwest::header::REFERER;
use serde_json::{Map, Value, json};

use crate::err;
use crate::net::{self, sign};
use crate::{fs_utils, settings};

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
    /// 发布时间：0=不限，1=一天内，7=一周内，182=半年内；180 为兼容别名
    #[arg(long, value_parser = ["0", "1", "7", "180", "182"])]
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
    let targets = resolve_targets(&args.urls, args.crawl_type)?;
    let settings_data = settings::load().map_err(err)?;
    let (cookie_value, user_agent) = net::credentials(&settings_data, args.cookie.as_deref())?;
    let filename_fields = settings_data
        .get("filenameFields")
        .and_then(Value::as_array)
        .map_or_else(
            || vec!["id".to_owned(), "title".to_owned()],
            |values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            },
        );
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
    if crawl_type == CrawlType::Collection && !inputs.is_empty() {
        return Err(
            "collection 仅支持当前 Cookie 登录账号的收藏夹，请不要传入 -u/--urls".to_owned(),
        );
    }
    if inputs.is_empty() {
        if crawl_type.is_account_only() {
            return Ok(vec![String::new()]);
        }
        eprint!(
            "采集类型 {}，请输入目标关键词/URL链接/ID或文件路径: ",
            crawl_type.as_str()
        );
        io::stderr().flush().map_err(err)?;
        let mut input = String::new();
        io::stdin().read_line(&mut input).map_err(err)?;
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
    fs::create_dir_all(&args.output_path).map_err(err)?;
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
    const MAX_ATTEMPTS: u8 = 3;

    let mut cursor = 0_i64;
    let mut search_id = String::new();
    let mut has_more = true;
    let mut results = Vec::new();
    while has_more && !net::limit_reached(results.len(), limit) {
        let request = list_request(target, cursor, &search_id, args)?;
        let mut response = None;
        for attempt in 1..=MAX_ATTEMPTS {
            match web.fetch_json(request.path, request.params.clone(), request.form.clone()) {
                Ok(value) => {
                    response = Some(value);
                    break;
                }
                Err(error) if attempt < MAX_ATTEMPTS => {
                    eprintln!("采集请求失败，{attempt}/{MAX_ATTEMPTS}：{error}");
                    thread::sleep(Duration::from_secs(u64::from(attempt)));
                }
                Err(error) => return Err(error),
            }
        }
        let response = response.ok_or_else(|| "采集请求未返回结果".to_owned())?;
        if target.kind == CrawlType::Search {
            response
                .pointer("/extra/logid")
                .or_else(|| response.pointer("/log_pb/impr_id"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .clone_into(&mut search_id);
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
        has_more = net::truthy(response.get("has_more"));
        if items.is_empty() {
            if has_more {
                return Err(
                    "接口返回 has_more 但没有数据，可能已触发风控；已停止以避免无限重试".to_owned(),
                );
            }
            break;
        }
        let next_cursor = ["max_cursor", "cursor", "min_time"]
            .into_iter()
            .find_map(|key| {
                response
                    .get(key)
                    .and_then(net::value_i64)
                    .filter(|value| *value != 0)
            })
            .unwrap_or_else(|| {
                if target.kind == CrawlType::Search {
                    cursor.saturating_add(i64::try_from(items.len()).unwrap_or(i64::MAX))
                } else {
                    cursor
                }
            });
        if has_more && next_cursor == cursor {
            return Err("分页游标没有推进，已停止以避免重复请求".to_owned());
        }
        cursor = next_cursor;
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
            if net::limit_reached(results.len(), limit) {
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
                ("need_time_list", "1"),
                ("time_list_query", "0"),
                ("whale_cut_token", ""),
                ("cut_version", "1"),
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
                ("publish_video_strategy_type", "2"),
                ("version_code", "170400"),
                ("version_name", "17.4.0"),
            ]),
            form: Some(pairs([("count", &count), ("cursor", &cursor.to_string())])),
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
            let sort_type = args.sort_type.as_deref().unwrap_or("0");
            let publish_time = match args.publish_time.as_deref().unwrap_or("0") {
                "180" => "182",
                value => value,
            };
            let filter_duration = args.filter_duration.as_deref().unwrap_or("0");
            let is_filtered = sort_type != "0" || publish_time != "0" || filter_duration != "0";
            let mut params = vec![
                ("search_channel".to_owned(), "aweme_general".to_owned()),
                ("enable_history".to_owned(), "1".to_owned()),
                ("keyword".to_owned(), target.id.clone()),
                ("search_source".to_owned(), "tab_search".to_owned()),
                ("query_correct_type".to_owned(), "1".to_owned()),
                (
                    "is_filter_search".to_owned(),
                    u8::from(is_filtered).to_string(),
                ),
                ("from_group_id".to_owned(), String::new()),
                ("disable_rs".to_owned(), "0".to_owned()),
                ("offset".to_owned(), cursor.to_string()),
                ("count".to_owned(), "10".to_owned()),
                ("need_filter_settings".to_owned(), "1".to_owned()),
                ("list_type".to_owned(), "multi".to_owned()),
                ("search_id".to_owned(), log_id.to_owned()),
            ];
            if is_filtered {
                params.push((
                    "filter_selected".to_owned(),
                    json!({
                        "sort_type": sort_type,
                        "publish_time": publish_time,
                        "content_type": "1",
                        "filter_duration": filter_duration,
                        "search_range": "0"
                    })
                    .to_string(),
                ));
            }
            ListRequest {
                path: "/aweme/v1/web/general/search/single/",
                params,
                form: None,
            }
        }
        CrawlType::Following => ListRequest {
            path: "/aweme/v1/web/user/following/list/",
            params: pairs([
                ("user_id", &target.id),
                ("sec_user_id", &target.id),
                ("offset", "0"),
                ("min_time", "0"),
                ("max_time", &cursor.to_string()),
                ("count", "20"),
                ("source_type", "1"),
                ("gps_access", "0"),
                ("address_book_access", "0"),
                ("min_change", "0"),
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
    common_params: Vec<(String, String)>,
}

impl WebClient {
    fn new(cookie: &str, user_agent: &str) -> Result<Self, String> {
        Ok(Self {
            client: net::web_client(cookie, user_agent, 60)?,
            user_agent: user_agent.to_owned(),
            common_params: net::web_query_params(cookie)
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        })
    }

    fn fetch_json(
        &self,
        path: &str,
        mut params: Vec<(String, String)>,
        form: Option<Vec<(String, String)>>,
    ) -> Result<Value, String> {
        for (key, value) in &self.common_params {
            if !params.iter().any(|(existing, _)| existing == key) {
                params.push((key.clone(), value.clone()));
            }
        }
        if path != "/aweme/v1/web/general/search/single/" {
            let query = net::encode_query(&params);
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
        let request = if path == "/aweme/v1/web/aweme/listcollection/" {
            request.header(
                REFERER,
                "https://www.douyin.com/user/self?showTab=favorite_collection",
            )
        } else {
            request
        };
        let response = request.send().map_err(err)?;
        let status = response.status();
        let text = response.text().map_err(err)?;
        if !status.is_success() {
            return Err(format!("网页接口请求失败: {status} {text}"));
        }
        if text.is_empty() {
            return Err("响应体为空，Cookie 可能已失效".to_owned());
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| format!("网页接口响应不是 JSON: {error}"))?;
        if net::contains_verify_check(&value) {
            return Err("触发验证码，请在浏览器完成验证".to_owned());
        }
        if value
            .get("status_code")
            .and_then(net::value_i64)
            .unwrap_or(0)
            != 0
        {
            return Err(format!("网页接口返回失败状态: {text}"));
        }
        Ok(value)
    }

    fn redirect_url(&self, url: reqwest::Url) -> Result<reqwest::Url, String> {
        self.client
            .get(url)
            .send()
            .map(|response| response.url().clone())
            .map_err(err)
    }

    fn get_html(&self, url: &str) -> Result<String, String> {
        let response = self.client.get(url).send().map_err(err)?;
        if !response.status().is_success() {
            return Err(format!("HTML 请求失败: {}", response.status()));
        }
        response.text().map_err(err)
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
        .and_then(net::value_i64)?;
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
        let id = value_text(room_id);
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
        .and_then(net::value_i64)
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
    let old: Value = serde_json::from_str(&fs::read_to_string(path).map_err(err)?)
        .map_err(|error| format!("旧采集数据无效: {error}"))?;
    let old_values = old.as_array().cloned().unwrap_or_default();
    let old_ids: HashSet<_> = old_values
        .iter()
        .filter_map(|value| value.get("id").map(ToString::to_string))
        .collect();
    results.retain(|value| {
        value
            .get("id")
            .is_none_or(|id| !old_ids.contains(&id.to_string()))
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
            let _ = writeln!(lines, "{BASE_URL}/user/{value}");
        }
    } else {
        for item in results {
            let filename = item_filename(item, options.kind, options.fields, options.separator);
            let item_dir = item_directory(data_stem, item, options.kind, &filename);
            match item.get("download_addr") {
                Some(Value::Array(urls)) => {
                    for (index, url) in urls.iter().filter_map(Value::as_str).enumerate() {
                        let _ = writeln!(
                            lines,
                            "{url}\n dir={}\n out={}_{}.jpeg",
                            item_dir.display(),
                            string_field(item, "id"),
                            index + 1
                        );
                    }
                }
                Some(Value::String(url)) => {
                    let _ = writeln!(
                        lines,
                        "{url}\n dir={}\n out={filename}.mp4",
                        data_stem.display()
                    );
                }
                _ => {}
            }
            if options.download_cover
                && let Some(url) = item
                    .get("cover")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
            {
                let _ = writeln!(
                    lines,
                    "{url}\n dir={}\n out={}_cover.jpg",
                    item_dir.display(),
                    string_field(item, "id")
                );
            }
            if options.download_title {
                write_title(item, &item_dir)?;
            }
        }
    }
    if !lines.is_empty() {
        fs_utils::atomic_write(manifest, lines.as_bytes()).map_err(err)?;
    }
    Ok(())
}

fn download_items(
    web: &WebClient,
    results: &[Value],
    data_stem: &Path,
    options: &DownloadOptions<'_>,
) -> Result<(), String> {
    fs::create_dir_all(data_stem).map_err(err)?;
    for item in results {
        let filename = item_filename(item, options.kind, options.fields, options.separator);
        let item_dir = item_directory(data_stem, item, options.kind, &filename);
        match item.get("download_addr") {
            Some(Value::Array(urls)) => {
                fs::create_dir_all(&item_dir).map_err(err)?;
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
            fs::create_dir_all(&item_dir).map_err(err)?;
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
        fs::create_dir_all(parent).map_err(err)?;
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
    fs_utils::atomic_copy(reader, path).map(|_| ()).map_err(err)
}

fn write_title(item: &Value, directory: &Path) -> Result<(), String> {
    fs_utils::atomic_write(
        &directory.join(format!("{}_title.txt", string_field(item, "id"))),
        string_field(item, "desc").as_bytes(),
    )
    .map_err(err)
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
                if item.get("type").and_then(net::value_i64) == Some(68) {
                    "图文".to_owned()
                } else {
                    "视频".to_owned()
                }
            }
            "duration" => item
                .get("duration")
                .and_then(net::value_i64)
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
    let mut text = serde_json::to_string_pretty(value).map_err(err)?;
    text.push('\n');
    fs_utils::atomic_write(path, text.as_bytes()).map_err(err)
}

fn default_download_root() -> PathBuf {
    settings::home_dir().join("Downloads").join("douyin")
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

fn value_text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn string_field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use super::{
        CrawlArgs, CrawlType, Target, WebClient, default_download_root, extract_escaped_value,
        item_filename, list_request, parse_aweme, parse_user, persist_download, resolve_targets,
        sanitize_filename,
    };
    use crate::test_support::{must, present};
    use serde_json::json;

    fn search_args() -> CrawlArgs {
        CrawlArgs {
            urls: Vec::new(),
            limit: 0,
            no_download: true,
            crawl_type: CrawlType::Search,
            output_path: default_download_root(),
            cookie: None,
            sort_type: None,
            publish_time: None,
            filter_duration: None,
            download_title: false,
            download_cover: false,
        }
    }

    #[test]
    fn current_search_params_keep_session_and_normalize_half_year_filter() {
        let target = Target {
            id: "关键词".to_owned(),
            url: "https://www.douyin.com/search/关键词".to_owned(),
            kind: CrawlType::Search,
        };
        let mut args = search_args();
        let unfiltered = must(list_request(&target, 10, "search-log", &args));
        assert!(
            !unfiltered
                .params
                .iter()
                .any(|(key, _)| key == "filter_selected")
        );
        assert!(
            unfiltered
                .params
                .contains(&("count".to_owned(), "10".to_owned()))
        );
        assert!(
            unfiltered
                .params
                .contains(&("search_id".to_owned(), "search-log".to_owned()))
        );

        args.publish_time = Some("180".to_owned());
        let filtered = must(list_request(&target, 0, "", &args));
        let (_, filters) = present(
            filtered
                .params
                .iter()
                .find(|(key, _)| key == "filter_selected"),
        );
        let filters = must(serde_json::from_str::<serde_json::Value>(filters));
        assert_eq!(filters["publish_time"], "182");
    }

    #[test]
    fn collection_rejects_other_account_targets() {
        assert_eq!(must(resolve_targets(&[], CrawlType::Collection)), vec![""]);
        assert!(resolve_targets(&["someone".to_owned()], CrawlType::Collection).is_err());
    }

    #[test]
    fn collection_cursor_and_count_are_form_fields() {
        let target = Target {
            id: "self".to_owned(),
            url: "https://www.douyin.com/user/self".to_owned(),
            kind: CrawlType::Collection,
        };
        let request = must(list_request(&target, 42, "", &search_args()));
        let form = present(request.form);
        assert!(form.contains(&("cursor".to_owned(), "42".to_owned())));
        assert!(form.contains(&("count".to_owned(), "18".to_owned())));
        assert!(
            request
                .params
                .contains(&("version_code".to_owned(), "170400".to_owned()))
        );
        assert!(!request.params.iter().any(|(key, _)| key == "sec_user_id"));
    }

    #[test]
    fn parses_video_and_image_awemes() {
        let video = present(parse_aweme(
            &json!({
                "aweme_type":4,"aweme_id":"1","create_time":10,"desc":"标题",
                "statistics":{"digg_count":2},"video":{"play_addr":{"url_list":["https://video"]},"duration":12000},
                "author":{"nickname":"作者","sec_uid":"sec","avatar_thumb":{"url_list":["https://avatar"]}}
            }),
            CrawlType::Post,
        ));
        assert_eq!(video["download_addr"], "https://video");
        assert_eq!(video["author_nickname"], "作者");
        let image = present(parse_aweme(
            &json!({
                "aweme_type":68,"aweme_id":"2","desc":"图集","images":[{"url_list":["https://image"]}]
            }),
            CrawlType::Aweme,
        ));
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
        let web = must(WebClient::new(
            "sessionid=test",
            crate::net::DEFAULT_USER_AGENT,
        ));
        let target = must(Target::parse(
            &web,
            "https://www.douyin.com/search/%E4%BA%8C%E6%89%8B%E8%BD%A6",
            CrawlType::Post,
        ));
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
        must(fs::create_dir_all(&directory));
        let path = directory.join("sample.bin");
        let mut body = Cursor::new(b"media");
        must(persist_download(&mut body, &path));
        assert_eq!(must(fs::read(&path)), b"media");
        must(fs::remove_file(path));
        must(fs::remove_dir(directory));
    }
}
