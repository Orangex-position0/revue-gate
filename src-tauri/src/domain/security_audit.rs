//! Security audit domain types: settings, resolved policy, request scope, and request-level risk report.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Runtime audit mode: observe only records risk, enforce may block once detectors exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuditMode {
    #[default]
    Observe,
    Enforce,
}

/// Amount of evidence kept in the structured audit report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuditEvidenceLevel {
    #[default]
    Summary,
    Detailed,
}

/// Security audit settings persisted with the gateway settings snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AuditSettings {
    pub enabled: bool,
    pub mode: AuditMode,
    pub block_critical: bool,
    pub scan_system_messages: bool,
    pub scan_byte_limit: u32,
    pub store_payload: bool,
    pub evidence_level: AuditEvidenceLevel,
}

impl Default for AuditSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: AuditMode::Observe,
            block_critical: true,
            scan_system_messages: false,
            scan_byte_limit: 64 * 1024,
            store_payload: true,
            evidence_level: AuditEvidenceLevel::Summary,
        }
    }
}

/// Resolved audit policy used by the data plane for one logical request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditPolicy {
    pub enabled: bool,
    pub mode: AuditMode,
    pub block_critical: bool,
    pub scan_system_messages: bool,
    pub scan_byte_limit: u32,
    pub store_payload: bool,
    pub evidence_level: AuditEvidenceLevel,
}

impl From<&AuditSettings> for AuditPolicy {
    fn from(settings: &AuditSettings) -> Self {
        Self {
            enabled: settings.enabled,
            mode: settings.mode,
            block_critical: settings.block_critical,
            scan_system_messages: settings.scan_system_messages,
            scan_byte_limit: settings.scan_byte_limit,
            store_payload: settings.store_payload,
            evidence_level: settings.evidence_level,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditScopeKind {
    MessageContent,
    SystemMessageContent,
    ToolCallArguments,
    ToolName,
    ToolDescription,
    ToolSchemaString,
    ToolSchemaKey,
    TopLevelParam,
}

/// One flattened string candidate that detectors may scan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditScopeItem {
    pub path: String,
    pub kind: AuditScopeKind,
    pub text: String,
}

/// Flattened request-side scan scope plus byte accounting for limit decisions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditScope {
    pub items: Vec<AuditScopeItem>,
    pub scanned_bytes: u32,
    pub candidate_bytes: u32,
    pub scan_byte_limit: u32,
    pub truncated: bool,
}

/// Build the deterministic request audit scope for one OpenAI-compatible logical request body.
pub fn build_audit_scope(body: &Value, policy: &AuditPolicy) -> AuditScope {
    let mut candidates = Vec::new();
    collect_user_and_tool_message_content(body, &mut candidates);
    collect_tool_call_arguments(body, &mut candidates);
    collect_tool_definitions(body, &mut candidates);
    collect_top_level_params(body, &mut candidates);
    if policy.scan_system_messages {
        collect_system_message_content(body, &mut candidates);
    }

    let candidate_bytes = sum_text_bytes(&candidates);
    let mut scanned_bytes = 0u32;
    let mut truncated = false;
    let mut items = Vec::new();
    for item in candidates {
        let item_bytes = saturating_u32(item.text.len());
        if scanned_bytes.saturating_add(item_bytes) > policy.scan_byte_limit {
            truncated = true;
            break;
        }
        scanned_bytes = scanned_bytes.saturating_add(item_bytes);
        items.push(item);
    }

    AuditScope {
        items,
        scanned_bytes,
        candidate_bytes,
        scan_byte_limit: policy.scan_byte_limit,
        truncated,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Clean,
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuditAction {
    Allow,
    LogOnly,
    Warn,
    Redact,
    Confirm,
    Block,
}

/// Request-level audit projection stored with the request log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditReport {
    pub mode: AuditMode,
    pub risk_level: RiskLevel,
    pub action: AuditAction,
    pub findings: Vec<AuditFinding>,
    pub scanned_bytes: u32,
    pub candidate_bytes: u32,
    pub scan_byte_limit: u32,
    pub truncated: bool,
    pub evidence_level: AuditEvidenceLevel,
}

/// Detector finding shape reserved for later detector slices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFinding {
    pub rule_id: String,
    pub category: String,
    pub risk_level: RiskLevel,
    pub action: AuditAction,
    pub path: String,
    pub evidence: Option<String>,
}

impl AuditReport {
    pub fn clean(policy: &AuditPolicy) -> Self {
        Self::clean_for_scope(policy, &AuditScope::empty(policy.scan_byte_limit))
    }

    pub fn clean_for_scope(policy: &AuditPolicy, scope: &AuditScope) -> Self {
        Self {
            mode: policy.mode,
            risk_level: RiskLevel::Clean,
            action: AuditAction::Allow,
            findings: Vec::new(),
            scanned_bytes: scope.scanned_bytes,
            candidate_bytes: scope.candidate_bytes,
            scan_byte_limit: scope.scan_byte_limit,
            truncated: scope.truncated,
            evidence_level: policy.evidence_level,
        }
    }
}

impl AuditScope {
    fn empty(scan_byte_limit: u32) -> Self {
        Self {
            items: Vec::new(),
            scanned_bytes: 0,
            candidate_bytes: 0,
            scan_byte_limit,
            truncated: false,
        }
    }
}

fn collect_user_and_tool_message_content(body: &Value, items: &mut Vec<AuditScopeItem>) {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return;
    };
    for (message_index, message) in messages.iter().enumerate() {
        if !matches!(
            message.get("role").and_then(Value::as_str),
            Some("user" | "tool")
        ) {
            continue;
        }
        if let Some(content) = message.get("content") {
            collect_string_leaves(
                content,
                &format!("/messages/{message_index}/content"),
                AuditScopeKind::MessageContent,
                items,
            );
        }
    }
}

fn collect_system_message_content(body: &Value, items: &mut Vec<AuditScopeItem>) {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return;
    };
    for (message_index, message) in messages.iter().enumerate() {
        if message.get("role").and_then(Value::as_str) != Some("system") {
            continue;
        }
        if let Some(content) = message.get("content") {
            collect_string_leaves(
                content,
                &format!("/messages/{message_index}/content"),
                AuditScopeKind::SystemMessageContent,
                items,
            );
        }
    }
}

fn collect_tool_call_arguments(body: &Value, items: &mut Vec<AuditScopeItem>) {
    let Some(messages) = body.get("messages").and_then(Value::as_array) else {
        return;
    };
    for (message_index, message) in messages.iter().enumerate() {
        let Some(tool_calls) = message.get("tool_calls").and_then(Value::as_array) else {
            continue;
        };
        for (call_index, call) in tool_calls.iter().enumerate() {
            if let Some(arguments) = call.pointer("/function/arguments") {
                collect_string_leaves(
                    arguments,
                    &format!(
                        "/messages/{message_index}/tool_calls/{call_index}/function/arguments"
                    ),
                    AuditScopeKind::ToolCallArguments,
                    items,
                );
            }
        }
    }
}

fn collect_tool_definitions(body: &Value, items: &mut Vec<AuditScopeItem>) {
    let Some(tools) = body.get("tools").and_then(Value::as_array) else {
        return;
    };
    for (tool_index, tool) in tools.iter().enumerate() {
        if let Some(name) = tool.pointer("/function/name").and_then(Value::as_str) {
            push_scope_item(
                items,
                format!("/tools/{tool_index}/function/name"),
                AuditScopeKind::ToolName,
                name,
            );
        }
        if let Some(description) = tool
            .pointer("/function/description")
            .and_then(Value::as_str)
        {
            push_scope_item(
                items,
                format!("/tools/{tool_index}/function/description"),
                AuditScopeKind::ToolDescription,
                description,
            );
        }
        if let Some(parameters) = tool.pointer("/function/parameters") {
            collect_schema_strings_and_keys(
                parameters,
                &format!("/tools/{tool_index}/function/parameters"),
                items,
            );
        }
    }
}

fn collect_schema_strings_and_keys(value: &Value, path: &str, items: &mut Vec<AuditScopeItem>) {
    match value {
        Value::String(text) => push_scope_item(
            items,
            path.to_string(),
            AuditScopeKind::ToolSchemaString,
            text,
        ),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_schema_strings_and_keys(value, &format!("{path}/{index}"), items);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                let child_path = format!("{path}/{}", escape_json_pointer_segment(key));
                push_scope_item(
                    items,
                    child_path.clone(),
                    AuditScopeKind::ToolSchemaKey,
                    key,
                );
                collect_schema_strings_and_keys(value, &child_path, items);
            }
        }
        _ => {}
    }
}

fn collect_top_level_params(body: &Value, items: &mut Vec<AuditScopeItem>) {
    let Some(map) = body.as_object() else {
        return;
    };
    for (key, value) in map {
        if is_skipped_top_level_param(key) {
            continue;
        }
        collect_string_leaves(
            value,
            &format!("/{}", escape_json_pointer_segment(key)),
            AuditScopeKind::TopLevelParam,
            items,
        );
    }
}

fn collect_string_leaves(
    value: &Value,
    path: &str,
    kind: AuditScopeKind,
    items: &mut Vec<AuditScopeItem>,
) {
    match value {
        Value::String(text) => push_scope_item(items, path.to_string(), kind, text),
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                collect_string_leaves(value, &format!("{path}/{index}"), kind, items);
            }
        }
        Value::Object(map) => {
            for (key, value) in map {
                collect_string_leaves(
                    value,
                    &format!("{path}/{}", escape_json_pointer_segment(key)),
                    kind,
                    items,
                );
            }
        }
        _ => {}
    }
}

fn push_scope_item(
    items: &mut Vec<AuditScopeItem>,
    path: String,
    kind: AuditScopeKind,
    text: &str,
) {
    items.push(AuditScopeItem {
        path,
        kind,
        text: text.to_string(),
    });
}

fn is_skipped_top_level_param(key: &str) -> bool {
    matches!(
        key,
        "messages"
            | "tools"
            | "model"
            | "stream"
            | "temperature"
            | "top_p"
            | "max_tokens"
            | "max_completion_tokens"
            | "presence_penalty"
            | "frequency_penalty"
            | "stop"
            | "seed"
            | "n"
            | "logit_bias"
            | "logprobs"
            | "top_logprobs"
            | "response_format"
            | "tool_choice"
            | "parallel_tool_calls"
            | "modalities"
            | "audio"
    )
}

fn escape_json_pointer_segment(segment: &str) -> String {
    segment.replace('~', "~0").replace('/', "~1")
}

fn sum_text_bytes(items: &[AuditScopeItem]) -> u32 {
    items.iter().fold(0u32, |sum, item| {
        sum.saturating_add(saturating_u32(item.text.len()))
    })
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn policy(scan_system_messages: bool, scan_byte_limit: u32) -> AuditPolicy {
        AuditPolicy {
            enabled: true,
            mode: AuditMode::Observe,
            block_critical: true,
            scan_system_messages,
            scan_byte_limit,
            store_payload: true,
            evidence_level: AuditEvidenceLevel::Summary,
        }
    }

    #[test]
    fn scope_builder_flattens_request_in_default_scan_order() {
        let body = json!({
            "model": "gpt-4o",
            "stream": false,
            "temperature": 0.7,
            "messages": [
                {"role": "system", "content": "system secret"},
                {"role": "user", "content": [{"type": "text", "text": "hello"}]},
                {
                    "role": "assistant",
                    "tool_calls": [{
                        "type": "function",
                        "function": {"name": "lookup", "arguments": "{\"city\":\"Paris\"}"}
                    }]
                },
                {"role": "tool", "content": "tool says ok"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "lookup",
                    "description": "Lookup weather",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "city/name": {
                                "type": "string",
                                "description": "City name"
                            }
                        },
                        "required": ["city/name"]
                    }
                }
            }],
            "metadata": {"ticket": "SEC-2"},
            "user": "user-123",
            "reasoning": {"effort": "low"},
            "web_search_options": {"search_context_size": "medium"},
            "extra_body": {"vendor": "value"},
            "provider_options": {"route": "west"},
            "unknownText": "scan me"
        });

        let scope = build_audit_scope(&body, &policy(false, 10_000));
        let actual: Vec<_> = scope
            .items
            .iter()
            .map(|item| (item.path.as_str(), item.kind, item.text.as_str()))
            .collect();

        assert_eq!(
            actual,
            vec![
                (
                    "/messages/1/content/0/text",
                    AuditScopeKind::MessageContent,
                    "hello"
                ),
                (
                    "/messages/1/content/0/type",
                    AuditScopeKind::MessageContent,
                    "text"
                ),
                (
                    "/messages/3/content",
                    AuditScopeKind::MessageContent,
                    "tool says ok"
                ),
                (
                    "/messages/2/tool_calls/0/function/arguments",
                    AuditScopeKind::ToolCallArguments,
                    "{\"city\":\"Paris\"}"
                ),
                ("/tools/0/function/name", AuditScopeKind::ToolName, "lookup"),
                (
                    "/tools/0/function/description",
                    AuditScopeKind::ToolDescription,
                    "Lookup weather"
                ),
                (
                    "/tools/0/function/parameters/properties",
                    AuditScopeKind::ToolSchemaKey,
                    "properties"
                ),
                (
                    "/tools/0/function/parameters/properties/city~1name",
                    AuditScopeKind::ToolSchemaKey,
                    "city/name"
                ),
                (
                    "/tools/0/function/parameters/properties/city~1name/description",
                    AuditScopeKind::ToolSchemaKey,
                    "description"
                ),
                (
                    "/tools/0/function/parameters/properties/city~1name/description",
                    AuditScopeKind::ToolSchemaString,
                    "City name"
                ),
                (
                    "/tools/0/function/parameters/properties/city~1name/type",
                    AuditScopeKind::ToolSchemaKey,
                    "type"
                ),
                (
                    "/tools/0/function/parameters/properties/city~1name/type",
                    AuditScopeKind::ToolSchemaString,
                    "string"
                ),
                (
                    "/tools/0/function/parameters/required",
                    AuditScopeKind::ToolSchemaKey,
                    "required"
                ),
                (
                    "/tools/0/function/parameters/required/0",
                    AuditScopeKind::ToolSchemaString,
                    "city/name"
                ),
                (
                    "/tools/0/function/parameters/type",
                    AuditScopeKind::ToolSchemaKey,
                    "type"
                ),
                (
                    "/tools/0/function/parameters/type",
                    AuditScopeKind::ToolSchemaString,
                    "object"
                ),
                ("/extra_body/vendor", AuditScopeKind::TopLevelParam, "value"),
                ("/metadata/ticket", AuditScopeKind::TopLevelParam, "SEC-2"),
                (
                    "/provider_options/route",
                    AuditScopeKind::TopLevelParam,
                    "west"
                ),
                ("/reasoning/effort", AuditScopeKind::TopLevelParam, "low"),
                ("/unknownText", AuditScopeKind::TopLevelParam, "scan me"),
                ("/user", AuditScopeKind::TopLevelParam, "user-123"),
                (
                    "/web_search_options/search_context_size",
                    AuditScopeKind::TopLevelParam,
                    "medium"
                ),
            ]
        );
        assert!(!actual.iter().any(|(_, _, text)| *text == "system secret"));
        assert_eq!(scope.candidate_bytes, 171);
        assert_eq!(scope.scanned_bytes, 171);
        assert!(!scope.truncated);
    }

    #[test]
    fn scope_builder_includes_system_messages_when_enabled() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "system", "content": "system policy"}
            ]
        });

        let scope = build_audit_scope(&body, &policy(true, 10_000));

        assert_eq!(
            scope.items.last(),
            Some(&AuditScopeItem {
                path: "/messages/1/content".to_string(),
                kind: AuditScopeKind::SystemMessageContent,
                text: "system policy".to_string(),
            })
        );
    }

    #[test]
    fn scope_builder_records_truncation_without_partial_items() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "abcd"},
                {"role": "tool", "content": "efgh"},
                {"role": "user", "content": "ij"}
            ]
        });

        let scope = build_audit_scope(&body, &policy(false, 8));

        assert_eq!(scope.items.len(), 2);
        assert_eq!(scope.scanned_bytes, 8);
        assert_eq!(scope.candidate_bytes, 10);
        assert!(scope.truncated);
    }

    #[test]
    fn scope_builder_stops_at_first_item_that_exceeds_limit() {
        let body = json!({
            "messages": [
                {"role": "user", "content": "abcd"},
                {"role": "tool", "content": "efghi"},
                {"role": "user", "content": "j"}
            ]
        });

        let scope = build_audit_scope(&body, &policy(false, 8));

        assert_eq!(
            scope
                .items
                .iter()
                .map(|item| item.text.as_str())
                .collect::<Vec<_>>(),
            vec!["abcd"]
        );
        assert_eq!(scope.scanned_bytes, 4);
        assert_eq!(scope.candidate_bytes, 10);
        assert!(scope.truncated);
    }
}
