-- 002_request_log_audit_fields.sql — 请求日志审计投影字段

ALTER TABLE request_logs ADD COLUMN risk_level TEXT;
ALTER TABLE request_logs ADD COLUMN audit_action TEXT;
ALTER TABLE request_logs ADD COLUMN audit_report TEXT;
