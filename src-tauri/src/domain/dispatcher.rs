//! 领域服务：渠道选择策略（ChannelSelector）——按模型筛选候选渠道 → 按优先级排序。
//!
//! 纯业务判断，不碰 DB 与 HTTP（见 docs/Architecture-backend.md）：
//! 1. 只保留启用渠道（禁用渠道不参与调度）；
//! 2. 模型匹配：渠道 `models` 包含请求模型、或 `model_mappings` 命中 `client_model`、
//!    或 `models` 为空（不受限渠道接受任意模型）；
//! 3. 按优先级升序稳定排序（数值越小越优先，同优先级保持输入顺序）。
//!
//! 编排职责（查找渠道、按序重试、应用映射）在 usecases/proxy.rs。

use super::channel::Channel;

/// 渠道选择策略领域服务：从全部渠道中筛出候选渠道并排序。
pub struct ChannelSelector;

impl ChannelSelector {
    /// 选出候选渠道：启用 → 模型匹配 → 优先级升序。
    pub fn select(channels: &[Channel], model: &str) -> Vec<Channel> {
        let mut candidates: Vec<Channel> = channels
            .iter()
            .filter(|c| c.enabled && Self::supports(c, model))
            .cloned()
            .collect();
        candidates.sort_by_key(|c| c.priority);
        candidates
    }

    /// 渠道是否可服务该客户端模型：`models` 非空时要求包含请求模型或有 `client_model`
    /// 映射命中；`models` 为空视为不受限，接受任意模型。
    pub fn supports(channel: &Channel, model: &str) -> bool {
        if channel.models.is_empty() {
            return true;
        }
        channel.models.iter().any(|m| m == model)
            || channel
                .model_mappings
                .iter()
                .any(|m| m.client_model == model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channel::ModelMapping;
    use crate::test_support::sample_channel;

    fn channel(name: &str, models: &[&str], priority: i32, enabled: bool) -> Channel {
        let mut c = sample_channel();
        c.name = name.to_string();
        c.models = models.iter().map(|m| m.to_string()).collect();
        c.priority = priority;
        c.enabled = enabled;
        c
    }

    fn mapped_channel(client: &str, upstream: &str) -> Channel {
        let mut c = channel("mapped", &[], 0, true);
        c.model_mappings = vec![ModelMapping {
            client_model: client.to_string(),
            upstream_model: upstream.to_string(),
        }];
        c
    }

    /// 禁用渠道不进入候选，即使模型匹配。
    #[test]
    fn select_drops_disabled_channels() {
        let channels = vec![channel("off", &["gpt-4o"], 0, false)];
        assert!(ChannelSelector::select(&channels, "gpt-4o").is_empty());
    }

    /// `models` 包含请求模型 → 入选。
    #[test]
    fn select_keeps_channel_when_model_in_models_list() {
        let channels = vec![channel("a", &["gpt-4o"], 1, true)];
        let selected = ChannelSelector::select(&channels, "gpt-4o");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "a");
    }

    /// `model_mappings.client_model` 命中 → 入选（映射名即客户端可用名）。
    #[test]
    fn select_keeps_channel_via_mapping_client_model() {
        let channels = vec![mapped_channel("chat", "gpt-4o")];
        let selected = ChannelSelector::select(&channels, "chat");
        assert_eq!(selected.len(), 1);
    }

    /// `models` 为空 = 不受限渠道，接受任意模型。
    #[test]
    fn select_keeps_unrestricted_channel_with_empty_models() {
        let channels = vec![channel("any", &[], 0, true)];
        assert_eq!(ChannelSelector::select(&channels, "anything-else").len(), 1);
    }

    /// 模型不被任何列表 / 映射命中 → 剔除。
    #[test]
    fn select_drops_channel_not_supporting_model() {
        let channels = vec![
            channel("a", &["gpt-4o"], 0, true),
            channel("b", &["claude-3"], 0, true),
        ];
        let selected = ChannelSelector::select(&channels, "claude-3");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "b");
    }

    /// 候选按优先级升序（数值越小越优先）；同优先级保持输入顺序（稳定排序）。
    #[test]
    fn select_sorts_by_priority_ascending_and_is_stable() {
        let mut a = channel("a", &["gpt-4o"], 5, true);
        a.model_mappings = vec![ModelMapping {
            client_model: "chat".into(),
            upstream_model: "gpt-4o".into(),
        }];
        let b = channel("b", &["gpt-4o"], 0, true);
        let c1 = channel("c1", &["gpt-4o"], 3, true);
        let c2 = channel("c2", &["gpt-4o"], 3, true);

        let channels = vec![a, b, c1, c2];
        let names: Vec<String> = ChannelSelector::select(&channels, "gpt-4o")
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, vec!["b", "c1", "c2", "a"], "优先级升序且同级稳定");
    }

    /// 无启用 / 无匹配渠道时返回空候选。
    #[test]
    fn select_empty_when_no_candidates() {
        let channels = vec![channel("a", &["claude-3"], 0, true)];
        assert!(ChannelSelector::select(&channels, "gpt-4o").is_empty());
    }
}
