use clap::{CommandFactory, Parser, Subcommand};
use douyin_cli::adb::Adb;

#[derive(Parser)]
#[command(version, about = "抖音命令行工具")]
struct Cli {
    #[command(flatten)]
    crawl: douyin_cli::crawler::CrawlArgs,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 直播权限、房间管理、主播状态上报与 OBS 推流
    Live(douyin_cli::live::LiveArgs),
    #[command(name = "__phone-worker", hide = true)]
    PhoneWorker,
    /// 采集作品评论与楼中楼回复
    Comment(douyin_cli::comments::CommentArgs),
    /// 安卓账号写操作：赞、关、评、藏（需 --yes）
    Social(douyin_cli::social::SocialArgs),
    /// 私信：Aweme REST 辅助接口 + imapi protobuf 收发
    Im(douyin_cli::im::ImArgs),
    /// 通过 stdio 或 Streamable HTTP 启动 MCP，支持事件 Webhook
    Mcp(douyin_cli::mcp_http::McpArgs),
    /// 调用官方 OpenAPI，需要开放平台应用授权
    Api(douyin_cli::api::ApiArgs),
    /// 使用官方 OAuth 上传并发布视频
    Publish(douyin_cli::publish::PublishArgs),
    /// 下载抖音 CDN 媒体 URL，不向媒体主机发送登录凭据
    Download {
        url: String,
        #[arg(short, long)]
        output: std::path::PathBuf,
    },
    /// 读取当前登录账号的完整个人资料
    Me,
    /// 离线分析热词、热梗候选与需求线索
    Insights(douyin_cli::insights::InsightsArgs),
    /// 离线统计作品元数据
    Stats(douyin_cli::stats::StatsArgs),
    /// 从已连接手机导入登录态，或检查、清除本地登录态
    Auth {
        #[arg(long)]
        adb: bool,
        #[arg(long, env = "ANDROID_SERIAL")]
        serial: Option<String>,
        #[command(subcommand)]
        command: Option<AuthCommand>,
    },
    /// 检查 ADB 设备与抖音安装状态
    Device {
        /// 多设备时指定序列号
        #[arg(long, env = "ANDROID_SERIAL")]
        serial: Option<String>,
    },
}

#[derive(Subcommand)]
enum AuthCommand {
    /// 保存独立的网页 Cookie，仅校验格式
    CookieLogin(douyin_cli::cookie::LoginArgs),
    /// 检查网页 Cookie；--offline 不访问服务端
    CookieStatus {
        #[arg(long)]
        offline: bool,
    },
    /// 清除网页 Cookie，保留设备登录和 OAuth
    CookieLogout,
    /// 通过官方 OAuth 授权接入账号
    Login(douyin_cli::oauth::LoginArgs),
    /// 使用官方授权 code 换取并保存 token
    Code {
        #[arg(long)]
        code: String,
        #[arg(long, env = "DOUYIN_CLIENT_SECRET", hide_env_values = true)]
        client_secret: Option<String>,
    },
    /// 刷新官方 access_token
    Refresh,
    /// 重新向服务端验证当前账号；--offline 仅检查本地配置
    Status {
        #[arg(long)]
        offline: bool,
        #[arg(long)]
        json: bool,
        /// 检查官方 OAuth，默认检查 ADB 导入的登录态
        #[arg(long)]
        oauth: bool,
    },
    /// 删除本地设备登录态
    Logout,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("错误: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let command = match cli.command {
        Some(command) => command,
        None if cli.crawl.should_run() => return douyin_cli::crawler::run(cli.crawl),
        None => {
            Cli::command()
                .print_long_help()
                .map_err(|_| "无法输出帮助")?;
            println!();
            return Ok(());
        }
    };
    match command {
        Command::Live(args) => douyin_cli::live::run(args)?,
        Command::PhoneWorker => douyin_cli::phone::worker()?,
        Command::Comment(args) => douyin_cli::comments::run(args)?,
        Command::Social(args) => douyin_cli::social::run(args)?,
        Command::Im(args) => douyin_cli::im::run(args)?,
        Command::Mcp(args) => douyin_cli::mcp_http::run(args)?,
        Command::Api(args) => douyin_cli::api::run(args)?,
        Command::Publish(args) => douyin_cli::publish::run(args)?,
        Command::Download { url, output } => douyin_cli::download::url(&url, &output)?,
        Command::Me => {
            let value = douyin_cli::net::NativeClient::saved()?.me()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&value).map_err(|_| "个人资料序列化失败")?
            );
        }
        Command::Insights(args) => douyin_cli::insights::run(args)?,
        Command::Stats(args) => douyin_cli::stats::run(args)?,
        Command::Auth {
            adb,
            serial,
            command,
        } => {
            let result = match (adb, command) {
                (true, None) => douyin_cli::auth::login_adb(serial.as_deref())?,
                (false, Some(AuthCommand::CookieLogin(args))) => douyin_cli::cookie::login(args)?,
                (false, Some(AuthCommand::CookieStatus { offline })) => {
                    douyin_cli::cookie::status(offline)?
                }
                (false, Some(AuthCommand::CookieLogout)) => douyin_cli::cookie::logout()?,
                (false, Some(AuthCommand::Login(args))) => douyin_cli::oauth::login(args)?,
                (
                    false,
                    Some(AuthCommand::Code {
                        code,
                        client_secret,
                    }),
                ) => douyin_cli::oauth::code(&code, client_secret)?,
                (false, Some(AuthCommand::Refresh)) => douyin_cli::oauth::refresh()?,
                (false, Some(AuthCommand::Status { offline, oauth, .. })) => {
                    if oauth || !douyin_cli::store::session_file().exists() {
                        douyin_cli::oauth::status(offline)?
                    } else {
                        douyin_cli::auth::status(offline)?
                    }
                }
                (false, Some(AuthCommand::Logout)) => {
                    douyin_cli::store::logout()?;
                    douyin_cli::oauth::logout()?;
                    serde_json::json!({"loggedOut":true})
                }
                _ => return Err("请使用 auth --adb、auth status 或 auth logout".into()),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&result).map_err(|e| e.to_string())?
            );
        }
        Command::Device { serial } => {
            let adb = Adb::connect(serial.as_deref())?;
            let info = adb.inspect()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&info).map_err(|e| e.to_string())?
            );
        }
    }
    Ok(())
}
