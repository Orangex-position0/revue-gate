CREATE INDEX IF NOT EXISTS idx_chunks_symbol ON kb_chunks(kb_id, symbol_kind)
    WHERE symbol_name IS NOT NULL;
