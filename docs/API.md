# revue-gate 接口文档

> 数据面（Data Plane）HTTP API 参考。revue-gate 对外暴露一个 OpenAI 兼容的本地端点，聚合多个上游供应商（OpenAI / Claude / Gemini / DeepSeek / 自定义 OpenAI-compatible 端点），负责认证、渠道调度、模型映射、转发、配额记账与请求日志。

## 文档信息

| 项 | 值 |
| --- | --- |
| 文档版本 | v1.0.0 |
| 接口版本 | `v1`（URL 前缀 `/v1/`） |
| 协议 | HTTP/1.1 |
| Base URL | `http://127.0.0.1:3000`（默认，可在设置中修改 host / port） |
| 数据格式 | `application/json`（流式接口为 `text/event-stream`） |

## 通用约定

### 请求头

| 请求头 | 类型 | 必填 | 说明 |
| --- | --- | --- | --- |
| `Authorization` | string | 仅 `/v1/chat/completions` | `Bearer <api-key>` 格式；API Key 由控制面管理，形如 `sk-revue-<16位hex>` |
| `Content-Type` | string | 是（有 body 时） | `application/json` |
| `x-request-id` | string | 否 | 客户端可注入的 trace id；未传则由网关用 uuid v7 生成，并在响应头中原样回传 |

### 错误响应格式

所有错误统一返回 OpenAI 风格的错误信封：

```json
{
  "error": {
    "message": "错误描述",
    "type": "错误分类"
  }
}
```

`type` 与状态码的映射关系：

| HTTP 状态码 | `type` | 说明 |
| --- | --- | --- |
| 400 | `invalid_request_error` | 请求体非法（JSON 解析失败 / model 为空等） |
| 401 | `authentication_error` | 缺少 / 无效 / 已禁用的 API Key |
| 404 | `not_found_error` | 没有可用渠道支持该模型 |
| 429 | `rate_limit_error` | 配额耗尽 |
| 502 | `api_error` | 所有候选渠道均失败（上游 5xx / 429 / 传输错误） |
| 500 | `api_error` | 内部仓储错误等 |

---

## 1. 健康检查

### GET `/health`

确认网关在线。无需认证。

**请求参数**：无

**响应参数**：

| 参数名 | 类型 | 必填 | 示例值 | 说明 |
| --- | --- | --- | --- | --- |
| `status` | string | 是 | `ok` | 固定为 `ok` |

**响应示例**：

```json
{ "status": "ok" }
```

---

## 2. 对话补全

### POST `/v1/chat/completions`

OpenAI 兼容的对话补全端点，支持流式与非流式。网关读取请求体中的 `model` 与 `stream` 字段用于调度，其余字段原样透传给上游。

**请求头**：`Authorization: Bearer <api-key>`（必填）；`Content-Type: application/json`

**请求体参数**（OpenAI Chat Completions 格式，除下列字段外均透传）：

| 参数名 | 类型 | 必填 | 默认值 | 取值范围 | 格式 | 示例值 | 备注 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `model` | string | 是 | 无 | 渠道配置的模型名或映射别名 | 字符串 | `gpt-4o` | 空串返回 400；无法路由时返回 404 |
| `messages` | array | 是 | 无 | OpenAI message 数组 | JSON 数组 | `[{"role":"user","content":"hi"}]` | 原样透传，由上游校验 |
| `stream` | boolean | 否 | `false` | `true` / `false` | 布尔 | `false` | `true` 时返回 SSE 流 |

**非流式请求示例**：

```bash
curl http://127.0.0.1:3000/v1/chat/completions \
  -H "Authorization: Bearer sk-revue-0123456789abcdef" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "你好"}]
  }'
```

**非流式响应**：上游响应体原样透传（`application/json`），OpenAI `chat.completion` 格式：

```json
{
  "id": "chatcmpl-123",
  "object": "chat.completion",
  "created": 1700000000,
  "model": "gpt-4o",
  "choices": [
    {
      "index": 0,
      "message": { "role": "assistant", "content": "你好！" },
      "finish_reason": "stop"
    }
  ],
  "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
}
```

**流式响应**：`Content-Type: text/event-stream`，SSE 帧原样透传，以 `data: [DONE]` 收尾：

```text
data: {"id":"chatcmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"你"}}]}

data: [DONE]
```

**行为说明**：

- 网关按 `model` 从启用的渠道中选出候选（按优先级分组、组内加权随机），失败（上游 429 / 5xx / 传输错误）按候选顺序重试，4xx 不重试、原样透传。
- 渠道若配置了模型映射，转发前会改写 `body["model"]` 为上游模型名。
- 流式请求中若上游连接中途断开，客户端会看到无 `[DONE]` 的截断流（响应头已发出，状态码保持 200）。

---

## 3. 模型列表

### GET `/v1/models`

返回所有启用渠道的模型合并去重结果，与 `/v1/chat/completions` 可路由的模型一致。无需认证。

**请求参数**：无

**响应参数**：

| 参数名 | 类型 | 必填 | 示例值 | 说明 |
| --- | --- | --- | --- | --- |
| `object` | string | 是 | `list` | 固定为 `list` |
| `data` | array | 是 | `[]` | 模型对象数组 |
| `data[].id` | string | 是 | `gpt-4o` | 模型名 |
| `data[].object` | string | 是 | `model` | 固定为 `model` |
| `data[].created` | number | 是 | `0` | 恒为 0（本地模型，无时间戳语义） |
| `data[].owned_by` | string | 是 | `revue-gate` | 固定为网关标识 |

**响应示例**：

```json
{
  "object": "list",
  "data": [
    { "id": "gpt-4o", "object": "model", "created": 0, "owned_by": "revue-gate" },
    { "id": "claude-3", "object": "model", "created": 0, "owned_by": "revue-gate" }
  ]
}
```

---

## 接口安全性说明

- **访问授权**：`/v1/chat/completions` 采用 Bearer Token 认证，API Key 由控制面（桌面应用）在本地 SQLite 中管理，仅存于本机；`/health` 与 `/v1/models` 无需认证。
- **数据传输加密**：网关仅监听本地回环地址（默认 `127.0.0.1`），不对外网暴露；数据面与上游供应商之间的传输由各供应商 HTTPS 端点保证。
- **敏感数据标注**：API Key（`sk-revue-*`）属于敏感信息，日志与响应中不会回显；上游响应中的 key 不会被返回，错误响应统一收敛为通用错误信封。
- **配额控制**：每个 API Key 可设置配额上限，超出后返回 429，防止无限消耗。

---

## 文档更新记录

| 版本 | 日期 | 变更摘要 |
| --- | --- | --- |
| v1.0.0 | 2026-08-17 | 首次发布，覆盖 `/health`、`/v1/chat/completions`、`/v1/models` 三个数据面端点 |
