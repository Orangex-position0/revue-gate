-- 001_init.sql — 初始建表：channels / api_keys / request_logs
-- 字段与 docs/Requirements.md 枚举对齐；ID 用 uuid v7 的 TEXT 表示，时间一律 UTC ISO-8601。

CREATE TABLE channels (
    id              TEXT PRIMARY KEY NOT NULL,          -- uuid v7
    name            TEXT NOT NULL,
    channel_type    TEXT NOT NULL,                      -- openai|deepseek|custom|claude|gemini
    base_url        TEXT,
    api_key         TEXT,                               -- 上游密钥，红线：不得下发下游
    models          TEXT NOT NULL DEFAULT '[]',         -- JSON 字符串数组
    priority        INTEGER NOT NULL DEFAULT 0,
    weight          INTEGER NOT NULL DEFAULT 1,
    model_mappings  TEXT NOT NULL DEFAULT '[]',         -- JSON [{client_model, upstream_model}]
    enabled         INTEGER NOT NULL DEFAULT 1,         -- 0/1
    last_test_at    TEXT,
    last_test_ok    INTEGER,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE api_keys (
    id          TEXT PRIMARY KEY NOT NULL,              -- uuid v7
    name        TEXT NOT NULL,
    key         TEXT NOT NULL UNIQUE,                   -- sk-revue-<16 位随机 hex>
    enabled     INTEGER NOT NULL DEFAULT 1,             -- 0/1
    quota_limit INTEGER,                                -- NULL = 无上限
    quota_used  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE request_logs (
    id                TEXT PRIMARY KEY NOT NULL,        -- uuid v7
    api_key_id        TEXT,
    channel_id        TEXT,
    model             TEXT NOT NULL,
    upstream_model    TEXT,
    status_code       INTEGER NOT NULL,
    prompt_tokens     INTEGER,
    completion_tokens INTEGER,
    total_tokens      INTEGER,
    duration_ms       INTEGER NOT NULL,
    error_message     TEXT,
    is_stream         INTEGER NOT NULL DEFAULT 0,       -- 0/1
    is_retry          INTEGER NOT NULL DEFAULT 0,       -- 0/1
    trace_id          TEXT NOT NULL,
    request_body      TEXT,
    created_at        TEXT NOT NULL
);
