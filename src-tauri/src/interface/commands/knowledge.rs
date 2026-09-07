//! Tauri management commands for local knowledge bases.

use crate::domain::knowledge::*;
use crate::infrastructure::sqlite::knowledge::SqliteKnowledgeRepository;
use crate::usecases::knowledge::{KnowledgeAdminUsecase, KnowledgeIngestionUsecase};
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
