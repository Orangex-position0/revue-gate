use crate::domain::knowledge::{
    ConversationMessage, KnowledgeSearchResult, KnowledgeSearchWarning,
};

#[derive(Debug, Clone)]
pub struct RagPromptPlan {
    pub messages: Vec<serde_json::Value>,
    pub context: String,
    pub used_results: Vec<KnowledgeSearchResult>,
    pub warnings: Vec<KnowledgeSearchWarning>,
    pub estimated_prompt_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct RagPromptBuilder {
    prompt_budget: usize,
}

impl RagPromptBuilder {
    pub fn new(prompt_budget: usize) -> Self {
        Self { prompt_budget }
    }

    pub fn build(
        &self,
        question: &str,
        results: &[KnowledgeSearchResult],
        history: &[ConversationMessage],
    ) -> RagPromptPlan {
        let mut context = String::new();
        let mut used_results = Vec::new();
        let mut estimated = estimate_tokens(question) + 220;
        for (index, result) in results.iter().enumerate() {
            let block = format!(
                "[{}] file={} chunk={} score={:.4}\n{}\n\n",
                index + 1,
                result.citation.filename.as_deref().unwrap_or("unknown"),
                result.chunk_id,
                result.score,
                result.content
            );
            let block_tokens = estimate_tokens(&block);
            if estimated + block_tokens > self.prompt_budget {
                break;
            }
            estimated += block_tokens;
            context.push_str(&block);
            used_results.push(result.clone());
        }

        let remaining = self.prompt_budget.saturating_sub(estimated);
        let packed_history = pack_recent_history(history, remaining);
        estimated += packed_history
            .iter()
            .map(|message| estimate_tokens(&message.content))
            .sum::<usize>();

        let mut warnings = Vec::new();
        if used_results.len() < results.len() {
            warnings.push(KnowledgeSearchWarning::new(
                "rag_context_truncated",
                "some retrieved chunks were omitted because of the token budget",
                None,
            ));
        }

        let system = "You answer using only the provided knowledge context. Cite sources with bracket numbers like [1]. If the context is insufficient, say what is missing.";
        let mut messages = vec![serde_json::json!({
            "role": "system",
            "content": system,
        })];
        for message in packed_history {
            messages.push(serde_json::json!({
                "role": match message.role {
                    crate::domain::knowledge::ConversationRole::User => "user",
                    crate::domain::knowledge::ConversationRole::Assistant => "assistant",
                },
                "content": message.content,
            }));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": format!("Knowledge context:\n{}\nQuestion:\n{}", context, question),
        }));

        RagPromptPlan {
            messages,
            context,
            used_results,
            warnings,
            estimated_prompt_tokens: estimated,
        }
    }
}

pub fn pack_recent_history(
    history: &[ConversationMessage],
    token_budget: usize,
) -> Vec<ConversationMessage> {
    let mut packed = Vec::new();
    let mut used = 0usize;
    for message in history.iter().rev() {
        let tokens = estimate_tokens(&message.content);
        if used + tokens > token_budget {
            break;
        }
        used += tokens;
        packed.push(message.clone());
    }
    packed.reverse();
    packed
}

pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::knowledge::{Citation, ConversationRole, KnowledgeSearchResult};

    fn result(id: &str, snippet: &str) -> KnowledgeSearchResult {
        KnowledgeSearchResult {
            kb_id: "kb".into(),
            chunk_id: id.into(),
            score: 1.0,
            vector_score: Some(1.0),
            keyword_score: None,
            vector_rank: Some(1),
            keyword_rank: None,
            snippet: snippet.into(),
            citation: Citation {
                citation_id: format!("kb:{id}"),
                kb_id: "kb".into(),
                doc_id: "doc".into(),
                chunk_id: id.into(),
                chunk_index: 0,
                document_revision: 1,
                filename: Some("a.md".into()),
                file_path: None,
                source_id: None,
                source_type: None,
                source_uri: None,
                symbol_name: None,
                symbol_kind: None,
                score: 1.0,
            },
            content: snippet.into(),
        }
    }

    #[test]
    fn token_packing_prioritizes_chunks_before_history() {
        let history = vec![
            ConversationMessage {
                role: ConversationRole::User,
                content: "long ".repeat(100),
                sources_json: None,
                retrieval_json: None,
                warnings_json: None,
                model: None,
                token_usage_json: None,
                caller_kind: None,
                trace_id: None,
                created_at: "now".into(),
            },
            ConversationMessage {
                role: ConversationRole::Assistant,
                content: "recent".into(),
                sources_json: None,
                retrieval_json: None,
                warnings_json: None,
                model: None,
                token_usage_json: None,
                caller_kind: None,
                trace_id: None,
                created_at: "now".into(),
            },
        ];
        let plan = RagPromptBuilder::new(260).build(
            "How does routing work?",
            &[
                result("chunk_1", "routing context"),
                result("chunk_2", &"x".repeat(800)),
            ],
            &history,
        );

        assert_eq!(plan.used_results.len(), 1);
        assert!(plan.context.contains("routing context"));
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.code == "rag_context_truncated")
        );
    }
}
