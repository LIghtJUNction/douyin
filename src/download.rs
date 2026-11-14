use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use reqwest::{blocking::Client, redirect::Policy};
use serde_json::Value;
use url::Url;

use crate::io;

pub(crate) fn filename(value: &str) -> String {
    let name: String = value
        .chars()
        .take(100)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let name = name.trim_matches([' ', '.']);
    if name.is_empty() {
        "untitled".into()
    } else {
        format!("item_{name}")
    }
}

pub fn items(items: &[Value], root: &Path, title: bool, cover: bool) -> Result<(), String> {
    // Never forward the API client's Cookie or device parameters to media hosts.
    let client = client()?;
    let mut errors = 0;
    for item in items {
        let result = (|| {
            let id_value = item
                .get("aweme_id")
                .or_else(|| item.get("id"))
                .ok_or("作品缺少 ID")?;
            let id = id_value
                .as_str()
                .map(str::to_owned)
                .or_else(|| id_value.as_u64().map(|id| id.to_string()))
                .ok_or("作品 ID 无效")?;
            let stem = filename(&id);
            if title {
                io::write_output(
                    item["desc"].as_str().unwrap_or(""),
                    Some(&root.join(format!("{stem}.txt"))),
                )?;
            }
            if let Some(images) = item["images"]
                .as_array()
                .or_else(|| item["download_addr"].as_array())
                .filter(|images| !images.is_empty())
            {
                for (index, image) in images.iter().enumerate() {
                    download(
                        &client,
                        address(image)?,
                        &root.join(format!("{stem}_{:03}.jpg", index + 1)),
                    )?;
                }
            } else {
                let video = &item["video"];
                let play = video
                    .get("play_addr")
                    .or_else(|| video.get("download_addr"))
                    .or_else(|| item.get("download_addr").filter(|value| value.is_string()))
                    .ok_or("作品没有可下载的视频地址")?;
                download(&client, address(play)?, &root.join(format!("{stem}.mp4")))?;
            }
            if cover {
                let value = item["video"]
                    .get("cover")
                    .or_else(|| item.get("cover"))
                    .ok_or("作品没有封面地址")?;
                download(
                    &client,
                    address(value)?,
                    &root.join(format!("{stem}_cover.jpg")),
                )?;
            }
            Ok::<_, String>(())
        })();
        if let Err(error) = result {
            errors += 1;
            eprintln!("下载失败：{error}");
        }
    }
    if errors > 0 {
        Err(format!("{errors} 个作品未完整下载，已完成文件保留"))
    } else {
        Ok(())
    }
}

pub fn url(raw: &str, path: &Path) -> Result<(), String> {
    media_url(raw)?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|_| "无法创建下载目录")?;
    }
    download(&client()?, raw, path)
}

fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|_| "无法创建下载客户端".into())
}

fn address(value: &Value) -> Result<&str, String> {
    if let Some(raw) = value.as_str().filter(|raw| !raw.is_empty()) {
        return Ok(raw);
    }
    value
        .get("url_list")
        .or_else(|| value.get("urlList"))
        .and_then(Value::as_array)
        .and_then(|urls| urls.iter().find_map(Value::as_str))
        .ok_or_else(|| "媒体地址列表为空".into())
}
fn media_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| "媒体 URL 无效")?;
    let host = url.host_str().ok_or("媒体 URL 缺少主机")?;
    let allowed = [
        "douyinvod.com",
        "douyinpic.com",
        "byteimg.com",
        "bytecdn.cn",
        "douyincdn.com",
        "snssdk.com",
        "amemv.com",
        "douyinstatic.com",
        "pstatp.com",
        "ibytedtos.com",
    ];
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || !allowed.iter().any(|domain| {
            host == *domain
                || host
                    .strip_suffix(domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        })
    {
        return Err("媒体地址不属于受支持的抖音 HTTPS CDN".into());
    }
    Ok(url)
}

fn download(client: &Client, raw: &str, path: &Path) -> Result<(), String> {
    if path.exists() {
        let size = std::fs::metadata(path)
            .map_err(|_| "读取已下载文件失败")?
            .len();
        if size > 0 {
            return Ok(());
        }
    }
    let mut url = media_url(raw)?;
    for _ in 0..5 {
        let response = client
            .get(url.clone())
            .header("Referer", "https://www.douyin.com/")
            .send()
            .map_err(|_| "媒体下载请求失败")?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or("媒体跳转地址缺失")?;
            let next = url.join(location).map_err(|_| "媒体跳转无效")?;
            url = media_url(next.as_str())?;
            continue;
        }
        if !response.status().is_success() {
            return Err(format!("媒体返回 HTTP {}", response.status().as_u16()));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        if !(content_type.starts_with("video/")
            || content_type.starts_with("image/")
            || content_type.starts_with("application/octet-stream"))
        {
            return Err("媒体响应类型异常，拒绝保存网页或错误响应".into());
        }
        let expected = response.content_length();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut pending =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| "无法创建媒体临时文件")?;
        let size = std::io::copy(&mut response.take(4 * 1024 * 1024 * 1024 + 1), &mut pending)
            .map_err(|_| "媒体下载中断，未提交不完整文件")?;
        if size == 0
            || size > 4 * 1024 * 1024 * 1024
            || expected.is_some_and(|length| length != size)
        {
            return Err("媒体大小异常或下载不完整".into());
        }
        pending
            .flush()
            .and_then(|_| pending.as_file().sync_all())
            .map_err(|_| "媒体文件写入失败")?;
        pending
            .persist_noclobber(path)
            .map_err(|_| "媒体文件已存在或无法提交")?;
        return Ok(());
    }
    Err("媒体跳转次数过多".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incremental_archives_can_reuse_native_and_legacy_media_without_network() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "item_1.mp4",
            "item_2_001.jpg",
            "item_2_002.jpg",
            "item_2_cover.jpg",
            "item_3.mp4",
        ] {
            std::fs::write(root.path().join(name), b"already-downloaded-fixture").unwrap();
        }
        let records = serde_json::json!([
            {"aweme_id":"1","desc":"native video","video":{"play_addr":{"url_list":["https://invalid.invalid/no-network"]},"cover":{"url_list":["https://invalid.invalid/no-network"]}}},
            {"id":2,"desc":"legacy album","download_addr":["https://invalid.invalid/no-network","https://invalid.invalid/no-network"],"cover":"https://invalid.invalid/no-network"},
            {"id":"3","desc":"legacy video","download_addr":"https://invalid.invalid/no-network","cover":"https://invalid.invalid/no-network"}
        ]);
        items(records.as_array().unwrap(), root.path(), true, false).unwrap();
        items(&[records[1].clone()], root.path(), false, true).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("item_2.txt")).unwrap(),
            "legacy album\n"
        );
        assert_eq!(
            std::fs::read(root.path().join("item_1.mp4")).unwrap(),
            b"already-downloaded-fixture"
        );
    }
    #[test]
    fn prevents_path_traversal_reserved_names_and_external_media() {
        for name in ["../escape", "CON", "...", "a/b\\c"] {
            let safe = filename(name);
            assert!(!safe.contains('/'));
            assert!(!safe.contains('\\'));
            assert!(!safe.starts_with('.'));
        }
        for url in [
            "https://localhost/a",
            "http://v1.douyinvod.com/a",
            "https://douyinvod.com.evil.invalid/a",
            "https://v1.douyinvod.com:444/a",
        ] {
            assert!(media_url(url).is_err());
        }
        assert!(media_url("https://v1.douyinvod.com/video/file.mp4").is_ok());
    }
}
