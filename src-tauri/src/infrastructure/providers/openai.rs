//! OpenAI-compatible 直通适配器：OpenAI / DeepSeek / Custom 共用同一实现。
//!
//! 请求原样转发（body 已含替换后的 model），非流式响应透传并解析 `usage`；
//! 流式响应逐帧透传原始 SSE 字节，同时用行缓冲扫描器解析 `usage`（仅检查不透传修改）。

use std::time::Duration;

use bytes::Bytes;
use futures_util::{Stream, StreamExt, stream};
use reqwest::Client;
use serde_json::Value;

use crate::domain::channel::{Channel, ChannelType};
use crate::domain::provider::{
    BoxStream, ChatRequest, ProviderAdaptor, ProviderError, ProviderResponse, StreamEvent,
    TestResult, Usage,
};
use crate::infrastructure::providers::{
    require_api_key, resolve_base_url, upstream_error_body, upstream_error_event,
};

/// OpenAI-compatible 直通适配器：按 `channel_type` 区分默认值（OpenAI / DeepSeek 有默认
/// Base URL 与模型列表，Custom 无默认必须显式配置）。
pub struct OpenAiCompatibleAdaptor {
    channel_type: ChannelType,
    client: Client,
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
            // Custom 无默认：渠道必须显式配置 Base URL。
            _ => None,
        }
    }

    /// 连通性测试：`GET {base_url}/models`，2xx 视为成功。
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

    /// 非流式转发：原样 POST body 到 `{base_url}/chat/completions`，透传响应并解析 usage。
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
            // 上游错误体原样透传会带回显密钥的风险（OpenAI 兼容端点 401 回显 sk-...），
            // 统一替换为通用错误体并保留状态码（红线：上游密钥不暴露给下游）。
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

    /// 流式转发：POST 后逐帧透传原始 SSE 字节；同时用行缓冲扫描器解析 usage。
    /// 非 2xx 的错误响应作为单帧直接透传（下游据此失败）。
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
            // 丢弃上游错误体（可能回显密钥），下发通用错误 SSE 帧（红线）。
            let status = resp.status().as_u16();
            return Ok(Box::pin(stream::once(async move {
                Ok(upstream_error_event(status))
            })));
        }
        Ok(Box::pin(relay_with_usage(resp.bytes_stream())))
    }
}

/// 逐帧透传上游 SSE 字节，并返回从各帧完整行解析出的 usage（如果有）。
fn relay_with_usage(
    byte_stream: impl Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
) -> impl Stream<Item = Result<StreamEvent, ProviderError>> + Send + 'static {
    let scanner = SseUsageScanner::new();
    // Box::pin 使未 pinned 的 impl Stream 满足 unfold 内部 `.next()` 的 Unpin 约束；
    // scanner 放进状态元组，避免被 FnMut 闭包按值移动。
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

/// 从非流式响应体解析 usage（解析失败返回 None，响应仍原样透传）。
fn extract_usage_from_body(body: &[u8]) -> Option<Usage> {
    let value: Value = serde_json::from_slice(body).ok()?;
    usage_from_value(&value)
}

/// 从任意 JSON 值提取 OpenAI 兼容 `usage` 对象。
fn usage_from_value(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let prompt = usage.get("prompt_tokens").and_then(Value::as_u64);
    let completion = usage.get("completion_tokens").and_then(Value::as_u64);
    let total = usage.get("total_tokens").and_then(Value::as_u64);
    if prompt.is_none() && completion.is_none() && total.is_none() {
        return None;
    }
    Some(
        Usage {
            prompt_tokens: prompt,
            completion_tokens: completion,
            total_tokens: total,
        }
        .normalized(),
    )
}

/// 流式 usage 扫描器：累积不完整行，仅用于解析 usage，不透传修改原始字节。
struct SseUsageScanner {
    pending: Vec<u8>,
}

impl SseUsageScanner {
    fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    /// 传入一个新 chunk，返回从其中完整 `data:` 行解析出的 usage（如有）。
    fn push(&mut self, chunk: &[u8]) -> Option<Usage> {
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

/// 解析单条 SSE `data:` 行，返回其 JSON 载荷字节（`[DONE]` 与非法行返回 None）。
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

    /// 构造 OpenAI 兼容非流式响应样本（含 usage）。
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

    /// 连通性测试：2xx 视为成功。
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

    /// 连通性测试：上游 4xx/5xx 视为失败并携带原因。
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

    /// 非流式转发：请求体原样透传，响应透传并解析 usage。
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
            Some(Usage {
                prompt_tokens: Some(10),
                completion_tokens: Some(5),
                total_tokens: Some(15),
            })
        );
        let body: Value = serde_json::from_slice(&resp.body).expect("json");
        assert_eq!(body["choices"][0]["message"]["content"], "hello");
    }

    /// 流式转发：上游 SSE 字节逐帧透传（拼接后不变），usage 从帧内完整行解析。
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
            Some(Usage {
                prompt_tokens: Some(3),
                completion_tokens: Some(2),
                total_tokens: Some(5),
            })
        );
    }

    /// 上游错误（非流式）：401 错误体会回显所提交的密钥，必须替换为通用错误体而非透传。
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

    /// 上游错误（流式）：非 2xx 错误体不原样透传，下发通用错误 SSE 帧（含 data: 行）。
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

    /// usage 扫描器：一条 `data:` 行跨多个 chunk 到达也能正确解析（行缓冲累计）。
    #[test]
    fn usage_scanner_handles_line_split_across_chunks() {
        let mut scanner = SseUsageScanner::new();
        assert_eq!(scanner.push(b"data: {\"us"), None);
        assert_eq!(
            scanner.push(b"age\":{\"total_tokens\":7}}\n\n"),
            Some(Usage {
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: Some(7),
            })
        );
        assert_eq!(scanner.push(b"data: [DONE]\n\n"), None);
    }
}
