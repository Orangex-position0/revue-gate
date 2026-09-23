use serde_json::{Value, json};

// ---------- MCP Tools ----------

pub(crate) fn tools() -> Vec<Value> {
    // Keep the public registry stable: clients can discover the complete Knowledge Service
    // surface even when a particular optional backend is not configured.
    let definitions = [
        (
            "search_knowledge_base",
            "Search MCP-enabled knowledge bases.",
            &["query"] as &[&str],
        ),
        (
            "list_knowledge_bases",
            "List MCP-enabled local knowledge bases.",
            &[],
        ),
        (
            "read_document",
            "Read a document from an MCP-enabled knowledge base.",
            &["kb_id", "doc_id"],
        ),
        (
            "ask_knowledge_base",
            "Ask a question using local RAG.",
            &["question"],
        ),
        (
            "get_knowledge_base_stats",
            "Get knowledge base statistics.",
            &["kb_id"],
        ),
        (
            "create_knowledge_base",
            "Create a knowledge base.",
            &["name"],
        ),
        (
            "update_knowledge_base",
            "Update a knowledge base.",
            &["kb_id"],
        ),
        (
            "delete_knowledge_base",
            "Delete a knowledge base.",
            &["kb_id"],
        ),
        (
            "upload_document",
            "Upload a document to a knowledge base.",
            &["kb_id", "filename", "content"],
        ),
        (
            "delete_document",
            "Delete a document.",
            &["kb_id", "doc_id"],
        ),
        (
            "list_documents",
            "List documents in a knowledge base.",
            &["kb_id"],
        ),
        ("build_index", "Build the knowledge base index.", &["kb_id"]),
        (
            "import_source",
            "Import documents from an external source.",
            &["kb_id", "source_type"],
        ),
    ];
    definitions
        .into_iter()
        .map(|(name, description, required)| {
            json!({
                "name": name,
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": required,
                },
            })
        })
        .collect()
}
