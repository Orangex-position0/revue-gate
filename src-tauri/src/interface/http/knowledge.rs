//! HTTP management surface for the Knowledge Service Module.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::domain::error::RepositoryError;
use crate::domain::knowledge::*;
use crate::interface::http::handlers::AppState;
use crate::usecases::knowledge::{KnowledgeAdminUsecase, KnowledgeIngestionUsecase};

pub fn create_knowledge_router() -> Router<AppState> {
    Router::new()
        .route("/api/kb", get(list_kbs).post(create_kb))
        .route(
            "/api/kb/{kb_id}",
            get(get_kb).put(update_kb).delete(delete_kb),
        )
        .route("/api/kb/{kb_id}/stats", get(stats))
        .route("/api/kb/{kb_id}/stats/recompute", post(stats))
        .route(
            "/api/kb/{kb_id}/documents",
            get(list_documents).post(upload_document),
        )
        .route(
            "/api/kb/{kb_id}/documents/{doc_id}",
            get(read_document).delete(delete_document),
        )
        .route(
            "/api/kb/{kb_id}/documents/{doc_id}/reprocess",
            post(reprocess_document),
        )
        .route(
            "/api/kb/{kb_id}/sources",
            get(list_sources).post(create_source),
        )
        .route(
            "/api/kb/{kb_id}/sources/{source_id}",
            axum::routing::delete(delete_source),
        )
        .route("/api/kb/{kb_id}/tasks", get(list_tasks))
}

fn repo(
    state: &AppState,
) -> Result<Arc<crate::infrastructure::sqlite::knowledge::SqliteKnowledgeRepository>, ApiError> {
    state.knowledge_repo.clone().ok_or(ApiError::Unavailable)
}
async fn list_kbs(State(state): State<AppState>) -> Result<Json<Vec<KbKnowledgeBase>>, ApiError> {
    Ok(Json(
        KnowledgeAdminUsecase::new(repo(&state)?).list_kbs().await?,
    ))
}
async fn create_kb(
    State(state): State<AppState>,
    Json(input): Json<CreateKbInput>,
) -> Result<(StatusCode, Json<KbKnowledgeBase>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            KnowledgeAdminUsecase::new(repo(&state)?)
                .create_kb(input)
                .await?,
        ),
    ))
}
async fn get_kb(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<KbKnowledgeBase>, ApiError> {
    Ok(Json(
        KnowledgeAdminUsecase::new(repo(&state)?)
            .get_kb(&id)
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}
async fn update_kb(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UpdateKbInput>,
) -> Result<Json<KbKnowledgeBase>, ApiError> {
    Ok(Json(
        KnowledgeAdminUsecase::new(repo(&state)?)
            .update_kb(&id, input)
            .await?,
    ))
}
async fn delete_kb(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    KnowledgeAdminUsecase::new(repo(&state)?)
        .delete_kb(&id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn stats(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<KbStats>, ApiError> {
    Ok(Json(repo(&state)?.recompute_stats(&id).await?))
}
async fn list_documents(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<KbDocument>>, ApiError> {
    Ok(Json(repo(&state)?.list_documents(&id).await?))
}
async fn read_document(
    State(state): State<AppState>,
    Path((kb, doc)): Path<(String, String)>,
) -> Result<Json<KbDocument>, ApiError> {
    Ok(Json(
        repo(&state)?
            .get_document(&kb, &doc)
            .await?
            .ok_or(ApiError::NotFound)?,
    ))
}
async fn upload_document(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<UploadDocumentInput>,
) -> Result<(StatusCode, Json<KbDocument>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            KnowledgeIngestionUsecase::new(repo(&state)?)
                .upload_document(&id, input)
                .await?,
        ),
    ))
}
async fn delete_document(
    State(state): State<AppState>,
    Path((kb, doc)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let repository = repo(&state)?;
    if repository.get_document(&kb, &doc).await?.is_none() {
        return Err(ApiError::NotFound);
    }
    repository.mark_document_deleted(&doc).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn reprocess_document(
    State(state): State<AppState>,
    Path((kb, doc)): Path<(String, String)>,
) -> Result<Json<KbDocument>, ApiError> {
    let repository = repo(&state)?;
    let current = repository
        .get_document(&kb, &doc)
        .await?
        .ok_or(ApiError::NotFound)?;
    let text = current
        .parsed_text
        .ok_or(ApiError::Conflict("document_has_no_parsed_text"))?;
    let parsed = crate::usecases::knowledge::parse_document(&current.filename, text.as_bytes())?;
    let base = repository.get_kb(&kb).await?.ok_or(ApiError::NotFound)?;
    let chunks = crate::usecases::knowledge::split(
        &parsed,
        &crate::usecases::knowledge::SplitConfig {
            chunk_size: base.chunk_size as usize,
            chunk_overlap: base.chunk_overlap as usize,
        },
    )?;
    Ok(Json(
        repository
            .replace_document_ready(&doc, parsed, chunks)
            .await?,
    ))
}
async fn list_sources(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<KbSource>>, ApiError> {
    Ok(Json(repo(&state)?.list_sources(&id).await?))
}
async fn create_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<CreateSourceInput>,
) -> Result<(StatusCode, Json<KbSource>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(
            KnowledgeAdminUsecase::new(repo(&state)?)
                .create_source(&id, input, false)
                .await?,
        ),
    ))
}
async fn delete_source(
    State(state): State<AppState>,
    Path((_kb, id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    repo(&state)?.delete_source(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_tasks(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<KbTask>>, ApiError> {
    Ok(Json(repo(&state)?.list_tasks(&id).await?))
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("not_found")]
    NotFound,
    #[error("knowledge_service_unavailable")]
    Unavailable,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("{0}")]
    Knowledge(#[from] crate::usecases::knowledge::KnowledgeError),
    #[error("{0}")]
    Parse(#[from] crate::usecases::knowledge::ParseError),
    #[error("{0}")]
    Split(#[from] crate::usecases::knowledge::SplitError),
    #[error("{0}")]
    Repository(#[from] RepositoryError),
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::NotFound | Self::Repository(RepositoryError::NotFound) => StatusCode::NOT_FOUND,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Parse(_) | Self::Split(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Knowledge(
                crate::usecases::knowledge::KnowledgeError::Validation(_)
                | crate::usecases::knowledge::KnowledgeError::Parse(_)
                | crate::usecases::knowledge::KnowledgeError::Split(_)
                | crate::usecases::knowledge::KnowledgeError::InvalidBase64,
            ) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (
            status,
            Json::<Value>(json!({"error":{"message":self.to_string(),"type":"knowledge_error"}})),
        )
            .into_response()
    }
}

pub const KNOWLEDGE_MCP_TOOLS: &[&str] = &[
    "list_knowledge_bases",
    "get_knowledge_base_stats",
    "list_documents",
    "read_document",
    "upload_document",
    "sync_source",
];
