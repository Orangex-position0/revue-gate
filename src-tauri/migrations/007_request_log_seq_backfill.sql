-- Backfill sequence values for rows created before the sequence column existed.
UPDATE request_logs SET seq = rowid WHERE seq = 0 OR seq IS NULL;
