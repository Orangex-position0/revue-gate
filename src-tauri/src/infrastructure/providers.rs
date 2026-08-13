//! 供应商适配器实现入口：按渠道类型解析适配器 + 各适配器共享的小工具。
//!
//! OpenAI / DeepSeek / Custom 共用 OpenAI-compatible 直通实现（openai.rs），Claude / Gemini
//! 做协议转换（claude.rs / gemini.rs）。协议转换逻辑限定在本目录（见 Spec 决策 6）。

pub mod claude;
pub mod gemini;
pub mod openai;
#[cfg(test)]
pub(crate) mod test_util;

use serde_json::{Value, json};

use crate::domain::channel::ChannelType;
use crate::domain::provider::{ProviderAdaptor, ProviderError, StreamEvent, Usage};

/// 按渠道类型返回对应的适配器实例（Box<dyn> 隐藏具体协议差异）。
pub fn adaptor_for(channel_type: ChannelType) -> Box<dyn ProviderAdaptor> {
    match channel_type {
        ChannelType::OpenAi | ChannelType::DeepSeek | ChannelType::Custom => {
            Box::new(openai::OpenAiCompatibleAdaptor::new(channel_type))
        }
        ChannelType::Claude => Box::new(claude::ClaudeAdaptor::new()),
        ChannelType::Gemini => Box::new(gemini::GeminiAdaptor::new()),
    }
}

/// 解析转发目标 Base URL：渠道显式配置优先，否则回退适配器默认值；两者皆无报配置错误。
pub(super) fn resolve_base_url(
    channel: &crate::domain::channel::Channel,
    default: Option<&'static str>,
) -> Result<String, ProviderError> {
    match channel.base_url.as_deref() {
        Some(url) if !url.trim().is_empty() => Ok(url.trim().to_string()),
        _ => default
            .map(str::to_string)
            .ok_or_else(|| ProviderError::NotConfigured("base url is required".into())),
    }
}

/// 渠道必须配置上游密钥；缺失报配置错误（上游密钥绝不随响应下发或写日志）。
pub(super) fn require_api_key(
    channel: &crate::domain::channel::Channel,
) -> Result<&str, ProviderError> {
    channel
        .api_key
        .as_deref()
        .ok_or_else(|| ProviderError::NotConfigured("api key is required".into()))
}

/// 从 OpenAI 请求的 `content` 字段提取纯文本：字符串直接取，text 块按序拼接。
/// 用于转换到 Anthropic / Gemini（两者均以文本为主要载体）。
pub(super) fn extract_text_content(content: Option<&Value>) -> Option<String> {
    match content {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let texts: Vec<String> = blocks
                .iter()
                .filter_map(|b| {
                    if b.get("type").and_then(Value::as_str) == Some("text") {
                        b.get("text").and_then(Value::as_str).map(str::to_string)
                    } else {
                        None
                    }
                })
                .collect();
            if texts.is_empty() {
                None
            } else {
                Some(texts.join("\n"))
            }
        }
        _ => None,
    }
}

/// 从 `usage` 值对象构造 JSON 对象（OpenAI 兼容 usage 字段），用于流式 chunk 携带用量。
pub(super) fn usage_to_json(u: Usage) -> Value {
    let mut v = json!({});
    if let Some(p) = u.prompt_tokens {
        v["prompt_tokens"] = json!(p);
    }
    if let Some(c) = u.completion_tokens {
        v["completion_tokens"] = json!(c);
    }
    if let Some(t) = u.total_tokens {
        v["total_tokens"] = json!(t);
    }
    v
}

/// 构造 OpenAI `chat.completion.chunk` 帧（content 为空时 delta 为空对象）。Claude / Gemini 转换共用。
pub(super) fn chunk_event(
    id: &str,
    model: &str,
    created: i64,
    content: &str,
    usage: Option<Usage>,
) -> StreamEvent {
    let mut chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": if content.is_empty() { json!({}) } else { json!({"content": content}) },
            "finish_reason": null,
        }],
    });
    if let Some(u) = usage {
        chunk["usage"] = usage_to_json(u);
    }
    StreamEvent {
        data: format!("data: {chunk}\n\n").into_bytes(),
        usage,
    }
}

/// 构造流结束帧：空 delta + 指定 finish_reason。Claude / Gemini 转换共用。
pub(super) fn finish_event(
    id: &str,
    model: &str,
    created: i64,
    finish_reason: &str,
) -> StreamEvent {
    let chunk = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{"index": 0, "delta": {}, "finish_reason": finish_reason}],
    });
    StreamEvent {
        data: format!("data: {chunk}\n\n").into_bytes(),
        usage: None,
    }
}

/// 构造上游错误响应体（非 2xx）：用通用错误 JSON 替换上游原文——OpenAI 兼容端点在 401
/// 错误体会回显所提交的密钥（`Incorrect API key provided: sk-...`），原样透传即泄漏
/// （红线：上游密钥不暴露给下游）。保留状态码，丢弃正文。
pub(super) fn upstream_error_body(status_code: u16) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "error": {
            "message": format!("upstream request failed with status {status_code}"),
            "type": "upstream_error",
        }
    }))
    .unwrap_or_default()
}

/// 构造上游错误 SSE 帧：`data: {错误 JSON}\n\n`。错误体同上不携带上游原文，
/// 且帧符合 `data:` 行格式，下游 SSE 解析器可正常消费。
pub(super) fn upstream_error_event(status_code: u16) -> StreamEvent {
    StreamEvent {
        data: format!(
            "data: {}\n\n",
            String::from_utf8_lossy(&upstream_error_body(status_code))
        )
        .into_bytes(),
        usage: None,
    }
}
