//! Provider adapter implementation entry: resolves the adapter by channel type + shared helpers.
//!
//! OpenAI / DeepSeek / Custom share the OpenAI-compatible passthrough implementation (openai.rs); Claude / Gemini
//! do protocol conversion (claude.rs / gemini.rs). Conversion logic is confined to this directory (see Spec decision 6).

pub mod claude;
pub mod gemini;
pub mod openai;
#[cfg(test)]
pub(crate) mod test_util;

use serde_json::{Value, json};

use crate::domain::channel::ChannelType;
use crate::domain::provider::{ProviderAdaptor, ProviderError, StreamEvent, TokenUsage};

/// Returns the adapter instance for the channel type (Box<dyn> hides the protocol differences).
pub fn adaptor_for(channel_type: ChannelType) -> Box<dyn ProviderAdaptor> {
    match channel_type {
        ChannelType::OpenAi | ChannelType::DeepSeek | ChannelType::Custom => {
            Box::new(openai::OpenAiCompatibleAdaptor::new(channel_type))
        }
        ChannelType::Claude => Box::new(claude::ClaudeAdaptor::new()),
        ChannelType::Gemini => Box::new(gemini::GeminiAdaptor::new()),
    }
}

/// Resolves the forwarding Base URL: explicit channel config wins, otherwise falls back to the adapter
/// default; if neither is present, reports a config error.
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

/// The channel must configure an upstream key; missing it is a config error (the upstream key is never
/// relayed in responses or written to logs).
pub(super) fn require_api_key(
    channel: &crate::domain::channel::Channel,
) -> Result<&str, ProviderError> {
    channel
        .api_key
        .as_deref()
        .ok_or_else(|| ProviderError::NotConfigured("api key is required".into()))
}

/// Extracts plain text from the `content` field of an OpenAI request: a string is taken as-is,
/// text blocks are joined in order. Used for conversion to Anthropic / Gemini (both use text
/// as their primary carrier).
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

/// Builds a JSON object from the `usage` value object (OpenAI-compatible usage field), so streaming
/// chunks can carry usage.
pub(super) fn usage_to_json(u: TokenUsage) -> Value {
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

/// Builds an OpenAI `chat.completion.chunk` frame (delta is an empty object when content is empty).
/// Shared by Claude / Gemini conversion.
pub(super) fn chunk_event(
    id: &str,
    model: &str,
    created: i64,
    content: &str,
    usage: Option<TokenUsage>,
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

/// Builds a stream-end frame: empty delta + given finish_reason. Shared by Claude / Gemini conversion.
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

/// Builds an upstream error response body (non-2xx): replaces the upstream text with a generic error JSON —
/// OpenAI-compatible endpoints echo the submitted key in 401 error bodies (`Incorrect API key provided: sk-...`),
/// so relaying verbatim would leak it (red line: upstream keys must not be exposed downstream). Keeps the status code, drops the body.
pub(super) fn upstream_error_body(status_code: u16) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "error": {
            "message": format!("upstream request failed with status {status_code}"),
            "type": "upstream_error",
        }
    }))
    .unwrap_or_default()
}

/// Builds an upstream error SSE frame: `data: {error JSON}\n\n`. The error body, as above, does not
/// carry upstream text, and the frame follows the `data:` line format so downstream SSE parsers can consume it.
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
