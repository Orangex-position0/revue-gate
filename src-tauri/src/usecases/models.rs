//! Model list use case: `/v1/models` — merge and dedup models across enabled channels, excluding disabled channels.
//!
//! Data-plane contract (docs/Requirements.md): enabled channels' `models` and `model_mappings.client_model`
//! are merged, deduped while preserving order (the client picks model names from this; mapped names are also routable), disabled channels do not participate; no key-based filtering.
//! `list_models` is a pure function, keeping it consistent with `ChannelSelector`'s "models in the list must be selectable".

use std::collections::HashSet;

use crate::domain::channel::{Channel, ChannelRepository};
use crate::domain::error::RepositoryError;

/// Model list use case: read all channels and merge with dedup across enabled channels.
pub struct ListModelsUsecase;
impl ListModelsUsecase {
    pub async fn execute(
        &self,
        repo: &dyn ChannelRepository,
    ) -> Result<Vec<String>, RepositoryError> {
        let channels = repo.list().await?;
        Ok(list_models(&channels))
    }
}

/// Pure function: merge models of enabled channels with dedup (keeping first-occurrence order); disabled channels are excluded.
pub fn list_models(channels: &[Channel]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut models = Vec::new();
    for channel in channels.iter().filter(|c| c.enabled) {
        for model in &channel.models {
            if seen.insert(model.clone()) {
                models.push(model.clone());
            }
        }
        for mapping in &channel.model_mappings {
            if seen.insert(mapping.client_model.clone()) {
                models.push(mapping.client_model.clone());
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channel::ModelMapping;
    use crate::test_support::{InMemoryChannelRepository, sample_channel};

    fn channel(name: &str, models: &[&str], enabled: bool) -> Channel {
        let mut c = sample_channel();
        c.name = name.to_string();
        c.models = models.iter().map(|m| m.to_string()).collect();
        c.enabled = enabled;
        c
    }

    fn mapped_channel(client: &str) -> Channel {
        let mut c = channel("mapped", &[], true);
        c.model_mappings = vec![ModelMapping {
            client_model: client.to_string(),
            upstream_model: "gpt-4o".to_string(),
        }];
        c
    }

    /// Models across multiple enabled channels are merged and deduped, preserving order (first-occurrence position).
    #[test]
    fn merges_models_across_enabled_channels_and_dedupes() {
        let channels = vec![
            channel("a", &["gpt-4o", "gpt-4o-mini"], true),
            channel("b", &["gpt-4o", "claude-3"], true),
        ];
        assert_eq!(
            list_models(&channels),
            vec!["gpt-4o", "gpt-4o-mini", "claude-3"]
        );
    }

    /// A mapped `client_model` is also an available model name (routable by the client).
    #[test]
    fn includes_mapping_client_models() {
        let mut c = mapped_channel("chat");
        c.models = vec!["gpt-4o".to_string()];
        assert_eq!(list_models(&[c]), vec!["gpt-4o", "chat"]);
    }

    /// Models of disabled channels are excluded.
    #[test]
    fn excludes_disabled_channels() {
        let channels = vec![
            channel("on", &["gpt-4o"], true),
            channel("off", &["claude-3"], false),
        ];
        assert_eq!(list_models(&channels), vec!["gpt-4o"]);
    }

    /// No enabled channels → empty list.
    #[test]
    fn empty_when_no_enabled_channels() {
        let channels = vec![channel("off", &["gpt-4o"], false)];
        assert!(list_models(&channels).is_empty());
        assert!(list_models(&[]).is_empty());
    }

    /// Use case layer: repository read + merge dedup end to end (seam A mock).
    #[tokio::test]
    async fn usecase_lists_models_via_repository() {
        let repo = InMemoryChannelRepository::new();
        repo.save(&channel("a", &["gpt-4o"], true))
            .await
            .expect("save a");
        repo.save(&channel("b", &["gpt-4o", "claude-3"], false))
            .await
            .expect("save b");

        let models = ListModelsUsecase.execute(&repo).await.expect("list models");
        assert_eq!(models, vec!["gpt-4o"]);
    }
}
