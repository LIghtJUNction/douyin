use ring::digest::{Context, SHA256};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

fn hash(mut input: impl Read) -> Result<String, String> {
    let mut digest = Context::new(&SHA256);
    let mut buffer = [0u8; 65536];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_| "读取手机支持文件失败")?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(digest
        .finish()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

pub(crate) fn server(abi: &str) -> Result<PathBuf, String> {
    let arch = match abi {
        "arm64-v8a" => "arm64",
        "armeabi-v7a" | "armeabi" => "arm",
        "x86_64" => "x86_64",
        "x86" => "x86",
        _ => return Err("手机通道暂不支持该处理器架构".into()),
    };
    let version = frida::Frida::version();
    let catalog: Value = serde_json::from_str(include_str!("../assets/phone/server-assets.json"))
        .map_err(|_| "内置手机支持文件清单无效")?;
    if catalog["version"] != version {
        return Err("内置手机通道资产与当前 SDK 版本不一致".into());
    }
    let name = format!("frida-server-{version}-android-{arch}.xz");
    let asset = catalog["assets"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["name"] == name))
        .ok_or("内置清单缺少该手机的支持文件")?;
    let expected = asset["digest"]
        .as_str()
        .and_then(|text| text.strip_prefix("sha256:"))
        .filter(|text| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("内置手机支持文件校验值无效")?;
    let directory = crate::store::root()
        .join("cache/phone")
        .join(version)
        .join(arch);
    fs::create_dir_all(&directory).map_err(|_| "无法创建手机支持文件缓存")?;
    let binary = directory.join("server");
    let metadata = directory.join("server.json");
    if let (Ok(file), Ok(bytes)) = (File::open(&binary), fs::read(&metadata))
        && let Ok(record) = serde_json::from_slice::<Value>(&bytes)
        && record["assetDigest"] == expected
        && hash(file).is_ok_and(|digest| record["binaryDigest"] == digest)
    {
        return Ok(binary);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法限制手机缓存目录权限")?;
    }
    eprintln!("首次使用手机通道，正在下载并校验支持文件（{version}/{arch}）");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .user_agent(concat!("douyin-cli/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "无法创建支持文件下载客户端")?;
    let id = asset["id"].as_u64().ok_or("内置支持文件 ID 无效")?;
    let mut archive =
        tempfile::NamedTempFile::new_in(&directory).map_err(|_| "无法创建支持文件临时缓存")?;
    let urls = [
        format!("https://github.com/frida/frida/releases/download/{version}/{name}"),
        format!("https://api.github.com/repos/frida/frida/releases/assets/{id}"),
    ];
    let mut downloaded = false;
    for url in urls {
        let result = (|| {
            let response = client
                .get(url)
                .header("Accept", "application/octet-stream")
                .send()
                .map_err(|_| "支持文件下载失败")?;
            if !response.status().is_success() {
                return Err("支持文件下载返回异常状态");
            }
            archive
                .as_file_mut()
                .set_len(0)
                .map_err(|_| "重置支持文件缓存失败")?;
            use std::io::{Seek, SeekFrom};
            archive
                .as_file_mut()
                .seek(SeekFrom::Start(0))
                .map_err(|_| "定位支持文件缓存失败")?;
            let size = std::io::copy(&mut response.take(64 * 1024 * 1024 + 1), &mut archive)
                .map_err(|_| "读取支持文件失败")?;
            if size > 64 * 1024 * 1024 || size != asset["size"].as_u64().unwrap_or(0) {
                return Err("支持文件长度校验失败");
            }
            let digest = hash(File::open(archive.path()).map_err(|_| "无法读取支持文件")?)
                .map_err(|_| "支持文件校验失败")?;
            if digest != expected {
                return Err("支持文件 SHA-256 校验失败");
            }
            Ok(())
        })();
        if result.is_ok() {
            downloaded = true;
            break;
        }
    }
    if !downloaded {
        return Err("无法下载并校验手机通道支持文件，请稍后重试".into());
    }
    let input = File::open(archive.path()).map_err(|_| "无法读取支持文件压缩包")?;
    let mut decoder = lzma_rust2::XzReader::new_mem_limit(input, false, 128 * 1024);
    let mut pending =
        tempfile::NamedTempFile::new_in(&directory).map_err(|_| "无法准备手机支持程序")?;
    let size = std::io::copy(
        &mut decoder.by_ref().take(128 * 1024 * 1024 + 1),
        &mut pending,
    )
    .map_err(|_| "手机支持文件解压失败")?;
    if size == 0 || size > 128 * 1024 * 1024 {
        return Err("手机支持程序大小无效".into());
    }
    pending
        .flush()
        .and_then(|_| pending.as_file().sync_all())
        .map_err(|_| "保存手机支持程序失败")?;
    let binary_hash = hash(File::open(pending.path()).map_err(|_| "无法校验手机支持程序")?)?;
    pending
        .persist(&binary)
        .map_err(|_| "提交手机支持程序失败")?;
    crate::store::save_at(
        &metadata,
        &json!({"assetDigest":expected,"binaryDigest":binary_hash}),
    )
    .map_err(|_| "保存手机支持程序校验记录失败")?;
    Ok(binary)
}
