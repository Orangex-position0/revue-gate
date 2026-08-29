use bytes::Bytes;
use revue_gate_lib::protocol::{CodecRegistry, ConversionContext, ProtocolError, ProtocolKind};
use serde_json::json;

fn ctx() -> ConversionContext {
    ConversionContext {
        trace_id: "trace-test".to_string(),
        now_unix: 1_700_000_000,
    }
}

#[test]
fn anthropic_messages_request_converts_to_canonical_chat() {
    let converted = CodecRegistry::convert_request(
        ProtocolKind::AnthropicMessages,
        json!({
            "model": "gpt-4o",
            "system": [{"type": "text", "text": "be brief"}],
            "max_tokens": 128,
            "temperature": 0.2,
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{
                "name": "weather",
                "description": "get weather",
                "input_schema": {"properties": {"city": {"type": "string"}}}
            }],
            "tool_choice": {"type": "tool", "name": "weather"}
        }),
        &ctx(),
    )
    .expect("convert");

    assert!(!converted.stream);
    assert_eq!(converted.body["model"], "gpt-4o");
    assert_eq!(converted.body["max_tokens"], 128);
    assert_eq!(
        converted.body["messages"][0],
        json!({"role": "system", "content": "be brief"})
    );
    assert_eq!(
        converted.body["messages"][1],
        json!({"role": "user", "content": "hi"})
    );
    assert_eq!(
        converted.body["tools"][0]["function"]["parameters"]["type"],
        "object"
    );
    assert_eq!(
        converted.body["tool_choice"],
        json!({"type": "function", "function": {"name": "weather"}})
    );
}

#[test]
fn anthropic_messages_response_converts_from_canonical_chat() {
    let converted = CodecRegistry::convert_response(
        ProtocolKind::AnthropicMessages,
        json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }),
        &ctx(),
    )
    .expect("convert");

    assert_eq!(converted.body["id"], "msg_chatcmpl-1");
    assert_eq!(converted.body["type"], "message");
    assert_eq!(
        converted.body["content"][0],
        json!({"type": "text", "text": "hello"})
    );
    assert_eq!(converted.body["stop_reason"], "end_turn");
    assert_eq!(converted.body["usage"]["input_tokens"], 10);
    assert_eq!(converted.body["usage"]["output_tokens"], 5);
}

#[test]
fn responses_request_converts_to_canonical_chat() {
    let converted = CodecRegistry::convert_request(
        ProtocolKind::OpenAiResponses,
        json!({
            "model": "gpt-4o",
            "instructions": "be brief",
            "input": "hi",
            "max_output_tokens": 64,
            "store": false
        }),
        &ctx(),
    )
    .expect("convert");

    assert_eq!(converted.body["model"], "gpt-4o");
    assert_eq!(converted.body["max_tokens"], 64);
    assert_eq!(
        converted.body["messages"][0],
        json!({"role": "system", "content": "be brief"})
    );
    assert_eq!(
        converted.body["messages"][1],
        json!({"role": "user", "content": "hi"})
    );
}

#[test]
fn responses_response_converts_from_canonical_chat() {
    let converted = CodecRegistry::convert_response(
        ProtocolKind::OpenAiResponses,
        json!({
            "id": "chatcmpl-1",
            "created": 1700000001,
            "model": "gpt-4o",
            "choices": [{
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }),
        &ctx(),
    )
    .expect("convert");

    assert_eq!(converted.body["id"], "resp_chatcmpl-1");
    assert_eq!(converted.body["object"], "response");
    assert_eq!(converted.body["status"], "completed");
    assert_eq!(converted.body["output_text"], "hello");
    assert_eq!(converted.body["output"][0]["content"][0]["text"], "hello");
    assert_eq!(converted.body["usage"]["total_tokens"], 15);
}

#[test]
fn unsupported_features_fail_before_proxy() {
    let err = CodecRegistry::convert_request(
        ProtocolKind::OpenAiResponses,
        json!({
            "model": "gpt-4o",
            "input": "hi",
            "background": true
        }),
        &ctx(),
    )
    .expect_err("unsupported");

    assert!(matches!(err, ProtocolError::UnsupportedFeature { .. }));
}

#[test]
fn anthropic_stream_transformer_emits_anthropic_events_without_done() {
    let mut transformer =
        CodecRegistry::stream_transformer(ProtocolKind::AnthropicMessages, &ctx());
    let frames = transformer
        .transform_chunk(Bytes::from_static(
            br#"data: {"id":"chatcmpl-1","model":"gpt-4o","choices":[{"delta":{"content":"he"}}]}"#,
        ))
        .expect("first partial");
    assert!(
        frames.is_empty(),
        "incomplete SSE record should be buffered"
    );

    let frames = transformer
        .transform_chunk(Bytes::from_static(
            b"\n\ndata: {\"id\":\"chatcmpl-1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\ndata: [DONE]\n\n",
        ))
        .expect("finish");
    let text = frames_to_string(frames);

    assert!(text.contains("event: message_start"));
    assert!(text.contains("event: content_block_delta"));
    assert!(text.contains(r#""text":"he""#));
    assert!(text.contains(r#""text":"llo""#));
    assert!(text.contains("event: message_stop"));
    assert!(!text.contains("[DONE]"));
}

#[test]
fn responses_stream_transformer_emits_responses_events_without_done() {
    let mut transformer = CodecRegistry::stream_transformer(ProtocolKind::OpenAiResponses, &ctx());
    let frames = transformer
        .transform_chunk(Bytes::from_static(
            b"data: {\"id\":\"chatcmpl-1\",\"created\":1700000001,\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":1,\"total_tokens\":2}}\n\ndata: [DONE]\n\n",
        ))
        .expect("stream");
    let text = frames_to_string(frames);

    assert!(text.contains("event: response.created"));
    assert!(text.contains("event: response.output_text.delta"));
    assert!(text.contains("event: response.completed"));
    assert!(!text.contains("[DONE]"));
}

fn frames_to_string(frames: Vec<revue_gate_lib::protocol::StreamFrame>) -> String {
    let bytes: Vec<u8> = frames
        .into_iter()
        .flat_map(|frame| frame.bytes.into_iter())
        .collect();
    String::from_utf8(bytes).expect("utf8")
}
