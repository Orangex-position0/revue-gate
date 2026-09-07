CREATE TABLE IF NOT EXISTS kb_knowledge_bases (
    id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, status INTEGER NOT NULL DEFAULT 1,
    doc_count INTEGER NOT NULL DEFAULT 0, chunk_count INTEGER NOT NULL DEFAULT 0,
    total_tokens INTEGER NOT NULL DEFAULT 0, embedding_model TEXT, embedding_channel_id TEXT,
    embedding_batch_size INTEGER NOT NULL DEFAULT 32, mcp_enabled INTEGER NOT NULL DEFAULT 1,
    chunk_size INTEGER NOT NULL DEFAULT 512, chunk_overlap INTEGER NOT NULL DEFAULT 64,
    excluded_dirs TEXT NOT NULL DEFAULT '', excluded_files TEXT NOT NULL DEFAULT '',
    included_files TEXT NOT NULL DEFAULT '', embedding_dim INTEGER NOT NULL DEFAULT 0,
    index_status TEXT NOT NULL DEFAULT 'none', created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS kb_sources (
    id TEXT PRIMARY KEY, kb_id TEXT NOT NULL, source_type TEXT NOT NULL, source_url TEXT,
    source_path TEXT, branch TEXT, status TEXT NOT NULL DEFAULT 'pending', file_count INTEGER NOT NULL DEFAULT 0,
    error TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    FOREIGN KEY (kb_id) REFERENCES kb_knowledge_bases(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS kb_documents (
    id TEXT PRIMARY KEY, kb_id TEXT NOT NULL, source_id TEXT, filename TEXT NOT NULL, file_path TEXT,
    file_type TEXT NOT NULL, file_size INTEGER NOT NULL DEFAULT 0, content_hash TEXT NOT NULL,
    parsed_text TEXT, revision INTEGER NOT NULL DEFAULT 0, last_ready_at TEXT,
    chunk_count INTEGER NOT NULL DEFAULT 0, token_count INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending', error_message TEXT, source_type TEXT NOT NULL DEFAULT 'upload',
    source_url TEXT, source_path TEXT, doc_meta TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
    FOREIGN KEY (kb_id) REFERENCES kb_knowledge_bases(id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES kb_sources(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS kb_chunks (
    id TEXT PRIMARY KEY, doc_id TEXT NOT NULL, kb_id TEXT NOT NULL, chunk_index INTEGER NOT NULL,
    document_revision INTEGER NOT NULL DEFAULT 1, content TEXT NOT NULL, token_count INTEGER NOT NULL DEFAULT 0,
    embedding BLOB, embedding_dim INTEGER NOT NULL DEFAULT 0, metadata TEXT NOT NULL DEFAULT '{}',
    symbol_name TEXT, symbol_kind TEXT, created_at TEXT NOT NULL,
    FOREIGN KEY (doc_id) REFERENCES kb_documents(id) ON DELETE CASCADE,
    FOREIGN KEY (kb_id) REFERENCES kb_knowledge_bases(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS kb_tasks (
    id TEXT PRIMARY KEY, kb_id TEXT NOT NULL, source_id TEXT, doc_id TEXT, task_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending', progress INTEGER NOT NULL DEFAULT 0, total_items INTEGER NOT NULL DEFAULT 0,
    done_items INTEGER NOT NULL DEFAULT 0, payload_json TEXT NOT NULL DEFAULT '{}', error_message TEXT,
    created_at TEXT NOT NULL, started_at TEXT, completed_at TEXT,
    FOREIGN KEY (kb_id) REFERENCES kb_knowledge_bases(id) ON DELETE CASCADE,
    FOREIGN KEY (source_id) REFERENCES kb_sources(id) ON DELETE SET NULL,
    FOREIGN KEY (doc_id) REFERENCES kb_documents(id) ON DELETE SET NULL
);
