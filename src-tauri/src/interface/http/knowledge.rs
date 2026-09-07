//! HTTP management surface for the Knowledge Service Module.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde_json::{Value, json};
use std::sync::Arc;

use crate::domain::error::RepositoryError;
use crate::domain::knowledge::*;
use crate::infrastructure::providers::openai::OpenAiCompatibleEmbeddingClient;
use crate::interface::http::handlers::AppState;
use crate::usecases::knowledge::{
    EmbedChunksReport, EmbeddingUsecase, FtsRebuildReport, KnowledgeAdminUsecase,
    KnowledgeIngestionUsecase, LinearVectorBackend, QueryEmbedding, rag::KnowledgeRagUsecase,
    retrieval::KnowledgeRetrievalUsecase,
};

pub fn create_knowledge_router() -> Router<AppState> {
    Router::new()
        .route("/api/kb", get(list_kbs).post(create_kb))
        .route("/api/kb/search", post(search_knowledge_global))
        .route("/api/kb/ask", post(ask_knowledge_global))
        .route(
            "/api/kb/{kb_id}",
            get(get_kb).put(update_kb).delete(delete_kb),
        )
        .route("/api/kb/{kb_id}/search", post(search_knowledge))
        .route("/api/kb/{kb_id}/ask", post(ask_knowledge))
        .route("/api/kb/{kb_id}/stats", get(stats))
        .route("/api/kb/{kb_id}/stats/recompute", post(stats))
        .route(
            "/api/kb/{kb_id}/index",
            get(index_status).post(rebuild_index),
        )
        .route("/api/kb/{kb_id}/fts/rebuild", post(rebuild_fts))
        .route(
            "/api/kb/{kb_id}/embeddings/rebuild",
            post(rebuild_embeddings),
        )
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
async fn index_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<KbIndexMeta>, ApiError> {
    Ok(Json(repo(&state)?.refresh_index_summary(&id).await?))
}
async fn rebuild_fts(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<FtsRebuildReport>, ApiError> {
    let repository = repo(&state)?;
    repository.rebuild_fts_for_kb(&id).await?;
    let meta = repository.refresh_index_summary(&id).await?;
    Ok(Json(FtsRebuildReport {
        kb_id: id,
        indexed_chunks: meta.chunk_count as usize,
    }))
}
async fn rebuild_embeddings(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<EmbedChunksReport>, ApiError> {
    let repository = repo(&state)?;
    let client = Arc::new(embedding_client(&state, &id).await?);
    Ok(Json(
        EmbeddingUsecase::new(repository, client)
            .embed_ready_chunks(&id, None)
            .await?,
    ))
}
async fn rebuild_index(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<KbIndexMeta>, ApiError> {
    let repository = repo(&state)?;
    let client = Arc::new(embedding_client(&state, &id).await?);
    EmbeddingUsecase::new(repository.clone(), client)
        .embed_ready_chunks(&id, None)
        .await?;
    repository.rebuild_fts_for_kb(&id).await?;
    Ok(Json(repository.refresh_index_summary(&id).await?))
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

async fn search_knowledge_global(
    State(state): State<AppState>,
    Json(input): Json<KnowledgeSearchInput>,
) -> Result<Json<KnowledgeSearchResponse>, ApiError> {
    Ok(Json(search_usecase(&state).await?.search(input).await?))
}

async fn search_knowledge(
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
    Json(mut input): Json<KnowledgeSearchInput>,
) -> Result<Json<KnowledgeSearchResponse>, ApiError> {
    override_path_kb_id(&mut input.kb_id, kb_id)?;
    Ok(Json(search_usecase(&state).await?.search(input).await?))
}

async fn ask_knowledge_global(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<KnowledgeAskInput>,
) -> Result<Json<RagAnswer>, ApiError> {
    Ok(Json(ask_with_state(&state, headers, input).await?))
}

async fn ask_knowledge(
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
    headers: HeaderMap,
    Json(mut input): Json<KnowledgeAskInput>,
) -> Result<Json<RagAnswer>, ApiError> {
    override_path_kb_id(&mut input.kb_id, kb_id)?;
    Ok(Json(ask_with_state(&state, headers, input).await?))
}

async fn ask_with_state(
    state: &AppState,
    headers: HeaderMap,
    input: KnowledgeAskInput,
) -> Result<RagAnswer, ApiError> {
    let repository = repo(state)?;
    let retrieval = Arc::new(search_usecase(state).await?);
    let rag = KnowledgeRagUsecase::new(retrieval, repository, state.proxy.clone());
    rag.ask(
        input,
        RagAuthContext {
            client_kind: RagClientKind::ExternalHttp,
            bearer_token: extract_bearer(&headers),
            trace_id: headers
                .get("x-request-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
                .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
        },
    )
    .await
    .map_err(ApiError::from)
}

async fn search_usecase(
    state: &AppState,
) -> Result<
    KnowledgeRetrievalUsecase<
        DynamicQueryEmbedding,
        LinearVectorBackend<crate::infrastructure::sqlite::knowledge::SqliteKnowledgeRepository>,
        crate::infrastructure::sqlite::knowledge::SqliteKnowledgeRepository,
    >,
    ApiError,
> {
    let repository = repo(state)?;
    let query_embedding = Arc::new(DynamicQueryEmbedding {
        repo: repository.clone(),
        channels: state.channel_repo.clone(),
    });
    let index_reader = Arc::new(LinearVectorBackend::new(repository.clone()));
    Ok(KnowledgeRetrievalUsecase::new(
        query_embedding,
        index_reader,
        repository,
    ))
}

fn override_path_kb_id(target: &mut Option<String>, path_kb_id: String) -> Result<(), ApiError> {
    if target
        .as_deref()
        .is_some_and(|body_kb_id| body_kb_id != path_kb_id)
    {
        return Err(ApiError::BadRequest("path_kb_id_conflicts_with_body_kb_id"));
    }
    *target = Some(path_kb_id);
    Ok(())
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

struct DynamicQueryEmbedding {
    repo: Arc<crate::infrastructure::sqlite::knowledge::SqliteKnowledgeRepository>,
    channels: Arc<dyn crate::domain::channel::ChannelRepository>,
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
            .ok_or(RepositoryError::NotFound)?;
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

async fn embedding_client(
    state: &AppState,
    kb_id: &str,
) -> Result<OpenAiCompatibleEmbeddingClient, ApiError> {
    let kb = repo(state)?
        .get_kb(kb_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    let model = kb
        .embedding_model
        .as_deref()
        .ok_or(ApiError::Conflict("embedding_model_required"))?;
    if let Some(channel_id) = kb.embedding_channel_id.as_deref() {
        let id = channel_id
            .parse()
            .map_err(|_| ApiError::Conflict("invalid_embedding_channel_id"))?;
        let channel = state
            .channel_repo
            .find_by_id(id)
            .await?
            .ok_or(ApiError::Conflict("embedding_channel_not_found"))?;
        return Ok(OpenAiCompatibleEmbeddingClient::new(channel));
    }
    let channel = state
        .channel_repo
        .list()
        .await?
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
        .ok_or(ApiError::Conflict("embedding_channel_not_found"))?;
    Ok(OpenAiCompatibleEmbeddingClient::new(channel))
}

#[derive(Debug, thiserror::Error)]
enum ApiError {
    #[error("{0}")]
    BadRequest(&'static str),
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
    #[error("{0}")]
    Embedding(#[from] crate::domain::knowledge::EmbeddingError),
    #[error("{0}")]
    Index(#[from] crate::domain::knowledge::IndexError),
    #[error("{0}")]
    Retrieval(#[from] crate::domain::knowledge::KnowledgeRetrievalError),
    #[error("{0}")]
    Rag(#[from] crate::domain::knowledge::KnowledgeRagError),
    #[error("{0}")]
    Proxy(#[from] crate::usecases::proxy::ProxyError),
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::NotFound | Self::Repository(RepositoryError::NotFound) => StatusCode::NOT_FOUND,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::Rag(crate::domain::knowledge::KnowledgeRagError::MissingLocalApiKey) => {
                StatusCode::UNAUTHORIZED
            }
            Self::Retrieval(
                crate::domain::knowledge::KnowledgeRetrievalError::QueryRequired
                | crate::domain::knowledge::KnowledgeRetrievalError::SearchScopeRequired,
            ) => StatusCode::BAD_REQUEST,
            Self::Retrieval(crate::domain::knowledge::KnowledgeRetrievalError::Forbidden) => {
                StatusCode::FORBIDDEN
            }
            Self::Retrieval(
                crate::domain::knowledge::KnowledgeRetrievalError::KnowledgeBaseNotFound,
            ) => StatusCode::NOT_FOUND,
            Self::Retrieval(
                crate::domain::knowledge::KnowledgeRetrievalError::IndexNotReady
                | crate::domain::knowledge::KnowledgeRetrievalError::VectorIndexNotReady
                | crate::domain::knowledge::KnowledgeRetrievalError::KeywordIndexNotReady,
            ) => StatusCode::CONFLICT,
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
    "get_knowledge_base_index_status",
];
