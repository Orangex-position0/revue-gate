//! OpenAI-compatible passthrough adapter: OpenAI / DeepSeek / Custom share the same implementation.
//!
//! Requests are forwarded as-is (the body already contains the substituted model); non-streaming responses are
//! relayed and `usage` is parsed; streaming responses relay raw SSE bytes frame by frame while a line-buffer
//! scanner parses `usage` (inspect only, never modifying what is relayed).

use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream};
use reqwest::Client;
use serde_json::Value;

use crate::domain::channel::{Channel, ChannelType};
use crate::domain::knowledge::{EmbeddingClient, EmbeddingError};
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent,
    TestResult, TokenUsage,
};
use crate::infrastructure::providers::{
    require_api_key, resolve_base_url, upstream_error_body, upstream_error_event,
};

/// OpenAI-compatible passthrough adapter: defaults differ by `channel_type` (OpenAI / DeepSeek have default
/// Base URL and model list; Custom has no defaults and must be explicitly configured).
pub struct OpenAiCompatibleAdaptor {
    channel_type: ChannelType,
    client: Client,
}

pub struct OpenAiCompatibleEmbeddingClient {
    channel: Channel,
    channel_type: ChannelType,
    client: Client,
}

impl OpenAiCompatibleEmbeddingClient {
    pub fn new(channel: Channel) -> Self {
        Self {
            channel_type: channel.channel_type,
            channel,
            client: Client::new(),
        }
    }

    fn default_base_url(&self) -> Option<&'static str> {
        match self.channel_type {
            ChannelType::OpenAi => Some("https://api.openai.com/v1"),
            ChannelType::DeepSeek => Some("https://api.deepseek.com/v1"),
            _ => None,
        }
    }
}

#[async_trait::async_trait]
impl EmbeddingClient for OpenAiCompatibleEmbeddingClient {
    async fn embed(
        &self,
        model: &str,
        inputs: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let base_url = resolve_base_url(&self.channel, self.default_base_url())
            .map_err(|e| EmbeddingError::RequestFailed(e.to_string()))?;
        let api_key = require_api_key(&self.channel)
            .map_err(|e| EmbeddingError::RequestFailed(e.to_string()))?;
        let url = format!("{}/embeddings", base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(api_key)
            .json(&serde_json::json!({
                "model": model,
                "input": inputs,
            }))
            .send()
            .await
            .map_err(|e| EmbeddingError::RequestFailed(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(EmbeddingError::RequestFailed(format!(
                "upstream embedding request failed with status {status}"
            )));
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|_| EmbeddingError::InvalidResponse)?;
        parse_embedding_vectors(&value)
    }
}

fn parse_embedding_vectors(value: &Value) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or(EmbeddingError::InvalidResponse)?;
    let mut items = data
        .iter()
        .map(|item| {
            let index = item
                .get("index")
                .and_then(Value::as_u64)
                .ok_or(EmbeddingError::InvalidResponse)? as usize;
            let embedding = item
                .get("embedding")
                .and_then(Value::as_array)
                .ok_or(EmbeddingError::InvalidResponse)?
                .iter()
                .map(|value| {
                    value
                        .as_f64()
                        .map(|value| value as f32)
                        .ok_or(EmbeddingError::InvalidResponse)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((index, embedding))
        })
        .collect::<Result<Vec<_>, EmbeddingError>>()?;
    items.sort_by_key(|(index, _)| *index);
    Ok(items.into_iter().map(|(_, vector)| vector).collect())
}

impl OpenAiCompatibleAdaptor {
    pub fn new(channel_type: ChannelType) -> Self {
        Self {
            channel_type,
            client: Client::new(),
        }
    }
}

#[async_trait::async_trait]
impl ProviderAdaptor for OpenAiCompatibleAdaptor {
    fn channel_type(&self) -> ChannelType {
        self.channel_type
    }

    fn default_models(&self) -> Vec<String> {
        match self.channel_type {
            ChannelType::OpenAi => vec![
                "gpt-4o".into(),
                "gpt-4o-mini".into(),
                "gpt-4.1".into(),
                "gpt-4.1-mini".into(),
            ],
            ChannelType::DeepSeek => vec!["deepseek-chat".into(), "deepseek-reasoner".into()],
            _ => Vec::new(),
        }
    }

    fn default_base_url(&self) -> Option<&'static str> {
        match self.channel_type {
            ChannelType::OpenAi => Some("https://api.openai.com/v1"),
            ChannelType::DeepSeek => Some("https://api.deepseek.com/v1"),
            // Custom has no defaults: the channel must explicitly configure a Base URL.
            _ => None,
        }
    }

    async fn fetch_models(&self, channel: &Channel) -> Result<Vec<String>, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let resp = self
            .client
            .get(&url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(ProviderError::Request(format!(
                "model discovery failed with status {status}"
            )));
        }
        let value: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?;
        parse_model_list(&value)
    }

    /// Connectivity test: `GET {base_url}/models`, 2xx counts as success.
    async fn test(&self, channel: &Channel) -> Result<TestResult, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!("{}/models", base_url.trim_end_matches('/'));
        let started = std::time::Instant::now();
        let result = self
            .client
            .get(&url)
            .bearer_auth(api_key)
            .timeout(Duration::from_secs(10))
            .send()
            .await;
        let latency_ms = started.elapsed().as_millis() as u64;
        Ok(match result {
            Ok(resp) if resp.status().is_success() => TestResult {
                ok: true,
                latency_ms,
                error: None,
            },
            Ok(resp) => TestResult {
                ok: false,
                latency_ms,
                error: Some(format!("upstream responded {}", resp.status())),
            },
            Err(e) => TestResult {
                ok: false,
                latency_ms,
                error: Some(e.to_string()),
            },
        })
    }

    /// Non-streaming forward: POST the body as-is to `{base_url}/chat/completions`, relay the response and parse usage.
    async fn forward(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(api_key)
            .json(&request.body)
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let status_code = resp.status().as_u16();
        if status_code >= 400 {
            // Relaying the upstream error body verbatim risks echoing the key (OpenAI-compatible endpoints echo
            // sk-... on 401); replace it with a generic error body while keeping the status code (red line:
            // upstream keys must not be exposed downstream).
            return Ok(ProviderResponse {
                status_code,
                body: upstream_error_body(status_code),
                usage: None,
            });
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?
            .to_vec();
        let usage = extract_usage_from_body(&body);
        Ok(ProviderResponse {
            status_code,
            body,
            usage,
        })
    }

    /// Streaming forward: after POST, relay raw SSE bytes frame by frame; a line-buffer scanner parses usage
    /// in parallel. Non-2xx error responses are relayed as a single frame (the downstream fails accordingly).
    async fn forward_stream(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .bearer_auth(api_key)
            .json(&request.body)
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        if !resp.status().is_success() {
            // Drop the upstream error body (may echo the key), emit a generic error SSE frame (red line).
            let status = resp.status().as_u16();
            return Ok(Box::pin(stream::once(async move {
                Ok(upstream_error_event(status))
            })));
        }
        Ok(Box::pin(relay_with_usage(resp.bytes_stream())))
    }
}

/// Relays upstream SSE bytes frame by frame, returning usage parsed from complete lines in each frame (if any).
fn relay_with_usage(
    byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent, ProviderError>> + Send + 'static {
    let scanner = SseUsageScanner::new();
    // Box::pin makes the unpinned impl Stream satisfy the Unpin bound required by `.next()` inside unfold;
    // the scanner goes into the state tuple so it is not moved by value into the FnMut closure.
    let byte_stream = Box::pin(byte_stream);
    futures_util::stream::unfold(
        (byte_stream, scanner),
        move |(mut byte_stream, mut scanner)| async move {
            match byte_stream.next().await {
                Some(Ok(bytes)) => {
                    let usage = scanner.push(&bytes);
                    let data = bytes.to_vec();
                    Some((Ok(StreamEvent { data, usage }), (byte_stream, scanner)))
                }
                Some(Err(e)) => Some((
                    Err(ProviderError::Request(e.to_string())),
                    (byte_stream, scanner),
                )),
                None => None,
            }
        },
    )
}

/// Parses usage from a non-streaming response body (returns None on parse failure; the response is still relayed as-is).
fn extract_usage_from_body(body: &[u8]) -> Option<TokenUsage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    usage_from_value(&value)
}

/// Extracts an OpenAI-compatible `usage` object from any JSON value.
fn usage_from_value(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    let prompt = usage.get("prompt_tokens").and_then(Value::as_u64);
    let completion = usage.get("completion_tokens").and_then(Value::as_u64);
    let total = usage.get("total_tokens").and_then(Value::as_u64);
    if prompt.is_none() && completion.is_none() && total.is_none() {
        return None;
    }
    Some(
        TokenUsage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
        }
        .normalized(),
    )
}

fn parse_model_list(value: &Value) -> Result<Vec<String>, ProviderError> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::InvalidResponse("missing data array".into()))?;
    let mut models = Vec::new();
    for item in data {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| ProviderError::InvalidResponse("model item missing id".into()))?;
        if !models.iter().any(|m| m == id) {
            models.push(id.to_string());
        }
    }
    Ok(models)
}

/// Streaming usage scanner: accumulates incomplete lines, only used to parse usage, never modifying the relayed bytes.
struct SseUsageScanner {
    pending: Vec<u8>,
}

impl SseUsageScanner {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// Feeds a new chunk and returns usage parsed from complete `data:` lines within it (if any).
    fn push(&mut self, chunk: &[u8]) -> Option<TokenUsage> {
        self.pending.extend_from_slice(chunk);
        let mut usage = None;
        let mut consumed = 0;
        for (i, &b) in self.pending.iter().enumerate() {
            if b == b'\n' {
                if let Some(line) = parse_sse_data_line(&self.pending[consumed..i])
                    && let Ok(value) = serde_json::from_slice::<Value>(line)
                    && let Some(u) = usage_from_value(&value)
                {
                    usage = Some(u);
                }
                consumed = i + 1;
            }
        }
        self.pending.drain(..consumed);
        usage
    }
}

/// Parses a single SSE `data:` line, returning its JSON payload bytes (`[DONE]` and invalid lines return None).
fn parse_sse_data_line(line: &[u8]) -> Option<&[u8]> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let payload = text.strip_prefix("data:")?.trim();
    if payload == "[DONE]" {
        return None;
    }
    Some(payload.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{
        Router,
        body::Body,
        extract::Json,
        http::{StatusCode, header},
        response::Response,
        routing::{get, post},
    };
    use futures_util::StreamExt;
    use serde_json::{Value, json};

    use super::*;
    use crate::infrastructure::providers::test_util;

    /// Builds a sample OpenAI-compatible non-streaming response (with usage).
    fn chat_completion_response() -> Value {
        json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "gpt-4o",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop",
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        })
    }

    /// Connectivity test: 2xx counts as success.
    #[tokio::test]
    async fn test_reports_ok_on_success() {
        let router = Router::new().route("/models", get(|| async { Json(json!({"data": []})) }));
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);

        let result = adaptor.test(&channel).await.expect("test");

        handle.abort();
        assert!(result.ok);
        assert_eq!(result.error, None);
    }

    /// Connectivity test: upstream 4xx/5xx counts as failure and carries the reason.
    #[tokio::test]
    async fn test_reports_failure_on_upstream_error() {
        let router = Router::new().route("/models", get(|| async { StatusCode::UNAUTHORIZED }));
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);

        let result = adaptor.test(&channel).await.expect("test");

        handle.abort();
        assert!(!result.ok);
        assert!(
            result.error.as_deref().unwrap_or_default().contains("401"),
            "error should mention upstream status"
        );
    }

    #[tokio::test]
    async fn fetch_models_reads_openai_compatible_model_ids() {
        let router = Router::new().route(
            "/models",
            get(|| async {
                Json(json!({
                    "object": "list",
                    "data": [
                        {"id": "gpt-4o", "object": "model"},
                        {"id": "gpt-4o-mini", "object": "model"},
                        {"id": "gpt-4o", "object": "model"}
                    ]
                }))
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);

        let models = adaptor.fetch_models(&channel).await.expect("fetch models");

        handle.abort();
        assert_eq!(models, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[tokio::test]
    async fn fetch_models_rejects_malformed_model_list() {
        let router = Router::new().route("/models", get(|| async { Json(json!({"data": [{}]})) }));
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);

        let err = adaptor
            .fetch_models(&channel)
            .await
            .expect_err("malformed response should fail");

        handle.abort();
        assert!(matches!(err, ProviderError::InvalidResponse(_)));
    }

    #[tokio::test]
    async fn deepseek_uses_openai_compatible_defaults() {
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::DeepSeek);

        assert_eq!(adaptor.channel_type(), ChannelType::DeepSeek);
        assert_eq!(
            adaptor.default_base_url(),
            Some("https://api.deepseek.com/v1")
        );
        assert_eq!(
            adaptor.default_models(),
            vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
        );
    }

    #[tokio::test]
    async fn deepseek_reuses_openai_compatible_forward_mapping() {
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let probe = received.clone();
        let router = Router::new().route(
            "/chat/completions",
            post(move |body: Json<Value>| async move {
                *probe.lock().unwrap() = Some(body.0);
                Json(json!({
                    "id": "deepseek-chatcmpl",
                    "object": "chat.completion",
                    "created": 1700000000,
                    "model": "deepseek-chat",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "hello"},
                        "finish_reason": "stop",
                    }],
                    "usage": {"prompt_tokens": 4, "completion_tokens": 6, "total_tokens": 10},
                }))
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::DeepSeek);
        let channel = test_util::test_channel(ChannelType::DeepSeek, &base);
        let request = ChatRequest {
            model: "deepseek-chat".to_string(),
            stream: false,
            body: json!({
                "model": "deepseek-chat",
                "messages": [{"role": "user", "content": "hi"}],
            }),
        };

        let resp = adaptor.forward(&channel, &request).await.expect("forward");

        handle.abort();
        assert_eq!(*received.lock().unwrap(), Some(request.body));
        assert_eq!(
            resp.usage,
            Some(TokenUsage {
                prompt_tokens: Some(4),
                completion_tokens: Some(6),
                total_tokens: Some(10),
            })
        );
        let body: Value = serde_json::from_slice(&resp.body).expect("json");
        assert_eq!(body["model"], "deepseek-chat");
        assert_eq!(body["choices"][0]["message"]["content"], "hello");
    }

    /// Non-streaming forward: the request body is relayed as-is; the response is relayed and usage is parsed.
    #[tokio::test]
    async fn forward_passes_body_and_parses_usage() {
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let probe = received.clone();
        let router = Router::new().route(
            "/chat/completions",
            post(move |body: Json<Value>| async move {
                *probe.lock().unwrap() = Some(body.0);
                Json(chat_completion_response())
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);
        let request = ChatRequest {
            model: "gpt-4o".to_string(),
            stream: false,
            body: json!({
                "model": "gpt-4o",
                "messages": [{"role": "user", "content": "hi"}],
            }),
        };

        let resp = adaptor.forward(&channel, &request).await.expect("forward");

        handle.abort();
        assert_eq!(resp.status_code, 200);
        assert_eq!(*received.lock().unwrap(), Some(request.body));
        assert_eq!(
            resp.usage,
            Some(TokenUsage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
            })
        );
        let body: Value = serde_json::from_slice(&resp.body).expect("json");
        assert_eq!(body["choices"][0]["message"]["content"], "hello");
    }

    /// Streaming forward: upstream SSE bytes are relayed frame by frame (unchanged when concatenated); usage is
    /// parsed from complete lines in frames.
    #[tokio::test]
    async fn forward_stream_relays_bytes_and_scans_usage() {
        let payload = concat!(
            "data: {\"id\":\"x\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}],",
            "\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        let router = Router::new().route(
            "/chat/completions",
            post(move || async move {
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(payload))
                    .expect("body")
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);
        let request = ChatRequest {
            model: "gpt-4o".to_string(),
            stream: true,
            body: json!({"model": "gpt-4o", "stream": true, "messages": []}),
        };

        let mut stream = adaptor
            .forward_stream(&channel, &request)
            .await
            .expect("forward_stream");
        let mut data = Vec::new();
        let mut usage = None;
        while let Some(event) = stream.next().await {
            let event = event.expect("event");
            data.extend_from_slice(&event.data);
            if event.usage.is_some() {
                usage = event.usage;
            }
        }

        handle.abort();
        assert_eq!(data, payload.as_bytes(), "bytes must be relayed unchanged");
        assert_eq!(
            usage,
            Some(TokenUsage {
                prompt_tokens: Some(3),
                completion_tokens: Some(2),
                total_tokens: Some(5),
            })
        );
    }

    /// Upstream error (non-streaming): a 401 error body echoes the submitted key; it must be replaced with a
    /// generic error body rather than relayed.
    #[tokio::test]
    async fn forward_masks_upstream_error_body() {
        let router = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    "{\"error\":{\"message\":\"Incorrect API key provided: sk-leaked\"}}",
                )
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);
        let request = ChatRequest {
            model: "gpt-4o".to_string(),
            stream: false,
            body: json!({"messages": []}),
        };

        let resp = adaptor.forward(&channel, &request).await.expect("forward");

        handle.abort();
        assert_eq!(resp.status_code, 401);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(
            !body.contains("sk-leaked"),
            "upstream error body must not be relayed: {body}"
        );
        assert!(body.contains("upstream request failed"), "got: {body}");
        assert_eq!(resp.usage, None);
    }

    /// Upstream error (streaming): non-2xx error bodies are not relayed verbatim; a generic error SSE frame
    /// (with a data: line) is emitted.
    #[tokio::test]
    async fn forward_stream_masks_upstream_error_body() {
        let router = Router::new().route(
            "/chat/completions",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    "{\"error\":{\"message\":\"Incorrect API key provided: sk-leaked\"}}",
                )
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = OpenAiCompatibleAdaptor::new(ChannelType::OpenAi);
        let channel = test_util::test_channel(ChannelType::OpenAi, &base);
        let request = ChatRequest {
            model: "gpt-4o".to_string(),
            stream: true,
            body: json!({"model": "gpt-4o", "stream": true, "messages": []}),
        };

        let mut stream = adaptor
            .forward_stream(&channel, &request)
            .await
            .expect("forward_stream");
        let mut data = Vec::new();
        while let Some(event) = stream.next().await {
            let event = event.expect("event");
            data.extend_from_slice(&event.data);
        }

        handle.abort();
        let text = String::from_utf8_lossy(&data);
        assert!(
            !text.contains("sk-leaked"),
            "upstream error body must not be relayed: {text}"
        );
        assert!(
            text.starts_with("data: ") && text.trim_end().ends_with('}'),
            "error frame must be a valid SSE data line: {text}"
        );
    }

    /// usage scanner: a single `data:` line arriving across multiple chunks is still parsed correctly
    /// (line-buffer accumulation).
    #[test]
    fn usage_scanner_handles_line_split_across_chunks() {
        let mut scanner = SseUsageScanner::new();
        assert_eq!(scanner.push(b"data: {\"us"), None);
        assert_eq!(
            scanner.push(b"age\":{\"total_tokens\":7}}\n\n"),
            Some(TokenUsage {
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: Some(7),
            })
        );
        assert_eq!(scanner.push(b"data: [DONE]\n\n"), None);
    }
}
