use clap::{Args, CommandFactory, Parser, Subcommand};
use serde_json::{Value, json};

use crate::{api, auth, comments, crawler, mcp, obscura, settings, subtitles};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
#[command(name = "douyin", version, about = "抖音 CLI（Rust）")]
#[command(long_about = "通用抖音命令行工具的 Rust 实现。更多命令见：douyin COMMAND --help")]
struct Cli {
    #[command(flatten)]
    crawl: crawler::CrawlArgs,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 调用抖音开放平台官方 OpenAPI
    Api(api::ApiArgs),
    /// 管理授权
    Auth(auth::AuthArgs),
    /// 抓取作品评论区
    Comment(comments::CommentArgs),
    /// 通过 stdio 启动抖音 MCP 服务器
    Mcp,
    /// Obscura 集成辅助命令
    Obscura(ObscuraArgs),
    /// 从本地视频/音频生成字幕
    Subtitle(subtitles::SubtitleArgs),
}

#[derive(Debug, Args)]
struct ObscuraArgs {
    #[command(subcommand)]
    command: ObscuraCommand,
}

#[derive(Debug, Subcommand)]
enum ObscuraCommand {
    /// 输出 Obscura 集成 manifest
    Manifest,
    /// 检查本地 Obscura 集成状态
    Status {
        /// Obscura 可执行文件名
        #[arg(long, default_value = "obscura")]
        binary: String,
    },
}

pub fn run() -> Result<(), String> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Api(args)) => api::run(args),
        Some(Command::Auth(args)) => auth::run(args),
        Some(Command::Comment(args)) => comments::run(args),
        Some(Command::Mcp) => mcp::run_stdio(),
        Some(Command::Obscura(args)) => run_obscura(args.command),
        Some(Command::Subtitle(args)) => subtitles::run(args),
        None if cli.crawl.should_run() => crawler::run(cli.crawl),
        None => {
            Cli::command()
                .print_long_help()
                .map_err(|error| error.to_string())?;
            println!();
            Ok(())
        }
    }
}

fn run_obscura(command: ObscuraCommand) -> Result<(), String> {
    match command {
        ObscuraCommand::Manifest => {
            print_json(&obscura::manifest(VERSION, &settings::settings_file()))
        }
        ObscuraCommand::Status { binary } => {
            let data = settings::load().map_err(|error| error.to_string())?;
            let openapi = settings::openapi(&data);
            let authorized = !string_value(&openapi, "accessToken").is_empty()
                && !string_value(&openapi, "openId").is_empty();
            print_json(&json!({
                "douyin": {
                    "version": VERSION,
                    "entrypoint": "douyin",
                    "configFile": settings::settings_file(),
                    "authorized": authorized
                },
                "obscura": obscura::status(&binary),
                "next": {
                    "auth": ["douyin", "auth", "login"],
                    "machineStatus": ["douyin", "auth", "status", "--json"],
                    "manifest": ["douyin", "obscura", "manifest"]
                }
            }))
        }
    }
}

fn string_value<'a>(values: &'a serde_json::Map<String, Value>, key: &str) -> &'a str {
    values.get(key).and_then(Value::as_str).unwrap_or("")
}

fn print_json(value: &Value) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );
    Ok(())
}
