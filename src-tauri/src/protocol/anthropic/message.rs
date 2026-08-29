use serde_json::{Map, Value, json};

use crate::protocol::report::ConversionWarningKind;
use crate::protocol::scalars::{
    TokenLimit, object, optional_bool, optional_number_value, required_non_empty_string,
};
use crate::protocol::{
    ConversionContext, ConversionReport, ConvertedRequest, ConvertedResponse, ProtocolError,
    ProtocolKind,
};

const ALLOWED_TOP_LEVEL: &[&str] = &[
    "model",
    "messages",
    "system",
    "max_tokens",
    "stream",
    "temperature",
    "top_p",
    "stop_sequences",
    "tools",
    "tool_choice",
    "metadata",
];

const UNSUPPORTED_TOP_LEVEL: &[&str] = &[
    "thinking",
    "redacted_thinking",
    "container",
    "context_management",
    "mcp_servers",
    "service_tier",
    "top_k",
];

pub(crate) fn to_openai_chat(
    request: Value,
    _context: &ConversionContext,
) -> Result<ConvertedRequest, ProtocolError> {
    let root = request
        .as_object()
        .ok_or_else(|| ProtocolError::malformed("request body must be a JSON object"))?;
    reject_top_level(root)?;
    let mut report =
        ConversionReport::new(ProtocolKind::AnthropicMessages, ProtocolKind::OpenAiChat);
    let model = required_non_empty_string(&request, "/model")?;
    let stream = optional_bool(&request, "/stream")?;
    let max_tokens = match request.pointer("/max_tokens") {
        Some(value) => TokenLimit::parse(value, "/max_tokens")?,
        None => return Err(ProtocolError::missing("/max_tokens")),
    };
    let mut body = Map::new();
    body.insert("model".to_string(), json!(model));
    body.insert("max_tokens".to_string(), max_tokens.into_value());
    if stream {
        body.insert("stream".to_string(), json!(true));
        body.insert("stream_options".to_string(), json!({"include_usage": true}));
    }
    copy_sampling(&request, &mut body)?;
    copy_stop_sequences(&request, &mut body)?;
    let mut messages = Vec::new();
    if let Some(system) = request.get("system") {
        let content = text_content(system, "/system", &mut report)?;
        messages.push(json!({"role": "system", "content": content}));
        report.map("/system", "/messages/0");
    }
    let source_messages = request
        .get("messages")
        .ok_or_else(|| ProtocolError::missing("/messages"))?
        .as_array()
        .ok_or_else(|| ProtocolError::invalid("/messages", "messages must be an array"))?;
    for (i, message) in source_messages.iter().enumerate() {
        append_message(message, i, &mut messages, &mut report)?;
    }
    body.insert("messages".to_string(), Value::Array(messages));
    if let Some(tools) = request.get("tools") {
        body.insert("tools".to_string(), convert_tools(tools, &mut report)?);
    }
    if stream && body.contains_key("tools") {
        return Err(ProtocolError::unsupported(
            "/tools",
            "streaming tool calls are not supported in the first version",
        ));
    }
    if let Some(choice) = request.get("tool_choice") {
        let converted = convert_tool_choice(choice)?;
        if stream && converted != json!("auto") && converted != json!("none") {
            return Err(ProtocolError::unsupported(
                "/tool_choice",
                "streaming tool calls are not supported in the first version",
            ));
        }
        body.insert("tool_choice".to_string(), converted);
    }
    if request.get("metadata").is_some() {
        report.warn(
            ConversionWarningKind::StrippedAnnotation,
            "/metadata",
            "metadata is accepted but not forwarded to canonical chat",
        );
    }
    Ok(ConvertedRequest {
        body: Value::Object(body),
        stream,
        report,
    })
}

pub(crate) fn from_openai_chat(
    response: Value,
    context: &ConversionContext,
) -> Result<ConvertedResponse, ProtocolError> {
    let mut report =
        ConversionReport::new(ProtocolKind::OpenAiChat, ProtocolKind::AnthropicMessages);
    let choice = first_choice(&response)?;
    let message = choice
        .get("message")
        .ok_or_else(|| ProtocolError::invalid("/choices/0/message", "message is required"))?;
    let canonical_id = non_empty_str(response.get("id")).unwrap_or(&context.trace_id);
    let content = message.get("content").and_then(Value::as_str).unwrap_or("");
    let mut blocks = Vec::new();
    if !content.is_empty() {
        blocks.push(json!({"type": "text", "text": content}));
    }
    if let Some(tool_calls) = message.get("tool_calls") {
        blocks.extend(convert_tool_calls(tool_calls)?);
    }
    let (input_tokens, output_tokens) = usage_tokens(&response, &mut report)?;
    let stop_reason = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("stop") => Some("end_turn"),
        Some("length") => Some("max_tokens"),
        Some("tool_calls") => Some("tool_use"),
        None => None,
        Some("content_filter") => {
            return Err(ProtocolError::unsupported(
                "/choices/0/finish_reason",
                "content_filter cannot be represented as Anthropic stop_reason",
            ));
        }
        Some(other) => {
            return Err(ProtocolError::unsupported(
                "/choices/0/finish_reason",
                format!("unknown finish_reason {other}"),
            ));
        }
    };
    Ok(ConvertedResponse {
        body: json!({
            "id": format!("msg_{canonical_id}"),
            "type": "message",
            "role": "assistant",
            "model": response.get("model").and_then(Value::as_str).unwrap_or("unknown"),
            "content": blocks,
            "stop_reason": stop_reason,
            "stop_sequence": Value::Null,
            "usage": {"input_tokens": input_tokens, "output_tokens": output_tokens},
        }),
        report,
    })
}

fn reject_top_level(root: &Map<String, Value>) -> Result<(), ProtocolError> {
    for key in root.keys() {
        if UNSUPPORTED_TOP_LEVEL.contains(&key.as_str()) {
            return Err(ProtocolError::unsupported(
                format!("/{key}"),
                "field is not supported by the canonical chat subset",
            ));
        }
        if !ALLOWED_TOP_LEVEL.contains(&key.as_str()) {
            return Err(ProtocolError::unsupported(
                format!("/{key}"),
                "unknown top-level field",
            ));
        }
    }
    Ok(())
}

fn copy_sampling(request: &Value, body: &mut Map<String, Value>) -> Result<(), ProtocolError> {
    if let Some(value) = optional_number_value(request, "/temperature")? {
        body.insert("temperature".to_string(), value);
    }
    if let Some(value) = optional_number_value(request, "/top_p")? {
        body.insert("top_p".to_string(), value);
    }
    Ok(())
}

fn copy_stop_sequences(
    request: &Value,
    body: &mut Map<String, Value>,
) -> Result<(), ProtocolError> {
    let Some(value) = request.get("stop_sequences") else {
        return Ok(());
    };
    let items = value.as_array().ok_or_else(|| {
        ProtocolError::invalid("/stop_sequences", "stop_sequences must be an array")
    })?;
    for (i, item) in items.iter().enumerate() {
        if !item.is_string() {
            return Err(ProtocolError::invalid(
                format!("/stop_sequences/{i}"),
                "stop sequence must be a string",
            ));
        }
    }
    body.insert("stop".to_string(), value.clone());
    Ok(())
}

fn append_message(
    message: &Value,
    index: usize,
    out: &mut Vec<Value>,
    report: &mut ConversionReport,
) -> Result<(), ProtocolError> {
    let role_path = format!("/messages/{index}/role");
    let role = message
        .get("role")
        .and_then(Value::as_str)
        .ok_or_else(|| ProtocolError::invalid(&role_path, "role must be a string"))?;
    let content_path = format!("/messages/{index}/content");
    let content = message
        .get("content")
        .ok_or_else(|| ProtocolError::missing(&content_path))?;
    match role {
        "user" => append_user_content(content, &content_path, out),
        "assistant" => append_assistant_content(content, &content_path, out),
        "system" => {
            out.push(
                json!({"role": "system", "content": text_content(content, &content_path, report)?}),
            );
            Ok(())
        }
        _ => Err(ProtocolError::invalid(
            role_path,
            "unsupported message role",
        )),
    }
}

fn append_user_content(
    content: &Value,
    path: &str,
    out: &mut Vec<Value>,
) -> Result<(), ProtocolError> {
    if let Some(text) = content.as_str() {
        out.push(json!({"role": "user", "content": text}));
        return Ok(());
    }
    let blocks = content
        .as_array()
        .ok_or_else(|| ProtocolError::invalid(path, "content must be a string or array"))?;
    let mut texts = Vec::new();
    let mut tool_messages = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}/{i}");
        match block.get("type").and_then(Value::as_str) {
            Some("text") => texts.push(required_block_text(block, &block_path)?),
            Some("tool_result") => {
                let id = block
                    .get("tool_use_id")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| ProtocolError::missing(format!("{block_path}/tool_use_id")))?;
                let text =
                    tool_result_text(block.get("content"), &format!("{block_path}/content"))?;
                let content = if block
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    format!("Tool error: {text}")
                } else {
                    text
                };
                tool_messages.push(json!({"role": "tool", "tool_call_id": id, "content": content}));
            }
            Some(other) => {
                return Err(ProtocolError::unsupported(
                    block_path,
                    format!("unsupported user block {other}"),
                ));
            }
            None => {
                return Err(ProtocolError::invalid(
                    block_path,
                    "content block type is required",
                ));
            }
        }
    }
    out.extend(tool_messages);
    if !texts.is_empty() {
        out.push(json!({"role": "user", "content": texts.join("\n\n")}));
    }
    Ok(())
}

fn append_assistant_content(
    content: &Value,
    path: &str,
    out: &mut Vec<Value>,
) -> Result<(), ProtocolError> {
    if let Some(text) = content.as_str() {
        out.push(json!({"role": "assistant", "content": text}));
        return Ok(());
    }
    let blocks = content
        .as_array()
        .ok_or_else(|| ProtocolError::invalid(path, "content must be a string or array"))?;
    let mut texts = Vec::new();
    let mut tool_calls = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}/{i}");
        match block.get("type").and_then(Value::as_str) {
            Some("text") => texts.push(required_block_text(block, &block_path)?),
            Some("tool_use") => tool_calls.push(convert_tool_use(block, &block_path)?),
            Some(other) => {
                return Err(ProtocolError::unsupported(
                    block_path,
                    format!("unsupported assistant block {other}"),
                ));
            }
            None => {
                return Err(ProtocolError::invalid(
                    block_path,
                    "content block type is required",
                ));
            }
        }
    }
    let mut msg = Map::new();
    msg.insert("role".to_string(), json!("assistant"));
    if !texts.is_empty() {
        msg.insert("content".to_string(), json!(texts.join("\n\n")));
    }
    if !tool_calls.is_empty() {
        msg.insert("tool_calls".to_string(), Value::Array(tool_calls));
    }
    out.push(Value::Object(msg));
    Ok(())
}

fn text_content(
    value: &Value,
    path: &str,
    report: &mut ConversionReport,
) -> Result<String, ProtocolError> {
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    let blocks = value.as_array().ok_or_else(|| {
        ProtocolError::invalid(path, "text content must be a string or text block array")
    })?;
    let mut texts = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}/{i}");
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if block.get("cache_control").is_some() {
                    report.warn(
                        ConversionWarningKind::StrippedAnnotation,
                        format!("{block_path}/cache_control"),
                        "cache_control is stripped",
                    );
                }
                texts.push(required_block_text(block, &block_path)?);
            }
            Some("cache_control") => {
                return Err(ProtocolError::unsupported(
                    block_path,
                    "standalone cache_control block",
                ));
            }
            Some(other) => {
                return Err(ProtocolError::unsupported(
                    block_path,
                    format!("unsupported block {other}"),
                ));
            }
            None => {
                return Err(ProtocolError::invalid(
                    block_path,
                    "content block type is required",
                ));
            }
        }
    }
    Ok(texts.join("\n\n"))
}

fn required_block_text(block: &Value, path: &str) -> Result<String, ProtocolError> {
    block
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ProtocolError::invalid(format!("{path}/text"), "text must be a string"))
}

fn tool_result_text(value: Option<&Value>, path: &str) -> Result<String, ProtocolError> {
    let Some(value) = value else {
        return Ok(String::new());
    };
    if let Some(text) = value.as_str() {
        return Ok(text.to_string());
    }
    let blocks = value
        .as_array()
        .ok_or_else(|| ProtocolError::invalid(path, "tool_result content must be text"))?;
    let mut texts = Vec::new();
    for (i, block) in blocks.iter().enumerate() {
        let block_path = format!("{path}/{i}");
        match block.get("type").and_then(Value::as_str) {
            Some("text") => texts.push(required_block_text(block, &block_path)?),
            Some(other) => {
                return Err(ProtocolError::unsupported(
                    block_path,
                    format!("unsupported tool_result block {other}"),
                ));
            }
            None => {
                return Err(ProtocolError::invalid(
                    block_path,
                    "content block type is required",
                ));
            }
        }
    }
    Ok(texts.join("\n\n"))
}

fn convert_tool_use(block: &Value, path: &str) -> Result<Value, ProtocolError> {
    let id = block
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProtocolError::missing(format!("{path}/id")))?;
    let name = block
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProtocolError::missing(format!("{path}/name")))?;
    let input = block
        .get("input")
        .ok_or_else(|| ProtocolError::missing(format!("{path}/input")))?;
    object(input, &format!("{path}/input"))?;
    Ok(json!({
        "id": id,
        "type": "function",
        "function": {"name": name, "arguments": input.to_string()}
    }))
}

fn convert_tools(tools: &Value, report: &mut ConversionReport) -> Result<Value, ProtocolError> {
    let tools = tools
        .as_array()
        .ok_or_else(|| ProtocolError::invalid("/tools", "tools must be an array"))?;
    let mut out = Vec::new();
    for (i, tool) in tools.iter().enumerate() {
        let path = format!("/tools/{i}");
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ProtocolError::missing(format!("{path}/name")))?;
        let mut function = Map::new();
        function.insert("name".to_string(), json!(name));
        if let Some(desc) = tool.get("description") {
            if !desc.is_string() {
                return Err(ProtocolError::invalid(
                    format!("{path}/description"),
                    "description must be a string",
                ));
            }
            function.insert("description".to_string(), desc.clone());
        }
        let schema = tool
            .get("input_schema")
            .ok_or_else(|| ProtocolError::missing(format!("{path}/input_schema")))?;
        let mut schema = object(schema, &format!("{path}/input_schema"))?.clone();
        if !schema.contains_key("type") {
            schema.insert("type".to_string(), json!("object"));
            report.warn(
                ConversionWarningKind::NormalizedField,
                format!("{path}/input_schema/type"),
                "missing schema type normalized to object",
            );
        }
        function.insert("parameters".to_string(), Value::Object(schema));
        out.push(json!({"type": "function", "function": function}));
    }
    Ok(Value::Array(out))
}

fn convert_tool_choice(choice: &Value) -> Result<Value, ProtocolError> {
    if let Some(s) = choice.as_str() {
        return match s {
            "auto" => Ok(json!("auto")),
            "any" => Ok(json!("required")),
            "none" => Ok(json!("none")),
            "tool" => Err(ProtocolError::invalid(
                "/tool_choice",
                "tool choice requires a name",
            )),
            _ => Err(ProtocolError::invalid(
                "/tool_choice",
                "unknown tool_choice value",
            )),
        };
    }
    let ty = choice.get("type").and_then(Value::as_str).ok_or_else(|| {
        ProtocolError::invalid("/tool_choice/type", "tool_choice type is required")
    })?;
    match ty {
        "auto" => Ok(json!("auto")),
        "any" => Ok(json!("required")),
        "none" => Ok(json!("none")),
        "tool" => {
            let name = choice
                .get("name")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| ProtocolError::missing("/tool_choice/name"))?;
            Ok(json!({"type": "function", "function": {"name": name}}))
        }
        _ => Err(ProtocolError::invalid(
            "/tool_choice/type",
            "unknown tool_choice type",
        )),
    }
}

pub(crate) fn first_choice(response: &Value) -> Result<&Value, ProtocolError> {
    let choices = response
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| ProtocolError::invalid("/choices", "choices must be a non-empty array"))?;
    choices
        .first()
        .ok_or_else(|| ProtocolError::invalid("/choices", "choices must be a non-empty array"))
}

pub(crate) fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub(crate) fn usage_tokens(
    response: &Value,
    report: &mut ConversionReport,
) -> Result<(u64, u64), ProtocolError> {
    let Some(usage) = response.get("usage") else {
        report.warn(
            ConversionWarningKind::NormalizedField,
            "/usage",
            "usage missing; emitted zero tokens",
        );
        return Ok((0, 0));
    };
    let input = number_field(usage, "prompt_tokens", "/usage/prompt_tokens")?.unwrap_or(0);
    let output = number_field(usage, "completion_tokens", "/usage/completion_tokens")?.unwrap_or(0);
    Ok((input, output))
}

pub(crate) fn usage_triplet(
    response: &Value,
    report: &mut ConversionReport,
) -> Result<(u64, u64, u64), ProtocolError> {
    let Some(usage) = response.get("usage") else {
        report.warn(
            ConversionWarningKind::NormalizedField,
            "/usage",
            "usage missing; emitted zero tokens",
        );
        return Ok((0, 0, 0));
    };
    let input = number_field(usage, "prompt_tokens", "/usage/prompt_tokens")?.unwrap_or(0);
    let output = number_field(usage, "completion_tokens", "/usage/completion_tokens")?.unwrap_or(0);
    let total = match number_field(usage, "total_tokens", "/usage/total_tokens")? {
        Some(total) => total,
        None => {
            report.warn(
                ConversionWarningKind::NormalizedField,
                "/usage/total_tokens",
                "total_tokens derived from prompt_tokens + completion_tokens",
            );
            input.saturating_add(output)
        }
    };
    Ok((input, output, total))
}

fn number_field(value: &Value, name: &str, path: &str) -> Result<Option<u64>, ProtocolError> {
    match value.get(name) {
        None => Ok(None),
        Some(v) => v
            .as_u64()
            .map(Some)
            .ok_or_else(|| ProtocolError::invalid(path, "token field must be an integer")),
    }
}

pub(crate) fn convert_tool_calls(tool_calls: &Value) -> Result<Vec<Value>, ProtocolError> {
    let calls = tool_calls.as_array().ok_or_else(|| {
        ProtocolError::invalid(
            "/choices/0/message/tool_calls",
            "tool_calls must be an array",
        )
    })?;
    let mut out = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let function = call.get("function").ok_or_else(|| {
            ProtocolError::missing(format!("/choices/0/message/tool_calls/{i}/function"))
        })?;
        let name = function
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                ProtocolError::missing(format!("/choices/0/message/tool_calls/{i}/function/name"))
            })?;
        let input = parse_arguments(
            function.get("arguments"),
            &format!("/choices/0/message/tool_calls/{i}/function/arguments"),
        )?;
        out.push(json!({
            "type": "tool_use",
            "id": non_empty_str(call.get("id")).unwrap_or("tool_call"),
            "name": name,
            "input": input,
        }));
    }
    Ok(out)
}

pub(crate) fn parse_arguments(value: Option<&Value>, path: &str) -> Result<Value, ProtocolError> {
    let Some(value) = value else {
        return Ok(json!({}));
    };
    let Some(text) = value.as_str() else {
        return Err(ProtocolError::invalid(
            path,
            "function arguments must be a string",
        ));
    };
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    let parsed: Value = serde_json::from_str(text).map_err(|e| {
        ProtocolError::invalid(
            path,
            format!("function arguments must be valid JSON object: {e}"),
        )
    })?;
    if !parsed.is_object() {
        return Err(ProtocolError::invalid(
            path,
            "function arguments must decode to an object",
        ));
    }
    Ok(parsed)
}
