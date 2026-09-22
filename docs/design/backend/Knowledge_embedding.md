# 知识库向量化与索引维护

本文沉淀 Knowledge 模块中“ready chunks 如何变成可检索资产”的设计。它不是实现草稿的搬运版，也不是 HNSW 选型报告；它解释当前代码已经落地的 MVP、关键边界、状态模型和后续演进约束。

本模块的上游是 Knowledge ingestion：文档已经解析、分块，并以 `kb_documents.status = ready` 和 current `revision` 写入 `kb_chunks`。本模块的下游是检索/RAG：它只暴露 query embedding、vector/keyword search primitive 和 index status，不负责引用组织、rank fusion、prompt 构建或回答生成。

## 职责边界

本模块负责：

- chunk embedding 的生成、编码、维度校验和写入。
- FTS5 全文索引表的显式同步和重建。
- 向量检索 primitive：当前 MVP 使用线性扫描 fallback，HNSW 是后续性能缓存。
- `kb_index_meta` 和 `kb_knowledge_bases.index_status` 的摘要刷新。
- HTTP/Tauri 的本地维护入口，以及 MCP 的只读 index status 能力。

本模块不负责：

- 文件上传、source sync、parser、splitter、document revision 生成。
- RAG 检索编排、hybrid fusion、citation、prompt、answer streaming、conversation history。
- 把 HNSW 作为权威存储。权威数据始终是 SQLite 中的 current ready chunks 和 embedding BLOB。

核心边界是：

```rust
#[async_trait::async_trait]
pub trait KnowledgeIndexReader: Send + Sync {
    async fn vector_search(
        &self,
        kb_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorHit>, IndexError>;

    async fn keyword_search(
        &self,
        kb_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KeywordHit>, IndexError>;

    async fn index_status(&self, kb_id: &str) -> Result<KbIndexMeta, IndexError>;
}
```

第三模块只依赖这些 primitive，不依赖 SQLite FTS5、HNSW 库或具体索引文件格式。

## 数据流总览

当前数据流按 Knowledge Base 独立维护：

```text
ready document revision
  -> current revision chunks
  -> chunk embedding BLOB
  -> kb_chunks_fts rows
  -> vector search fallback / future HNSW cache
  -> kb_index_meta + kb_knowledge_bases.index_status
  -> retrieval / RAG primitives
```

所有查询都只面向 current ready revision。旧 revision 的 chunk、已删除文档的 chunk、非 ready 文档的 chunk 都不能进入 FTS、vector search 或 RAG citation。

## Embedding 存储与生成

`kb_chunks.embedding` 使用裸 `f32 little-endian` BLOB。这个格式足够简单、长期稳定、跨语言可读，也不把 Rust 序列化库绑定进数据库 schema。

编码/解码规则：

- `Vec<f32>` 按 `to_le_bytes()` 顺序拼接。
- BLOB 长度必须能被 4 整除。
- `embedding_dim` 是运行期维度校验字段，不从外部请求临时推断。
- 维度不匹配应返回稳定错误或跳过 dirty chunk，不能静默混入结果。

chunk embedding 和 query embedding 都必须使用 KB 自己的 embedding 配置：

- `kb_knowledge_bases.embedding_model`
- `kb_knowledge_bases.embedding_channel_id`
- `kb_knowledge_bases.embedding_batch_size`

外部调用方不能临时传入一个 embedding model 去搜索某个 KB。否则同一个 KB 的 chunk embedding 和 query embedding 可能落在不同向量空间，结果没有意义。

当前实现中的关键角色：

- `EmbeddingClient`：domain trait，抽象 embedding provider。
- `OpenAiCompatibleEmbeddingClient`：调用 `{base_url}/embeddings`，复用 channel 的 base URL 和 API key。
- `EmbeddingUsecase`：按 KB batch size 找 missing chunks、调用 client、写入成功 batch。
- `QueryEmbeddingUsecase`：读取 KB embedding config，生成 query vector，并校验维度。

上游错误必须脱敏。provider 返回的错误正文可能包含 API key 或内部细节，控制面只暴露状态码和稳定错误摘要。

## FTS5 全文索引

FTS5 是 `kb_chunks` 的派生索引，用于 keyword search primitive。它不是权威数据；缺失或损坏时可以从 current ready chunks 重建。

迁移表：

```sql
CREATE VIRTUAL TABLE IF NOT EXISTS kb_chunks_fts USING fts5(
    chunk_id UNINDEXED,
    kb_id UNINDEXED,
    doc_id UNINDEXED,
    document_revision UNINDEXED,
    content,
    symbol_name,
    symbol_kind,
    tokenize = 'unicode61 remove_diacritics 2'
);
```

设计决策：不用 SQLite trigger 同步 FTS5。

原因很直接：trigger 会把副作用藏进数据库层。后续开发者读 Rust repository 时只看到 `kb_chunks` 写入，却看不到 FTS row 被自动改了；迁移审查、排障和测试都会变难。当前项目选择应用层显式维护：

- 写入新 chunk 时，同一事务内 upsert `kb_chunks_fts`。
- 替换或删除文档时，先删 FTS row，再删 chunk row。
- 迁移回填、异常恢复和调试通过 `rebuild_fts_for_kb()` 重建。
- keyword search 必须 join `kb_documents`，只返回 current ready revision。

关键词查询的关键约束：

```sql
SELECT
    fts.chunk_id,
    bm25(kb_chunks_fts) AS score
FROM kb_chunks_fts fts
JOIN kb_documents d
    ON d.id = fts.doc_id
   AND d.revision = fts.document_revision
WHERE fts.kb_id = ?
  AND d.status = 'ready'
  AND kb_chunks_fts MATCH ?
ORDER BY score
LIMIT ?;
```

FTS query string 由 repository 内部从用户输入构造，不能把原始用户字符串无约束拼进 `MATCH`。

## 向量检索与 HNSW 演进

MVP 必须有 `LinearVectorBackend`，它扫描 current ready revision 中已经有 embedding 的 chunks，计算 cosine similarity，按 score 降序返回 `VectorHit`。

这个 fallback 有几个作用：

- 在 HNSW 未接入时提供可用的 vector primitive。
- 在 HNSW 文件损坏、缺失或维度不兼容时兜底。
- 为测试提供稳定、不依赖外部索引库的实现。

HNSW 是后续性能缓存，不是权威数据。引入 HNSW 时必须遵守：

- 从 SQLite current ready embedded chunks 构建。
- 维度不匹配的 dirty chunks 要跳过并记录。
- 索引文件可删除、可重建。
- library-specific 类型只能留在 infrastructure/backend 内部。
- manifest 必须包含 `format_version`、`kb_id`、`embedding_dim`、chunk id 映射和构建参数。
- HNSW 构建失败不应让知识库完全不可检索；keyword search 和 linear vector fallback 仍应可用。

不在本文展开 `hnswlib-rs`、`hnsw_rs`、`usearch`、Qdrant 等库选型。真正开始接 HNSW 时，应单独写 ADR 或 implementation note。

## 索引状态模型

索引状态是这个模块的核心，不是附录。UI、HTTP、Tauri、MCP 和排障都依赖它。

两层状态：

- `kb_knowledge_bases.index_status`：摘要状态，给列表页和快速判断用。
- `kb_index_meta`：细分状态，解释 embedding、FTS、HNSW 哪一段不可用。

`KbIndexMeta` 当前字段：

```rust
pub struct KbIndexMeta {
    pub kb_id: String,
    pub index_type: String,
    pub embedding_dim: i64,
    pub chunk_count: i64,
    pub embedded_count: i64,
    pub fts_status: String,
    pub hnsw_status: String,
    pub index_path: Option<String>,
    pub built_at: Option<String>,
    pub status: KbIndexStatus,
    pub error_message: Option<String>,
    pub updated_at: String,
}
```

状态语义：

| 状态 | 含义 |
| --- | --- |
| `none` | 没有 current ready chunks，或从未开始索引 |
| `needs_embedding` | current ready chunks 中仍有缺失 embedding |
| `embedding` | 正在生成 chunk embedding |
| `needs_fts_rebuild` | FTS5 派生索引缺失、过期或需要修复 |
| `fts_building` | 正在重建 FTS5 |
| `needs_hnsw_rebuild` | embedding/FTS 可用，但 HNSW 性能缓存缺失或过期 |
| `hnsw_building` | 正在构建 HNSW |
| `ready` | 当前可检索资产满足要求 |
| `failed` | 最近一次索引维护失败，查看 `error_message` |

状态必须由 current counts 和细分状态推导，handler 不应该随手写裸字符串。

当前 summary refresh 的事实来源：

- current ready chunk 数量。
- current ready chunk 中 embedding 非空数量。
- current ready FTS row 数量。
- 当前实现 `hnsw_required = false`，所以 HNSW 不参与 ready 判定。

注意：未来 `needs_hnsw_rebuild` 不等于知识库不可检索。它表示性能缓存需要重建，keyword search 和 linear vector fallback 仍然可以工作。

## 对外入口边界

HTTP/Tauri 可以提供本地维护入口，因为它们运行在本地控制面，允许执行高影响写操作：

- 查看索引：`GET /api/kb/{kb_id}/index`
- 重建整体索引：`POST /api/kb/{kb_id}/index`
- 重建 embeddings：`POST /api/kb/{kb_id}/embeddings/rebuild`
- 重建 FTS：`POST /api/kb/{kb_id}/fts/rebuild`
- Tauri command：`get_knowledge_base_index_status`
- Tauri command：`rebuild_knowledge_fts_index`

MCP MVP 只开放只读状态和检索/问答能力，不开放 rebuild/import/delete/update：

- 允许：`get_knowledge_base_index_status`
- 不允许：`rebuild_embeddings`、`rebuild_fts_index`、`rebuild_hnsw_index`、修改 embedding config

原因是 rebuild 会消耗 provider 额度、写 SQLite、未来还会修改本地索引文件；MCP 工具面默认应避免高影响维护操作。

## 当前实现边界与后续演进

当前已落地：

- `kb_chunks.embedding` / `embedding_dim`。
- `kb_index_meta` 和 `kb_knowledge_bases.index_status`。
- `kb_chunks_fts` FTS5 表。
- embedding BLOB encode/decode。
- OpenAI-compatible `/embeddings` client。
- `EmbeddingUsecase` 批量补齐 missing chunks。
- `QueryEmbeddingUsecase`。
- `KnowledgeIndexRepository` / `KnowledgeIndexReader`。
- `LinearVectorBackend`。
- FTS 显式同步、FTS rebuild、keyword search。
- HTTP/Tauri index status 和 rebuild 入口。
- MCP 只读 index status 工具。

暂未作为完整能力落地：

- HNSW backend、manifest、索引文件持久化。
- 自动后台 orchestrator 队列。
- HNSW 失败后的状态细分和自动降级告警。
- embedding model/channel 变化时的完整清空旧 embedding + drop HNSW 流程。
- provider retry/fallback 的细粒度策略沉淀到 embedding usecase。

后续演进时不要破坏这些 contract：

- RAG 只能依赖 `KnowledgeIndexReader` 和 query embedding primitive。
- SQLite embedding BLOB 格式保持 `f32 little-endian`。
- FTS 仍通过 repository 显式同步，不加隐藏 trigger。
- MCP 不开放高影响 rebuild 写操作，除非有新的权限模型。
- HNSW 只能是可重建缓存，不能成为唯一权威索引。

## 代码地图

当前代码仍以单文件为主，后续体量继续增长时再拆目录；不要为了贴合旧 spec 先做空拆分。

| 位置 | 职责 |
| --- | --- |
| `src-tauri/src/domain/knowledge.rs` | Knowledge domain 类型、索引状态、embedding/index trait |
| `src-tauri/src/usecases/knowledge.rs` | embedding codec、embedding usecase、query embedding、linear vector backend |
| `src-tauri/src/usecases/knowledge/retrieval.rs` | 检索编排和 RAG 依赖的 search DTO/usecase |
| `src-tauri/src/usecases/knowledge/rag.rs` | RAG ask usecase |
| `src-tauri/src/infrastructure/sqlite/knowledge.rs` | SQLite repository、FTS 同步、index summary refresh |
| `src-tauri/src/infrastructure/providers/openai.rs` | OpenAI-compatible chat/embedding provider |
| `src-tauri/src/interface/http/knowledge.rs` | Knowledge HTTP 管理、search、ask 入口 |
| `src-tauri/src/interface/commands/knowledge.rs` | Tauri Knowledge commands |
| `src-tauri/src/infrastructure/mcp/knowledge.rs` | MCP Knowledge tools |
| `src-tauri/migrations/012_knowledge_chunks_fts.sql` | FTS5 virtual table |

## 测试与排障

最小测试面：

- embedding codec：round-trip、非法长度、维度不匹配。
- embedding usecase：多批 missing chunks 能继续补齐；部分失败保留成功 batch。
- FTS：ready chunk 能被索引；文档替换后旧 FTS row 不再命中。
- vector fallback：cosine similarity 排序稳定。
- index summary：ready chunk、embedded count、FTS count 推导状态正确。
- MCP/HTTP/Tauri：高影响 rebuild 只在本地控制面开放。

常见排障路径：

1. 文档是否是 current `ready` revision。
2. `kb_index_meta.chunk_count` 是否大于 0。
3. `embedded_count < chunk_count` 时先查 embedding config 和 provider。
4. `fts_status != ready` 时执行 FTS rebuild。
5. vector search 无结果时检查 query embedding 维度和 chunk embedding 维度。
6. 未来 HNSW 出问题时先降级 linear fallback，再重建 HNSW 缓存。
