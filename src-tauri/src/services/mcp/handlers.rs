use std::{collections::HashMap, sync::Arc};

use tokio::sync::{mpsc, RwLock};

/// MCP server instructions — agent 首次连接时注入 system prompt
const MCP_INSTRUCTIONS: &str = r#"# WaLiAPI 知识库 — 本地 RAG + 向量检索

知识库已预建索引：文档已解析、分块、向量化并存入本地 SQLite + HNSW 索引。
所有检索都是本地操作，亚秒级响应。

## 工具使用优先级

1. **ask_knowledge_base** — 首选。直接提问，返回 AI 生成的回答 + 来源引用。
   适合：任何问题、概念理解、代码含义、流程梳理。

2. **search_knowledge_base** — 当需要看原始文本片段，或 ask_knowledge_base 回答不够时使用。
   返回匹配的 chunk 原文 + 相似度分数。

3. **list_knowledge_bases** — 首次使用时调用一次，获取可用知识库 ID。
   之后无需重复调用。

4. **其他工具** — 按需使用（上传文档、管理索引等）。

## 反模式

- ❌ 不要先 search 再自己总结 — 直接用 ask_knowledge_base，它内部已做 RAG
- ❌ 不要每次都调 list_knowledge_bases — 缓存第一次的结果
- ❌ 不要对同一问题反复 search 不同关键词 — 一次 ask_knowledge_base 通常足够

## 代码文件

知识库中的代码文件按符号边界分块（函数/类/方法），每个 chunk 是完整符号。
chunk metadata 包含 symbol_name、symbol_kind、signature，可用于精确过滤。"#;

type SessionSender = mpsc::UnboundedSender<String>;

fn sse_sessions() -> &'static Arc<RwLock<HashMap<String, SessionSender>>> {
    static SESSIONS: std::sync::OnceLock<Arc<RwLock<HashMap<String, SessionSender>>>> =
        std::sync::OnceLock::new();
    SESSIONS.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

// ---------- MCP Tools ----------

fn tools() -> Vec<Value> {
    vec![
        json!({
            "name": "list_knowledge_bases",
            "description": "List MCP-enabled local knowledge bases.",
            "inputSchema": {
                "type": "object",
                "properties": {},
            },
        }),
        json!({
            "name": "read_knowledge_chunk",
            "description": "Read one chunk from an MCP-enabled knowledge base.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "kbId": { "type": "string" },
                    "chunkId": { "type": "string" },
                },
                "required": ["kbId", "chunkId"],
            },
        }),
    ]
}
