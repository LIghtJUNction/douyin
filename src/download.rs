use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use reqwest::{blocking::Client, redirect::Policy};
use serde_json::Value;
use url::Url;

use crate::io;

const MAX_CANDIDATE_URLS: usize = 8;

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
                        &addresses(image)?,
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
                download(
                    &client,
                    &addresses(play)?,
                    &root.join(format!("{stem}.mp4")),
                )?;
            }
            if cover {
                let value = item["video"]
                    .get("cover")
                    .or_else(|| item.get("cover"))
                    .ok_or("作品没有封面地址")?;
                download(
                    &client,
                    &addresses(value)?,
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
    download(&client()?, &[raw], path)
}

fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|_| "无法创建下载客户端".into())
}

fn addresses(value: &Value) -> Result<Vec<&str>, String> {
    if let Some(raw) = value.as_str().filter(|raw| !raw.trim().is_empty()) {
        return Ok(vec![raw]);
    }
    let mut candidates = Vec::new();
    if let Some(urls) = value
        .get("url_list")
        .or_else(|| value.get("urlList"))
        .and_then(Value::as_array)
    {
        for raw in urls.iter().filter_map(Value::as_str) {
            if !raw.trim().is_empty() && !candidates.contains(&raw) {
                candidates.push(raw);
                if candidates.len() == MAX_CANDIDATE_URLS {
                    break;
                }
            }
        }
    }
    if candidates.is_empty() {
        Err("媒体地址列表为空".into())
    } else {
        Ok(candidates)
    }
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

fn existing_size(path: &Path) -> Result<Option<u64>, String> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(metadata.len())),
        Ok(_) => Err("下载目标必须是普通文件，不能是目录或特殊文件".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("读取已下载文件失败".into()),
    }
}

fn download(client: &Client, candidates: &[&str], path: &Path) -> Result<(), String> {
    download_with(client, candidates, path, media_url)
}

fn download_with(
    client: &Client,
    candidates: &[&str],
    path: &Path,
    parse_url: fn(&str) -> Result<Url, String>,
) -> Result<(), String> {
    if existing_size(path)?.is_some_and(|size| size > 0) {
        return Ok(());
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut attempts = 0;
    let mut last_error = String::new();
    for raw in candidates.iter().take(MAX_CANDIDATE_URLS) {
        attempts += 1;
        match fetch_media(client, raw, parent, parse_url) {
            Ok(pending) => {
                // Preserve a file completed by another downloader. Replace an
                // empty target only after the entire new file is safely written.
                match existing_size(path)? {
                    Some(0) => pending.persist(path).map_err(|_| "无法替换空的媒体文件")?,
                    Some(_) => return Ok(()),
                    None => pending
                        .persist_noclobber(path)
                        .map_err(|_| "媒体文件已存在或无法提交")?,
                };
                return Ok(());
            }
            Err(error) => last_error = error,
        }
    }
    if attempts == 0 {
        Err("媒体地址列表为空".into())
    } else {
        // Signed CDN URLs can contain credentials; only report a safe cause.
        Err(format!("{attempts} 个媒体地址均下载失败：{last_error}"))
    }
}

fn fetch_media(
    client: &Client,
    raw: &str,
    parent: &Path,
    parse_url: fn(&str) -> Result<Url, String>,
) -> Result<tempfile::NamedTempFile, String> {
    let mut url = parse_url(raw)?;
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
            url = parse_url(next.as_str())?;
            continue;
        }
        // No Range header was sent, so a partial response is not a complete file.
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!("媒体返回 HTTP {}", response.status().as_u16()));
        }
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !(content_type.starts_with("video/")
            || content_type.starts_with("image/")
            || content_type.starts_with("application/octet-stream"))
        {
            return Err("媒体响应类型异常，拒绝保存网页或错误响应".into());
        }
        let expected = response.content_length();
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
        return Ok(pending);
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
    fn loopback_url(raw: &str) -> Result<Url, String> {
        let url = Url::parse(raw).map_err(|_| "测试 URL 无效")?;
        if url.scheme() != "http"
            || url.host_str() != Some("127.0.0.1")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("测试请求必须留在回环地址".into());
        }
        Ok(url)
    }

    fn reply(status: u16, content_type: &str, body: &str, extra: &str) -> String {
        format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}",
            body.len()
        )
    }

    fn fixture(replies: Vec<String>) -> (Client, String, std::thread::JoinHandle<Vec<String>>) {
        use std::net::TcpListener;
        use std::time::Instant;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let thread = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing media request");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("accept failed: {error}"),
                    }
                };
                // Accepted sockets can inherit the listener's nonblocking mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 16 * 1024);
                }
                requests.push(String::from_utf8(request).unwrap());
                stream.write_all(reply.as_bytes()).unwrap();
            }
            requests
        });
        let client = Client::builder()
            .redirect(Policy::none())
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        (client, base, thread)
    }

    #[test]
    fn empty_file_recovers_via_fallback_and_checked_redirect_without_credentials() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("video.mp4");
        std::fs::write(&path, b"").unwrap();
        let (client, base, thread) = fixture(vec![
            reply(403, "text/plain", "expired", ""),
            reply(302, "text/plain", "", "Location: /final\r\n"),
            reply(200, "video/mp4", "complete-video", ""),
        ]);
        download_with(
            &client,
            &[
                &format!("{base}/expired?token=private"),
                &format!("{base}/next"),
            ],
            &path,
            loopback_url,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete-video");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        let requests = thread.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("GET /expired?token=private "));
        assert!(requests[1].starts_with("GET /next "));
        assert!(requests[2].starts_with("GET /final "));
        for request in requests {
            let headers = request.to_ascii_lowercase();
            assert!(headers.contains("\r\nreferer: https://www.douyin.com/\r\n"));
            for header in ["cookie", "authorization", "access-token", "x-tt-token"] {
                assert!(!headers.contains(&format!("\r\n{header}:")));
            }
        }
    }

    #[test]
    fn directories_are_rejected_and_existing_complete_files_are_reused() {
        let root = tempfile::tempdir().unwrap();
        let client = client().unwrap();
        let error = download(&client, &["invalid"], root.path()).unwrap_err();
        assert!(error.contains("普通文件"));
        let path = root.path().join("video.mp4");
        std::fs::write(&path, b"complete").unwrap();
        download(&client, &["invalid"], &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete");
    }

    #[test]
    fn failed_candidates_preserve_empty_or_absent_targets_and_hide_urls() {
        for existing in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("video.mp4");
            if existing {
                std::fs::write(&path, b"").unwrap();
            }
            let (client, base, thread) = fixture(vec![
                reply(403, "text/plain", "private-response", ""),
                reply(200, "text/html", "private-response", ""),
            ]);
            let error = download_with(
                &client,
                &[
                    &format!("{base}/one?token=private"),
                    &format!("{base}/two?token=private"),
                ],
                &path,
                loopback_url,
            )
            .unwrap_err();
            assert!(error.contains("2 个媒体地址"));
            assert!(!error.contains("private"));
            assert!(!error.contains(&base));
            assert_eq!(path.exists(), existing);
            if existing {
                assert!(std::fs::read(&path).unwrap().is_empty());
            }
            assert_eq!(
                std::fs::read_dir(root.path()).unwrap().count(),
                usize::from(existing)
            );
            assert_eq!(thread.join().unwrap().len(), 2);
        }
    }

    #[test]
    fn truncated_and_partial_responses_can_fall_back_without_committing_them() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("video.mp4");
        let truncated = "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 100\r\nConnection: close\r\n\r\ntruncated".into();
        let (client, base, thread) = fixture(vec![
            truncated,
            reply(206, "video/mp4", "partial", ""),
            reply(200, "video/mp4", "full", ""),
        ]);
        download_with(
            &client,
            &[
                &format!("{base}/truncated"),
                &format!("{base}/partial"),
                &format!("{base}/full"),
            ],
            &path,
            loopback_url,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"full");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        assert_eq!(thread.join().unwrap().len(), 3);
    }

    #[test]
    fn every_candidate_and_redirect_is_validated_before_requesting_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("video.mp4");
        let (client, base, thread) = fixture(vec![
            reply(
                302,
                "text/plain",
                "",
                "Location: https://evil.invalid/private\r\n",
            ),
            reply(200, "video/mp4", "full", ""),
        ]);
        download_with(
            &client,
            &[
                "https://evil.invalid/private",
                &format!("{base}/redirect"),
                &format!("{base}/full"),
            ],
            &path,
            loopback_url,
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"full");
        assert_eq!(thread.join().unwrap().len(), 2);
    }

    #[test]
    fn fallback_and_redirect_attempts_are_bounded() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("video.mp4");
        let (client, base, thread) =
            fixture(vec![reply(404, "text/plain", "", ""); MAX_CANDIDATE_URLS]);
        let candidates: Vec<String> = (0..MAX_CANDIDATE_URLS + 1)
            .map(|i| format!("{base}/{i}"))
            .collect();
        let urls: Vec<&str> = candidates.iter().map(String::as_str).collect();
        let error = download_with(&client, &urls, &path, loopback_url).unwrap_err();
        assert!(error.contains(&format!("{MAX_CANDIDATE_URLS} 个媒体地址")));
        assert_eq!(thread.join().unwrap().len(), MAX_CANDIDATE_URLS);
        assert!(!path.exists());

        let (client, base, thread) =
            fixture(vec![
                reply(302, "text/plain", "", "Location: /again\r\n");
                5
            ]);
        let error =
            download_with(&client, &[&format!("{base}/again")], &path, loopback_url).unwrap_err();
        assert!(error.contains("跳转次数过多"));
        assert_eq!(thread.join().unwrap().len(), 5);
        assert!(!path.exists());
    }

    #[test]
    fn address_candidates_preserve_order_and_skip_empty_or_duplicate_entries() {
        let value =
            serde_json::json!({"url_list": ["", "  ", null, 4, "first", "first", "second"]});
        assert_eq!(addresses(&value).unwrap(), ["first", "second"]);
        assert_eq!(addresses(&serde_json::json!("legacy")).unwrap(), ["legacy"]);
        assert_eq!(
            addresses(&serde_json::json!({"urlList": ["other"]})).unwrap(),
            ["other"]
        );
        assert!(addresses(&serde_json::json!({"url_list": ["", null]})).is_err());
        let urls: Vec<String> = (0..MAX_CANDIDATE_URLS + 2).map(|n| n.to_string()).collect();
        let value = serde_json::json!({"url_list": urls});
        assert_eq!(addresses(&value).unwrap().len(), MAX_CANDIDATE_URLS);
    }
}
