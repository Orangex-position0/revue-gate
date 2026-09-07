use serde::{Deserialize, Serialize};

use crate::domain::knowledge::{
    Citation, KnowledgeAnswerOptions, KnowledgeAskInput, KnowledgeAskSearchOptions,
    KnowledgeIndexReader, KnowledgeRagError, KnowledgeRepository, KnowledgeRetrievalError,
    KnowledgeSearchFilters, KnowledgeSearchInput, KnowledgeSearchMode, KnowledgeSearchResponse,
    KnowledgeSearchScope, RagAnswer, RagAuthContext, RagClientKind,
};
use crate::usecases::knowledge::citation::to_citation;
use crate::usecases::knowledge::rag::{KnowledgeRagUsecase, RagProxyPort, RagRetrievalPort};
use crate::usecases::knowledge::retrieval::{KnowledgeQueryEmbedding, KnowledgeRetrievalUsecase};

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("invalid_params: {0}")]
    InvalidParams(String),
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("not_found: {0}")]
    NotFound(String),
    #[error("unavailable: {0}")]
    Unavailable(String),
    #[error("internal: {0}")]
    Internal(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpKnowledgeBaseSummary {
    pub kb_id: String,
    pub name: String,
    pub description: Option<String>,
    pub document_count: i64,
    pub chunk_count: i64,
    pub index_status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpSearchKnowledgeInput {
    pub kb_id: Option<String>,
    pub query: String,
    #[serde(default)]
    pub scope: KnowledgeSearchScope,
    #[serde(default)]
    pub mode: KnowledgeSearchMode,
    #[serde(default = "crate::domain::knowledge::default_search_limit")]
    pub limit: usize,
    #[serde(default)]
    pub filters: KnowledgeSearchFilters,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAskKnowledgeInput {
    pub kb_id: Option<String>,
    pub question: String,
    pub conversation_id: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub search: KnowledgeAskSearchOptions,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpReadKnowledgeChunkInput {
    pub kb_id: String,
    pub chunk_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpKnowledgeChunkContent {
    pub citation: Citation,
    pub content: String,
}

pub async fn list_knowledge_bases_tool<R>(
    repo: &R,
) -> Result<Vec<McpKnowledgeBaseSummary>, McpError>
where
    R: KnowledgeRepository,
{
    Ok(repo
        .list_kbs()
        .await?
        .into_iter()
        .filter(|kb| kb.mcp_enabled != 0)
        .map(|kb| McpKnowledgeBaseSummary {
            kb_id: kb.id,
            name: kb.name,
            description: kb.description,
            document_count: kb.doc_count,
            chunk_count: kb.chunk_count,
            index_status: kb.index_status.to_string(),
        })
        .collect())
}

pub async fn get_knowledge_base_index_status_tool<I>(
    index_reader: &I,
    kb_id: &str,
) -> Result<crate::domain::knowledge::KbIndexMeta, McpError>
where
    I: KnowledgeIndexReader,
{
    Ok(index_reader.index_status(kb_id).await?)
}

pub async fn search_knowledge_base_tool<Q, I, R>(
    retrieval: &KnowledgeRetrievalUsecase<Q, I, R>,
    input: McpSearchKnowledgeInput,
) -> Result<KnowledgeSearchResponse, McpError>
where
    Q: KnowledgeQueryEmbedding,
    I: KnowledgeIndexReader,
    R: KnowledgeRepository,
{
    Ok(retrieval
        .search(KnowledgeSearchInput {
            query: input.query,
            kb_id: input.kb_id,
            scope: input.scope,
            mode: input.mode,
            limit: input.limit,
            filters: input.filters,
            fusion: Default::default(),
            mcp_only: true,
        })
        .await?)
}

pub async fn ask_knowledge_base_tool<T, R, P>(
    rag: &KnowledgeRagUsecase<T, R, P>,
    input: McpAskKnowledgeInput,
) -> Result<RagAnswer, McpError>
where
    T: RagRetrievalPort,
    R: KnowledgeRepository,
    P: RagProxyPort,
{
    if input.stream {
        return Err(McpError::InvalidParams(
            "ask_knowledge_base does not support streaming".into(),
        ));
    }
    Ok(rag
        .ask(
            KnowledgeAskInput {
                question: input.question,
                kb_id: input.kb_id,
                conversation_id: input.conversation_id,
                search: input.search,
                answer: KnowledgeAnswerOptions {
                    model: input
                        .model
                        .unwrap_or_else(crate::domain::knowledge::default_rag_model),
                    stream: false,
                    temperature: Some(0.2),
                    max_output_tokens: None,
                    context_token_budget: 6_000,
                },
                history: Vec::new(),
                mcp_only: true,
            },
            RagAuthContext {
                client_kind: RagClientKind::Mcp,
                bearer_token: None,
                trace_id: uuid::Uuid::now_v7().to_string(),
            },
        )
        .await?)
}

pub async fn read_knowledge_chunk_tool<R>(
    repo: &R,
    input: McpReadKnowledgeChunkInput,
) -> Result<McpKnowledgeChunkContent, McpError>
where
    R: KnowledgeRepository,
{
    let kb = repo
        .get_kb(&input.kb_id)
        .await?
        .ok_or_else(|| McpError::NotFound("knowledge base not found".into()))?;
    if kb.mcp_enabled == 0 {
        return Err(McpError::Forbidden(
            "knowledge base is not MCP-enabled".into(),
        ));
    }
    let chunk = repo
        .get_chunk(&input.kb_id, &input.chunk_id)
        .await?
        .ok_or_else(|| McpError::NotFound("knowledge chunk not found".into()))?;
    let document = repo.get_document(&input.kb_id, &chunk.doc_id).await?;
    Ok(McpKnowledgeChunkContent {
        citation: to_citation(&chunk, document.as_ref(), 0.0),
        content: chunk.content,
    })
}

impl From<KnowledgeRetrievalError> for McpError {
    fn from(error: KnowledgeRetrievalError) -> Self {
        match error {
            KnowledgeRetrievalError::Forbidden => {
                McpError::Forbidden("knowledge base is not MCP-enabled".into())
            }
            KnowledgeRetrievalError::KnowledgeBaseNotFound => {
                McpError::NotFound("knowledge base not found".into())
            }
            KnowledgeRetrievalError::IndexNotReady
            | KnowledgeRetrievalError::VectorIndexNotReady
            | KnowledgeRetrievalError::KeywordIndexNotReady => {
                McpError::Unavailable("knowledge index is not ready".into())
            }
            KnowledgeRetrievalError::QueryRequired
            | KnowledgeRetrievalError::SearchScopeRequired => {
                McpError::InvalidParams(error.to_string())
            }
            other => McpError::Internal(other.to_string()),
        }
    }
}

impl From<KnowledgeRagError> for McpError {
    fn from(error: KnowledgeRagError) -> Self {
        match error {
            KnowledgeRagError::StreamingNotSupported => McpError::InvalidParams(error.to_string()),
            KnowledgeRagError::Retrieval(error) => McpError::from(error),
            other => McpError::Internal(other.to_string()),
        }
    }
}

impl From<crate::domain::error::RepositoryError> for McpError {
    fn from(error: crate::domain::error::RepositoryError) -> Self {
        match error {
            crate::domain::error::RepositoryError::NotFound => {
                McpError::NotFound("not found".into())
            }
            other => McpError::Internal(other.to_string()),
        }
    }
}

impl From<crate::domain::knowledge::IndexError> for McpError {
    fn from(error: crate::domain::knowledge::IndexError) -> Self {
        McpError::Internal(error.to_string())
    }
}
