use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::infrastructure::mcp::knowledge::McpError;

#[derive(Debug, Deserialize)]
pub(crate) struct McpRequest {
    pub(crate) id: Option<Value>,
    pub(crate) method: String,
    #[serde(default)]
    pub(crate) params: Value,
}

#[derive(Debug, Serialize)]
pub(crate) struct McpResponse {
    jsonrpc: &'static str,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<McpJsonError>,
}

impl McpResponse {
    pub(crate) fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    pub(crate) fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(McpJsonError {
                code,
                message: message.into(),
            }),
        }
    }

    pub(crate) fn from_error(id: Option<Value>, error: McpError) -> Self {
        let code = match error {
            McpError::InvalidParams(_) => -32602,
            McpError::NotFound(_) => -32004,
            McpError::Forbidden(_) => -32003,
            McpError::Unavailable(_) => -32001,
            McpError::Internal(_) => -32603,
        };
        Self::error(id, code, error.to_string())
    }
}

#[derive(Debug, Serialize)]
struct McpJsonError {
    code: i32,
    message: String,
}
