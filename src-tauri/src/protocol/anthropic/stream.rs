use bytes::Bytes;
use serde_json::{Value, json};

use crate::protocol::sse::SseDecoder;
use crate::protocol::{ConversionContext, ProtocolError, StreamFrame, StreamTransformer};

pub(crate) struct AnthropicStreamTransformer {
    decoder: SseDecoder,
    trace_id: String,
    started: bool,
    block_open: bool,
    model: Option<String>,
    message_id: Option<String>,
    output_tokens: u64,
    final_stop_reason: Option<String>,
}

impl AnthropicStreamTransformer {
    pub(crate) fn new(context: &ConversionContext) -> Self {
        Self {
            decoder: SseDecoder::new(),
            trace_id: context.trace_id.clone(),
            started: false,
            block_open: false,
            model: None,
            message_id: None,
            output_tokens: 0,
            final_stop_reason: None,
        }
    }
}

impl StreamTransformer for AnthropicStreamTransformer {
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
                self.message_id = Some(
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
                frames.push(frame("message_start", json!({
                    "type": "message_start",
                    "message": {
                        "id": format!("msg_{}", self.message_id.as_deref().unwrap_or(&self.trace_id)),
                        "type": "message",
                        "role": "assistant",
                        "model": self.model.as_deref().unwrap_or("unknown"),
                        "content": [],
                        "stop_reason": Value::Null,
                        "stop_sequence": Value::Null,
                        "usage": {"input_tokens": 0, "output_tokens": 0}
                    }
                })));
            }
            let choice = value.pointer("/choices/0");
            if let Some(text) = choice
                .and_then(|c| c.pointer("/delta/content"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                if !self.block_open {
                    self.block_open = true;
                    frames.push(frame(
                        "content_block_start",
                        json!({
                            "type": "content_block_start",
                            "index": 0,
                            "content_block": {"type": "text", "text": ""}
                        }),
                    ));
                }
                frames.push(frame(
                    "content_block_delta",
                    json!({
                        "type": "content_block_delta",
                        "index": 0,
                        "delta": {"type": "text_delta", "text": text}
                    }),
                ));
            }
            if let Some(reason) = choice
                .and_then(|c| c.get("finish_reason"))
                .and_then(Value::as_str)
            {
                self.final_stop_reason = Some(
                    match reason {
                        "stop" => "end_turn",
                        "length" => "max_tokens",
                        "tool_calls" => "tool_use",
                        other => other,
                    }
                    .to_string(),
                );
            }
            if let Some(tokens) = value
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_u64)
            {
                self.output_tokens = tokens;
            }
        }
        Ok(frames)
    }

    fn finish(&mut self) -> Result<Vec<StreamFrame>, ProtocolError> {
        self.decoder.finish()?;
        let mut frames = Vec::new();
        if self.block_open {
            self.block_open = false;
            frames.push(frame(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 0}),
            ));
        }
        if self.started {
            frames.push(frame("message_delta", json!({
                "type": "message_delta",
                "delta": {
                    "stop_reason": self.final_stop_reason.clone().unwrap_or_else(|| "end_turn".to_string()),
                    "stop_sequence": Value::Null
                },
                "usage": {"output_tokens": self.output_tokens}
            })));
            frames.push(frame("message_stop", json!({"type": "message_stop"})));
            self.started = false;
        }
        Ok(frames)
    }
}

fn frame(event: &str, data: Value) -> StreamFrame {
    StreamFrame {
        bytes: Bytes::from(format!("event: {event}\ndata: {data}\n\n")),
    }
}
