//! 客户端 ↔ 编排层 协议消息(对应 docs/protocol-draft.md,P0)。
//! 文本帧 = 控制/事件 JSON;二进制帧 = 音频(16k PCM s16le)。

use serde::{Deserialize, Serialize};

/// 客户端 → 服务端:连接后第一帧。
/// `protocol`/`sample_rate`/`format`/`language` 是协议契约字段(客户端必发,
/// 见 protocol-draft.md),编排层目前只用 `want_*`;保留以备 language 路由等。
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct Hello {
    pub protocol: String,
    pub sample_rate: u32, // 固定 16000
    pub format: String,   // "pcm_s16le"
    pub language: String, // "zh" / "auto" / "en" ...(透传给 ASR 路由)
    pub want_optimize: bool,
    pub want_translate: bool,
}

/// 客户端 → 服务端:控制帧(stop / reset)。
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientControl {
    Stop,
    Reset,
}

/// 服务端 → 客户端:事件(均 JSON 文本帧)。`type` 标签区分。
#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    Ready { session_id: String },
    Segment {
        id: u64,
        text: String,
        t_start: Option<f32>,
        t_end: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        speaker: Option<String>,
    },
    Optimized { r#ref: u64, text: String },
    Translated { r#ref: u64, text: String },
    Error { code: String, message: String, fatal: bool },
    Done { session_id: String },
}

impl ServerEvent {
    pub fn json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| {
            r#"{"type":"error","code":"enc","message":"serialize failed","fatal":true}"#
                .to_string()
        })
    }
}
