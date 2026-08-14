//! 模型列表用例：`/v1/models` —— 所有启用渠道的模型合并去重，禁用渠道剔除。
//!
//! 数据面契约（docs/Requirements.md）：启用渠道的 `models` 与 `model_mappings.client_model`
//! 合并，去重保序（客户端据此选模型名，映射名同样可路由），禁用渠道不参与；不按密钥过滤。
//! `list_models` 为纯函数，便于与 `ChannelSelector` 保持「列表中的模型必可被选中」的一致。

use std::collections::HashSet;

use crate::domain::channel::{Channel, ChannelRepository};
use crate::domain::error::RepositoryError;

/// 模型列表用例：读取全部渠道并按启用渠道合并去重。
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

/// 纯函数：启用渠道的模型合并去重（保留首次出现顺序）；禁用渠道被剔除。
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

    /// 多启用渠道的模型合并去重，保序（首次出现位置）。
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

    /// 映射的 `client_model` 也是可用模型名（客户端可路由）。
    #[test]
    fn includes_mapping_client_models() {
        let mut c = mapped_channel("chat");
        c.models = vec!["gpt-4o".to_string()];
        assert_eq!(list_models(&[c]), vec!["gpt-4o", "chat"]);
    }

    /// 禁用渠道的模型被剔除。
    #[test]
    fn excludes_disabled_channels() {
        let channels = vec![
            channel("on", &["gpt-4o"], true),
            channel("off", &["claude-3"], false),
        ];
        assert_eq!(list_models(&channels), vec!["gpt-4o"]);
    }

    /// 无启用渠道 → 空列表。
    #[test]
    fn empty_when_no_enabled_channels() {
        let channels = vec![channel("off", &["gpt-4o"], false)];
        assert!(list_models(&channels).is_empty());
        assert!(list_models(&[]).is_empty());
    }

    /// 用例层：仓储读取 + 合并去重端到端（seam A mock）。
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
