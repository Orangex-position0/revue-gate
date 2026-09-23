-- Add a stable insertion sequence for log pagination and export ordering.
ALTER TABLE request_logs ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;
