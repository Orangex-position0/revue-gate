//! Gemini adapter: OpenAI-compatible request ↔ Gemini generateContent API protocol conversion.
//!
//! Request conversion: system messages split into `systemInstruction`, user/assistant mapped to `contents.parts`,
//! `max_tokens` injected with a default when missing; non-streaming responses convert the Gemini `candidates`
//! structure to OpenAI `chat.completion`; streaming (`:streamGenerateContent?alt=sse`) converts Gemini SSE
//! events one by one to OpenAI `chat.completion.chunk` and appends `[DONE]`. Conversion logic is confined to this file.

use std::time::Duration;

use bytes::Bytes;
use chrono::Utc;
use futures_util::{Stream, StreamExt, stream};
use reqwest::Client;
use serde_json::{Value, json};

use crate::domain::channel::Channel;
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent,
    TestResult, TokenUsage,
};
use crate::infrastructure::providers::{
    chunk_event, extract_text_content, finish_event, require_api_key, resolve_base_url,
    upstream_error_body, upstream_error_event,
};

/// Gemini adapter: default Base URL `https://generativelanguage.googleapis.com`; the model list pre-fills the channel form.
pub struct GeminiAdaptor {
    client: Client,
}

impl GeminiAdaptor {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for GeminiAdaptor {
    fn default() -> Self {
        Self::new()
    }
}

/// Gemini requests require `maxOutputTokens`, which OpenAI requests may not carry; inject a default here.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 4096;
/// Gemini REST auth header (the key goes in a header, not the URL query string, to avoid it being written to logs).
const GEMINI_API_KEY_HEADER: &str = "x-goog-api-key";

#[async_trait::async_trait]
impl ProviderAdaptor for GeminiAdaptor {
    fn channel_type(&self) -> crate::domain::channel::ChannelType {
        crate::domain::channel::ChannelType::Gemini
    }

    fn default_models(&self) -> Vec<String> {
        vec![
            "gemini-2.0-flash".into(),
            "gemini-2.5-flash".into(),
            "gemini-2.5-pro".into(),
        ]
    }

    fn default_base_url(&self) -> Option<&'static str> {
        Some("https://generativelanguage.googleapis.com")
    }

    /// Connectivity test: `GET {base_url}/v1beta/models` (Gemini model list endpoint).
    async fn test(&self, channel: &Channel) -> Result<TestResult, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!("{}/v1beta/models", base_url.trim_end_matches('/'));
        let started = std::time::Instant::now();
        let result = self
            .client
            .get(&url)
            .header(GEMINI_API_KEY_HEADER, api_key)
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

    /// Non-streaming forward: convert request → POST `/v1beta/models/{model}:generateContent` → convert response to OpenAI format.
    async fn forward(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!(
            "{}/v1beta/models/{}:generateContent",
            base_url.trim_end_matches('/'),
            request.model
        );
        let gemini_body = to_gemini_request(request)?;
        let resp = self
            .client
            .post(&url)
            .header(GEMINI_API_KEY_HEADER, api_key)
            .json(&gemini_body)
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
        let gemini: Value = serde_json::from_slice(&body).map_err(|e| {
            ProviderError::InvalidResponse(format!("gemini response not json: {e}"))
        })?;
        let openai = gemini_to_openai(&gemini, &request.model)?;
        let usage = usage_from_gemini(&gemini);
        Ok(ProviderResponse {
            status_code,
            body: serde_json::to_vec(&openai)
                .map_err(|e| ProviderError::InvalidResponse(e.to_string()))?,
            usage,
        })
    }

    /// Streaming forward: POST `:streamGenerateContent?alt=sse` → convert each event to an OpenAI chunk.
    async fn forward_stream(
        &self,
        channel: &Channel,
        request: &ChatRequest,
    ) -> Result<BoxStream<'static, Result<StreamEvent, ProviderError>>, ProviderError> {
        let base_url = resolve_base_url(channel, self.default_base_url())?;
        let api_key = require_api_key(channel)?;
        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse",
            base_url.trim_end_matches('/'),
            request.model
        );
        let gemini_body = to_gemini_request(request)?;
        let resp = self
            .client
            .post(&url)
            .header(GEMINI_API_KEY_HEADER, api_key)
            .json(&gemini_body)
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
        Ok(Box::pin(gemini_sse_to_openai(
            resp.bytes_stream(),
            request.model.clone(),
        )))
    }
}

/// Converts an OpenAI-compatible request into a Gemini `generateContent` request body.
fn to_gemini_request(request: &ChatRequest) -> Result<Value, ProviderError> {
    let messages = request
        .body
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut contents = Vec::new();
    let mut system_texts = Vec::new();
    for message in &messages {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        let content = extract_text_content(message.get("content"));
        match role {
            // System messages split into the separate `systemInstruction` field.
            "system" => {
                if let Some(text) = content {
                    system_texts.push(text);
                }
            }
            "assistant" => contents.push(json!({
                "role": "model",
                "parts": [{"text": content.unwrap_or_default()}],
            })),
            // Messages with no matching semantics (tool / function / developer) fall back to user (MVP trade-off).
            _ => contents.push(json!({
                "role": "user",
                "parts": [{"text": content.unwrap_or_default()}],
            })),
        }
    }
    let max_output_tokens = request
        .body
        .get("max_tokens")
        .or_else(|| request.body.get("max_completion_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS);
    let mut generation_config = json!({ "maxOutputTokens": max_output_tokens });
    if let Some(t) = request.body.get("temperature") {
        generation_config["temperature"] = t.clone();
    }
    if let Some(t) = request.body.get("top_p") {
        generation_config["topP"] = t.clone();
    }
    let mut gemini = json!({
        "contents": contents,
        "generationConfig": generation_config,
    });
    if !system_texts.is_empty() {
        gemini["systemInstruction"] = json!({"parts": [{"text": system_texts.join("\n")}]});
    }
    if request.stream {
        // In streaming, make the last chunk carry usageMetadata so usage can be parsed.
        gemini["streamConfig"] = json!({"streamIncludeUsage": true});
    }
    Ok(gemini)
}

/// Converts a non-streaming Gemini response to the OpenAI `chat.completion` format.
fn gemini_to_openai(resp: &Value, model: &str) -> Result<Value, ProviderError> {
    let content = resp
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    let finish_reason = match resp
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str)
    {
        Some("MAX_TOKENS") => "length",
        _ => "stop",
    };
    Ok(json!({
        "id": "chatcmpl-gemini",
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

/// Extracts usage from a Gemini response (`promptTokenCount` → prompt, `candidatesTokenCount` → completion).
fn usage_from_gemini(resp: &Value) -> Option<TokenUsage> {
    let usage = resp.get("usageMetadata")?;
    let prompt = usage.get("promptTokenCount").and_then(Value::as_u64);
    let completion = usage.get("candidatesTokenCount").and_then(Value::as_u64);
    let total = usage.get("totalTokenCount").and_then(Value::as_u64);
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

/// Converts Gemini streaming SSE events one by one into OpenAI chunks, appending `[DONE]` at stream end.
/// A single SSE line may carry text, finishReason and usageMetadata at once; emit multiple events in order.
/// State (line buffer / pending event queue / whether [DONE] was emitted) is held inside the unfold closure.
fn gemini_sse_to_openai(
    byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    model: String,
) -> impl Stream<Item = Result<StreamEvent, ProviderError>> + Send + 'static {
    let created = Utc::now().timestamp();
    let id = format!("chatcmpl-gemini-{created}");
    // Box::pin makes the unpinned impl Stream satisfy the Unpin bound required by `.next()` inside unfold.
    let byte_stream = Box::pin(byte_stream);
    // `model` / `id` go into the state tuple (rather than being captured by the closure); otherwise the
    // async block cannot borrow them inside.
    futures_util::stream::unfold(
        (
            byte_stream,
            Vec::<u8>::new(),
            Vec::<StreamEvent>::new(),
            model,
            id,
            false,
        ),
        move |(mut byte_stream, mut pending, mut queued, model, id, mut done)| async move {
            loop {
                if !queued.is_empty() {
                    // A single SSE line can produce multiple events, emitted in order (the queue is tiny, so remove(0) is fine).
                    let event = queued.remove(0);
                    return Some((Ok(event), (byte_stream, pending, queued, model, id, done)));
                }
                if let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=pos).collect();
                    process_gemini_line(&line, &model, created, &id, &mut queued);
                    continue;
                }
                match byte_stream.next().await {
                    Some(Ok(bytes)) => pending.extend_from_slice(&bytes),
                    Some(Err(e)) => {
                        return Some((
                            Err(ProviderError::Request(e.to_string())),
                            (byte_stream, pending, queued, model, id, done),
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
                                (byte_stream, pending, queued, model, id, done),
                            ));
                        }
                        return None;
                    }
                }
            }
        },
    )
}

/// Processes one Gemini SSE `data:` line: text → content chunk, finishReason → finish chunk,
/// usageMetadata → attached to the corresponding chunk. Nothing happens when there is no output.
fn process_gemini_line(
    line: &[u8],
    model: &str,
    created: i64,
    id: &str,
    queued: &mut Vec<StreamEvent>,
) {
    let text = std::str::from_utf8(line).ok();
    let Some(text) = text else { return };
    let payload = text.trim().strip_prefix("data:");
    let Some(payload) = payload else { return };
    let Ok(value) = serde_json::from_str::<Value>(payload.trim()) else {
        return;
    };
    let content = value
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(Value::as_str)
        .unwrap_or("");
    let finish = value
        .pointer("/candidates/0/finishReason")
        .and_then(Value::as_str);
    let usage = usage_from_gemini(&value);
    // usageMetadata usually appears with the first (prompt count) and last chunks; attach it to the nearest event.
    if !content.is_empty() {
        queued.push(chunk_event(id, model, created, content, usage));
    } else if usage.is_some() {
        queued.push(chunk_event(id, model, created, "", usage));
    }
    if finish.is_some() {
        let finish_reason = match finish {
            Some("MAX_TOKENS") => "length",
            _ => "stop",
        };
        queued.push(finish_event(id, model, created, finish_reason));
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

    /// Connectivity test: `GET /v1beta/models` 2xx counts as success.
    #[tokio::test]
    async fn test_reports_ok_on_success() {
        let router = Router::new().route(
            "/v1beta/models",
            get(|| async { Json(json!({"models": []})) }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = GeminiAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Gemini, &base);

        let result = adaptor.test(&channel).await.expect("test");

        handle.abort();
        assert!(result.ok);
        assert_eq!(result.error, None);
    }

    /// Non-streaming forward: the request is converted to the Gemini generateContent format (systemInstruction
    /// split out, maxOutputTokens injected, roles mapped user/model), and the response is converted back to the
    /// OpenAI format with usage parsed.
    #[tokio::test]
    async fn forward_converts_request_and_response() {
        let received: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let probe = received.clone();
        let router = Router::new().route(
            "/v1beta/models/gemini-test:generateContent",
            post(move |body: Json<Value>| async move {
                *probe.lock().unwrap() = Some(body.0);
                Json(json!({
                    "candidates": [{
                        "content": {"parts": [{"text": "gemini reply"}]},
                        "finishReason": "STOP",
                    }],
                    "usageMetadata": {
                        "promptTokenCount": 5,
                        "candidatesTokenCount": 9,
                        "totalTokenCount": 14,
                    },
                }))
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = GeminiAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Gemini, &base);
        let request = ChatRequest {
            model: "gemini-test".to_string(),
            stream: false,
            body: json!({
                "model": "gemini-test",
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
        assert_eq!(sent["systemInstruction"]["parts"][0]["text"], "sys");
        assert_eq!(sent["generationConfig"]["maxOutputTokens"], 4096);
        assert_eq!(sent["generationConfig"]["temperature"], 0.7);
        let contents = sent["contents"].as_array().expect("contents");
        assert_eq!(
            contents.len(),
            2,
            "system must be split into systemInstruction"
        );
        assert_eq!(contents[0]["role"], "user");
        assert_eq!(contents[0]["parts"][0]["text"], "hi");
        assert_eq!(contents[1]["role"], "model");
        assert_eq!(contents[1]["parts"][0]["text"], "prev");

        let body: Value = serde_json::from_slice(&resp.body).expect("json");
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["choices"][0]["message"]["content"], "gemini reply");
        assert_eq!(
            resp.usage,
            Some(TokenUsage {
                prompt_tokens: Some(5),
                completion_tokens: Some(9),
                total_tokens: Some(14),
            })
        );
    }

    /// Streaming forward: Gemini SSE events are converted one by one into OpenAI chunks (including usage and
    /// finish), ending with `[DONE]`.
    #[tokio::test]
    async fn forward_stream_converts_sse_to_chunks() {
        let payload = concat!(
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}],\"role\":\"model\"},",
            "\"finishReason\":null}],\"usageMetadata\":{\"promptTokenCount\":2,\"totalTokenCount\":2}}\n\n",
            "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\" world\"}],\"role\":\"model\"},",
            "\"finishReason\":null}]}\n\n",
            "data: {\"candidates\":[{\"content\":{},\"finishReason\":\"STOP\"}],",
            "\"usageMetadata\":{\"promptTokenCount\":2,\"candidatesTokenCount\":4,\"totalTokenCount\":6}}\n\n",
            "data: [DONE]\n\n",
        );
        let router = Router::new().route(
            "/v1beta/models/gemini-test:streamGenerateContent",
            post(move || async move {
                Response::builder()
                    .header(header::CONTENT_TYPE, "text/event-stream")
                    .body(Body::from(payload))
                    .expect("body")
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = GeminiAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Gemini, &base);
        let request = ChatRequest {
            model: "gemini-test".to_string(),
            stream: true,
            body: json!({"model": "gemini-test", "stream": true, "messages": []}),
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
            Some(TokenUsage {
                prompt_tokens: Some(2),
                completion_tokens: Some(4),
                total_tokens: Some(6),
            })
        );
    }

    /// Upstream error: non-2xx response bodies are not relayed verbatim (prevents the upstream echoing the
    /// key in the error body); replaced with a generic error body while keeping the status code.
    #[tokio::test]
    async fn forward_sanitizes_upstream_error_body() {
        let router = Router::new().route(
            "/v1beta/models/gemini-test:generateContent",
            post(|| async {
                (
                    StatusCode::FORBIDDEN,
                    "{\"error\":{\"message\":\"API key not valid: sk-leaked\"}}",
                )
            }),
        );
        let (base, handle) = test_util::spawn(router).await;
        let adaptor = GeminiAdaptor::new();
        let channel = test_util::test_channel(ChannelType::Gemini, &base);
        let request = ChatRequest {
            model: "gemini-test".to_string(),
            stream: false,
            body: json!({"messages": []}),
        };

        let resp = adaptor.forward(&channel, &request).await.expect("forward");

        handle.abort();
        assert_eq!(resp.status_code, 403);
        let body = String::from_utf8_lossy(&resp.body);
        assert!(
            !body.contains("sk-leaked"),
            "upstream error body must not be relayed: {body}"
        );
        assert!(body.contains("upstream request failed"), "got: {body}");
        assert_eq!(resp.usage, None);
    }
}
