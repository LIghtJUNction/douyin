//! Explicit publishing intent. Unsupported settings never disappear silently.
use std::path::PathBuf;

use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Args, Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Options {
    /// 要求平台“内容由 AI 生成”声明；当前后端缺少已验证字段，会在上传前停止
    #[arg(long)]
    pub ai_generated: bool,
    /// 本地 JPG 自定义封面；仅官方 OAuth 投稿支持
    #[arg(long, value_name = "JPG")]
    pub cover: Option<PathBuf>,
    /// 独立短标题；当前没有已验证的非空字段协议，会在上传前停止
    #[arg(long)]
    pub short_title: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Backend {
    OpenApi,
    Native,
}

impl Backend {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::OpenApi => "openapi",
            Self::Native => "native",
        }
    }
}

impl Options {
    pub(crate) fn validate(&self, backend: Backend) -> Result<(), String> {
        let mut errors = Vec::new();
        if self.ai_generated {
            errors.push(
                "--ai-generated：当前后端没有经过验证的平台 AI 声明字段；不会以文案代替平台标识",
            );
        }
        if self.cover.is_some() && backend == Backend::Native {
            errors.push(
                "--cover：原生后端尚无经过验证的本地 JPG 上传协议；不会改用视频快照冒充自定义封面",
            );
        }
        if self.short_title.is_some() {
            errors.push(
                "--short-title：当前后端没有经过验证的非空短标题协议；不会丢弃短标题或并入文案",
            );
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "阶段 options_check：{}；尚未上传或创建作品",
                errors.join("；")
            ))
        }
    }

    /// `encoded` means included in the prepared create body, never platform
    /// acceptance. Preflight has no uploaded image ID and therefore says false.
    pub(crate) fn report(&self, backend: Backend, image_id: Option<&str>) -> Value {
        let cover_supported = backend == Backend::OpenApi;
        json!({
            "ai_generated": {
                "requested": self.ai_generated,
                "supported": false,
                "encoded": false,
                "platform_verified": false,
                "reason": "no_verified_platform_declaration_parameter"
            },
            "custom_cover": {
                "requested": self.cover.is_some(),
                "supported": cover_supported,
                "encoded": cover_supported && self.cover.is_some() && image_id.is_some(),
                "requires_upload": cover_supported && self.cover.is_some() && image_id.is_none(),
                "request_field": if cover_supported { Some("custom_cover_image_url") } else { None },
                "platform_verified": false
            },
            "short_title": {
                "requested": self.short_title.is_some(),
                "supported": false,
                "encoded": false,
                "platform_verified": false,
                "reason": "no_verified_nonempty_short_title_contract"
            }
        })
    }

    pub(crate) fn blocked_preflight(&self, backend: Backend) -> Option<Value> {
        self.validate(backend).err().map(|reason| {
            json!({
                "ok": false, "dry_run": true, "stage": "options_check",
                "backend": backend.name(), "reason": reason,
                "settings": self.report(backend, None),
                "uploaded": false, "submitted": false, "create_sent": false
            })
        })
    }
}
