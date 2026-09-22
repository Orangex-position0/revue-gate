use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::State,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    infrastructure::mcp::knowledge::{
        McpError, McpReadKnowledgeChunkInput, list_knowledge_bases_tool, read_knowledge_chunk_tool,
    },
    interface::http::{
        handlers::AppState,
        service_modules::{ServiceModule, ServiceModuleStatus},
    },
};

mod handlers;
mod model;

pub struct McpService;

#[async_trait]
impl ServiceModule for McpService {
    fn id(&self) -> &'static str {
        "mcp"
    }

    fn name(&self) -> &'static str {
        "MCP Server"
    }

    fn description(&self) -> &'static str {
        "Model Context Protocol server exposing knowledge tools."
    }

    fn path_prefixes(&self) -> &'static [&'static str] {
        &["/mcp"]
    }

    async fn get_status(&self, state: &AppState, enabled: bool) -> ServiceModuleStatus {
        let configured = state.knowledge_repo.is_some();
        ServiceModuleStatus {
            id: self.id().into(),
            name: self.name().into(),
            description: self.description().into(),
            path_prefixes: self
                .path_prefixes()
                .iter()
                .map(|path| (*path).into())
                .collect(),
            enabled,
            running: enabled && configured,
            stats: json!({
                "transport": "streamable_http",
                "tools": ["list_knowledge_bases", "read_knowledge_chunk"],
                "configured": configured,
            }),
        }
    }

    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/mcp", get(mcp_info).post(handle_mcp))
            .route("/mcp/", post(handle_mcp))
            .route("/mcp/sse", method_router)
    }
}

async fn mcp_info() -> Json<Value> {
    Json(json!({
        "name": "revue-gate MCP Server",
        "transport": "streamable_http",
        "endpoint": "/mcp",
    }))
}

async fn handle_mcp(State(state): State<AppState>, Json(request): Json<McpRequest>) -> Response {
    Json(dispatch(&state, request).await).into_response()
}

async fn dispatch(state: &AppState, request: McpRequest) -> McpResponse {
    match request.method.as_str() {
        "initialize" => McpResponse::success(
            request.id,
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "revue-gate",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "instructions": "Use the knowledge tools to inspect MCP-enabled local knowledge bases.",
            }),
        ),
        "notifications/initialized" | "ping" => McpResponse::success(request.id, json!({})),
        "tools/list" => McpResponse::success(request.id, json!({ "tools": tools() })),
        "tools/call" => match handle_tool_call(state, request.params).await {
            Ok(value) => McpResponse::success(request.id, value),
            Err(error) => McpResponse::from_error(request.id, error),
        },
        _ => McpResponse::error(request.id, -32601, "method not found"),
    }
}

async fn handle_tool_call(state: &AppState, params: Value) -> Result<Value, McpError> {
    let repo = state
        .knowledge_repo
        .as_deref()
        .ok_or_else(|| McpError::Unavailable("knowledge service is unavailable".into()))?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::InvalidParams("tool name is required".into()))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    match name {
        "list_knowledge_bases" => Ok(tool_result(list_knowledge_bases_tool(repo).await?)),
        "read_knowledge_chunk" => {
            let input: McpReadKnowledgeChunkInput = serde_json::from_value(arguments)
                .map_err(|error| McpError::InvalidParams(error.to_string()))?;
            Ok(tool_result(read_knowledge_chunk_tool(repo, input).await?))
        }
        _ => Err(McpError::InvalidParams(format!("unknown tool: {name}"))),
    }
}

fn tool_result(value: impl Serialize) -> Value {
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "null".to_string());
    json!({ "content": [{ "type": "text", "text": text }] })
}
