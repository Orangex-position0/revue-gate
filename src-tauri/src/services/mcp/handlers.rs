use serde_json::{Value, json};

// ---------- MCP Tools ----------

pub(crate) fn tools() -> Vec<Value> {
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
