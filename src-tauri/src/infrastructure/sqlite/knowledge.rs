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
    sqlx::query("INSERT INTO kb_chunks(id,doc_id,kb_id,chunk_index,document_revision,content,token_count,metadata,symbol_name,symbol_kind,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(Uuid::now_v7().to_string()).bind(doc_id).bind(kb_id).bind(chunk.chunk_index).bind(revision).bind(chunk.content).bind(chunk.token_count).bind(chunk.metadata.to_string()).bind(chunk.symbol_name).bind(chunk.symbol_kind).bind(now).execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
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
}
