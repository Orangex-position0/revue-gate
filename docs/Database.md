# 数据库表结构

> 本文档描述 revue-gate 的 SQLite 数据库表结构。
> 权威来源是 `src-tauri/migrations/001_init.sql`（sqlx 内嵌迁移，`sqlx::migrate!`），改动表结构时必须新增迁移文件并同步更新本文档。

## 通用约定

- **ID**：uuid v7，以 `TEXT` 存储（时间有序主键，`Uuid::now_v7()` 生成）。
- **时间**：一律 UTC ISO-8601 字符串（`DateTime<Utc>`），如 `2026-06-03T15:30:00Z`。
- **布尔值**：用 `INTEGER` 的 `0/1` 表示。
- **数组 / 对象**：以 `JSON` 字符串存储（TEXT 列，如 `'[]'`、`'[{"client_model":"gpt-4o","upstream_model":"gpt-4o"}]'`）。
- **外键**：当前不声明 `FOREIGN KEY` 约束，`request_logs` 里的 `api_key_id` / `channel_id` 只是普通引用字段。

## 表清单

| 表名 | 用途 |
| --- | --- |
| `channels` | 上游渠道（供应商）配置 |
| `api_keys` | 下游 API 密钥与配额 |
| `request_logs` | 请求日志 |

---

## channels（上游渠道）

| 列名 | 类型 | 约束 | 说明 |
| --- | --- | --- | --- |
| `id` | TEXT | PK NOT NULL | uuid v7 |
| `name` | TEXT | NOT NULL | 渠道名称 |
| `channel_type` | TEXT | NOT NULL | `openai` \| `deepseek` \| `custom` \| `claude` \| `gemini` |
| `base_url` | TEXT | | 上游端点地址（可空） |
| `api_key` | TEXT | | 上游密钥，红线：不得下发下游 |
| `models` | TEXT | NOT NULL DEFAULT `'[]'` | 模型列表，JSON 字符串数组 |
| `priority` | INTEGER | NOT NULL DEFAULT `0` | 优先级（调度排序依据） |
| `weight` | INTEGER | NOT NULL DEFAULT `1` | 权重 |
| `model_mappings` | TEXT | NOT NULL DEFAULT `'[]'` | JSON 数组，元素 `{client_model, upstream_model}` |
| `enabled` | INTEGER | NOT NULL DEFAULT `1` | 启用 `1` / 禁用 `0` |
| `last_test_at` | TEXT | | 最近连通性测试时间 |
| `last_test_ok` | INTEGER | | 最近测试结果（`0/1`） |
| `created_at` | TEXT | NOT NULL | 创建时间（UTC） |
| `updated_at` | TEXT | NOT NULL | 更新时间（UTC） |

## api_keys（下游密钥）

| 列名 | 类型 | 约束 | 说明 |
| --- | --- | --- | --- |
| `id` | TEXT | PK NOT NULL | uuid v7 |
| `name` | TEXT | NOT NULL | 密钥名称 |
| `key` | TEXT | NOT NULL UNIQUE | `sk-revue-<16 位随机 hex>`（25 字符） |
| `enabled` | INTEGER | NOT NULL DEFAULT `1` | 启用 `1` / 禁用 `0` |
| `quota_limit` | INTEGER | | 配额上限，`NULL` = 无上限 |
| `quota_used` | INTEGER | NOT NULL DEFAULT `0` | 已用配额 |
| `created_at` | TEXT | NOT NULL | 创建时间（UTC） |
| `updated_at` | TEXT | NOT NULL | 更新时间（UTC） |

## request_logs（请求日志）

| 列名 | 类型 | 约束 | 说明 |
| --- | --- | --- | --- |
| `id` | TEXT | PK NOT NULL | uuid v7 |
| `api_key_id` | TEXT | | 发起请求的密钥 id（可空，无 FK 约束） |
| `channel_id` | TEXT | | 实际转发到的渠道 id（可空，无 FK 约束） |
| `model` | TEXT | NOT NULL | 客户端请求的模型名 |
| `upstream_model` | TEXT | | 映射后转发给上游的模型名 |
| `status_code` | INTEGER | NOT NULL | 返回给下游的状态码 |
| `prompt_tokens` | INTEGER | | 输入 token 数 |
| `completion_tokens` | INTEGER | | 输出 token 数 |
| `total_tokens` | INTEGER | | 总 token 数 |
| `duration_ms` | INTEGER | NOT NULL | 耗时（毫秒） |
| `error_message` | TEXT | | 错误信息（可空） |
| `is_stream` | INTEGER | NOT NULL DEFAULT `0` | 是否流式（`0/1`） |
| `is_retry` | INTEGER | NOT NULL DEFAULT `0` | 是否重试请求（`0/1`） |
| `trace_id` | TEXT | NOT NULL | 请求追踪 id（`x-request-id`） |
| `request_body` | TEXT | | 请求体原文（可空） |
| `created_at` | TEXT | NOT NULL | 创建时间（UTC） |
