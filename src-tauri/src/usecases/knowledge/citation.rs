use crate::domain::error::RepositoryError;
use crate::domain::knowledge::{
    Citation, KbChunk, KnowledgeRepository, KnowledgeSearchFilters, KnowledgeSearchResult,
};

use super::fusion::FusedHit;

pub async fn hydrate_search_results<R>(
    repo: &R,
    kb_id: &str,
    hits: Vec<FusedHit>,
    filters: &KnowledgeSearchFilters,
) -> Result<Vec<KnowledgeSearchResult>, RepositoryError>
where
    R: KnowledgeRepository,
{
    let mut results = Vec::new();
    for hit in hits {
        let Some(chunk) = repo.get_chunk(kb_id, &hit.chunk_id).await? else {
            continue;
        };
        if !matches_filters(&chunk, filters) {
            continue;
        }
        let document = repo.get_document(kb_id, &chunk.doc_id).await?;
        if !matches_document_filters(document.as_ref(), filters) {
            continue;
        }
        let citation = to_citation(&chunk, document.as_ref(), hit.score);
        results.push(KnowledgeSearchResult {
            kb_id: kb_id.to_string(),
            chunk_id: chunk.id.clone(),
            score: hit.score,
            vector_score: hit.vector_score,
            keyword_score: hit.keyword_score,
            vector_rank: hit.vector_rank,
            keyword_rank: hit.keyword_rank,
            snippet: snippet(&chunk.content, 360),
            citation,
            content: chunk.content,
        });
    }
    Ok(results)
}

pub fn to_citation(
    chunk: &KbChunk,
    document: Option<&crate::domain::knowledge::KbDocument>,
    score: f32,
) -> Citation {
    let source_uri = document.and_then(|document| {
        document
            .source_url
            .clone()
            .or_else(|| document.source_path.clone())
            .or_else(|| document.file_path.clone())
    });
    Citation {
        citation_id: format!("{}:{}", chunk.kb_id, chunk.id),
        kb_id: chunk.kb_id.clone(),
        doc_id: chunk.doc_id.clone(),
        chunk_id: chunk.id.clone(),
        chunk_index: chunk.chunk_index,
        document_revision: chunk.document_revision,
        filename: document.map(|document| document.filename.clone()),
        file_path: document.and_then(|document| document.file_path.clone()),
        source_id: document.and_then(|document| document.source_id.clone()),
        source_type: document.map(|document| document.source_type.clone()),
        source_uri,
        symbol_name: chunk.symbol_name.clone(),
        symbol_kind: chunk.symbol_kind.clone(),
        score,
    }
}

pub fn matches_filters(chunk: &KbChunk, filters: &KnowledgeSearchFilters) -> bool {
    filters
        .doc_id
        .as_deref()
        .is_none_or(|doc_id| chunk.doc_id == doc_id)
        && filters
            .symbol_name
            .as_deref()
            .is_none_or(|name| chunk.symbol_name.as_deref() == Some(name))
        && filters
            .symbol_kind
            .as_deref()
            .is_none_or(|kind| chunk.symbol_kind.as_deref() == Some(kind))
}

fn matches_document_filters(
    document: Option<&crate::domain::knowledge::KbDocument>,
    filters: &KnowledgeSearchFilters,
) -> bool {
    filters.source_id.as_deref().is_none_or(|source_id| {
        document.and_then(|document| document.source_id.as_deref()) == Some(source_id)
    })
}

pub fn snippet(content: &str, max_chars: usize) -> String {
    let compact = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        return compact;
    }
    let mut out = compact
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk() -> KbChunk {
        KbChunk {
            id: "chunk_1".into(),
            doc_id: "doc_1".into(),
            kb_id: "kb_1".into(),
            chunk_index: 2,
            document_revision: 3,
            content: "hello world".into(),
            token_count: 2,
            embedding: None,
            embedding_dim: 0,
            metadata: "{}".into(),
            symbol_name: Some("route_request".into()),
            symbol_kind: Some("function".into()),
            created_at: "now".into(),
        }
    }

    #[test]
    fn citation_is_chunk_level_and_keeps_source_metadata() {
        let document = crate::domain::knowledge::KbDocument {
            id: "doc_1".into(),
            kb_id: "kb_1".into(),
            source_id: Some("source_1".into()),
            filename: "README.md".into(),
            file_path: Some("docs/README.md".into()),
            file_type: "markdown".into(),
            file_size: 10,
            content_hash: "hash".into(),
            parsed_text: None,
            revision: 3,
            last_ready_at: None,
            chunk_count: 1,
            token_count: 2,
            status: crate::domain::knowledge::KbDocumentStatus::Ready,
            error_message: None,
            source_type: "local_dir".into(),
            source_url: None,
            source_path: Some("E:/docs".into()),
            doc_meta: "{}".into(),
            created_at: "now".into(),
            updated_at: "now".into(),
        };

        let citation = to_citation(&chunk(), Some(&document), 0.42);

        assert_eq!(citation.chunk_id, "chunk_1");
        assert_eq!(citation.doc_id, "doc_1");
        assert_eq!(citation.source_uri.as_deref(), Some("E:/docs"));
        assert_eq!(citation.symbol_name.as_deref(), Some("route_request"));
    }

    #[test]
    fn chunk_filters_match_symbol_and_document() {
        assert!(matches_filters(
            &chunk(),
            &KnowledgeSearchFilters {
                doc_id: Some("doc_1".into()),
                symbol_name: Some("route_request".into()),
                symbol_kind: Some("function".into()),
                source_id: None,
            }
        ));
        assert!(!matches_filters(
            &chunk(),
            &KnowledgeSearchFilters {
                doc_id: Some("other".into()),
                ..KnowledgeSearchFilters::default()
            }
        ));
    }
}
