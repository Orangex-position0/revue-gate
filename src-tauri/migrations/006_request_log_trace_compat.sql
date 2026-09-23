-- Trace IDs are present in the initial schema for fresh databases.
-- This migration is intentionally a compatibility marker for databases created by v0.1.
UPDATE request_logs SET trace_id = id WHERE trace_id IS NULL OR trace_id = '';
