use std::sync::Arc;

use crate::domain::knowledge::{
    EmbeddingError, FusionConfig, KbIndexMeta, KbIndexStatus, KbKnowledgeBase,
    KnowledgeIndexReader, KnowledgeRepository, KnowledgeRetrievalError, KnowledgeSearchInput,
    KnowledgeSearchMode, KnowledgeSearchResponse, KnowledgeSearchScope, KnowledgeSearchWarning,
};

use super::QueryEmbedding;
use super::citation::hydrate_search_results;
use super::fusion::{rank_keyword_hits, rank_vector_hits, rrf_fuse};

const MAX_SEARCH_LIMIT: usize = 50;

#[async_trait::async_trait]
pub trait KnowledgeQueryEmbedding: Send + Sync {
    async fn embed_query(&self, kb_id: &str, query: &str)
    -> Result<QueryEmbedding, EmbeddingError>;
}

#[async_trait::async_trait]
impl<R, C> KnowledgeQueryEmbedding for super::QueryEmbeddingUsecase<R, C>
where
    R: KnowledgeRepository,
    C: crate::domain::knowledge::EmbeddingClient,
{
    async fn embed_query(
        &self,
        kb_id: &str,
        query: &str,
    ) -> Result<QueryEmbedding, EmbeddingError> {
        super::QueryEmbeddingUsecase::embed_query(self, kb_id, query).await
    }
}

pub struct KnowledgeRetrievalUsecase<Q, I, R> {
    query_embedding: Arc<Q>,
    index_reader: Arc<I>,
    repo: Arc<R>,
}

struct SearchPlan {
    actual_mode: KnowledgeSearchMode,
    warnings: Vec<KnowledgeSearchWarning>,
}

impl<Q, I, R> KnowledgeRetrievalUsecase<Q, I, R>
where
    Q: KnowledgeQueryEmbedding,
    I: KnowledgeIndexReader,
    R: KnowledgeRepository,
{
    pub fn new(query_embedding: Arc<Q>, index_reader: Arc<I>, repo: Arc<R>) -> Self {
        Self {
            query_embedding,
            index_reader,
            repo,
        }
    }

    pub async fn search(
        &self,
        mut input: KnowledgeSearchInput,
    ) -> Result<KnowledgeSearchResponse, KnowledgeRetrievalError> {
        let query = normalize_query(&input.query)?;
        input.limit = normalize_limit(input.limit);
        let kbs = self
            .resolve_search_scope(input.kb_id.as_deref(), &input.scope, input.mcp_only)
            .await?;
        let mut all_results = Vec::new();
        let mut warnings = Vec::new();
        let mut actual_mode = input.mode;

        for kb in kbs {
            let plan = self.plan_search(&kb, input.mode).await?;
            actual_mode = merge_actual_mode(actual_mode, plan.actual_mode);
            warnings.extend(plan.warnings);
            let hits = match plan.actual_mode {
                KnowledgeSearchMode::Hybrid => {
                    let embedding = self.query_embedding.embed_query(&kb.id, &query).await?;
                    let vector = self
                        .index_reader
                        .vector_search(&kb.id, &embedding.vector, input.limit)
                        .await?;
                    let keyword = self
                        .index_reader
                        .keyword_search(&kb.id, &query, input.limit)
                        .await?;
                    rrf_fuse(
                        rank_vector_hits(vector),
                        rank_keyword_hits(keyword),
                        input.limit,
                        input.fusion.rrf_k,
                    )
                }
                KnowledgeSearchMode::Vector => {
                    let embedding = self.query_embedding.embed_query(&kb.id, &query).await?;
                    self.index_reader
                        .vector_search(&kb.id, &embedding.vector, input.limit)
                        .await?
                        .into_iter()
                        .enumerate()
                        .map(|(index, hit)| super::fusion::FusedHit {
                            chunk_id: hit.chunk_id,
                            score: hit.score,
                            vector_score: Some(hit.score),
                            keyword_score: None,
                            vector_rank: Some(index + 1),
                            keyword_rank: None,
                        })
                        .collect()
                }
                KnowledgeSearchMode::Keyword => self
                    .index_reader
                    .keyword_search(&kb.id, &query, input.limit)
                    .await?
                    .into_iter()
                    .enumerate()
                    .map(|(index, hit)| super::fusion::FusedHit {
                        chunk_id: hit.chunk_id,
                        score: -hit.score,
                        vector_score: None,
                        keyword_score: Some(hit.score),
                        vector_rank: None,
                        keyword_rank: Some(index + 1),
                    })
                    .collect(),
            };
            all_results.extend(
                hydrate_search_results(self.repo.as_ref(), &kb.id, hits, &input.filters).await?,
            );
        }

        all_results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.kb_id.cmp(&right.kb_id))
                .then_with(|| left.chunk_id.cmp(&right.chunk_id))
        });
        all_results.truncate(input.limit);

        Ok(KnowledgeSearchResponse {
            query,
            requested_mode: input.mode,
            actual_mode,
            results: all_results,
            warnings,
        })
    }

    async fn resolve_search_scope(
        &self,
        kb_id: Option<&str>,
        scope: &KnowledgeSearchScope,
        mcp_only: bool,
    ) -> Result<Vec<KbKnowledgeBase>, KnowledgeRetrievalError> {
        if let Some(kb_id) = kb_id {
            let kb = self
                .repo
                .get_kb(kb_id)
                .await?
                .ok_or(KnowledgeRetrievalError::KnowledgeBaseNotFound)?;
            return allow_kb(kb, mcp_only).map(|kb| vec![kb]);
        }

        let mut kbs = self.repo.list_kbs().await?;
        if !scope.kb_ids.is_empty() {
            let mut selected = Vec::new();
            for selected_id in &scope.kb_ids {
                let kb = kbs
                    .iter()
                    .find(|kb| kb.id == *selected_id)
                    .cloned()
                    .ok_or(KnowledgeRetrievalError::KnowledgeBaseNotFound)?;
                selected.push(allow_kb(kb, mcp_only)?);
            }
            return Ok(selected);
        }

        if scope.all_enabled || mcp_only {
            kbs.retain(|kb| kb.status != 0);
            return kbs
                .into_iter()
                .map(|kb| allow_kb(kb, mcp_only))
                .collect::<Result<Vec<_>, _>>();
        }

        Err(KnowledgeRetrievalError::SearchScopeRequired)
    }

    async fn plan_search(
        &self,
        kb: &KbKnowledgeBase,
        requested: KnowledgeSearchMode,
    ) -> Result<SearchPlan, KnowledgeRetrievalError> {
        let meta = self.index_reader.index_status(&kb.id).await?;
        let vector_ready = vector_ready(&meta);
        let keyword_ready = keyword_ready(&meta);
        match requested {
            KnowledgeSearchMode::Hybrid if vector_ready && keyword_ready => Ok(SearchPlan {
                actual_mode: KnowledgeSearchMode::Hybrid,
                warnings: Vec::new(),
            }),
            KnowledgeSearchMode::Hybrid if keyword_ready => Ok(SearchPlan {
                actual_mode: KnowledgeSearchMode::Keyword,
                warnings: vec![KnowledgeSearchWarning::new(
                    "hybrid_degraded_to_keyword",
                    "vector index is not ready; keyword search was used",
                    Some(kb.id.clone()),
                )],
            }),
            KnowledgeSearchMode::Hybrid if vector_ready => Ok(SearchPlan {
                actual_mode: KnowledgeSearchMode::Vector,
                warnings: vec![KnowledgeSearchWarning::new(
                    "hybrid_degraded_to_vector",
                    "keyword index is not ready; vector search was used",
                    Some(kb.id.clone()),
                )],
            }),
            KnowledgeSearchMode::Hybrid => Err(KnowledgeRetrievalError::IndexNotReady),
            KnowledgeSearchMode::Vector if vector_ready => Ok(SearchPlan {
                actual_mode: KnowledgeSearchMode::Vector,
                warnings: Vec::new(),
            }),
            KnowledgeSearchMode::Vector => Err(KnowledgeRetrievalError::VectorIndexNotReady),
            KnowledgeSearchMode::Keyword if keyword_ready => Ok(SearchPlan {
                actual_mode: KnowledgeSearchMode::Keyword,
                warnings: Vec::new(),
            }),
            KnowledgeSearchMode::Keyword => Err(KnowledgeRetrievalError::KeywordIndexNotReady),
        }
    }
}

fn normalize_query(query: &str) -> Result<String, KnowledgeRetrievalError> {
    let query = query.trim();
    if query.is_empty() {
        return Err(KnowledgeRetrievalError::QueryRequired);
    }
    Ok(query.to_string())
}

fn normalize_limit(limit: usize) -> usize {
    limit.clamp(1, MAX_SEARCH_LIMIT)
}

fn allow_kb(
    kb: KbKnowledgeBase,
    mcp_only: bool,
) -> Result<KbKnowledgeBase, KnowledgeRetrievalError> {
    if mcp_only && kb.mcp_enabled == 0 {
        return Err(KnowledgeRetrievalError::Forbidden);
    }
    Ok(kb)
}

fn vector_ready(meta: &KbIndexMeta) -> bool {
    meta.embedded_count > 0
        && matches!(
            meta.status,
            KbIndexStatus::Ready | KbIndexStatus::NeedsFtsRebuild
        )
}

fn keyword_ready(meta: &KbIndexMeta) -> bool {
    meta.chunk_count > 0 && meta.fts_status == KbIndexStatus::Ready.to_string()
}

fn merge_actual_mode(left: KnowledgeSearchMode, right: KnowledgeSearchMode) -> KnowledgeSearchMode {
    if left == right {
        left
    } else {
        KnowledgeSearchMode::Hybrid
    }
}

#[allow(dead_code)]
fn _default_fusion() -> FusionConfig {
    FusionConfig::default()
}
