CREATE TABLE IF NOT EXISTS kb_index_meta (
    kb_id TEXT PRIMARY KEY, index_type TEXT NOT NULL DEFAULT 'hnsw', embedding_dim INTEGER NOT NULL DEFAULT 0,
    chunk_count INTEGER NOT NULL DEFAULT 0, embedded_count INTEGER NOT NULL DEFAULT 0,
    fts_status TEXT NOT NULL DEFAULT 'none', hnsw_status TEXT NOT NULL DEFAULT 'none', index_path TEXT,
    built_at TEXT, status TEXT NOT NULL DEFAULT 'none', error_message TEXT, updated_at TEXT NOT NULL,
    FOREIGN KEY (kb_id) REFERENCES kb_knowledge_bases(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS kb_conversations (
    id TEXT PRIMARY KEY, kb_id TEXT NOT NULL, conversation_id TEXT NOT NULL, role TEXT NOT NULL,
    content TEXT NOT NULL, sources_json TEXT, retrieval_json TEXT, warnings_json TEXT, model TEXT,
    token_usage_json TEXT, caller_kind TEXT, trace_id TEXT, created_at TEXT NOT NULL,
    FOREIGN KEY (kb_id) REFERENCES kb_knowledge_bases(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_kb_conversations_kb_conversation_created
    ON kb_conversations(kb_id, conversation_id, created_at);
