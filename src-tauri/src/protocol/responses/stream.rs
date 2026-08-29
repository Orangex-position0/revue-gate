use bytes::Bytes;
use serde_json::{Value, json};

use crate::protocol::sse::SseDecoder;
use crate::protocol::{ConversionContext, ProtocolError, StreamFrame, StreamTransformer};

pub(crate) struct ResponsesStreamTransformer {
    decoder: SseDecoder,
    trace_id: String,
    now_unix: i64,
    started: bool,
    response_id: Option<String>,
    model: Option<String>,
    created_at: Option<i64>,
    text: String,
    status: String,
    input_tokens: u64,
    output_tokens: u64,
    total_tokens: u64,
}

impl ResponsesStreamTransformer {
    pub(crate) fn new(context: &ConversionContext) -> Self {
        Self {
            decoder: SseDecoder::new(),
            trace_id: context.trace_id.clone(),
            now_unix: context.now_unix,
            started: false,
            response_id: None,
            model: None,
            created_at: None,
            text: String::new(),
            status: "completed".to_string(),
            input_tokens: 0,
            output_tokens: 0,
            total_tokens: 0,
        }
    }
}

impl StreamTransformer for ResponsesStreamTransformer {
    fn transform_chunk(&mut self, chunk: Bytes) -> Result<Vec<StreamFrame>, ProtocolError> {
        let mut frames = Vec::new();
        for record in self.decoder.push(chunk)? {
            let _event = record.event.as_deref();
            if record.data.trim() == "[DONE]" {
                frames.extend(self.finish()?);
                continue;
            }
            let value: Value = serde_json::from_str(&record.data).map_err(|e| {
                ProtocolError::stream(format!("invalid canonical stream JSON: {e}"))
            })?;
            if !self.started {
                self.started = true;
                self.response_id = Some(
                    value
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or(&self.trace_id)
                        .to_string(),
                );
                self.model = Some(
                    value
                        .get("model")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                        .to_string(),
                );
                self.created_at = Some(
                    value
                        .get("created")
                        .and_then(Value::as_i64)
                        .unwrap_or(self.now_unix),
                );
                frames.push(frame(
                    "response.created",
                    response_value(self, "in_progress"),
                ));
            }
            if let Some(text) = value
                .pointer("/choices/0/delta/content")
                .and_then(Value::as_str)
            {
                self.text.push_str(text);
                frames.push(frame("response.output_text.delta", json!({
                    "type": "response.output_text.delta",
                    "response_id": format!("resp_{}", self.response_id.as_deref().unwrap_or(&self.trace_id)),
                    "output_index": 0,
                    "content_index": 0,
                    "delta": text
                })));
            }
            if let Some(reason) = value
                .pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
            {
                self.status = match reason {
                    "length" => "incomplete",
                    _ => "completed",
                }
                .to_string();
            }
            if let Some(usage) = value.get("usage") {
                self.input_tokens = usage
                    .get("prompt_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.input_tokens);
                self.output_tokens = usage
                    .get("completion_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(self.output_tokens);
                self.total_tokens = usage
                    .get("total_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| self.input_tokens.saturating_add(self.output_tokens));
            }
        }
        Ok(frames)
    }

    fn finish(&mut self) -> Result<Vec<StreamFrame>, ProtocolError> {
        self.decoder.finish()?;
        if !self.started {
            return Ok(Vec::new());
        }
        let frames = vec![
            frame(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "response_id": format!("resp_{}", self.response_id.as_deref().unwrap_or(&self.trace_id)),
                    "output_index": 0,
                    "content_index": 0,
                    "text": self.text
                }),
            ),
            frame("response.completed", response_value(self, &self.status)),
        ];
        self.started = false;
        Ok(frames)
    }
}

fn response_value(state: &ResponsesStreamTransformer, status: &str) -> Value {
    json!({
        "type": format!("response.{status}"),
        "response": {
            "id": format!("resp_{}", state.response_id.as_deref().unwrap_or(&state.trace_id)),
            "object": "response",
            "created_at": state.created_at.unwrap_or(state.now_unix),
            "status": status,
            "model": state.model.as_deref().unwrap_or("unknown"),
            "output": [{
                "type": "message",
                "id": format!("msg_{}", state.response_id.as_deref().unwrap_or(&state.trace_id)),
                "status": status,
                "role": "assistant",
                "content": [{"type": "output_text", "text": state.text, "annotations": []}]
            }],
            "output_text": state.text,
            "usage": {
                "input_tokens": state.input_tokens,
                "output_tokens": state.output_tokens,
                "total_tokens": state.total_tokens
            }
        }
    })
}

fn frame(event: &str, data: Value) -> StreamFrame {
    StreamFrame {
        bytes: Bytes::from(format!("event: {event}\ndata: {data}\n\n")),
    }
}
