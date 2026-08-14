//! Claude adapter: OpenAI-compatible request ↔ Anthropic Messages API protocol conversion.
//!
//! Request conversion: system messages split into the separate `system` field, remaining messages mapped,
//! `max_tokens` injected with a default when missing; non-streaming responses convert the Anthropic
//! structure to OpenAI `chat.completion`; streaming converts Anthropic SSE events one by one to OpenAI
//! `chat.completion.chunk` and appends `[DONE]`. Conversion logic is confined to this file.

use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use futures_util::{Stream, StreamExt, stream};
use reqwest::Client;
use serde_json::{Value, json};

use crate::domain::channel::Channel;
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent,
    TestResult, Usage,
};
use crate::infrastructure::providers::{
    chunk_event, extract_text_content, finish_event, require_api_key, resolve_base_url,
    upstream_error_body, upstream_error_event,
};

/// Claude adapter: default Base URL `https://api.anthropic.com`; the model list pre-fills the channel form.
pub struct ClaudeAdaptor {
    client: Client,
}

impl ClaudeAdaptor {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for ClaudeAdaptor {
    fn default() -> Self {
        Self::new()
    }
}

/// The Claude Messages API requires `max_tokens`, which OpenAI requests may not carry; inject a default here.
const DEFAULT_MAX_TOKENS: u64 = 4096;
/// Anthropic API version header (officially required).
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[async_trait::async_trait]
impl ProviderAdaptor for ClaudeAdaptor {
    fn channel_type(&self) -> crate::domain::channel::ChannelType {
        crate::domain::channel::ChannelType::Claude
    }

    fn default_models(&self) -> Vec<String> {
        vec![
            "claude-opus-4-1".into(),
            "claude-sonnet-4-5".into(),
            "claude-haiku-4-5".into(),
        ]
    }

    fn default_base_url(&self) -> Option<&'static str> {
        Some("https://api.anthropic.com")
    }

    /// Connectivity test: `GET {base_url}/v1/models` (Claude model list endpoint).
    async fn test(&self, channel: &Channel) -> Result<TestResult, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!("{}/v1/models", base_url.trim_end_matches('/'));
        let started = std::time::Instant::now();
        let result = self
            .client
            .get(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
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

    /// Non-streaming forward: convert request → POST `/v1/messages` → convert response to OpenAI format and parse usage.
    async fn forward(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let anthropic_body = to_anthropic_request(request)?;
        let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&anthropic_body)
            .send()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let status_code = resp.status().as_u16();
        if status_code >= 400 {
            // Relaying the upstream error body verbatim risks echoing the key; replace it with a generic error body (red line).
            return Ok(ProviderResponse {
                status_code,
                body: upstream_error_body(status_code),
                usage: None,
            });
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| ProviderError::Request(e.to_string()))?;
        let anthropic: Value = serde_json::from_slice(&body).map_err(|e| {
            ProviderError::InvalidResponse(format!("anthropic response not json: {e}"))
        })?;
        let openai = anthropic_to_openai(&anthropic, &request.model)?;
        let usage = usage_from_anthropic(&anthropic);
        Ok(ProviderResponse {
            status_code,
            body: serde_json::to_vec(&openai)
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?,
            usage,
        })
    }

    /// Streaming forward: convert request → POST `/v1/messages` (stream: true) → convert each event to an OpenAI chunk.
    async fn forward_stream(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let anthropic_body = to_anthropic_request(request)?;
        let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
        let resp = self
            .client
            .post(&url)
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&anthropic_body)
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
        Ok(Box::pin(anthropic_sse_to_openai(
            resp.bytes_stream(),
            request.model.clone(),
        )))
    }
}

/// Converts an OpenAI-compatible request into an Anthropic Messages request body.
fn to_anthropic_request(request: &ChatRequest) -> Result<Value, ProviderError> {
    let messages = request
        .body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut anthropic_messages = Vec::new();
    let mut system_texts = Vec::new();
    for message in &messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = extract_text_content(message.get("content"));
        match role {
            // System messages split into the separate `system` field.
            "system" => {
                if let Some(text) = content {
                    system_texts.push(text);
                }
            }
            "assistant" => anthropic_messages.push(json!({
                "role": "assistant",
                "content": content.unwrap_or_default(),
            })),
            // Messages with no matching semantics (tool / function / developer) fall back to user (MVP trade-off).
            _ => anthropic_messages.push(json!({
                "role": "user",
                "content": content.unwrap_or_default(),
            })),
        }
    }
    let max_tokens = request
        .body
        .get("max_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_TOKENS);
    let mut anthropic = json!({
        "model": request.model,
        "max_tokens": max_tokens,
        "messages": anthropic_messages,
    });
    if !system_texts.is_empty() {
        anthropic["system"] = Value::String(system_texts.join("\n"));
    }
    if let Some(t) = request.body.get("temperature") {
        anthropic["temperature"] = t.clone();
    }
    if let Some(t) = request.body.get("top_p") {
        anthropic["top_p"] = t.clone();
    }
    if request.stream {
        anthropic["stream"] = Value::Bool(true);
    }
    Ok(anthropic)
}

/// Converts a non-streaming Anthropic response to the OpenAI `chat.completion` format.
fn anthropic_to_openai(resp: &Value, model: &str) -> Result<Value, ProviderError> {
    let id = resp.get("id").and_then(Value::as_str).unwrap_or("msg");
    let content = resp
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let finish_reason = match resp.get("stop_reason").and_then(Value::as_str) {
        Some("max_tokens") => "length",
        _ => "stop",
    };
    Ok(json!({
        "id": id,
        "object": "chat.completion",
        "created": Utc::now().timestamp(),
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content},
            "finish_reason": finish_reason,
        }],
    }))
}

/// Extracts usage from an Anthropic response (`input_tokens` → prompt, `output_tokens` → completion).
fn usage_from_anthropic(resp: &Value) -> Option<Usage> {
    let usage = resp.get("usage")?;
    let prompt = usage.get("input_tokens").and_then(Value::as_u64);
    let completion = usage.get("output_tokens").and_then(Value::as_u64);
    if prompt.is_none() && completion.is_none() {
        return None;
    }
    Some(
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: None,
        }
        .normalized(),
    )
}

/// Converts Anthropic streaming SSE events one by one into OpenAI chunks, appending `[DONE]` at stream end.
/// State (line buffer / message id / stop_reason / prompt_tokens / whether [DONE] was emitted) is held
/// inside the unfold closure.
fn anthropic_sse_to_openai(
    byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    model: String,
) -> impl Stream<Item = Result<StreamEvent, ProviderError>> + Send + 'static {
    let created = Utc::now().timestamp();
    // Box::pin makes the unpinned impl Stream satisfy the Unpin bound required by `.next()` inside unfold.
    let byte_stream = Box::pin(byte_stream);
    // `model` etc. go into the state tuple (rather than being captured by the closure); otherwise the
    // async block cannot borrow it inside.
    futures_util::stream::unfold(
        (
            byte_stream,
            Vec::<u8>::new(),
            None::<String>,
            model,
            None::<String>,
            None::<u64>,
            false,
        ),
        move |(
            mut byte_stream,
            mut pending,
            mut message_id,
            model,
            mut stop_reason,
            mut prompt_tokens,
            mut done,
        )| async move {
            loop {
                if let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    if let Some(event) = process_anthropic_line(
                        &line,
                        &model,
                        created,
                        &mut message_id,
                        &mut stop_reason,
                        &mut prompt_tokens,
                    ) {
                        return Some((
                            Ok(event),
                            (
                                byte_stream,
                                pending,
                                message_id,
                                model,
                                stop_reason,
                                prompt_tokens,
                                done,
                            ),
                        ));
                    }
                    continue;
                }
                match byte_stream.next().await {
                    Some(Ok(bytes)) => pending.extend_from_slice(&bytes),
                    Some(Err(e)) => {
                        return Some((
                            Err(ProviderError::Request(e.to_string())),
                            (
                                byte_stream,
                                pending,
                                message_id,
                                model,
                                stop_reason,
                                prompt_tokens,
                                done,
                            ),
                        ));
                    }
                    None => {
                        if !done {
                            done = true;
                            return Some((
                                Ok(StreamEvent {
                                    data: b"data: [DONE]\n\n".to_vec(),
                                    usage: None,
                                }),
                                (
                                    byte_stream,
                                    pending,
                                    message_id,
                                    model,
                                    stop_reason,
                                    prompt_tokens,
                                    done,
                                ),
                            ));
                        }
                        return None;
                    }
                }
            }
        },
    )
}

/// Processes one Anthropic SSE `data:` line, returning the OpenAI chunk to relay (None when there is no output).
/// `stop_reason` is captured by message_delta and consumed by message_stop; `prompt_tokens` is captured by
/// message_start and merged into usage by message_delta.
fn process_anthropic_line(
    line: &[u8],
    model: &str,
    created: i64,
    message_id: &mut Option<String>,
    stop_reason: &mut Option<String>,
    prompt_tokens: &mut Option<u64>,
) -> Option<StreamEvent> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let payload = text.strip_prefix("data:")?.trim();
    let value: Value = serde_json::from_str(payload).ok()?;
    match value.get("type").and_then(Value::as_str) {
        Some("message_start") => {
            if let Some(id) = value.pointer("/message/id").and_then(Value::as_str) {
                *message_id = Some(id.to_string());
            }
            // In streaming, the prompt count appears only once at message_start; capture it and merge it into the final usage.
            if let Some(input) = value
                .pointer("/message/usage/input_tokens")
                .and_then(Value::as_u64)
            {
                *prompt_tokens = Some(input);
            }
            None
        }
        Some("content_block_delta") => {
            let delta = value.pointer("/delta/text").and_then(Value::as_str)?;
            if delta.is_empty() {
                None
            } else {
                Some(chunk_event(
                    message_id.as_deref().unwrap_or("msg"),
                    model,
                    created,
                    delta,
                    None,
                ))
            }
        }
        Some("message_delta") => {
            // stop_reason decides the finish_reason of the end frame (max_tokens → length).
            if let Some(r) = value.pointer("/delta/stop_reason").and_then(Value::as_str) {
                *stop_reason = Some(r.to_string());
            }
            let output = value
                .pointer("/usage/output_tokens")
                .and_then(Value::as_u64)?;
            let usage = Usage {
                prompt_tokens: *prompt_tokens,
                completion_tokens: Some(output),
                total_tokens: None,
            }
            .normalized();
            Some(chunk_event(
                message_id.as_deref().unwrap_or("msg"),
                model,
                created,
                "",
                Some(usage),
            ))
        }
        Some("message_stop") => {
            let finish_reason = match stop_reason.as_deref() {
                Some("max_tokens") => "length",
                _ => "stop",
            };
            Some(finish_event(
                message_id.as_deref().unwrap_or("msg"),
                model,
                created,
                finish_reason,
            ))
        }
        _ => None,
    }
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
    use crate::domain::channel::ChannelType;
    use crate::infrastructure::providers::test_util;

    /// Connectivity test: `GET /v1/models` 2xx counts as success.
    #[tokio::test]
    async fn test_reports_ok_on_success() {
        let router = Router::new().route("/v1/models", get(|| async { Json(json!({"data": []})) }));
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = ClaudeAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Claude, &base);

        let result = adaptor.test(&channel).await.expect("test");

        handle.abort();
        assert!(result.ok);
        assert_eq!(result.error, None);
    }

    /// Non-streaming forward: the request is converted to the Anthropic Messages format (system split out,
    /// max_tokens injected), and the response is converted back to the OpenAI format with usage parsed.
    #[tokio::test]
    async fn forward_converts_request_and_response() {
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let probe = received.clone();
        let router = Router::new().route(
            "/v1/messages",
            post(move |body: Json<Value>| async move {
                *probe.lock().unwrap() = Some(body.0);
                Json(json!({
                    "id": "msg_01",
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "text", "text": "hello from claude"}],
                    "model": "claude-sonnet-4-5",
                    "stop_reason": "end_turn",
                    "usage": {"input_tokens": 12, "output_tokens": 4},
                }))
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = ClaudeAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Claude, &base);
        let request = ChatRequest {
            model: "claude-sonnet-4-5".to_string(),
            stream: false,
            body: json!({
                "model": "claude-sonnet-4-5",
                "messages": [
                    {"role": "system", "content": "sys"},
                    {"role": "user", "content": "hi"},
                    {"role": "assistant", "content": "prev"},
                ],
                "temperature": 0.7,
            }),
        };

        let resp = adaptor.forward(&channel, &request).await.expect("forward");

        handle.abort();
        assert_eq!(resp.status_code, 200);
        let guard = received.lock().unwrap();
        let sent = guard.as_ref().expect("request captured");
        assert_eq!(sent["system"], "sys");
        assert_eq!(sent["max_tokens"], 4096);
        assert_eq!(sent["model"], "claude-sonnet-4-5");
        let messages = sent["messages"].as_array().expect("messages");
        assert_eq!(messages.len(), 2, "system must be split out of messages");
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"], "hi");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "prev");
        assert_eq!(sent["temperature"], 0.7);

        let body: Value = serde_json::from_slice(&resp.body).expect("json");
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(
            body["choices"][0]["message"]["content"],
            "hello from claude"
        );
        assert_eq!(
            resp.usage,
            Some(Usage {
                prompt_tokens: Some(12),
                completion_tokens: Some(4),
                total_tokens: Some(16),
            })
        );
    }

    /// Streaming forward: Anthropic SSE events are converted one by one into OpenAI chunks, ending with `[DONE]`.
    #[tokio::test]
    async fn forward_stream_converts_sse_to_chunks() {
        let payload = concat!(
            "event: message_start\n",
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_01\",\"role\":\"assistant\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,",
            "\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n",
            "event: content_block_delta\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,",
            "\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\n",
            "event: message_delta\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},",
            "\"usage\":{\"output_tokens\":15}}\n\n",
            "event: message_stop\n",
            "data: {\"type\":\"message_stop\"}\n\n",
            "data: [DONE]\n\n",
        );
        let router = Router::new().route(
            "/v1/messages",
            post(move || async move {
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(payload))
                    .expect("body")
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = ClaudeAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Claude, &base);
        let request = ChatRequest {
            model: "claude-sonnet-4-5".to_string(),
            stream: true,
            body: json!({"model": "claude-sonnet-4-5", "stream": true, "messages": []}),
        };

        let mut stream = adaptor
            .forward_stream(&channel, &request)
            .await
            .expect("forward_stream");
        let mut data = String::new();
        let mut usage = None;
        while let Some(event) = stream.next().await {
            let event = event.expect("event");
            data.push_str(&String::from_utf8_lossy(&event.data));
            if event.usage.is_some() {
                usage = event.usage;
            }
        }

        handle.abort();
        assert!(
            data.contains("\"content\":\"Hello\""),
            "first delta: {data}"
        );
        assert!(
            data.contains("\"content\":\" world\""),
            "second delta: {data}"
        );
        assert!(
            data.contains("\"finish_reason\":\"stop\""),
            "finish frame: {data}"
        );
        assert!(
            data.ends_with("data: [DONE]\n\n"),
            "must end with [DONE]: {data}"
        );
        assert_eq!(
            usage,
            Some(Usage {
                prompt_tokens: None,
                completion_tokens: Some(15),
                total_tokens: Some(15),
            })
        );
    }

    /// Upstream error: non-2xx response bodies are not relayed verbatim (prevents the upstream echoing the
    /// key in the error body); replaced with a generic error body while keeping the status code.
    #[tokio::test]
    async fn forward_sanitizes_upstream_error_body() {
        let router = Router::new().route(
            "/v1/messages",
            post(|| async {
                (
                    StatusCode::UNAUTHORIZED,
                    "{\"error\":{\"message\":\"invalid x-api-key: sk-leaked\"}}",
                )
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = ClaudeAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Claude, &base);
        let request = ChatRequest {
            model: "claude-sonnet-4-5".to_string(),
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

    /// Streaming forward (max_tokens truncation): finish_reason maps to length; usage merges the prompt
    /// count from message_start with the completion count from message_delta.
    #[tokio::test]
    async fn forward_stream_maps_length_finish_and_full_usage() {
        let payload = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_02\",\"role\":\"assistant\",",
            "\"usage\":{\"input_tokens\":9}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,",
            "\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"max_tokens\"},",
            "\"usage\":{\"output_tokens\":256}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n",
            "data: [DONE]\n\n",
        );
        let router = Router::new().route(
            "/v1/messages",
            post(move || async move {
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(payload))
                    .expect("body")
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = ClaudeAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Claude, &base);
        let request = ChatRequest {
            model: "claude-sonnet-4-5".to_string(),
            stream: true,
            body: json!({"model": "claude-sonnet-4-5", "stream": true, "messages": []}),
        };

        let mut stream = adaptor
            .forward_stream(&channel, &request)
            .await
            .expect("forward_stream");
        let mut data = String::new();
        let mut usage = None;
        while let Some(event) = stream.next().await {
            let event = event.expect("event");
            data.push_str(&String::from_utf8_lossy(&event.data));
            if event.usage.is_some() {
                usage = event.usage;
            }
        }

        handle.abort();
        assert!(
            data.contains("\"finish_reason\":\"length\""),
            "max_tokens must map to length: {data}"
        );
        assert_eq!(
            usage,
            Some(Usage {
                prompt_tokens: Some(9),
                completion_tokens: Some(256),
                total_tokens: Some(265),
            })
        );
    }
}
