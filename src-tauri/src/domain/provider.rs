//! ProviderAdaptor trait: isolates protocol differences across providers (see docs/Architecture-backend.md).
//!
//! The domain layer defines only the contract and pure types: OpenAI / DeepSeek / Custom use OpenAI-compatible passthrough,
//! Claude / Gemini do protocol conversion; implementations all live in infrastructure/providers (conversion logic never leaves that directory).
//! Adapter methods return domain types and do not expose technical types like reqwest/axum; `forward_stream` returns
//! an OpenAI-compatible SSE byte stream that the data plane only relays. `futures-util` is just the stream abstraction, not a technical dependency (see Spec decision 6).

pub use futures_util::stream::BoxStream;
use serde_json::Value;

use crate::domain::channel::{Channel, ChannelType};

/// usage: token statistics for one upstream call. Field names differ per provider (OpenAI `usage` / Claude `input|output_tokens`
/// / Gemini `usageMetadata`); adapters normalize them into this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

impl Usage {
    /// Derives total from prompt + completion when total is missing (some providers do not return total).
    pub fn normalized(self) -> Usage {
        let total_tokens = self.total_tokens.or_else(|| {
            Some(self.prompt_tokens.unwrap_or(0) + self.completion_tokens.unwrap_or(0))
        });
        Usage {
            total_tokens,
            ..self
        }
    }

    /// Merge two usage values: each field accumulates as "sum when both sides have it, otherwise keep the side that has it" (saturating addition prevents overflow).
    /// Used for per-frame usage aggregation of streaming responses (OpenAI-compatible streams usually carry usage only in the last frame, so the aggregate equals taking the last frame).
    pub fn accumulate(self, other: Usage) -> Usage {
        Usage {
            prompt_tokens: opt_add(self.prompt_tokens, other.prompt_tokens),
            completion_tokens: opt_add(self.completion_tokens, other.completion_tokens),
            total_tokens: opt_add(self.total_tokens, other.total_tokens),
        }
    }
}

/// Saturating sum when both sides have a value; keep the present side when only one has it; None when neither does.
fn opt_add(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.saturating_add(y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

/// Request forwarded upstream: the proxy has applied model mapping and rewritten `body["model"]` (see stage 07 forwarding use case).
/// `model` / `stream` stay consistent with the body so adapters can build the target protocol request.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    /// Upstream model name after model mapping is applied.
    pub model: String,
    /// Whether the request is streaming.
    pub stream: bool,
    /// Raw OpenAI-compatible request body (JSON).
    pub body: Value,
}

/// Non-streaming upstream response: adapters have converted it to OpenAI-compatible format. `body` is raw bytes;
/// upstream error responses (HTML/non-JSON) are also relayed as-is without loss.
#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub status_code: u16,
    /// Converted / relayed response body.
    pub body: Vec<u8>,
    /// Usage parsed from the response (None on error responses or parse failures).
    pub usage: Option<Usage>,
}

/// Streaming response event: `data` is the OpenAI-compatible SSE bytes relayed to downstream, `usage` is the usage carried by this frame (if any).
#[derive(Debug, Clone)]
pub struct StreamEvent {
    pub data: Vec<u8>,
    pub usage: Option<Usage>,
}

/// Connectivity test result: success / failure, latency, failure reason (adapters probe with their respective model-list endpoints).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub ok: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Provider adapter errors: missing configuration and transport / parse failures. Upstream business errors (4xx/5xx) are not represented here —
/// `forward` returns the status code and body as-is, and the forwarding use case decides on retries.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ProviderError {
    #[error("provider not configured: {0}")]
    NotConfigured(String),
    #[error("request error: {0}")]
    Request(String),
    #[error("invalid upstream response: {0}")]
    InvalidResponse(String),
}

/// Provider adapter trait: isolates protocol differences; the core proxy depends only on this trait (see docs/Architecture-backend.md).
#[async_trait::async_trait]
pub trait ProviderAdaptor: Send + Sync {
    fn channel_type(&self) -> ChannelType;
    /// Default model list when creating a channel (for frontend form prefill).
    fn default_models(&self) -> Vec<String>;
    /// Default Base URL; Custom returns None with no default (the channel must configure it explicitly).
    fn default_base_url(&self) -> Option<&'static str>;
    /// Connectivity test: probes the model-list endpoint of each provider, returns latency and failure reason.
    async fn test(&self, channel: &Channel) -> Result<TestResult, ProviderError>;
    /// Non-streaming forward: build the upstream request, convert the protocol, parse usage.
    async fn forward(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError>;
    /// Streaming forward: returns an OpenAI-compatible SSE byte stream with per-frame parsed usage.
    async fn forward_stream(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(prompt: Option<u64>, completion: Option<u64>, total: Option<u64>) -> Usage {
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
        }
    }

    /// Saturating per-field sum when both sides have a value (accumulating usage across streamed frames).
    #[test]
    fn accumulate_sums_fields_present_on_both_sides() {
        let a = usage(Some(10), Some(5), Some(15));
        let b = usage(Some(3), Some(2), Some(5));
        assert_eq!(a.accumulate(b), usage(Some(13), Some(7), Some(20)));
    }

    /// Keep the present side when the other is missing (still aggregates when a provider omits some usage fields).
    #[test]
    fn accumulate_keeps_present_side_when_other_missing() {
        let a = usage(Some(10), None, None);
        let b = usage(None, Some(2), Some(5));
        assert_eq!(a.accumulate(b), usage(Some(10), Some(2), Some(5)));
    }

    /// Fields missing on both sides stay None; the empty object is the identity element.
    #[test]
    fn accumulate_none_fields_stay_none() {
        let a = usage(None, None, None);
        let b = usage(Some(1), None, None);
        assert_eq!(a.accumulate(b), usage(Some(1), None, None));
        assert_eq!(a.accumulate(a), usage(None, None, None));
    }

    /// Derives total from prompt + completion when missing; an existing total is not overwritten.
    #[test]
    fn normalized_derives_total_from_parts_when_missing() {
        assert_eq!(
            usage(Some(10), Some(5), None).normalized(),
            usage(Some(10), Some(5), Some(15))
        );
        assert_eq!(
            usage(Some(10), Some(5), Some(20)).normalized(),
            usage(Some(10), Some(5), Some(20))
        );
    }
}
