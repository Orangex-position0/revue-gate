//! Security audit domain types: settings, resolved policy, request scope, and request-level risk report.

use serde::{Deserialize, Serialize};
use serde_json::Value;

const DETECTOR_FINDING_LIMIT: usize = 5;

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
#[serde(rename_all = "lowercase")]
pub enum AuditConfidence {
    Low,
    Medium,
    High,
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

/// One deterministic detector finding emitted from a scanned request scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFinding {
    pub rule_id: String,
    pub category: String,
    pub risk_level: RiskLevel,
    pub action: AuditAction,
    pub confidence: AuditConfidence,
    pub scope_kind: AuditScopeKind,
    pub path: String,
    pub redacted_excerpt: String,
    pub match_hash: String,
    pub suggested_action: String,
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

    pub fn for_scope(policy: &AuditPolicy, scope: &AuditScope) -> Self {
        let findings = detect_findings(scope, policy);
        if findings.is_empty() {
            return Self::clean_for_scope(policy, scope);
        }

        let risk_level = findings
            .iter()
            .map(|finding| finding.risk_level)
            .max_by_key(|risk| risk_rank(*risk))
            .unwrap_or(RiskLevel::Clean);
        let action = report_action(risk_level, policy);

        Self {
            mode: policy.mode,
            risk_level,
            action,
            findings,
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

#[derive(Debug, Clone, Copy)]
struct DetectorRule {
    id: &'static str,
    category: &'static str,
    risk_level: RiskLevel,
    action: AuditAction,
    confidence: AuditConfidence,
    suggested_action: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct MatchSpan {
    start: usize,
    end: usize,
}

fn detect_findings(scope: &AuditScope, policy: &AuditPolicy) -> Vec<AuditFinding> {
    let rules = detector_rules(policy);
    let mut counts = vec![0usize; rules.len()];
    let mut findings = Vec::new();

    for item in &scope.items {
        for (rule_index, rule) in rules.iter().enumerate() {
            if counts[rule_index] >= DETECTOR_FINDING_LIMIT {
                continue;
            }

            for span in find_rule_matches(rule.id, &item.text) {
                if counts[rule_index] >= DETECTOR_FINDING_LIMIT {
                    break;
                }

                let matched = &item.text[span.start..span.end];
                findings.push(AuditFinding {
                    rule_id: rule.id.to_string(),
                    category: rule.category.to_string(),
                    risk_level: rule.risk_level,
                    action: rule.action,
                    confidence: rule.confidence,
                    scope_kind: item.kind,
                    path: item.path.clone(),
                    redacted_excerpt: redacted_excerpt(&item.text, span, rule.id),
                    match_hash: match_hash(rule.id, matched),
                    suggested_action: rule.suggested_action.to_string(),
                });
                counts[rule_index] += 1;
            }
        }
    }

    findings
}

fn detector_rules(policy: &AuditPolicy) -> Vec<DetectorRule> {
    let critical_action = if policy.block_critical {
        AuditAction::Block
    } else {
        AuditAction::Warn
    };
    vec![
        DetectorRule {
            id: "credential.private_key",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.provider_api_key",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.aws_access_key_id",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.aws_secret_access_key",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.gcp_oauth_token",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.azure_storage_connection_string",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.database_url",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.bearer_token",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.jwt",
            category: "credential",
            risk_level: RiskLevel::Critical,
            action: critical_action,
            confidence: AuditConfidence::High,
            suggested_action: "Rotate the credential and remove it from the request payload.",
        },
        DetectorRule {
            id: "credential.local_revue_key",
            category: "credential",
            risk_level: RiskLevel::High,
            action: AuditAction::Warn,
            confidence: AuditConfidence::High,
            suggested_action: "Replace the local gateway key if it was shared unintentionally.",
        },
        DetectorRule {
            id: "sensitive_path.local_secret",
            category: "sensitivePath",
            risk_level: RiskLevel::High,
            action: AuditAction::Warn,
            confidence: AuditConfidence::High,
            suggested_action: "Remove local secret paths from the request payload.",
        },
    ]
}

fn find_rule_matches(rule_id: &str, text: &str) -> Vec<MatchSpan> {
    match rule_id {
        "credential.private_key" => find_private_key_blocks(text),
        "credential.provider_api_key" => find_provider_api_keys(text),
        "credential.aws_access_key_id" => find_aws_access_key_ids(text),
        "credential.aws_secret_access_key" => find_assignment_secret(
            text,
            &["aws_secret_access_key", "AWS_SECRET_ACCESS_KEY"],
            32,
        ),
        "credential.gcp_oauth_token" => find_prefixed_secrets(text, &["ya29."], 32),
        "credential.azure_storage_connection_string" => find_azure_storage_connection_strings(text),
        "credential.database_url" => find_prefixed_tokens(
            text,
            &[
                "postgres://",
                "postgresql://",
                "mysql://",
                "mongodb://",
                "redis://",
            ],
            16,
        ),
        "credential.bearer_token" => find_bearer_tokens(text),
        "credential.jwt" => find_jwt_tokens(text),
        "credential.local_revue_key" => find_prefixed_secrets(text, &["sk-revue-"], 25),
        "sensitive_path.local_secret" => find_sensitive_paths(text),
        _ => Vec::new(),
    }
}

fn find_private_key_blocks(text: &str) -> Vec<MatchSpan> {
    let mut spans = Vec::new();
    let mut offset = 0;
    while let Some(relative_start) = text[offset..].find("-----BEGIN ") {
        let start = offset + relative_start;
        let after_begin = start + "-----BEGIN ".len();
        let Some(relative_end_label) = text[after_begin..].find(" PRIVATE KEY-----") else {
            offset = after_begin;
            continue;
        };
        let label = &text[after_begin..after_begin + relative_end_label];
        let end_marker = format!("-----END {label} PRIVATE KEY-----");
        let Some(relative_end) = text[after_begin..].find(&end_marker) else {
            offset = after_begin;
            continue;
        };
        let end = after_begin + relative_end + end_marker.len();
        spans.push(MatchSpan { start, end });
        offset = end;
    }
    spans
}

fn find_provider_api_keys(text: &str) -> Vec<MatchSpan> {
    find_prefixed_secrets(text, &["sk-", "xai-", "anthropic-", "AIza"], 32)
        .into_iter()
        .filter(|span| {
            let token = &text[span.start..span.end];
            let lower = token.to_ascii_lowercase();
            !lower.starts_with("sk-revue-")
        })
        .collect()
}

fn find_aws_access_key_ids(text: &str) -> Vec<MatchSpan> {
    find_tokens(text)
        .into_iter()
        .filter(|span| {
            let token = &text[span.start..span.end];
            token.len() == 20
                && token.get(..4).is_some_and(|prefix| {
                    prefix.eq_ignore_ascii_case("AKIA") || prefix.eq_ignore_ascii_case("ASIA")
                })
                && token.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .collect()
}

fn find_assignment_secret(text: &str, names: &[&str], min_len: usize) -> Vec<MatchSpan> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    for name in names {
        let needle = name.to_ascii_lowercase();
        let mut offset = 0;
        while let Some(relative_start) = lower[offset..].find(&needle) {
            let after_name = offset + relative_start + needle.len();
            let Some((value_start, value_end)) = assignment_value_span(text, after_name) else {
                offset = after_name;
                continue;
            };
            if value_end.saturating_sub(value_start) >= min_len {
                spans.push(MatchSpan {
                    start: value_start,
                    end: value_end,
                });
            }
            offset = value_end;
        }
    }
    spans
}

fn assignment_value_span(text: &str, after_name: usize) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = after_name;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() || !matches!(bytes[index], b':' | b'=') {
        return None;
    }
    index += 1;
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }
    if index >= bytes.len() {
        return None;
    }
    let quote = matches!(bytes[index], b'"' | b'\'').then_some(bytes[index]);
    if quote.is_some() {
        index += 1;
    }
    let start = index;
    while index < bytes.len() {
        if quote.is_some_and(|q| bytes[index] == q)
            || (quote.is_none() && is_token_boundary(bytes[index]))
        {
            break;
        }
        index += 1;
    }
    (index > start).then_some((start, index))
}

fn find_prefixed_tokens(text: &str, prefixes: &[&str], min_len: usize) -> Vec<MatchSpan> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    for prefix in prefixes {
        let needle = prefix.to_ascii_lowercase();
        let mut offset = 0;
        while let Some(relative_start) = lower[offset..].find(&needle) {
            let start = offset + relative_start;
            if start > 0 && !is_secret_start_boundary(text.as_bytes()[start - 1]) {
                offset = start + needle.len();
                continue;
            }
            let end = consume_token(text, start);
            if end.saturating_sub(start) >= min_len {
                spans.push(MatchSpan { start, end });
            }
            offset = end.max(start + needle.len());
        }
    }
    spans.sort_by_key(|span| span.start);
    spans
}

fn find_prefixed_secrets(text: &str, prefixes: &[&str], min_len: usize) -> Vec<MatchSpan> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    for prefix in prefixes {
        let needle = prefix.to_ascii_lowercase();
        let mut offset = 0;
        while let Some(relative_start) = lower[offset..].find(&needle) {
            let start = offset + relative_start;
            if start > 0 && !is_secret_start_boundary(text.as_bytes()[start - 1]) {
                offset = start + needle.len();
                continue;
            }
            let end = consume_token(text, start);
            if end.saturating_sub(start) >= min_len {
                spans.push(MatchSpan { start, end });
            }
            offset = end.max(start + needle.len());
        }
    }
    spans.sort_by_key(|span| span.start);
    spans
}

fn find_azure_storage_connection_strings(text: &str) -> Vec<MatchSpan> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut offset = 0;
    while let Some(relative_start) = lower[offset..].find("defaultendpointsprotocol=") {
        let start = offset + relative_start;
        let end = text[start..]
            .find(char::is_whitespace)
            .map(|relative_end| start + relative_end)
            .unwrap_or(text.len());
        let candidate = &lower[start..end];
        if candidate.contains("accountkey=") {
            spans.push(MatchSpan { start, end });
        }
        offset = end.max(start + "defaultendpointsprotocol=".len());
    }
    spans
}

fn find_bearer_tokens(text: &str) -> Vec<MatchSpan> {
    let lower = text.to_ascii_lowercase();
    let mut spans = Vec::new();
    let mut offset = 0;
    while let Some(relative_start) = lower[offset..].find("bearer ") {
        let token_start = offset + relative_start + "bearer ".len();
        let token_end = consume_token(text, token_start);
        if token_end.saturating_sub(token_start) >= 20 {
            spans.push(MatchSpan {
                start: token_start,
                end: token_end,
            });
        }
        offset = token_end.max(token_start + 1);
    }
    spans
}

fn find_jwt_tokens(text: &str) -> Vec<MatchSpan> {
    find_tokens(text)
        .into_iter()
        .filter(|span| {
            let token = &text[span.start..span.end];
            let mut parts = token.split('.');
            parts.next().is_some_and(is_base64_urlish)
                && parts.next().is_some_and(is_base64_urlish)
                && parts.next().is_some_and(is_base64_urlish)
                && parts.next().is_none()
        })
        .collect()
}

fn find_sensitive_paths(text: &str) -> Vec<MatchSpan> {
    let lower = text.to_ascii_lowercase();
    let needles = [
        ".env",
        "~/.ssh",
        "/.ssh/",
        "\\.ssh\\",
        ".aws/credentials",
        ".aws\\credentials",
        "application_default_credentials.json",
        "gcloud/credentials.db",
    ];
    let mut spans = Vec::new();
    for needle in needles {
        let mut offset = 0;
        while let Some(relative_start) = lower[offset..].find(needle) {
            let start = offset + relative_start;
            spans.push(MatchSpan {
                start,
                end: start + needle.len(),
            });
            offset = start + needle.len();
        }
    }
    spans.sort_by_key(|span| span.start);
    spans
}

fn find_tokens(text: &str) -> Vec<MatchSpan> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && is_token_boundary(bytes[index]) {
            index += 1;
        }
        let start = index;
        while index < bytes.len() && !is_token_boundary(bytes[index]) {
            index += 1;
        }
        if index > start {
            spans.push(MatchSpan { start, end: index });
        }
    }
    spans
}

fn consume_token(text: &str, start: usize) -> usize {
    let bytes = text.as_bytes();
    let mut index = start;
    while index < bytes.len() && !is_token_boundary(bytes[index]) {
        index += 1;
    }
    index
}

fn is_token_boundary(byte: u8) -> bool {
    byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'"' | b'\'' | b',' | b';' | b')' | b'(' | b'[' | b']' | b'{' | b'}' | b'='
        )
}

fn is_secret_start_boundary(byte: u8) -> bool {
    is_token_boundary(byte) || matches!(byte, b':')
}

fn is_base64_urlish(value: &str) -> bool {
    value.len() >= 8
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn redacted_excerpt(text: &str, span: MatchSpan, rule_id: &str) -> String {
    let context_start = text[..span.start]
        .char_indices()
        .rev()
        .nth(24)
        .map(|(index, ch)| index + ch.len_utf8())
        .unwrap_or(0);
    let context_end = text[span.end..]
        .char_indices()
        .nth(24)
        .map(|(index, _)| span.end + index)
        .unwrap_or(text.len());
    let mut excerpt = String::new();
    if context_start > 0 {
        excerpt.push_str("...");
    }
    excerpt.push_str(&text[context_start..span.start]);
    excerpt.push_str(&format!("[redacted:{rule_id}]"));
    excerpt.push_str(&text[span.end..context_end]);
    if context_end < text.len() {
        excerpt.push_str("...");
    }
    excerpt
}

fn match_hash(rule_id: &str, matched: &str) -> String {
    let normalized = matched.trim().to_ascii_lowercase();
    let mut hash = 0xcbf29ce484222325u64;
    for byte in rule_id.bytes().chain([0]).chain(normalized.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn report_action(risk_level: RiskLevel, policy: &AuditPolicy) -> AuditAction {
    match risk_level {
        RiskLevel::Critical if policy.block_critical => AuditAction::Block,
        RiskLevel::Critical | RiskLevel::High => AuditAction::Warn,
        RiskLevel::Medium => AuditAction::LogOnly,
        RiskLevel::Low | RiskLevel::Clean => AuditAction::Allow,
    }
}

fn risk_rank(risk_level: RiskLevel) -> u8 {
    match risk_level {
        RiskLevel::Clean => 0,
        RiskLevel::Low => 1,
        RiskLevel::Medium => 2,
        RiskLevel::High => 3,
        RiskLevel::Critical => 4,
    }
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

    #[test]
    fn detector_flags_credentials_and_sensitive_paths_with_safe_evidence() {
        let raw_openai_key =
            "sk-proj-abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let raw_private_key =
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc123\n-----END OPENSSH PRIVATE KEY-----";
        let raw_jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.signature-part";
        let raw_database_url = "postgres://app:secret@localhost:5432/prod";
        let scope = AuditScope {
            items: vec![
                AuditScopeItem {
                    path: "/messages/0/content".to_string(),
                    kind: AuditScopeKind::MessageContent,
                    text: format!(
                        "key={raw_openai_key}\n{raw_private_key}\nAuthorization: Bearer {raw_jwt}"
                    ),
                },
                AuditScopeItem {
                    path: "/metadata/databaseUrl".to_string(),
                    kind: AuditScopeKind::TopLevelParam,
                    text: raw_database_url.to_string(),
                },
                AuditScopeItem {
                    path: "/metadata/path".to_string(),
                    kind: AuditScopeKind::TopLevelParam,
                    text: "~/.ssh/id_rsa and .env".to_string(),
                },
            ],
            scanned_bytes: 256,
            candidate_bytes: 256,
            scan_byte_limit: 512,
            truncated: false,
        };

        let report = AuditReport::for_scope(&policy(false, 512), &scope);

        assert_eq!(report.risk_level, RiskLevel::Critical);
        assert_eq!(report.action, AuditAction::Block);

        let provider_key = report
            .findings
            .iter()
            .find(|finding| finding.rule_id == "credential.provider_api_key")
            .expect("provider API key finding");
        assert_eq!(provider_key.category, "credential");
        assert_eq!(provider_key.risk_level, RiskLevel::Critical);
        assert_eq!(provider_key.action, AuditAction::Block);
        assert_eq!(provider_key.confidence, AuditConfidence::High);
        assert_eq!(provider_key.scope_kind, AuditScopeKind::MessageContent);
        assert_eq!(provider_key.path, "/messages/0/content");
        assert_eq!(
            provider_key.suggested_action,
            "Rotate the credential and remove it from the request payload."
        );
        assert!(!provider_key.redacted_excerpt.contains(raw_openai_key));
        assert!(provider_key.redacted_excerpt.contains("[redacted:"));
        assert!(!provider_key.match_hash.is_empty());

        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "credential.private_key"
                && finding.risk_level == RiskLevel::Critical
                && finding.action == AuditAction::Block
                && !finding.redacted_excerpt.contains(raw_private_key)
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "credential.bearer_token"
                && finding.category == "credential"
                && finding.path == "/messages/0/content"
                && !finding.redacted_excerpt.contains(raw_jwt)
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "credential.database_url"
                && finding.path == "/metadata/databaseUrl"
                && !finding.redacted_excerpt.contains(raw_database_url)
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "sensitive_path.local_secret"
                && finding.category == "sensitivePath"
                && finding.path == "/metadata/path"
        }));
    }

    #[test]
    fn detector_distinguishes_local_revue_keys_from_blocking_provider_keys() {
        let raw_local_key = "sk-revue-0123456789abcdef";
        let scope = AuditScope {
            items: vec![AuditScopeItem {
                path: "/messages/0/content".to_string(),
                kind: AuditScopeKind::MessageContent,
                text: format!("local gateway key {raw_local_key}"),
            }],
            scanned_bytes: 42,
            candidate_bytes: 42,
            scan_byte_limit: 512,
            truncated: false,
        };

        let report = AuditReport::for_scope(&policy(false, 512), &scope);

        assert_eq!(report.risk_level, RiskLevel::High);
        assert_eq!(report.action, AuditAction::Warn);
        assert_eq!(report.findings.len(), 1);
        let finding = &report.findings[0];
        assert_eq!(finding.rule_id, "credential.local_revue_key");
        assert_eq!(finding.risk_level, RiskLevel::High);
        assert_eq!(finding.action, AuditAction::Warn);
        assert_eq!(finding.confidence, AuditConfidence::High);
        assert!(!finding.redacted_excerpt.contains(raw_local_key));
    }

    #[test]
    fn detector_hashes_are_stable_for_same_rule_and_normalized_match() {
        let scope = AuditScope {
            items: vec![
                AuditScopeItem {
                    path: "/messages/0/content".to_string(),
                    kind: AuditScopeKind::MessageContent,
                    text: "AKIAIOSFODNN7EXAMPLE".to_string(),
                },
                AuditScopeItem {
                    path: "/messages/1/content".to_string(),
                    kind: AuditScopeKind::MessageContent,
                    text: "akiaiosfodnn7example".to_string(),
                },
            ],
            scanned_bytes: 40,
            candidate_bytes: 40,
            scan_byte_limit: 512,
            truncated: false,
        };

        let report = AuditReport::for_scope(&policy(false, 512), &scope);
        let hashes: Vec<_> = report
            .findings
            .iter()
            .filter(|finding| finding.rule_id == "credential.aws_access_key_id")
            .map(|finding| finding.match_hash.as_str())
            .collect();

        assert_eq!(hashes.len(), 2);
        assert_eq!(hashes[0], hashes[1]);
    }

    #[test]
    fn detector_limits_findings_per_rule() {
        let scope = AuditScope {
            items: (0..6)
                .map(|index| AuditScopeItem {
                    path: format!("/messages/{index}/content"),
                    kind: AuditScopeKind::MessageContent,
                    text: format!("AKIAIOSFODNN7EXAMPL{index}"),
                })
                .collect(),
            scanned_bytes: 120,
            candidate_bytes: 120,
            scan_byte_limit: 512,
            truncated: false,
        };

        let report = AuditReport::for_scope(&policy(false, 512), &scope);
        let aws_findings = report
            .findings
            .iter()
            .filter(|finding| finding.rule_id == "credential.aws_access_key_id")
            .count();

        assert_eq!(aws_findings, 5);
    }

    #[test]
    fn detector_handles_colon_prefixed_database_urls_and_cloud_credentials() {
        let gcp_key = "ya29.a0AfH6SMAabcdefghijklmnopqrstuvwxyz1234567890";
        let azure_key = "DefaultEndpointsProtocol=https;AccountName=prod;AccountKey=abcdefghijklmnopqrstuvwxyz0123456789+/abcdefghijklmnopqrstuvwxyz0123456789+/==;EndpointSuffix=core.windows.net";
        let scope = AuditScope {
            items: vec![AuditScopeItem {
                path: "/metadata/secrets".to_string(),
                kind: AuditScopeKind::TopLevelParam,
                text: format!(
                    "url:postgres://app:secret@localhost/prod gcp={gcp_key} azure={azure_key}"
                ),
            }],
            scanned_bytes: 256,
            candidate_bytes: 256,
            scan_byte_limit: 512,
            truncated: false,
        };

        let report = AuditReport::for_scope(&policy(false, 512), &scope);

        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "credential.database_url"
                && !finding
                    .redacted_excerpt
                    .contains("postgres://app:secret@localhost/prod")
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "credential.gcp_oauth_token"
                && finding.risk_level == RiskLevel::Critical
                && finding.action == AuditAction::Block
                && !finding.redacted_excerpt.contains(gcp_key)
        }));
        assert!(report.findings.iter().any(|finding| {
            finding.rule_id == "credential.azure_storage_connection_string"
                && finding.risk_level == RiskLevel::Critical
                && finding.action == AuditAction::Block
                && !finding.redacted_excerpt.contains(azure_key)
        }));
    }
}
