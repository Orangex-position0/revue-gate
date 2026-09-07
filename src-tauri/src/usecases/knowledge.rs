//! Knowledge ingestion, parsing and splitting use cases.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use base64::Engine;
use serde_json::{Value, json};

use crate::domain::error::RepositoryError;
use crate::domain::knowledge::{
    CreateKbInput, CreateSourceInput, CreateTaskInput, KbDocument, KbKnowledgeBase, KbSource,
    KbStats, KbTaskType, KnowledgeRepository, KnowledgeValidationError, NewKbChunk, ParsedDocument,
    UpdateKbInput, UploadDocumentInput, UpsertDocumentInput, validate_chunk_config,
    validate_embedding_batch_size, validate_source, validate_upload_size,
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
}
