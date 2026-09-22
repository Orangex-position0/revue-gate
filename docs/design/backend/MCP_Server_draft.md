# MCP Server Spec

本文章为 MCP_Server 模块的 Spec 文档，要包括：

- **可测试的成功标准**：比如具体的测试数值，禁止模糊的结果描述
- **明确的目标边界**：Goals 和 Non-Goals 都要有，显式告诉 AI “什么不该做”
- **技术选型约束**：防止 AI 自行引入新组件

模块作用：通过 MCP 对外暴露各种扩展服务（向量化索引、混合检索、RAG 问答等），让任意 AI Agent 能直接调用服务相关工具。

# 设计思路

按以下流程来，一个步骤为一组讨论，讨论结束后就提醒用户可以补充到文档中：

1. 技术栈

## MCP 传输方式

项目将支持两种传输：

- Streamable HTTP
- SSE

通过一组路由暴露：

- post `/mcp`
- get `/mcp/sse`
- post `/mcp?session_id=xxx`

## model

mcpRequest + mcpResponse + mcpError:

```rust
#[derive(Debug, Deserialize)]
pub struct McpRequest {
    pub jsonrpc: String,          // "2.0"
    pub id: Option<serde_json::Value>,
    pub method: String,           // "initialize"/"tools/list"/"tools/call"
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct McpResponse {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    result: Option<serde_json::Value>,
    error: Option<McpError>,
}

#[derive(Debug, Serialize)]
pub struct McpError {
    code: i32,
    message: String,
}
```

## instructions 注入

MCP 规范允许 Server 在 initialize 响应中注入 instructions，作为 Agent 的系统提示：

```ts
const MCP_INSTRUCTIONS: &str = r#"# WaLiAPI 知识库 — 本地 RAG + 向量检索

知识库已预建索引：文档已解析、分块、向量化并存入本地 SQLite + HNSW 索引。
所有检索都是本地操作，亚秒级响应。

## 工具使用优先级

1. **ask_knowledge_base** — 首选。直接提问，返回 AI 生成的回答 + 来源引用。
   适合：任何问题、概念理解、代码含义、流程梳理。

2. **search_knowledge_base** — 当需要看原始文本片段，或 ask 回答不够时使用。

3. **list_knowledge_bases** — 首次使用时调用一次，获取可用知识库 ID。

4. **其他工具** — 按需使用（上传文档、管理索引等）。

## 反模式

- ❌ 不要先 search 再自己总结 — 直接用 ask_knowledge_base
- ❌ 不要每次都调 list_knowledge_bases — 缓存第一次的结果
- ❌ 不要对同一问题反复 search 不同关键词

## 代码文件

知识库中的代码文件按符号边界分块（函数/类/方法），每个 chunk 是完整符号。
chunk metadata 包含 symbol_name、symbol_kind、signature，可用于精确过滤。"#;
```

## dispatch

作用：MCP 分发器，根据 MCP 请求的 method 分发到对应的处理函数：

```rust
async fn dispatch_jsonrpc_async(shared: &SharedState, req: &McpRequest) -> McpResponse {
    match req.method.as_str() {
        "initialize" => {
            McpResponse::success(req.id.clone(), serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
                "serverInfo": {
                    "name": "WaLiAPI Knowledge Base",
                    "version": "0.1.0"
                },
                "instructions": MCP_INSTRUCTIONS  // ← 注入系统提示
            }))
        }
        "notifications/initialized" => {
            McpResponse::success(req.id.clone(), serde_json::json!({}))
        }
        "tools/list" => {
            McpResponse::success(req.id.clone(), serde_json::json!({
                "tools": get_tools()
            }))
        }
        "tools/call" => {
            let tool_name = req.params.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or_default();
            match handle_tool_call(shared, tool_name, &args).await {
                Ok(result) => McpResponse::success(req.id.clone(), result),
                Err(e) => McpResponse::error(req.id.clone(), -32603, e),
            }
        }
        "ping" => {
            McpResponse::success(req.id.clone(), serde_json::json!({}))
        }
        _ => McpResponse::error(req.id.clone(), -32601, format!("Unknown method: {}", req.method))
    }
}
```

## mcp tool

### 工具分类

类别 工具 权限
检索 search_knowledge_base, ask_knowledge_base 读取
信息 list_knowledge_bases, read_document, get_knowledge_base_stats 读取
管理 create_knowledge_base, update_knowledge_base, delete_knowledge_base 写入
文档 upload_document, delete_document, list_documents 写入
索引 build_index 写入
导入 import_source 写入

工具定义参考：

```json
{
    "name": "search_knowledge_base",
    "description": "Semantic search across a local knowledge base. Uses HNSW vector index for O(log n) retrieval. Returns matching text chunks with cosine similarity scores (0-1).",
    "inputSchema": {
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Natural language search query"
            },
            "kb_id": {
                "type": "string",
                "description": "Specific KB ID. If omitted, searches all MCP-enabled KBs."
            },
            "top_k": {
                "type": "integer",
                "description": "Max results (default: 5)",
                "default": 5
            }
        },
        "required": ["query"]
    }
}
```

### handle_tool_call

作用：处理工具调用请求，根据工具名称和参数调用对应的处理函数。

```rust
async fn handle_tool_call(shared: &SharedState, tool_name: &str, args: &Value) -> Result<Value, String> {
    let pool = &shared.state.inner().db.pool;
    match tool_name {
        "search_knowledge_base" => {
            let query = args.get("query").and_then(|q| q.as_str()).unwrap_or("");
            let kb_id = args.get("kb_id").and_then(|k| k.as_str()).unwrap_or("");
            let top_k = args.get("top_k").and_then(|k| k.as_u64()).unwrap_or(5) as usize;
            // ... embed query, hybrid_search, format results
        }
        "ask_knowledge_base" => {
            let question = args.get("question").and_then(|q| q.as_str()).unwrap_or("");
            // ... RAG ask, format answer
        }
        "list_knowledge_bases" => {
            let repo = KbRepository::new(pool.clone());
            let kbs = repo.get_all_mcp_enabled_kbs().await?;
            // Format as MCP tool result
        }
        "create_knowledge_base" => {
            // ... create KB
        }
        // ... 其他 9 个工具
        _ => Err(format!("Unknown tool: {}", tool_name)),
    }
}
```

## SSE 传输

### SSE Session 管理

每个 SSE 客户端会获得唯一的 `session_id`，POST 请求将通过 `session_id` 关联到对应的 SSE 会话。

id 生成策略：UUID v7

### SSE 端点

### POST 端点

## McpService

由于 MCP 服务是扩展模块，不属于主应用的核心功能，因此将其独立为一个服务。所以使用时需要注册到服务中心。

```rust
pub struct McpService;

#[async_trait]
impl Service for McpService {
    fn id(&self) -> &'static str { "mcp" }
    fn name(&self) -> &'static str { "MCP Server" }

    fn routes(&self, _state: Arc<AppState>) -> Router<SharedState> {
        Router::new()
            .route("/mcp", post(handle_mcp).get(handle_mcp_sse))
            .route("/mcp/", post(handle_mcp).get(handle_mcp_sse))
            .route("/mcp/sse", get(handle_mcp_sse).post(handle_mcp))
    }
}
```

路由设计：

- /mcp 和 /mcp/ 两种路径：有些 MCP Client 发送带尾部斜杠的请求
- /mcp/sse：旧版 SSE 端点，保持向后兼容
- GET /mcp 和 GET /mcp/sse：建立 SSE 连接
- POST /mcp 和 POST /mcp/sse：发送 JSON-RPC 请求

## 前端

项目中，后端实现的独立服务 KnowledgeBase 还没有对应的前端页面，现在来实现。

### page 分类

功能 说明
知识库 CRUD 创建 / 编辑 / 删除知识库
文档管理 上传文件、查看文档状态、删除文档
多源导入 Git/URL/ 本地目录导入
索引管理 构建 / 重建 / 删除 HNSW 索引
搜索 向量 + FTS5 混合搜索
RAG 问答 多轮对话式问答
MCP 配置 启用 / 禁用 MCP 暴露
对话历史 查看 / 清除对话记录

### 具体代码

可直接参考 E:\test\References\waliapi 对于 KnowledgeBase 页面的实现

## Tauri Command

```text
KnowledgeBasePage (React)
     │
     │  invoke("get_knowledge_bases")        ← Tauri 命令
     │  invoke("create_knowledge_base", ...)  ← Tauri 命令
     │  invoke("ask_knowledge_base", ...)     ← Tauri 命令
     │
     │  fetch("/api/kb/search?query=...")     ← HTTP API
     │  fetch("/api/kb/ask", POST)             ← HTTP API
     │
     ▼
WaLiAPI 后端 (Rust)
```

## 最后验收

```sh
npm run tauri dev

# 1. MCP initialize（Streamable HTTP）
curl -X POST http://localhost:3456/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}'
# → protocolVersion + capabilities + instructions

# 2. MCP tools/list
curl -X POST http://localhost:3456/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
# → 13 个工具定义

# 3. MCP search_knowledge_base
curl -X POST http://localhost:3456/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search_knowledge_base","arguments":{"query":"handle stream","top_k":3}}}'
# → 搜索结果

# 4. MCP ask_knowledge_base
curl -X POST http://localhost:3456/mcp \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"ask_knowledge_base","arguments":{"question":"如何处理流式请求？"}}}'
# → RAG 回答 + 来源引用

# 5. SSE 连接
curl -N http://localhost:3456/mcp/sse
# → event: endpoint
# → data: /mcp?session_id=xxx

# 6. SSE 模式下的 JSON-RPC
curl -X POST "http://localhost:3456/mcp?session_id=xxx" \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":5,"method":"ping","params":{}}'
# → SSE 流中推送响应

# 7. Claude Desktop 配置
# 在 claude_desktop_config.json 中添加：
{
  "mcpServers": {
    "waliapi": {
      "url": "http://localhost:3456/mcp"
    }
  }
}
```
