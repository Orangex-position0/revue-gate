use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Query, State},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::stream;
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Arc, OnceLock},
};
use tokio::sync::{Mutex, mpsc};

use self::{
    handlers::tools,
    model::{McpRequest, McpResponse},
};
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
            .route("/mcp", get(handle_sse).post(handle_mcp))
            .route("/mcp/", post(handle_mcp))
            .route("/mcp/sse", get(handle_sse).post(handle_mcp))
    }
}

type SessionMap = Arc<Mutex<HashMap<String, mpsc::Sender<String>>>>;
static SSE_SESSIONS: OnceLock<SessionMap> = OnceLock::new();

fn sessions() -> &'static SessionMap {
    SSE_SESSIONS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

#[derive(Debug, Deserialize)]
struct SessionQuery {
    session_id: Option<String>,
}

async fn handle_mcp(
    State(state): State<AppState>,
    Query(query): Query<SessionQuery>,
    Json(request): Json<McpRequest>,
) -> Response {
    if let Some(session_id) = query.session_id {
        let response = dispatch(&state, request).await;
        let payload = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
        let sender = sessions().lock().await.get(&session_id).cloned();
        if let Some(sender) = sender {
            let _ = sender.send(payload).await;
            return axum::http::StatusCode::ACCEPTED.into_response();
        }
        return (axum::http::StatusCode::NOT_FOUND, "unknown MCP SSE session").into_response();
    }
    Json(dispatch(&state, request).await).into_response()
}

async fn handle_sse() -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let session_id = uuid::Uuid::now_v7().to_string();
    let (sender, receiver) = mpsc::channel::<String>(16);
    sessions().lock().await.insert(session_id.clone(), sender);
    let endpoint = Event::default()
        .event("endpoint")
        .data(format!("/mcp?session_id={session_id}"));
    let events = stream::unfold(
        (Some(endpoint), receiver),
        |(first, mut receiver)| async move {
            if let Some(event) = first {
                return Some((Ok(event), (None, receiver)));
            }
            receiver.recv().await.map(|payload| {
                (
                    Ok(Event::default().event("message").data(payload)),
                    (None, receiver),
                )
            })
        },
    );
    Sse::new(events)
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
                "instructions": "Use list_knowledge_bases once, then prefer ask_knowledge_base for grounded answers and search_knowledge_base when raw chunks are required. MCP access is restricted to MCP-enabled local knowledge bases.",
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
