use serde_json::{Map, Value, json};

use crate::protocol::anthropic::message::{
    first_choice, non_empty_str, parse_arguments, usage_triplet,
};
use crate::protocol::report::ConversionWarningKind;
use crate::protocol::scalars::{
    TokenLimit, object, optional_bool, optional_number_value, required_non_empty_string,
};
use crate::protocol::{
    ConversionContext, ConversionReport, ConvertedRequest, ConvertedResponse, ProtocolError,
    ProtocolKind,
};

pub(crate) fn to_openai_chat(
    request: Value,
    _context: &ConversionContext,
) -> Result<ConvertedRequest, ProtocolError> {
    if !request.is_object() {
        return Err(ProtocolError::malformed(
            "request body must be a JSON object",
        ));
    }
    reject_stateful(&request)?;
    let mut report = ConversionReport::new(ProtocolKind::OpenAiResponses, ProtocolKind::OpenAiChat);
    let model = required_non_empty_string(&request, "/model")?;
    let stream = optional_bool(&request, "/stream")?;
    let mut body = Map::new();
    body.insert("model".to_string(), json!(model));
    if stream {
        body.insert("stream".to_string(), json!(true));
        body.insert("stream_options".to_string(), json!({"include_usage": true}));
    }
    if let Some(value) = optional_number_value(&request, "/temperature")? {
        body.insert("temperature".to_string(), value);
    }
    if let Some(value) = optional_number_value(&request, "/top_p")? {
        body.insert("top_p".to_string(), value);
    }
    if let Some(value) = request.get("max_output_tokens") {
        body.insert(
            "max_tokens".to_string(),
            TokenLimit::parse(value, "/max_output_tokens")?.into_value(),
        );
    }
    let mut messages = Vec::new();
    if let Some(instructions) = request.get("instructions") {
        let text = instructions.as_str().ok_or_else(|| {
            ProtocolError::invalid("/instructions", "instructions must be a string")
        })?;
        messages.push(json!({"role": "system", "content": text}));
    }
    let input = request
        .get("input")
        .ok_or_else(|| ProtocolError::missing("/input"))?;
    append_input(input, &mut messages)?;
    body.insert("messages".to_string(), Value::Array(messages));
    if let Some(tools) = request.get("tools") {
        body.insert("tools".to_string(), convert_tools(tools)?);
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
    if matches!(request.get("store"), Some(Value::Bool(false))) {
        report.warn(
            ConversionWarningKind::StrippedAnnotation,
            "/store",
            "store=false is accepted but not forwarded",
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
    let mut report = ConversionReport::new(ProtocolKind::OpenAiChat, ProtocolKind::OpenAiResponses);
    let choice = first_choice(&response)?;
    let message = choice
        .get("message")
        .ok_or_else(|| ProtocolError::invalid("/choices/0/message", "message is required"))?;
    let canonical_id = non_empty_str(response.get("id")).unwrap_or(&context.trace_id);
    let created_at = match response.get("created") {
        Some(value) => value.as_i64().ok_or_else(|| {
            ProtocolError::invalid("/created", "created must be an integer timestamp")
        })?,
        None => context.now_unix,
    };
    let status = match choice.get("finish_reason").and_then(Value::as_str) {
        Some("stop") | Some("tool_calls") | None => "completed",
        Some("length") => "incomplete",
        Some("content_filter") => {
            return Err(ProtocolError::unsupported(
                "/choices/0/finish_reason",
                "content_filter cannot be represented as Responses status",
            ));
        }
        Some(other) => {
            return Err(ProtocolError::unsupported(
                "/choices/0/finish_reason",
                format!("unknown finish_reason {other}"),
            ));
        }
    };
    let text = message.get("content").and_then(Value::as_str).unwrap_or("");
    let mut output = Vec::new();
    output.push(json!({
        "type": "message",
        "id": format!("msg_{canonical_id}"),
        "status": status,
        "role": "assistant",
        "content": [{"type": "output_text", "text": text, "annotations": []}]
    }));
    if let Some(tool_calls) = message.get("tool_calls") {
        output.extend(convert_tool_calls(tool_calls)?);
    }
    let (input_tokens, output_tokens, total_tokens) = usage_triplet(&response, &mut report)?;
    Ok(ConvertedResponse {
        body: json!({
            "id": format!("resp_{canonical_id}"),
            "object": "response",
            "created_at": created_at,
            "status": status,
            "model": response.get("model").and_then(Value::as_str).unwrap_or("unknown"),
            "output": output,
            "output_text": text,
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "total_tokens": total_tokens
            }
        }),
        report,
    })
}

fn reject_stateful(request: &Value) -> Result<(), ProtocolError> {
    for key in ["background", "previous_response_id", "conversation"] {
        if request.get(key).is_some() {
            return Err(ProtocolError::unsupported(
                format!("/{key}"),
                "stateful Responses features are not supported",
            ));
        }
    }
    match request.get("store") {
        Some(Value::Bool(true)) => Err(ProtocolError::unsupported(
            "/store",
            "stored Responses are not supported",
        )),
        Some(Value::Bool(false)) | None => Ok(()),
        Some(_) => Err(ProtocolError::invalid("/store", "store must be a boolean")),
    }
}

fn append_input(input: &Value, messages: &mut Vec<Value>) -> Result<(), ProtocolError> {
    if let Some(text) = input.as_str() {
        messages.push(json!({"role": "user", "content": text}));
        return Ok(());
    }
    let items = input.as_array().ok_or_else(|| {
        ProtocolError::invalid("/input", "input must be a string or message array")
    })?;
    for (i, item) in items.iter().enumerate() {
        let path = format!("/input/{i}");
        let ty = item.get("type").and_then(Value::as_str);
        if matches!(ty, Some("function_call" | "function_call_output")) {
            return Err(ProtocolError::unsupported(
                path,
                "Responses function call items are not accepted as input",
            ));
        }
        let role = item.get("role").and_then(Value::as_str).ok_or_else(|| {
            ProtocolError::invalid(format!("{path}/role"), "role must be a string")
        })?;
        let content = item
            .get("content")
            .ok_or_else(|| ProtocolError::missing(format!("{path}/content")))?;
        let text = response_input_text(content, &format!("{path}/content"))?;
        messages.push(json!({"role": role, "content": text}));
    }
    Ok(())
}

fn response_input_text(content: &Value, path: &str) -> Result<String, ProtocolError> {
    if let Some(text) = content.as_str() {
        return Ok(text.to_string());
    }
    let parts = content.as_array().ok_or_else(|| {
        ProtocolError::invalid(path, "content must be a string or content part array")
    })?;
    let mut texts = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        let part_path = format!("{path}/{i}");
        match part.get("type").and_then(Value::as_str) {
            Some("input_text") => {
                let text = part.get("text").and_then(Value::as_str).ok_or_else(|| {
                    ProtocolError::invalid(format!("{part_path}/text"), "text must be a string")
                })?;
                texts.push(text);
            }
            Some(other) => {
                return Err(ProtocolError::unsupported(
                    part_path,
                    format!("unsupported Responses content part {other}"),
                ));
            }
            None => {
                return Err(ProtocolError::invalid(
                    part_path,
                    "content part type is required",
                ));
            }
        }
    }
    Ok(texts.join("\n\n"))
}

fn convert_tools(tools: &Value) -> Result<Value, ProtocolError> {
    let tools = tools
        .as_array()
        .ok_or_else(|| ProtocolError::invalid("/tools", "tools must be an array"))?;
    let mut out = Vec::new();
    for (i, tool) in tools.iter().enumerate() {
        let path = format!("/tools/{i}");
        let ty = tool.get("type").and_then(Value::as_str).ok_or_else(|| {
            ProtocolError::invalid(format!("{path}/type"), "tool type is required")
        })?;
        if ty != "function" {
            return Err(ProtocolError::unsupported(
                path,
                format!("unsupported Responses tool type {ty}"),
            ));
        }
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ProtocolError::missing(format!("{path}/name")))?;
        let parameters = tool
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"}));
        object(&parameters, &format!("{path}/parameters"))?;
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
        function.insert("parameters".to_string(), parameters);
        out.push(json!({"type": "function", "function": function}));
    }
    Ok(Value::Array(out))
}

fn convert_tool_choice(choice: &Value) -> Result<Value, ProtocolError> {
    if let Some(s) = choice.as_str() {
        return match s {
            "auto" | "none" | "required" => Ok(json!(s)),
            _ => Err(ProtocolError::invalid(
                "/tool_choice",
                "unknown tool_choice value",
            )),
        };
    }
    let ty = choice.get("type").and_then(Value::as_str).ok_or_else(|| {
        ProtocolError::invalid("/tool_choice/type", "tool_choice type is required")
    })?;
    if ty != "function" {
        return Err(ProtocolError::invalid(
            "/tool_choice/type",
            "unknown tool_choice type",
        ));
    }
    let name = choice
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ProtocolError::missing("/tool_choice/name"))?;
    Ok(json!({"type": "function", "function": {"name": name}}))
}

fn convert_tool_calls(tool_calls: &Value) -> Result<Vec<Value>, ProtocolError> {
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
        let args = parse_arguments(
            function.get("arguments"),
            &format!("/choices/0/message/tool_calls/{i}/function/arguments"),
        )?;
        out.push(json!({
            "type": "function_call",
            "id": non_empty_str(call.get("id")).unwrap_or("tool_call"),
            "call_id": non_empty_str(call.get("id")).unwrap_or("tool_call"),
            "name": name,
            "arguments": args.to_string(),
            "status": "completed"
        }));
    }
    Ok(out)
}
