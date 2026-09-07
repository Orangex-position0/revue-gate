//! Tauri management commands for local knowledge bases.

use crate::domain::channel::ChannelRepository;
use crate::domain::knowledge::*;
use crate::infrastructure::providers::openai::OpenAiCompatibleEmbeddingClient;
use crate::infrastructure::sqlite::channel::SqliteChannelRepository;
use crate::infrastructure::sqlite::knowledge::SqliteKnowledgeRepository;
use crate::usecases::knowledge::{
    KnowledgeAdminUsecase, KnowledgeIngestionUsecase, LinearVectorBackend, QueryEmbedding,
    retrieval::KnowledgeRetrievalUsecase,
};
use tauri::State;

fn usecase(
    repo: &State<'_, SqliteKnowledgeRepository>,
) -> KnowledgeAdminUsecase<SqliteKnowledgeRepository> {
    KnowledgeAdminUsecase::new(std::sync::Arc::new((*repo.inner()).clone()))
}

#[tauri::command]
pub async fn list_knowledge_bases(
    repo: State<'_, SqliteKnowledgeRepository>,
) -> Result<Vec<KbKnowledgeBase>, String> {
    usecase(&repo).list_kbs().await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn get_knowledge_base(
    repo: State<'_, SqliteKnowledgeRepository>,
    id: String,
) -> Result<Option<KbKnowledgeBase>, String> {
    usecase(&repo).get_kb(&id).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn create_knowledge_base(
    repo: State<'_, SqliteKnowledgeRepository>,
    input: CreateKbInput,
) -> Result<KbKnowledgeBase, String> {
    usecase(&repo)
        .create_kb(input)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn update_knowledge_base(
    repo: State<'_, SqliteKnowledgeRepository>,
    id: String,
    input: UpdateKbInput,
) -> Result<KbKnowledgeBase, String> {
    usecase(&repo)
        .update_kb(&id, input)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn delete_knowledge_base(
    repo: State<'_, SqliteKnowledgeRepository>,
    id: String,
) -> Result<(), String> {
    usecase(&repo)
        .delete_kb(&id)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn upload_knowledge_document(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
    input: UploadDocumentInput,
) -> Result<KbDocument, String> {
    KnowledgeIngestionUsecase::new(std::sync::Arc::new((*repo.inner()).clone()))
        .upload_document(&kb_id, input)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn list_knowledge_documents(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
) -> Result<Vec<KbDocument>, String> {
    repo.list_documents(&kb_id).await.map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn get_knowledge_base_index_status(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
) -> Result<KbIndexMeta, String> {
    repo.refresh_index_summary(&kb_id)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn rebuild_knowledge_fts_index(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
) -> Result<KbIndexMeta, String> {
    repo.rebuild_fts_for_kb(&kb_id)
        .await
        .map_err(|e| e.to_string())?;
    repo.refresh_index_summary(&kb_id)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn create_knowledge_source(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
    input: CreateSourceInput,
) -> Result<KbSource, String> {
    usecase(&repo)
        .create_source(&kb_id, input, true)
        .await
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn list_knowledge_sources(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
) -> Result<Vec<KbSource>, String> {
    repo.list_sources(&kb_id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_knowledge(
    repo: State<'_, SqliteKnowledgeRepository>,
    channels: State<'_, SqliteChannelRepository>,
    input: KnowledgeSearchInput,
) -> Result<KnowledgeSearchResponse, String> {
    let repo = std::sync::Arc::new((*repo.inner()).clone());
    let query_embedding = std::sync::Arc::new(DynamicQueryEmbedding {
        repo: repo.clone(),
        channels: std::sync::Arc::new((*channels.inner()).clone()),
    });
    let index_reader = std::sync::Arc::new(LinearVectorBackend::new(repo.clone()));
    KnowledgeRetrievalUsecase::new(query_embedding, index_reader, repo)
        .search(input)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_knowledge_conversation_messages(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
    conversation_id: String,
    limit: Option<usize>,
) -> Result<Vec<ConversationMessage>, String> {
    repo.list_conversation_messages(&kb_id, &conversation_id, limit.unwrap_or(50))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn clear_knowledge_conversation(
    repo: State<'_, SqliteKnowledgeRepository>,
    kb_id: String,
    conversation_id: String,
) -> Result<(), String> {
    repo.clear_conversation(&kb_id, &conversation_id)
        .await
        .map_err(|e| e.to_string())
}

struct DynamicQueryEmbedding {
    repo: std::sync::Arc<SqliteKnowledgeRepository>,
    channels: std::sync::Arc<SqliteChannelRepository>,
}

#[async_trait::async_trait]
impl crate::usecases::knowledge::retrieval::KnowledgeQueryEmbedding for DynamicQueryEmbedding {
    async fn embed_query(
        &self,
        kb_id: &str,
        query: &str,
    ) -> Result<QueryEmbedding, EmbeddingError> {
        let kb = self
            .repo
            .get_kb(kb_id)
            .await?
            .ok_or(crate::domain::error::RepositoryError::NotFound)?;
        let model = kb
            .embedding_model
            .as_deref()
            .ok_or(EmbeddingError::ConfigUnavailable)?;
        let channels = self.channels.list().await?;
        let channel = if let Some(channel_id) = kb.embedding_channel_id.as_deref() {
            let id: uuid::Uuid = channel_id
                .parse()
                .map_err(|_| EmbeddingError::ConfigUnavailable)?;
            channels
                .into_iter()
                .find(|channel| channel.id == id && channel.enabled)
        } else {
            channels
                .into_iter()
                .filter(|channel| channel.enabled)
                .find(|channel| {
                    channel.models.is_empty()
                        || channel.models.iter().any(|candidate| candidate == model)
                        || channel
                            .model_mappings
                            .iter()
                            .any(|mapping| mapping.client_model == model)
                })
        }
        .ok_or(EmbeddingError::ConfigUnavailable)?;
        let mut vectors = OpenAiCompatibleEmbeddingClient::new(channel)
            .embed(model, vec![query.to_string()])
            .await?;
        let vector = vectors.pop().ok_or(EmbeddingError::InvalidResponse)?;
        let expected = kb.embedding_dim.max(0) as usize;
        if expected > 0 && vector.len() != expected {
            return Err(EmbeddingError::DimensionMismatch {
                expected,
                actual: vector.len(),
            });
        }
        Ok(QueryEmbedding {
            kb_id: kb_id.to_string(),
            model: model.to_string(),
            vector,
        })
    }
}
