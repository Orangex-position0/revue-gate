//! Knowledge ingestion, parsing and splitting use cases.

pub mod citation;
pub mod fusion;
pub mod rag;
pub mod retrieval;
pub mod token_budget;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::domain::error::RepositoryError;
use crate::domain::knowledge::{
    ChunkEmbeddingUpdate, CreateKbInput, CreateSourceInput, CreateTaskInput, EmbeddingClient,
    EmbeddingError, IndexError, KbDocument, KbIndexMeta, KbKnowledgeBase, KbSource, KbStats,
    KbTaskType, KeywordHit, KnowledgeIndexReader, KnowledgeIndexRepository, KnowledgeRepository,
    KnowledgeValidationError, NewKbChunk, ParsedDocument, UpdateKbInput, UploadDocumentInput,
    UpsertDocumentInput, VectorHit, validate_chunk_config, validate_embedding_batch_size,
    validate_source, validate_upload_size,
};

#[derive(Debug, thiserror::Error)]
pub enum KnowledgeError {
    #[error("{0}")]
    Validation(#[from] KnowledgeValidationError),
    #[error("{0}")]
    Parse(#[from] ParseError),
    #[error("{0}")]
    Split(#[from] SplitError),
    #[error("invalid_base64")]
    InvalidBase64,
    #[error("serialization_error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0}")]
    Repository(#[from] RepositoryError),
    #[error("{0}")]
    Embedding(#[from] EmbeddingError),
    #[error("{0}")]
    Index(#[from] IndexError),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("unsupported_file_type:{0}")]
    UnsupportedFileType(String),
    #[error("invalid_text_encoding")]
    InvalidTextEncoding,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SplitError {
    #[error("invalid_chunk_config")]
    InvalidConfig,
}

#[derive(Debug, Clone, Copy)]
pub struct SplitConfig {
    pub chunk_size: usize,
    pub chunk_overlap: usize,
}

impl Default for SplitConfig {
    fn default() -> Self {
        Self {
            chunk_size: 512,
            chunk_overlap: 64,
        }
    }
}

pub fn parse_document(filename: &str, content: &[u8]) -> Result<ParsedDocument, ParseError> {
    let extension = filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    let decoded = std::str::from_utf8(content).map_err(|_| ParseError::InvalidTextEncoding)?;
    let (text, file_type, language) = match extension.as_str() {
        "txt" => (decoded.to_string(), "text", None),
        "md" | "markdown" => (decoded.to_string(), "markdown", None),
        "html" | "htm" => (extract_html_text(decoded), "html", None),
        "json" | "yaml" | "yml" | "toml" => (decoded.to_string(), "text", None),
        "rs" => (decoded.to_string(), "code", Some("rust")),
        "ts" | "tsx" => (decoded.to_string(), "code", Some("typescript")),
        "js" | "jsx" => (decoded.to_string(), "code", Some("javascript")),
        "py" => (decoded.to_string(), "code", Some("python")),
        "go" => (decoded.to_string(), "code", Some("go")),
        "java" => (decoded.to_string(), "code", Some("java")),
        _ => return Err(ParseError::UnsupportedFileType(extension)),
    };
    Ok(ParsedDocument {
        text,
        file_type: file_type.to_string(),
        language: language.map(str::to_string),
        title: None,
        metadata: json!({"parser": "kb_mvp_text"}),
    })
}

fn extract_html_text(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let name = tag
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("");
                if matches!(
                    name,
                    "br" | "p" | "div" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li"
                ) && !text.ends_with('\n')
                {
                    text.push('\n');
                }
            }
            _ if in_tag => tag.push(ch),
            _ => text.push(ch),
        }
    }
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn split(parsed: &ParsedDocument, config: &SplitConfig) -> Result<Vec<NewKbChunk>, SplitError> {
    if parsed.file_type == "markdown" {
        split_markdown(&parsed.text, config, &parsed.metadata)
    } else {
        split_text(&parsed.text, config, &parsed.metadata)
    }
}

pub fn split_text(
    text: &str,
    config: &SplitConfig,
    metadata: &Value,
) -> Result<Vec<NewKbChunk>, SplitError> {
    if config.chunk_size == 0 || config.chunk_overlap >= config.chunk_size {
        return Err(SplitError::InvalidConfig);
    }
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let chars: Vec<char> = text.chars().collect();
    let step = config.chunk_size - config.chunk_overlap;
    Ok((0..chars.len())
        .step_by(step)
        .take_while(|start| *start == 0 || chars.len() - *start > config.chunk_overlap)
        .enumerate()
        .map(|(index, start)| {
            let end = (start + config.chunk_size).min(chars.len());
            let content: String = chars[start..end].iter().collect();
            NewKbChunk {
                chunk_index: index as i64,
                token_count: content.split_whitespace().count().max(1) as i64,
                content,
                metadata: metadata.clone(),
                symbol_name: None,
                symbol_kind: None,
            }
        })
        .collect())
}

fn split_markdown(
    text: &str,
    config: &SplitConfig,
    metadata: &Value,
) -> Result<Vec<NewKbChunk>, SplitError> {
    let mut sections: Vec<(Option<String>, String)> = Vec::new();
    for line in text.lines() {
        if let Some(heading) = line
            .strip_prefix('#')
            .map(|v| v.trim_start_matches('#').trim())
            .filter(|v| !v.is_empty())
        {
            sections.push((Some(heading.to_string()), format!("{line}\n")));
        } else if let Some((_, body)) = sections.last_mut() {
            body.push_str(line);
            body.push('\n');
        } else {
            sections.push((None, format!("{line}\n")));
        }
    }
    let mut chunks = Vec::new();
    for (heading, section) in sections {
        let mut section_meta = metadata.clone();
        if let (Some(map), Some(heading)) = (section_meta.as_object_mut(), heading) {
            map.insert("heading".to_string(), Value::String(heading));
        }
        for mut chunk in split_text(section.trim_end(), config, &section_meta)? {
            chunk.chunk_index = chunks.len() as i64;
            chunks.push(chunk);
        }
    }
    Ok(chunks)
}

pub struct KnowledgeAdminUsecase<R> {
    repo: Arc<R>,
}

impl<R: KnowledgeRepository> KnowledgeAdminUsecase<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }
    pub async fn list_kbs(&self) -> Result<Vec<KbKnowledgeBase>, KnowledgeError> {
        Ok(self.repo.list_kbs().await?)
    }
    pub async fn get_kb(&self, id: &str) -> Result<Option<KbKnowledgeBase>, KnowledgeError> {
        Ok(self.repo.get_kb(id).await?)
    }
    pub async fn create_kb(&self, input: CreateKbInput) -> Result<KbKnowledgeBase, KnowledgeError> {
        if input.name.trim().is_empty() {
            return Err(KnowledgeValidationError::NameRequired.into());
        }
        validate_embedding_batch_size(input.embedding_batch_size)?;
        Ok(self.repo.create_kb(input).await?)
    }
    pub async fn update_kb(
        &self,
        id: &str,
        input: UpdateKbInput,
    ) -> Result<KbKnowledgeBase, KnowledgeError> {
        validate_embedding_batch_size(input.embedding_batch_size)?;
        validate_chunk_config(input.chunk_size, input.chunk_overlap)?;
        Ok(self.repo.update_kb(id, input).await?)
    }
    pub async fn delete_kb(&self, id: &str) -> Result<(), KnowledgeError> {
        Ok(self.repo.delete_kb(id).await?)
    }
    pub async fn stats(&self, id: &str) -> Result<KbStats, KnowledgeError> {
        Ok(self.repo.recompute_stats(id).await?)
    }
    pub async fn create_source(
        &self,
        id: &str,
        input: CreateSourceInput,
        allow_local_dir: bool,
    ) -> Result<KbSource, KnowledgeError> {
        validate_source(&input, allow_local_dir)?;
        Ok(self.repo.create_source(id, input).await?)
    }
}

pub struct KnowledgeIngestionUsecase<R> {
    repo: Arc<R>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbedChunksReport {
    pub kb_id: String,
    pub attempted_chunks: usize,
    pub embedded_chunks: usize,
    pub failed_chunks: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryEmbedding {
    pub kb_id: String,
    pub model: String,
    pub vector: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FtsRebuildReport {
    pub kb_id: String,
    pub indexed_chunks: usize,
}

pub fn encode_embedding(vector: &[f32]) -> Vec<u8> {
    vector
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

pub fn decode_embedding(blob: &[u8]) -> Result<Vec<f32>, IndexError> {
    if blob.len() % std::mem::size_of::<f32>() != 0 {
        return Err(IndexError::InvalidEmbeddingBlob);
    }
    Ok(blob
        .chunks_exact(std::mem::size_of::<f32>())
        .map(|bytes| f32::from_le_bytes(bytes.try_into().expect("chunk length is four")))
        .collect())
}

pub fn decode_embedding_with_dim(blob: &[u8], expected_dim: i64) -> Result<Vec<f32>, IndexError> {
    let vector = decode_embedding(blob)?;
    let expected = expected_dim.max(0) as usize;
    if expected > 0 && vector.len() != expected {
        return Err(IndexError::DimensionMismatch {
            expected,
            actual: vector.len(),
        });
    }
    Ok(vector)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut a_norm = 0.0f32;
    let mut b_norm = 0.0f32;
    for (left, right) in a.iter().zip(b) {
        dot += left * right;
        a_norm += left * left;
        b_norm += right * right;
    }
    if a_norm == 0.0 || b_norm == 0.0 {
        return 0.0;
    }
    dot / (a_norm.sqrt() * b_norm.sqrt())
}

pub struct EmbeddingUsecase<R, C> {
    repo: Arc<R>,
    client: Arc<C>,
}

impl<R, C> EmbeddingUsecase<R, C>
where
    R: KnowledgeRepository + KnowledgeIndexRepository,
    C: EmbeddingClient,
{
    pub fn new(repo: Arc<R>, client: Arc<C>) -> Self {
        Self { repo, client }
    }

    pub async fn embed_ready_chunks(
        &self,
        kb_id: &str,
        document_id: Option<&str>,
    ) -> Result<EmbedChunksReport, EmbeddingError> {
        let kb = self
            .repo
            .get_kb(kb_id)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        let model = kb
            .embedding_model
            .as_deref()
            .ok_or(EmbeddingError::ConfigUnavailable)?;
        let batch_size = kb.embedding_batch_size.clamp(1, 1024) as usize;
        let mut chunks = match document_id {
            Some(id) => self
                .repo
                .list_ready_chunks(kb_id, Some(id))
                .await?
                .into_iter()
                .filter(|chunk| chunk.embedding.is_none())
                .collect::<Vec<_>>(),
            None => {
                self.repo
                    .list_chunks_missing_embedding(kb_id, batch_size)
                    .await?
            }
        };
        let mut attempted_chunks = 0usize;
        let mut embedded_chunks = 0usize;
        let mut failed_chunks = 0usize;

        loop {
            if chunks.is_empty() {
                break;
            }
            attempted_chunks += chunks.len();
            for batch in chunks.chunks(batch_size) {
                let inputs = batch
                    .iter()
                    .map(|chunk| chunk.content.clone())
                    .collect::<Vec<_>>();
                match self.client.embed(model, inputs).await {
                    Ok(vectors) if vectors.len() == batch.len() => {
                        let mut updates = Vec::with_capacity(batch.len());
                        for (chunk, vector) in batch.iter().zip(vectors) {
                            if kb.embedding_dim > 0 && vector.len() != kb.embedding_dim as usize {
                                failed_chunks += 1;
                                continue;
                            }
                            updates.push(ChunkEmbeddingUpdate {
                                chunk_id: chunk.id.clone(),
                                embedding_dim: vector.len() as i64,
                                embedding: encode_embedding(&vector),
                            });
                        }
                        embedded_chunks += updates.len();
                        self.repo.update_chunk_embeddings(updates).await?;
                    }
                    Ok(_) => failed_chunks += batch.len(),
                    Err(_) => failed_chunks += batch.len(),
                }
            }
            if document_id.is_some() {
                break;
            }
            chunks = self
                .repo
                .list_chunks_missing_embedding(kb_id, batch_size)
                .await?;
        }
        self.repo.refresh_index_summary(kb_id).await?;
        Ok(EmbedChunksReport {
            kb_id: kb_id.to_string(),
            attempted_chunks,
            embedded_chunks,
            failed_chunks,
        })
    }
}

pub struct QueryEmbeddingUsecase<R, C> {
    repo: Arc<R>,
    client: Arc<C>,
}

impl<R, C> QueryEmbeddingUsecase<R, C>
where
    R: KnowledgeRepository,
    C: EmbeddingClient,
{
    pub fn new(repo: Arc<R>, client: Arc<C>) -> Self {
        Self { repo, client }
    }

    pub async fn embed_query(
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
        let mut vectors = self.client.embed(model, vec![query.to_string()]).await?;
        let vector = vectors.pop().ok_or(EmbeddingError::InvalidResponse)?;
        let expected_dim = kb.embedding_dim.max(0) as usize;
        if expected_dim > 0 && vector.len() != expected_dim {
            return Err(EmbeddingError::DimensionMismatch {
                expected: expected_dim,
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

pub struct LinearVectorBackend<R> {
    repo: Arc<R>,
}

impl<R: KnowledgeIndexRepository> LinearVectorBackend<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }
}

#[async_trait::async_trait]
impl<R> KnowledgeIndexReader for LinearVectorBackend<R>
where
    R: KnowledgeRepository + KnowledgeIndexRepository,
{
    async fn vector_search(
        &self,
        kb_id: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorHit>, IndexError> {
        let chunks = self.repo.list_ready_chunks(kb_id, None).await?;
        let mut hits = Vec::new();
        for chunk in chunks {
            let Some(blob) = chunk.embedding.as_deref() else {
                continue;
            };
            let vector = match decode_embedding_with_dim(blob, chunk.embedding_dim) {
                Ok(vector) if vector.len() == query_embedding.len() => vector,
                _ => continue,
            };
            hits.push(VectorHit {
                chunk_id: chunk.id,
                score: cosine_similarity(query_embedding, &vector),
            });
        }
        hits.sort_by(|left, right| right.score.total_cmp(&left.score));
        hits.truncate(limit);
        Ok(hits)
    }

    async fn keyword_search(
        &self,
        kb_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KeywordHit>, IndexError> {
        Ok(self.repo.keyword_search(kb_id, query, limit).await?)
    }

    async fn index_status(&self, kb_id: &str) -> Result<KbIndexMeta, IndexError> {
        Ok(self.repo.get_index_meta(kb_id).await?)
    }
}

impl<R: KnowledgeRepository> KnowledgeIngestionUsecase<R> {
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }
    pub async fn upload_document(
        &self,
        kb_id: &str,
        input: UploadDocumentInput,
    ) -> Result<KbDocument, KnowledgeError> {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(&input.content_base64)
            .map_err(|_| KnowledgeError::InvalidBase64)?;
        validate_upload_size(bytes.len())?;
        let parsed = parse_document(&input.filename, &bytes)?;
        let kb = self
            .repo
            .get_kb(kb_id)
            .await?
            .ok_or(RepositoryError::NotFound)?;
        let chunks = split(
            &parsed,
            &SplitConfig {
                chunk_size: kb.chunk_size as usize,
                chunk_overlap: kb.chunk_overlap as usize,
            },
        )?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        let pending = self
            .repo
            .upsert_document_pending(UpsertDocumentInput {
                kb_id: kb_id.to_string(),
                source_id: None,
                filename: input.filename,
                file_path: None,
                source_type: "upload".to_string(),
                source_url: None,
                source_path: None,
                content_hash: format!("{:016x}", hasher.finish()),
                file_size: bytes.len() as i64,
                file_type: parsed.file_type.clone(),
            })
            .await?;
        let task = self
            .repo
            .create_task(CreateTaskInput {
                kb_id: kb_id.to_string(),
                source_id: None,
                doc_id: Some(pending.id.clone()),
                task_type: KbTaskType::ProcessDocument,
                payload_json: "{}".to_string(),
            })
            .await?;
        self.repo.mark_task_running(&task.id).await?;
        match self
            .repo
            .replace_document_ready(&pending.id, parsed, chunks)
            .await
        {
            Ok(document) => {
                self.repo
                    .mark_task_succeeded(&task.id, chrono::Utc::now().to_rfc3339())
                    .await?;
                Ok(document)
            }
            Err(error) => {
                let message = error.to_string();
                self.repo
                    .mark_task_failed(&task.id, message.clone())
                    .await?;
                self.repo.mark_document_failed(&pending.id, message).await?;
                Err(error.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::knowledge::{
        ChunkEmbeddingUpdate, KbIndexMeta, KbTask, KeywordHit, KnowledgeIndexReader,
        KnowledgeIndexRepository, UpdateIndexMetaInput,
    };
    use std::sync::RwLock;

    #[test]
    fn parser_accepts_mvp_formats_and_rejects_pdf() {
        assert_eq!(
            parse_document("note.md", b"# Title")
                .expect("markdown")
                .file_type,
            "markdown"
        );
        let code = parse_document("main.rs", b"fn main() {}").expect("code");
        assert_eq!(code.file_type, "code");
        assert_eq!(code.language.as_deref(), Some("rust"));
        assert_eq!(
            parse_document("page.html", b"<h1>Hello</h1><p>world</p>")
                .expect("html")
                .text,
            "Hello\nworld"
        );
        assert_eq!(
            parse_document("file.pdf", b"pdf")
                .expect_err("unsupported")
                .to_string(),
            "unsupported_file_type:pdf"
        );
    }

    #[test]
    fn splitter_applies_overlap_and_markdown_heading_metadata() {
        let chunks = split_text(
            "abcdefghij",
            &SplitConfig {
                chunk_size: 6,
                chunk_overlap: 2,
            },
            &serde_json::json!({}),
        )
        .expect("split");
        assert_eq!(
            chunks
                .iter()
                .map(|c| c.content.as_str())
                .collect::<Vec<_>>(),
            vec!["abcdef", "efghij"]
        );
        let parsed = parse_document("note.md", b"# A\nalpha\n## B\nbeta").expect("parse");
        let chunks = split(
            &parsed,
            &SplitConfig {
                chunk_size: 64,
                chunk_overlap: 8,
            },
        )
        .expect("split");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[1].metadata["heading"], "B");
    }

    #[test]
    fn embedding_codec_round_trips_little_endian_f32() {
        let vector = vec![1.0, -2.5, 0.25];
        let blob = encode_embedding(&vector);

        assert_eq!(decode_embedding(&blob).expect("decode"), vector);
    }

    #[test]
    fn embedding_codec_rejects_invalid_length_and_dim_mismatch() {
        assert!(matches!(
            decode_embedding(&[0, 1, 2]),
            Err(IndexError::InvalidEmbeddingBlob)
        ));
        assert!(matches!(
            decode_embedding_with_dim(&encode_embedding(&[1.0, 2.0]), 3),
            Err(IndexError::DimensionMismatch {
                expected: 3,
                actual: 2
            })
        ));
    }

    struct InMemoryKnowledgeIndexRepo {
        kb: RwLock<KbKnowledgeBase>,
        chunks: RwLock<Vec<crate::domain::knowledge::KbChunk>>,
    }

    impl InMemoryKnowledgeIndexRepo {
        fn new() -> Self {
            Self {
                kb: RwLock::new(KbKnowledgeBase {
                    id: "kb".into(),
                    name: "kb".into(),
                    description: None,
                    status: 1,
                    doc_count: 1,
                    chunk_count: 0,
                    total_tokens: 0,
                    embedding_model: Some("embedding-model".into()),
                    embedding_channel_id: None,
                    embedding_batch_size: 2,
                    mcp_enabled: 1,
                    chunk_size: 512,
                    chunk_overlap: 64,
                    excluded_dirs: String::new(),
                    excluded_files: String::new(),
                    included_files: String::new(),
                    embedding_dim: 0,
                    index_status: crate::domain::knowledge::KbIndexStatus::None,
                    created_at: "now".into(),
                    updated_at: "now".into(),
                }),
                chunks: RwLock::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl KnowledgeRepository for InMemoryKnowledgeIndexRepo {
        async fn list_kbs(&self) -> Result<Vec<KbKnowledgeBase>, RepositoryError> {
            panic!("unused")
        }
        async fn get_kb(&self, kb_id: &str) -> Result<Option<KbKnowledgeBase>, RepositoryError> {
            let kb = self.kb.read().expect("lock").clone();
            Ok((kb.id == kb_id).then_some(kb))
        }
        async fn create_kb(
            &self,
            _input: CreateKbInput,
        ) -> Result<KbKnowledgeBase, RepositoryError> {
            panic!("unused")
        }
        async fn update_kb(
            &self,
            _kb_id: &str,
            _input: UpdateKbInput,
        ) -> Result<KbKnowledgeBase, RepositoryError> {
            panic!("unused")
        }
        async fn delete_kb(&self, _kb_id: &str) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn list_documents(&self, _kb_id: &str) -> Result<Vec<KbDocument>, RepositoryError> {
            panic!("unused")
        }
        async fn get_document(
            &self,
            _kb_id: &str,
            _doc_id: &str,
        ) -> Result<Option<KbDocument>, RepositoryError> {
            panic!("unused")
        }
        async fn upsert_document_pending(
            &self,
            _input: UpsertDocumentInput,
        ) -> Result<KbDocument, RepositoryError> {
            panic!("unused")
        }
        async fn replace_document_ready(
            &self,
            _document_id: &str,
            _parsed: ParsedDocument,
            _chunks: Vec<NewKbChunk>,
        ) -> Result<KbDocument, RepositoryError> {
            panic!("unused")
        }
        async fn mark_document_failed(
            &self,
            _doc_id: &str,
            _error: String,
        ) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn mark_document_deleted(&self, _doc_id: &str) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn list_ready_chunks(
            &self,
            kb_id: &str,
            _document_id: Option<&str>,
        ) -> Result<Vec<crate::domain::knowledge::KbChunk>, RepositoryError> {
            Ok(self
                .chunks
                .read()
                .expect("lock")
                .iter()
                .filter(|chunk| chunk.kb_id == kb_id)
                .cloned()
                .collect())
        }
        async fn get_chunk(
            &self,
            _kb_id: &str,
            _chunk_id: &str,
        ) -> Result<Option<crate::domain::knowledge::KbChunk>, RepositoryError> {
            panic!("unused")
        }
        async fn create_source(
            &self,
            _kb_id: &str,
            _input: CreateSourceInput,
        ) -> Result<KbSource, RepositoryError> {
            panic!("unused")
        }
        async fn list_sources(&self, _kb_id: &str) -> Result<Vec<KbSource>, RepositoryError> {
            panic!("unused")
        }
        async fn delete_source(&self, _source_id: &str) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn create_task(&self, _input: CreateTaskInput) -> Result<KbTask, RepositoryError> {
            panic!("unused")
        }
        async fn list_tasks(&self, _kb_id: &str) -> Result<Vec<KbTask>, RepositoryError> {
            panic!("unused")
        }
        async fn mark_task_running(&self, _task_id: &str) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn mark_task_succeeded(
            &self,
            _task_id: &str,
            _completed_at: String,
        ) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn mark_task_failed(
            &self,
            _task_id: &str,
            _error: String,
        ) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn recompute_stats(&self, _kb_id: &str) -> Result<KbStats, RepositoryError> {
            panic!("unused")
        }
        async fn service_stats(
            &self,
        ) -> Result<crate::domain::knowledge::KnowledgeServiceStats, RepositoryError> {
            panic!("unused")
        }
    }

    #[async_trait::async_trait]
    impl KnowledgeIndexRepository for InMemoryKnowledgeIndexRepo {
        async fn list_chunks_missing_embedding(
            &self,
            kb_id: &str,
            limit: usize,
        ) -> Result<Vec<crate::domain::knowledge::KbChunk>, RepositoryError> {
            Ok(self
                .chunks
                .read()
                .expect("lock")
                .iter()
                .filter(|chunk| chunk.kb_id == kb_id && chunk.embedding.is_none())
                .take(limit)
                .cloned()
                .collect())
        }
        async fn update_chunk_embeddings(
            &self,
            updates: Vec<ChunkEmbeddingUpdate>,
        ) -> Result<(), RepositoryError> {
            let mut chunks = self.chunks.write().expect("lock");
            for update in updates {
                if let Some(chunk) = chunks.iter_mut().find(|chunk| chunk.id == update.chunk_id) {
                    chunk.embedding = Some(update.embedding);
                    chunk.embedding_dim = update.embedding_dim;
                }
            }
            Ok(())
        }
        async fn clear_embeddings_for_kb(&self, _kb_id: &str) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn sync_fts_rows_for_document(
            &self,
            _kb_id: &str,
            _document_id: &str,
        ) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn rebuild_fts_for_kb(&self, _kb_id: &str) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn get_index_meta(&self, _kb_id: &str) -> Result<KbIndexMeta, RepositoryError> {
            panic!("unused")
        }
        async fn update_index_meta(
            &self,
            _input: UpdateIndexMetaInput,
        ) -> Result<(), RepositoryError> {
            panic!("unused")
        }
        async fn refresh_index_summary(&self, kb_id: &str) -> Result<KbIndexMeta, RepositoryError> {
            let chunks = self.chunks.read().expect("lock");
            let chunk_count = chunks.iter().filter(|chunk| chunk.kb_id == kb_id).count() as i64;
            let embedded_count = chunks
                .iter()
                .filter(|chunk| chunk.kb_id == kb_id && chunk.embedding.is_some())
                .count() as i64;
            Ok(KbIndexMeta {
                kb_id: kb_id.into(),
                index_type: "linear".into(),
                embedding_dim: 0,
                chunk_count,
                embedded_count,
                fts_status: "none".into(),
                hnsw_status: "none".into(),
                index_path: None,
                built_at: None,
                status: crate::domain::knowledge::KbIndexStatus::None,
                error_message: None,
                updated_at: "now".into(),
            })
        }
        async fn keyword_search(
            &self,
            _kb_id: &str,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<KeywordHit>, RepositoryError> {
            Ok(Vec::new())
        }
    }

    fn chunk(id: &str, vector: Option<&[f32]>) -> crate::domain::knowledge::KbChunk {
        crate::domain::knowledge::KbChunk {
            id: id.into(),
            doc_id: "doc".into(),
            kb_id: "kb".into(),
            chunk_index: 0,
            document_revision: 1,
            content: id.into(),
            token_count: 1,
            embedding: vector.map(encode_embedding),
            embedding_dim: vector.map_or(0, |v| v.len() as i64),
            metadata: "{}".into(),
            symbol_name: None,
            symbol_kind: None,
            created_at: "now".into(),
        }
    }

    struct MockEmbeddingClient;

    #[async_trait::async_trait]
    impl EmbeddingClient for MockEmbeddingClient {
        async fn embed(
            &self,
            _model: &str,
            inputs: Vec<String>,
        ) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            Ok(inputs.into_iter().map(|_| vec![1.0, 0.0]).collect())
        }
    }

    #[tokio::test]
    async fn embedding_usecase_continues_until_all_missing_chunks_are_embedded() {
        let repo = Arc::new(InMemoryKnowledgeIndexRepo::new());
        repo.chunks.write().expect("lock").extend([
            chunk("one", None),
            chunk("two", None),
            chunk("three", None),
        ]);
        let client = Arc::new(MockEmbeddingClient);

        let report = EmbeddingUsecase::new(repo.clone(), client)
            .embed_ready_chunks("kb", None)
            .await
            .expect("embed chunks");

        assert_eq!(report.attempted_chunks, 3);
        assert_eq!(report.embedded_chunks, 3);
        assert_eq!(
            repo.chunks
                .read()
                .expect("lock")
                .iter()
                .filter(|chunk| chunk.embedding.is_some())
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn linear_vector_backend_sorts_by_cosine_similarity() {
        let repo = Arc::new(InMemoryKnowledgeIndexRepo::new());
        repo.chunks.write().expect("lock").extend([
            chunk("close", Some(&[1.0, 0.0])),
            chunk("far", Some(&[0.0, 1.0])),
            chunk("missing", None),
        ]);
        let backend = LinearVectorBackend::new(repo);

        let hits = backend
            .vector_search("kb", &[1.0, 0.0], 10)
            .await
            .expect("search");

        assert_eq!(
            hits.iter()
                .map(|hit| hit.chunk_id.as_str())
                .collect::<Vec<_>>(),
            vec!["close", "far"]
        );
    }
}
