//! Knowledge Service Module domain types and repository boundary.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::error::RepositoryError;

macro_rules! text_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
        #[serde(rename_all = "snake_case")]
        #[sqlx(type_name = "TEXT", rename_all = "snake_case")]
        pub enum $name { $($variant),+ }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(match self { $(Self::$variant => $value),+ })
            }
        }
    };
}

text_enum!(KbDocumentStatus { Pending => "pending", Processing => "processing", Ready => "ready", Failed => "failed", Deleted => "deleted" });
text_enum!(KbSourceStatus { Pending => "pending", Importing => "importing", Ready => "ready", Syncing => "syncing", Failed => "failed", Disabled => "disabled" });
text_enum!(KbTaskType { ImportSource => "import_source", SyncSource => "sync_source", ProcessDocument => "process_document", ReindexDocument => "reindex_document" });
text_enum!(KbTaskStatus { Pending => "pending", Running => "running", Succeeded => "succeeded", Failed => "failed" });
text_enum!(KbIndexStatus { None => "none", NeedsEmbedding => "needs_embedding", Embedding => "embedding", NeedsFtsRebuild => "needs_fts_rebuild", FtsBuilding => "fts_building", NeedsHnswRebuild => "needs_hnsw_rebuild", HnswBuilding => "hnsw_building", Ready => "ready", Failed => "failed" });
text_enum!(ConversationRole { User => "user", Assistant => "assistant" });

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KbKnowledgeBase {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub status: i64,
    pub doc_count: i64,
    pub chunk_count: i64,
    pub total_tokens: i64,
    pub embedding_model: Option<String>,
    pub embedding_channel_id: Option<String>,
    pub embedding_batch_size: i64,
    pub mcp_enabled: i64,
    pub chunk_size: i64,
    pub chunk_overlap: i64,
    pub excluded_dirs: String,
    pub excluded_files: String,
    pub included_files: String,
    pub embedding_dim: i64,
    pub index_status: KbIndexStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KbSource {
    pub id: String,
    pub kb_id: String,
    pub source_type: String,
    pub source_url: Option<String>,
    pub source_path: Option<String>,
    pub branch: Option<String>,
    pub status: KbSourceStatus,
    pub file_count: i64,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KbDocument {
    pub id: String,
    pub kb_id: String,
    pub source_id: Option<String>,
    pub filename: String,
    pub file_path: Option<String>,
    pub file_type: String,
    pub file_size: i64,
    pub content_hash: String,
    pub parsed_text: Option<String>,
    pub revision: i64,
    pub last_ready_at: Option<String>,
    pub chunk_count: i64,
    pub token_count: i64,
    pub status: KbDocumentStatus,
    pub error_message: Option<String>,
    pub source_type: String,
    pub source_url: Option<String>,
    pub source_path: Option<String>,
    pub doc_meta: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KbChunk {
    pub id: String,
    pub doc_id: String,
    pub kb_id: String,
    pub chunk_index: i64,
    pub document_revision: i64,
    pub content: String,
    pub token_count: i64,
    pub embedding: Option<Vec<u8>>,
    pub embedding_dim: i64,
    pub metadata: String,
    pub symbol_name: Option<String>,
    pub symbol_kind: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KbTask {
    pub id: String,
    pub kb_id: String,
    pub source_id: Option<String>,
    pub doc_id: Option<String>,
    pub task_type: KbTaskType,
    pub status: KbTaskStatus,
    pub progress: i64,
    pub total_items: i64,
    pub done_items: i64,
    pub payload_json: String,
    pub error_message: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedDocument {
    pub text: String,
    pub file_type: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewKbChunk {
    pub chunk_index: i64,
    pub content: String,
    pub token_count: i64,
    pub metadata: Value,
    pub symbol_name: Option<String>,
    pub symbol_kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateKbInput {
    pub name: String,
    pub description: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_channel_id: Option<String>,
    pub embedding_batch_size: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateKbInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_channel_id: Option<String>,
    pub embedding_batch_size: Option<i64>,
    pub status: Option<i64>,
    pub mcp_enabled: Option<i64>,
    pub chunk_size: Option<i64>,
    pub chunk_overlap: Option<i64>,
    pub excluded_dirs: Option<String>,
    pub excluded_files: Option<String>,
    pub included_files: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadDocumentInput {
    pub filename: String,
    pub content_base64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSourceInput {
    pub source_type: String,
    pub source_url: Option<String>,
    pub source_path: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertDocumentInput {
    pub kb_id: String,
    pub source_id: Option<String>,
    pub filename: String,
    pub file_path: Option<String>,
    pub source_type: String,
    pub source_url: Option<String>,
    pub source_path: Option<String>,
    pub content_hash: String,
    pub file_size: i64,
    pub file_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTaskInput {
    pub kb_id: String,
    pub source_id: Option<String>,
    pub doc_id: Option<String>,
    pub task_type: KbTaskType,
    pub payload_json: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct KbStats {
    pub doc_count: i64,
    pub chunk_count: i64,
    pub total_tokens: i64,
}

pub const MAX_UPLOAD_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KnowledgeValidationError {
    #[error("name_required")]
    NameRequired,
    #[error("embedding_batch_size_out_of_range")]
    InvalidEmbeddingBatchSize,
    #[error("invalid_chunk_config")]
    InvalidChunkConfig,
    #[error("upload_too_large")]
    UploadTooLarge,
    #[error("invalid_source")]
    InvalidSource,
    #[error("unsafe_source_url")]
    UnsafeSourceUrl,
}

pub fn validate_embedding_batch_size(value: Option<i64>) -> Result<(), KnowledgeValidationError> {
    if value.is_some_and(|value| !(1..=1024).contains(&value)) {
        return Err(KnowledgeValidationError::InvalidEmbeddingBatchSize);
    }
    Ok(())
}

pub fn validate_chunk_config(
    size: Option<i64>,
    overlap: Option<i64>,
) -> Result<(), KnowledgeValidationError> {
    let size = size.unwrap_or(512);
    let overlap = overlap.unwrap_or(64);
    if size <= 0 || overlap < 0 || overlap >= size {
        return Err(KnowledgeValidationError::InvalidChunkConfig);
    }
    Ok(())
}

pub fn validate_upload_size(decoded_bytes: usize) -> Result<(), KnowledgeValidationError> {
    if decoded_bytes > MAX_UPLOAD_BYTES {
        return Err(KnowledgeValidationError::UploadTooLarge);
    }
    Ok(())
}

pub fn validate_source(
    input: &CreateSourceInput,
    allow_local_dir: bool,
) -> Result<(), KnowledgeValidationError> {
    match input.source_type.as_str() {
        "upload" => Ok(()),
        "local_dir"
            if allow_local_dir
                && input
                    .source_path
                    .as_deref()
                    .is_some_and(|path| !path.trim().is_empty()) =>
        {
            Ok(())
        }
        "git"
            if input
                .source_url
                .as_deref()
                .is_some_and(|url| url.starts_with("https://")) =>
        {
            Ok(())
        }
        "url" => {
            let url = input
                .source_url
                .as_deref()
                .ok_or(KnowledgeValidationError::InvalidSource)?;
            let host = url
                .strip_prefix("https://")
                .and_then(|rest| rest.split(['/', ':']).next())
                .ok_or(KnowledgeValidationError::UnsafeSourceUrl)?;
            if host.eq_ignore_ascii_case("localhost")
                || host.starts_with("127.")
                || host.starts_with("10.")
                || host.starts_with("192.168.")
                || host.starts_with("169.254.")
            {
                return Err(KnowledgeValidationError::UnsafeSourceUrl);
            }
            Ok(())
        }
        _ => Err(KnowledgeValidationError::InvalidSource),
    }
}

#[async_trait::async_trait]
pub trait KnowledgeRepository: Send + Sync {
    async fn list_kbs(&self) -> Result<Vec<KbKnowledgeBase>, RepositoryError>;
    async fn get_kb(&self, kb_id: &str) -> Result<Option<KbKnowledgeBase>, RepositoryError>;
    async fn create_kb(&self, input: CreateKbInput) -> Result<KbKnowledgeBase, RepositoryError>;
    async fn update_kb(
        &self,
        kb_id: &str,
        input: UpdateKbInput,
    ) -> Result<KbKnowledgeBase, RepositoryError>;
    async fn delete_kb(&self, kb_id: &str) -> Result<(), RepositoryError>;
    async fn list_documents(&self, kb_id: &str) -> Result<Vec<KbDocument>, RepositoryError>;
    async fn get_document(
        &self,
        kb_id: &str,
        doc_id: &str,
    ) -> Result<Option<KbDocument>, RepositoryError>;
    async fn upsert_document_pending(
        &self,
        input: UpsertDocumentInput,
    ) -> Result<KbDocument, RepositoryError>;
    async fn replace_document_ready(
        &self,
        document_id: &str,
        parsed: ParsedDocument,
        chunks: Vec<NewKbChunk>,
    ) -> Result<KbDocument, RepositoryError>;
    async fn mark_document_failed(
        &self,
        doc_id: &str,
        error: String,
    ) -> Result<(), RepositoryError>;
    async fn mark_document_deleted(&self, doc_id: &str) -> Result<(), RepositoryError>;
    async fn list_ready_chunks(
        &self,
        kb_id: &str,
        document_id: Option<&str>,
    ) -> Result<Vec<KbChunk>, RepositoryError>;
    async fn get_chunk(
        &self,
        kb_id: &str,
        chunk_id: &str,
    ) -> Result<Option<KbChunk>, RepositoryError>;
    async fn create_source(
        &self,
        kb_id: &str,
        input: CreateSourceInput,
    ) -> Result<KbSource, RepositoryError>;
    async fn list_sources(&self, kb_id: &str) -> Result<Vec<KbSource>, RepositoryError>;
    async fn delete_source(&self, source_id: &str) -> Result<(), RepositoryError>;
    async fn create_task(&self, input: CreateTaskInput) -> Result<KbTask, RepositoryError>;
    async fn list_tasks(&self, kb_id: &str) -> Result<Vec<KbTask>, RepositoryError>;
    async fn mark_task_running(&self, task_id: &str) -> Result<(), RepositoryError>;
    async fn mark_task_succeeded(
        &self,
        task_id: &str,
        completed_at: String,
    ) -> Result<(), RepositoryError>;
    async fn mark_task_failed(&self, task_id: &str, error: String) -> Result<(), RepositoryError>;
    async fn recompute_stats(&self, kb_id: &str) -> Result<KbStats, RepositoryError>;
    async fn service_stats(&self) -> Result<KnowledgeServiceStats, RepositoryError>;
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeServiceStats {
    pub knowledge_bases: i64,
    pub ready_documents: i64,
    pub pending_tasks: i64,
    pub failed_tasks: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validation_rejects_invalid_config() {
        assert_eq!(
            validate_embedding_batch_size(Some(0)),
            Err(KnowledgeValidationError::InvalidEmbeddingBatchSize)
        );
        assert_eq!(
            validate_chunk_config(Some(64), Some(64)),
            Err(KnowledgeValidationError::InvalidChunkConfig)
        );
        assert_eq!(
            validate_upload_size(MAX_UPLOAD_BYTES + 1),
            Err(KnowledgeValidationError::UploadTooLarge)
        );
    }
}
