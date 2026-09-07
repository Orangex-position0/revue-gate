use std::sync::Arc;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::domain::knowledge::{
    ConversationMessage, ConversationRole, KnowledgeAnswerOptions, KnowledgeAskInput,
    KnowledgeRagError, KnowledgeRepository, KnowledgeRetrievalError, KnowledgeSearchInput,
    KnowledgeSearchResponse, RagAnswer, RagAuthContext, RagClientKind, UsageInfo,
};
use crate::usecases::proxy::{ProxyRequest, ProxySuccess};

use super::token_budget::RagPromptBuilder;

#[async_trait::async_trait]
pub trait RagRetrievalPort: Send + Sync {
    async fn search(
        &self,
        input: KnowledgeSearchInput,
    ) -> Result<KnowledgeSearchResponse, KnowledgeRetrievalError>;
}

#[async_trait::async_trait]
impl<Q, I, R> RagRetrievalPort for super::retrieval::KnowledgeRetrievalUsecase<Q, I, R>
where
    Q: super::retrieval::KnowledgeQueryEmbedding,
    I: crate::domain::knowledge::KnowledgeIndexReader,
    R: KnowledgeRepository,
{
    async fn search(
        &self,
        input: KnowledgeSearchInput,
    ) -> Result<KnowledgeSearchResponse, KnowledgeRetrievalError> {
        super::retrieval::KnowledgeRetrievalUsecase::search(self, input).await
    }
}

#[async_trait::async_trait]
pub trait RagProxyPort: Send + Sync {
    async fn execute(
        &self,
        request: ProxyRequest,
    ) -> Result<ProxySuccess, crate::usecases::proxy::ProxyError>;
}

#[async_trait::async_trait]
impl RagProxyPort for crate::usecases::proxy::ProxyRequestUsecase {
    async fn execute(
        &self,
        request: ProxyRequest,
    ) -> Result<ProxySuccess, crate::usecases::proxy::ProxyError> {
        crate::usecases::proxy::ProxyRequestUsecase::execute(self, request).await
    }
}

pub struct KnowledgeRagUsecase<T, R, P> {
    retrieval: Arc<T>,
    repo: Arc<R>,
    proxy: Arc<P>,
}

impl<T, R, P> KnowledgeRagUsecase<T, R, P>
where
    T: RagRetrievalPort,
    R: KnowledgeRepository,
    P: RagProxyPort,
{
    pub fn new(retrieval: Arc<T>, repo: Arc<R>, proxy: Arc<P>) -> Self {
        Self {
            retrieval,
            repo,
            proxy,
        }
    }

    pub async fn ask(
        &self,
        input: KnowledgeAskInput,
        auth: RagAuthContext,
    ) -> Result<RagAnswer, KnowledgeRagError> {
        if input.answer.stream {
            return Err(KnowledgeRagError::StreamingNotSupported);
        }
        if auth.client_kind == RagClientKind::ExternalHttp && auth.bearer_token.is_none() {
            return Err(KnowledgeRagError::MissingLocalApiKey);
        }

        let retrieval = self
            .retrieval
            .search(KnowledgeSearchInput {
                query: input.question.clone(),
                kb_id: input.kb_id.clone(),
                scope: input.search.scope.clone(),
                mode: input.search.mode,
                limit: input.search.limit,
                filters: input.search.filters.clone(),
                fusion: Default::default(),
                mcp_only: input.mcp_only,
            })
            .await?;
        let sources = retrieval
            .results
            .iter()
            .map(|result| result.citation.clone())
            .collect::<Vec<_>>();
        let conversation_id = input
            .conversation_id
            .clone()
            .or_else(|| input.kb_id.as_ref().map(|_| Uuid::now_v7().to_string()));

        if retrieval.results.is_empty() {
            let answer = "No relevant knowledge chunks were found for this question.".to_string();
            self.save_exchange_best_effort(
                input.kb_id.as_deref(),
                conversation_id.as_deref(),
                &input.question,
                &answer,
                &input.answer,
                &retrieval,
                &auth,
                None,
            )
            .await;
            return Ok(RagAnswer {
                answer,
                conversation_id,
                retrieval: retrieval.clone(),
                sources,
                usage: None,
                warnings: retrieval.warnings.clone(),
            });
        }

        let prompt = RagPromptBuilder::new(input.answer.context_token_budget).build(
            &input.question,
            &retrieval.results,
            &input.history,
        );
        let request = build_rag_proxy_request(&input.answer, &auth, prompt.messages);
        let success = self.proxy.execute(request).await?;
        let ProxySuccess::NonStream(response) = success else {
            return Err(KnowledgeRagError::InvalidModelResponse);
        };
        let value: Value = serde_json::from_slice(&response.body)?;
        let answer = extract_answer_text(&value).ok_or(KnowledgeRagError::InvalidModelResponse)?;
        let usage = response.usage.map(|usage| UsageInfo {
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
        });
        let mut warnings = retrieval.warnings.clone();
        warnings.extend(prompt.warnings.clone());
        self.save_exchange_best_effort(
            input.kb_id.as_deref(),
            conversation_id.as_deref(),
            &input.question,
            &answer,
            &input.answer,
            &retrieval,
            &auth,
            usage,
        )
        .await;

        Ok(RagAnswer {
            answer,
            conversation_id,
            retrieval,
            sources: prompt
                .used_results
                .iter()
                .map(|result| result.citation.clone())
                .collect(),
            usage,
            warnings,
        })
    }

    async fn save_exchange_best_effort(
        &self,
        kb_id: Option<&str>,
        conversation_id: Option<&str>,
        question: &str,
        answer: &str,
        options: &KnowledgeAnswerOptions,
        retrieval: &KnowledgeSearchResponse,
        auth: &RagAuthContext,
        usage: Option<UsageInfo>,
    ) {
        let (Some(kb_id), Some(conversation_id)) = (kb_id, conversation_id) else {
            return;
        };
        let now = chrono::Utc::now().to_rfc3339();
        let user = ConversationMessage {
            role: ConversationRole::User,
            content: question.to_string(),
            sources_json: None,
            retrieval_json: None,
            warnings_json: None,
            model: None,
            token_usage_json: None,
            caller_kind: Some(auth.client_kind.to_string()),
            trace_id: Some(auth.trace_id.clone()),
            created_at: now.clone(),
        };
        let assistant = ConversationMessage {
            role: ConversationRole::Assistant,
            content: answer.to_string(),
            sources_json: serde_json::to_string(
                &retrieval
                    .results
                    .iter()
                    .map(|result| result.citation.clone())
                    .collect::<Vec<_>>(),
            )
            .ok(),
            retrieval_json: serde_json::to_string(retrieval).ok(),
            warnings_json: serde_json::to_string(&retrieval.warnings).ok(),
            model: Some(options.model.clone()),
            token_usage_json: usage.and_then(|usage| serde_json::to_string(&usage).ok()),
            caller_kind: Some(auth.client_kind.to_string()),
            trace_id: Some(auth.trace_id.clone()),
            created_at: now,
        };
        if let Err(error) = self
            .repo
            .save_conversation_message(kb_id, conversation_id, user)
            .await
        {
            tracing::warn!(error = %error, "failed to save RAG user message");
            return;
        }
        if let Err(error) = self
            .repo
            .save_conversation_message(kb_id, conversation_id, assistant)
            .await
        {
            tracing::warn!(error = %error, "failed to save RAG assistant message");
        }
    }
}

pub fn build_rag_proxy_request(
    options: &KnowledgeAnswerOptions,
    auth: &RagAuthContext,
    messages: Vec<Value>,
) -> ProxyRequest {
    let mut body = json!({
        "model": options.model,
        "messages": messages,
        "stream": false,
        "temperature": options.temperature.unwrap_or(0.2),
    });
    if let Some(max_tokens) = options.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    ProxyRequest {
        bearer_token: auth.bearer_token.clone(),
        model: options.model.clone(),
        stream: false,
        body,
        trace_id: auth.trace_id.clone(),
    }
}

fn extract_answer_text(value: &Value) -> Option<String> {
    value
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_string)
}
