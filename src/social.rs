//! Android social write APIs recovered from the pinned 40.8.0 Retrofit definitions.
//! Parameter names come from APK annotations; do not invent undocumented fields.

use clap::{Args, Subcommand};
use serde_json::{Value, json};

use crate::net::NativeClient;

#[derive(Args)]
pub struct SocialArgs {
    #[command(subcommand)]
    command: SocialCommand,
}

#[derive(Subcommand)]
enum SocialCommand {
    /// 对作品点赞或取消点赞（GET /aweme/v1/commit/item/digg/）
    Digg {
        aweme_id: String,
        /// 1 点赞，0 取消；与 APK diggItem type 一致
        #[arg(long, default_value_t = 1)]
        action: u8,
        #[arg(long, default_value_t = 0)]
        channel_id: i32,
        #[arg(long)]
        yes: bool,
    },
    /// 关注或取消关注用户（GET /aweme/v1/commit/follow/user/）
    Follow {
        /// 目标用户 ID；可与 --sec-user-id 同时提供
        #[arg(long)]
        user_id: Option<String>,
        #[arg(long)]
        sec_user_id: Option<String>,
        /// 1 关注，0 取消；与 APK follow type 一致
        #[arg(long, default_value_t = 1)]
        action: u8,
        /// 默认取自 40.8.0 RedPacketFollowViewModel 调用点
        #[arg(long, default_value_t = 191)]
        from: i32,
        #[arg(long, default_value_t = 27)]
        from_pre: i32,
        #[arg(long, default_value_t = 0)]
        from_action: i32,
        /// 默认取自同一调用点的 channel_id 常量 0x00049446
        #[arg(long, default_value_t = 0x0004_9446)]
        channel_id: i32,
        #[arg(long)]
        yes: bool,
    },
    /// 发布作品评论（POST /aweme/v1/comment/publish/）
    Comment {
        aweme_id: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        reply_id: Option<String>,
        #[arg(long, default_value_t = 0)]
        channel_id: i32,
        /// 通过已连接的 rooted 手机为此路径生成 TicketGuard 请求头
        #[arg(long)]
        adb_guard: bool,
        #[arg(long)]
        yes: bool,
    },
    /// 点赞或取消点赞评论（GET /aweme/v1/comment/digg/）
    CommentDigg {
        comment_id: String,
        aweme_id: String,
        /// 1 点赞，0 取消点赞
        #[arg(long, default_value_t = 1)]
        action: u8,
        #[arg(long, default_value_t = 0)]
        channel_id: i32,
        #[arg(long)]
        yes: bool,
    },
    /// 点踩或取消点踩评论（GET /aweme/v1/comment/digg/）
    CommentDislike {
        comment_id: String,
        aweme_id: String,
        /// 1 点踩，0 取消点踩
        #[arg(long, default_value_t = 1)]
        action: u8,
        #[arg(long, default_value_t = 0)]
        channel_id: i32,
        #[arg(long)]
        yes: bool,
    },
    /// 读取作品的分享链接，不发送给联系人
    Share { aweme_id: String },
    /// 收藏或取消收藏作品（POST /aweme/v1/aweme/collect/）
    Collect {
        aweme_id: String,
        /// 1 收藏，0 取消；与 APK collect action 一致
        #[arg(long, default_value_t = 1)]
        action: u8,
        #[arg(long)]
        yes: bool,
    },
}

pub fn run(args: SocialArgs) -> Result<(), String> {
    let client = NativeClient::saved()?;
    let value = match args.command {
        SocialCommand::Digg {
            aweme_id,
            action,
            channel_id,
            yes,
        } => {
            require_yes(yes)?;
            digg(&client, &aweme_id, action, channel_id)?
        }
        SocialCommand::Follow {
            user_id,
            sec_user_id,
            action,
            from,
            from_pre,
            from_action,
            channel_id,
            yes,
        } => {
            require_yes(yes)?;
            follow(
                &client,
                user_id.as_deref(),
                sec_user_id.as_deref(),
                action,
                from,
                from_pre,
                from_action,
                channel_id,
            )?
        }
        SocialCommand::Comment {
            aweme_id,
            text,
            reply_id,
            channel_id,
            adb_guard,
            yes,
        } => {
            require_yes(yes)?;
            publish_comment(
                &client,
                &aweme_id,
                &text,
                reply_id.as_deref(),
                channel_id,
                adb_guard,
            )?
        }
        SocialCommand::CommentDigg {
            comment_id,
            aweme_id,
            action,
            channel_id,
            yes,
        } => {
            require_yes(yes)?;
            comment_digg(&client, &comment_id, &aweme_id, action, channel_id)?
        }
        SocialCommand::CommentDislike {
            comment_id,
            aweme_id,
            action,
            channel_id,
            yes,
        } => {
            require_yes(yes)?;
            comment_dislike(&client, &comment_id, &aweme_id, action, channel_id)?
        }
        SocialCommand::Share { aweme_id } => {
            let value = share(&client, &aweme_id)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&value).map_err(|_| "分享结果序列化失败")?
            );
            return Ok(());
        }
        SocialCommand::Collect {
            aweme_id,
            action,
            yes,
        } => {
            require_yes(yes)?;
            collect(&client, &aweme_id, action)?
        }
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&summary(&value)?).map_err(|_| "写操作结果序列化失败")?
    );
    Ok(())
}

fn require_yes(yes: bool) -> Result<(), String> {
    if yes {
        Ok(())
    } else {
        Err("写操作会修改账号状态，需要 --yes".into())
    }
}

fn require_aweme_id(aweme_id: &str) -> Result<(), String> {
    if aweme_id.is_empty() || !aweme_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("作品 ID 必须是正整数字符串".into());
    }
    Ok(())
}

fn require_action01(action: u8) -> Result<(), String> {
    if action <= 1 {
        Ok(())
    } else {
        Err("action 只能是 0 或 1".into())
    }
}

/// Minimal diggItem query set from classes31 `X/142V.diggItem`.
pub(crate) fn digg(
    client: &NativeClient,
    aweme_id: &str,
    action: u8,
    channel_id: i32,
) -> Result<Value, String> {
    require_aweme_id(aweme_id)?;
    require_action01(action)?;
    client.get(
        "/aweme/v1/commit/item/digg/",
        &[
            ("aweme_id".into(), aweme_id.into()),
            ("type".into(), action.to_string()),
            ("channel_id".into(), channel_id.to_string()),
            ("filter_warn".into(), "0".into()),
            ("is_commerce".into(), "0".into()),
        ],
    )
}

/// Prefer sec_user_id path from classes36 `commitFollowUser`; fall back to user_id.
#[allow(clippy::too_many_arguments)]
pub(crate) fn follow(
    client: &NativeClient,
    user_id: Option<&str>,
    sec_user_id: Option<&str>,
    action: u8,
    from: i32,
    from_pre: i32,
    from_action: i32,
    channel_id: i32,
) -> Result<Value, String> {
    require_action01(action)?;
    let mut params = vec![
        ("type".into(), action.to_string()),
        ("from".into(), from.to_string()),
        ("from_pre".into(), from_pre.to_string()),
        ("from_action".into(), from_action.to_string()),
        ("channel_id".into(), channel_id.to_string()),
    ];
    match (user_id, sec_user_id) {
        (None, None) => return Err("关注需要 --user-id 或 --sec-user-id".into()),
        (Some(uid), None) => {
            if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
                return Err("user_id 必须是正整数字符串".into());
            }
            params.insert(0, ("user_id".into(), uid.into()));
        }
        (None, Some(sec)) => {
            if sec.trim().is_empty() {
                return Err("sec_user_id 不能为空".into());
            }
            params.insert(0, ("sec_user_id".into(), sec.into()));
        }
        (Some(uid), Some(sec)) => {
            if uid.is_empty() || !uid.bytes().all(|b| b.is_ascii_digit()) {
                return Err("user_id 必须是正整数字符串".into());
            }
            if sec.trim().is_empty() {
                return Err("sec_user_id 不能为空".into());
            }
            // IFeedLiveShareProfileApi.followUserAction shape.
            params.insert(0, ("user_id".into(), uid.into()));
            params.insert(1, ("sec_user_id".into(), sec.into()));
        }
    }
    client.get("/aweme/v1/commit/follow/user/", &params)
}

/// CommentRealApi.LJII required Field subset; optional Fields omitted rather than guessed.
pub(crate) fn publish_comment(
    client: &NativeClient,
    aweme_id: &str,
    text: &str,
    reply_id: Option<&str>,
    channel_id: i32,
    adb_guard: bool,
) -> Result<Value, String> {
    require_aweme_id(aweme_id)?;
    let text = text.trim();
    if text.is_empty() || text.len() > 2048 {
        return Err("评论内容不能为空且不超过 2048 字节".into());
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err("评论内容含控制字符".into());
    }
    // CommentRealApi.LJII Field subset actually sent for plain text comments.
    // unique_key / enter_from appear in call sites; Query need_shark_async is on the same method.
    let unique_key = format!(
        "{}-{}",
        aweme_id,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let mut form = vec![
        ("aweme_id".into(), aweme_id.into()),
        ("text".into(), text.into()),
        ("channel_id".into(), channel_id.to_string()),
        ("is_self_see".into(), "0".into()),
        ("action_type".into(), "0".into()),
        ("enter_from".into(), "homepage_hot".into()),
        ("publish_scene".into(), "0".into()),
        ("need_risk_check".into(), "0".into()),
        ("is_commerce".into(), "0".into()),
        ("unique_key".into(), unique_key),
        ("text_extra".into(), "[]".into()),
    ];
    if let Some(reply_id) = reply_id {
        if reply_id.is_empty() || !reply_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("reply_id 必须是正整数字符串".into());
        }
        form.push(("reply_id".into(), reply_id.into()));
    }
    const PATH: &str = "/aweme/v1/comment/publish/";
    let query = [("need_shark_async".into(), "0".into())];
    if adb_guard {
        let headers = crate::ticket_zt::for_path(PATH, None)?.http_headers();
        client.post_with_query_headers(PATH, &query, &form, &headers)
    } else {
        client.post_with_query(PATH, &query, &form)
    }
}

fn comment_reaction(
    client: &NativeClient,
    comment_id: &str,
    aweme_id: &str,
    digg_type: u8,
    channel_id: i32,
) -> Result<Value, String> {
    if comment_id.is_empty() || !comment_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("评论 ID 必须是正整数字符串".into());
    }
    require_aweme_id(aweme_id)?;
    client.get(
        "/aweme/v1/comment/digg/",
        &[
            ("cid".into(), comment_id.into()),
            ("aweme_id".into(), aweme_id.into()),
            ("digg_type".into(), digg_type.to_string()),
            ("channel_id".into(), channel_id.to_string()),
        ],
    )
}

pub(crate) fn comment_digg(
    client: &NativeClient,
    comment_id: &str,
    aweme_id: &str,
    action: u8,
    channel_id: i32,
) -> Result<Value, String> {
    require_action01(action)?;
    comment_reaction(
        client,
        comment_id,
        aweme_id,
        comment_digg_type(action),
        channel_id,
    )
}

pub(crate) fn comment_dislike(
    client: &NativeClient,
    comment_id: &str,
    aweme_id: &str,
    action: u8,
    channel_id: i32,
) -> Result<Value, String> {
    require_action01(action)?;
    comment_reaction(
        client,
        comment_id,
        aweme_id,
        comment_dislike_type(action),
        channel_id,
    )
}

fn comment_digg_type(action: u8) -> u8 {
    if action == 1 { 1 } else { 2 }
}

fn comment_dislike_type(action: u8) -> u8 {
    if action == 1 { 4 } else { 3 }
}

fn share(client: &NativeClient, aweme_id: &str) -> Result<Value, String> {
    require_aweme_id(aweme_id)?;
    let item = client.aweme(aweme_id)?;
    let share_url = item
        .pointer("/share_info/share_url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .ok_or("作品响应缺少分享链接")?;
    let parsed = url::Url::parse(share_url).map_err(|_| "分享链接无效")?;
    if parsed.scheme() != "https"
        || !matches!(
            parsed.host_str(),
            Some("www.iesdouyin.com" | "www.douyin.com")
        )
    {
        return Err("分享链接不属于受支持的抖音 HTTPS 主机".into());
    }
    Ok(json!({"aweme_id": aweme_id, "share_url": share_url}))
}

/// classes9 POST collect with aweme_id + action.
pub(crate) fn collect(client: &NativeClient, aweme_id: &str, action: u8) -> Result<Value, String> {
    require_aweme_id(aweme_id)?;
    require_action01(action)?;
    client.post(
        "/aweme/v1/aweme/collect/",
        &[
            ("aweme_id".into(), aweme_id.into()),
            ("action".into(), action.to_string()),
        ],
    )
}

fn summary(value: &Value) -> Result<Value, String> {
    let status = value
        .get("status_code")
        .and_then(Value::as_i64)
        .ok_or("写操作响应缺少 status_code")?;
    let comment_id = value
        .pointer("/comment/cid")
        .or_else(|| value.pointer("/comment/id"))
        .or_else(|| value.get("cid"))
        .or_else(|| value.get("comment_id"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "status_code": status,
        "status_msg": value.get("status_msg").cloned().unwrap_or(Value::Null),
        "ok": status == 0,
        "comment_id": comment_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_binary_action_and_empty_targets() {
        assert!(require_action01(2).is_err());
        assert!(require_aweme_id("").is_err());
        assert!(require_aweme_id("12a").is_err());
        assert!(require_aweme_id("7698380132884519355").is_ok());
    }

    #[test]
    fn summary_requires_status_code() {
        assert!(summary(&json!({})).is_err());
        let ok = summary(&json!({"status_code":0,"status_msg":"ok"})).unwrap();
        assert_eq!(ok["ok"], true);
        let comment = summary(&json!({"status_code":0,"comment":{"cid":"123"}})).unwrap();
        assert_eq!(comment["comment_id"], "123");
    }

    #[test]
    fn comment_reaction_types_match_apk_and_server_acceptance() {
        assert_eq!(comment_digg_type(1), 1);
        assert_eq!(comment_digg_type(0), 2);
        assert_eq!(comment_dislike_type(1), 4);
        assert_eq!(comment_dislike_type(0), 3);
    }
}
