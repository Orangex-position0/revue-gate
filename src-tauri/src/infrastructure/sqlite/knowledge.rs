//! SQLite implementation of the Knowledge Service repository.

use sqlx::{Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::domain::error::RepositoryError;
use crate::domain::knowledge::*;
use crate::infrastructure::sqlite::db_err;

const KB_COLUMNS: &str = "id,name,description,status,doc_count,chunk_count,total_tokens,embedding_model,embedding_channel_id,embedding_batch_size,mcp_enabled,chunk_size,chunk_overlap,excluded_dirs,excluded_files,included_files,embedding_dim,index_status,created_at,updated_at";
const DOC_COLUMNS: &str = "id,kb_id,source_id,filename,file_path,file_type,file_size,content_hash,parsed_text,revision,last_ready_at,chunk_count,token_count,status,error_message,source_type,source_url,source_path,doc_meta,created_at,updated_at";
const CHUNK_COLUMNS: &str = "id,doc_id,kb_id,chunk_index,document_revision,content,token_count,embedding,embedding_dim,metadata,symbol_name,symbol_kind,created_at";
const SOURCE_COLUMNS: &str = "id,kb_id,source_type,source_url,source_path,branch,status,file_count,error,created_at,updated_at";
const TASK_COLUMNS: &str = "id,kb_id,source_id,doc_id,task_type,status,progress,total_items,done_items,payload_json,error_message,created_at,started_at,completed_at";
const INDEX_META_COLUMNS: &str = "kb_id,index_type,embedding_dim,chunk_count,embedded_count,fts_status,hnsw_status,index_path,built_at,status,error_message,updated_at";

#[derive(Clone)]
pub struct SqliteKnowledgeRepository {
    pool: SqlitePool,
}
impl SqliteKnowledgeRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl KnowledgeRepository for SqliteKnowledgeRepository {
    async fn list_kbs(&self) -> Result<Vec<KbKnowledgeBase>, RepositoryError> {
        sqlx::query_as(&format!(
            "SELECT {KB_COLUMNS} FROM kb_knowledge_bases ORDER BY name,id"
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }
    async fn get_kb(&self, id: &str) -> Result<Option<KbKnowledgeBase>, RepositoryError> {
        sqlx::query_as(&format!(
            "SELECT {KB_COLUMNS} FROM kb_knowledge_bases WHERE id=?"
        ))
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)
    }
    async fn create_kb(&self, input: CreateKbInput) -> Result<KbKnowledgeBase, RepositoryError> {
        let id = Uuid::now_v7().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO kb_knowledge_bases(id,name,description,embedding_model,embedding_channel_id,embedding_batch_size,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(&id).bind(input.name.trim()).bind(input.description).bind(input.embedding_model).bind(input.embedding_channel_id).bind(input.embedding_batch_size.unwrap_or(32)).bind(&now).bind(&now).execute(&self.pool).await.map_err(db_err)?;
        self.get_kb(&id).await?.ok_or(RepositoryError::NotFound)
    }
    async fn update_kb(
        &self,
        id: &str,
        input: UpdateKbInput,
    ) -> Result<KbKnowledgeBase, RepositoryError> {
        let result=sqlx::query("UPDATE kb_knowledge_bases SET name=COALESCE(?,name),description=COALESCE(?,description),embedding_model=COALESCE(?,embedding_model),embedding_channel_id=COALESCE(?,embedding_channel_id),embedding_batch_size=COALESCE(?,embedding_batch_size),status=COALESCE(?,status),mcp_enabled=COALESCE(?,mcp_enabled),chunk_size=COALESCE(?,chunk_size),chunk_overlap=COALESCE(?,chunk_overlap),excluded_dirs=COALESCE(?,excluded_dirs),excluded_files=COALESCE(?,excluded_files),included_files=COALESCE(?,included_files),updated_at=? WHERE id=?")
            .bind(input.name).bind(input.description).bind(input.embedding_model).bind(input.embedding_channel_id).bind(input.embedding_batch_size).bind(input.status).bind(input.mcp_enabled).bind(input.chunk_size).bind(input.chunk_overlap).bind(input.excluded_dirs).bind(input.excluded_files).bind(input.included_files).bind(chrono::Utc::now().to_rfc3339()).bind(id).execute(&self.pool).await.map_err(db_err)?;
        affected(result.rows_affected())?;
        self.get_kb(id).await?.ok_or(RepositoryError::NotFound)
    }
    async fn delete_kb(&self, id: &str) -> Result<(), RepositoryError> {
        affected(
            sqlx::query("DELETE FROM kb_knowledge_bases WHERE id=?")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?
                .rows_affected(),
        )
    }
    async fn list_documents(&self, kb_id: &str) -> Result<Vec<KbDocument>, RepositoryError> {
        sqlx::query_as(&format!("SELECT {DOC_COLUMNS} FROM kb_documents WHERE kb_id=? AND status!='deleted' ORDER BY created_at DESC")).bind(kb_id).fetch_all(&self.pool).await.map_err(db_err)
    }
    async fn get_document(
        &self,
        kb_id: &str,
        id: &str,
    ) -> Result<Option<KbDocument>, RepositoryError> {
        sqlx::query_as(&format!(
            "SELECT {DOC_COLUMNS} FROM kb_documents WHERE kb_id=? AND id=?"
        ))
        .bind(kb_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)
    }
    async fn upsert_document_pending(
        &self,
        input: UpsertDocumentInput,
    ) -> Result<KbDocument, RepositoryError> {
        let existing: Option<String> = sqlx::query_scalar("SELECT id FROM kb_documents WHERE kb_id=? AND status!='deleted' AND ((? IS NOT NULL AND source_id=? AND source_path=?) OR (? IS NULL AND source_id IS NULL AND filename=?)) ORDER BY created_at DESC LIMIT 1")
            .bind(&input.kb_id)
            .bind(&input.source_id)
            .bind(&input.source_id)
            .bind(&input.source_path)
            .bind(&input.source_id)
            .bind(&input.filename)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let id = existing.unwrap_or_else(|| Uuid::now_v7().to_string());
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO kb_documents(id,kb_id,source_id,filename,file_path,file_type,file_size,content_hash,status,source_type,source_url,source_path,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET file_path=excluded.file_path,file_type=excluded.file_type,file_size=excluded.file_size,content_hash=excluded.content_hash,status=CASE WHEN kb_documents.status='ready' THEN 'ready' ELSE 'pending' END,error_message=NULL,source_url=excluded.source_url,source_path=excluded.source_path,updated_at=excluded.updated_at")
            .bind(&id).bind(&input.kb_id).bind(&input.source_id).bind(&input.filename).bind(&input.file_path).bind(&input.file_type).bind(input.file_size).bind(&input.content_hash).bind(KbDocumentStatus::Pending.to_string()).bind(&input.source_type).bind(&input.source_url).bind(&input.source_path).bind(&now).bind(&now).execute(&self.pool).await.map_err(db_err)?;
        self.get_document(&input.kb_id, &id)
            .await?
            .ok_or(RepositoryError::NotFound)
    }
    async fn replace_document_ready(
        &self,
        id: &str,
        parsed: ParsedDocument,
        chunks: Vec<NewKbChunk>,
    ) -> Result<KbDocument, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let (kb_id, old_revision): (String, i64) =
            sqlx::query_as("SELECT kb_id,revision FROM kb_documents WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?
                .ok_or(RepositoryError::NotFound)?;
        let revision = old_revision + 1;
        let now = chrono::Utc::now().to_rfc3339();
        let token_count: i64 = chunks.iter().map(|c| c.token_count).sum();
        sqlx::query("DELETE FROM kb_chunks_fts WHERE doc_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM kb_chunks WHERE doc_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("UPDATE kb_documents SET parsed_text=?,file_type=?,doc_meta=?,revision=?,last_ready_at=?,chunk_count=?,token_count=?,status='ready',error_message=NULL,updated_at=? WHERE id=?")
            .bind(&parsed.text).bind(&parsed.file_type).bind(parsed.metadata.to_string()).bind(revision).bind(&now).bind(chunks.len() as i64).bind(token_count).bind(&now).bind(id).execute(&mut *tx).await.map_err(db_err)?;
        for chunk in chunks {
            insert_chunk(&mut tx, id, &kb_id, revision, chunk, &now).await?;
        }
        refresh_index_summary_in_tx(&mut tx, &kb_id).await?;
        refresh_stats(&mut tx, &kb_id).await?;
        tx.commit().await.map_err(db_err)?;
        self.get_document(&kb_id, id)
            .await?
            .ok_or(RepositoryError::NotFound)
    }
    async fn mark_document_failed(&self, id: &str, error: String) -> Result<(), RepositoryError> {
        affected(sqlx::query("UPDATE kb_documents SET status=CASE WHEN status='ready' THEN status ELSE 'failed' END,error_message=CASE WHEN status='ready' THEN error_message ELSE ? END,updated_at=? WHERE id=?").bind(error).bind(chrono::Utc::now().to_rfc3339()).bind(id).execute(&self.pool).await.map_err(db_err)?.rows_affected())
    }
    async fn mark_document_deleted(&self, id: &str) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let kb: Option<String> = sqlx::query_scalar("SELECT kb_id FROM kb_documents WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        let kb = kb.ok_or(RepositoryError::NotFound)?;
        sqlx::query("DELETE FROM kb_chunks_fts WHERE doc_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM kb_chunks WHERE doc_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("UPDATE kb_documents SET status='deleted',updated_at=? WHERE id=?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        refresh_stats(&mut tx, &kb).await?;
        refresh_index_summary_in_tx(&mut tx, &kb).await?;
        tx.commit().await.map_err(db_err)
    }
    async fn list_ready_chunks(
        &self,
        kb_id: &str,
        document_id: Option<&str>,
    ) -> Result<Vec<KbChunk>, RepositoryError> {
        sqlx::query_as(&format!("SELECT {CHUNK_COLUMNS} FROM kb_chunks c WHERE c.kb_id=? AND (? IS NULL OR c.doc_id=?) AND EXISTS(SELECT 1 FROM kb_documents d WHERE d.id=c.doc_id AND d.status='ready' AND d.revision=c.document_revision) ORDER BY c.doc_id,c.chunk_index")).bind(kb_id).bind(document_id).bind(document_id).fetch_all(&self.pool).await.map_err(db_err)
    }
    async fn get_chunk(&self, kb_id: &str, id: &str) -> Result<Option<KbChunk>, RepositoryError> {
        sqlx::query_as(&format!("SELECT {CHUNK_COLUMNS} FROM kb_chunks c WHERE c.kb_id=? AND c.id=? AND EXISTS(SELECT 1 FROM kb_documents d WHERE d.id=c.doc_id AND d.status='ready' AND d.revision=c.document_revision)")).bind(kb_id).bind(id).fetch_optional(&self.pool).await.map_err(db_err)
    }
    async fn create_source(
        &self,
        kb_id: &str,
        input: CreateSourceInput,
    ) -> Result<KbSource, RepositoryError> {
        let id = Uuid::now_v7().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO kb_sources(id,kb_id,source_type,source_url,source_path,branch,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)").bind(&id).bind(kb_id).bind(input.source_type).bind(input.source_url).bind(input.source_path).bind(input.branch).bind(&now).bind(&now).execute(&self.pool).await.map_err(db_err)?;
        sqlx::query_as(&format!(
            "SELECT {SOURCE_COLUMNS} FROM kb_sources WHERE id=?"
        ))
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(db_err)
    }
    async fn list_sources(&self, kb_id: &str) -> Result<Vec<KbSource>, RepositoryError> {
        sqlx::query_as(&format!(
            "SELECT {SOURCE_COLUMNS} FROM kb_sources WHERE kb_id=? ORDER BY created_at DESC"
        ))
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }
    async fn delete_source(&self, id: &str) -> Result<(), RepositoryError> {
        affected(
            sqlx::query("DELETE FROM kb_sources WHERE id=?")
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?
                .rows_affected(),
        )
    }
    async fn create_task(&self, input: CreateTaskInput) -> Result<KbTask, RepositoryError> {
        let id = Uuid::now_v7().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO kb_tasks(id,kb_id,source_id,doc_id,task_type,payload_json,created_at) VALUES(?,?,?,?,?,?,?)").bind(&id).bind(input.kb_id).bind(input.source_id).bind(input.doc_id).bind(input.task_type.to_string()).bind(input.payload_json).bind(&now).execute(&self.pool).await.map_err(db_err)?;
        sqlx::query_as(&format!("SELECT {TASK_COLUMNS} FROM kb_tasks WHERE id=?"))
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)
    }
    async fn list_tasks(&self, kb_id: &str) -> Result<Vec<KbTask>, RepositoryError> {
        sqlx::query_as(&format!(
            "SELECT {TASK_COLUMNS} FROM kb_tasks WHERE kb_id=? ORDER BY created_at DESC"
        ))
        .bind(kb_id)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }
    async fn mark_task_running(&self, id: &str) -> Result<(), RepositoryError> {
        affected(
            sqlx::query("UPDATE kb_tasks SET status='running',started_at=? WHERE id=?")
                .bind(chrono::Utc::now().to_rfc3339())
                .bind(id)
                .execute(&self.pool)
                .await
                .map_err(db_err)?
                .rows_affected(),
        )
    }
    async fn mark_task_succeeded(
        &self,
        id: &str,
        completed: String,
    ) -> Result<(), RepositoryError> {
        affected(
            sqlx::query(
                "UPDATE kb_tasks SET status='succeeded',progress=100,completed_at=? WHERE id=?",
            )
            .bind(completed)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?
            .rows_affected(),
        )
    }
    async fn mark_task_failed(&self, id: &str, error: String) -> Result<(), RepositoryError> {
        affected(
            sqlx::query(
                "UPDATE kb_tasks SET status='failed',error_message=?,completed_at=? WHERE id=?",
            )
            .bind(error)
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?
            .rows_affected(),
        )
    }
    async fn recompute_stats(&self, kb_id: &str) -> Result<KbStats, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let stats = refresh_stats(&mut tx, kb_id).await?;
        tx.commit().await.map_err(db_err)?;
        Ok(stats)
    }
    async fn service_stats(&self) -> Result<KnowledgeServiceStats, RepositoryError> {
        sqlx::query_as::<_,(i64,i64,i64,i64)>("SELECT (SELECT COUNT(*) FROM kb_knowledge_bases),(SELECT COUNT(*) FROM kb_documents WHERE status='ready'),(SELECT COUNT(*) FROM kb_tasks WHERE status IN ('pending','running')),(SELECT COUNT(*) FROM kb_tasks WHERE status='failed')").fetch_one(&self.pool).await.map(|v|KnowledgeServiceStats{knowledge_bases:v.0,ready_documents:v.1,pending_tasks:v.2,failed_tasks:v.3}).map_err(db_err)
    }

    async fn save_conversation_message(
        &self,
        kb_id: &str,
        conversation_id: &str,
        message: ConversationMessage,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO kb_conversations(
                id,kb_id,conversation_id,role,content,sources_json,retrieval_json,warnings_json,
                model,token_usage_json,caller_kind,trace_id,created_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(Uuid::now_v7().to_string())
        .bind(kb_id)
        .bind(conversation_id)
        .bind(message.role.to_string())
        .bind(message.content)
        .bind(message.sources_json)
        .bind(message.retrieval_json)
        .bind(message.warnings_json)
        .bind(message.model)
        .bind(message.token_usage_json)
        .bind(message.caller_kind)
        .bind(message.trace_id)
        .bind(message.created_at)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn list_conversation_messages(
        &self,
        kb_id: &str,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationMessage>, RepositoryError> {
        sqlx::query_as::<_, ConversationMessage>(
            "SELECT role,content,sources_json,retrieval_json,warnings_json,model,token_usage_json,caller_kind,trace_id,created_at
             FROM kb_conversations
             WHERE kb_id=? AND conversation_id=?
             ORDER BY created_at DESC,id DESC
             LIMIT ?",
        )
        .bind(kb_id)
        .bind(conversation_id)
        .bind(limit.clamp(1, 200) as i64)
        .fetch_all(&self.pool)
        .await
        .map(|mut rows| {
            rows.reverse();
            rows
        })
        .map_err(db_err)
    }

    async fn clear_conversation(
        &self,
        kb_id: &str,
        conversation_id: &str,
    ) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM kb_conversations WHERE kb_id=? AND conversation_id=?")
            .bind(kb_id)
            .bind(conversation_id)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl KnowledgeIndexRepository for SqliteKnowledgeRepository {
    async fn list_chunks_missing_embedding(
        &self,
        kb_id: &str,
        limit: usize,
    ) -> Result<Vec<KbChunk>, RepositoryError> {
        sqlx::query_as(&format!(
            "SELECT {CHUNK_COLUMNS} FROM kb_chunks c
             WHERE c.kb_id=? AND c.embedding IS NULL
               AND EXISTS(
                   SELECT 1 FROM kb_documents d
                   WHERE d.id=c.doc_id AND d.status='ready' AND d.revision=c.document_revision
               )
             ORDER BY c.doc_id,c.chunk_index
             LIMIT ?"
        ))
        .bind(kb_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }

    async fn update_chunk_embeddings(
        &self,
        updates: Vec<ChunkEmbeddingUpdate>,
    ) -> Result<(), RepositoryError> {
        if updates.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let mut kb_id = None;
        for update in updates {
            let row: Option<String> = sqlx::query_scalar(
                "UPDATE kb_chunks SET embedding=?,embedding_dim=?
                 WHERE id=?
                 RETURNING kb_id",
            )
            .bind(update.embedding)
            .bind(update.embedding_dim)
            .bind(update.chunk_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
            kb_id = row.or(kb_id);
        }
        if let Some(kb_id) = kb_id {
            refresh_index_summary_in_tx(&mut tx, &kb_id).await?;
        }
        tx.commit().await.map_err(db_err)
    }

    async fn clear_embeddings_for_kb(&self, kb_id: &str) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        sqlx::query(
            "UPDATE kb_chunks SET embedding=NULL,embedding_dim=0
             WHERE kb_id=? AND EXISTS(
                 SELECT 1 FROM kb_documents d
                 WHERE d.id=kb_chunks.doc_id
                   AND d.status='ready'
                   AND d.revision=kb_chunks.document_revision
             )",
        )
        .bind(kb_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        refresh_index_summary_in_tx(&mut tx, kb_id).await?;
        tx.commit().await.map_err(db_err)
    }

    async fn sync_fts_rows_for_document(
        &self,
        kb_id: &str,
        document_id: &str,
    ) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        sqlx::query("DELETE FROM kb_chunks_fts WHERE kb_id=? AND doc_id=?")
            .bind(kb_id)
            .bind(document_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        let chunks = list_ready_chunks_for_fts_in_tx(&mut tx, kb_id, Some(document_id)).await?;
        for chunk in &chunks {
            upsert_chunk_fts_in_tx(&mut tx, chunk).await?;
        }
        refresh_index_summary_in_tx(&mut tx, kb_id).await?;
        tx.commit().await.map_err(db_err)
    }

    async fn rebuild_fts_for_kb(&self, kb_id: &str) -> Result<(), RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        sqlx::query("DELETE FROM kb_chunks_fts WHERE kb_id=?")
            .bind(kb_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        let chunks = list_ready_chunks_for_fts_in_tx(&mut tx, kb_id, None).await?;
        for chunk in &chunks {
            upsert_chunk_fts_in_tx(&mut tx, chunk).await?;
        }
        refresh_index_summary_in_tx(&mut tx, kb_id).await?;
        tx.commit().await.map_err(db_err)
    }

    async fn get_index_meta(&self, kb_id: &str) -> Result<KbIndexMeta, RepositoryError> {
        if let Some(meta) = sqlx::query_as(&format!(
            "SELECT {INDEX_META_COLUMNS} FROM kb_index_meta WHERE kb_id=?"
        ))
        .bind(kb_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?
        {
            return Ok(meta);
        }
        self.refresh_index_summary(kb_id).await
    }

    async fn update_index_meta(&self, input: UpdateIndexMetaInput) -> Result<(), RepositoryError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO kb_index_meta(
                kb_id,index_type,embedding_dim,chunk_count,embedded_count,fts_status,
                hnsw_status,index_path,status,error_message,updated_at
             ) VALUES(?,?,?,?,?,?,?,?,?,?,?)
             ON CONFLICT(kb_id) DO UPDATE SET
                index_type=excluded.index_type,
                embedding_dim=excluded.embedding_dim,
                chunk_count=excluded.chunk_count,
                embedded_count=excluded.embedded_count,
                fts_status=excluded.fts_status,
                hnsw_status=excluded.hnsw_status,
                index_path=excluded.index_path,
                status=excluded.status,
                error_message=excluded.error_message,
                updated_at=excluded.updated_at",
        )
        .bind(&input.kb_id)
        .bind(input.index_type)
        .bind(input.embedding_dim)
        .bind(input.chunk_count)
        .bind(input.embedded_count)
        .bind(input.fts_status)
        .bind(input.hnsw_status)
        .bind(input.index_path)
        .bind(input.status.to_string())
        .bind(input.error_message)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn refresh_index_summary(&self, kb_id: &str) -> Result<KbIndexMeta, RepositoryError> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let meta = refresh_index_summary_in_tx(&mut tx, kb_id).await?;
        tx.commit().await.map_err(db_err)?;
        Ok(meta)
    }

    async fn keyword_search(
        &self,
        kb_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KeywordHit>, RepositoryError> {
        let query = build_fts_query(query);
        if query.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as(
            "SELECT fts.chunk_id, bm25(kb_chunks_fts) AS score
             FROM kb_chunks_fts fts
             JOIN kb_documents d
               ON d.id=fts.doc_id
              AND d.revision=fts.document_revision
             WHERE fts.kb_id=?
               AND d.status='ready'
               AND kb_chunks_fts MATCH ?
             ORDER BY score
             LIMIT ?",
        )
        .bind(kb_id)
        .bind(query)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)
    }
}

fn affected(rows: u64) -> Result<(), RepositoryError> {
    if rows == 0 {
        Err(RepositoryError::NotFound)
    } else {
        Ok(())
    }
}
async fn insert_chunk(
    tx: &mut Transaction<'_, Sqlite>,
    doc_id: &str,
    kb_id: &str,
    revision: i64,
    chunk: NewKbChunk,
    now: &str,
) -> Result<(), RepositoryError> {
    let inserted = InsertedChunkForFts {
        id: Uuid::now_v7().to_string(),
        kb_id: kb_id.to_string(),
        doc_id: doc_id.to_string(),
        document_revision: revision,
        content: chunk.content,
        symbol_name: chunk.symbol_name,
        symbol_kind: chunk.symbol_kind,
    };
    sqlx::query("INSERT INTO kb_chunks(id,doc_id,kb_id,chunk_index,document_revision,content,token_count,metadata,symbol_name,symbol_kind,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&inserted.id)
        .bind(doc_id)
        .bind(kb_id)
        .bind(chunk.chunk_index)
        .bind(revision)
        .bind(&inserted.content)
        .bind(chunk.token_count)
        .bind(chunk.metadata.to_string())
        .bind(&inserted.symbol_name)
        .bind(&inserted.symbol_kind)
        .bind(now)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    upsert_chunk_fts_in_tx(tx, &inserted).await?;
    Ok(())
}

struct InsertedChunkForFts {
    id: String,
    kb_id: String,
    doc_id: String,
    document_revision: i64,
    content: String,
    symbol_name: Option<String>,
    symbol_kind: Option<String>,
}

async fn upsert_chunk_fts_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    chunk: &InsertedChunkForFts,
) -> Result<(), RepositoryError> {
    sqlx::query("DELETE FROM kb_chunks_fts WHERE chunk_id=?")
        .bind(&chunk.id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    sqlx::query(
        "INSERT INTO kb_chunks_fts(
            chunk_id,kb_id,doc_id,document_revision,content,symbol_name,symbol_kind
         ) VALUES(?,?,?,?,?,?,?)",
    )
    .bind(&chunk.id)
    .bind(&chunk.kb_id)
    .bind(&chunk.doc_id)
    .bind(chunk.document_revision)
    .bind(&chunk.content)
    .bind(chunk.symbol_name.as_deref().unwrap_or(""))
    .bind(chunk.symbol_kind.as_deref().unwrap_or(""))
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    Ok(())
}

async fn list_ready_chunks_for_fts_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    kb_id: &str,
    document_id: Option<&str>,
) -> Result<Vec<InsertedChunkForFts>, RepositoryError> {
    sqlx::query_as::<
        _,
        (
            String,
            String,
            String,
            i64,
            String,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT c.id,c.kb_id,c.doc_id,c.document_revision,c.content,c.symbol_name,c.symbol_kind
         FROM kb_chunks c
         JOIN kb_documents d
           ON d.id=c.doc_id
          AND d.revision=c.document_revision
         WHERE c.kb_id=?
           AND (? IS NULL OR c.doc_id=?)
           AND d.status='ready'
         ORDER BY c.doc_id,c.chunk_index",
    )
    .bind(kb_id)
    .bind(document_id)
    .bind(document_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(db_err)
    .map(|rows| {
        rows.into_iter()
            .map(
                |(id, kb_id, doc_id, document_revision, content, symbol_name, symbol_kind)| {
                    InsertedChunkForFts {
                        id,
                        kb_id,
                        doc_id,
                        document_revision,
                        content,
                        symbol_name,
                        symbol_kind,
                    }
                },
            )
            .collect()
    })
}

async fn refresh_index_summary_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    kb_id: &str,
) -> Result<KbIndexMeta, RepositoryError> {
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM kb_knowledge_bases WHERE id=?")
        .bind(kb_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?;
    if exists.is_none() {
        return Err(RepositoryError::NotFound);
    }
    let (chunk_count, embedded_count, embedding_dim): (i64, i64, i64) = sqlx::query_as(
        "SELECT
             COUNT(c.id),
             COUNT(CASE WHEN c.embedding IS NOT NULL THEN 1 END),
             COALESCE(MAX(CASE WHEN c.embedding IS NOT NULL THEN c.embedding_dim END),0)
         FROM kb_chunks c
         JOIN kb_documents d
           ON d.id=c.doc_id
          AND d.revision=c.document_revision
         WHERE c.kb_id=?
           AND d.status='ready'",
    )
    .bind(kb_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;
    let fts_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM kb_chunks_fts fts
         JOIN kb_documents d
           ON d.id=fts.doc_id
          AND d.revision=fts.document_revision
         WHERE fts.kb_id=?
           AND d.status='ready'",
    )
    .bind(kb_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;
    let fts_status = if chunk_count == 0 {
        KbIndexStatus::None
    } else if fts_count == chunk_count {
        KbIndexStatus::Ready
    } else {
        KbIndexStatus::NeedsFtsRebuild
    };
    let status = derive_index_status(&IndexHealth {
        ready_chunk_count: chunk_count,
        embedded_count,
        embedding_running: false,
        fts_status,
        hnsw_status: KbIndexStatus::None,
        hnsw_required: false,
        last_error: None,
    });
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO kb_index_meta(
            kb_id,index_type,embedding_dim,chunk_count,embedded_count,fts_status,
            hnsw_status,status,error_message,updated_at
         ) VALUES(?,?,?,?,?,?,?,?,NULL,?)
         ON CONFLICT(kb_id) DO UPDATE SET
            embedding_dim=excluded.embedding_dim,
            chunk_count=excluded.chunk_count,
            embedded_count=excluded.embedded_count,
            fts_status=excluded.fts_status,
            hnsw_status=excluded.hnsw_status,
            status=excluded.status,
            error_message=NULL,
            updated_at=excluded.updated_at",
    )
    .bind(kb_id)
    .bind("linear")
    .bind(embedding_dim)
    .bind(chunk_count)
    .bind(embedded_count)
    .bind(fts_status.to_string())
    .bind(KbIndexStatus::None.to_string())
    .bind(status.to_string())
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    sqlx::query(
        "UPDATE kb_knowledge_bases SET embedding_dim=?,index_status=?,updated_at=? WHERE id=?",
    )
    .bind(embedding_dim)
    .bind(status.to_string())
    .bind(&now)
    .bind(kb_id)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    sqlx::query_as(&format!(
        "SELECT {INDEX_META_COLUMNS} FROM kb_index_meta WHERE kb_id=?"
    ))
    .bind(kb_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)
}

fn build_fts_query(query: &str) -> String {
    query
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .take(16)
        .collect::<Vec<_>>()
        .join(" ")
}
async fn refresh_stats(
    tx: &mut Transaction<'_, Sqlite>,
    kb_id: &str,
) -> Result<KbStats, RepositoryError> {
    let stats:KbStats=sqlx::query_as("SELECT COUNT(DISTINCT d.id) doc_count,COUNT(c.id) chunk_count,COALESCE(SUM(c.token_count),0) total_tokens FROM kb_documents d LEFT JOIN kb_chunks c ON c.doc_id=d.id AND c.document_revision=d.revision WHERE d.kb_id=? AND d.status='ready'").bind(kb_id).fetch_one(&mut **tx).await.map_err(db_err)?;
    let result=sqlx::query("UPDATE kb_knowledge_bases SET doc_count=?,chunk_count=?,total_tokens=?,updated_at=? WHERE id=?").bind(stats.doc_count).bind(stats.chunk_count).bind(stats.total_tokens).bind(chrono::Utc::now().to_rfc3339()).bind(kb_id).execute(&mut **tx).await.map_err(db_err)?;
    affected(result.rows_affected())?;
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::sqlite::init_pool;
    use serde_json::json;
    async fn repo() -> SqliteKnowledgeRepository {
        SqliteKnowledgeRepository::new(init_pool("sqlite::memory:").await.expect("pool"))
    }
    async fn kb(repo: &SqliteKnowledgeRepository) -> KbKnowledgeBase {
        repo.create_kb(CreateKbInput {
            name: "docs".into(),
            description: None,
            embedding_model: None,
            embedding_channel_id: None,
            embedding_batch_size: None,
        })
        .await
        .expect("kb")
    }
    #[tokio::test]
    async fn ready_replacement_is_revisioned_and_atomic() {
        let repo = repo().await;
        let kb = kb(&repo).await;
        let doc = repo
            .upsert_document_pending(UpsertDocumentInput {
                kb_id: kb.id.clone(),
                source_id: None,
                filename: "a.txt".into(),
                file_path: None,
                source_type: "upload".into(),
                source_url: None,
                source_path: None,
                content_hash: "1".into(),
                file_size: 3,
                file_type: "text".into(),
            })
            .await
            .expect("pending");
        let parsed = ParsedDocument {
            text: "one".into(),
            file_type: "text".into(),
            language: None,
            title: None,
            metadata: json!({}),
        };
        let ready = repo
            .replace_document_ready(
                &doc.id,
                parsed.clone(),
                vec![NewKbChunk {
                    chunk_index: 0,
                    content: "one".into(),
                    token_count: 1,
                    metadata: json!({}),
                    symbol_name: None,
                    symbol_kind: None,
                }],
            )
            .await
            .expect("ready");
        assert_eq!(ready.revision, 1);
        let ready = repo
            .replace_document_ready(
                &doc.id,
                ParsedDocument {
                    text: "two".into(),
                    ..parsed
                },
                vec![NewKbChunk {
                    chunk_index: 0,
                    content: "two".into(),
                    token_count: 1,
                    metadata: json!({}),
                    symbol_name: None,
                    symbol_kind: None,
                }],
            )
            .await
            .expect("replace");
        assert_eq!(ready.revision, 2);
        let chunks = repo
            .list_ready_chunks(&kb.id, Some(&doc.id))
            .await
            .expect("chunks");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].content, "two");
    }
    #[tokio::test]
    async fn failure_preserves_ready_revision() {
        let repo = repo().await;
        let kb = kb(&repo).await;
        let doc = repo
            .upsert_document_pending(UpsertDocumentInput {
                kb_id: kb.id.clone(),
                source_id: None,
                filename: "a.txt".into(),
                file_path: None,
                source_type: "upload".into(),
                source_url: None,
                source_path: None,
                content_hash: "1".into(),
                file_size: 3,
                file_type: "text".into(),
            })
            .await
            .expect("pending");
        let ready = repo
            .replace_document_ready(
                &doc.id,
                ParsedDocument {
                    text: "one".into(),
                    file_type: "text".into(),
                    language: None,
                    title: None,
                    metadata: json!({}),
                },
                vec![],
            )
            .await
            .expect("ready");
        repo.mark_document_failed(&doc.id, "boom".into())
            .await
            .expect("mark");
        let after = repo
            .get_document(&kb.id, &doc.id)
            .await
            .expect("get")
            .expect("doc");
        assert_eq!(after.status, KbDocumentStatus::Ready);
        assert_eq!(after.revision, ready.revision);
    }

    #[tokio::test]
    async fn source_documents_are_identified_by_source_path_not_filename() {
        let repo = repo().await;
        let kb = kb(&repo).await;
        let source = repo
            .create_source(
                &kb.id,
                CreateSourceInput {
                    source_type: "local_dir".into(),
                    source_url: None,
                    source_path: Some("C:/docs".into()),
                    branch: None,
                },
            )
            .await
            .expect("source");
        let input = |path: &str| UpsertDocumentInput {
            kb_id: kb.id.clone(),
            source_id: Some(source.id.clone()),
            filename: "README.md".into(),
            file_path: Some(path.into()),
            source_type: "local_dir".into(),
            source_url: None,
            source_path: Some(path.into()),
            content_hash: path.into(),
            file_size: 1,
            file_type: "markdown".into(),
        };
        let first = repo
            .upsert_document_pending(input("a/README.md"))
            .await
            .expect("first");
        let second = repo
            .upsert_document_pending(input("b/README.md"))
            .await
            .expect("second");
        assert_ne!(first.id, second.id);
    }

    #[tokio::test]
    async fn fts_rebuild_indexes_current_ready_chunks() {
        let repo = repo().await;
        let kb = kb(&repo).await;
        let doc = repo
            .upsert_document_pending(UpsertDocumentInput {
                kb_id: kb.id.clone(),
                source_id: None,
                filename: "search.md".into(),
                file_path: None,
                source_type: "upload".into(),
                source_url: None,
                source_path: None,
                content_hash: "1".into(),
                file_size: 10,
                file_type: "markdown".into(),
            })
            .await
            .expect("pending");
        repo.replace_document_ready(
            &doc.id,
            ParsedDocument {
                text: "needle haystack".into(),
                file_type: "markdown".into(),
                language: None,
                title: None,
                metadata: json!({}),
            },
            vec![NewKbChunk {
                chunk_index: 0,
                content: "needle haystack".into(),
                token_count: 2,
                metadata: json!({}),
                symbol_name: Some("needle_fn".into()),
                symbol_kind: Some("function".into()),
            }],
        )
        .await
        .expect("ready");

        repo.rebuild_fts_for_kb(&kb.id).await.expect("rebuild fts");
        let hits = repo
            .keyword_search(&kb.id, "needle", 5)
            .await
            .expect("keyword search");

        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn replacing_document_removes_stale_fts_rows() {
        let repo = repo().await;
        let kb = kb(&repo).await;
        let doc = repo
            .upsert_document_pending(UpsertDocumentInput {
                kb_id: kb.id.clone(),
                source_id: None,
                filename: "search.md".into(),
                file_path: None,
                source_type: "upload".into(),
                source_url: None,
                source_path: None,
                content_hash: "1".into(),
                file_size: 10,
                file_type: "markdown".into(),
            })
            .await
            .expect("pending");
        let parsed = ParsedDocument {
            text: "oldtoken".into(),
            file_type: "markdown".into(),
            language: None,
            title: None,
            metadata: json!({}),
        };
        repo.replace_document_ready(
            &doc.id,
            parsed.clone(),
            vec![NewKbChunk {
                chunk_index: 0,
                content: "oldtoken".into(),
                token_count: 1,
                metadata: json!({}),
                symbol_name: None,
                symbol_kind: None,
            }],
        )
        .await
        .expect("first ready");
        repo.replace_document_ready(
            &doc.id,
            ParsedDocument {
                text: "newtoken".into(),
                ..parsed
            },
            vec![NewKbChunk {
                chunk_index: 0,
                content: "newtoken".into(),
                token_count: 1,
                metadata: json!({}),
                symbol_name: None,
                symbol_kind: None,
            }],
        )
        .await
        .expect("second ready");

        assert!(
            repo.keyword_search(&kb.id, "oldtoken", 5)
                .await
                .expect("old search")
                .is_empty()
        );
        assert_eq!(
            repo.keyword_search(&kb.id, "newtoken", 5)
                .await
                .expect("new search")
                .len(),
            1
        );
    }
}
