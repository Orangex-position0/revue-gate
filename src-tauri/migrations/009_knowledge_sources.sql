CREATE INDEX IF NOT EXISTS idx_kb_sources_kb ON kb_sources(kb_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_kb_documents_source_path
    ON kb_documents(kb_id, source_id, source_path) WHERE source_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_kb_documents_kb_status ON kb_documents(kb_id, status);
CREATE INDEX IF NOT EXISTS idx_kb_chunks_doc_revision ON kb_chunks(doc_id, document_revision);
CREATE INDEX IF NOT EXISTS idx_kb_chunks_kb ON kb_chunks(kb_id);
CREATE INDEX IF NOT EXISTS idx_kb_tasks_kb_created ON kb_tasks(kb_id, created_at);
