# 知识库检索与 RAG 设计

本文沉淀 Knowledge 模块中“query 如何变成可引用回答”的设计。它不是 `Knowledge_RAG_draft.md` 的搬运版；draft 后续可以删除，本文保留长期有用的模块边界、数据流、关键设计决策、当前实现状态和后续演进约束。

## 模块定位

Knowledge RAG 是知识库模块的第三层，位于文档准备和索引能力之后：

```text
Knowledge.md:
  source -> document -> current ready revision chunks

Knowledge_embedding.md:
  ready chunks -> embedding / FTS / vector search / keyword search / index status

Knowledge_rag.md:
  query -> scope -> retrieval -> citation -> prompt -> proxy -> answer/history
```

本模块负责：

- 检索编排：解析范围、检查索引状态、选择 Keyword、Vector 或 Hybrid。
- 融合排序：把 keyword hit 与 vector hit 合并成稳定结果。
- 引用组织：把 chunk 命中映射为结构化 Citation 和 snippet。
- RAG prompt：按 token budget 装入检索上下文和最近历史。
- 回答生成：通过 Gateway proxy 调用模型，不绕过现有 Channel、鉴权、审计和日志。
- 对话历史：保存用户问题、助手回答、sources、retrieval、warnings 和 usage。
- 对外适配：HTTP、Tauri、MCP 的参数映射和错误映射。

本模块不负责：

- 文件上传、source sync、parser、splitter、document revision 生成。
- chunk embedding 批处理、embedding BLOB 编码、FTS5 同步、HNSW 构建。
- 模型 Channel 选择、上游 API key 管理、请求审计和配额扣减。

核心原则是：RAG 只编排检索证据和生成请求，底层内容与索引由前两层提供，模型访问仍交给 Gateway。

## 端到端流程

```text
KnowledgeSearchInput
  -> normalize query / limit
  -> resolve KB scope
  -> plan search by index status
  -> keyword_search and/or vector_search
  -> RRF fusion
  -> hydrate chunks/documents into Citation
  -> KnowledgeSearchResponse

KnowledgeAskInput
  -> KnowledgeRetrievalUsecase::search
  -> pack context chunks before history
  -> build OpenAI-compatible chat request
  -> ProxyRequestUsecase::execute
  -> parse answer and usage
  -> best-effort conversation history
  -> RagAnswer
```

搜索和问答共用检索 DTO，但 use case 分离：

- `KnowledgeRetrievalUsecase` 只负责找证据。
- `KnowledgeRagUsecase` 只负责把证据组织成 prompt 并生成回答。

这个拆分很重要。检索结果可以直接给 UI、Agent 或调试工具使用；RAG 不应该成为检索能力的唯一出口。

## 搜索范围

### 设计原则

跨知识库搜索必须显式指定范围。没有范围的全局搜索会返回 `search_scope_required`，避免调用方不小心扫描所有本地知识库。

范围解析规则：

```text
single KB route:
  path kb_id -> exactly one KB

global HTTP/Tauri:
  body kb_id          -> exactly one KB
  scope.kb_ids set    -> selected KBs
  scope.all_enabled   -> all enabled KBs
  otherwise           -> reject

MCP:
  kb_id present       -> one MCP-enabled KB
  scope.kb_ids set    -> selected MCP-enabled KBs
  no explicit scope   -> all MCP-enabled KBs
```

MCP 永远只能看到 `mcp_enabled != 0` 的 Knowledge Base。HTTP/Tauri 的本地管理权限不由 `mcp_only` 替代。

### 当前实现

范围解析在 `KnowledgeRetrievalUsecase::resolve_search_scope` 中完成。HTTP path `kb_id` 会覆盖 body 的 `kb_id`；如果两者冲突，handler 返回 400。

当前过滤字段包括：

- `doc_id`
- `source_id`
- `symbol_name`
- `symbol_kind`

过滤作用于 current ready revision 的 chunk/document 行。source 过滤需要回查 document，不能只看 chunk。

### 后续扩展

后续可以增加 tag、language、metadata path 等过滤，但不要把过滤逻辑散落到 HTTP/Tauri/MCP 层。新增过滤应进入统一 DTO 和 retrieval hydration 流程。

## 搜索模式与降级

### 设计原则

支持三种搜索模式：

- `keyword`：基于 FTS5 BM25。
- `vector`：query embedding + vector search。
- `hybrid`：keyword 与 vector 分别检索后做融合。

默认模式是 `hybrid`。Hybrid 只有在两路索引都可用时才是真正的 Hybrid；单路不可用时可以降级，但必须在响应中说明。

降级规则：

| 请求模式 | 索引状态 | 行为 |
| --- | --- | --- |
| `hybrid` | keyword + vector ready | Hybrid |
| `hybrid` | keyword ready, vector not ready | 降级 Keyword，返回 `hybrid_degraded_to_keyword` |
| `hybrid` | vector ready, keyword not ready | 降级 Vector，返回 `hybrid_degraded_to_vector` |
| `hybrid` | 两者都不可用 | `index_not_ready` |
| `vector` | vector not ready | `vector_index_not_ready` |
| `keyword` | keyword not ready | `keyword_index_not_ready` |

显式 `keyword` 或 `vector` 不做静默换轨。调用方显式选择模式，通常是在调试或验证某一路索引。

### 当前实现

`KnowledgeRetrievalUsecase::plan_search` 读取 `KnowledgeIndexReader::index_status`，根据 `KbIndexMeta` 规划实际模式和 warnings。

当前 vector ready 判断基于 embedded chunk count 和 summary status。Keyword ready 判断基于 FTS status。索引状态由 `Knowledge_embedding.md` 描述的索引维护层负责刷新。

### 后续扩展

如果未来引入 reranker、adaptive candidate expansion 或 HNSW fallback，仍要保留 `requested_mode`、`actual_mode` 和 warnings。调用方需要知道答案来自哪条路径。

## Query Embedding

### 设计原则

跨 KB 搜索不能复用同一个 query embedding。不同 Knowledge Base 可能使用不同 embedding model 或 channel，向量空间不一定兼容。

正确做法是：每个 KB 使用自己的 embedding 配置生成 query vector。

### 当前实现

HTTP 和 Tauri search 使用动态 query embedding 端口：

- 按目标 KB 读取 `embedding_model` 和 `embedding_channel_id`。
- 若指定 channel，则使用该 enabled channel。
- 若未指定 channel，则从 enabled channels 中寻找支持目标 model 或 model mapping 的 channel。
- 对返回向量做维度校验。

Keyword-only search 不会提前解析 embedding channel。只有 `vector` 或实际执行到 vector 的 `hybrid` 才会触发 query embedding。

### 后续扩展

后续如果支持 provider fallback 或 embedding retry，应该放在 query embedding 端口或 embedding usecase 内，不要让 retrieval usecase 直接理解具体 provider。

## 融合排序

### 设计原则

Keyword 的 BM25 和 Vector 的 cosine similarity 分值尺度不同，不能直接相加。当前使用 Reciprocal Rank Fusion：

```text
score = sum(1 / (rrf_k + rank))
```

默认 `rrf_k = 60`。RRF 只关心各通道内的排名，简单、稳定、足够解释 MVP 行为。

### 当前实现

`usecases/knowledge/fusion.rs` 提供：

- `rank_vector_hits`
- `rank_keyword_hits`
- `rrf_fuse`

融合结果保留：

- fused score
- vector score/rank
- keyword score/rank

同分时按 chunk id 稳定排序，避免相同输入产生随机结果。

### 后续扩展

不要提前引入 reranker 抽象。只有当真实搜索质量不足，并且有评估样本时，再新增 reranker 或调整 fusion 策略。

## Citation 与上下文

### 设计原则

Citation 是机器可消费的来源事实，模型正文里的 `[1]` 只是给人看的引用编号。API 调用方应依赖结构化 `sources`，而不是从回答文本里解析来源。

Citation 的最小来源单元是 chunk，不是 document。document 只提供文件名、路径、source 等上下文元数据。

### 当前实现

`usecases/knowledge/citation.rs` 负责：

- 根据 hit 回查 current ready chunk。
- 回查 document 元数据。
- 应用 doc/source/symbol filters。
- 生成 snippet。
- 构造 `Citation`。

`KnowledgeSearchResult` 内部保留完整 chunk content，但序列化响应只暴露 snippet 和 Citation。RAG prompt 使用 full content，search API 返回短 snippet。

### 后续扩展

如果支持 read chunk 工具或 UI 展开原文，应继续通过 Repository 的 ready/revision 校验读取，不要用裸 chunk id 直接查表绕过可见性约束。

## RAG Prompt 与 Token Budget

### 设计原则

RAG prompt 的优先级是：检索 chunks 优先，历史消息其次。历史是辅助上下文，不能挤掉证据。

MVP 不做对话摘要，不调用第二个模型压缩历史。这样行为确定、成本可控，也少一条失败链路。

### 当前实现

`RagPromptBuilder` 使用粗略 token 估算：

```text
estimated_tokens = ceil(chars / 4)
```

默认 prompt budget 为 6000。构建流程：

1. 写入系统指令。
2. 按检索排名打包 context blocks。
3. 用剩余预算打包最近历史。
4. 最后写入用户问题。

如果部分检索结果因预算被省略，返回 `rag_context_truncated` warning。`sources` 只包含实际进入 prompt 的 Citation。

### 后续扩展

接入精确 tokenizer 前，不要把这个估算当成 provider usage。它只用于本地 prompt 裁剪。未来可按模型提供 tokenizer 或保守预算表。

## 回答生成与历史

### 设计原则

RAG 必须通过 `ProxyRequestUsecase` 生成回答。这样模型路由、Channel 选择、API key 校验、审计、日志和 usage 统计保持在 Gateway 主路径上。

空检索不调用模型。没有证据时继续让模型生成，只会制造不可审计的幻觉。

### 当前实现

`KnowledgeRagUsecase::ask` 执行：

1. 检索。
2. 空结果直接返回固定回答。
3. 构造非流式 OpenAI-compatible chat request。
4. 调用 proxy。
5. 解析 `/choices/0/message/content` 和 usage。
6. best-effort 保存 user 与 assistant 两条 conversation message。

历史保存失败不会让已经成功生成的回答失败。保存内容包括：

- 用户问题
- 助手回答
- sources JSON
- retrieval JSON
- warnings JSON
- model
- token usage
- caller kind
- trace id

### 后续扩展

当前没有 streaming RAG。`stream = true` 会返回 `streaming_not_supported`。真正做 streaming 时，第一帧应先发送 retrieval/sources/warnings，再发送 delta，并定义断连后的日志和历史保存语义。

## 对外入口

### HTTP

当前已接入：

```text
POST /api/kb/search
POST /api/kb/{kb_id}/search
POST /api/kb/ask
POST /api/kb/{kb_id}/ask
```

外部 HTTP ask 必须带本地 API Key：

```text
Authorization: Bearer <local api key>
```

错误映射保持结构化：

- 缺少 API key：401
- scope 缺失或 query 为空：400
- KB 不存在：404
- MCP/权限拒绝：403
- 索引未就绪：409

### Tauri

当前已接入：

- `search_knowledge`
- `list_knowledge_conversation_messages`
- `clear_knowledge_conversation`

Tauri ask 尚未接入。原因不是 DTO 缺失，而是当前 proxy 鉴权路径要求 Bearer token；Tauri 内部身份需要明确设计，不能伪造一个总是失败或绕过审计的入口。

### MCP

当前已实现可复用工具函数：

- `list_knowledge_bases_tool`
- `get_knowledge_base_index_status_tool`
- `search_knowledge_base_tool`
- `ask_knowledge_base_tool`
- `read_knowledge_chunk_tool`

但仓库尚未接入 MCP runtime/tool registry，所以这些函数不是外部 Agent 已可发现的工具。后续注册时应复用这些函数，不要在注册层复制权限判断和检索逻辑。

MCP 约束：

- 只能访问 `mcp_enabled` Knowledge Base。
- search/read 不写 history。
- ask 写 history。
- ask 不支持 streaming。
- 不开放 import/delete/rebuild/update。

## 代码地图

| 位置 | 职责 |
| --- | --- |
| `src-tauri/src/domain/knowledge.rs` | Search/RAG DTO、Citation、warnings、errors、conversation DTO、repository ports |
| `src-tauri/src/usecases/knowledge/retrieval.rs` | scope 解析、模式规划、降级、检索编排 |
| `src-tauri/src/usecases/knowledge/fusion.rs` | RRF 排序融合 |
| `src-tauri/src/usecases/knowledge/citation.rs` | chunk/document hydration、filters、Citation、snippet |
| `src-tauri/src/usecases/knowledge/token_budget.rs` | prompt 组装、context/history 裁剪、token 估算 |
| `src-tauri/src/usecases/knowledge/rag.rs` | ask usecase、proxy request、answer parsing、history best effort |
| `src-tauri/src/interface/http/knowledge.rs` | HTTP search/ask 参数映射、鉴权上下文、错误映射 |
| `src-tauri/src/interface/commands/knowledge.rs` | Tauri search 和 conversation commands |
| `src-tauri/src/infrastructure/sqlite/knowledge.rs` | conversation history 持久化、chunk/document 读取 |
| `src-tauri/src/infrastructure/mcp/knowledge.rs` | MCP Knowledge 工具函数 |

## 当前状态与后续工作

已实现：

- 检索 DTO、RRF、Citation、filters、token budget、非流式 RAG usecase。
- HTTP search/ask。
- Tauri search 与 conversation list/clear。
- MCP 工具函数骨架。
- SQLite conversation history。
- Query embedding 按 KB 动态读取 embedding config。
- 空检索不调用模型。

未完成：

- HTTP/Tauri streaming。
- Tauri ask 内部身份。
- MCP runtime 注册。
- retrieval/RAG 更完整的集成测试。
- reranker、conversation summary、adaptive expansion。

这些缺口应按真实用户价值推进。不要因为 DTO 或字段已经存在，就提前实现没有调用方的抽象。
