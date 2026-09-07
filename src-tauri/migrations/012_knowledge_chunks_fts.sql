CREATE VIRTUAL TABLE IF NOT EXISTS kb_chunks_fts USING fts5(
    chunk_id UNINDEXED,
    kb_id UNINDEXED,
    doc_id UNINDEXED,
    document_revision UNINDEXED,
    content,
    symbol_name,
    symbol_kind,
    tokenize = 'unicode61 remove_diacritics 2'
);
