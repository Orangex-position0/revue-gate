# 多供应商协议适配

> 本文档分析各供应商的协议差异（认证 / 请求体 / 响应格式 / Token 计数字段），以实际代码为准。
> 适配器的架构、trait 契约与装配见 [Provider-adapter.md](./Provider-adapter.md)；本文只讲"协议差异本身"。

## 为什么需要针对不同供应商协议做不同适配

不同模型供应商的 API 协议不同，这包括：

- 认证方式
- 请求体结构
- 响应格式
- Token 计数字段等

统一目标：对外只暴露 OpenAI 兼容协议，对内由各适配器做协议转换，把差异隔离在 `infrastructure/providers/` 目录内（转换逻辑锁定在该目录，见 Spec decision 6）。

## 设计思路

分析不同主流供应商的协议，找出不同点：

- 对于所有 "OpenAI 兼容" 的端点（OpenAI / DeepSeek / Custom），提供一个通用适配器 `OpenAiCompatibleAdaptor`，请求原样转发，不做协议转换。
- 对协议不一致的 Claude（Anthropic Messages API）与 Gemini（generateContent API），分别用 `ClaudeAdaptor` / `GeminiAdaptor` 做协议转换。

## 各供应商协议差异

> 代码依据：`infrastructure/providers/openai.rs` / `claude.rs` / `gemini.rs`。

| 维度 | OpenAI 兼容（OpenAI/DeepSeek/Custom） | Claude（Anthropic） | Gemini |
| --- | --- | --- | --- |
| 认证 | `Authorization: Bearer <key>` | `x-api-key` + `anthropic-version` 两个 header | `x-goog-api-key` header |
| 非流式端点 | `{base}/chat/completions` | `{base}/v1/messages` | `{base}/v1beta/models/{model}:generateContent` |
| 流式端点 | 同上（body `stream:true`） | 同上（body `stream:true`） | `{base}/v1beta/models/{model}:streamGenerateContent?alt=sse` |
| 连通性测试端点 | `{base}/models` | `{base}/v1/models` | `{base}/v1beta/models` |
| 模型发现端点 | `{base}/models`（解析 `data[].id`） | 当前未实现，显式 unsupported | 当前未实现，显式 unsupported |
| 请求体 | `messages[{role,content}]` + `model` / `max_tokens` / `temperature` / `top_p` | `messages[]` + **独立 `system` 字段**；`max_tokens` 必填（缺则注入默认值） | `contents[{role,parts[{text}]}]` + `systemInstruction`；`generationConfig{maxOutputTokens,temperature,topP}` |
| 角色映射 | 原样 | system→`system` 字段；assistant→assistant；其余（tool/function/developer）→user（MVP 取舍） | system→`systemInstruction`；assistant→`role:"model"`；其余→user |
| 响应内容 | `choices[0].message.content` | `content[]` 块内 `text` 拼接 | `candidates[0].content.parts[].text` 拼接 |
| 结束原因 | `finish_reason` | `stop_reason`：`max_tokens`→`length`，其余→`stop` | `finishReason`：`MAX_TOKENS`→`length`，其余→`stop` |
| 流式 | SSE 原样中继 + 行缓冲扫描 usage（只读解析，不改动字节） | Anthropic SSE 事件逐帧转 OpenAI `chat.completion.chunk` + 追加 `[DONE]` | `:streamGenerateContent?alt=sse` + `streamConfig.streamIncludeUsage:true`，逐帧转 chunk + `[DONE]` |

### Token 计数字段映射

三家字段名完全不同，适配器统一收敛为 `TokenUsage`（`domain/provider.rs`）：

| 语义 | OpenAI | Claude | Gemini |
| --- | --- | --- | --- |
| prompt | `usage.prompt_tokens` | `usage.input_tokens` | `usageMetadata.promptTokenCount` |
| completion | `usage.completion_tokens` | `usage.output_tokens` | `usageMetadata.candidatesTokenCount` |
| total | `usage.total_tokens` | 无，由前两项推导 | `usageMetadata.totalTokenCount` |

## 转换要点

- **Claude**：system 消息拆入独立 `system` 字段；`max_tokens` 缺失时注入默认（`DEFAULT_MAX_TOKENS`）；`temperature` / `top_p` 原样透传；响应把 Anthropic `content` 块拼成 OpenAI `message.content`。
- **Gemini**：`max_tokens` / `max_completion_tokens` 都可作输入，缺失注入默认（`DEFAULT_MAX_OUTPUT_TOKENS`）；`top_p` 转 camelCase `topP`；流式开启 `streamIncludeUsage` 让末帧携带 `usageMetadata` 以便解析。

## 统一出口

无论上游是哪家，`forward` / `forward_stream` 对外输出的都是 OpenAI 兼容的 JSON / SSE，下游只感知一种协议。适配器返回 domain 类型（`ProviderResponse` / `StreamEvent` / `TokenUsage`），不暴露 reqwest / axum。

> 安全红线：上游错误体可能回显密钥（OpenAI 兼容端点的 401 会回显 `sk-...`），适配器一律替换为通用错误体，保留状态码。详见 [Provider-adapter.md](./Provider-adapter.md)。
