# Knowledge Service 设计

## 1. 文档定位与实现状态

本文是 revue-gate Knowledge Service 的长期设计说明，也是该模块的权威总览。它记录已经落地的系统边界、关键选择、不变量、失败语义和代码入口，供后续开发、排障和演进评审使用。

本文不承担需求清单或阶段计划的职责。早期 spec 中仍有价值的设计背景已沉淀到这里；若本文与代码不一致，应先确认代码变化是否有意，再同步更新本文。专项算法形成稳定实现后，可拆入 `Knowledge_embedding.md` 和 `Knowledge_rag.md`，本文只保留跨流程约束与导航。

文中的状态含义如下：

- **已实现**：当前代码路径可用，并有相应持久化或测试支撑。
- **设计约束**：后续实现和重构必须保持的行为，不代表存在独立组件。
- **待演进**：数据结构或接口可能已预留，但完整能力尚未接通。

### 1.1 能力状态矩阵

| 能力 | 当前状态 | 说明 |
| --- | --- | --- |
| 知识库 CRUD 与统计 | 已实现 | SQLite 持久化，HTTP 和 Tauri 均有管理入口 |
| 单文件上传、解析、分块 | 已实现 | JSON 中传输 Base64，解码后最大 1 MiB，同步完成处理 |
| 文档 revision 与原子替换 | 已实现 | 新 revision、chunks、FTS 和统计在同一事务提交 |
| Keyword 检索 | 已实现 | SQLite FTS5，使用 `unicode61` tokenizer |
| Embedding 生成与保存 | 已实现 | 复用上游 Channel，向量以 little-endian `f32` BLOB 保存 |
| Vector 检索 | 已实现 | 当前为进程内 cosine similarity 线性扫描 |
| Hybrid 检索 | 已实现 | Vector 与 Keyword 使用 RRF 融合，可按索引状态降级 |
| Citation 与过滤 | 已实现 | 引用定位到 chunk 和 document revision |
| RAG 生成与会话记录 | 已实现 | 复用 Gateway proxy；会话保存采用 best effort |
| HTTP Service Module | 已实现 | 通过 Service Registry 挂载在 `/api/kb` |
| Tauri 管理命令 | 已实现 | 覆盖主要管理、检索和会话操作 |
| MCP Knowledge 适配器 | 部分实现 | 工具函数和权限约束已实现，运行时注册尚未接入 |
| `local_dir`、Git、URL 导入与同步 | 待演进 | Source 元数据和任务类型已预留，尚无同步 worker |
| HNSW 索引 | 待演进 | 元数据和状态已预留，当前检索不依赖 HNSW |
| 基于语法树的代码分块 | 待演进 | `symbol_name`、`symbol_kind` 已预留，解析器尚未填充 |
| Streaming RAG | 待演进 | 当前 HTTP、MCP 和 use case 均只接受非流式请求 |

## 2. 为什么知识库属于 revue-gate

revue-gate 已经负责统一管理模型 Channel、模型映射、本地 API Key、路由、失败处理、请求日志、审计和用量。知识库的向量化与问答同样依赖这些能力。若把知识库拆成独立服务，会重新引入一套上游配置、密钥管理和可观测链路，增加部署与状态同步成本。

因此 Knowledge Service 被设计为与 Gateway 同进程运行的 **Service Module**：

- Knowledge 负责文档生命周期、索引、检索、引用和 RAG 上下文构建。
- Gateway 继续负责模型访问、Channel 选择、鉴权、代理、日志与用量。
- HTTP、Tauri 和 MCP 只是同一组 use case 的不同适配层，不各自实现业务规则。
- SQLite 保存可恢复的本地状态，适配个人或小团队的本地知识场景。

这个边界让知识库成为 revue-gate 的增强能力，而不是第二套网关。

## 3. 目标、非目标与系统边界

### 3.1 目标

Knowledge Service 需要提供以下稳定能力：

1. 以 Knowledge Base 为检索与权限边界，管理文档、来源、分块和索引配置。
2. 将可支持的文本输入规范化为 `ParsedDocument`，再生成可检索的 chunks。
3. 保证文档重处理过程中，调用方只能看到一个完整、就绪的 revision。
4. 同时提供 Keyword、Vector 和 Hybrid 检索，并显式报告降级。
5. 生成可追溯到原文 chunk 的 Citation，RAG 不丢失来源信息。
6. 复用 Gateway 的模型调用链路，不在 Knowledge 内部维护第二套凭据和路由。
7. 向本地 UI、HTTP 客户端和外部 Agent 提供一致的业务语义。

### 3.2 非目标

当前设计不追求：

- 通用内容管理系统或多人协同编辑。
- 大规模分布式索引和跨节点一致性。
- OCR、PDF、Office 文档或媒体解析。
- 自动摘要、内容改写或语义清洗。
- 任意本地路径和任意 URL 的无约束抓取。
- 由 Knowledge 自行实现模型路由、计费和密钥管理。

### 3.3 边界规则

Knowledge Base 是最小的配置和暴露边界。每个 Knowledge Base 独立保存文档与来源集合、分块参数、Embedding 配置、索引健康状态，以及是否允许通过 MCP 暴露。

跨 Knowledge Base 检索必须显式指定 `kb_id`、`scope.kb_ids` 或 `scope.all_enabled`。没有范围的全局搜索直接返回 `search_scope_required`，不能悄悄扫描所有数据。

## 4. 端到端工作流总览

```mermaid
flowchart LR
    A[Upload or Source] --> B[Validate and Parse]
    B --> C[Split into Chunks]
    C --> D[Atomic Revision Replace]
    D --> E[FTS5 Index]
    D --> F[Embedding BLOB]
    E --> G[Keyword Search]
    F --> H[Vector Search]
    G --> I[RRF Fusion]
    H --> I
    I --> J[Citation Hydration]
    J --> K[RAG Prompt Budget]
    K --> L[Gateway Proxy]
    L --> M[Answer and Conversation]
```

主要流程分为四段：

1. **摄取**：校验输入，解析文件，按照 Knowledge Base 配置分块。
2. **提交**：在事务中替换文档 revision、chunks、FTS 行和聚合统计。
3. **建索引与检索**：为 ready chunks 生成 embedding，执行 Keyword、Vector 或 Hybrid 检索。
4. **生成**：将结果转换为 Citation，按 token budget 组装上下文，经 Gateway proxy 调用模型。

当前上传接口同步执行前两段，并创建 `process_document` 任务记录状态。Embedding 重建和 FTS 重建由显式接口触发。`KbTask` 目前是可观察的处理记录，不应被理解为已有通用后台任务调度器。

## 5. 核心设计原则与不变量

### 5.1 只有 ready 的当前 revision 可检索

所有读取 chunks、统计索引、构建 FTS 和向量化的查询，都必须同时满足：

```sql
d.status = 'ready'
AND d.revision = c.document_revision
```

不能只按 `kb_id` 或 `doc_id` 读取 chunks。revision 条件是阻止半成品、旧版本和失败重处理结果泄漏到检索面的最后一道约束。

### 5.2 替换成功才推进 revision

解析和分块先在事务外完成。只有准备好完整的新 chunks 后，Repository 才在事务中：

```text
begin
  read old_revision
  new_revision = old_revision + 1
  delete old FTS rows and chunks
  update document as ready with new_revision
  insert new chunks and matching FTS rows
  refresh index summary and KB statistics
commit
```

任一步失败都回滚整个替换。已有 ready 文档在重处理失败后继续保持原 revision 可用；从未成功过的文档才进入 `failed`。

### 5.3 索引是派生状态

`kb_documents.parsed_text` 与当前 revision 的 `kb_chunks` 是可恢复的内容基础。FTS 行、embedding 和未来的 HNSW 都是派生数据，可以重建，不应成为文档真实性的唯一来源。

### 5.4 检索降级必须可见

Hybrid 请求只有在 Keyword 与 Vector 均可用时才真正执行 Hybrid：

- Vector 未就绪但 FTS 就绪时，降级为 Keyword，并返回 `hybrid_degraded_to_keyword`。
- FTS 未就绪但 Vector 就绪时，降级为 Vector，并返回 `hybrid_degraded_to_vector`。
- 两者都未就绪时，返回 `index_not_ready`。
- 显式请求单一模式时，该索引未就绪就报错，不自动切换到另一模式。

降级信息属于响应契约。调用方不能只看 `requested_mode`，还要读取 `actual_mode` 和 `warnings`。

### 5.5 Citation 与答案分离

检索结果先生成结构化 Citation，再进入 RAG。模型输出中的 `[n]` 只是上下文编号，API 返回的 `sources` 才是机器可消费的权威来源信息。

### 5.6 协议层不复制业务规则

HTTP、Tauri 和 MCP 负责参数适配、调用方身份与错误映射。解析、分块、索引状态推导、检索融合和 RAG 编排都位于 domain 或 usecases 层。

## 6. 领域模型与实体关系

### 6.1 实体关系

```text
KbKnowledgeBase
  |-- 0..n KbSource
  |      |-- 0..n KbDocument
  |      `-- 0..n KbTask
  |-- 0..n KbDocument
  |      |-- 0..n KbChunk
  |      `-- 0..n KbTask
  |-- 0..1 KbIndexMeta
  `-- 0..n KbConversationMessage
```

| 实体 | 职责 | 关键字段 |
| --- | --- | --- |
| `KbKnowledgeBase` | 检索、配置和 MCP 暴露边界 | chunk 配置、embedding 配置、缓存统计、`index_status`、`mcp_enabled` |
| `KbSource` | 描述文档来源 | 类型、URL、路径、branch、同步状态与错误 |
| `KbDocument` | 保存文档身份和当前就绪版本 | 来源信息、hash、`parsed_text`、`revision`、状态与统计 |
| `KbChunk` | 最小检索单元 | `document_revision`、正文、token 估算、embedding、symbol 元数据 |
| `KbTask` | 记录处理操作的进度与失败 | task type、status、progress、关联 source/document |
| `KbIndexMeta` | 汇总派生索引健康度 | chunk 数、embedding 数、FTS/HNSW 状态、维度和错误 |
| `kb_conversations` | 保存单库问答历史和审计信息 | role、content、sources、retrieval、warnings、model、usage、caller、trace |

### 6.2 状态模型

文档状态为 `pending -> processing -> ready`，失败时进入 `failed`，逻辑删除时进入 `deleted`。当前上传流程创建 pending 文档后直接处理，Repository 原子提交为 ready；状态枚举保留了更完整的异步处理语义。

任务类型包括 `import_source`、`sync_source`、`process_document`、`reindex_document`。任务状态包括 `pending`、`running`、`succeeded` 和 `failed`。Source 与任务类型的存在是长期模型的一部分，但不代表导入和同步 worker 已实现。

索引总状态由 `derive_index_status` 根据事实推导，依次考虑错误、空库、embedding 进行中、缺失 embedding、FTS 状态和 HNSW 状态。调用方不应自行拼接一套不同的状态规则。

### 6.3 SQLite 表与迁移

| Migration | 内容 |
| --- | --- |
| `008_knowledge_base.sql` | Knowledge Base、Source、Document、Chunk、Task 主表 |
| `009_knowledge_sources.sql` | 来源、文档、revision 和任务查询索引 |
| `010_knowledge_index_and_history.sql` | 索引元数据与会话记录 |
| `011_knowledge_chunk_symbols.sql` | symbol 查询索引 |
| `012_knowledge_chunks_fts.sql` | `kb_chunks_fts` FTS5 虚表 |

Schema 的几个重要选择：

- ID 使用字符串 UUID，便于跨接口传递和本地生成。
- `kb_chunks.embedding` 使用 BLOB，`embedding_dim` 与数据一起保存。
- `kb_documents` 保存解析后的文本，不保存上传原始 bytes。
- `KbSource` 被删除时，数据库外键会级联删除其 documents；任务对 source/document 使用 `SET NULL`，保留操作记录。
- Knowledge Base 上的 doc、chunk 和 token 数是缓存统计，只统计 ready 的当前 revision。

## 7. 文档导入、解析和分块

### 7.1 上传契约

当前可工作的摄取入口是单文件上传：

1. 接收 filename 与 Base64 内容。
2. 解码后校验大小，最大值为 1 MiB。
3. 按扩展名选择解析策略。
4. 根据 Knowledge Base 的 `chunk_size` 与 `chunk_overlap` 分块。
5. 用 filename 识别同一 Knowledge Base 下的重复上传文档。
6. 创建处理任务并原子替换文档内容。

上传内容必须是 UTF-8。当前实现没有编码探测或回退，非法 UTF-8 返回 `invalid_text_encoding`。

### 7.2 支持的格式

| 类别 | 扩展名 | 处理方式 |
| --- | --- | --- |
| 纯文本 | `txt` | 原文保留 |
| Markdown | `md`、`markdown` | 原文保留，分块时识别标题 |
| Web 文本 | `html`、`htm` | 去除标签，保留常用块级元素的换行 |
| 结构化文本 | `json`、`yaml`、`yml`、`toml` | 作为文本处理，不做语义重排 |
| 代码 | `rs`、`ts`、`tsx`、`js`、`jsx`、`py`、`go`、`java` | 作为代码文本处理并记录 language |

PDF、Office、图片和其他二进制格式不在当前范围内。新增解析器时仍应输出统一的 `ParsedDocument`，不要让格式特有逻辑渗透到 Repository 和检索层。

### 7.3 Parser 与 Splitter 分离

Parser 负责 bytes 到 UTF-8 文本的验证与转换、格式识别、必要的文本提取，以及 file type、language 和 parser metadata。Splitter 负责校验分块参数，将文本切成有序 chunks，估算 token 数并继承 metadata。

普通文本按字符窗口切分，步长为 `chunk_size - chunk_overlap`。Markdown 先按标题形成 section，再对 section 使用相同窗口；标题写入 chunk metadata。当前 token 数采用空白分词估算，主要用于统计，不等价于模型 tokenizer。

代码文件目前仍走通用文本分块。`symbol_name` 和 `symbol_kind` 是未来接入 tree-sitter 等语法感知分块器的稳定扩展点，不应在文档中描述为现有能力。

### 7.4 Source 安全边界

Source 支持 `upload`、`local_dir`、`git` 和 `url` 类型的模型与校验：

- HTTP 不允许创建 `local_dir` source，避免远程调用方读取宿主机路径。
- Tauri 可以创建本地目录 source，因为调用方位于本地管理面。
- Git source 只接受 HTTPS URL。
- URL source 只接受 HTTPS，并拒绝代码中识别出的本机和私网目标。

这些校验只保护 Source 配置入口。实际目录扫描、Git 拉取、URL 抓取、增量 diff 和周期同步尚未实现。

## 8. 文档版本、事务边界与失败恢复

### 8.1 文档身份

上传文档以 Knowledge Base 内的 filename 复用已有记录。来自 Source 的文档以 `(kb_id, source_id, source_path)` 唯一标识。文档进入 `deleted` 后，不再参与这个复用查询。

`content_hash` 用于记录输入内容指纹。当前实现使用 Rust `DefaultHasher` 生成十六进制值，只适合当前实现中的变更识别，不应被当作密码学摘要或跨实现稳定协议。

### 8.2 原子替换

`replace_document_ready` 是最重要的写入事务边界。它同时维护 document revision 和 ready 状态、当前 revision 的 chunks、对应的 FTS 行、`KbIndexMeta` 汇总，以及 Knowledge Base 的缓存统计。

禁止把这些步骤拆成协议层的多个独立 Repository 调用，否则进程崩溃时可能留下 ready 文档与索引不一致的状态。

### 8.3 失败语义

处理失败时，task 记录错误。文档失败处理遵循“旧版本优先可用”：

- 若文档从未 ready，则标记为 `failed` 并保存错误。
- 若已有 ready revision，则保持 ready 和旧 revision，不用失败的半成品覆盖它。
- 替换事务失败后，旧 chunks、FTS 和统计通过事务回滚恢复。

对调用方而言，重处理是一次原子切换，不存在可检索的中间版本。

### 8.4 删除语义

删除单个文档会清除它的 chunks 和 FTS 行，并把文档状态更新为 `deleted`，随后刷新统计与索引摘要。删除 Source 当前是物理删除，依靠外键级联删除所属 documents 和 chunks。删除 Knowledge Base 同样依靠级联清理其内容。

若未来需要审计型软删除，必须同时重新设计 Source 删除语义、任务保留周期和统计口径，不能只改一个表的状态字段。

## 9. 向量化与索引维护

### 9.1 Embedding 配置和上游复用

每个 Knowledge Base 可配置 `embedding_model`、`embedding_channel_id` 和 batch size。生成向量时：

1. 若指定 Channel，则选择该 enabled Channel。
2. 否则从 enabled Channels 中寻找支持目标 model 或 model mapping 的 Channel。
3. 使用 OpenAI-compatible embedding client 发起请求。
4. 校验每个 chunk 都有向量，且 batch 内维度一致。
5. 将向量编码为 little-endian `f32` BLOB，并保存维度。

这条链路复用了 revue-gate 的 Channel 配置。Knowledge 不保存独立的 provider key，也不绕过现有模型映射。

更新 Knowledge Base 的 embedding model 后，旧向量不能继续混用。重建流程会清空当前 ready chunks 的 embedding，再按新配置生成。

### 9.2 FTS5

`kb_chunks_fts` 保存 chunk 身份、Knowledge Base、document revision、正文和 symbol 字段。新 revision 提交时，同一事务写入 FTS 行。显式 rebuild 会从 ready 的当前 revision 重建整个 Knowledge Base 的 FTS 数据。

Keyword 查询使用 FTS5 `MATCH` 和 `bm25`，并再次 join `kb_documents` 校验 ready 与 revision。即使 FTS 残留旧行，也不能绕过可见性不变量。

### 9.3 Vector 检索现状

当前 `LinearVectorBackend` 从 SQLite 读取带 embedding 的 ready chunks，在进程内解码 BLOB、计算 cosine similarity、排序并截断结果。这满足本地 MVP 的正确性，但时间和内存开销随 chunk 数线性增长。

`kb_index_meta` 保留 `index_type`、`hnsw_status` 和 `index_path`，领域状态也包含 HNSW 构建阶段。这些字段是演进接口，不表示当前已经存在 HNSW 文件或近似最近邻查询。现有索引摘要明确写入 `index_type = linear`，并且 `hnsw_required = false`。

### 9.4 索引健康度

索引摘要记录 ready chunk 总数、已写入 embedding 的 chunk 数、实际 embedding 维度、FTS 覆盖情况、HNSW 状态和最后错误。

文档 revision 切换会生成没有 embedding 的新 chunks，因此 Knowledge Base 通常先进入 `needs_embedding`。FTS 与 chunks 在同一事务写入，正常上传后 Keyword 可先于 Vector 使用。

## 10. 检索、引用与 RAG 生成

### 10.1 搜索范围和模式

检索输入包含 query、Knowledge Base 范围、mode、limit、filters 和融合配置。limit 被限制在 1 到 50。

支持三种模式：

- `keyword`：FTS5 BM25。
- `vector`：query embedding 加 cosine similarity。
- `hybrid`：分别取得两路排名后进行 RRF 融合。

MCP 调用始终设置 `mcp_only = true`，只能访问 `mcp_enabled` 的 Knowledge Base。HTTP 和 Tauri 的管理权限不由这个字段代替。

### 10.2 RRF 融合

RRF 不直接比较 BM25 与 cosine 的分值尺度，而是累加排名贡献：

```text
score(chunk) = sum(1 / (rrf_k + rank))
```

默认 `rrf_k = 60`。同分时使用稳定的 Knowledge Base ID 与 chunk ID 排序，保证相同输入尽量产生确定结果。响应同时保留 vector、keyword 的原始 score 和 rank，方便排障。

### 10.3 过滤与 Citation

当前过滤字段包括 document ID、source ID、symbol name 和 symbol kind。候选命中会回查 chunk 与 document，再应用过滤并构造 Citation。

Citation 包含 `kb_id`、`doc_id`、`chunk_id`、`chunk_index`、`document_revision`、文件与来源信息、symbol 元数据和 score。`citation_id` 使用 `kb_id:chunk_id`。snippet 会压缩空白并限制为 360 个字符。

读取 Citation 对应内容时仍需走 Repository 的 ready/revision 校验，不能按裸 chunk ID 绕过边界。

### 10.4 RAG 编排

RAG use case 的职责顺序固定为：

1. 执行 Knowledge 检索并保留 `requested_mode`、`actual_mode` 与 warnings。
2. 将结果按排名转成带 `[n]` 编号的上下文块。
3. 先装入检索 chunks，再用剩余 token budget 装入最近会话历史。
4. 通过 `ProxyRequestUsecase` 调用生成模型。
5. 返回 answer、实际使用的 sources、retrieval、usage 和 warnings。
6. 在单 Knowledge Base 会话中 best effort 保存用户与助手消息。

Prompt 明确要求模型仅使用提供的知识上下文，以 `[n]` 引用来源，并在材料不足时说明缺失。若没有任何检索结果，use case 直接返回无相关知识片段，不发起模型请求。

当前 token 估算采用 `ceil(chars / 4)`，不是模型精确 tokenizer。默认上下文预算为 6000 tokens。检索片段优先于 history；超出预算的片段被截断时返回 `rag_context_truncated`。

### 10.5 Gateway 边界与鉴权

RAG 生成必须经过 Gateway proxy，以复用模型路由、Channel、日志、审计和 usage 统计。外部 HTTP 调用要求 Bearer token，并沿用本地 API Key 校验。MCP 适配器使用内部调用身份，但仍受 `mcp_enabled` 限制。

当前只支持非流式回答。`stream = true` 会返回 `streaming_not_supported`，不能在协议层假装流式再缓存完整响应。

## 11. HTTP、Tauri 与 MCP 界面

### 11.1 HTTP

HTTP 路由归属于 `/api/kb` Service Module：

```text
GET,POST       /api/kb
POST           /api/kb/search
POST           /api/kb/ask
GET,PUT,DELETE /api/kb/{kb_id}
POST           /api/kb/{kb_id}/search
POST           /api/kb/{kb_id}/ask
GET             /api/kb/{kb_id}/stats
POST            /api/kb/{kb_id}/stats/recompute
GET,POST        /api/kb/{kb_id}/index
POST            /api/kb/{kb_id}/fts/rebuild
POST            /api/kb/{kb_id}/embeddings/rebuild
GET,POST        /api/kb/{kb_id}/documents
GET,DELETE      /api/kb/{kb_id}/documents/{doc_id}
POST            /api/kb/{kb_id}/documents/{doc_id}/reprocess
GET,POST        /api/kb/{kb_id}/sources
DELETE          /api/kb/{kb_id}/sources/{source_id}
GET             /api/kb/{kb_id}/tasks
```

带 path `kb_id` 的请求会覆盖或校验 body 中的 Knowledge Base 范围，避免一个请求同时指向两个库。全局 `/search` 和 `/ask` 必须在 body 中提供明确 scope。

### 11.2 Tauri

Tauri commands 已覆盖 Knowledge Base 管理、文档上传和列表、Source 创建和列表、索引状态、FTS rebuild、Knowledge 搜索，以及会话历史的 list 和 clear。

Tauri 当前没有独立的 ask command，RAG 问答主要通过本地 HTTP 接口进入。Tauri 的 Source 创建允许 `local_dir`，这是它与外部 HTTP 的有意差异。

### 11.3 MCP

MCP 适配层已经实现以下工具函数语义：

- `list_knowledge_bases`
- `get_knowledge_base_index_status`
- `search_knowledge_base`
- `ask_knowledge_base`
- `read_knowledge_chunk`

列表、搜索、问答和 chunk 读取都会执行 MCP 暴露边界检查。错误被映射为 `invalid_params`、`forbidden`、`not_found`、`unavailable` 或 `internal`。

当前仓库尚未把这些适配函数注册到一个可运行的 MCP server/tool registry。因此它们是可复用的协议适配实现，不是已经可从外部发现的 MCP 工具。接入运行时注册时应复用现有函数，不要在注册层复制检索与权限逻辑。

## 12. Service Module 组装和代码导航

### 12.1 运行时组装

`KnowledgeServiceModule` 实现统一的 `ServiceModule` 接口：

- 稳定 ID 为 `knowledge`。
- 路由前缀为 `/api/kb`。
- `production_service_registry` 负责注册。
- Service settings 决定模块路由是否启用。
- 状态页结合 Repository 是否配置、失败任务数和统计给出运行状态。

Knowledge Repository 在应用启动时基于共享 SQLite pool 创建，同时注入 Tauri state 和 HTTP `AppState`。HTTP 与 Tauri 在处理请求时组装轻量 use case；Query Embedding 动态读取最新 Channel 配置，Vector backend 当前使用线性实现。

Service Module 的统一注册、路由验证和状态语义见 [Service Registry 设计](./Service_Registry.md)。

### 12.2 代码导航

| 关注点 | 代码位置 |
| --- | --- |
| 领域实体、状态、Repository ports、校验 | [`domain/knowledge.rs`](../../../src-tauri/src/domain/knowledge.rs) |
| 管理、解析、分块、摄取、Embedding、线性向量检索 | [`usecases/knowledge.rs`](../../../src-tauri/src/usecases/knowledge.rs) |
| 检索范围、模式规划与降级 | [`usecases/knowledge/retrieval.rs`](../../../src-tauri/src/usecases/knowledge/retrieval.rs) |
| RRF 融合 | [`usecases/knowledge/fusion.rs`](../../../src-tauri/src/usecases/knowledge/fusion.rs) |
| Citation 与过滤 | [`usecases/knowledge/citation.rs`](../../../src-tauri/src/usecases/knowledge/citation.rs) |
| Token budget 与 prompt 组装 | [`usecases/knowledge/token_budget.rs`](../../../src-tauri/src/usecases/knowledge/token_budget.rs) |
| RAG 编排与 Gateway proxy 调用 | [`usecases/knowledge/rag.rs`](../../../src-tauri/src/usecases/knowledge/rag.rs) |
| SQLite 事务、查询、FTS 和会话 | [`infrastructure/sqlite/knowledge.rs`](../../../src-tauri/src/infrastructure/sqlite/knowledge.rs) |
| MCP 适配函数 | [`infrastructure/mcp/knowledge.rs`](../../../src-tauri/src/infrastructure/mcp/knowledge.rs) |
| HTTP 路由与 handler | [`interface/http/knowledge.rs`](../../../src-tauri/src/interface/http/knowledge.rs) |
| Service Module 注册 | [`interface/http/service_modules.rs`](../../../src-tauri/src/interface/http/service_modules.rs) |
| Tauri commands | [`interface/commands/knowledge.rs`](../../../src-tauri/src/interface/commands/knowledge.rs) |
| 应用状态注入与 command 注册 | [`lib.rs`](../../../src-tauri/src/lib.rs) |
| SQLite migrations | [`migrations`](../../../src-tauri/migrations) |

Repository ports 的划分有意区分三类能力：

- `KnowledgeRepository` 管理领域实体、当前 revision chunks、任务和会话。
- `KnowledgeIndexRepository` 负责 embedding、FTS 与索引元数据的维护。
- `KnowledgeIndexReader` 为检索 use case 提供 Vector、Keyword 和状态读取。

后续替换向量索引实现时，应实现新的 `KnowledgeIndexReader` 或配套 Repository adapter，不要改写检索与 RAG 编排。

## 13. 测试策略与关键验收场景

测试重点不是结构体字段是否存在，而是跨层不变量是否成立。

### 13.1 Domain 与纯函数

- chunk 参数与 embedding batch size 的边界。
- Source URL 和本地路径安全限制。
- 索引状态推导优先级。
- Parser 支持列表与非法 UTF-8。
- 普通文本 overlap 和 Markdown heading metadata。
- embedding BLOB 编解码与维度校验。
- cosine similarity、RRF 稳定排序、过滤和 Citation 字段。
- token budget 对 chunks 和最近 history 的取舍。

### 13.2 Repository 集成测试

- 首次处理成功后 document、chunks、FTS、统计和 revision 一致。
- 重处理成功后旧 chunks 不可见，新 revision 可检索。
- 替换失败时旧 ready revision 保持可用。
- deleted 文档不计入统计和索引。
- FTS rebuild 只包含 ready 的当前 revision。
- embedding 更新与清空会刷新索引摘要。
- 会话消息按时间顺序读取，limit 有边界。

### 13.3 Use case 与接口测试

- 没有 scope 的全局搜索被拒绝。
- MCP 不能访问 `mcp_enabled = false` 的库。
- Hybrid 在单路索引不可用时按契约降级并返回 warning。
- 显式 Vector 或 Keyword 请求不进行隐式换轨。
- RAG 没有命中时不调用模型。
- 上下文超预算时 sources 只包含实际送入模型的 chunks。
- 外部 HTTP ask 缺少 Bearer token 时被拒绝。
- `stream = true` 在 HTTP 和 MCP 均明确报错。
- Service Registry 正确注册、启用、禁用和汇报 Knowledge 状态。

文档改动通常不需要运行完整 Rust 测试。实现改动至少运行受影响模块的测试；涉及 migration、事务或检索不变量时，应运行完整 `cargo test`，并使用临时 SQLite 数据库验证迁移链。

## 14. 当前限制、待演进事项和相关文档

### 14.1 当前限制

1. **Source 只有管理模型**：目录扫描、Git clone/pull、URL 抓取、增量同步和后台 worker 尚未实现。
2. **Vector 检索是线性扫描**：适合本地小规模数据，不适合把 HNSW 元数据误当成已上线的 ANN 索引。
3. **代码分块不理解语法结构**：symbol 字段通常为空，代码仍按字符窗口切分。
4. **上传入口有 1 MiB 限制且只接受 UTF-8**：大文件、其他编码与二进制格式需要新的摄取策略。
5. **内容 hash 不是稳定协议**：当前 `DefaultHasher` 不适合跨版本缓存键或完整性校验。
6. **MCP 尚未完成运行时注册**：适配器存在，但外部 Agent 还不能据此假定工具可发现。
7. **RAG 不支持 streaming**：答案完整生成后返回。
8. **Token 估算较粗**：字符数除以 4 不能精确反映不同模型和语言的 tokenizer。
9. **任务不是异步调度系统**：当前上传在请求内完成，任务表主要记录进度和错误。

### 14.2 演进顺序

后续演进应按瓶颈和用户价值推进，而不是因为字段已经预留就一次性实现全部能力：

1. 接通 Source import/sync worker，并先定义取消、重试、删除和路径消失语义。
2. 用稳定 hash 算法和明确的幂等键替换 `DefaultHasher`。
3. 当线性扫描成为可测量瓶颈后，再引入 HNSW adapter、持久化文件和崩溃恢复。
4. 为主要代码语言引入语法感知分块，并保持通用文本 fallback。
5. 把 MCP 适配器接入运行时注册，增加工具发现和权限集成测试。
6. 若产品确实需要，再设计 streaming RAG、断连取消和部分响应审计。
7. 根据实际模型接入 tokenizer，提高上下文预算和 usage 预估精度。

任何演进都必须保持以下跨版本契约：ready/current revision 可见性、原子替换、显式检索范围、可见降级、结构化 Citation，以及生成请求复用 Gateway。

### 14.3 相关文档

- [Service Registry 设计](./Service_Registry.md)：Service Module 注册、启停、路由所有权与状态汇报。
- `Knowledge_embedding.md`：计划承载 Embedding provider 选择、向量存储和 ANN 索引的专项设计；内容稳定前不作为实现依据。
- `Knowledge_rag.md`：计划承载检索评估、上下文编排和生成策略的专项设计；内容稳定前不作为实现依据。
