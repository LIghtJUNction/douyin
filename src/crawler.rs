use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, ValueEnum};
use serde_json::{Value, json};
use url::Url;

use crate::{io, net::NativeClient, paging};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CrawlType {
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
    Feed,
}

#[derive(Debug, Args)]
pub struct CrawlArgs {
    /// 作品、账号、音乐、话题、合集 ID 或 URL，搜索词，或批量目标文件
    #[arg(short = 'u', long = "urls")]
    urls: Vec<String>,
    /// 最多采集记录数，0 为不限
    #[arg(short, long, default_value_t = 0)]
    limit: usize,
    #[arg(short = 't', long = "type", value_enum, default_value = "post")]
    crawl_type: CrawlType,
    /// 输出目录；数据写入目标对应的 JSON 文件
    #[arg(short = 'p', long = "path", default_value = "./downloads")]
    output_path: PathBuf,
    #[arg(long)]
    no_download: bool,
    #[arg(long)]
    download_title: bool,
    #[arg(long)]
    download_cover: bool,
    #[arg(long,value_parser=["0","1","2"])]
    sort_type: Option<String>,
    #[arg(long,value_parser=["0","1","7","180","182"])]
    publish_time: Option<String>,
    #[arg(long,value_parser=["","0-1","1-5","5-10000"])]
    filter_duration: Option<String>,
}

impl CrawlArgs {
    pub fn should_run(&self) -> bool {
        !self.urls.is_empty()
            || self.limit != 0
            || self.crawl_type != CrawlType::Post
            || self.no_download
            || self.download_cover
            || self.download_title
            || self.sort_type.is_some()
            || self.publish_time.is_some()
            || self.filter_duration.is_some()
            || self.output_path != std::path::Path::new("./downloads")
    }
}

pub fn run(args: CrawlArgs) -> Result<(), String> {
    let client = NativeClient::saved()?;
    let targets = targets(&args.urls)?;
    std::fs::create_dir_all(&args.output_path).map_err(|_| "无法创建采集输出目录")?;
    let mut failures = 0;
    for target in targets {
        let result = (|| {
            let id = resolve_target(&target, args.crawl_type)?;
            let mut data = fetch(&client, args.crawl_type, &id, &args)?;
            let kind = args
                .crawl_type
                .to_possible_value()
                .map(|v| v.get_name().to_owned())
                .unwrap_or_default();
            let archive_id = if id.is_empty() {
                client.account_id()?
            } else {
                &id
            };
            data["target"] = json!({"type":kind,"id":archive_id});
            let filename = format!("{}_{}.json", kind, crate::download::filename(archive_id));
            let path = args.output_path.join(filename);
            if args.crawl_type == CrawlType::Post {
                crate::archive::save_posts(&mut data, &path)?;
            } else {
                io::write_output(
                    &serde_json::to_string_pretty(&data).map_err(|_| "采集数据序列化失败")?,
                    Some(&path),
                )?;
            }
            if !args.no_download
                && !matches!(args.crawl_type, CrawlType::Following | CrawlType::Follower)
            {
                crate::download::items(
                    data["items"].as_array().ok_or("采集结果缺少 items")?,
                    &args.output_path,
                    args.download_title,
                    args.download_cover,
                )?;
            }
            eprintln!(
                "已保存 {} 条记录：{}",
                data["items"].as_array().map_or(0, Vec::len),
                path.display()
            );
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            failures += 1;
            eprintln!("采集失败：{error}");
        }
    }
    if failures > 0 {
        Err(format!("{failures} 个目标未完成，已保存的结果保留"))
    } else {
        Ok(())
    }
}

fn fetch(
    client: &NativeClient,
    kind: CrawlType,
    target: &str,
    args: &CrawlArgs,
) -> Result<Value, String> {
    if kind == CrawlType::Aweme {
        let value = client.get(
            "/aweme/v1/aweme/detail/",
            &[("aweme_id".into(), target.into())],
        )?;
        let item = value
            .get("aweme_detail")
            .filter(|v| v.is_object())
            .ok_or("接口没有返回作品详情")?;
        return Ok(json!({"items":[item],"has_more":false,"pages":1}));
    }
    let target = if target.is_empty() {
        client.account_sec_id()?
    } else {
        target
    };
    let profile = if kind == CrawlType::Post && !client.is_web_transport() {
        Some(if client.is_current_account(target) {
            client.me()?
        } else {
            client.user(target)?
        })
    } else {
        None
    };
    let mut search_id = String::new();
    let mut first = true;
    let id_key = if matches!(kind, CrawlType::Following | CrawlType::Follower) {
        "uid"
    } else {
        "aweme_id"
    };
    let result = paging::collect("0".into(), args.limit, id_key, |cursor, remaining| {
        if !first {
            std::thread::sleep(Duration::from_millis(800));
        }
        first = false;
        let (path, list_key, request_cursor, response_cursor, id_key) = match kind {
            CrawlType::Post => (
                "/aweme/v1/aweme/post/",
                "aweme_list",
                "max_cursor",
                "max_cursor",
                "sec_user_id",
            ),
            CrawlType::Favorite => (
                "/aweme/v1/aweme/favorite/",
                "aweme_list",
                "max_cursor",
                "max_cursor",
                "sec_user_id",
            ),
            CrawlType::Music => (
                "/aweme/v1/music/aweme/",
                "aweme_list",
                "cursor",
                "cursor",
                "music_id",
            ),
            CrawlType::Hashtag => (
                "/aweme/v1/challenge/aweme/",
                "aweme_list",
                "cursor",
                "cursor",
                "ch_id",
            ),
            CrawlType::Mix => (
                "/aweme/v1/mix/aweme/",
                "aweme_list",
                "cursor",
                "cursor",
                "mix_id",
            ),
            CrawlType::Following => (
                "/aweme/v1/user/following/list/",
                "followings",
                "max_time",
                "min_time",
                "sec_user_id",
            ),
            CrawlType::Follower => (
                "/aweme/v1/user/follower/list/",
                "followers",
                "max_time",
                "min_time",
                "sec_user_id",
            ),
            CrawlType::Collection => (
                "/aweme/v1/aweme/listcollection/",
                "aweme_list",
                "cursor",
                "cursor",
                "sec_user_id",
            ),
            CrawlType::Search => (
                "/aweme/v2/search_feed/",
                "aweme_list",
                "offset",
                "offset",
                "keyword",
            ),
            CrawlType::Feed => (
                "/aweme/v1/feed/",
                "aweme_list",
                "max_cursor",
                "max_cursor",
                "sec_user_id",
            ),
            CrawlType::Aweme => return Err("单作品类型不使用列表分页".into()),
        };
        let mut params = vec![
            (request_cursor.into(), cursor.into()),
            ("count".into(), remaining.unwrap_or(20).min(20).to_string()),
        ];
        if target.bytes().all(|b| b.is_ascii_digit())
            && matches!(
                kind,
                CrawlType::Post | CrawlType::Favorite | CrawlType::Following | CrawlType::Follower
            )
        {
            params.push(("user_id".into(), target.into()));
        } else {
            params.push((id_key.into(), target.into()));
        }
        if matches!(kind, CrawlType::Following | CrawlType::Follower) {
            params.push(("min_time".into(), "0".into()));
            if target == client.account_sec_id()? {
                params.push(("user_id".into(), client.account_id()?.into()));
            }
        }
        if kind == CrawlType::Search {
            let request_search_id = if search_id.is_empty() {
                crate::native_auth::request_ticket()?
            } else {
                search_id.clone()
            };
            params.extend([
                ("pull_type".into(), "1".into()),
                ("search_source".into(), "normal_search".into()),
                ("search_id".into(), request_search_id),
            ]);
            if let Some(sort) = &args.sort_type {
                params.push(("sort_type".into(), sort.clone()));
            }
            if let Some(time) = &args.publish_time {
                params.push((
                    "publish_time".into(),
                    if time == "180" {
                        "182".into()
                    } else {
                        time.clone()
                    },
                ));
            }
            if let Some(duration) = &args.filter_duration {
                params.push(("filter_duration".into(), duration.clone()));
            }
        }
        let response = if kind == CrawlType::Search {
            client.post(path, &params)?
        } else {
            client.get(path, &params)?
        };
        if let Some(id) = response["log_pb"]["impr_id"].as_str() {
            search_id = id.to_owned();
        }
        let response_list_key = if kind == CrawlType::Hashtag && response["mix_list"].is_array() {
            "mix_list"
        } else {
            list_key
        };
        let mut page = paging::parse_page(&response, response_list_key, response_cursor)?;
        if matches!(kind, CrawlType::Search | CrawlType::Hashtag) {
            page.items = page
                .items
                .into_iter()
                .filter_map(|row| {
                    row.get("aweme_info")
                        .filter(|item| item.is_object())
                        .cloned()
                        .or_else(|| row.get("aweme_id").map(|_| row.clone()))
                })
                .collect();
        }
        Ok(page)
    })?;
    if result["items"].as_array().is_some_and(Vec::is_empty)
        && profile
            .as_ref()
            .and_then(|p| integer_count(&p["aweme_count"]))
            .is_some_and(|count| count > 0)
    {
        return Err("账号资料显示有作品，但接口返回空列表；可能受可见范围、签名或风控影响，无法确认采集完整".into());
    }
    if kind == CrawlType::Post
        && profile.is_none()
        && result["items"].as_array().is_some_and(Vec::is_empty)
    {
        return Err("网页接口返回空作品列表，且没有账号资料可交叉验证，无法确认采集成功".into());
    }
    Ok(result)
}

fn integer_count(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn targets(inputs: &[String]) -> Result<Vec<String>, String> {
    if inputs.is_empty() {
        return Ok(vec![String::new()]);
    }
    let mut targets = Vec::new();
    for input in inputs {
        if std::path::Path::new(input).is_file() {
            targets.extend(
                io::read_input(input)?
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_owned),
            );
        } else {
            targets.push(input.to_owned());
        }
    }
    if targets.is_empty() {
        return Err("目标文件没有可采集的记录".into());
    }
    Ok(targets)
}

pub(crate) fn resolve_target(input: &str, kind: CrawlType) -> Result<String, String> {
    if kind == CrawlType::Search {
        return Ok(input.to_owned());
    }
    if input.is_empty() {
        return if matches!(
            kind,
            CrawlType::Post
                | CrawlType::Favorite
                | CrawlType::Following
                | CrawlType::Follower
                | CrawlType::Collection
                | CrawlType::Feed
        ) {
            Ok(String::new())
        } else {
            Err("该采集类型需要指定目标".into())
        };
    }
    if !input.contains("http") {
        return if input
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            Ok(input.into())
        } else {
            Err("目标 ID 含无效字符".into())
        };
    }
    let raw = input
        .split_whitespace()
        .find(|part| part.starts_with("https://"))
        .unwrap_or(input)
        .trim_end_matches(['。', '，', ',', ';', '；']);
    let url = Url::parse(raw).map_err(|_| "目标 URL 无效")?;
    validate_target_url(&url)?;
    let url = if url.host_str() == Some("v.douyin.com") {
        resolve_short_url(url)?
    } else {
        url
    };
    let segment = url
        .path_segments()
        .ok_or("目标 URL 没有路径")?
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>();
    let expected = match kind {
        CrawlType::Aweme => vec!["video", "note"],
        CrawlType::Post | CrawlType::Favorite | CrawlType::Following | CrawlType::Follower => {
            vec!["user"]
        }
        CrawlType::Music => vec!["music"],
        CrawlType::Hashtag => vec!["hashtag", "challenge"],
        CrawlType::Mix => vec!["collection", "mix"],
        _ => vec![],
    };
    if kind == CrawlType::Aweme
        && let Some((_, id)) = url.query_pairs().find(|(key, _)| key == "modal_id")
    {
        return validate_id(&id);
    }
    if segment.len() >= 2 && expected.contains(&segment[0]) {
        validate_id(segment[1])
    } else {
        Err("目标 URL 与采集类型不匹配".into())
    }
}

fn validate_id(id: &str) -> Result<String, String> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        Ok(id.into())
    } else {
        Err("目标 ID 无效".into())
    }
}
fn validate_target_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some("www.douyin.com" | "douyin.com" | "v.douyin.com" | "www.iesdouyin.com")
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("仅支持抖音官方 HTTPS 分享链接".into());
    }
    Ok(())
}
fn resolve_short_url(mut url: Url) -> Result<Url, String> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "无法创建分享链接客户端")?;
    for _ in 0..5 {
        validate_target_url(&url)?;
        let response = client
            .get(url.clone())
            .send()
            .map_err(|_| "解析分享链接失败")?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("分享链接重定向没有地址")?;
            url = url.join(location).map_err(|_| "分享链接跳转无效")?;
        } else if response.status().is_success() {
            return Ok(url);
        } else {
            return Err("分享链接返回异常状态".into());
        }
    }
    Err("分享链接跳转次数过多".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_target_kind_and_domains_without_network() {
        assert_eq!(
            resolve_target("https://www.douyin.com/video/123", CrawlType::Aweme).unwrap(),
            "123"
        );
        assert_eq!(
            resolve_target(
                "https://www.douyin.com/user/abc?modal_id=456",
                CrawlType::Aweme
            )
            .unwrap(),
            "456"
        );
        for url in [
            "https://evil.invalid/video/123",
            "https://www.douyin.com.evil.invalid/video/123",
            "https://www.douyin.com:444/video/123",
            "https://www.douyin.com/user/123",
        ] {
            assert!(resolve_target(url, CrawlType::Aweme).is_err());
        }
    }
}
