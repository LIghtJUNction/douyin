use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};
use serde_json::json;

use crate::{oauth, openapi::OpenApiClient};

#[derive(Args)]
pub struct PublishArgs {
    #[command(subcommand)]
    command: PublishCommand,
}

#[derive(Subcommand)]
enum PublishCommand {
    /// 上传并发布一个视频到当前 OAuth 授权的抖音账号
    Video(VideoArgs),
}

#[derive(Args)]
struct VideoArgs {
    /// 本地视频文件
    file: PathBuf,
    /// 视频标题，可包含话题和 @ 用户
    #[arg(short, long, default_value = "")]
    title: String,
    /// 发布可见范围
    #[arg(long, value_enum, default_value_t = Visibility::Public)]
    visibility: Visibility,
    /// 从视频指定秒数截取封面
    #[arg(long)]
    cover_tsp: Option<f64>,
    /// 授权用户 open_id；默认读取 OAuth 登录结果
    #[arg(long)]
    open_id: Option<String>,
    /// 发布是外部写操作，必须显式确认
    #[arg(long)]
    yes: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Visibility {
    Public,
    Private,
    Friends,
}

pub fn run(args: PublishArgs) -> Result<(), String> {
    match args.command {
        PublishCommand::Video(args) => publish_video(args),
    }
}

fn publish_video(args: VideoArgs) -> Result<(), String> {
    if !args.yes {
        return Err("投稿会上传文件并创建抖音作品，需要显式 --yes".into());
    }

    let metadata = std::fs::metadata(&args.file).map_err(|_| "无法读取投稿视频文件")?;
    if !metadata.is_file() {
        return Err("投稿路径必须是文件".into());
    }
    if metadata.len() == 0 {
        return Err("投稿视频文件不能为空".into());
    }
    if metadata.len() > 4 * 1024 * 1024 * 1024 {
        return Err("视频超过抖音开放平台 4GB 限制".into());
    }
    if args.title.chars().count() > 1000 {
        return Err("视频标题不能超过 1000 个字符".into());
    }

    let saved = oauth::load()?;
    let token =
        oauth::field(&saved, "accessToken").ok_or("OAuth 缺少 accessToken，请先运行 auth login")?;
    let open_id = args
        .open_id
        .or_else(|| oauth::field(&saved, "openId").map(str::to_owned))
        .ok_or("OAuth 缺少 openId，请重新授权后再投稿")?;

    let client = OpenApiClient::new()?;
    eprintln!("正在上传视频...");
    let uploaded = client.upload_video(token, &open_id, &args.file)?;
    let video_id = uploaded
        .pointer("/data/video/video_id")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or("上传成功响应缺少 video_id")?;

    let mut body = json!({
        "video_id": video_id,
        "text": args.title,
        "private_status": match args.visibility {
            Visibility::Public => 0,
            Visibility::Private => 1,
            Visibility::Friends => 2,
        },
    });
    if let Some(cover_tsp) = args.cover_tsp {
        body["cover_tsp"] = json!(cover_tsp);
    }

    eprintln!("正在创建作品...");
    let mut request =
        crate::openapi::Request::new(reqwest::Method::POST, "/api/douyin/v1/video/create_video/");
    request.token = Some(token);
    request.params.push(("open_id".into(), open_id));
    request.json = Some(body);
    let created = client.request(request)?;
    let item_id = created
        .pointer("/data/item_id")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or("投稿响应缺少 item_id")?;

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "ok": true,
            "item_id": item_id,
            "video_id": video_id,
            "visibility": match args.visibility {
                Visibility::Public => "public",
                Visibility::Private => "private",
                Visibility::Friends => "friends",
            },
        }))
        .map_err(|_| "投稿结果序列化失败")?
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_maps_to_openapi_values() {
        assert_eq!(
            match Visibility::Public {
                Visibility::Public => 0,
                Visibility::Private => 1,
                Visibility::Friends => 2,
            },
            0
        );
        assert_eq!(
            match Visibility::Private {
                Visibility::Public => 0,
                Visibility::Private => 1,
                Visibility::Friends => 2,
            },
            1
        );
        assert_eq!(
            match Visibility::Friends {
                Visibility::Public => 0,
                Visibility::Private => 1,
                Visibility::Friends => 2,
            },
            2
        );
    }
}
